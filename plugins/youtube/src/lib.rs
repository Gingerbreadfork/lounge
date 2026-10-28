//! YouTube channels: the public feed of each channel or playlist you list becomes a "New on
//! YouTube" row, newest first, opened in the YouTube app. No account and no API key: the feeds
//! are the ones YouTube publishes for every channel.
use lounge_plugin_sdk::{self as sdk, host, Action, HttpRequest, Item, Kind, Manifest, RefreshContext, RowSpec, RowStyle, SettingKind, SettingSpec};
use quick_xml::events::Event;
use std::collections::BTreeMap;

pub struct YouTube;

const APP_PACKAGE: &str = "com.google.android.youtube.tv";
const MAX_FEEDS: usize = 40;
const ART_KEY: &str = "hd_art";
/// With `MAX_FEEDS`, stays under the host's fetches per call.
const ART_CHECKS: usize = 20;

impl sdk::Plugin for YouTube {
    fn manifest() -> Manifest {
        Manifest {
            id: "tv.lounge.youtube".into(),
            name: "YouTube channels".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            description: "New videos from the channels and playlists you follow, without an account.".into(),
            publisher_name: "Lounge".into(),
            kinds: vec![Kind::Content],
            fetch_hosts: vec!["www.youtube.com".into(), "*.ytimg.com".into()],
            refresh_minutes: 30,
            rows: vec![RowSpec { id: "new".into(), title: "New on YouTube".into(), style: RowStyle::Landscape }],
            settings: vec![
                SettingSpec {
                    key: "channels".into(),
                    label: "Channels".into(),
                    kind: SettingKind::Text,
                    required: true,
                    help: Some("Channel links or ids (UC…), playlist links or ids (PL…), one per line or separated by commas. Links with an @handle need the channel id instead: open the channel, View page source, and search for channelId.".into()),
                    default: None,
                },
                SettingSpec { key: "per_source".into(), label: "Videos per channel".into(), kind: SettingKind::Number, required: false, help: None, default: Some("4".into()) },
                SettingSpec { key: "days".into(), label: "Only the last … days".into(), kind: SettingKind::Number, required: false, help: Some("0 shows the feed's newest videos whatever their age.".into()), default: Some("30".into()) },
                SettingSpec { key: "shorts".into(), label: "Include Shorts".into(), kind: SettingKind::Toggle, required: false, help: None, default: Some("false".into()) },
            ],
            homepage: None,
        }
    }

    fn refresh(ctx: &RefreshContext) -> Result<Vec<Item>, String> {
        let raw = host::required_setting("channels", "Channels")?;
        let sources = parse_sources(&raw)?;
        if sources.is_empty() {
            return Err("No channels yet: add a channel link or id in the plugin's settings".into());
        }
        let per_source = number_setting("per_source", 4).clamp(1, 15) as usize;
        let days = number_setting("days", 30).clamp(0, 3_650);
        let since = if days == 0 { 0 } else { ctx.now_ms - days * 86_400_000 };
        let shorts = host::setting("shorts").is_some_and(|v| v == "true");
        let mut videos = Vec::new();
        let mut failures = Vec::new();
        for source in sources.iter().take(MAX_FEEDS) {
            match feed(source) {
                Ok(xml) => {
                    let mut parsed = parse_feed(&xml);
                    parsed.retain(|v| v.published_ms >= since && (shorts || !v.short));
                    parsed.truncate(per_source);
                    videos.extend(parsed);
                }
                Err(e) => failures.push((source.label(), e)),
            }
        }
        for (label, e) in &failures {
            sdk::warn!("{label}: {e}");
        }
        if videos.is_empty()
            && let Some((_, e)) = failures.into_iter().next()
        {
            return Err(e);
        }
        videos.sort_by_key(|v| std::cmp::Reverse(v.published_ms));
        let mut seen = std::collections::HashSet::new();
        videos.retain(|v| seen.insert(v.id.clone()));
        let recent = ctx.now_ms - 3 * 86_400_000;
        let count = videos.len();
        let hd = hd_art(&videos);
        sdk::info!("{count} videos from {} feeds, {} with large pictures", sources.len().min(MAX_FEEDS), hd.values().filter(|v| **v).count());
        Ok(videos
            .into_iter()
            .enumerate()
            .map(|(i, v)| Item {
                id: format!("video:{}", v.id),
                title: v.title,
                subtitle: Some(v.channel),
                added_ms: Some(v.published_ms),
                is_new: v.published_ms >= recent,
                thumbnail: Some(thumbnail(&v.id, hd.get(&v.id) == Some(&true))),
                row: Some("new".into()),
                weight: (count - i) as i32,
                action: Action { view: Some(format!("https://www.youtube.com/watch?v={}", v.id)), fallback_launch: Some(APP_PACKAGE.into()), ..Default::default() },
                ..Default::default()
            })
            .collect())
    }
}

