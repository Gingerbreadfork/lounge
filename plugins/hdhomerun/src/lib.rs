//! HDHomeRun: the tuner's channel lineup becomes Live now and the guide, and a channel plays
//! straight from the tuner in the TV's player. The tuner is found through HDHomeRun's discovery
//! service unless an address is given; the guide comes from their guide service with the
//! device's own key, which costs nothing and needs no account.
use lounge_plugin_sdk::{self as sdk, host, Action, Item, ItemKind, Kind, Manifest, RefreshContext, SettingKind, SettingSpec};
use serde::Deserialize;
use std::collections::HashSet;

pub struct HdHomeRun;

impl sdk::Plugin for HdHomeRun {
    fn manifest() -> Manifest {
        Manifest {
            id: "tv.lounge.hdhomerun".into(),
            name: "HDHomeRun".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            description: "Over-the-air channels and a guide from an HDHomeRun tuner on your network.".into(),
            publisher_name: "Lounge".into(),
            kinds: vec![Kind::Content, Kind::Epg],
            // Tuners are found by discovery, so any on the home network may be reached.
            fetch_hosts: vec!["$device".into(), "local-network".into(), "*".into()],
            refresh_minutes: 60,
            rows: vec![],
            settings: vec![
                SettingSpec { key: "device".into(), label: "Tuner address".into(), kind: SettingKind::Url, required: false, help: Some("Like http://192.168.0.20. Leave it empty and the tuner is found automatically.".into()), default: None },
                SettingSpec { key: "skip_drm".into(), label: "Hide channels marked DRM".into(), kind: SettingKind::Toggle, required: false, help: Some("They cannot play outside HDHomeRun's own app.".into()), default: Some("true".into()) },
            ],
            homepage: Some("https://www.hdhomerun.com".into()),
        }
    }

    fn refresh(ctx: &RefreshContext) -> Result<Vec<Item>, String> {
        let base = match host::setting("device").filter(|d| !d.trim().is_empty()) {
            Some(d) => sdk::server_address(&d),
            None => discover()?,
        };
        let device: Discover = get_json(&format!("{base}/discover.json")).map_err(|e| {
            sdk::warn!("{e}");
            format!("No HDHomeRun answered at {}. Check the tuner and its address, then try again.", sdk::address(&base))
        })?;
        let lineup: Vec<LineupEntry> = get_json(&device.lineup_url.clone().unwrap_or_else(|| format!("{base}/lineup.json"))).map_err(|e| {
            sdk::warn!("{e}");
            format!("The HDHomeRun at {} didn't send its channels. Check the tuner, then try again.", sdk::address(&base))
        })?;
        let skip_drm = host::setting("skip_drm").map(|v| v != "false").unwrap_or(true);
        let mut numbers = HashSet::new();
        let mut channels: Vec<&LineupEntry> = lineup.iter().filter(|c| (!skip_drm || c.drm != Some(1)) && !c.url.is_empty() && numbers.insert(c.guide_number.as_str())).collect();
        if channels.is_empty() {
            return Err(format!("{} has no channels yet. Run a channel scan in the HDHomeRun app first.", device.friendly_name.as_deref().unwrap_or("The tuner")));
        }
        channels.truncate(sdk::MAX_ITEMS);
        let guide: Vec<GuideChannel> = match &device.device_auth {
            Some(auth) => match get_json(&format!("https://api.hdhomerun.com/api/guide?DeviceAuth={}", url_encode(auth))) {
                Ok(guide) => guide,
                Err(e) => {
                    sdk::warn!("guide unavailable: {e}");
                    if sdk::last_guide_still_showing(ctx) {
                        return Err(sdk::GUIDE_KEPT.into());
                    }
                    Vec::new()
                }
            },
            None => Vec::new(),
        };
        let mut items: Vec<Item> = Vec::new();
        for c in &channels {
            let logo = guide.iter().find(|g| g.guide_number == c.guide_number).and_then(|g| g.image_url.clone());
            items.push(channel_item(c, logo));
        }
        let window_end = ctx.now_ms + i64::from(ctx.window_hours.max(1)) * 3_600_000;
        let mut schedule = sdk::ScheduleBudget::new(sdk::MAX_ITEMS - items.len());
        for g in guide.iter().filter(|g| numbers.contains(g.guide_number.as_str())) {
            for p in &g.guide {
                let (start, end) = (p.start_time * 1000, p.end_time * 1000);
                if end > ctx.now_ms && start < window_end {
                    schedule.push(schedule_item(&g.guide_number, p, start, end));
                }
            }
        }
        let schedule = schedule.finish();
        sdk::info!("{} channels, {} programmes from {}", channels.len(), schedule.len(), device.friendly_name.as_deref().unwrap_or("the tuner"));
        items.extend(schedule);
        Ok(items)
    }
}

