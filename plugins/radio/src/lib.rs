//! Internet radio from the radio-browser.info directory: the stations the person names and the
//! most popular ones, anywhere or in one country, as compact tiles with each station's picture,
//! played in a player app. Search covers the whole directory.

use lounge_plugin_sdk::{self as sdk, host, Action, HttpRequest, HttpResponse, Item, ItemKind, Kind, Manifest, RefreshContext, RowSpec, RowStyle, SettingKind, SettingSpec};
use serde::{Deserialize, Serialize};
use std::cell::Cell;
use std::collections::{HashMap, HashSet};

pub struct Radio;

const SERVICE: &str = "Radio Browser";
/// The first address is answered by one of the directory's servers in turn; the second is one
/// server by name, asked when the first does not answer.
const API: [&str; 2] = ["https://all.api.radio-browser.info", "https://de1.api.radio-browser.info"];
/// How long one address may take, so a stalled one leaves time to ask the other within a press.
const TIMEOUT_MS: u32 = 5_000;
/// How many popular stations the row holds.
const POPULAR: usize = 30;
/// What each named station or address was found to be, kept between refreshes.
const RESOLVED_KEY: &str = "resolved";
const DAY_MS: i64 = 86_400_000;
/// How many requests and how long the lookups of named stations may take in one refresh, leaving
/// the rest for reading the stations and the popular row. Entries not reached follow at the next.
const LOOKUP_FETCHES: usize = 40;
const LOOKUP_MS: i64 = 30_000;
/// The most requests one lookup makes: two searches, each perhaps at both addresses.
const FETCHES_PER_LOOKUP: usize = 4;
/// How many stations one request asks for by id.
const BATCH: usize = 50;
const GONE: &str = "Radio Browser no longer lists that station.";

impl sdk::Plugin for Radio {
    fn manifest() -> Manifest {
        Manifest {
            id: "tv.lounge.radio".into(),
            name: "Internet radio".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            description: "Stations you name and the most popular ones from the radio-browser.info directory, played in a player app such as VLC.".into(),
            publisher_name: "Lounge".into(),
            kinds: vec![Kind::Content, Kind::Search],
            // Each station's picture is on its own website.
            fetch_hosts: vec!["*.api.radio-browser.info".into(), "*".into()],
            refresh_minutes: 6 * 60,
            rows: vec![
                RowSpec { id: "yours".into(), title: "Your radio stations".into(), style: RowStyle::Compact },
                RowSpec { id: "popular".into(), title: "Popular radio".into(), style: RowStyle::Compact },
            ],
            settings: vec![
                SettingSpec {
                    key: "stations".into(),
                    label: "Your stations".into(),
                    kind: SettingKind::List,
                    required: false,
                    help: Some("Station names, links to stations on radio-browser.info or stream addresses, one per line or separated by commas. Lounge's search finds stations too.".into()),
                    default: None,
                },
                SettingSpec {
                    key: "country".into(),
                    label: "Country for popular stations".into(),
                    kind: SettingKind::Text,
                    required: false,
                    help: Some("A name or two letters, like Australia or AU. Left empty, the row shows the most popular stations anywhere.".into()),
                    default: None,
                },
            ],
            homepage: Some("https://www.radio-browser.info".into()),
            self_signed_servers: false,
            for_apps: Vec::new(),
            sent_to: Default::default(),
        }
    }

    fn refresh(ctx: &RefreshContext) -> Result<Vec<Item>, String> {
        let mut items = yours(ctx.now_ms)?;
        let typed = host::setting("country").map(|c| c.trim().to_string()).filter(|c| !c.is_empty());
        let code = match &typed {
            Some(t) => Some(country_code(t)?.ok_or_else(|| format!("Radio Browser has no country called {t}. Check the country in the plugin's settings."))?),
            None => None,
        };
        let popular = popular(code.as_deref())?;
        if popular.is_empty()
            && let Some(t) = &typed
        {
            return Err(format!("Radio Browser lists no stations for {t}. Check the country in the plugin's settings."));
        }
        let count = popular.len() as i32;
        items.extend(popular.iter().enumerate().map(|(i, st)| station_item(st, "popular", Some("popular"), count - i as i32, code.is_none())));
        sdk::dedup_ids(&mut items);
        sdk::info!("{} stations", items.len());
        Ok(items)
    }

