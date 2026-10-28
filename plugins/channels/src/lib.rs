//! Channels DVR: the server's channels become Live now with the guide behind them, half-watched
//! recordings go into Continue watching, and the rest fill a "Recordings" row. Everything plays
//! in the TV's player from the server, which needs no login on the local network.
use lounge_plugin_sdk::{self as sdk, host, Action, Item, ItemKind, Kind, Manifest, RefreshContext, RowSpec, RowStyle, SettingKind, SettingSpec};
use serde::Deserialize;
use std::collections::HashSet;

pub struct Channels;

impl sdk::Plugin for Channels {
    fn manifest() -> Manifest {
        Manifest {
            id: "tv.lounge.channels".into(),
            name: "Channels DVR".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            description: "Live channels, the guide and your recordings from a Channels DVR server.".into(),
            publisher_name: "Lounge".into(),
            kinds: vec![Kind::Content, Kind::Epg],
            fetch_hosts: vec!["$server".into(), "*".into()],
            refresh_minutes: 30,
            rows: vec![RowSpec { id: "recordings".into(), title: "Recordings".into(), style: RowStyle::Landscape }],
            settings: vec![
                SettingSpec { key: "server".into(), label: "Server address".into(), kind: SettingKind::Url, required: true, help: Some("Like http://192.168.0.10:8089".into()), default: None },
                SettingSpec { key: "favorites_only".into(), label: "Only favourite channels".into(), kind: SettingKind::Toggle, required: false, help: None, default: Some("false".into()) },
                SettingSpec { key: "recordings".into(), label: "Recordings to show".into(), kind: SettingKind::Number, required: false, help: Some("The newest ones.".into()), default: Some("30".into()) },
            ],
            homepage: Some("https://getchannels.com/dvr-server/".into()),
        }
    }

    fn refresh(ctx: &RefreshContext) -> Result<Vec<Item>, String> {
        let base = host::server_setting("server", "Server address")?;
        let favorites_only = host::setting("favorites_only").map(|v| v == "true").unwrap_or(false);
        let max_recordings = host::setting("recordings").and_then(|v| v.trim().parse::<usize>().ok()).unwrap_or(30).clamp(1, 100);
        let mut numbers = HashSet::new();
        let mut channels: Vec<Channel> = lineup(&base)
            .map_err(|e| format!("No Channels DVR at {}: {e}", sdk::redact_url(&base)))?
            .into_iter()
            .filter(|c| !c.hidden && (c.favorite || !favorites_only) && !c.number().is_empty() && numbers.insert(c.number()))
            .collect();
        if channels.is_empty() {
            return Err("Channels DVR has no channels to show. Add a source in its settings, or turn off the favourites filter.".into());
        }
        if channels.len() > sdk::MAX_ITEMS {
            sdk::warn!("{} channels, of which the TV shows the first {}; turn on the favourites filter to choose", channels.len(), sdk::MAX_ITEMS);
            channels.truncate(sdk::MAX_ITEMS);
        }
        let mut items: Vec<Item> = channels.iter().map(|c| channel_item(&base, c)).collect();

        match host::get_text(&format!("{base}/dvr/files")).and_then(|body| newest_recordings(&body, max_recordings)) {
            Ok(files) => {
                let count = files.len();
                items.extend(files.iter().enumerate().map(|(i, f)| {
                    let mut item = recording_item(&base, f, (count - i) as i32);
                    item.is_new &= sdk::new_this_week(item.added_ms, ctx.now_ms);
                    item
                }));
            }
            Err(e) if e == host::TOO_LARGE => sdk::warn!("the recordings list is larger than the 8 MB a plugin may read, so recordings are left out; the channels and guide are not affected"),
            Err(e) => sdk::warn!("recordings unavailable: {e}"),
        }
        items.truncate(sdk::MAX_ITEMS);
        let recordings = items.len() - channels.len();

        let window_end = ctx.now_ms + i64::from(ctx.window_hours.max(1)) * 3_600_000;
        let mut schedule = sdk::ScheduleBudget::new(sdk::MAX_ITEMS - items.len());
        let read = host::get_text(&format!("{base}/devices/ANY/guide")).and_then(|body| {
            sdk::for_each_in_array(&body, |g: GuideEntry| {
                let number = g.channel.number();
                if !numbers.contains(&number) {
                    return;
                }
                for a in &g.airings {
                    let (start, end) = (a.time * 1000, (a.time + a.duration) * 1000);
                    if end > ctx.now_ms && start < window_end {
                        schedule.push(schedule_item(&number, a, start, end));
                    }
                }
            })
        });
        if let Err(e) = read {
            sdk::warn!("guide unavailable: {e}");
            if sdk::last_guide_still_showing(ctx) {
                return Err(sdk::GUIDE_KEPT.into());
            }
        }
        let schedule = schedule.finish();
        sdk::info!("{} channels, {} programmes, {recordings} recordings", channels.len(), schedule.len());
        items.extend(schedule);
        Ok(items)
    }
}