sdk::export_plugin!(HdHomeRun);

#[derive(Deserialize, Default)]
struct Discover {
    #[serde(rename = "FriendlyName")]
    friendly_name: Option<String>,
    #[serde(rename = "DeviceAuth")]
    device_auth: Option<String>,
    #[serde(rename = "LineupURL")]
    lineup_url: Option<String>,
}

#[derive(Deserialize, Default, Clone, Debug)]
pub struct LineupEntry {
    #[serde(rename = "GuideNumber", default)]
    pub guide_number: String,
    #[serde(rename = "GuideName", default)]
    pub guide_name: String,
    #[serde(rename = "URL", default)]
    pub url: String,
    #[serde(rename = "HD")]
    pub hd: Option<i64>,
    #[serde(rename = "DRM")]
    pub drm: Option<i64>,
    #[serde(rename = "Favorite")]
    pub favorite: Option<i64>,
}

#[derive(Deserialize, Default, Clone, Debug)]
pub struct GuideChannel {
    #[serde(rename = "GuideNumber", default)]
    pub guide_number: String,
    #[serde(rename = "GuideName", default)]
    pub guide_name: String,
    #[serde(rename = "ImageURL")]
    pub image_url: Option<String>,
    #[serde(rename = "Guide", default)]
    pub guide: Vec<Programme>,
}

#[derive(Deserialize, Default, Clone, Debug)]
pub struct Programme {
    #[serde(rename = "StartTime", default)]
    pub start_time: i64,
    #[serde(rename = "EndTime", default)]
    pub end_time: i64,
    #[serde(rename = "Title", default)]
    pub title: String,
    #[serde(rename = "EpisodeTitle")]
    pub episode_title: Option<String>,
    #[serde(rename = "EpisodeNumber")]
    pub episode_number: Option<String>,
    #[serde(rename = "Synopsis")]
    pub synopsis: Option<String>,
    #[serde(rename = "ImageURL")]
    pub image_url: Option<String>,
}

/// Asks HDHomeRun's discovery service which tuners share this network's address.
fn discover() -> Result<String, String> {
    #[derive(Deserialize)]
    struct Found {
        #[serde(rename = "BaseURL")]
        base_url: Option<String>,
        #[serde(rename = "LocalIP")]
        local_ip: Option<String>,
        #[serde(rename = "LineupURL")]
        lineup_url: Option<String>,
    }
    let found: Vec<Found> = get_json("https://ipv4-api.hdhomerun.com/discover").map_err(|e| {
        sdk::warn!("{e}");
        "Could not look for tuners. Check the TV's internet connection, or enter the tuner's address in the plugin's settings.".to_string()
    })?;
    found
        .into_iter()
        .filter(|f| f.lineup_url.is_some())
        .find_map(|f| f.base_url.or_else(|| f.local_ip.map(|ip| format!("http://{ip}"))))
        .ok_or_else(|| "No HDHomeRun tuner was found on this network. Enter its address in the plugin's settings.".into())
}

fn get_json<T: serde::de::DeserializeOwned>(url: &str) -> Result<T, String> {
    parse(url, &host::get_text(url)?)
}

fn parse<T: serde::de::DeserializeOwned>(url: &str, body: &str) -> Result<T, String> {
    serde_json::from_str(body).map_err(|e| format!("unexpected answer from {}: {e}", sdk::redact_url(url)))
}

pub fn channel_item(c: &LineupEntry, logo: Option<String>) -> Item {
    Item {
        id: format!("ch:{}", c.guide_number),
        kind: ItemKind::Channel,
        title: c.guide_name.clone(),
        channel_id: Some(c.guide_number.clone()),
        channel_name: Some(c.guide_name.clone()),
        channel_number: Some(c.guide_number.clone()),
        logo,
        weight: if c.favorite == Some(1) { 10 } else { 0 },
        action: Action::play(c.url.clone(), c.guide_name.clone()),
        ..Default::default()
    }
}

