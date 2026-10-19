//! IPTV plugin: an M3U playlist becomes channels in Live now and the guide; an optional XMLTV
//! feed (plain or gzipped) fills in what is on. Streams open in whatever video player the TV
//! has (`play` action).

use lounge_plugin_sdk::{self as sdk, epoch_ms, host, Action, Item, ItemKind, Kind, Manifest, RefreshContext, SettingKind, SettingSpec};
use std::collections::HashMap;
use std::io::Read;

pub struct Iptv;

const MAX_SCHEDULE: usize = 30_000;

impl sdk::Plugin for Iptv {
    fn manifest() -> Manifest {
        Manifest {
            id: "tv.lounge.iptv".into(),
            name: "IPTV playlist".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            description: "Channels from an M3U playlist, with programme information from an XMLTV guide.".into(),
            publisher_name: "Lounge".into(),
            kinds: vec![Kind::Content, Kind::Epg],
            fetch_hosts: vec!["*".into()],
            refresh_minutes: 60,
            rows: Vec::new(),
            settings: vec![
                SettingSpec { key: "playlist_url".into(), label: "Playlist URL (M3U)".into(), kind: SettingKind::Url, required: true, help: Some("The .m3u or .m3u8 playlist your provider gave you.".into()), default: None },
                SettingSpec { key: "epg_url".into(), label: "Guide URL (XMLTV)".into(), kind: SettingKind::Url, required: false, help: Some("Optional. Plain or .gz XMLTV; channel ids must match the playlist's tvg-id.".into()), default: None },
                SettingSpec { key: "group".into(), label: "Only this group".into(), kind: SettingKind::Text, required: false, help: Some("Optional group-title filter, for very large playlists.".into()), default: None },
            ],
            homepage: None,
        }
    }

    fn refresh(ctx: &RefreshContext) -> Result<Vec<Item>, String> {
        let playlist_url = host::required_setting("playlist_url", "Playlist URL")?;
        let group = host::setting("group");
        let playlist = host::get_text(&playlist_url)?;
        let channels = parse_m3u(&playlist, group.as_deref());
        if channels.is_empty() {
            return Err("the playlist has no channels".into());
        }
        sdk::info!("{} channels in playlist", channels.len());

        let mut items: Vec<Item> = channels.iter().map(|c| c.to_item()).collect();

        if let Some(epg_url) = host::setting("epg_url") {
            let response = host::fetch(&sdk::HttpRequest::get(&epg_url))?;
            if !response.ok() {
                return Err(format!("HTTP {} for the guide", response.status));
            }
            let xml = decode_maybe_gzip(response.bytes())?;
            let by_id: HashMap<&str, &Channel> = channels.iter().map(|c| (c.id.as_str(), c)).collect();
            let by_name: HashMap<String, &Channel> = channels.iter().map(|c| (normalise(&c.name), c)).collect();
            let from = ctx.now_ms - 3 * 3_600_000;
            let to = ctx.now_ms + (ctx.window_hours.max(6) as i64) * 3_600_000;
            let (programmes, aliases) = parse_xmltv(&xml, from, to, &by_id, &by_name);
            let matched = programmes.iter().filter(|p| by_id.contains_key(p.channel.as_str())).count();
            sdk::info!("{} programmes in window for {} channels ({} matched by name)", programmes.len(), by_id.len(), aliases);
            for p in programmes {
                if let Some(ch) = by_id.get(p.channel.as_str()) {
                    items.push(p.to_item(ch));
                }
            }
            if matched == 0 {
                sdk::warn!("no guide channel ids match the playlist's tvg-id values or names");
            }
        }
        Ok(items)
    }
}

sdk::export_plugin!(Iptv);

#[derive(Clone, Debug, PartialEq)]
pub struct Channel {
    pub id: String,
    pub name: String,
    pub number: Option<String>,
    pub logo: Option<String>,
    pub group: Option<String>,
    pub url: String,
}

impl Channel {
    fn to_item(&self) -> Item {
        Item {
            id: format!("ch:{}", self.id),
            kind: ItemKind::Channel,
            title: self.name.clone(),
            subtitle: self.group.clone(),
            logo: self.logo.clone(),
            channel_id: Some(self.id.clone()),
            channel_name: Some(self.name.clone()),
            channel_number: self.number.clone(),
            action: Action::play(&self.url, &self.name),
            ..Default::default()
        }
    }
}

