//! Seerr plugin, which also reads Jellyseerr and Overseerr: what has been requested and is still
//! to come, saying whether it waits for approval or is on its way, and what has lately become
//! available. A tile searches for its title, so it opens in whichever app has it.

use lounge_plugin_sdk::{self as sdk, host, mark, parse_iso8601, t, tc, tn, Action, HttpRequest, Item, ItemKind, Kind, Manifest, RefreshContext, RowSpec, RowStyle, SettingKind, SettingSpec};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

pub struct Seerr;

const SERVICE: &str = "Seerr";
/// How many requests each row asks for.
const TAKE: u32 = 20;
/// Titles and artwork looked up before, by media type and TMDB id.
const TITLES_KEY: &str = "titles";
/// How long something stays in Now available after it arrived.
const AVAILABLE_FOR_MS: i64 = 30 * DAY_MS;
const DAY_MS: i64 = 24 * 3_600_000;
/// How many titles already known are looked up again in one refresh, for a new name or poster.
const LOOK_AGAIN: usize = 8;
const IMAGES: &str = "https://image.tmdb.org/t/p";
const KEY_REFUSED: &str = mark!("Seerr didn't accept the API key. Check it in the plugin's settings.");

impl sdk::Plugin for Seerr {
    sdk::locales!();

    fn manifest() -> Manifest {
        Manifest {
            id: "tv.lounge.seerr".into(),
            name: "Seerr".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            description: t!("What has been requested and is on its way, and what has just become available, from Seerr, Jellyseerr or Overseerr."),
            publisher_name: "Lounge".into(),
            kinds: vec![Kind::Content],
            fetch_hosts: vec!["$server".into(), "image.tmdb.org".into()],
            refresh_minutes: 30,
            refresh_seconds: 0,
            rows: vec![
                // Landscape tiles carry a second line, which says where each request stands.
                RowSpec { id: "requested".into(), title: t!("Requested on Seerr"), style: RowStyle::Landscape },
                RowSpec { id: "available".into(), title: t!("Now available on Seerr"), style: RowStyle::Poster },
            ],
            settings: vec![
                SettingSpec { key: "server".into(), label: t!("Server address"), kind: SettingKind::Url, required: true, help: Some(t!("Like http://192.168.0.10:5055")), default: None },
                SettingSpec {
                    key: "api_key".into(),
                    label: t!("API key"),
                    kind: SettingKind::Password,
                    required: true,
                    // Translators: "Settings, General" is where the key is in Seerr's own menus. The phone companion is Lounge's page on a phone.
                    help: Some(t!("In Seerr, Jellyseerr or Overseerr under Settings, General. It is easiest pasted from the phone companion.")),
                    default: None,
                },
            ],
            homepage: Some("https://docs.seerr.dev".into()),
            self_signed_servers: false,
            for_apps: Vec::new(),
            sent_to: Default::default(),
            locales: Default::default(),
        }
    }

