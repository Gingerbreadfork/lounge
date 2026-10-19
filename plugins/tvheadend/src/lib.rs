//! Tvheadend: channels become Live now with the guide behind them, finished recordings a
//! "Recordings" row, and the guide answers search. Everything plays in the TV's player straight
//! from the server. The API is read with the user's password; streams and pictures, which the
//! TV fetches on its own, carry the persistent token from the user's Tvheadend settings.
use lounge_plugin_sdk::{self as sdk, host, Action, HttpRequest, Item, ItemKind, Kind, Manifest, RefreshContext, RowSpec, RowStyle, SettingKind, SettingSpec};
use serde::Deserialize;

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
            fetch_hosts: vec!["*".into()],
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
        let channels = s.channels()?;
        if channels.is_empty() {
            return Err("Tvheadend has no enabled channels. Map some services to channels first.".into());
        }
        let mut items: Vec<Item> = channels.iter().map(|c| s.channel_item(c)).collect();
        let window_end = ctx.now_ms + i64::from(ctx.window_hours.max(1)) * 3_600_000;
        let mut events = 0;
        'pages: for page in 0..MAX_PAGES {
            let grid: Grid<Event> = s.get_json(&format!("/api/epg/events/grid?start={}&limit={PAGE}", page * PAGE))?;
            let got = grid.entries.len();
            for e in grid.entries {
                let start = e.start * 1000;
                if start > window_end {
                    break 'pages;
                }
                if e.stop * 1000 < ctx.now_ms - 3_600_000 || !channels.iter().any(|c| c.uuid == e.channel_uuid) {
                    continue;
                }
                items.push(s.event_item(&e, None));
                events += 1;
            }
            if got < PAGE {
                break;
            }
        }
        match s.get_json::<Grid<Recording>>("/api/dvr/entry/grid_finished?limit=40&sort=start&dir=desc") {
            Ok(grid) => {
                let count = grid.entries.len();
                items.extend(grid.entries.iter().enumerate().map(|(i, r)| s.recording_item(r, (count - i) as i32)));
            }
            Err(e) => sdk::warn!("recordings unavailable: {e}"),
        }
        sdk::info!("{} channels, {events} programmes, {} recordings", channels.len(), items.len() - channels.len() - events);
        Ok(items)
    }

    fn search(query: &str, limit: u32) -> Result<Vec<Item>, String> {
        let s = Server::from_settings()?;
        let grid: Grid<Event> = s.get_json(&format!("/api/epg/events/grid?title={}&limit={}", url_encode(query), limit.clamp(1, 40)))?;
        Ok(grid.entries.iter().map(|e| s.event_item(e, Some(ItemKind::Program))).collect())
    }
}

sdk::export_plugin!(Tvheadend);

struct Server {
    base: String,
    auth_header: Option<String>,
    url_auth: String,
    profile: String,
    tag: Option<String>,
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
    #[serde(default)]
    pub number: Option<i64>,
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub icon_public_url: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
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
        let base = host::required_setting("server", "Server address")?.trim_end_matches('/').to_string();
        let username = host::setting("username").map(|u| u.trim().to_string()).filter(|u| !u.is_empty());
        let password = host::secret("password").unwrap_or_default();
        let token = host::secret("token").map(|t| t.trim().to_string()).filter(|t| !t.is_empty());
        let auth_header = username.as_ref().map(|u| format!("Basic {}", base64(format!("{u}:{password}").as_bytes())));
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
        Ok(Server { base, auth_header, url_auth, profile, tag })
    }

    fn get_json<T: serde::de::DeserializeOwned>(&self, path: &str) -> Result<T, String> {
        let mut req = HttpRequest::get(format!("{}{}", self.base, path)).header("Accept", "application/json");
        if let Some(h) = &self.auth_header {
            req = req.header("Authorization", h);
        }
        let r = host::fetch(&req)?;
        match r.status {
            401 | 403 => return Err("Tvheadend rejected the username or password".into()),
            s if !(200..300).contains(&s) => return Err(format!("Tvheadend answered HTTP {s} for {path}")),
            _ => {}
        }
        serde_json::from_str(&r.body).map_err(|e| format!("unexpected answer for {path}: {e}"))
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
            channel_number: c.number.filter(|n| *n > 0).map(|n| n.to_string()),
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

    fn server() -> Server {
        Server { base: "http://tvh:9981".into(), auth_header: Some(format!("Basic {}", base64(b"me:pw"))), url_auth: "auth=tok".into(), profile: "pass".into(), tag: None }
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

    #[test]
    fn public_urls_work_without_a_token() {
        let s = Server { url_auth: String::new(), ..server() };
        assert_eq!(s.public_url("/dvrfile/r1"), "http://tvh:9981/dvrfile/r1");
    }
}
