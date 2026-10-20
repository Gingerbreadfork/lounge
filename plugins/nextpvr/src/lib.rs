//! NextPVR: channels become Live now with the guide behind them, half-watched recordings go
//! into Continue watching and the rest fill a "Recordings" row. The server wants a session
//! signed in with its PIN; the session is kept between refreshes, and play addresses are
//! worked out when something is selected so they always carry a live session.
use lounge_plugin_sdk::{self as sdk, host, Action, Item, ItemKind, Kind, Manifest, RefreshContext, RowSpec, RowStyle, SettingKind, SettingSpec};
use md5::{Digest, Md5};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub struct NextPvr;

const SESSION_KEY: &str = "session";
const CURSOR_KEY: &str = "cursor";
const WATCHED_KEY: &str = "watched";
const LISTING_FETCHES: usize = 48;
/// The guide is kept between refreshes in this many store keys, channels spread over them.
const GUIDE_KEYS: i64 = 16;
/// Set once the per-channel guide keys of earlier versions have been removed.
const GUIDE_MOVED_KEY: &str = "guide_moved";
/// Largest guide value written to one key, under the host's 256 KB.
const GUIDE_KEY_BYTES: usize = 240 * 1024;

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
        let mut channels: Vec<Channel> = channels.channels.into_iter().filter(|c| !c.channel_name.is_empty()).collect();
        if channels.is_empty() {
            return Err("NextPVR has no channels yet. Scan for channels in its settings first.".into());
        }
        if channels.len() > sdk::MAX_ITEMS {
            sdk::warn!("{} channels, of which the TV shows the first {}", channels.len(), sdk::MAX_ITEMS);
            channels.truncate(sdk::MAX_ITEMS);
        }
        let mut items: Vec<Item> = channels.iter().map(|c| s.channel_item(c)).collect();
        match s.call::<RecordingList>("recording.list", "&filter=ready") {
            Ok(list) => {
                let max = host::setting("recordings").and_then(|v| v.trim().parse::<usize>().ok()).unwrap_or(30).clamp(1, 100);
                let mut recs: Vec<Recording> = list.recordings.into_iter().filter(|r| r.status.as_deref().unwrap_or("ready") == "ready").collect();
                recs.sort_by_key(|r| std::cmp::Reverse(r.start_ms()));
                recs.truncate(max);
                let mut watched = Watched::load();
                let count = recs.len();
                for (i, r) in recs.iter().enumerate() {
                    let mut item = s.recording_item(r, (count - i) as i32);
                    if item.kind == ItemKind::Resume {
                        item.last_used_ms = watched.observe(r.id, r.playback_position.unwrap_or(0), ctx.now_ms);
                    }
                    items.push(item);
                }
                watched.save();
            }
            Err(e) => sdk::warn!("recordings unavailable: {e}"),
        }
        items.truncate(sdk::MAX_ITEMS);
        let recordings = items.len() - channels.len();
        let window_end = ctx.now_ms + i64::from(ctx.window_hours.max(1)) * 3_600_000;
        let schedule = s.listings(&channels, ctx.now_ms, window_end, sdk::MAX_ITEMS - items.len());
        sdk::info!("{} channels, {} programmes, {recordings} recordings", channels.len(), schedule.len());
        items.extend(schedule);
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
        let init: Status = get_json(&format!("{base}/service?method=session.initiate&ver=1.0&device=lounge&format=json")).map_err(|e| format!("No NextPVR at {}: {e}", sdk::redact_url(&base)))?;
        let (sid, salt) = match (init.sid, init.salt) {
            (Some(sid), Some(salt)) => (sid, salt),
            _ => return Err("NextPVR did not start a session".into()),
        };
        let digest = md5_hex(&format!(":{}:{salt}", md5_hex(&pin)));
        let login: Status = get_json(&format!("{base}/service?method=session.login&sid={sid}&md5={digest}&format=json")).map_err(|e| format!("session.login: {e}"))?;
        if login.stat != "ok" {
            return Err(format!("NextPVR rejected the PIN{}", login.msg.map(|m| format!(": {m}")).unwrap_or_default()));
        }
        host::kv_set(SESSION_KEY, serde_json::to_string(&SavedSession { base: base.clone(), sid: sid.clone() }).unwrap_or_default().as_bytes());
        Ok(Server { base, sid })
    }

    fn call<T: serde::de::DeserializeOwned>(&self, method: &str, params: &str) -> Result<T, String> {
        get_json(&format!("{}/service?method={method}&sid={}{params}&format=json", self.base, self.sid)).map_err(|e| format!("{method}: {e}"))
    }

    /// The guide needs one call per channel, so each refresh takes the next slice of channels and
    /// keeps the rest from last time, in a fixed number of store keys.
    fn listings(&self, channels: &[Channel], now_ms: i64, window_end: i64, budget: usize) -> Vec<Item> {
        if host::kv_get(GUIDE_MOVED_KEY).is_none() {
            for c in channels {
                host::kv_delete(&format!("epg:{}", c.channel_id));
            }
            host::kv_set(GUIDE_MOVED_KEY, b"1");
        }
        let cursor = host::kv_get_string(CURSOR_KEY).and_then(|c| c.parse::<usize>().ok()).unwrap_or(0) % channels.len().max(1);
        let fresh: Vec<usize> = (0..channels.len().min(LISTING_FETCHES)).map(|i| (cursor + i) % channels.len()).collect();
        host::kv_set(CURSOR_KEY, ((cursor + fresh.len()) % channels.len()).to_string().as_bytes());
        let start_s = (now_ms - 3_600_000) / 1000 / 300 * 300;
        let end_s = window_end / 1000 / 300 * 300;
        let mut cache = GuideCache::load();
        for &i in &fresh {
            let c = &channels[i];
            match self.call::<ListingList>("channel.listings", &format!("&channel_id={}&start={start_s}&end={end_s}", c.channel_id)) {
                Ok(l) => cache.put(c.channel_id, l.listings.iter().filter_map(Cached::from_listing).filter(|l| l.end > now_ms).collect()),
                Err(e) => sdk::warn!("{}: {e}", c.channel_name),
            }
        }
        cache.save(channels, now_ms);
        let mut schedule = sdk::ScheduleBudget::new(budget);
        for c in channels {
            for l in cache.get(c.channel_id) {
                if l.end > now_ms && l.start < window_end {
                    schedule.push(l.to_item(c.channel_id));
                }
            }
        }
        schedule.finish()
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
        } else {
            item.row = Some("recordings".into());
            item.weight = weight;
            item.is_new = position_ms == 0;
        }
        item
    }
}