    fn refresh(ctx: &RefreshContext) -> Result<Vec<Item>, String> {
        let s = Session::connect()?;
        let to_come: Page = s.get_json(&format!("/api/v1/request?take={TAKE}&filter=unavailable&sort=added"))?;
        let arrived: Page = s.get_json(&format!("/api/v1/request?take={TAKE}&filter=available&sort=modified"))?;
        let to_come: Vec<Request> = to_come.results.into_iter().filter(|r| r.status != DECLINED).collect();
        let arrived: Vec<Request> = arrived.results.into_iter().filter(|r| r.status != DECLINED && recent(r, ctx.now_ms)).collect();

        let mut titles = Titles::load();
        let mut asked = HashSet::new();
        let mut again = 0;
        for r in to_come.iter().chain(&arrived) {
            let Some(key) = r.media.key() else { continue };
            let known = titles.0.get(&key);
            if known.is_some_and(|t| !t.stale(ctx.now_ms)) || !asked.insert(key.clone()) {
                continue;
            }
            if known.is_some() {
                if again == LOOK_AGAIN {
                    continue;
                }
                again += 1;
            }
            match s.title(&r.media, ctx.now_ms) {
                Ok(Some(title)) => {
                    titles.0.insert(key, title);
                }
                // A title Seerr no longer gives keeps what was known, until it is due again.
                Ok(None) => {
                    if let Some(t) = titles.0.get_mut(&key) {
                        t.at = ctx.now_ms;
                    }
                }
                Err(e) if e == sdk::tr(KEY_REFUSED) => return Err(e),
                // What was found so far is kept, and the rest is looked up at the next refresh.
                Err(e) => {
                    sdk::warn!("some titles wait for the next refresh: {e}");
                    break;
                }
            }
        }
        // Only the titles still on show are kept, so the store stays small.
        let shown: HashSet<String> = to_come.iter().chain(&arrived).filter_map(|r| r.media.key()).collect();
        titles.0.retain(|k, _| shown.contains(k));
        titles.save();

        // A name next to each request says something only when more than one person asks.
        let several = to_come.iter().chain(&arrived).filter_map(|r| r.requested_by.as_ref().and_then(|p| p.id)).collect::<HashSet<_>>().len() > 1;
        let mut items = Vec::new();
        let count = to_come.len() as i32;
        for (i, r) in to_come.iter().enumerate() {
            items.extend(titles.of(r).map(|(key, t)| requested_item(r, &key, t, several, count - i as i32)));
        }
        let count = arrived.len() as i32;
        for (i, r) in arrived.iter().enumerate() {
            items.extend(titles.of(r).map(|(key, t)| available_item(r, &key, t, several, count - i as i32, ctx.now_ms)));
        }
        sdk::dedup_ids(&mut items);
        sdk::info!("{} items from {}", items.len(), sdk::redact_url(&s.server));
        Ok(items)
    }
}

sdk::export_plugin!(Seerr);

/// A request's own status: waiting for approval, approved, declined, failed or completed.
const PENDING: i64 = 1;
const DECLINED: i64 = 3;
const FAILED: i64 = 4;
/// The media's status when some of what was asked for has arrived.
const PARTLY_AVAILABLE: i64 = 4;

struct Session {
    server: String,
    key: String,
}

impl Session {
    fn connect() -> Result<Session, String> {
        // Translators: names the setting inside "Add the {label} in the plugin's settings."
        let server = host::server_setting("server", &t!("server address"))?;
        // Translators: names the setting inside "Add the {label} in the plugin's settings."
        let key = host::secret("api_key").map(|k| k.trim().to_string()).filter(|k| !k.is_empty()).ok_or_else(|| sdk::not_set(&tc!("inside a sentence", "API key")))?;
        Ok(Session { server, key })
    }

    fn request(&self, path: &str) -> HttpRequest {
        HttpRequest::get(format!("{}{path}", self.server)).header("X-Api-Key", &self.key).header("Accept", "application/json")
    }

    fn get_json<T: serde::de::DeserializeOwned>(&self, path: &str) -> Result<T, String> {
        let r = sdk::fetch_from(SERVICE, &self.server, &self.request(path))?;
        match r.status {
            401 | 403 => return Err(sdk::tr(KEY_REFUSED)),
            404 => return Err(sdk::wrong_server(SERVICE, &self.server)),
            _ if !r.ok() => {
                sdk::warn!("HTTP {} for {path}", r.status);
                return Err(sdk::answered_error(SERVICE, &self.server, r.status));
            }
            _ => {}
        }
        serde_json::from_str(&r.body).map_err(|e| {
            sdk::warn!("unexpected answer for {path}: {e}");
            sdk::unreadable(SERVICE, &self.server)
        })
    }

