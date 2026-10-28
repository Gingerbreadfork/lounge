//! Tvheadend: channels become Live now with the guide behind them, finished recordings a
//! "Recordings" row, and the guide answers search. Everything plays in the TV's player straight
//! from the server. The API is read with the user's password, answering the server's Digest or
//! Basic challenge; streams and pictures, which the TV fetches on its own, carry the persistent
//! token from the user's Tvheadend settings.
use lounge_plugin_sdk::{self as sdk, host, Action, HttpRequest, Item, ItemKind, Kind, Manifest, RefreshContext, RowSpec, RowStyle, SettingKind, SettingSpec};
use md5::{Digest, Md5};
use serde::Deserialize;
use std::cell::RefCell;

pub struct Tvheadend;

const PAGE: usize = 1000;
const MAX_PAGES: usize = 8;

impl sdk::Plugin for Tvheadend {
    fn manifest() -> Manifest {
        Manifest {
            id: "tv.lounge.tvheadend".into(),
            name: "Tvheadend".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            description: "Live channels, the guide and your recordings from a Tvheadend server.".into(),
            publisher_name: "Lounge".into(),
            kinds: vec![Kind::Content, Kind::Epg, Kind::Search],
            fetch_hosts: vec!["$server".into(), "*".into()],
            refresh_minutes: 30,
            rows: vec![RowSpec { id: "recordings".into(), title: "Recordings".into(), style: RowStyle::Landscape }],
            settings: vec![
                SettingSpec { key: "server".into(), label: "Server address".into(), kind: SettingKind::Url, required: true, help: Some("Like http://192.168.0.10:9981".into()), default: None },
                SettingSpec { key: "username".into(), label: "Username".into(), kind: SettingKind::Text, required: false, help: Some("Leave both empty if the server allows anonymous access.".into()), default: None },
                SettingSpec { key: "password".into(), label: "Password".into(), kind: SettingKind::Password, required: false, help: None, default: None },
                SettingSpec {
                    key: "token".into(),
                    label: "Persistent auth token".into(),
                    kind: SettingKind::Password,
                    required: false,
                    help: Some("Lets the TV play streams without a password. In Tvheadend go to Configuration, Users, Access entries, open your user and turn on Persistent authentication; the token appears there.".into()),
                    default: None,
                },
                SettingSpec { key: "profile".into(), label: "Stream profile".into(), kind: SettingKind::Text, required: false, help: Some("A profile from Configuration, Stream. pass sends the channel as is.".into()), default: Some("pass".into()) },
                SettingSpec { key: "tag".into(), label: "Only channels tagged".into(), kind: SettingKind::Text, required: false, help: Some("A channel tag name such as TV. Empty means every enabled channel.".into()), default: None },
            ],
            homepage: Some("https://tvheadend.org".into()),
        }
    }