    fn search(query: &str, limit: u32) -> Result<Vec<Item>, String> {
        let found: Vec<Station> = stations(&format!("/json/stations/search?name={}&limit={}&order=clickcount&reverse=true&hidebroken=true", enc(query), limit.clamp(1, 50) * 2))?;
        Ok(distinct(found).iter().take(limit as usize).map(|st| station_item(st, "found", None, 0, true)).collect())
    }

    /// Asks the directory for the station's address at the press, which it counts as a listen,
    /// as it asks every player to.
    fn resolve(action: &str) -> Result<Action, String> {
        let id = action.strip_prefix("station:").filter(|id| is_uuid(id)).ok_or_else(|| GONE.to_string())?;
        let r = get(&format!("/json/url/{id}"))?;
        if r.status == 404 {
            return Err(GONE.into());
        }
        if !r.ok() {
            return Err(sdk::online_error(SERVICE, r.status));
        }
        let answer: Played = serde_json::from_str(&r.body).map_err(|e| {
            sdk::warn!("unexpected answer for a station's address: {e}");
            unreadable()
        })?;
        let url = answer.url.filter(|u| answer.ok && (u.starts_with("http://") || u.starts_with("https://"))).ok_or_else(|| GONE.to_string())?;
        Ok(Action::play(url, answer.name.unwrap_or_default().trim()))
    }
}

sdk::export_plugin!(Radio);

/// The stations the person named, in their order. Names and addresses are looked up once and
/// what they turned out to be is kept, so a refresh after that asks for the stations by id.
fn yours(now_ms: i64) -> Result<Vec<Item>, String> {
    let lines: Vec<String> = entries(&host::setting("stations").unwrap_or_default());
    let mut resolved = Resolved::load();
    for line in &lines {
        let wanted = wanted(line);
        if matches!(wanted, Wanted::Id(_)) || resolved.0.get(line).is_some_and(|r| !r.stale(now_ms)) {
            continue;
        }
        if FETCHES.with(Cell::get) + FETCHES_PER_LOOKUP > LOOKUP_FETCHES || host::now_ms() - now_ms > LOOKUP_MS {
            sdk::info!("more stations wait for the next refresh");
            break;
        }
        match look_up(&wanted) {
            Ok(found) => {
                if found.is_none() {
                    sdk::warn!("no station found for {}", shown(line));
                }
                resolved.0.insert(line.clone(), Kept { found, at: now_ms });
            }
            // What was found so far is kept, and the rest is looked up at the next refresh.
            Err(e) => {
                sdk::warn!("stations wait for the next refresh: {e}");
                break;
            }
        }
    }
    resolved.0.retain(|line, _| lines.contains(line));
    resolved.save();

    let ids: Vec<String> = lines
        .iter()
        .filter_map(|line| match wanted(line) {
            Wanted::Id(id) => Some(id),
            _ => match resolved.0.get(line).and_then(|k| k.found.as_ref()) {
                Some(Found::Id(id)) => Some(id.clone()),
                _ => None,
            },
        })
        .collect();
    let mut by_id: HashMap<String, Station> = HashMap::new();
    for chunk in ids.chunks(BATCH) {
        for st in stations(&format!("/json/stations/byuuid?uuids={}", chunk.join(",")))? {
            by_id.insert(st.stationuuid.clone(), st);
        }
    }
    let count = lines.len() as i32;
    let mut items = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        let weight = count - i as i32;
        let found = match wanted(line) {
            Wanted::Id(id) => Some(Found::Id(id)),
            _ => resolved.0.get(line).and_then(|k| k.found.clone()),
        };
        match found {
            Some(Found::Id(id)) => match by_id.get(&id) {
                Some(st) => items.push(station_item(st, "yours", Some("yours"), weight, true)),
                // The directory dropped the station, so the entry is looked up afresh next time.
                None => {
                    sdk::warn!("Radio Browser no longer lists the station for {}", shown(line));
                    if resolved.0.remove(line).is_some() {
                        resolved.save();
                    }
                }
            },
            Some(Found::Stream(url)) => items.push(stream_item(&url, weight)),
            None => {}
        }
    }
    Ok(items)
}