    /// A film's or show's title and artwork, or nothing when Seerr no longer knows it.
    fn title(&self, media: &Media, now_ms: i64) -> Result<Option<Title>, String> {
        let Some(id) = media.tmdb_id else { return Ok(None) };
        let path = format!("/api/v1/{}/{id}{}", if media.media_type == "tv" { "tv" } else { "movie" }, language_query());
        let r = sdk::fetch_from(SERVICE, &self.server, &self.request(&path))?;
        if !r.ok() {
            sdk::warn!("HTTP {} for {path}", r.status);
            return if matches!(r.status, 401 | 403) { Err(sdk::tr(KEY_REFUSED)) } else { Ok(None) };
        }
        let d: Details = serde_json::from_str(&r.body).map_err(|e| {
            sdk::warn!("unexpected answer for {path}: {e}");
            sdk::unreadable(SERVICE, &self.server)
        })?;
        Ok(d.title.or(d.name).filter(|t| !t.is_empty()).map(|t| Title {
            name: t,
            poster: d.poster_path.filter(|p| p.starts_with('/')),
            backdrop: d.backdrop_path.filter(|p| p.starts_with('/')),
            year: d.release_date.or(d.first_air_date).and_then(|date| date.get(..4).filter(|y| y.bytes().all(|b| b.is_ascii_digit())).map(str::to_string)),
            at: now_ms,
        }))
    }
}

fn requested_item(r: &Request, key: &str, t: &Title, several: bool, weight: i32) -> Item {
    let seasons = r.seasons_text();
    let state = r.state(seasons.is_none());
    let words = [seasons, Some(state), several.then(|| r.requester(false)).flatten()];
    Item {
        id: format!("requested:{key}"),
        kind: ItemKind::Program,
        title: t.name.clone(),
        subtitle: Some(sentence(words)),
        poster: t.poster.as_ref().map(|p| format!("{IMAGES}/w500{p}")),
        thumbnail: t.backdrop.as_ref().map(|b| format!("{IMAGES}/w1280{b}")),
        row: Some("requested".into()),
        weight,
        action: search(&t.name),
        ..Default::default()
    }
}

/// Something that has arrived, dated by when it did so the New badge shows for a week.
fn available_item(r: &Request, key: &str, t: &Title, several: bool, weight: i32, now_ms: i64) -> Item {
    let arrived = r.arrived_ms();
    let words = [t.year.clone(), several.then(|| r.requester(t.year.is_none())).flatten()];
    Item {
        id: format!("available:{key}"),
        kind: ItemKind::Program,
        title: t.name.clone(),
        subtitle: Some(sentence(words)).filter(|s| !s.is_empty()),
        added_ms: arrived,
        is_new: sdk::new_this_week(arrived, now_ms),
        poster: t.poster.as_ref().map(|p| format!("{IMAGES}/w500{p}")),
        thumbnail: t.backdrop.as_ref().map(|b| format!("{IMAGES}/w1280{b}")),
        row: Some("available".into()),
        weight,
        action: search(&t.name),
        ..Default::default()
    }
}

/// Lounge's own search for the title, which finds it in whichever app or plugin has it.
fn search(title: &str) -> Action {
    Action { search: Some(title.to_string()), title: Some(title.to_string()), ..Default::default() }
}

/// Facts joined with commas. Each is already worded for its place in the line.
fn sentence<const N: usize>(words: [Option<String>; N]) -> String {
    words.into_iter().flatten().filter(|w| !w.is_empty()).collect::<Vec<_>>().join(", ")
}

/// The query that asks for a title in the language in use. Unasked, Seerr answers in its own.
fn language_query() -> String {
    match sdk::words::language().as_str() {
        sdk::words::SOURCE => String::new(),
        tag => format!("?language={tag}"),
    }
}

/// Where the titles are kept. Each language keeps its own, since a title is named in each.
fn titles_key() -> String {
    match sdk::words::language().as_str() {
        sdk::words::SOURCE => TITLES_KEY.to_string(),
        tag => format!("{TITLES_KEY}:{tag}"),
    }
}

/// Whether a request that has arrived did so within the last month.
fn recent(r: &Request, now_ms: i64) -> bool {
    r.arrived_ms().is_none_or(|at| now_ms - at <= AVAILABLE_FOR_MS)
}

#[derive(Deserialize, Default)]
struct Page {
    #[serde(default)]
    results: Vec<Request>,
}

#[derive(Deserialize, Default, Clone)]
pub struct Request {
    #[serde(default)]
    status: i64,
    #[serde(default)]
    media: Media,
    #[serde(rename = "requestedBy")]
    requested_by: Option<Person>,
    #[serde(default)]
    seasons: Vec<Season>,
    #[serde(rename = "is4k", default)]
    is_4k: bool,
}