pub fn schedule_item(channel: &str, p: &Programme, start_ms: i64, end_ms: i64) -> Item {
    let (season, episode) = p.episode_number.as_deref().map(parse_episode).unwrap_or((None, None));
    Item {
        id: format!("prog:{channel}:{}", p.start_time),
        kind: ItemKind::Schedule,
        title: p.title.clone(),
        subtitle: p.episode_title.clone().or_else(|| p.synopsis.as_deref().map(first_sentence)),
        season,
        episode,
        channel_id: Some(channel.to_string()),
        start_ms: Some(start_ms),
        end_ms: Some(end_ms),
        thumbnail: p.image_url.clone(),
        ..Default::default()
    }
}

/// "S02E04" → ("2", "4"); other shapes give nothing rather than nonsense.
pub fn parse_episode(s: &str) -> (Option<String>, Option<String>) {
    let s = s.trim().to_ascii_uppercase();
    let rest = match s.strip_prefix('S') {
        Some(r) => r,
        None => return (None, None),
    };
    let (season, episode) = match rest.split_once('E') {
        Some(x) => x,
        None => return (None, None),
    };
    match (season.parse::<u32>(), episode.parse::<u32>()) {
        (Ok(sn), Ok(ep)) => (Some(sn.to_string()), Some(ep.to_string())),
        _ => (None, None),
    }
}

fn first_sentence(s: &str) -> String {
    let end = s.find(". ").map(|i| i + 1).unwrap_or(s.len()).min(120);
    let cut = s.char_indices().take_while(|(i, _)| *i < end).map(|(i, c)| i + c.len_utf8()).last().unwrap_or(0);
    s[..cut].trim().to_string()
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
    fn lineup_and_guide_become_channels_and_programmes() {
        let lineup: Vec<LineupEntry> = serde_json::from_str(r#"[
            {"GuideNumber": "7.1", "GuideName": "ABC", "URL": "http://192.168.0.20:5004/auto/v7.1", "HD": 1, "Favorite": 1},
            {"GuideNumber": "9.2", "GuideName": "Shop", "URL": "http://192.168.0.20:5004/auto/v9.2", "DRM": 1}
        ]"#).unwrap();
        let guide: Vec<GuideChannel> = serde_json::from_str(r#"[{"GuideNumber": "7.1", "GuideName": "ABC", "ImageURL": "https://img.hdhomerun.com/channels/7.png",
            "Guide": [{"StartTime": 1760800000, "EndTime": 1760803600, "Title": "News", "EpisodeNumber": "S12E04", "Synopsis": "Tonight. And more.", "ImageURL": "https://img/news.jpg"}]}]"#).unwrap();
        let ch = channel_item(&lineup[0], guide[0].image_url.clone());
        assert_eq!(ch.kind, ItemKind::Channel);
        assert_eq!(ch.channel_number.as_deref(), Some("7.1"));
        assert_eq!(ch.action.play.as_deref(), Some("http://192.168.0.20:5004/auto/v7.1"));
        assert_eq!(ch.weight, 10);
        assert!(lineup[1].drm == Some(1));
        let p = &guide[0].guide[0];
        let s = schedule_item("7.1", p, p.start_time * 1000, p.end_time * 1000);
        assert_eq!(s.kind, ItemKind::Schedule);
        assert_eq!((s.season.as_deref(), s.episode.as_deref()), (Some("12"), Some("4")));
        assert_eq!(s.subtitle.as_deref(), Some("Tonight."));
        assert_eq!((s.start_ms, s.end_ms), (Some(1760800000000), Some(1760803600000)));
    }

    #[test]
    fn errors_never_show_the_device_key() {
        let e = parse::<Vec<GuideChannel>>("https://api.hdhomerun.com/api/guide?DeviceAuth=SECRETKEY", "<html>").unwrap_err();
        assert!(e.starts_with("unexpected answer from https://api.hdhomerun.com/api/guide?…"), "{e}");
        assert!(!e.contains("SECRETKEY"));
    }

    #[test]
    fn episode_strings() {
        assert_eq!(parse_episode("S02E04"), (Some("2".into()), Some("4".into())));
        assert_eq!(parse_episode("E04"), (None, None));
        assert_eq!(parse_episode("Part 2"), (None, None));
    }
}