/// The server's channel lineup, or its older channel list when the lineup is not there.
fn lineup(base: &str) -> Result<Vec<Channel>, String> {
    get_json(&format!("{base}/devices/ANY/channels")).or_else(|e| get_json(&format!("{base}/api/channels")).map_err(|_| e))
}

/// Most part-watched recordings kept for Continue watching.
const MAX_STARTED: usize = 40;

/// The newest part-watched recordings and the newest `max` of the other finished ones, read from
/// the server's list one at a time so a large library is never held whole.
pub fn newest_recordings(json: &str, max: usize) -> Result<Vec<Recording>, String> {
    let newest_first = |files: &mut Vec<Recording>, keep: usize| {
        files.sort_by_key(|f| std::cmp::Reverse(f.airing.time));
        files.truncate(keep);
    };
    let (mut started, mut kept) = (Vec::new(), Vec::new());
    sdk::for_each_in_array(json, |r: Recording| {
        if r.completed && !r.cancelled && !r.delete && r.duration > 60.0 {
            let (list, keep) = if r.in_progress() { (&mut started, MAX_STARTED) } else { (&mut kept, max) };
            list.push(r);
            if list.len() >= 2 * keep {
                newest_first(list, keep);
            }
        }
    })
    .map_err(|e| format!("unexpected recordings list: {e}"))?;
    newest_first(&mut started, MAX_STARTED);
    newest_first(&mut kept, max);
    started.extend(kept);
    Ok(started)
}

sdk::export_plugin!(Channels);

#[derive(Deserialize, Default, Clone, Debug)]
pub struct Channel {
    #[serde(rename = "GuideNumber", alias = "Number", default)]
    pub number: String,
    #[serde(rename = "GuideName", alias = "Name", default)]
    pub name: String,
    #[serde(rename = "Logo", default)]
    pub logo: Option<String>,
    #[serde(rename = "Favorite", default)]
    pub favorite: bool,
    #[serde(rename = "Hidden", default)]
    pub hidden: bool,
    #[serde(rename = "HD", default)]
    pub hd: bool,
}

impl Channel {
    fn number(&self) -> String {
        self.number.trim().to_string()
    }
}

#[derive(Deserialize, Default, Clone, Debug)]
pub struct GuideEntry {
    #[serde(rename = "Channel", default)]
    pub channel: Channel,
    #[serde(rename = "Airings", default)]
    pub airings: Vec<Airing>,
}