sdk::export_plugin!(YouTube);

/// A source's feed, or what went wrong reading it as a sentence for the TV.
fn feed(source: &Source) -> Result<String, String> {
    let r = sdk::fetch_online("YouTube", &HttpRequest::get(source.feed_url()))?;
    match r.status {
        404 => Err(format!("YouTube has no {}. Check the channels in the plugin's settings.", source.label())),
        s if !(200..300).contains(&s) => Err(sdk::online_error("YouTube", s)),
        _ => Ok(if r.body_base64.is_some() { String::from_utf8_lossy(&sdk::body_bytes(r)).into_owned() } else { r.body }),
    }
}

/// The 1280×720 picture when YouTube has one, else the 480×360 one every video has.
fn thumbnail(id: &str, hd: bool) -> String {
    format!("https://i.ytimg.com/vi/{id}/{}.jpg", if hd { "hq720" } else { "hqdefault" })
}

/// Which videos have the large picture. YouTube answers 404 for the ones that do not, so each
/// is asked once and the answer kept; a few unchecked ones per refresh fall back to the small
/// picture until next time.
fn hd_art(videos: &[Video]) -> BTreeMap<String, bool> {
    let mut known: BTreeMap<String, bool> = host::kv_get_string(ART_KEY).and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
    known.retain(|id, _| videos.iter().any(|v| &v.id == id));
    let mut checks = 0;
    for v in videos {
        if known.contains_key(&v.id) || checks >= ART_CHECKS {
            continue;
        }
        checks += 1;
        let mut req = HttpRequest::get(thumbnail(&v.id, true));
        req.method = "HEAD".into();
        match host::fetch(&req) {
            Ok(r) if r.status == 200 || r.status == 404 => {
                known.insert(v.id.clone(), r.status == 200);
            }
            _ => {}
        }
    }
    if checks > 0 {
        host::kv_set(ART_KEY, serde_json::to_string(&known).unwrap_or_default().as_bytes());
    }
    known
}

fn number_setting(key: &str, default: i64) -> i64 {
    host::setting(key).and_then(|v| v.trim().parse().ok()).unwrap_or(default)
}

/// A channel or a playlist, as the user wrote it.
#[derive(Debug, PartialEq, Eq)]
pub enum Source {
    Channel(String),
    Playlist(String),
}

impl Source {
    fn feed_url(&self) -> String {
        match self {
            Source::Channel(id) => format!("https://www.youtube.com/feeds/videos.xml?channel_id={id}"),
            Source::Playlist(id) => format!("https://www.youtube.com/feeds/videos.xml?playlist_id={id}"),
        }
    }

    fn label(&self) -> String {
        match self {
            Source::Channel(id) => format!("channel {id}"),
            Source::Playlist(id) => format!("playlist {id}"),
        }
    }
}