/// Parse an extended M3U. Attributes on `#EXTINF` (`tvg-id`, `tvg-name`, `tvg-logo`, `tvg-chno`,
/// `group-title`) are optional; the id falls back to the name, then the URL.
pub fn parse_m3u(text: &str, only_group: Option<&str>) -> Vec<Channel> {
    let mut out = Vec::new();
    let mut pending: Option<(HashMap<String, String>, String)> = None;
    let mut index = 0usize;
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(rest) = line.strip_prefix("#EXTINF:") {
            let (attrs_part, name) = match rest.rfind(',') {
                Some(i) => (&rest[..i], rest[i + 1..].trim()),
                None => (rest, ""),
            };
            pending = Some((parse_attrs(attrs_part), name.to_string()));
        } else if line.starts_with('#') {
            continue;
        } else if let Some((attrs, name)) = pending.take() {
            index += 1;
            let group = attrs.get("group-title").cloned().filter(|g| !g.is_empty());
            if let Some(want) = only_group {
                if group.as_deref().map(|g| !g.eq_ignore_ascii_case(want)).unwrap_or(true) {
                    continue;
                }
            }
            let name = if name.is_empty() { attrs.get("tvg-name").cloned().unwrap_or_else(|| format!("Channel {index}")) } else { name };
            let id = attrs
                .get("tvg-id")
                .cloned()
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| slug(&name));
            out.push(Channel {
                id,
                name,
                number: attrs.get("tvg-chno").cloned().filter(|s| !s.is_empty()).or_else(|| Some(index.to_string())),
                logo: attrs.get("tvg-logo").cloned().filter(|s| s.starts_with("http")),
                group,
                url: line.to_string(),
            });
        }
    }
    out
}

fn parse_attrs(s: &str) -> HashMap<String, String> {
    let mut out = HashMap::new();
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        while i < bytes.len() && (bytes[i] == b' ' || bytes[i] == b'\t') {
            i += 1;
        }
        let key_start = i;
        while i < bytes.len() && bytes[i] != b'=' && bytes[i] != b' ' {
            i += 1;
        }
        let key = &s[key_start..i];
        if i >= bytes.len() || bytes[i] != b'=' {
            continue;
        }
        i += 1;
        let value = if i < bytes.len() && bytes[i] == b'"' {
            i += 1;
            let start = i;
            while i < bytes.len() && bytes[i] != b'"' {
                i += 1;
            }
            let v = &s[start..i.min(bytes.len())];
            i += 1;
            v
        } else {
            let start = i;
            while i < bytes.len() && bytes[i] != b' ' {
                i += 1;
            }
            &s[start..i]
        };
        if !key.is_empty() {
            out.insert(key.to_ascii_lowercase(), value.to_string());
        }
    }
    out
}