/// A listing as kept between refreshes: what a schedule item needs and nothing more.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Cached {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subtitle: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub season: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub episode: Option<i64>,
    pub start: i64,
    pub end: i64,
}

impl Cached {
    fn from_listing(l: &Listing) -> Option<Cached> {
        let subtitle = l.subtitle.clone().filter(|s| !s.is_empty()).or_else(|| l.description.as_deref().map(first_sentence)).filter(|s| !s.is_empty());
        Some(Cached { name: l.name.clone(), subtitle, season: l.season, episode: l.episode, start: ms_from_any(l.start), end: ms_from_any(l.end) }).filter(|c| !c.name.is_empty())
    }

    fn to_item(&self, channel_id: i64) -> Item {
        Item {
            id: format!("prog:{channel_id}:{}", self.start),
            kind: ItemKind::Schedule,
            title: self.name.clone(),
            subtitle: self.subtitle.clone(),
            season: self.season.map(|n| n.to_string()),
            episode: self.episode.map(|n| n.to_string()),
            duration_ms: Some(self.end - self.start),
            channel_id: Some(channel_id.to_string()),
            start_ms: Some(self.start),
            end_ms: Some(self.end),
            ..Default::default()
        }
    }
}

/// The guide kept between refreshes, channels spread over [`GUIDE_KEYS`] store keys by id.
#[derive(Default)]
pub struct GuideCache {
    parts: Vec<BTreeMap<i64, Vec<Cached>>>,
    changed: Vec<bool>,
}

impl GuideCache {
    fn key(part: usize) -> String {
        format!("guide:{part}")
    }

    fn part(channel_id: i64) -> usize {
        channel_id.rem_euclid(GUIDE_KEYS) as usize
    }

    pub fn empty() -> GuideCache {
        GuideCache { parts: vec![BTreeMap::new(); GUIDE_KEYS as usize], changed: vec![false; GUIDE_KEYS as usize] }
    }

    fn load() -> GuideCache {
        let mut cache = GuideCache::empty();
        for (n, part) in cache.parts.iter_mut().enumerate() {
            *part = host::kv_get(&Self::key(n)).and_then(|v| serde_json::from_slice(&v).ok()).unwrap_or_default();
        }
        cache
    }

    pub fn put(&mut self, channel_id: i64, listings: Vec<Cached>) {
        let n = Self::part(channel_id);
        self.parts[n].insert(channel_id, listings);
        self.changed[n] = true;
    }

    pub fn get(&self, channel_id: i64) -> &[Cached] {
        self.parts[Self::part(channel_id)].get(&channel_id).map_or(&[], Vec::as_slice)
    }

    /// Writes the parts that changed, without channels that are gone or listings that are over.
    fn save(&mut self, channels: &[Channel], now_ms: i64) {
        for n in 0..self.parts.len() {
            if self.changed[n] {
                host::kv_set(&Self::key(n), &self.encode(n, channels, now_ms));
            }
        }
    }