    fn refresh(ctx: &RefreshContext) -> Result<Vec<Item>, String> {
        let s = Server::from_settings()?;
        let mut channels = s.channels()?;
        if channels.is_empty() {
            return Err("Tvheadend has no enabled channels. Map some services to channels first.".into());
        }
        if channels.len() > sdk::MAX_ITEMS {
            sdk::warn!("{} channels, of which the TV shows the first {}; choose a channel tag in the settings", channels.len(), sdk::MAX_ITEMS);
            channels.truncate(sdk::MAX_ITEMS);
        }
        let mut items: Vec<Item> = channels.iter().map(|c| s.channel_item(c)).collect();
        match s.get_json::<Grid<Recording>>("/api/dvr/entry/grid_finished?limit=40&sort=start&dir=desc") {
            Ok(grid) => {
                let count = grid.entries.len();
                items.extend(grid.entries.iter().enumerate().map(|(i, r)| s.recording_item(r, (count - i) as i32)));
            }
            Err(e) => sdk::warn!("recordings unavailable: {e}"),
        }
        items.truncate(sdk::MAX_ITEMS);
        let recordings = items.len() - channels.len();

        let uuids: std::collections::HashSet<&str> = channels.iter().map(|c| c.uuid.as_str()).collect();
        let window_end = ctx.now_ms + i64::from(ctx.window_hours.max(1)) * 3_600_000;
        let mut schedule = sdk::ScheduleBudget::new(sdk::MAX_ITEMS - items.len());
        // The tag narrows the guide on the server, so its pages reach further ahead.
        let tag = s.tag.as_deref().map(|t| format!("&channelTag={}", url_encode(t))).unwrap_or_default();
        'pages: for page in 0..MAX_PAGES {
            let grid: Grid<Event> = match s.get_json(&format!("/api/epg/events/grid?start={}&limit={PAGE}&sort=start&dir=ASC{tag}", page * PAGE)) {
                Ok(grid) => grid,
                Err(e) => {
                    sdk::warn!("guide unavailable: {e}");
                    if page == 0 && sdk::last_guide_still_showing(ctx) {
                        return Err(sdk::GUIDE_KEPT.into());
                    }
                    break;
                }
            };
            let got = grid.entries.len();
            for e in grid.entries {
                if e.start * 1000 >= window_end {
                    break 'pages;
                }
                if e.stop * 1000 > ctx.now_ms && uuids.contains(e.channel_uuid.as_str()) {
                    schedule.push(s.event_item(&e, None));
                }
            }
            if got < PAGE {
                break;
            }
        }
        let schedule = schedule.finish();
        sdk::info!("{} channels, {} programmes, {recordings} recordings", channels.len(), schedule.len());
        items.extend(schedule);
        Ok(items)
    }

    fn search(query: &str, limit: u32) -> Result<Vec<Item>, String> {
        let s = Server::from_settings()?;
        let tag = s.tag.as_deref().map(|t| format!("&channelTag={}", url_encode(t))).unwrap_or_default();
        let grid: Grid<Event> = s.get_json(&format!("/api/epg/events/grid?title={}&limit=200&sort=start&dir=ASC{tag}", url_encode(query)))?;
        let now = host::now_ms();
        Ok(first_airings(grid.entries, limit.clamp(1, 40) as usize).iter().map(|e| s.search_item(e, now)).collect())
    }
}

sdk::export_plugin!(Tvheadend);

struct Server {
    base: String,
    /// User name and password for the API.
    login: Option<(String, String)>,
    /// How the server last asked to be answered; later requests answer it straight away.
    auth: RefCell<Option<Auth>>,
    url_auth: String,
    profile: String,
    tag: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
enum Auth {
    Basic,
    Digest(Challenge),
}

/// A Digest challenge (RFC 7616) the plugin can answer: MD5 or MD5-sess, with or without
/// `qop=auth`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Challenge {
    pub realm: String,
    pub nonce: String,
    pub opaque: Option<String>,
    pub qop_auth: bool,
    pub sess: bool,
    /// Requests answered with this nonce so far.
    pub count: u32,
}

/// What a `WWW-Authenticate` header asks for: Digest when it is offered with an algorithm the
/// plugin speaks, else Basic, else nothing. An unsupported Digest algorithm is returned as the
/// error.
fn read_challenge(header: &str) -> Result<Option<Auth>, String> {
    let mut basic = false;
    let mut unsupported = None;
    for (scheme, params) in challenges(header) {
        if scheme.eq_ignore_ascii_case("basic") {
            basic = true;
        } else if scheme.eq_ignore_ascii_case("digest") {
            let get = |k: &str| params.iter().find(|(key, _)| key.eq_ignore_ascii_case(k)).map(|(_, v)| v.clone());
            let algorithm = get("algorithm").unwrap_or_else(|| "MD5".into());
            let sess = algorithm.eq_ignore_ascii_case("MD5-sess");
            if !sess && !algorithm.eq_ignore_ascii_case("MD5") {
                unsupported = Some(algorithm);
                continue;
            }
            let qop = get("qop");
            if qop.as_deref().is_some_and(|q| !q.split(',').any(|o| o.trim().eq_ignore_ascii_case("auth"))) {
                unsupported = Some(format!("qop={}", qop.unwrap_or_default()));
                continue;
            }
            let Some(nonce) = get("nonce") else { continue };
            let challenge = Challenge { realm: get("realm").unwrap_or_default(), nonce, opaque: get("opaque"), qop_auth: qop.is_some(), sess, count: 0 };
            return Ok(Some(Auth::Digest(challenge)));
        }
    }
    match (basic, unsupported) {
        (true, _) => Ok(Some(Auth::Basic)),
        (false, Some(what)) => Err(format!("Tvheadend asks for Digest sign-in with {what}, which the plugin cannot answer. In Tvheadend's general settings, set the digest hash type to MD5.")),
        (false, None) => Ok(None),
    }
}