#[derive(Deserialize, Default, Clone, Debug)]
pub struct Airing {
    #[serde(rename = "Time", default)]
    pub time: i64,
    #[serde(rename = "Duration", default)]
    pub duration: i64,
    #[serde(rename = "Title", default)]
    pub title: String,
    #[serde(rename = "EpisodeTitle")]
    pub episode_title: Option<String>,
    #[serde(rename = "Summary")]
    pub summary: Option<String>,
    #[serde(rename = "Image")]
    pub image: Option<String>,
    #[serde(rename = "SeasonNumber")]
    pub season_number: Option<i64>,
    #[serde(rename = "EpisodeNumber")]
    pub episode_number: Option<i64>,
    #[serde(rename = "Channel", default)]
    pub channel: String,
    #[serde(rename = "Categories", default)]
    pub categories: Vec<String>,
}

#[derive(Deserialize, Default, Clone, Debug)]
pub struct Recording {
    #[serde(rename = "ID", default)]
    pub id: String,
    #[serde(rename = "Airing", default)]
    pub airing: Airing,
    #[serde(rename = "Completed", default)]
    pub completed: bool,
    #[serde(rename = "Cancelled", default)]
    pub cancelled: bool,
    #[serde(rename = "Delete", default)]
    pub delete: bool,
    #[serde(rename = "Watched", default)]
    pub watched: bool,
    #[serde(rename = "Duration", default)]
    pub duration: f64,
    #[serde(rename = "PlaybackTime", default)]
    pub playback_time: f64,
    /// Epoch milliseconds.
    #[serde(rename = "CreatedAt", default)]
    pub created_at: i64,
    /// Epoch milliseconds; changes when the playback position does.
    #[serde(rename = "UpdatedAt", default)]
    pub updated_at: i64,
}

impl Recording {
    /// Watched past the first minute and short of the end, so it belongs in Continue watching.
    fn in_progress(&self) -> bool {
        let position_ms = self.playback_time * 1000.0;
        !self.watched && position_ms > 60_000.0 && position_ms < self.duration * 1000.0 * 0.95
    }
}

fn get_json<T: serde::de::DeserializeOwned>(url: &str) -> Result<T, String> {
    let body = host::get_text(url)?;
    serde_json::from_str(&body).map_err(|e| format!("unexpected answer from {}: {e}", sdk::redact_url(url)))
}

/// Epoch milliseconds from a stamp in milliseconds, or in seconds as older servers send.
pub fn epoch_ms(stamp: i64) -> Option<i64> {
    match stamp {
        s if s <= 0 => None,
        s if s < 100_000_000_000 => Some(s * 1000),
        s => Some(s),
    }
}

pub fn channel_item(base: &str, c: &Channel) -> Item {
    let number = c.number();
    Item {
        id: format!("ch:{number}"),
        kind: ItemKind::Channel,
        title: c.name.clone(),
        channel_id: Some(number.clone()),
        channel_name: Some(c.name.clone()),
        channel_number: Some(number.clone()),
        logo: c.logo.clone().filter(|l| !l.is_empty()),
        weight: if c.favorite { 10 } else { 0 },
        action: Action::play(format!("{base}/devices/ANY/channels/{number}/stream.mpg"), c.name.clone()),
        ..Default::default()
    }
}

pub fn schedule_item(channel: &str, a: &Airing, start_ms: i64, end_ms: i64) -> Item {
    Item {
        id: format!("prog:{channel}:{}", a.time),
        kind: ItemKind::Schedule,
        title: a.title.clone(),
        subtitle: a.episode_title.clone().filter(|s| !s.is_empty()).or_else(|| a.summary.as_deref().map(first_sentence)),
        season: a.season_number.map(|n| n.to_string()),
        episode: a.episode_number.map(|n| n.to_string()),
        duration_ms: Some(end_ms - start_ms),
        channel_id: Some(channel.to_string()),
        start_ms: Some(start_ms),
        end_ms: Some(end_ms),
        thumbnail: a.image.clone().filter(|i| !i.is_empty()),
        ..Default::default()
    }
}