/// What an entry names: for a name the station called exactly that, else the best of those
/// whose name holds it; for an address the station that streams from it.
fn look_up(wanted: &Wanted) -> Result<Option<Found>, String> {
    Ok(match wanted {
        Wanted::Name(name) => {
            let options = "limit=10&order=clickcount&reverse=true&hidebroken=true";
            let exact = stations(&format!("/json/stations/search?name={}&nameExact=true&{options}", enc(name)))?;
            let found = if exact.is_empty() { stations(&format!("/json/stations/search?name={}&{options}", enc(name)))? } else { exact };
            best_match(name, found).map(|st| Found::Id(st.stationuuid))
        }
        Wanted::Address(url) => Some(match stations(&format!("/json/stations/byurl?url={}", enc(url)))?.into_iter().next() {
            Some(st) => Found::Id(st.stationuuid),
            None => Found::Stream(url.clone()),
        }),
        Wanted::Id(_) => None,
    })
}

/// The station a name means: one called exactly that, else the most listened to whose name holds it.
fn best_match(name: &str, found: Vec<Station>) -> Option<Station> {
    let wanted = name.trim().to_lowercase();
    let exact = found.iter().position(|st| st.name.trim().to_lowercase() == wanted);
    exact.map(|i| found[i].clone()).or_else(|| found.into_iter().next())
}

/// The two-letter code for a country as the person typed it: a code as it is, a common name for
/// one, the directory's country called that, or else the one with most stations whose name holds it.
fn country_code(typed: &str) -> Result<Option<String>, String> {
    let typed = typed.trim().to_lowercase();
    if let Some(code) = common_name(&typed) {
        return Ok(Some(code.into()));
    }
    if typed.len() == 2 && typed.bytes().all(|b| b.is_ascii_alphabetic()) {
        return Ok(Some(typed.to_ascii_uppercase()));
    }
    let r = get("/json/countries")?;
    if !r.ok() {
        return Err(sdk::online_error(SERVICE, r.status));
    }
    let all: Vec<serde_json::Value> = serde_json::from_str(&r.body).map_err(|e| {
        sdk::warn!("unexpected answer for the countries: {e}");
        unreadable()
    })?;
    let all: Vec<Country> = all.into_iter().filter_map(|v| serde_json::from_value(v).ok()).collect();
    Ok(match_country(&typed, &all))
}

/// Names people use that the directory's names do not hold.
fn common_name(typed: &str) -> Option<&'static str> {
    match typed {
        "uk" | "britain" | "great britain" | "england" | "scotland" | "wales" | "northern ireland" => Some("GB"),
        "usa" | "america" => Some("US"),
        "holland" => Some("NL"),
        "south korea" => Some("KR"),
        "north korea" => Some("KP"),
        _ => None,
    }
}

fn match_country(typed: &str, all: &[Country]) -> Option<String> {
    let plain = |name: &str| {
        let name = name.trim().to_lowercase();
        name.strip_prefix("the ").map(str::to_string).unwrap_or(name)
    };
    let typed = plain(typed);
    all.iter()
        .find(|c| plain(&c.name) == typed)
        .or_else(|| all.iter().filter(|c| plain(&c.name).contains(&typed)).max_by_key(|c| c.stationcount))
        .map(|c| c.iso_3166_1.to_ascii_uppercase())
        .filter(|code| code.len() == 2 && code.bytes().all(|b| b.is_ascii_alphabetic()))
}

/// The most popular stations, anywhere or in the country with this code, each station once.
fn popular(code: Option<&str>) -> Result<Vec<Station>, String> {
    let path = match code {
        Some(code) => format!("/json/stations/bycountrycodeexact/{}?limit={}&order=clickcount&reverse=true&hidebroken=true", enc(code), POPULAR * 2),
        None => format!("/json/stations/topclick/{}?hidebroken=true", POPULAR * 2),
    };
    Ok(distinct(stations(&path)?).into_iter().take(POPULAR).collect())
}