impl Request {
    /// Where the request stands, in a few words, worded to start the tile's second line when
    /// `first` and to follow a comma otherwise.
    fn state(&self, first: bool) -> String {
        let (status, downloads) = if self.is_4k { (self.media.status_4k, &self.media.download_status_4k) } else { (self.media.status, &self.media.download_status) };
        let size: f64 = downloads.iter().map(|d| d.size).filter(|s| s.is_finite() && *s > 0.0).sum();
        let left: f64 = downloads.iter().map(|d| d.size_left).filter(|s| s.is_finite() && *s >= 0.0).sum();
        let percent = if size > 0.0 { (((size - left.min(size)) / size) * 100.0).round() as i64 } else { 0 };
        match (self.status, first) {
            // Translators: the state of a request for a film or a show, at the start of the line under its title.
            (PENDING, true) => t!("Waiting for approval"),
            // Translators: the same state after a comma, as in "Season 1, waiting for approval".
            (PENDING, false) => t!("waiting for approval"),
            (FAILED, true) => t!("The request failed"),
            // Translators: after a comma, as in "Season 1, the request failed".
            (FAILED, false) => t!("the request failed"),
            // Translators: {percent} is how much of the download is done, as in "Downloading, 40%".
            (_, true) if size > 0.0 => t!("Downloading, {percent}%", percent = percent),
            // Translators: after a comma, as in "2 seasons, downloading, 40%".
            (_, false) if size > 0.0 => t!("downloading, {percent}%", percent = percent),
            // Translators: some of the episodes or seasons asked for have arrived.
            (_, true) if status == Some(PARTLY_AVAILABLE) => t!("Partly available"),
            // Translators: after a comma, as in "2 seasons, partly available".
            (_, false) if status == Some(PARTLY_AVAILABLE) => t!("partly available"),
            // Translators: the request is approved and what was asked for has not arrived yet.
            (_, true) => t!("On its way"),
            // Translators: after a comma, as in "Season 1, on its way".
            (_, false) => t!("on its way"),
        }
    }

    /// The seasons asked for, for a show, which start the tile's second line.
    fn seasons_text(&self) -> Option<String> {
        match self.seasons.as_slice() {
            [] => None,
            // Translators: the one season of a show that was requested, by its number.
            [one] => Some(t!("Season {number}", number = one.season_number)),
            // Translators: how many seasons of a show were requested, always two or more.
            many => Some(tn!(many.len(), "{n} season", "{n} seasons")),
        }
    }

    /// Who asked, worded to start the line when `first` and to follow a comma otherwise.
    fn requester(&self, first: bool) -> Option<String> {
        let name = self.requested_by.as_ref()?.display_name.as_ref().filter(|n| !n.trim().is_empty())?.trim();
        Some(if first {
            // Translators: {name} is the Seerr user who asked for the film or show.
            t!("Requested by {name}", name = name)
        } else {
            // Translators: after a comma, as in "2008, requested by sam".
            t!("requested by {name}", name = name)
        })
    }

    /// When the media arrived. Seerr can write `mediaAddedAt` when the request is made, so the
    /// later of it and the media's last change is taken.
    fn arrived_ms(&self) -> Option<i64> {
        let added = self.media.media_added_at.as_deref().and_then(parse_iso8601);
        let changed = self.media.updated_at.as_deref().and_then(parse_iso8601);
        added.max(changed)
    }
}

#[derive(Deserialize, Default, Clone)]
struct Media {
    #[serde(rename = "tmdbId")]
    tmdb_id: Option<i64>,
    #[serde(rename = "mediaType", default)]
    media_type: String,
    status: Option<i64>,
    #[serde(rename = "status4k")]
    status_4k: Option<i64>,
    #[serde(rename = "mediaAddedAt")]
    media_added_at: Option<String>,
    #[serde(rename = "updatedAt")]
    updated_at: Option<String>,
    #[serde(rename = "downloadStatus", default)]
    download_status: Vec<Download>,
    #[serde(rename = "downloadStatus4k", default)]
    download_status_4k: Vec<Download>,
}