/// A finished recording: a resume point when it was started and not finished, otherwise a tile
/// in the Recordings row.
pub fn recording_item(base: &str, r: &Recording, weight: i32) -> Item {
    let a = &r.airing;
    let duration_ms = (r.duration * 1000.0) as i64;
    let position_ms = (r.playback_time * 1000.0) as i64;
    let in_progress = r.in_progress();
    let mut item = Item {
        id: format!("rec:{}", r.id),
        kind: if in_progress { ItemKind::Resume } else { ItemKind::Program },
        title: a.title.clone(),
        subtitle: a.episode_title.clone().filter(|s| !s.is_empty()).or_else(|| a.summary.as_deref().map(first_sentence)),
        season: a.season_number.map(|n| n.to_string()),
        episode: a.episode_number.map(|n| n.to_string()),
        duration_ms: Some(duration_ms),
        added_ms: epoch_ms(r.created_at).or(epoch_ms(a.time)),
        thumbnail: a.image.clone().filter(|i| !i.is_empty()),
        channel_name: if a.channel.is_empty() { None } else { Some(a.channel.clone()) },
        action: Action::play(format!("{base}/dvr/files/{}/stream.mpg", r.id), a.title.clone()),
        ..Default::default()
    };
    if in_progress {
        item.position_ms = Some(position_ms);
        item.action.start_ms = Some(position_ms);
        item.progress = Some((position_ms as f32 / duration_ms as f32).clamp(0.0, 1.0));
        item.last_used_ms = epoch_ms(r.updated_at);
    } else {
        item.row = Some("recordings".into());
        item.weight = weight;
        item.is_new = !r.watched && position_ms == 0;
    }
    item
}