thread_local! {
    /// The address that last answered, asked first by the requests after it.
    static ANSWERED: Cell<usize> = const { Cell::new(0) };
    /// Requests made in this call.
    static FETCHES: Cell<usize> = const { Cell::new(0) };
}

/// A GET from the directory. An address that does not answer in time, or answers with a server
/// error, hands the request to the other, which is then asked first.
fn get(path: &str) -> Result<HttpResponse, String> {
    let first = ANSWERED.with(Cell::get);
    let mut last = None;
    for i in (0..API.len()).map(|k| (first + k) % API.len()) {
        let base = API[i];
        let mut request = HttpRequest::get(format!("{base}{path}")).header("Accept", "application/json");
        request.timeout_ms = TIMEOUT_MS;
        FETCHES.with(|f| f.set(f.get() + 1));
        match sdk::fetch_online(SERVICE, &request) {
            Ok(r) if r.status < 500 => {
                ANSWERED.with(|a| a.set(i));
                return Ok(r);
            }
            Ok(r) => {
                sdk::warn!("HTTP {} from {base}", r.status);
                last = Some(sdk::online_error(SERVICE, r.status));
            }
            Err(e) if e == host::BUDGET_SPENT || e == host::TOO_LARGE => return Err(e),
            Err(e) => last = Some(e),
        }
    }
    Err(last.unwrap_or_else(|| sdk::offline(SERVICE)))
}

/// Stations from the directory. One entry that does not read as a station is passed over.
fn stations(path: &str) -> Result<Vec<Station>, String> {
    let r = get(path)?;
    if !r.ok() {
        sdk::warn!("HTTP {} for {}", r.status, path.split('?').next().unwrap_or(path));
        return Err(sdk::online_error(SERVICE, r.status));
    }
    let all: Vec<serde_json::Value> = serde_json::from_str(&r.body).map_err(|e| {
        sdk::warn!("unexpected answer for {}: {e}", path.split('?').next().unwrap_or(path));
        unreadable()
    })?;
    Ok(all.into_iter().filter_map(|v| serde_json::from_value::<Station>(v).ok()).filter(|st| is_uuid(&st.stationuuid) && !st.name.trim().is_empty()).collect())
}

fn unreadable() -> String {
    format!("{SERVICE} sent a reply this plugin can't read.")
}

/// Each station once: the directory often lists one station several times, and the first, most
/// listened to, is kept. Stations of one name in different countries are different stations.
fn distinct(all: Vec<Station>) -> Vec<Station> {
    let mut seen = HashSet::new();
    all.into_iter().filter(|st| seen.insert((st.name.trim().to_lowercase(), st.countrycode.to_uppercase()))).collect()
}

fn station_item(st: &Station, prefix: &str, row: Option<&str>, weight: i32, with_country: bool) -> Item {
    let name = st.name.trim().to_string();
    Item {
        id: format!("{prefix}:{}", st.stationuuid),
        kind: ItemKind::Program,
        subtitle: Some(st.facts(with_country)).filter(|s| !s.is_empty()),
        logo: Some(st.favicon.trim().to_string()).filter(|f| f.starts_with("https://") || f.starts_with("http://")),
        row: row.map(str::to_string),
        weight,
        action: Action { resolve: Some(format!("station:{}", st.stationuuid)), title: Some(name.clone()), ..Default::default() },
        title: name,
        ..Default::default()
    }
}

/// A stream the directory does not list, named after its server and played as it is.
fn stream_item(url: &str, weight: i32) -> Item {
    let server = stream_name(url);
    Item {
        id: format!("yours:stream:{:x}", fnv(url)),
        kind: ItemKind::Program,
        subtitle: Some("A stream you added".into()),
        row: Some("yours".into()),
        weight,
        action: Action::play(url, &server),
        title: server,
        ..Default::default()
    }
}