/// The challenges in a `WWW-Authenticate` value: each scheme with its parameters. A token not
/// followed by `=` starts the next challenge.
fn challenges(header: &str) -> Vec<(String, Vec<(String, String)>)> {
    let mut out: Vec<(String, Vec<(String, String)>)> = Vec::new();
    let b = header.as_bytes();
    let mut i = 0;
    while i < b.len() {
        while i < b.len() && (b[i].is_ascii_whitespace() || b[i] == b',') {
            i += 1;
        }
        let start = i;
        while i < b.len() && !b[i].is_ascii_whitespace() && b[i] != b'=' && b[i] != b',' {
            i += 1;
        }
        let token = header[start..i].to_string();
        let mut j = i;
        while j < b.len() && b[j].is_ascii_whitespace() {
            j += 1;
        }
        if token.is_empty() {
            i += 1;
            continue;
        }
        if j >= b.len() || b[j] != b'=' {
            out.push((token, Vec::new()));
            continue;
        }
        j += 1;
        while j < b.len() && b[j].is_ascii_whitespace() {
            j += 1;
        }
        let mut value = String::new();
        if j < b.len() && b[j] == b'"' {
            j += 1;
            let mut escaped = false;
            for c in header[j..].chars() {
                j += c.len_utf8();
                match (c, escaped) {
                    (_, true) => {
                        value.push(c);
                        escaped = false;
                    }
                    ('\\', false) => escaped = true,
                    ('"', false) => break,
                    _ => value.push(c),
                }
            }
        } else {
            let end = header[j..].find(',').map_or(header.len(), |e| j + e);
            value = header[j..end].trim().to_string();
            j = end;
        }
        i = j;
        match out.last_mut() {
            Some((_, params)) => params.push((token, value)),
            None => out.push((String::new(), vec![(token, value)])),
        }
    }
    out
}

fn md5_hex(s: &str) -> String {
    Md5::digest(s.as_bytes()).iter().map(|b| format!("{b:02x}")).collect()
}

fn quoted(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

impl Challenge {
    /// The `Authorization` value answering this challenge for one request.
    pub fn answer(&self, username: &str, password: &str, method: &str, uri: &str, cnonce: &str) -> String {
        let nc = format!("{:08x}", self.count.max(1));
        let mut ha1 = md5_hex(&format!("{username}:{}:{password}", self.realm));
        if self.sess {
            ha1 = md5_hex(&format!("{ha1}:{}:{cnonce}", self.nonce));
        }
        let ha2 = md5_hex(&format!("{method}:{uri}"));
        let response = if self.qop_auth { md5_hex(&format!("{ha1}:{}:{nc}:{cnonce}:auth:{ha2}", self.nonce)) } else { md5_hex(&format!("{ha1}:{}:{ha2}", self.nonce)) };
        let mut out = format!(
            "Digest username={}, realm={}, nonce={}, uri={}, algorithm={}, response=\"{response}\"",
            quoted(username),
            quoted(&self.realm),
            quoted(&self.nonce),
            quoted(uri),
            if self.sess { "MD5-sess" } else { "MD5" }
        );
        if self.qop_auth {
            out.push_str(&format!(", qop=auth, nc={nc}, cnonce={}", quoted(cnonce)));
        }
        if let Some(opaque) = &self.opaque {
            out.push_str(&format!(", opaque={}", quoted(opaque)));
        }
        out
    }
}

/// The path and query of `url`, which Digest signs.
fn request_target(url: &str) -> &str {
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    rest.find('/').map_or("/", |i| &rest[i..])
}

#[derive(Deserialize, Default)]
struct Grid<T> {
    #[serde(default)]
    entries: Vec<T>,
}

#[derive(Deserialize, Default, Clone, Debug)]
pub struct Channel {
    #[serde(default)]
    pub uuid: String,
    #[serde(default)]
    pub name: String,
    #[serde(default, deserialize_with = "channel_number")]
    pub number: Option<String>,
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub icon_public_url: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
}

/// A channel number as Tvheadend sends it: an integer, or a string such as "7.1" for a subchannel.
/// Zero and anything else count as no number, so one odd channel cannot fail the whole list.
fn channel_number<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Number {
        Int(i64),
        Float(f64),
        Text(String),
        Other(serde::de::IgnoredAny),
    }
    Ok(match Option::<Number>::deserialize(d)? {
        Some(Number::Int(n)) if n > 0 => Some(n.to_string()),
        Some(Number::Float(n)) if n > 0.0 => Some(n.to_string()),
        Some(Number::Text(t)) if !matches!(t.trim(), "" | "0") => Some(t.trim().to_string()),
        _ => None,
    })
}