fn first_sentence(s: &str) -> String {
    let end = s.find(". ").map(|i| i + 1).unwrap_or(s.len()).min(120);
    let cut = s.char_indices().take_while(|(i, _)| *i < end).map(|(i, c)| i + c.len_utf8()).last().unwrap_or(0);
    s[..cut].trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channels_read_either_naming() {
        let a: Vec<Channel> = serde_json::from_str(r#"[{"GuideNumber": "2.1", "GuideName": "KTVU", "Logo": "http://l/2.png", "Favorite": true, "Hidden": false, "HD": true}]"#).unwrap();
        let b: Vec<Channel> = serde_json::from_str(r#"[{"Number": "6000", "Name": "Pluto Kids", "Hidden": true}]"#).unwrap();
        let item = channel_item("http://dvr:8089", &a[0]);
        assert_eq!(item.channel_number.as_deref(), Some("2.1"));
        assert_eq!(item.action.play.as_deref(), Some("http://dvr:8089/devices/ANY/channels/2.1/stream.mpg"));
        assert_eq!(item.weight, 10);
        assert_eq!(b[0].name, "Pluto Kids");
        assert!(b[0].hidden);
    }

    #[test]
    fn the_guide_turns_into_schedule_items() {
        let g: Vec<GuideEntry> = serde_json::from_str(r#"[{"Channel": {"Number": "2.1", "Name": "KTVU"}, "Airings": [
            {"Time": 1760800000, "Duration": 1800, "Title": "News", "EpisodeTitle": "", "Summary": "Tonight. Later.", "SeasonNumber": 12, "EpisodeNumber": 3, "Image": "http://i/n.jpg", "Channel": "2.1"}
        ]}]"#).unwrap();
        let a = &g[0].airings[0];
        let s = schedule_item(&g[0].channel.number(), a, a.time * 1000, (a.time + a.duration) * 1000);
        assert_eq!(s.kind, ItemKind::Schedule);
        assert_eq!(s.subtitle.as_deref(), Some("Tonight."));
        assert_eq!((s.season.as_deref(), s.episode.as_deref()), (Some("12"), Some("3")));
        assert_eq!(s.end_ms, Some(1760801800000));
    }

    #[test]
    fn recordings_split_into_resume_and_row() {
        let files: Vec<Recording> = serde_json::from_str(r#"[
            {"ID": "1", "Airing": {"Time": 1760700000, "Duration": 3600, "Title": "Film", "Image": "http://i/f.jpg"}, "Completed": true, "Duration": 3590.5, "PlaybackTime": 1200, "Watched": false, "CreatedAt": 1760703600123, "UpdatedAt": 1760790000456},
            {"ID": "2", "Airing": {"Time": 1760600000, "Duration": 1800, "Title": "Show", "EpisodeTitle": "Pilot"}, "Completed": true, "Duration": 1790, "PlaybackTime": 0, "Watched": false},
            {"ID": "3", "Airing": {"Time": 1760500000, "Duration": 1800, "Title": "Gone"}, "Completed": true, "Duration": 1790, "Delete": true}
        ]"#).unwrap();
        let a = recording_item("http://dvr:8089", &files[0], 2);
        assert_eq!(a.kind, ItemKind::Resume);
        assert_eq!(a.position_ms, Some(1_200_000));
        assert!((a.progress.unwrap() - 0.334).abs() < 0.01);
        assert_eq!(a.last_used_ms, Some(1760790000456));
        assert_eq!(a.added_ms, Some(1760703600123));
        assert_eq!(a.action.play.as_deref(), Some("http://dvr:8089/dvr/files/1/stream.mpg"));
        assert_eq!(a.action.start_ms, Some(1_200_000));
        let b = recording_item("http://dvr:8089", &files[1], 1);
        assert_eq!(b.kind, ItemKind::Program);
        assert_eq!(b.row.as_deref(), Some("recordings"));
        assert_eq!(b.subtitle.as_deref(), Some("Pilot"));
        assert!(b.is_new);
        assert_eq!(b.added_ms, Some(1760600000000));
        assert!(files[2].delete);
    }

    #[test]
    fn stamps_in_seconds_or_milliseconds_land_in_milliseconds() {
        assert_eq!(epoch_ms(1760790000456), Some(1760790000456));
        assert_eq!(epoch_ms(1760790000), Some(1760790000000));
        assert_eq!(epoch_ms(0), None);
        let r: Recording = serde_json::from_str(r#"{"ID": "9", "Airing": {"Time": 1760700000, "Title": "F"}, "Completed": true, "Duration": 3600, "PlaybackTime": 600, "UpdatedAt": 0}"#).unwrap();
        assert_eq!(recording_item("http://dvr:8089", &r, 1).last_used_ms, None);
    }

    #[test]
    fn only_the_newest_finished_recordings_are_kept() {
        let mut json = String::from("[");
        for i in 0..500 {
            let done = i % 7 != 0;
            json.push_str(&format!(r#"{{"ID": "{i}", "Airing": {{"Time": {}, "Title": "R{i}", "Summary": "{}"}}, "Completed": {done}, "Duration": 1800}},"#, 1_760_000_000 + i * 3600, "x".repeat(200)));
        }
        json.push_str(r#"{"ID": "short", "Airing": {"Time": 1790000000}, "Completed": true, "Duration": 30}]"#);
        let kept = newest_recordings(&json, 5).unwrap();
        let ids: Vec<&str> = kept.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids, ["499", "498", "496", "495", "494"]);
        assert!(newest_recordings("{\"error\": \"nope\"}", 5).is_err());
    }

    #[test]
    fn part_watched_recordings_stay_past_the_rows_limit() {
        let json = r#"[{"ID": "new", "Airing": {"Time": 1760003600}, "Completed": true, "Duration": 1800},
            {"ID": "newer", "Airing": {"Time": 1760007200}, "Completed": true, "Duration": 1800},
            {"ID": "old", "Airing": {"Time": 1760000000}, "Completed": true, "Duration": 1800, "PlaybackTime": 600}]"#;
        let ids: Vec<String> = newest_recordings(json, 1).unwrap().into_iter().map(|r| r.id).collect();
        assert_eq!(ids, ["old", "newer"]);
    }
}