    /// One part as stored, kept under [`GUIDE_KEY_BYTES`] by giving up the furthest listings.
    pub fn encode(&mut self, n: usize, channels: &[Channel], now_ms: i64) -> Vec<u8> {
        let part = &mut self.parts[n];
        part.retain(|id, _| channels.iter().any(|c| c.channel_id == *id));
        for listings in part.values_mut() {
            listings.retain(|l| l.end > now_ms);
            listings.sort_by_key(|l| l.start);
        }
        let mut cap = part.values().map(Vec::len).max().unwrap_or(0);
        loop {
            let bytes = serde_json::to_vec(&*part).unwrap_or_default();
            if bytes.len() <= GUIDE_KEY_BYTES || cap == 0 {
                return bytes;
            }
            cap = cap * 3 / 4;
            part.values_mut().for_each(|l| l.truncate(cap));
        }
    }
}

/// When each part-watched recording was last seen to move. NextPVR keeps a position but not
/// when it changed, so the plugin notes the refresh at which it did.
#[derive(Serialize, Deserialize, Default)]
pub struct Watched {
    seen: BTreeMap<i64, (i64, Option<i64>)>,
    #[serde(skip)]
    now: BTreeMap<i64, (i64, Option<i64>)>,
}

impl Watched {
    fn load() -> Watched {
        host::kv_get(WATCHED_KEY).and_then(|v| serde_json::from_slice(&v).ok()).unwrap_or_default()
    }

    /// The last time the position was seen to change, or `None` while that is not known.
    pub fn observe(&mut self, id: i64, position: i64, now_ms: i64) -> Option<i64> {
        let last_used = match self.seen.get(&id) {
            Some(&(before, when)) if before == position => when,
            Some(_) => Some(now_ms),
            None => None,
        };
        self.now.insert(id, (position, last_used));
        last_used
    }

    /// Keeps only the recordings observed this time.
    fn save(self) {
        host::kv_set(WATCHED_KEY, serde_json::to_string(&Watched { seen: self.now, now: BTreeMap::new() }).unwrap_or_default().as_bytes());
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
        assert_eq!(a.last_used_ms, None);
        let b = s.recording_item(&list.recordings[1], 1);
        assert_eq!(b.kind, ItemKind::Program);
        assert_eq!(b.added_ms, sdk::parse_iso8601("2026-10-17T20:00:00Z"));
        assert!(b.is_new);
    }

    #[test]
    fn watching_is_dated_only_once_the_position_moves() {
        let mut w = Watched::default();
        assert_eq!(w.observe(10, 1800, 1_000), None);
        let mut w = Watched { seen: w.now, now: BTreeMap::new() };
        assert_eq!(w.observe(10, 1800, 2_000), None);
        let mut w = Watched { seen: w.now, now: BTreeMap::new() };
        assert_eq!(w.observe(10, 2400, 3_000), Some(3_000));
        let mut w = Watched { seen: w.now, now: BTreeMap::new() };
        assert_eq!(w.observe(10, 2400, 4_000), Some(3_000));
        assert_eq!(w.observe(11, 60, 4_000), None);
        let json = serde_json::to_string(&Watched { seen: w.now, now: BTreeMap::new() }).unwrap();
        let back: Watched = serde_json::from_str(&json).unwrap();
        assert_eq!(back.seen[&10], (2400, Some(3_000)));
    }

    fn listing(start_s: i64, description: &str) -> Cached {
        Cached::from_listing(&Listing { id: 1, name: "News".into(), subtitle: None, description: Some(description.into()), start: start_s, end: start_s + 1800, season: Some(2), episode: None }).unwrap()
    }

    #[test]
    fn the_guide_lives_in_a_fixed_number_of_keys() {
        let channels: Vec<Channel> = (0..600).map(|i| Channel { channel_id: i, channel_number: i, channel_name: format!("C{i}"), ..Default::default() }).collect();
        let mut cache = GuideCache::empty();
        for c in &channels {
            cache.put(c.channel_id, (0..60).map(|n| listing(1_760_800_000 + n * 1800, &format!("{} Tonight. Later.", "Long words ".repeat(20)))).collect());
        }
        cache.put(-5, vec![listing(1_760_800_000, "x")]);
        assert_eq!(cache.parts.len(), GUIDE_KEYS as usize);
        assert!(cache.get(7)[0].subtitle.as_ref().is_some_and(|s| s.chars().count() <= 120));
        assert_eq!(cache.get(7)[0].to_item(7).id, "prog:7:1760800000000");
        let now = 1_760_800_000_000 + 3_600_000;
        for n in 0..GUIDE_KEYS as usize {
            let bytes = cache.encode(n, &channels, now);
            assert!(bytes.len() <= GUIDE_KEY_BYTES, "part {n}: {}", bytes.len());
            let back: BTreeMap<i64, Vec<Cached>> = serde_json::from_slice(&bytes).unwrap();
            assert!(back.values().all(|l| !l.is_empty() && l.iter().all(|x| x.end > now)));
            assert!(!back.contains_key(&-5));
        }
    }
}