#[derive(Deserialize, Default, Clone, Debug)]
pub struct Event {
    #[serde(rename = "eventId", default)]
    pub event_id: i64,
    #[serde(rename = "channelUuid", default)]
    pub channel_uuid: String,
    #[serde(rename = "channelName")]
    pub channel_name: Option<String>,
    #[serde(rename = "channelNumber")]
    pub channel_number: Option<String>,
    #[serde(default)]
    pub start: i64,
    #[serde(default)]
    pub stop: i64,
    #[serde(default)]
    pub title: String,
    pub subtitle: Option<String>,
    pub summary: Option<String>,
    pub description: Option<String>,
    pub image: Option<String>,
    #[serde(rename = "seasonNumber")]
    pub season_number: Option<i64>,
    #[serde(rename = "episodeNumber")]
    pub episode_number: Option<i64>,
}

#[derive(Deserialize, Default, Clone, Debug)]
pub struct Recording {
    #[serde(default)]
    pub uuid: String,
    #[serde(default)]
    pub disp_title: String,
    pub disp_subtitle: Option<String>,
    pub disp_description: Option<String>,
    #[serde(default)]
    pub start: i64,
    #[serde(default)]
    pub stop: i64,
    pub channelname: Option<String>,
    pub channel_icon: Option<String>,
    pub image: Option<String>,
    pub url: Option<String>,
}

impl Server {
    fn from_settings() -> Result<Server, String> {
        let base = host::server_setting("server", "Server address")?;
        let username = host::setting("username").map(|u| u.trim().to_string()).filter(|u| !u.is_empty());
        let password = host::secret("password").unwrap_or_default();
        let token = host::secret("token").map(|t| t.trim().to_string()).filter(|t| !t.is_empty());
        let url_auth = match (&token, &username) {
            (Some(t), _) => format!("auth={t}"),
            (None, Some(_)) => {
                sdk::warn!("no persistent auth token: streams and pictures may be refused by the server");
                String::new()
            }
            (None, None) => String::new(),
        };
        let profile = host::setting("profile").map(|p| p.trim().to_string()).filter(|p| !p.is_empty()).unwrap_or_else(|| "pass".into());
        let tag = host::setting("tag").map(|t| t.trim().to_string()).filter(|t| !t.is_empty());
        Ok(Server { base, login: username.map(|u| (u, password)), auth: RefCell::new(None), url_auth, profile, tag })
    }

    fn request(&self, url: &str) -> HttpRequest {
        let req = HttpRequest::get(url).header("Accept", "application/json");
        let mut auth = self.auth.borrow_mut();
        let (Some((user, password)), Some(auth)) = (&self.login, auth.as_mut()) else { return req };
        let value = match auth {
            Auth::Basic => format!("Basic {}", base64(format!("{user}:{password}").as_bytes())),
            Auth::Digest(c) => {
                c.count += 1;
                let cnonce = &md5_hex(&format!("{}:{}:{}", host::now_ms(), c.count, c.nonce))[..16];
                c.answer(user, password, "GET", request_target(url), cnonce)
            }
        };
        req.header("Authorization", &value)
    }

    fn get_json<T: serde::de::DeserializeOwned>(&self, path: &str) -> Result<T, String> {
        let url = format!("{}{}", self.base, path);
        let mut r = host::fetch(&self.request(&url))?;
        if r.status == 401 && self.login.is_some() {
            let asked = read_challenge(r.headers.get("www-authenticate").map(String::as_str).unwrap_or(""))?;
            if asked.is_some() {
                *self.auth.borrow_mut() = asked;
                r = host::fetch(&self.request(&url))?;
            }
        }
        match r.status {
            401 => return Err(self.rejected()),
            403 => return Err("Tvheadend refused this user (HTTP 403). Check its access entry allows the web interface.".into()),
            s if !(200..300).contains(&s) => return Err(format!("Tvheadend answered HTTP {s} for {}", path.split('?').next().unwrap_or(path))),
            _ => {}
        }
        serde_json::from_str(&r.body).map_err(|e| format!("unexpected answer for {}: {e}", path.split('?').next().unwrap_or(path)))
    }

