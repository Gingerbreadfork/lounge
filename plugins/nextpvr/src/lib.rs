//! NextPVR: channels become Live now with the guide behind them, half-watched recordings go
//! into Continue watching and the rest fill a "Recordings" row. The server wants a session
//! signed in with its PIN; the session is kept between refreshes, and play addresses are
//! worked out when something is selected so they always carry a live session.
use lounge_plugin_sdk::{self as sdk, host, Action, Item, ItemKind, Kind, Manifest, RefreshContext, RowSpec, RowStyle, SettingKind, SettingSpec};
use md5::{Digest, Md5};
use serde::{Deserialize, Serialize};

pub struct NextPvr;

const SESSION_KEY: &str = "session";
const CURSOR_KEY: &str = "cursor";
const LISTING_FETCHES: usize = 48;

impl sdk::Plugin for NextPvr {
    fn manifest() -> Manifest {
        Manifest {
            id: "tv.lounge.nextpvr".into(),
            name: "NextPVR".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            description: "Live channels, the guide and your recordings from a NextPVR server.".into(),
            publisher_name: "Lounge".into(),
            kinds: vec![Kind::Content, Kind::Epg],
            fetch_hosts: vec!["*".into()],
            refresh_minutes: 30,
            rows: vec![RowSpec { id: "recordings".into(), title: "Recordings".into(), style: RowStyle::Landscape }],
            settings: vec![
                SettingSpec { key: "server".into(), label: "Server address".into(), kind: SettingKind::Url, required: true, help: Some("Like http://192.168.0.10:8866".into()), default: None },
                SettingSpec { key: "pin".into(), label: "PIN".into(), kind: SettingKind::Password, required: false, help: Some("The server's PIN from Settings, General. New installs use 0000.".into()), default: Some("0000".into()) },
                SettingSpec { key: "recordings".into(), label: "Recordings to show".into(), kind: SettingKind::Number, required: false, help: Some("The newest ones.".into()), default: Some("30".into()) },
            ],
            homepage: Some("https://nextpvr.com".into()),
        }
    }

    fn refresh(ctx: &RefreshContext) -> Result<Vec<Item>, String> {
        let s = Server::connect()?;
        let channels: ChannelList = s.call("channel.list", "")?;
        let channels: Vec<Channel> = channels.channels.into_iter().filter(|c| !c.channel_name.is_empty()).collect();
        if channels.is_empty() {
            return Err("NextPVR has no channels yet. Scan for channels in its settings first.".into());
        }
        let mut items: Vec<Item> = channels.iter().map(|c| s.channel_item(c)).collect();
        let window_end = ctx.now_ms + i64::from(ctx.window_hours.max(1)) * 3_600_000;
        let programmes = s.listings(&channels, ctx.now_ms, window_end, &mut items);
        let mut recordings = 0;
        match s.call::<RecordingList>("recording.list", "&filter=ready") {
            Ok(list) => {
                let max = host::setting("recordings").and_then(|v| v.trim().parse::<usize>().ok()).unwrap_or(30).clamp(1, 100);
                let mut recs: Vec<Recording> = list.recordings.into_iter().filter(|r| r.status.as_deref().unwrap_or("ready") == "ready").collect();
                recs.sort_by_key(|r| std::cmp::Reverse(r.start_ms()));
                recs.truncate(max);
                let count = recs.len();
                for (i, r) in recs.iter().enumerate() {
                    items.push(s.recording_item(r, (count - i) as i32));
                    recordings += 1;
                }
            }
            Err(e) => sdk::warn!("recordings unavailable: {e}"),
        }
        sdk::info!("{} channels, {programmes} programmes, {recordings} recordings", channels.len());
        Ok(items)
    }

    fn resolve(action: &str) -> Result<Action, String> {
        let s = Server::connect()?;
        let (what, rest) = action.split_once(':').ok_or_else(|| format!("unknown action {action}"))?;
        let (id, title) = rest.split_once('|').unwrap_or((rest, rest));
        match what {
            "live" => Ok(Action::play(format!("{}/live?channel={id}&client=lounge&sid={}", s.base, s.sid), title)),
            "rec" => Ok(Action::play(format!("{}/live?recording={id}&client=lounge&sid={}", s.base, s.sid), title)),
            _ => Err(format!("unknown action {action}")),
        }
    }
}

sdk::export_plugin!(NextPvr);

struct Server {
    base: String,
    sid: String,
}