fn slug(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars().flat_map(char::to_lowercase) {
        if c.is_alphanumeric() {
            out.push(c);
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    out.trim_matches('-').to_string()
}

#[derive(Clone, Debug, PartialEq)]
pub struct Programme {
    pub channel: String,
    pub title: String,
    pub description: Option<String>,
    pub start_ms: i64,
    pub end_ms: i64,
}

impl Programme {
    fn to_item(&self, ch: &Channel) -> Item {
        Item {
            id: format!("prog:{}:{}", self.channel, self.start_ms),
            kind: ItemKind::Schedule,
            title: self.title.clone(),
            subtitle: self.description.as_ref().map(|d| truncate(d, 140)),
            duration_ms: Some(self.end_ms - self.start_ms),
            channel_id: Some(self.channel.clone()),
            channel_name: Some(ch.name.clone()),
            channel_number: ch.number.clone(),
            start_ms: Some(self.start_ms),
            end_ms: Some(self.end_ms),
            action: Action::play(&ch.url, &self.title),
            ..Default::default()
        }
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max { s.to_string() } else { format!("{}…", s.chars().take(max - 1).collect::<String>().trim_end()) }
}

fn decode_maybe_gzip(bytes: Vec<u8>) -> Result<String, String> {
    if bytes.len() > 2 && bytes[0] == 0x1f && bytes[1] == 0x8b {
        let mut out = String::new();
        flate2::read::GzDecoder::new(&bytes[..]).read_to_string(&mut out).map_err(|e| format!("gzip: {e}"))?;
        Ok(out)
    } else {
        String::from_utf8(bytes).map_err(|_| "the guide is not UTF-8".to_string())
    }
}

pub fn normalise(name: &str) -> String {
    let lower = name.to_lowercase();
    let cut = lower.split(['(', '[']).next().unwrap_or("");
    cut.chars().filter(|c| c.is_alphanumeric()).collect()
}

/// Programmes from an XMLTV document that overlap `[from_ms, to_ms)` on playlist channels. Guide
/// ids rarely match a playlist's `tvg-id`, so display names are matched too and programmes are re-
/// keyed to the playlist id. Also returns how many guide channels matched by name.
pub fn parse_xmltv(xml: &str, from_ms: i64, to_ms: i64, by_id: &HashMap<&str, &Channel>, by_name: &HashMap<String, &Channel>) -> (Vec<Programme>, usize) {
    use quick_xml::events::Event;
    let mut reader = quick_xml::Reader::from_str(xml);
    reader.config_mut().trim_text(true);
    let mut out = Vec::new();
    let mut aliases: HashMap<String, String> = HashMap::new();
    let mut current: Option<Programme> = None;
    let mut field: Option<&'static str> = None;
    let mut channel_el: Option<String> = None;
    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) => match e.name().as_ref() {
                "channel" => {
                    channel_el = e.attributes().flatten().find(|a| a.key.as_ref() == "id").map(|a| a.value.into_owned());
                }
                "display-name" if channel_el.is_some() => field = Some("display-name"),
                "programme" => {
                    let mut channel = String::new();
                    let (mut start, mut stop) = (None, None);
                    for a in e.attributes().flatten() {
                        let v = a.value.into_owned();
                        match a.key.as_ref() {
                            "channel" => channel = v,
                            "start" => start = parse_xmltv_time(&v),
                            "stop" => stop = parse_xmltv_time(&v),
                            _ => {}
                        }
                    }
                    let key = if by_id.contains_key(channel.as_str()) { Some(channel) } else { aliases.get(&channel).cloned() };
                    current = match (start, stop, key) {
                        (Some(s), Some(e), Some(channel)) if e > from_ms && s < to_ms => {
                            Some(Programme { channel, title: String::new(), description: None, start_ms: s, end_ms: e })
                        }
                        _ => None,
                    };
                }
                "title" if current.is_some() => field = Some("title"),
                "desc" if current.is_some() => field = Some("desc"),
                "sub-title" if current.is_some() => field = Some("sub"),
                _ => {}
            },
            Ok(Event::Text(t)) => {
                let raw = t.into_inner();
                let text = quick_xml::escape::unescape(raw.as_ref()).map(|c| c.into_owned()).unwrap_or_else(|_| raw.into_owned());
                if field == Some("display-name") {
                    if let Some(id) = &channel_el {
                        if !by_id.contains_key(id.as_str()) {
                            if let Some(ch) = by_name.get(&normalise(&text)) {
                                aliases.entry(id.clone()).or_insert_with(|| ch.id.clone());
                            }
                        }
                    }
                } else if let (Some(p), Some(f)) = (current.as_mut(), field) {
                    match f {
                        "title" if p.title.is_empty() => p.title = text,
                        "desc" if p.description.is_none() => p.description = Some(text),
                        "sub" if p.description.is_none() => p.description = Some(text),
                        _ => {}
                    }
                }
            }
            Ok(Event::End(e)) => match e.name().as_ref() {
                "channel" => channel_el = None,
                "programme" => {
                    if let Some(p) = current.take() {
                        if !p.title.is_empty() {
                            out.push(p);
                            if out.len() >= MAX_SCHEDULE {
                                break;
                            }
                        }
                    }
                }
                "title" | "desc" | "sub-title" | "display-name" => field = None,
                _ => {}
            },
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
    }
    (out, aliases.len())
}

/// "20261019001500 +0000" → epoch ms. The zone part is optional (UTC).
pub fn parse_xmltv_time(s: &str) -> Option<i64> {
    let s = s.trim();
    let digits: String = s.chars().take_while(|c| c.is_ascii_digit()).collect();
    if digits.len() < 12 {
        return None;
    }
    let num = |a: usize, b: usize| digits.get(a..b).and_then(|x| x.parse::<u32>().ok());
    let year = digits[0..4].parse::<i64>().ok()?;
    let (month, day, hour, minute) = (num(4, 6)?, num(6, 8)?, num(8, 10)?, num(10, 12)?);
    let second = num(12, 14).unwrap_or(0);
    let zone = s[digits.len()..].trim();
    let offset = if zone.len() >= 5 {
        let sign = if zone.starts_with('-') { -1 } else { 1 };
        let hh: i32 = zone[1..3].parse().ok()?;
        let mm: i32 = zone[3..5].parse().ok()?;
        sign * (hh * 60 + mm)
    } else {
        0
    };
    Some(epoch_ms(year, month, day, hour, minute, second, offset))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_extended_m3u() {
        let m3u = r#"#EXTM3U
#EXTINF:-1 tvg-id="abc.au" tvg-name="ABC" tvg-logo="https://x/abc.png" group-title="News",ABC TV
https://example.com/abc.m3u8
#EXTINF:-1 group-title="Sport",Sport One
https://example.com/sport.m3u8
#EXTINF:-1 tvg-chno="7",Seven
https://example.com/seven.m3u8
"#;
        let chans = parse_m3u(m3u, None);
        assert_eq!(chans.len(), 3);
        assert_eq!(chans[0].id, "abc.au");
        assert_eq!(chans[0].logo.as_deref(), Some("https://x/abc.png"));
        assert_eq!(chans[0].number.as_deref(), Some("1"));
        assert_eq!(chans[1].id, "sport-one");
        assert_eq!(chans[2].number.as_deref(), Some("7"));
        assert_eq!(parse_m3u(m3u, Some("sport")).len(), 1);
    }

    #[test]
    fn parses_xmltv_times_and_programmes() {
        assert_eq!(parse_xmltv_time("20261019001500 +0000"), Some(1_792_368_900_000));
        assert_eq!(parse_xmltv_time("20261019101500 +1000"), Some(1_792_368_900_000));
        let xml = r#"<?xml version="1.0"?><tv>
<channel id="abc.au"><display-name>ABC</display-name></channel>
<programme start="20261019000000 +0000" stop="20261019003000 +0000" channel="abc.au"><title>News</title><desc>Tonight's news.</desc></programme>
<programme start="20261019003000 +0000" stop="20261019010000 +0000" channel="other"><title>Nope</title></programme>
<programme start="20261020000000 +0000" stop="20261020003000 +0000" channel="abc.au"><title>Tomorrow</title></programme>
</tv>"#;
        let from = parse_xmltv_time("20261019000500 +0000").unwrap();
        let to = from + 6 * 3_600_000;
        let abc = Channel { id: "abc.au".into(), name: "ABC TV".into(), number: None, logo: None, group: None, url: "u".into() };
        let by_id: HashMap<&str, &Channel> = [("abc.au", &abc)].into_iter().collect();
        let by_name: HashMap<String, &Channel> = [(normalise(&abc.name), &abc)].into_iter().collect();
        let (progs, _) = parse_xmltv(xml, from, to, &by_id, &by_name);
        assert_eq!(progs.len(), 1);
        assert_eq!(progs[0].title, "News");
        assert_eq!(progs[0].description.as_deref(), Some("Tonight's news."));

        // Guide ids that differ from the playlist are matched through display names.
        let xml2 = r#"<tv><channel id="guide-7"><display-name>ABC TV (1080p)</display-name></channel>
<programme start="20261019000000 +0000" stop="20261019003000 +0000" channel="guide-7"><title>Matched</title></programme></tv>"#;
        let (progs2, aliased) = parse_xmltv(xml2, from, to, &by_id, &by_name);
        assert_eq!(aliased, 1);
        assert_eq!(progs2.len(), 1);
        assert_eq!(progs2[0].channel, "abc.au");
    }
}