/// Accepts channel ids, `/channel/UC…` links, playlist ids and `list=` links; refuses what it cannot
/// turn into a feed, naming the line so the user can fix it.
pub fn parse_sources(raw: &str) -> Result<Vec<Source>, String> {
    let mut out = Vec::new();
    for piece in raw.split(['\n', ',', ';']).map(str::trim).filter(|s| !s.is_empty()) {
        // A bare id pasted from a share link can carry "?si=…" after it.
        let token = id_token(piece);
        if let Some(id) = find_param(piece, "list=") {
            out.push(Source::Playlist(id));
        } else if let Some(id) = find_param(piece, "channel_id=") {
            out.push(Source::Channel(id));
        } else if let Some(rest) = piece.split("/channel/").nth(1) {
            out.push(Source::Channel(id_token(rest)));
        } else if let Some(rest) = piece.split("/playlist/").nth(1) {
            out.push(Source::Playlist(id_token(rest)));
        } else if token.starts_with("UC") && token.len() == 24 && !piece.contains('/') {
            out.push(Source::Channel(token));
        } else if ["PL", "UU", "OL", "FL", "RD"].iter().any(|p| token.starts_with(p)) && token.len() >= 13 && !piece.contains('/') {
            out.push(Source::Playlist(token));
        } else if piece.contains('@') {
            return Err(format!("\"{piece}\" is a handle; use the channel's id (starts with UC) instead"));
        } else {
            return Err(format!("\"{piece}\" is not a channel or playlist id or link. Check the channels in the plugin's settings."));
        }
    }
    out.dedup();
    Ok(out)
}

fn find_param(s: &str, key: &str) -> Option<String> {
    let start = s.find(key)? + key.len();
    Some(id_token(&s[start..]))
}

fn id_token(s: &str) -> String {
    s.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-').collect()
}

#[derive(Debug, Clone, PartialEq)]
pub struct Video {
    pub id: String,
    pub title: String,
    pub channel: String,
    pub published_ms: i64,
    /// A vertical Short rather than a video.
    pub short: bool,
}

/// Videos from YouTube's Atom feed, in feed order (newest first).
pub fn parse_feed(xml: &str) -> Vec<Video> {
    let mut reader = quick_xml::Reader::from_str(xml);
    let mut out = Vec::new();
    let mut buf = Vec::new();
    let mut path: Vec<String> = Vec::new();
    let mut text = String::new();
    let mut current: Option<Video> = None;
    let mut feed_author = String::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let name = e.name().as_ref().to_string();
                if name == "entry" {
                    current = Some(Video { id: String::new(), title: String::new(), channel: feed_author.clone(), published_ms: 0, short: false });
                }
                path.push(name);
                text.clear();
            }
            Ok(Event::Text(t)) => {
                let raw = t.into_inner();
                text.push_str(&quick_xml::escape::unescape(raw.as_ref()).map(|c| c.into_owned()).unwrap_or_else(|_| raw.into_owned()));
            }
            Ok(Event::GeneralRef(r)) => text.push_str(&entity(&r)),
            // An entry's own link goes to /shorts/ when it is a Short.
            Ok(Event::Empty(e)) if e.name().as_ref() == "link" && path.last().is_some_and(|p| p == "entry") => {
                let href = e.try_get_attribute("href").ok().flatten().map(|a| a.value.into_owned()).unwrap_or_default();
                if let Some(v) = current.as_mut() {
                    v.short |= href.contains("/shorts/");
                }
            }
            Ok(Event::End(_)) => {
                let value = text.trim().to_string();
                text.clear();
                let in_entry = path.iter().any(|p| p == "entry");
                let tag = path.last().map(String::as_str).unwrap_or("");
                let parent = path.iter().rev().nth(1).map(String::as_str).unwrap_or("");
                if !value.is_empty() {
                    match (in_entry, tag, parent) {
                        (true, "yt:videoId", _) => current.as_mut().map(|v| v.id = value),
                        (true, "title", "entry") => current.as_mut().map(|v| v.title = value),
                        (true, "name", "author") => current.as_mut().map(|v| v.channel = value),
                        (true, "published", "entry") => current.as_mut().map(|v| v.published_ms = sdk::parse_iso8601(&value).unwrap_or(0)),
                        (false, "name", "author") => {
                            feed_author = value;
                            None
                        }
                        _ => None,
                    };
                }
                if tag == "entry" {
                    out.extend(current.take().filter(|v| !v.id.is_empty() && !v.title.is_empty()));
                }
                path.pop();
            }
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    out
}