#[derive(Serialize, Deserialize)]
struct SavedSession {
    base: String,
    sid: String,
}

#[derive(Deserialize, Default)]
struct Status {
    #[serde(default)]
    stat: String,
    #[serde(default)]
    sid: Option<String>,
    #[serde(default)]
    salt: Option<String>,
    #[serde(default)]
    msg: Option<String>,
}

#[derive(Deserialize, Default)]
struct ChannelList {
    #[serde(default)]
    channels: Vec<Channel>,
}

#[derive(Deserialize, Default, Clone, Debug)]
pub struct Channel {
    #[serde(rename = "channelId", default)]
    pub channel_id: i64,
    #[serde(rename = "channelNumber", default)]
    pub channel_number: i64,
    #[serde(rename = "channelMinor", default)]
    pub channel_minor: i64,
    #[serde(rename = "channelName", default)]
    pub channel_name: String,
    #[serde(rename = "channelIcon", default)]
    pub channel_icon: bool,
}

impl Channel {
    fn number(&self) -> String {
        if self.channel_minor > 0 { format!("{}.{}", self.channel_number, self.channel_minor) } else { self.channel_number.to_string() }
    }
}

#[derive(Deserialize, Default)]
struct ListingList {
    #[serde(default)]
    listings: Vec<Listing>,
}

#[derive(Deserialize, Serialize, Default, Clone, Debug)]
pub struct Listing {
    #[serde(default)]
    pub id: i64,
    #[serde(default)]
    pub name: String,
    pub subtitle: Option<String>,
    pub description: Option<String>,
    #[serde(default)]
    pub start: i64,
    #[serde(default)]
    pub end: i64,
    pub season: Option<i64>,
    pub episode: Option<i64>,
}

#[derive(Deserialize, Default)]
struct RecordingList {
    #[serde(default)]
    recordings: Vec<Recording>,
}

#[derive(Deserialize, Default, Clone, Debug)]
pub struct Recording {
    #[serde(default)]
    pub id: i64,
    #[serde(default)]
    pub name: String,
    pub desc: Option<String>,
    pub subtitle: Option<String>,
    pub channel: Option<String>,
    #[serde(rename = "channelId")]
    pub channel_id: Option<i64>,
    pub status: Option<String>,
    #[serde(rename = "startTime")]
    pub start_time: Option<String>,
    #[serde(rename = "startTimeTicks")]
    pub start_time_ticks: Option<i64>,
    #[serde(rename = "durationSeconds")]
    pub duration_seconds: Option<i64>,
    #[serde(rename = "playbackPosition")]
    pub playback_position: Option<i64>,
    pub season: Option<i64>,
    pub episode: Option<i64>,
}

impl Recording {
    fn start_ms(&self) -> i64 {
        self.start_time_ticks.map(ms_from_any).or_else(|| self.start_time.as_deref().and_then(sdk::parse_iso8601)).unwrap_or(0)
    }
}

/// NextPVR gives times in seconds or milliseconds depending on the call; both land as milliseconds.
pub fn ms_from_any(t: i64) -> i64 {
    if t > 100_000_000_000 { t } else { t * 1000 }
}

impl Server {
    /// Reuses the saved session when the server still accepts it, otherwise signs in with the PIN.
    fn connect() -> Result<Server, String> {
        let base = host::required_setting("server", "Server address")?.trim_end_matches('/').to_string();
        if let Some(saved) = host::kv_get_string(SESSION_KEY).and_then(|s| serde_json::from_str::<SavedSession>(&s).ok()).filter(|s| s.base == base) {
            let s = Server { base: base.clone(), sid: saved.sid };
            if s.call::<Status>("session.verify", "").map(|st| st.stat == "ok").unwrap_or(false) {
                return Ok(s);
            }
        }
        let pin = host::secret("pin").map(|p| p.trim().to_string()).filter(|p| !p.is_empty()).unwrap_or_else(|| "0000".into());
        let init: Status = get_json(&format!("{base}/service?method=session.initiate&ver=1.0&device=lounge&format=json")).map_err(|e| format!("No NextPVR at {base}: {e}"))?;
        let (sid, salt) = match (init.sid, init.salt) {
            (Some(sid), Some(salt)) => (sid, salt),
            _ => return Err("NextPVR did not start a session".into()),
        };
        let digest = md5_hex(&format!(":{}:{salt}", md5_hex(&pin)));
        let login: Status = get_json(&format!("{base}/service?method=session.login&sid={sid}&md5={digest}&format=json"))?;
        if login.stat != "ok" {
            return Err(format!("NextPVR rejected the PIN{}", login.msg.map(|m| format!(": {m}")).unwrap_or_default()));
        }
        host::kv_set(SESSION_KEY, serde_json::to_string(&SavedSession { base: base.clone(), sid: sid.clone() }).unwrap_or_default().as_bytes());
        Ok(Server { base, sid })
    }