/// A stream's server and the first part of its path, as "radio.example.org/live", without any
/// sign-in written into the address.
fn stream_name(url: &str) -> String {
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    let rest = rest.split(['?', '#']).next().unwrap_or(rest);
    let (authority, path) = rest.split_once('/').unwrap_or((rest, ""));
    let host = authority.rsplit('@').next().unwrap_or(authority);
    let host = host.split(':').next().unwrap_or(host);
    match path.split('/').find(|p| !p.is_empty()) {
        Some(first) => format!("{host}/{first}"),
        None => host.to_string(),
    }
}

/// An entry as the log shows it: an address by its server, so a sign-in in it is not written down.
fn shown(line: &str) -> String {
    match wanted(line) {
        Wanted::Address(url) => stream_name(&url),
        _ => line.to_string(),
    }
}

/// What one entry of the setting names.
#[derive(Debug, PartialEq)]
enum Wanted {
    Id(String),
    Address(String),
    Name(String),
}

fn wanted(line: &str) -> Wanted {
    if let Some(id) = uuid_in(line)
        && (line.len() == 36 || line.contains("radio-browser.info"))
    {
        return Wanted::Id(id);
    }
    if line.starts_with("http://") || line.starts_with("https://") {
        return Wanted::Address(line.to_string());
    }
    Wanted::Name(line.to_string())
}

/// The setting's entries, one per line or separated by commas, each once.
fn entries(raw: &str) -> Vec<String> {
    let mut seen = HashSet::new();
    raw.split(['\n', ',']).map(|e| e.trim().to_string()).filter(|e| !e.is_empty() && seen.insert(e.to_lowercase())).collect()
}

/// A station id written anywhere in the text, read byte by byte so other alphabets are no trouble.
fn uuid_in(text: &str) -> Option<String> {
    text.as_bytes().windows(36).find(|w| uuid_bytes(w)).map(|w| String::from_utf8_lossy(w).to_ascii_lowercase())
}

fn is_uuid(s: &str) -> bool {
    uuid_bytes(s.as_bytes())
}

fn uuid_bytes(b: &[u8]) -> bool {
    b.len() == 36 && b.iter().enumerate().all(|(i, b)| if matches!(i, 8 | 13 | 18 | 23) { *b == b'-' } else { b.is_ascii_hexdigit() })
}