impl Media {
    /// The key a film or show is known by here, which two requests for it share.
    fn key(&self) -> Option<String> {
        self.tmdb_id.map(|id| format!("{}:{id}", if self.media_type == "tv" { "tv" } else { "movie" }))
    }
}

#[derive(Deserialize, Default, Clone)]
struct Download {
    #[serde(default)]
    size: f64,
    #[serde(rename = "sizeLeft", default)]
    size_left: f64,
}

#[derive(Deserialize, Default, Clone)]
struct Person {
    id: Option<i64>,
    #[serde(rename = "displayName")]
    display_name: Option<String>,
}

#[derive(Deserialize, Default, Clone)]
struct Season {
    #[serde(rename = "seasonNumber", default)]
    season_number: i64,
}

#[derive(Deserialize)]
struct Details {
    title: Option<String>,
    name: Option<String>,
    #[serde(rename = "posterPath")]
    poster_path: Option<String>,
    #[serde(rename = "backdropPath")]
    backdrop_path: Option<String>,
    #[serde(rename = "releaseDate")]
    release_date: Option<String>,
    #[serde(rename = "firstAirDate")]
    first_air_date: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
struct Title {
    name: String,
    poster: Option<String>,
    backdrop: Option<String>,
    year: Option<String>,
    /// When it was looked up.
    #[serde(default)]
    at: i64,
}

impl Title {
    /// Whether to look it up again: a week on, or a day on while it still has no poster, as a
    /// title asked for before its release often lacks one.
    fn stale(&self, now_ms: i64) -> bool {
        let age = now_ms - self.at;
        age > 7 * DAY_MS || (self.poster.is_none() && age > DAY_MS)
    }
}

/// The titles looked up so far, kept between refreshes so each is asked for once.
struct Titles(HashMap<String, Title>);

impl Titles {
    fn load() -> Titles {
        Titles(host::kv_get(&titles_key()).and_then(|v| serde_json::from_slice(&v).ok()).unwrap_or_default())
    }

    fn save(&self) {
        host::kv_set(&titles_key(), serde_json::to_string(&self.0).unwrap_or_default().as_bytes());
    }

    fn of(&self, r: &Request) -> Option<(String, &Title)> {
        let key = r.media.key()?;
        let title = self.0.get(&key)?;
        Some((key, title))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session() -> Session {
        Session { server: "http://seerr:5055".into(), key: "k".into() }
    }

    /// Requests as the server lists them: waiting, approved, and a show with one season.
    const TO_COME: &str = r#"{"pageInfo":{"pages":1,"pageSize":20,"results":3,"page":1},"results":[
        {"id":5,"status":1,"type":"movie","is4k":false,"media":{"downloadStatus":[],"downloadStatus4k":[],"id":5,"mediaType":"movie","tmdbId":10098,"status":2,"status4k":1,"mediaAddedAt":null,"updatedAt":"2026-11-11T11:11:15.000Z"},
         "requestedBy":{"id":2,"displayName":"sam"},"seasons":[]},
        {"id":1,"status":2,"type":"movie","is4k":false,"media":{"downloadStatus":[],"downloadStatus4k":[],"id":1,"mediaType":"movie","tmdbId":10378,"status":3,"status4k":1},
         "requestedBy":{"id":1,"displayName":"admin"},"seasons":[]},
        {"id":2,"status":2,"type":"tv","is4k":false,"media":{"downloadStatus":[],"downloadStatus4k":[],"id":2,"mediaType":"tv","tmdbId":136315,"tvdbId":403294,"status":3,"status4k":1},
         "requestedBy":{"id":1,"displayName":"admin"},"seasons":[{"id":1,"seasonNumber":1,"status":2}]}
    ]}"#;

    fn title(name: &str) -> Title {
        Title { name: name.into(), poster: Some("/p.jpg".into()), backdrop: Some("/b.jpg".into()), year: Some("2008".into()), at: 0 }
    }