    fn call<T: serde::de::DeserializeOwned>(&self, method: &str, params: &str) -> Result<T, String> {
        get_json(&format!("{}/service?method={method}&sid={}{params}&format=json", self.base, self.sid))
    }

    /// The guide needs one call per channel, so each refresh takes the next slice of channels and
    /// keeps the rest from last time.
    fn listings(&self, channels: &[Channel], now_ms: i64, window_end: i64, items: &mut Vec<Item>) -> usize {
        let cursor = host::kv_get_string(CURSOR_KEY).and_then(|c| c.parse::<usize>().ok()).unwrap_or(0) % channels.len().max(1);
        let fresh: Vec<usize> = (0..channels.len().min(LISTING_FETCHES)).map(|i| (cursor + i) % channels.len()).collect();
        host::kv_set(CURSOR_KEY, ((cursor + fresh.len()) % channels.len()).to_string().as_bytes());
        let start_s = (now_ms - 3_600_000) / 1000 / 300 * 300;
        let end_s = window_end / 1000 / 300 * 300;
        let mut count = 0;
        for (i, c) in channels.iter().enumerate() {
            let key = format!("epg:{}", c.channel_id);
            let listings: Vec<Listing> = if fresh.contains(&i) {
                match self.call::<ListingList>("channel.listings", &format!("&channel_id={}&start={start_s}&end={end_s}", c.channel_id)) {
                    Ok(l) => {
                        host::kv_set(&key, serde_json::to_string(&l.listings).unwrap_or_default().as_bytes());
                        l.listings
                    }
                    Err(e) => {
                        sdk::warn!("{}: {e}", c.channel_name);
                        host::kv_get_string(&key).and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
                    }
                }
            } else {
                host::kv_get_string(&key).and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
            };
            for l in listings {
                let (start, end) = (ms_from_any(l.start), ms_from_any(l.end));
                if end < now_ms - 3_600_000 || start > window_end || l.name.is_empty() {
                    continue;
                }
                items.push(Item {
                    id: format!("prog:{}:{start}", c.channel_id),
                    kind: ItemKind::Schedule,
                    title: l.name.clone(),
                    subtitle: l.subtitle.clone().filter(|s| !s.is_empty()).or_else(|| l.description.as_deref().map(first_sentence)),
                    season: l.season.map(|n| n.to_string()),
                    episode: l.episode.map(|n| n.to_string()),
                    duration_ms: Some(end - start),
                    channel_id: Some(c.channel_id.to_string()),
                    start_ms: Some(start),
                    end_ms: Some(end),
                    ..Default::default()
                });
                count += 1;
            }
        }
        count
    }

    fn channel_item(&self, c: &Channel) -> Item {
        Item {
            id: format!("ch:{}", c.channel_id),
            kind: ItemKind::Channel,
            title: c.channel_name.clone(),
            channel_id: Some(c.channel_id.to_string()),
            channel_name: Some(c.channel_name.clone()),
            channel_number: Some(c.number()),
            logo: if c.channel_icon { Some(format!("{}/service?method=channel.icon&channel_id={}&sid={}", self.base, c.channel_id, self.sid)) } else { None },
            action: Action { resolve: Some(format!("live:{}|{}", c.number(), c.channel_name)), title: Some(c.channel_name.clone()), ..Default::default() },
            ..Default::default()
        }
    }