    fn rejected(&self) -> String {
        match (&self.login, &*self.auth.borrow()) {
            (None, _) => "Tvheadend wants a username and password: enter them in the plugin's settings".into(),
            (Some(_), Some(Auth::Digest(_))) => "Tvheadend rejected the username or password (tried HTTP Digest)".into(),
            (Some(_), Some(Auth::Basic)) => "Tvheadend rejected the username or password (tried HTTP Basic)".into(),
            (Some(_), None) => "Tvheadend refused the request without saying how to sign in".into(),
        }
    }

    fn channels(&self) -> Result<Vec<Channel>, String> {
        let grid: Grid<Channel> = self.get_json("/api/channel/grid?limit=2000&sort=number&dir=asc")?;
        let mut tag_uuid = None;
        if let Some(name) = &self.tag {
            let tags: Grid<Tag> = self.get_json("/api/channeltag/grid?limit=500")?;
            tag_uuid = tags.entries.into_iter().find(|t| t.name.eq_ignore_ascii_case(name)).map(|t| t.uuid);
            if tag_uuid.is_none() {
                return Err(format!("Tvheadend has no channel tag called \"{name}\""));
            }
        }
        Ok(grid.entries.into_iter().filter(|c| c.enabled != Some(false) && !c.name.is_empty()).filter(|c| tag_uuid.as_ref().map(|t| c.tags.contains(t)).unwrap_or(true)).collect())
    }

    /// An address the TV can fetch on its own, with the token on the end.
    fn public_url(&self, path: &str) -> String {
        let path = path.trim_start_matches('/');
        let joiner = if path.contains('?') { '&' } else { '?' };
        if self.url_auth.is_empty() { format!("{}/{path}", self.base) } else { format!("{}/{path}{joiner}{}", self.base, self.url_auth) }
    }

    fn picture(&self, p: &Option<String>) -> Option<String> {
        p.as_deref().filter(|p| !p.is_empty()).map(|p| if p.starts_with("http") { p.to_string() } else { self.public_url(p) })
    }

    fn channel_item(&self, c: &Channel) -> Item {
        Item {
            id: format!("ch:{}", c.uuid),
            kind: ItemKind::Channel,
            title: c.name.clone(),
            channel_id: Some(c.uuid.clone()),
            channel_name: Some(c.name.clone()),
            channel_number: c.number.clone(),
            logo: self.picture(&c.icon_public_url),
            action: Action::play(self.public_url(&format!("stream/channel/{}?profile={}", c.uuid, self.profile)), c.name.clone()),
            ..Default::default()
        }
    }

    fn event_item(&self, e: &Event, kind: Option<ItemKind>) -> Item {
        let is_search = kind.is_some();
        Item {
            id: format!("prog:{}:{}", e.channel_uuid, e.start),
            kind: kind.unwrap_or(ItemKind::Schedule),
            title: e.title.clone(),
            subtitle: e.subtitle.clone().or_else(|| e.summary.clone()).or_else(|| e.description.as_deref().map(first_sentence)).or_else(|| if is_search { e.channel_name.clone() } else { None }),
            season: e.season_number.map(|n| n.to_string()),
            episode: e.episode_number.map(|n| n.to_string()),
            duration_ms: Some((e.stop - e.start) * 1000),
            channel_id: Some(e.channel_uuid.clone()),
            channel_name: e.channel_name.clone(),
            channel_number: e.channel_number.clone(),
            start_ms: Some(e.start * 1000),
            end_ms: Some(e.stop * 1000),
            thumbnail: self.picture(&e.image),
            action: if is_search { Action::play(self.public_url(&format!("stream/channel/{}?profile={}", e.channel_uuid, self.profile)), e.title.clone()) } else { Action::default() },
            ..Default::default()
        }
    }