    #[test]
    fn requests_say_where_they_stand() {
        let page: Page = serde_json::from_str(TO_COME).unwrap();
        let [waiting, approved, show] = &page.results[..] else { panic!("three requests") };
        assert_eq!(sentence([waiting.seasons_text(), Some(waiting.state(true)), waiting.requester(false)]), "Waiting for approval, requested by sam");
        assert_eq!(sentence([approved.seasons_text(), Some(approved.state(true)), None]), "On its way");
        assert_eq!(sentence([show.seasons_text(), Some(show.state(false)), None]), "Season 1, on its way");
        let item = requested_item(show, "tv:136315", &title("The Bear"), false, 3);
        assert_eq!((item.id.as_str(), item.row.as_deref(), item.weight), ("requested:tv:136315", Some("requested"), 3));
        assert_eq!(item.subtitle.as_deref(), Some("Season 1, on its way"));
        assert_eq!(item.poster.as_deref(), Some("https://image.tmdb.org/t/p/w500/p.jpg"));
        assert_eq!(item.thumbnail.as_deref(), Some("https://image.tmdb.org/t/p/w1280/b.jpg"));
        assert_eq!((item.action.search.as_deref(), item.action.resolve.as_deref()), (Some("The Bear"), None));
    }

    #[test]
    fn a_download_says_how_far_it_has_got() {
        let r: Request = serde_json::from_str(
            r#"{"id":7,"status":2,"media":{"mediaType":"movie","tmdbId":1,"status":3,
                "downloadStatus":[{"mediaType":"movie","externalId":1,"size":1000,"sizeLeft":600,"status":"downloading","timeLeft":"00:10:00","title":"x","downloadId":"d"}]}}"#,
        )
        .unwrap();
        assert_eq!((r.state(false), r.state(true)), ("downloading, 40%".to_string(), "Downloading, 40%".to_string()));
        // A 4K request reads its own downloads.
        let four_k: Request = serde_json::from_str(r#"{"id":8,"status":2,"is4k":true,"media":{"mediaType":"movie","tmdbId":1,"status":5,"status4k":4,"downloadStatus":[{"size":10,"sizeLeft":5}]}}"#).unwrap();
        assert_eq!(four_k.state(false), "partly available");
        let failed: Request = serde_json::from_str(r#"{"id":9,"status":4,"media":{"mediaType":"tv","tmdbId":2,"status":2}}"#).unwrap();
        assert_eq!(failed.state(false), "the request failed");
        let seasons: Request = serde_json::from_str(r#"{"id":10,"status":2,"seasons":[{"seasonNumber":1},{"seasonNumber":2}],"media":{"mediaType":"tv","tmdbId":2,"status":3}}"#).unwrap();
        assert_eq!(seasons.seasons_text().as_deref(), Some("2 seasons"));
    }

    #[test]
    fn what_arrived_is_dated_and_fades_after_a_month() {
        let now = parse_iso8601("2026-11-11T12:00:00Z").unwrap();
        let r: Request = serde_json::from_str(r#"{"id":3,"status":5,"media":{"mediaType":"movie","tmdbId":133701,"status":5,"mediaAddedAt":"2026-11-10T11:11:04.000Z","updatedAt":"2026-11-11T11:11:04.000Z"},"requestedBy":{"id":1,"displayName":"admin"}}"#).unwrap();
        let item = available_item(&r, "movie:133701", &title("Tears of Steel"), false, 1, now);
        assert_eq!((item.id.as_str(), item.row.as_deref(), item.subtitle.as_deref()), ("available:movie:133701", Some("available"), Some("2008")));
        assert_eq!((item.added_ms, item.is_new), (parse_iso8601("2026-11-11T11:11:04.000Z"), true));
        // Seerr wrote mediaAddedAt when this was requested in October; it arrived in November.
        let late: Request = serde_json::from_str(r#"{"id":6,"status":5,"media":{"mediaType":"movie","tmdbId":7,"status":5,"mediaAddedAt":"2026-10-01T09:00:00.000Z","updatedAt":"2026-11-10T20:00:00.000Z"}}"#).unwrap();
        assert_eq!(late.arrived_ms(), parse_iso8601("2026-11-10T20:00:00.000Z"));
        assert!(recent(&late, now));
        assert!(recent(&r, now));
        let old: Request = serde_json::from_str(r#"{"id":4,"status":5,"media":{"mediaType":"movie","tmdbId":1,"status":5,"mediaAddedAt":"2026-09-01T00:00:00.000Z"}}"#).unwrap();
        assert!(!recent(&old, now));
        // A request whose arrival is unknown stays rather than vanishing.
        assert!(recent(&Request::default(), now));
    }

    #[test]
    fn titles_are_read_from_films_and_shows_alike() {
        let film: Details = serde_json::from_str(r#"{"id":10378,"title":"Big Buck Bunny","releaseDate":"2008-04-10","posterPath":"/i9j.jpg","backdropPath":"/xtd.jpg","runtime":8}"#).unwrap();
        assert_eq!((film.title.as_deref(), film.release_date.as_deref()), (Some("Big Buck Bunny"), Some("2008-04-10")));
        let show: Details = serde_json::from_str(r#"{"id":136315,"name":"The Bear","firstAirDate":"2022-06-23","posterPath":"/eKf.jpg","backdropPath":null}"#).unwrap();
        assert_eq!((show.name.as_deref(), show.backdrop_path), (Some("The Bear"), None));
        let kept = serde_json::to_string(&title("A")).unwrap();
        assert_eq!(serde_json::from_str::<Title>(&kept).unwrap(), title("A"));
        // An entry kept before titles were dated is due at once.
        let undated: Title = serde_json::from_str(r#"{"name":"A","poster":"/p.jpg","backdrop":null,"year":null}"#).unwrap();
        assert!(undated.stale(DAY_MS * 400));
        let now = DAY_MS * 400;
        let fresh = Title { at: now - DAY_MS * 2, ..title("A") };
        assert!(!fresh.stale(now), "a title with a poster waits a week");
        assert!(Title { poster: None, ..fresh.clone() }.stale(now), "one without a poster is asked for again after a day");
        assert!(Title { at: now - DAY_MS * 8, ..fresh }.stale(now));
        let r: Request = serde_json::from_str(r#"{"id":1,"media":{"mediaType":"tv","tmdbId":5}}"#).unwrap();
        assert_eq!(r.media.key().as_deref(), Some("tv:5"));
        assert_eq!(Request::default().media.key(), None);
    }

    #[test]
    fn sentences_start_with_a_capital() {
        let r: Request = serde_json::from_str(r#"{"id":1,"status":2,"media":{"mediaType":"movie","tmdbId":1,"status":3},"requestedBy":{"id":2,"displayName":" sam "}}"#).unwrap();
        assert_eq!(sentence([None, Some(r.state(true))]), "On its way");
        assert_eq!(sentence([Some("2008".into()), r.requester(false)]), "2008, requested by sam");
        assert_eq!(sentence([None, r.requester(true)]), "Requested by sam");
        assert_eq!(sentence([Some(String::new()), None]), "");
        // A title is asked for in the language in use, and each language keeps its own.
        assert_eq!((language_query(), titles_key()), (String::new(), "titles".to_string()));
        let shipped = [<Seerr as sdk::Plugin>::locales()];
        assert_eq!(sdk::words::in_language("pt-PT", &shipped, || (language_query(), titles_key())), ("?language=pt-PT".to_string(), "titles:pt-PT".to_string()));
    }

    #[test]
    fn the_manifest_asks_for_the_server_and_its_key() {
        let m = <Seerr as sdk::Plugin>::manifest();
        assert!(m.validate().is_ok(), "{:?}", m.validate());
        assert_eq!(m.settings.iter().filter(|s| s.required).map(|s| s.key.as_str()).collect::<Vec<_>>(), ["server", "api_key"]);
        assert!(m.fetch_hosts.iter().any(|h| h == "image.tmdb.org"), "posters come from TMDB");
        assert_eq!(<Seerr as sdk::Plugin>::refresh(&RefreshContext::default()).err().as_deref(), Some("Add the server address in the plugin's settings."));
        let req = session().request("/api/v1/request");
        assert_eq!(req.headers.get("X-Api-Key").map(String::as_str), Some("k"));
    }
}