    fn recording_item(&self, r: &Recording, weight: i32) -> Item {
        let duration_ms = r.duration_seconds.map(|d| d * 1000);
        let position_ms = r.playback_position.map(|p| p * 1000).unwrap_or(0);
        let in_progress = match duration_ms {
            Some(d) if d > 0 => position_ms > 60_000 && (position_ms as f64) < d as f64 * 0.95,
            _ => false,
        };
        let mut item = Item {
            id: format!("rec:{}", r.id),
            kind: if in_progress { ItemKind::Resume } else { ItemKind::Program },
            title: r.name.clone(),
            subtitle: r.subtitle.clone().filter(|s| !s.is_empty()).or_else(|| r.desc.as_deref().map(first_sentence)).or_else(|| r.channel.clone()),
            season: r.season.map(|n| n.to_string()),
            episode: r.episode.map(|n| n.to_string()),
            duration_ms,
            added_ms: Some(r.start_ms()),
            channel_name: r.channel.clone(),
            thumbnail: Some(format!("{}/service?method=recording.artwork&recording_id={}&sid={}", self.base, r.id, self.sid)),
            action: Action { resolve: Some(format!("rec:{}|{}", r.id, r.name)), title: Some(r.name.clone()), ..Default::default() },
            ..Default::default()
        };
        if in_progress {
            item.position_ms = Some(position_ms);
            item.progress = duration_ms.map(|d| (position_ms as f32 / d as f32).clamp(0.0, 1.0));
            item.last_used_ms = Some(r.start_ms());
        } else {
            item.row = Some("recordings".into());
            item.weight = weight;
            item.is_new = position_ms == 0;
        }
        item
    }
}

fn get_json<T: serde::de::DeserializeOwned>(url: &str) -> Result<T, String> {
    let body = host::get_text(url)?;
    serde_json::from_str(&body).map_err(|e| format!("unexpected answer from NextPVR: {e}"))
}

pub fn md5_hex(s: &str) -> String {
    let digest = Md5::digest(s.as_bytes());
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

fn first_sentence(s: &str) -> String {
    let end = s.find(". ").map(|i| i + 1).unwrap_or(s.len()).min(120);
    let cut = s.char_indices().take_while(|(i, _)| *i < end).map(|(i, c)| i + c.len_utf8()).last().unwrap_or(0);
    s[..cut].trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn server() -> Server {
        Server { base: "http://npvr:8866".into(), sid: "S1".into() }
    }

    #[test]
    fn the_login_digest_matches_nextpvr() {
        assert_eq!(md5_hex("0000"), "4a7d1ed414474e4033ac29ccb8653d9b");
        assert_eq!(md5_hex(&format!(":{}:abc", md5_hex("0000"))), md5_hex(":4a7d1ed414474e4033ac29ccb8653d9b:abc"));
    }

    #[test]
    fn channels_resolve_to_live_streams() {
        let list: ChannelList = serde_json::from_str(r#"{"stat": "ok", "channels": [{"channelId": 3, "channelNumber": 7, "channelMinor": 1, "channelName": "ABC", "channelIcon": true}]}"#).unwrap();
        let item = server().channel_item(&list.channels[0]);
        assert_eq!(item.channel_number.as_deref(), Some("7.1"));
        assert_eq!(item.logo.as_deref(), Some("http://npvr:8866/service?method=channel.icon&channel_id=3&sid=S1"));
        assert_eq!(item.action.resolve.as_deref(), Some("live:7.1|ABC"));
    }

    #[test]
    fn listings_land_in_milliseconds_either_way() {
        assert_eq!(ms_from_any(1760800000), 1760800000000);
        assert_eq!(ms_from_any(1760800000000), 1760800000000);
        let l: ListingList = serde_json::from_str(r#"{"listings": [{"id": 1, "name": "News", "start": 1760800000000, "end": 1760803600000, "season": 2, "episode": 5, "description": "Tonight. More."}]}"#).unwrap();
        assert_eq!(l.listings[0].season, Some(2));
    }

    #[test]
    fn recordings_split_into_resume_and_row() {
        let list: RecordingList = serde_json::from_str(r#"{"recordings": [
            {"id": 10, "name": "Film", "status": "ready", "startTimeTicks": 1760700000, "durationSeconds": 7200, "playbackPosition": 1800, "channel": "BBC One"},
            {"id": 11, "name": "Show", "subtitle": "Pilot", "status": "ready", "startTime": "2026-10-17T20:00:00Z", "durationSeconds": 1800, "playbackPosition": 0}
        ]}"#).unwrap();
        let s = server();
        let a = s.recording_item(&list.recordings[0], 2);
        assert_eq!(a.kind, ItemKind::Resume);
        assert_eq!(a.progress, Some(0.25));
        assert_eq!(a.action.resolve.as_deref(), Some("rec:10|Film"));
        let b = s.recording_item(&list.recordings[1], 1);
        assert_eq!(b.kind, ItemKind::Program);
        assert_eq!(b.added_ms, sdk::parse_iso8601("2026-10-17T20:00:00Z"));
        assert!(b.is_new);
    }
}