    /// A search result. Choosing it tunes the channel to whatever is on now, so it says which
    /// channel and when the programme airs.
    fn search_item(&self, e: &Event, now_ms: i64) -> Item {
        let mut item = self.event_item(e, Some(ItemKind::Program));
        let when = airs(e.start * 1000, now_ms);
        item.subtitle = Some(match e.channel_name.as_deref().filter(|c| !c.is_empty()) {
            Some(c) => format!("{when}, {c}"),
            None => when,
        });
        item
    }

    fn recording_item(&self, r: &Recording, weight: i32) -> Item {
        let path = r.url.clone().filter(|u| !u.is_empty()).unwrap_or_else(|| format!("dvrfile/{}", r.uuid));
        Item {
            id: format!("rec:{}", r.uuid),
            kind: ItemKind::Program,
            title: r.disp_title.clone(),
            subtitle: r.disp_subtitle.clone().filter(|s| !s.is_empty()).or_else(|| r.channelname.clone()),
            duration_ms: Some((r.stop - r.start) * 1000),
            added_ms: Some(r.stop * 1000),
            channel_name: r.channelname.clone(),
            logo: self.picture(&r.channel_icon),
            thumbnail: self.picture(&r.image),
            row: Some("recordings".into()),
            weight,
            action: Action::play(self.public_url(&path), r.disp_title.clone()),
            ..Default::default()
        }
    }
}

#[derive(Deserialize, Default)]
struct Tag {
    #[serde(default)]
    uuid: String,
    #[serde(default)]
    name: String,
}

/// The earliest airing of each programme on each channel, from events sorted by start.
fn first_airings(events: Vec<Event>, limit: usize) -> Vec<Event> {
    let mut seen = std::collections::HashSet::new();
    let mut out: Vec<Event> = events.into_iter().filter(|e| seen.insert((e.title.to_lowercase(), e.channel_uuid.clone()))).collect();
    out.truncate(limit);
    out
}

/// "On now", "Starts in 20 minutes", "Starts in 3 h", "Starts in 2 days".
fn airs(start_ms: i64, now_ms: i64) -> String {
    let minutes = (start_ms - now_ms + 59_999) / 60_000;
    match minutes {
        ..=0 => "On now".into(),
        1 => "Starts in 1 minute".into(),
        2..=59 => format!("Starts in {minutes} minutes"),
        60..=1_439 => format!("Starts in {} h", minutes / 60),
        1_440..=2_879 => "Starts in 1 day".into(),
        _ => format!("Starts in {} days", minutes / 1_440),
    }
}

fn first_sentence(s: &str) -> String {
    let end = s.find(". ").map(|i| i + 1).unwrap_or(s.len()).min(120);
    let cut = s.char_indices().take_while(|(i, _)| *i < end).map(|(i, c)| i + c.len_utf8()).last().unwrap_or(0);
    s[..cut].trim().to_string()
}