/// The text an entity reference stands for: `&amp;`, `&#8217;` and friends.
fn entity(r: &quick_xml::events::BytesRef) -> String {
    if let Ok(Some(c)) = r.resolve_char_ref() {
        return c.to_string();
    }
    match r.as_ref() {
        "amp" => "&",
        "lt" => "<",
        "gt" => ">",
        "quot" => "\"",
        "apos" => "'",
        _ => "",
    }
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    const FEED: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<feed xmlns:yt="http://www.youtube.com/xml/schemas/2015" xmlns:media="http://search.yahoo.com/mrss/" xmlns="http://www.w3.org/2005/Atom">
 <title>Marques Brownlee</title>
 <author><name>Marques Brownlee</name><uri>https://www.youtube.com/channel/UCBJycsmduvYEL83R_U4JriQ</uri></author>
 <entry>
  <id>yt:video:scwlR0QHGOk</id>
  <yt:videoId>scwlR0QHGOk</yt:videoId>
  <title>The NEW Tesla Roadster: My Honest Reaction</title>
  <author><name>Marques Brownlee</name></author>
  <published>2026-10-17T02:09:23+00:00</published>
  <media:group><media:title>The NEW Tesla Roadster: My Honest Reaction</media:title><media:thumbnail url="https://i4.ytimg.com/vi/scwlR0QHGOk/hqdefault.jpg" width="480" height="360"/></media:group>
 </entry>
 <entry>
  <yt:videoId>sh0rtsVid01</yt:videoId>
  <title>One minute on the new phone</title>
  <link rel="alternate" href="https://www.youtube.com/shorts/sh0rtsVid01"/>
  <published>2026-10-12T10:00:00+00:00</published>
 </entry>
 <entry>
  <yt:videoId>abc123DEF45</yt:videoId>
  <link rel="alternate" href="https://www.youtube.com/watch?v=abc123DEF45"/>
  <title>Second &amp; last, it&#8217;s here</title>
  <author><name>Marques Brownlee</name></author>
  <published>2026-10-10T10:00:00+00:00</published>
 </entry>
</feed>"#;

    #[test]
    fn feed_entries_are_read_in_order() {
        let v = parse_feed(FEED);
        assert_eq!(v.len(), 3);
        assert_eq!(v[0].id, "scwlR0QHGOk");
        assert_eq!(v[0].title, "The NEW Tesla Roadster: My Honest Reaction");
        assert_eq!(v[0].channel, "Marques Brownlee");
        assert_eq!(v[0].published_ms, 1792202963000);
        assert_eq!(v.iter().map(|v| v.short).collect::<Vec<_>>(), [false, true, false]);
        assert_eq!(v[2].title, "Second & last, it\u{2019}s here");
    }

    #[test]
    fn sources_come_in_every_shape() {
        let s = parse_sources("UCBJycsmduvYEL83R_U4JriQ, https://www.youtube.com/channel/UCBJycsmduvYEL83R_U4JriQ/videos\nhttps://www.youtube.com/playlist?list=PLx1234567890abc\nPLx1234567890abc").unwrap();
        assert_eq!(s, vec![Source::Channel("UCBJycsmduvYEL83R_U4JriQ".into()), Source::Playlist("PLx1234567890abc".into())]);
        assert!(parse_sources("https://www.youtube.com/@mkbhd").unwrap_err().contains("handle"));
        assert_eq!(parse_sources("PLx1234567890abc?si=Zx9").unwrap(), vec![Source::Playlist("PLx1234567890abc".into())]);
        assert_eq!(parse_sources("not a thing").unwrap_err(), "\"not a thing\" is not a channel or playlist id or link. Check the channels in the plugin's settings.");
    }

    #[test]
    fn feed_urls_point_at_youtube() {
        assert_eq!(Source::Channel("UCx".into()).feed_url(), "https://www.youtube.com/feeds/videos.xml?channel_id=UCx");
        assert_eq!(Source::Playlist("PLy".into()).feed_url(), "https://www.youtube.com/feeds/videos.xml?playlist_id=PLy");
    }
}