fn enc(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

fn fnv(s: &str) -> u64 {
    s.bytes().fold(0xcbf29ce484222325, |h, b| (h ^ b as u64).wrapping_mul(0x100000001b3))
}

#[derive(Deserialize, Default, Clone)]
struct Station {
    #[serde(default)]
    stationuuid: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    country: String,
    #[serde(default)]
    countrycode: String,
    #[serde(default)]
    tags: String,
    #[serde(default)]
    favicon: String,
}

impl Station {
    /// The station's country and first two kinds of programme, as "France, news, public radio".
    fn facts(&self, with_country: bool) -> String {
        let country = short_country(&self.country);
        let (named, code, station) = (country.to_lowercase(), self.countrycode.to_lowercase(), self.name.to_lowercase());
        // A tag that repeats the station's own name or gives its frequency says nothing new.
        let kinds = self
            .tags
            .split(',')
            .map(|t| t.trim().to_lowercase())
            .filter(|t| !t.is_empty() && t.len() <= 24 && !FORMATS.contains(&t.as_str()) && *t != named && *t != code && !station.contains(t.as_str()) && !frequency(t))
            .take(2);
        let facts: Vec<String> = (with_country && !country.is_empty()).then_some(country).into_iter().chain(kinds).collect();
        let joined = facts.join(", ");
        let mut chars = joined.chars();
        chars.next().map(|c| c.to_uppercase().chain(chars).collect()).unwrap_or_default()
    }
}

/// Whether a tag is a broadcast frequency, as "am1629" or "fm 102.9".
fn frequency(tag: &str) -> bool {
    let rest = tag.strip_prefix("am").or_else(|| tag.strip_prefix("fm")).map(str::trim_start);
    rest.is_some_and(|r| r.starts_with(|c: char| c.is_ascii_digit()))
}

/// Tags that name how a station is sent rather than what it plays.
const FORMATS: [&str; 9] = ["aac", "aac+", "mp3", "hls", "ogg", "opus", "flac", "stream", "radio"];

/// A country as people say it: the directory gives ISO's long names, such as "The United
/// Kingdom Of Great Britain And Northern Ireland".
fn short_country(name: &str) -> String {
    let name = name.trim().trim_start_matches("The ");
    let name = name.split(" (").next().unwrap_or(name).split(", ").next().unwrap_or(name);
    match name {
        "United States Of America" => "United States".into(),
        "United Kingdom Of Great Britain And Northern Ireland" => "United Kingdom".into(),
        "Russian Federation" => "Russia".into(),
        "Republic Of Korea" => "South Korea".into(),
        "Democratic Peoples Republic Of Korea" => "North Korea".into(),
        "Netherlands" => "the Netherlands".into(),
        other => ["Islamic Republic Of ", "Bolivarian Republic Of ", "Plurinational State Of ", "United Republic Of ", "Republic Of "]
            .iter()
            .find_map(|form| other.strip_prefix(form))
            .unwrap_or(other)
            .into(),
    }
}

#[derive(Deserialize)]
struct Country {
    #[serde(default)]
    name: String,
    #[serde(default)]
    iso_3166_1: String,
    #[serde(default)]
    stationcount: u32,
}

#[derive(Deserialize)]
struct Played {
    #[serde(default)]
    ok: bool,
    name: Option<String>,
    url: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
enum Found {
    Id(String),
    Stream(String),
}

#[derive(Serialize, Deserialize, Clone)]
struct Kept {
    found: Option<Found>,
    at: i64,
}

impl Kept {
    /// Whether to look the entry up again. A station found stays the one found; an address the
    /// directory did not list is asked about weekly, and an entry that found nothing daily.
    fn stale(&self, now_ms: i64) -> bool {
        let age = now_ms - self.at;
        match self.found {
            Some(Found::Id(_)) => false,
            Some(Found::Stream(_)) => age > 7 * DAY_MS,
            None => age > DAY_MS,
        }
    }
}

/// What each entry was found to be, by the entry as written.
struct Resolved(HashMap<String, Kept>);

impl Resolved {
    fn load() -> Resolved {
        Resolved(host::kv_get(RESOLVED_KEY).and_then(|v| serde_json::from_slice(&v).ok()).unwrap_or_default())
    }

    fn save(&self) {
        host::kv_set(RESOLVED_KEY, serde_json::to_string(&self.0).unwrap_or_default().as_bytes());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RTL: &str = r#"{"changeuuid":"x","stationuuid":"042d3140-227c-4fac-9387-4903b692d5f2","name":"RTL ","url":"https://x/index.m3u8","url_resolved":"https://x/index.m3u8",
        "homepage":"https://www.rtl.fr/","favicon":"https://duckduckgo.com/i/035a5e15d396c67a.png","tags":"généraliste,aac,France","country":"France","countrycode":"FR",
        "codec":"AAC","bitrate":64,"hls":1,"lastcheckok":1,"votes":3000,"clickcount":4000}"#;

    #[test]
    fn a_station_reads_as_its_country_and_what_it_plays() {
        let st: Station = serde_json::from_str(RTL).unwrap();
        let item = station_item(&st, "popular", Some("popular"), 5, true);
        assert_eq!((item.id.as_str(), item.title.as_str(), item.subtitle.as_deref()), ("popular:042d3140-227c-4fac-9387-4903b692d5f2", "RTL", Some("France, généraliste")));
        assert_eq!(item.logo.as_deref(), Some("https://duckduckgo.com/i/035a5e15d396c67a.png"));
        assert_eq!((item.row.as_deref(), item.weight), (Some("popular"), 5));
        assert_eq!(item.action.resolve.as_deref(), Some("station:042d3140-227c-4fac-9387-4903b692d5f2"));
        // A row of one country's stations needn't say the country on each.
        assert_eq!(st.facts(false), "Généraliste");
        let uk = Station { country: "The United Kingdom Of Great Britain And Northern Ireland".into(), tags: "news,talk,bbc,public radio".into(), ..Default::default() };
        assert_eq!(uk.facts(true), "United Kingdom, news, talk");
        assert_eq!(short_country("Islamic Republic Of Iran"), "Iran");
        assert_eq!(short_country("The Republic Of Korea"), "South Korea");
        let abc = Station { name: "ABC Radio National AAC+".into(), country: "Australia".into(), tags: "abc,features,am1629,fm 102.9,news".into(), ..Default::default() };
        assert_eq!(abc.facts(false), "Features, news", "the network's name and the frequency are left out");
        assert!(frequency("am1629") && frequency("fm 102.9") && !frequency("ambient") && !frequency("fmusic"));
        assert_eq!(short_country("Taiwan, Republic Of China"), "Taiwan");
    }

    #[test]
    fn entries_name_stations_three_ways() {
        assert_eq!(entries(" ABC Classic \nRadio Paradise, abc classic\n\n"), ["ABC Classic", "Radio Paradise"]);
        assert_eq!(wanted("042d3140-227c-4fac-9387-4903b692d5f2"), Wanted::Id("042d3140-227c-4fac-9387-4903b692d5f2".into()));
        assert_eq!(wanted("https://www.radio-browser.info/history/042D3140-227C-4FAC-9387-4903B692D5F2"), Wanted::Id("042d3140-227c-4fac-9387-4903b692d5f2".into()));
        assert_eq!(wanted("https://stream.example.com/live.mp3"), Wanted::Address("https://stream.example.com/live.mp3".into()));
        // An id inside some other address is that address, not a station.
        assert_eq!(wanted("https://cdn.example.com/042d3140-227c-4fac-9387-4903b692d5f2.mp3"), Wanted::Address("https://cdn.example.com/042d3140-227c-4fac-9387-4903b692d5f2.mp3".into()));
        assert_eq!(wanted("Triple J"), Wanted::Name("Triple J".into()));
        assert!(!is_uuid("042d3140-227c-4fac-9387-4903b692d5fz") && !is_uuid("short"));
        assert_eq!(wanted("全澳华语广播华厦之声 Радио Россия"), Wanted::Name("全澳华语广播华厦之声 Радио Россия".into()));
    }

    #[test]
    fn a_stream_the_directory_does_not_know_plays_as_it_is() {
        let item = stream_item("http://radio.example.org:8000/live?x=1", 2);
        assert_eq!((item.title.as_str(), item.subtitle.as_deref()), ("radio.example.org/live", Some("A stream you added")));
        assert_eq!(stream_name("https://me:secret@radio.example.org/"), "radio.example.org", "a sign-in in the address is not shown");
        assert_eq!(stream_name("http://radio.example.org/jazz/high"), "radio.example.org/jazz");
        assert_eq!(shown("https://me:secret@radio.example.org/live"), "radio.example.org/live");
        assert_eq!(item.action.play.as_deref(), Some("http://radio.example.org:8000/live?x=1"));
        assert_eq!(item.id, stream_item("http://radio.example.org:8000/live?x=1", 9).id, "the id follows the address, not the place");
    }

    #[test]
    fn a_name_finds_the_station_called_exactly_that() {
        let one = |name: &str, id: &str| Station { name: name.into(), stationuuid: id.into(), ..Default::default() };
        let found = vec![one("ABC Classic 2 HLS", "a"), one("ABC Classic", "b"), one("ABC Classic (NSW)", "c")];
        assert_eq!(best_match(" abc classic ", found.clone()).map(|s| s.stationuuid).as_deref(), Some("b"));
        assert_eq!(best_match("classic", found).map(|s| s.stationuuid).as_deref(), Some("a"), "else the most listened to");
        assert!(best_match("x", vec![]).is_none());
    }

    #[test]
    fn a_country_is_found_as_people_call_it() {
        let c = |name: &str, code: &str, stationcount: u32| Country { name: name.into(), iso_3166_1: code.into(), stationcount };
        let all = [
            c("The United States Minor Outlying Islands", "UM", 26),
            c("The United States Of America", "US", 7067),
            c("The United Kingdom Of Great Britain And Northern Ireland", "GB", 1993),
            c("The Democratic Peoples Republic Of Korea", "KP", 7),
            c("The Republic Of Korea", "KR", 85),
            c("The Russian Federation", "RU", 2531),
            c("Niger", "NE", 3),
            c("Nigeria", "NG", 120),
        ];
        let code = |typed: &str| match_country(typed, &all);
        assert_eq!(code("United States").as_deref(), Some("US"), "the one with most stations");
        assert_eq!(code("united kingdom").as_deref(), Some("GB"));
        assert_eq!(code("Korea").as_deref(), Some("KR"));
        assert_eq!(code("Russia").as_deref(), Some("RU"));
        assert_eq!(code("Niger").as_deref(), Some("NE"), "a name the directory has exactly wins");
        assert_eq!(code("the united states of america").as_deref(), Some("US"));
        assert_eq!(code("Atlantis"), None);
        assert_eq!((common_name("uk"), common_name("holland"), common_name("france")), (Some("GB"), Some("NL"), None));
    }

    #[test]
    fn a_station_listed_twice_is_shown_once() {
        let one = |name: &str, id: &str, country: &str| Station { name: name.into(), stationuuid: id.into(), countrycode: country.into(), ..Default::default() };
        let kept = distinct(vec![one("ABC Classic", "a", "AU"), one(" abc classic", "b", "AU"), one("Triple J", "c", "AU"), one("Radio 1", "d", "BE"), one("Radio 1", "e", "CZ")]);
        assert_eq!(kept.iter().map(|s| s.stationuuid.as_str()).collect::<Vec<_>>(), ["a", "c", "d", "e"], "one name in two countries is two stations");
    }

    #[test]
    fn what_an_entry_was_found_to_be_is_looked_up_again_in_time() {
        let now = 100 * DAY_MS;
        assert!(!Kept { found: Some(Found::Id("a".into())), at: 0 }.stale(now), "a station found stays the one found");
        assert!(!Kept { found: Some(Found::Stream("http://s".into())), at: now - DAY_MS * 3 }.stale(now));
        assert!(Kept { found: Some(Found::Stream("http://s".into())), at: now - DAY_MS * 8 }.stale(now));
        assert!(Kept { found: None, at: now - DAY_MS * 2 }.stale(now), "nothing found is tried again after a day");
        let kept: HashMap<String, Kept> = [("Triple J".to_string(), Kept { found: Some(Found::Stream("http://s".into())), at: 1 })].into();
        let back: HashMap<String, Kept> = serde_json::from_str(&serde_json::to_string(&kept).unwrap()).unwrap();
        assert_eq!(back["Triple J"].found, Some(Found::Stream("http://s".into())));
    }

    #[test]
    fn the_address_comes_back_with_the_listen_counted() {
        let played: Played = serde_json::from_str(r#"{"ok":true,"message":"retrieved station url","stationuuid":"042d3140-227c-4fac-9387-4903b692d5f2","name":"RTL","url":"https://live.example/index.m3u8"}"#).unwrap();
        assert!(played.ok && played.url.as_deref() == Some("https://live.example/index.m3u8"));
        assert_eq!(<Radio as sdk::Plugin>::resolve("station:not-an-id").err().as_deref(), Some(GONE));
    }

    #[test]
    fn the_manifest_needs_no_settings() {
        let m = <Radio as sdk::Plugin>::manifest();
        assert!(m.validate().is_ok(), "{:?}", m.validate());
        assert!(m.settings.iter().all(|s| !s.required), "it shows popular stations as soon as it is on");
        assert_eq!(enc("abc classic & co"), "abc%20classic%20%26%20co");
        // Off the TV there is no host, so a refresh fails the way an unreachable directory would.
        assert_eq!(<Radio as sdk::Plugin>::refresh(&RefreshContext::default()).err().as_deref(), Some("Radio Browser didn't answer. Check the TV's internet connection, then try again."));
    }
}