pub fn base64(bytes: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = chunk.iter().enumerate().fold(0u32, |acc, (i, b)| acc | (u32::from(*b) << (16 - 8 * i)));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(T[((n >> (18 - 6 * i)) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

fn url_encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_keeps_each_programmes_first_airing_and_says_when() {
        let ev = |title: &str, channel: &str, start: i64| -> Event {
            serde_json::from_value(serde_json::json!({"channelUuid": channel, "channelName": "BBC One", "title": title, "start": start, "stop": start + 1800})).unwrap()
        };
        let kept = first_airings(vec![ev("News", "c1", 100), ev("news", "c1", 200), ev("News", "c2", 300), ev("Quiz", "c1", 400)], 10);
        assert_eq!(kept.iter().map(|e| (e.channel_uuid.as_str(), e.start)).collect::<Vec<_>>(), [("c1", 100), ("c2", 300), ("c1", 400)]);
        assert_eq!(airs(1_000, 5_000), "On now");
        assert_eq!(airs(20 * 60_000, 0), "Starts in 20 minutes");
        assert_eq!(airs(3 * 3_600_000 + 60_000, 0), "Starts in 3 h");
        assert_eq!(airs(3 * 86_400_000, 0), "Starts in 3 days");
        let item = server().search_item(&ev("News", "c1", 0), 0);
        assert_eq!(item.subtitle.as_deref(), Some("On now, BBC One"));
    }

    fn server() -> Server {
        Server { base: "http://tvh:9981".into(), login: Some(("me".into(), "pw".into())), auth: RefCell::new(None), url_auth: "auth=tok".into(), profile: "pass".into(), tag: None }
    }

    #[test]
    fn base64_matches_the_standard() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"me:pw"), "bWU6cHc=");
    }

    #[test]
    fn subchannel_numbers_keep_their_minor_part() {
        let grid: Grid<Channel> = serde_json::from_str(
            r#"{"entries": [
                {"uuid": "a", "name": "KQED", "number": "7.1"},
                {"uuid": "b", "name": "Two", "number": 2},
                {"uuid": "c", "name": "Unnumbered", "number": 0},
                {"uuid": "d", "name": "Missing"},
                {"uuid": "e", "name": "Odd", "number": true}]}"#,
        )
        .unwrap();
        let numbers: Vec<Option<&str>> = grid.entries.iter().map(|c| c.number.as_deref()).collect();
        assert_eq!(numbers, vec![Some("7.1"), Some("2"), None, None, None]);
    }

    #[test]
    fn channels_and_events_turn_into_items() {
        let grid: Grid<Channel> = serde_json::from_str(r#"{"entries": [{"uuid": "u1", "name": "BBC One", "number": 1, "enabled": true, "icon_public_url": "imagecache/5", "tags": ["t1"]}], "total": 1}"#).unwrap();
        let s = server();
        let ch = s.channel_item(&grid.entries[0]);
        assert_eq!(ch.kind, ItemKind::Channel);
        assert_eq!(ch.channel_number.as_deref(), Some("1"));
        assert_eq!(ch.logo.as_deref(), Some("http://tvh:9981/imagecache/5?auth=tok"));
        assert_eq!(ch.action.play.as_deref(), Some("http://tvh:9981/stream/channel/u1?profile=pass&auth=tok"));
        let events: Grid<Event> = serde_json::from_str(r#"{"entries": [{"eventId": 7, "channelUuid": "u1", "channelName": "BBC One", "channelNumber": "1", "start": 1760800000, "stop": 1760803600, "title": "News", "summary": "Tonight.", "seasonNumber": 3, "episodeNumber": 9, "image": "https://img/news.jpg"}]}"#).unwrap();
        let e = s.event_item(&events.entries[0], None);
        assert_eq!(e.kind, ItemKind::Schedule);
        assert_eq!((e.start_ms, e.end_ms), (Some(1760800000000), Some(1760803600000)));
        assert_eq!((e.season.as_deref(), e.episode.as_deref()), (Some("3"), Some("9")));
        assert_eq!(e.subtitle.as_deref(), Some("Tonight."));
        assert!(e.action.is_empty());
        let found = s.event_item(&events.entries[0], Some(ItemKind::Program));
        assert_eq!(found.kind, ItemKind::Program);
        assert!(found.action.play.is_some());
    }

    #[test]
    fn recordings_play_the_dvr_file() {
        let grid: Grid<Recording> = serde_json::from_str(r#"{"entries": [{"uuid": "r1", "disp_title": "Film", "disp_subtitle": "", "start": 1760800000, "stop": 1760807200, "channelname": "BBC Two", "url": "dvrfile/r1"}]}"#).unwrap();
        let r = server().recording_item(&grid.entries[0], 1);
        assert_eq!(r.subtitle.as_deref(), Some("BBC Two"));
        assert_eq!(r.duration_ms, Some(7_200_000));
        assert_eq!(r.action.play.as_deref(), Some("http://tvh:9981/dvrfile/r1?auth=tok"));
        assert_eq!(r.row.as_deref(), Some("recordings"));
    }

    fn digest(header: &str) -> Challenge {
        match read_challenge(header) {
            Ok(Some(Auth::Digest(c))) => c,
            other => panic!("{header}: {other:?}"),
        }
    }

    #[test]
    fn digest_answers_match_the_rfc_examples() {
        let mut c = digest(r#"Digest realm="testrealm@host.com", qop="auth,auth-int", nonce="dcd98b7102dd2f0e8b11d0f600bfb0c093", opaque="5ccc069c403ebaf9f0171e9517f40e41""#);
        c.count = 1;
        let answer = c.answer("Mufasa", "Circle Of Life", "GET", "/dir/index.html", "0a4f113b");
        assert!(answer.contains(r#"response="6629fae49393a05397450978507c4ef1""#), "{answer}");
        assert!(answer.contains(r#"qop=auth, nc=00000001, cnonce="0a4f113b""#));
        assert!(answer.ends_with(r#"opaque="5ccc069c403ebaf9f0171e9517f40e41""#));

        let mut c = digest(r#"Digest realm="http-auth@example.org", qop="auth, auth-int", algorithm=SHA-256, nonce="7ypf/xlj9XXwfDPEoM4URrv/xwf94BcCAzFZH4GiTo0v", opaque="FQhe/qaU925kfnzjCev0ciny7QMkPqMAFRtzCUYo5tdS", Digest realm="http-auth@example.org", qop="auth, auth-int", algorithm=MD5, nonce="7ypf/xlj9XXwfDPEoM4URrv/xwf94BcCAzFZH4GiTo0v", opaque="FQhe/qaU925kfnzjCev0ciny7QMkPqMAFRtzCUYo5tdS""#);
        c.count = 1;
        let answer = c.answer("Mufasa", "Circle of Life", "GET", "/dir/index.html", "f2/wE4q74E6zIJEtWaHKaf5wv/H5QzzpXusqGemxURZJ");
        assert!(answer.contains(r#"response="8ca523f5e9506fed4657c9700eebdbec""#), "{answer}");
    }

    #[test]
    fn challenges_in_every_shape_tvheadend_sends() {
        let c = digest(r#"Digest realm="tvheadend", qop=auth, nonce="0123abcd", opaque="ffee""#);
        assert_eq!((c.realm.as_str(), c.nonce.as_str(), c.opaque.as_deref(), c.qop_auth, c.sess), ("tvheadend", "0123abcd", Some("ffee"), true, false));
        let legacy = digest(r#"Digest realm="tvheadend", nonce="n""#);
        assert!(!legacy.qop_auth);
        assert!(!legacy.answer("me", "pw", "GET", "/api", "c").contains("qop="));
        assert!(digest(r#"Digest realm="r", nonce="n", algorithm=MD5-sess, qop="auth""#).sess);
        assert_eq!(read_challenge(r#"Basic realm="tvheadend""#), Ok(Some(Auth::Basic)));
        assert!(matches!(read_challenge(r#"Basic realm="x", Digest realm="x", nonce="n""#), Ok(Some(Auth::Digest(_)))));
        assert_eq!(read_challenge(""), Ok(None));
        assert!(read_challenge(r#"Digest realm="tvheadend", qop=auth, algorithm=SHA-256, nonce="n""#).unwrap_err().contains("SHA-256"));
        assert_eq!(read_challenge(r#"Digest realm="x", algorithm=SHA-256, nonce="n", Basic realm="x""#), Ok(Some(Auth::Basic)));
        assert_eq!(digest(r#"Digest realm="a \"b\" c", nonce="n,1""#).realm, r#"a "b" c"#);
    }

    #[test]
    fn requests_answer_the_last_challenge() {
        let s = server();
        assert!(!s.request("http://tvh:9981/api/x").headers.contains_key("Authorization"));
        *s.auth.borrow_mut() = Some(Auth::Basic);
        assert_eq!(s.request("http://tvh:9981/api/x").headers["Authorization"], format!("Basic {}", base64(b"me:pw")));
        *s.auth.borrow_mut() = read_challenge(r#"Digest realm="tvheadend", qop=auth, nonce="abc""#).unwrap();
        let first = s.request("http://tvh:9981/tvh/api/epg/events/grid?start=0&limit=1000").headers["Authorization"].clone();
        let second = s.request("http://tvh:9981/api/x").headers["Authorization"].clone();
        assert!(first.contains(r#"uri="/tvh/api/epg/events/grid?start=0&limit=1000""#) && first.contains("nc=00000001"), "{first}");
        assert!(second.contains("nc=00000002"));
        assert_eq!(s.rejected(), "Tvheadend rejected the username or password (tried HTTP Digest)");
        *s.auth.borrow_mut() = Some(Auth::Basic);
        assert_eq!(s.rejected(), "Tvheadend rejected the username or password (tried HTTP Basic)");
        assert_eq!(request_target("http://host"), "/");
    }

    #[test]
    fn public_urls_work_without_a_token() {
        let s = Server { url_auth: String::new(), ..server() };
        assert_eq!(s.public_url("/dvrfile/r1"), "http://tvh:9981/dvrfile/r1");
    }
}
