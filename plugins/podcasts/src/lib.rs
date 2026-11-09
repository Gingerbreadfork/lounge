//! Podcasts: the RSS feed of each show you list becomes new episodes on the home, handed to
//! a player app on the TV, with a tile per show that plays its latest. Apple Podcasts links
//! and OPML exports are turned into feeds, and Apple's directory answers search. No account:
//! the feeds are the ones every podcast publishes. Feeds are asked only for what changed, and
//! a feed too big to read whole is read from its start, where the newest episodes are.
use lounge_plugin_sdk::{self as sdk, host, Action, HttpRequest, HttpResponse, Item, ItemKind, Kind, Manifest, RefreshContext, RowSpec, RowStyle, SettingKind, SettingSpec};
use quick_xml::events::Event;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub struct Podcasts;

const ITUNES: &str = "https://itunes.apple.com";
const FEEDS_KEY: &str = "feeds";
const APPLE_KEY: &str = "apple";
const OPML_KEY: &str = "opml";
const STOREFRONT_KEY: &str = "storefront";
/// Feeds read in one refresh; the rest keep last time's episodes and come first next time.
const FEEDS_PER_REFRESH: usize = 40;
/// Feed bytes read in one refresh, inside the fuel the host allows for decoding them.
const BYTES_PER_REFRESH: usize = 100 << 20;
/// How much of a feed too big to read whole is asked for: its start, holding the newest
/// episodes.
const PARTIAL_BYTES: usize = 3 << 20;
/// Episodes kept per show between refreshes.
const STORED_PER_FEED: usize = 10;
const MOST_EPISODES: usize = 200;
/// The key-value store takes 256 KB a key; the cache stays well under it.
const CACHE_BYTES: usize = 200 * 1024;
const DAY_MS: i64 = 86_400_000;
const FEED_TIMEOUT_MS: u32 = 10_000;
const RESOLVE_TIMEOUT_MS: u32 = 8_000;
/// Tags whose text is read; everything else, show notes above all, is skipped unread.
const READ_TAGS: [&str; 12] = ["title", "guid", "id", "pubDate", "published", "updated", "itunes:duration", "itunes:season", "itunes:episode", "itunes:episodeType", "itunes:image", "url"];
/// Items read from one feed. Feeds list newest first, and a long-running show's early years
/// are not worth the fuel.
const MOST_ITEMS_READ: usize = 100;
const OUT_OF_DATE: &str = "This tile is out of date. Refresh the plugin, then try again.";

impl sdk::Plugin for Podcasts {
    fn manifest() -> Manifest {
        Manifest {
            id: "tv.lounge.podcasts".into(),
            name: "Podcasts".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            description: "New episodes of the podcasts you follow, from their feeds, played in a player app such as VLC.".into(),
            publisher_name: "Lounge".into(),
            kinds: vec![Kind::Content, Kind::Search],
            fetch_hosts: vec!["itunes.apple.com".into(), "*".into()],
            refresh_minutes: 30,
            rows: vec![
                RowSpec { id: "new".into(), title: "New podcast episodes".into(), style: RowStyle::Landscape },
                RowSpec { id: "shows".into(), title: "Your podcasts".into(), style: RowStyle::Landscape },
            ],
            settings: vec![
                SettingSpec {
                    key: "feeds".into(),
                    label: "Podcasts you follow".into(),
                    kind: SettingKind::Text,
                    required: false,
                    help: Some("Feed links, Apple Podcasts links or a link to an OPML export, separated by spaces or commas. An OPML file from another podcast app can be pasted in whole. Lounge's search finds podcasts too, and plays their latest episode.".into()),
                    default: None,
                },
                SettingSpec { key: "per_feed".into(), label: "Episodes per podcast".into(), kind: SettingKind::Number, required: false, help: Some("From 1 to 10.".into()), default: Some("3".into()) },
                SettingSpec { key: "days".into(), label: "Only the last … days".into(), kind: SettingKind::Number, required: false, help: Some("0 shows each feed's newest episodes whatever their age.".into()), default: Some("30".into()) },
                SettingSpec { key: "shows".into(), label: "Your podcasts row".into(), kind: SettingKind::Toggle, required: false, help: Some("A tile per podcast that plays its latest episode.".into()), default: Some("true".into()) },
                SettingSpec { key: "country".into(), label: "Country for search".into(), kind: SettingKind::Text, required: false, help: Some("Two letters, like au or gb, so search finds the podcasts of your country first. Left empty, it follows your Apple links.".into()), default: None },
            ],
            homepage: None,
            self_signed_servers: false,
            for_apps: Vec::new(),
        }
    }

    fn refresh(ctx: &RefreshContext) -> Result<Vec<Item>, String> {
        let sources = parse_sources(&host::setting("feeds").unwrap_or_default())?;
        if sources.is_empty() {
            return Err("No podcasts yet: add a feed link, an Apple Podcasts link or an OPML export in the plugin's settings".into());
        }
        let per_feed = number_setting("per_feed", 3).clamp(1, STORED_PER_FEED as i64) as usize;
        let days = number_setting("days", 30).clamp(0, 3_650);
        let since = if days == 0 { 0 } else { ctx.now_ms - days * DAY_MS };
        let mut budget = Budget::default();
        let mut failures: Vec<String> = Vec::new();
        let feeds = resolve_sources(sources, &mut budget, &mut failures);

        let mut cache = FeedCache::load();
        let dead = cache.refresh(&feeds, ctx.now_ms, &mut budget, &mut failures);
        cache.save();
        forget_apple(&dead);
        for e in &failures {
            sdk::warn!("{e}");
        }
        if cache.shows.values().all(|s| s.episodes.is_empty())
            && let Some(e) = failures.into_iter().next()
        {
            return Err(e);
        }

        let mut episodes: Vec<(&Show, &Episode)> = cache.shows.values().flat_map(|s| s.episodes.iter().filter(|e| e.published_ms >= since && !e.trailer).take(per_feed).map(move |e| (s, e))).collect();
        episodes.sort_by_key(|(_, e)| std::cmp::Reverse(e.published_ms));
        episodes.truncate(MOST_EPISODES);
        let recent = ctx.now_ms - 3 * DAY_MS;
        let count = episodes.len();
        let mut items: Vec<Item> = episodes.into_iter().enumerate().map(|(i, (show, e))| episode_item(show, e, (count - i) as i32, e.published_ms >= recent)).collect();
        if host::setting("shows").map(|v| v != "false").unwrap_or(true) {
            let mut shows: Vec<&Show> = cache.shows.values().filter(|s| s.latest().is_some()).collect();
            shows.sort_by_key(|s| std::cmp::Reverse(s.latest().map_or(0, |e| e.published_ms)));
            let total = shows.len();
            items.extend(shows.into_iter().enumerate().filter_map(|(i, s)| show_item(s, (total - i) as i32)));
        }
        sdk::dedup_ids(&mut items);
        let shows = cache.shows.values().filter(|s| !s.episodes.is_empty()).count();
        sdk::info!("{count} episode{} from {shows} podcast{}, {} read this time", plural(count), plural(shows), budget.fetches);
        Ok(items)
    }

    fn search(query: &str, limit: u32) -> Result<Vec<Item>, String> {
        let query = query.trim();
        if query.is_empty() {
            return Ok(Vec::new());
        }
        let country = host::setting("country").map(|c| c.trim().to_ascii_lowercase()).filter(|c| c.len() == 2).or_else(|| host::kv_get_string(STOREFRONT_KEY));
        let country = country.map(|c| format!("&country={c}")).unwrap_or_default();
        let found: Directory = fetch_json(&format!("{ITUNES}/search?media=podcast&entity=podcast&limit={}&term={}{country}", limit.clamp(1, 25), url_encode(query)))?;
        Ok(found
            .results
            .into_iter()
            .filter(|r| !r.feed_url.is_empty() && !r.collection_name.is_empty())
            .map(|r| Item {
                id: format!("found:{}", r.collection_id),
                kind: ItemKind::Program,
                title: r.collection_name.clone(),
                subtitle: Some(r.artist_name.clone()).filter(|a| !a.is_empty()),
                thumbnail: r.artwork_url_600.clone().filter(|u| u.starts_with("https://")),
                action: Action { resolve: Some(format!("latest:{}", r.feed_url)), title: Some(r.collection_name), ..Default::default() },
                ..Default::default()
            })
            .collect())
    }

    /// Plays the latest episode of a podcast found through search.
    fn resolve(action: &str) -> Result<Action, String> {
        let Some(url) = action.strip_prefix("latest:") else {
            return Err(OUT_OF_DATE.into());
        };
        let fetched = match fetch_feed(url, None, RESOLVE_TIMEOUT_MS) {
            Ok(Fetched::Body(body)) => body,
            Ok(Fetched::Unchanged) => return Err(OUT_OF_DATE.into()),
            Err(FeedError::Budget) => return Err("Too much was asked for at once. Try again in a moment.".into()),
            Err(FeedError::Gone(e) | FeedError::TooLarge(e) | FeedError::Other(e)) => return Err(e.replace(" Check the link in the plugin's settings.", "")),
        };
        let show = parse_feed(&fetched.text, url);
        let latest = show.latest().ok_or_else(|| format!("{} has no episodes to play yet.", if show.title.is_empty() { "This podcast" } else { show.title.as_str() }))?;
        Ok(Action::play(&latest.url, &latest.title))
    }
}

sdk::export_plugin!(Podcasts);

fn number_setting(key: &str, default: i64) -> i64 {
    host::setting(key).and_then(|v| v.trim().parse().ok()).unwrap_or(default)
}

fn plural(n: usize) -> &'static str {
    if n == 1 { "" } else { "s" }
}

/// What one refresh may still read: the host's fetches, kept inside its fuel by counting bytes.
#[derive(Default)]
struct Budget {
    fetches: usize,
    bytes: usize,
    spent: bool,
}

impl Budget {
    fn feeds_left(&self) -> bool {
        !self.spent && self.fetches < FEEDS_PER_REFRESH && self.bytes < BYTES_PER_REFRESH
    }
}

fn fetch_json<T: serde::de::DeserializeOwned>(url: &str) -> Result<T, String> {
    let r = sdk::fetch_online("Apple Podcasts", &HttpRequest::get(url).header("Accept", "application/json"))?;
    if !r.ok() {
        sdk::warn!("HTTP {} for {}", r.status, sdk::redact_url(url));
        return Err(sdk::online_error("Apple Podcasts", r.status));
    }
    serde_json::from_str(&r.body).map_err(|e| {
        sdk::warn!("unexpected answer from Apple: {e}");
        sdk::unreadable("Apple Podcasts", ITUNES)
    })
}

// ---------------------------------------------------------------------------
// Reading a feed: only what changed, and only the start of one too big
// ---------------------------------------------------------------------------

/// Why a feed did not come: it is gone, it is too big even in part, the refresh's fetches are
/// used up, or something else said in a sentence.
enum FeedError {
    Gone(String),
    TooLarge(String),
    Budget,
    Other(String),
}

/// A feed's text with what the server said about it, or that nothing has changed.
enum Fetched {
    Unchanged,
    Body(FeedBody),
}

struct FeedBody {
    text: String,
    etag: Option<String>,
    last_modified: Option<String>,
    /// Only the start of the feed was asked for.
    partial: bool,
    /// What the answer cost to read, in bytes, with the slow kind counted several times over.
    weight: usize,
}

/// The feed at `url`. With what was kept from last time, the server is asked only for what
/// changed; a feed the host finds too big is asked for again from its start.
fn fetch_feed(url: &str, cached: Option<&Show>, timeout_ms: u32) -> Result<Fetched, FeedError> {
    let partial = cached.is_some_and(|s| s.partial);
    match fetch_feed_once(url, cached, partial, timeout_ms) {
        Err(FeedError::TooLarge(_)) if !partial => fetch_feed_once(url, None, true, timeout_ms),
        other => other,
    }
}

fn fetch_feed_once(url: &str, cached: Option<&Show>, partial: bool, timeout_ms: u32) -> Result<Fetched, FeedError> {
    let mut req = HttpRequest::get(url).header("Accept", "application/rss+xml, application/atom+xml, application/xml, text/xml, */*").header("User-Agent", "Lounge/0.5 (Android TV podcast reader)");
    req.timeout_ms = timeout_ms;
    if partial {
        req = req.header("Range", &format!("bytes=0-{}", PARTIAL_BYTES - 1)).header("Accept-Encoding", "identity");
    } else if let Some(c) = cached {
        if let Some(etag) = &c.etag {
            req = req.header("If-None-Match", etag);
        }
        if let Some(when) = &c.last_modified {
            req = req.header("If-Modified-Since", when);
        }
    }
    let r = host::fetch(&req).map_err(|e| {
        if e == host::TOO_LARGE {
            FeedError::TooLarge(format!("The feed at {} is too big to read, even in part. Only its newest episodes are needed; the publisher's other links may give a smaller feed.", sdk::address(url)))
        } else if e.starts_with("fetch budget") {
            FeedError::Budget
        } else {
            sdk::warn!("{e}");
            FeedError::Other(format!("{} didn't answer. Check the link in the plugin's settings and the TV's connection.", sdk::address(url)))
        }
    })?;
    match r.status {
        304 => Ok(Fetched::Unchanged),
        404 | 410 => Err(FeedError::Gone(format!("There is no feed at {} any more. Check the link in the plugin's settings.", sdk::address(url)))),
        s if !(200..300).contains(&s) => Err(FeedError::Other(format!("{} answered with error {s}. Try again later.", sdk::address(url)))),
        s => {
            let header = |name: &str| r.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.trim().to_string()).filter(|v| !v.is_empty());
            let etag = header("etag");
            let last_modified = header("last-modified");
            let (text, weight) = feed_text(r);
            Ok(Fetched::Body(FeedBody { text, etag, last_modified, partial: partial && s == 206, weight }))
        }
    }
}

/// A feed's text from the answer. Text that is not UTF-8 is read as Latin-1, which is what a
/// feed that says so, or says nothing, almost always is; it costs the host several times the
/// fuel, and weighs accordingly.
fn feed_text(r: HttpResponse) -> (String, usize) {
    if r.body_base64.is_none() {
        let weight = r.body.len();
        return (r.body, weight);
    }
    let bytes = sdk::body_bytes(r);
    let weight = bytes.len() * 6;
    let head = String::from_utf8_lossy(&bytes[..bytes.len().min(200)]).to_ascii_lowercase();
    let text = if head.contains("iso-8859-1") || head.contains("windows-1252") || head.contains("latin1") || !head.contains("utf-8") {
        bytes.iter().map(|&b| b as char).collect()
    } else {
        String::from_utf8_lossy(&bytes).into_owned()
    };
    (text, weight)
}

// ---------------------------------------------------------------------------
// What the person listed: feed links, Apple links, OPML
// ---------------------------------------------------------------------------

/// A podcast as the person wrote it: its feed, an Apple Podcasts page to ask Apple about, or
/// an OPML export holding feeds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    Feed(String),
    Apple(String),
    Opml(String),
}

/// Accepts feed links, Apple Podcasts links, links to OPML files and pasted OPML, separated by
/// line breaks, spaces, commas or semicolons; refuses what is none of those, naming it so the
/// person can fix it.
pub fn parse_sources(raw: &str) -> Result<Vec<Source>, String> {
    let mut out = Vec::new();
    if raw.contains("xmlUrl=") || raw.contains("<opml") {
        for url in opml_feeds(raw) {
            push_unique(&mut out, Source::Feed(url));
        }
        return Ok(out);
    }
    for piece in raw.split_whitespace().flat_map(links_in) {
        let lower = piece.to_ascii_lowercase();
        let source = if let Some(id) = apple_id(piece) {
            if let Some(storefront) = apple_storefront(piece) {
                host::kv_set(STOREFRONT_KEY, storefront.as_bytes());
            }
            Source::Apple(id)
        } else if is_apple(&lower) {
            return Err(format!("\"{piece}\" is an Apple link without a podcast id. Open the podcast's own page and copy that link."));
        } else {
            let url = match lower.find("://") {
                Some(i) if ["http", "https"].contains(&&lower[..i]) => format!("{}{}", &lower[..i + 3], &piece[i + 3..]),
                Some(i) if ["feed", "itpc", "pcast", "podcast"].contains(&&lower[..i]) => format!("https://{}", &piece[i + 3..]),
                Some(_) => return Err(format!("\"{piece}\" is not a link Lounge can fetch. Check the podcasts in the plugin's settings.")),
                None if piece.contains('.') => format!("https://{piece}"),
                None => return Err(format!("\"{piece}\" is not a feed link or an Apple Podcasts link. Check the podcasts in the plugin's settings.")),
            };
            if lower.contains(".opml") || lower.contains("/opml") || lower.contains("format=opml") { Source::Opml(url) } else { Source::Feed(url) }
        };
        push_unique(&mut out, source);
    }
    Ok(out)
}

/// The links in one unbroken run of text: split at commas and semicolons, unless what follows
/// one is no link of its own and so belongs to the address before it.
fn links_in(run: &str) -> Vec<&str> {
    let parts: Vec<&str> = run.split([',', ';']).filter(|p| !p.is_empty()).collect();
    if parts.iter().skip(1).all(|p| p.contains('.')) { parts } else { vec![run] }
}

/// Whether a lowercased link leads to Apple itself, not merely to a name that ends like it.
fn is_apple(lower: &str) -> bool {
    let rest = lower.split_once("://").map_or(lower, |(_, rest)| rest);
    let host = rest.split(['/', '?', '#', ':']).next().unwrap_or("");
    host == "apple.com" || host.ends_with(".apple.com")
}

fn push_unique<T: PartialEq>(list: &mut Vec<T>, value: T) {
    if !list.contains(&value) {
        list.push(value);
    }
}

/// The `xmlUrl` of every outline in an OPML document, in order.
pub fn opml_feeds(opml: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = opml;
    while let Some(i) = rest.find("xmlUrl=") {
        rest = &rest[i + 7..];
        let Some(quote) = rest.chars().next().filter(|c| *c == '"' || *c == '\'') else { continue };
        let Some(end) = rest[1..].find(quote) else { break };
        let url = unescape_soft(&rest[1..1 + end]).trim().to_string();
        if (url.starts_with("http://") || url.starts_with("https://")) && !out.contains(&url) {
            out.push(url);
        }
        rest = &rest[1 + end..];
    }
    out
}

/// The podcast id in an Apple link, or in one of the links that carry it: `/id123`, `?id=123`,
/// `overcast.fm/itunes123`, `pod.link/123`.
pub fn apple_id(link: &str) -> Option<String> {
    let lower = link.to_ascii_lowercase();
    let digits = |from: usize| -> Option<String> {
        let id: String = link[from..].chars().take_while(|c| c.is_ascii_digit()).collect();
        (id.len() >= 6).then_some(id)
    };
    if lower.contains("podcasts.apple.com/") || lower.contains("itunes.apple.com/") {
        if let Some(i) = lower.rfind("/id") {
            return digits(i + 3);
        }
        if let Some(i) = lower.find("?id=").or_else(|| lower.find("&id=")) {
            return digits(i + 4);
        }
        return None;
    }
    if let Some(i) = lower.find("overcast.fm/itunes") {
        return digits(i + "overcast.fm/itunes".len());
    }
    if let Some(i) = lower.find("pod.link/") {
        return digits(i + "pod.link/".len());
    }
    None
}

/// The two-letter storefront in an Apple link, such as `au` in `podcasts.apple.com/au/…`.
pub fn apple_storefront(link: &str) -> Option<String> {
    let lower = link.to_ascii_lowercase();
    let after = lower.find("apple.com/")? + "apple.com/".len();
    let code: String = lower[after..].chars().take_while(|c| c.is_ascii_alphabetic()).collect();
    (code.len() == 2).then_some(code)
}

#[derive(Deserialize, Default)]
struct Directory {
    #[serde(default)]
    results: Vec<DirectoryEntry>,
}

#[derive(Deserialize, Default, Clone)]
struct DirectoryEntry {
    #[serde(rename = "collectionId", default)]
    collection_id: i64,
    #[serde(rename = "collectionName", default)]
    collection_name: String,
    #[serde(rename = "artistName", default)]
    artist_name: String,
    #[serde(rename = "feedUrl", default)]
    feed_url: String,
    #[serde(rename = "artworkUrl600", default)]
    artwork_url_600: Option<String>,
}

/// What Apple said about a podcast id, kept so it is asked once.
#[derive(Serialize, Deserialize, Default, Clone)]
struct AppleKnown {
    feed: String,
    name: String,
    art: Option<String>,
}

/// A feed to read, with where it came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeedRef {
    pub url: String,
    pub apple_id: Option<String>,
    pub art: Option<String>,
}

fn apple_known() -> BTreeMap<String, AppleKnown> {
    host::kv_get(APPLE_KEY).and_then(|v| serde_json::from_slice(&v).ok()).unwrap_or_default()
}

/// Apple links become feeds through one directory call for all the ids not yet known, and
/// OPML links through their file; what comes back is remembered.
fn resolve_sources(sources: Vec<Source>, budget: &mut Budget, failures: &mut Vec<String>) -> Vec<FeedRef> {
    let mut known = apple_known();
    let unknown: Vec<String> = sources.iter().filter_map(|s| if let Source::Apple(id) = s { (!known.contains_key(id)).then(|| id.clone()) } else { None }).collect();
    if !unknown.is_empty() {
        budget.fetches += 1;
        match fetch_json::<Directory>(&format!("{ITUNES}/lookup?id={}", unknown.join(","))) {
            Ok(d) => {
                for r in d.results.into_iter().filter(|r| r.collection_id > 0) {
                    if r.feed_url.is_empty() {
                        failures.push(format!("{} is only on Apple Podcasts, with no feed to read.", if r.collection_name.is_empty() { "A podcast you listed".to_string() } else { r.collection_name.clone() }));
                        continue;
                    }
                    known.insert(r.collection_id.to_string(), AppleKnown { feed: r.feed_url, name: r.collection_name, art: r.artwork_url_600.filter(|u| u.starts_with("https://")) });
                }
                for id in &unknown {
                    if !known.contains_key(id) && !failures.iter().any(|f| f.contains("only on Apple")) {
                        failures.push(format!("Apple has no podcast with the id {id}. Check the link in the plugin's settings."));
                    }
                }
                host::kv_set(APPLE_KEY, serde_json::to_string(&known).unwrap_or_default().as_bytes());
            }
            Err(e) => failures.push(e),
        }
    }
    let mut opml_known: BTreeMap<String, Vec<String>> = host::kv_get(OPML_KEY).and_then(|v| serde_json::from_slice(&v).ok()).unwrap_or_default();
    let mut feeds: Vec<FeedRef> = Vec::new();
    for source in sources {
        match source {
            Source::Feed(url) => push_unique(&mut feeds, FeedRef { url, apple_id: None, art: None }),
            Source::Apple(id) => {
                if let Some(k) = known.get(&id) {
                    let feed = FeedRef { url: k.feed.clone(), apple_id: Some(id), art: k.art.clone() };
                    // The same show listed by its feed as well is read once, with Apple's cover.
                    match feeds.iter_mut().find(|f| f.url == feed.url) {
                        Some(f) => {
                            f.apple_id = feed.apple_id;
                            f.art = feed.art;
                        }
                        None => feeds.push(feed),
                    }
                }
            }
            Source::Opml(url) => {
                budget.fetches += 1;
                let listed = match fetch_feed(&url, None, FEED_TIMEOUT_MS) {
                    Ok(Fetched::Body(body)) => {
                        let listed = opml_feeds(&body.text);
                        if listed.is_empty() {
                            failures.push(format!("{} holds no podcast feeds. Check the link in the plugin's settings.", sdk::address(&url)));
                        } else {
                            opml_known.insert(url.clone(), listed.clone());
                            host::kv_set(OPML_KEY, serde_json::to_string(&opml_known).unwrap_or_default().as_bytes());
                        }
                        listed
                    }
                    Ok(Fetched::Unchanged) => opml_known.get(&url).cloned().unwrap_or_default(),
                    Err(FeedError::Budget) => {
                        budget.spent = true;
                        opml_known.get(&url).cloned().unwrap_or_default()
                    }
                    Err(FeedError::Gone(e) | FeedError::TooLarge(e) | FeedError::Other(e)) => {
                        failures.push(e);
                        opml_known.get(&url).cloned().unwrap_or_default()
                    }
                };
                for feed in listed {
                    push_unique(&mut feeds, FeedRef { url: feed, apple_id: None, art: None });
                }
            }
        }
    }
    feeds
}

/// Forgets what Apple said about podcasts whose feeds have gone, so Apple is asked again.
fn forget_apple(ids: &[String]) {
    if ids.is_empty() {
        return;
    }
    let mut known = apple_known();
    for id in ids {
        known.remove(id);
    }
    host::kv_set(APPLE_KEY, serde_json::to_string(&known).unwrap_or_default().as_bytes());
}

// ---------------------------------------------------------------------------
// Feeds, read a few dozen a refresh and kept between refreshes
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize, Default, Clone, Debug, PartialEq)]
pub struct Show {
    pub url: String,
    pub title: String,
    pub image: Option<String>,
    /// When the feed was last read in full or found unchanged.
    #[serde(default)]
    pub fetched_ms: i64,
    /// When it was last asked for, so a failing feed does not go first every time.
    #[serde(default)]
    pub attempted_ms: i64,
    #[serde(default)]
    pub etag: Option<String>,
    #[serde(default)]
    pub last_modified: Option<String>,
    /// The feed is too big to read whole, so only its start is asked for.
    #[serde(default)]
    pub partial: bool,
    /// When the feed was last found too big even in part; it is left alone for a day.
    #[serde(default)]
    pub oversized_ms: i64,
    /// Newest first.
    #[serde(default)]
    pub episodes: Vec<Episode>,
}

impl Show {
    /// The newest episode that is an episode, not a trailer.
    pub fn latest(&self) -> Option<&Episode> {
        self.episodes.iter().find(|e| !e.trailer)
    }
}

#[derive(Serialize, Deserialize, Default, Clone, Debug, PartialEq)]
pub struct Episode {
    pub id: String,
    pub title: String,
    /// The audio or video file.
    pub url: String,
    pub published_ms: i64,
    pub duration_ms: Option<i64>,
    pub image: Option<String>,
    pub season: Option<u32>,
    pub number: Option<u32>,
    #[serde(default)]
    pub trailer: bool,
}

#[derive(Serialize, Deserialize, Default)]
struct FeedCache {
    shows: BTreeMap<String, Show>,
}

impl FeedCache {
    fn load() -> FeedCache {
        host::kv_get(FEEDS_KEY).and_then(|v| serde_json::from_slice(&v).ok()).unwrap_or_default()
    }

    fn save(&self) {
        let mut trimmed = FeedCache { shows: self.shows.clone() };
        for per_show in [STORED_PER_FEED, 5, 3, 1] {
            for s in trimmed.shows.values_mut() {
                s.episodes.truncate(per_show);
            }
            if let Ok(json) = serde_json::to_vec(&trimmed)
                && json.len() <= CACHE_BYTES
            {
                host::kv_set(FEEDS_KEY, &json);
                return;
            }
        }
        sdk::warn!("too many podcasts to remember between refreshes; starting over");
        host::kv_delete(FEEDS_KEY);
    }

    /// Reads the feeds not asked for longest, as many as fit, and forgets ones no longer
    /// listed. Returns the Apple ids of feeds that have gone, so Apple can be asked again.
    fn refresh(&mut self, feeds: &[FeedRef], now_ms: i64, budget: &mut Budget, failures: &mut Vec<String>) -> Vec<String> {
        self.shows.retain(|url, _| feeds.iter().any(|f| &f.url == url));
        let mut gone = Vec::new();
        let mut due: Vec<&FeedRef> = feeds.iter().collect();
        due.sort_by_key(|f| self.shows.get(&f.url).map_or(0, |s| s.attempted_ms));
        for f in due {
            if !budget.feeds_left() {
                break;
            }
            let entry = self.shows.entry(f.url.clone()).or_insert_with(|| Show { url: f.url.clone(), ..Default::default() });
            if entry.oversized_ms > 0 && now_ms - entry.oversized_ms < DAY_MS {
                continue;
            }
            budget.fetches += 1;
            entry.attempted_ms = now_ms;
            match fetch_feed(&f.url, Some(entry), FEED_TIMEOUT_MS) {
                Ok(Fetched::Unchanged) => entry.fetched_ms = now_ms,
                Ok(Fetched::Body(body)) => {
                    budget.bytes += body.weight;
                    let mut show = parse_feed(&body.text, &f.url);
                    if show.episodes.is_empty() {
                        if show.title.is_empty() {
                            failures.push(format!("{} is not a podcast feed. Check the link in the plugin's settings.", sdk::address(&f.url)));
                            if let Some(id) = &f.apple_id {
                                gone.push(id.clone());
                            }
                        } else {
                            failures.push(format!("{} has no episodes with a file to play.", show.title));
                        }
                        continue;
                    }
                    show.episodes.truncate(STORED_PER_FEED);
                    show.fetched_ms = now_ms;
                    show.attempted_ms = now_ms;
                    show.etag = body.etag;
                    show.last_modified = body.last_modified;
                    show.partial = body.partial;
                    // Apple's cover is sized for a screen; a feed's own can be too big to show.
                    if f.art.is_some() {
                        show.image = f.art.clone();
                    }
                    *entry = show;
                }
                Err(FeedError::Gone(e)) => {
                    failures.push(e);
                    if let Some(id) = &f.apple_id {
                        gone.push(id.clone());
                    }
                }
                Err(FeedError::TooLarge(e)) => {
                    failures.push(e);
                    entry.oversized_ms = now_ms;
                }
                Err(FeedError::Budget) => {
                    budget.spent = true;
                    break;
                }
                // A feed that did not come keeps last time's episodes.
                Err(FeedError::Other(e)) => failures.push(e),
            }
        }
        gone
    }
}

// ---------------------------------------------------------------------------
// Tiles
// ---------------------------------------------------------------------------

/// A length as the launcher writes one: "48 min", "2 h 25 min", "2 h".
pub fn length_text(ms: i64) -> String {
    let minutes = (ms + 30_000) / 60_000;
    match (minutes / 60, minutes % 60) {
        (0, m) => format!("{} min", m.max(1)),
        (h, 0) => format!("{h} h"),
        (h, m) => format!("{h} h {m} min"),
    }
}

fn episode_item(show: &Show, e: &Episode, weight: i32, is_new: bool) -> Item {
    let subtitle = match e.duration_ms {
        Some(d) if d > 0 => format!("{}, {}", show.title, length_text(d)),
        _ => show.title.clone(),
    };
    Item {
        id: format!("ep:{}", e.id),
        kind: ItemKind::Program,
        title: e.title.clone(),
        subtitle: Some(subtitle).filter(|s| !s.is_empty()),
        season: e.season.map(|s| s.to_string()).filter(|_| e.number.is_some()),
        episode: e.number.map(|n| n.to_string()).filter(|_| e.season.is_some()),
        duration_ms: e.duration_ms,
        added_ms: Some(e.published_ms),
        is_new,
        thumbnail: e.image.clone().or_else(|| show.image.clone()),
        row: Some("new".into()),
        weight,
        action: Action::play(&e.url, &e.title),
        ..Default::default()
    }
}

/// A podcast's own tile, which plays its latest episode.
fn show_item(show: &Show, weight: i32) -> Option<Item> {
    let latest = show.latest()?;
    Some(Item {
        id: format!("show:{}", fnv(&show.url)),
        kind: ItemKind::Program,
        title: show.title.clone(),
        subtitle: Some(format!("Latest: {}", latest.title)),
        thumbnail: show.image.clone().or_else(|| latest.image.clone()),
        row: Some("shows".into()),
        weight,
        action: Action::play(&latest.url, &latest.title),
        ..Default::default()
    })
}

// ---------------------------------------------------------------------------
// RSS, as podcasts publish it
// ---------------------------------------------------------------------------

/// A show and its episodes from an RSS or Atom feed, newest first. Episodes without a file
/// are left out; one without a date goes last. An unclosed tag in the feed's prose ends the
/// read where it stands, keeping what came before.
pub fn parse_feed(xml: &str, url: &str) -> Show {
    let mut reader = quick_xml::Reader::from_str(xml);
    reader.config_mut().allow_dangling_amp = true;
    let mut buf = Vec::new();
    let mut path: Vec<String> = Vec::new();
    let mut text = String::new();
    let mut reading = false;
    let mut items_read = 0;
    let mut show = Show { url: url.to_string(), ..Default::default() };
    let mut channel_image_from_tag = false;
    let mut current: Option<Draft> = None;
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let name = e.name().as_ref().to_string();
                if name == "item" || name == "entry" {
                    items_read += 1;
                    if items_read > MOST_ITEMS_READ {
                        break;
                    }
                    current = Some(Draft::default());
                }
                take_element(&name, &e, current.as_mut(), &mut show, &mut channel_image_from_tag);
                reading = READ_TAGS.contains(&name.as_str());
                path.push(name);
                text.clear();
            }
            Ok(Event::Empty(e)) => {
                let name = e.name().as_ref().to_string();
                take_element(&name, &e, current.as_mut(), &mut show, &mut channel_image_from_tag);
            }
            Ok(Event::Text(t)) if reading => text.push_str(&unescape_soft(t.into_inner().as_ref())),
            // Feeds often escape the text inside CDATA as well, so it is unescaped when it reads that way.
            Ok(Event::CData(c)) if reading => text.push_str(&unescape_soft(c.into_inner().as_ref())),
            Ok(Event::GeneralRef(r)) if reading => text.push_str(&entity(&r)),
            Ok(Event::End(_)) => {
                let value = text.trim().to_string();
                text.clear();
                reading = false;
                let tag = path.last().map(String::as_str).unwrap_or("");
                let parent = path.iter().rev().nth(1).map(String::as_str).unwrap_or("");
                if !value.is_empty() {
                    match current.as_mut() {
                        Some(d) => d.take_text(tag, parent, &value),
                        None => match (tag, parent) {
                            ("title", "channel") | ("title", "feed") => show.title = value,
                            // The RSS image is the fallback; iTunes' is the one sized for a screen.
                            ("url", "image") if !channel_image_from_tag => show.image = Some(value),
                            ("itunes:image", _) if value.starts_with("http") && !channel_image_from_tag => {
                                show.image = Some(value);
                                channel_image_from_tag = true;
                            }
                            _ => {}
                        },
                    }
                }
                if (tag == "item" || tag == "entry")
                    && let Some(episode) = current.take().and_then(|d| d.finish(url))
                {
                    show.episodes.push(episode);
                }
                path.pop();
            }
            Ok(Event::Eof) => break,
            Err(e) => {
                sdk::warn!("{}: the feed could not be read past where it stands: {e}", sdk::address(url));
                break;
            }
            _ => {}
        }
        buf.clear();
    }
    let mut seen = std::collections::HashSet::new();
    show.episodes.retain(|e| seen.insert(e.id.clone()));
    show.episodes.sort_by_key(|e| std::cmp::Reverse(e.published_ms));
    show
}

/// What an element's attributes say: an enclosure, an iTunes image, an Atom enclosure link.
fn take_element(name: &str, e: &quick_xml::events::BytesStart, current: Option<&mut Draft>, show: &mut Show, channel_image_from_tag: &mut bool) {
    if name == "enclosure" {
        if let Some(d) = current {
            d.take_enclosure(e);
        }
    } else if name.ends_with(":image") {
        if let Some(href) = attribute(e, "href") {
            match current {
                Some(d) => d.image = Some(href),
                None => {
                    show.image = Some(href);
                    *channel_image_from_tag = true;
                }
            }
        }
    } else if name == "link"
        && attribute(e, "rel").as_deref() == Some("enclosure")
        && let Some(href) = attribute(e, "href")
        && let Some(d) = current
        && d.url.is_empty()
    {
        d.url = href;
        d.enclosure_type = attribute(e, "type").unwrap_or_default();
    }
}

/// An attribute's text; one whose entities are broken is taken as written.
fn attribute(e: &quick_xml::events::BytesStart, name: &str) -> Option<String> {
    e.try_get_attribute(name).ok().flatten().map(|a| unescape_soft(a.value.as_ref()).trim().to_string()).filter(|v| !v.is_empty())
}

/// Text with its entities resolved. One that the strict rules refuse, such as `&nbsp;` or a
/// bare `&`, has the usual entities replaced and the rest left as written.
pub fn unescape_soft(raw: &str) -> String {
    if let Ok(text) = quick_xml::escape::unescape(raw) {
        return text.into_owned();
    }
    let mut out = String::with_capacity(raw.len());
    let mut rest = raw;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        let end = rest.find(';').filter(|&j| j <= 10);
        let replaced = end.and_then(|j| {
            let name = &rest[1..j];
            let text = match name {
                "amp" => Some("&".to_string()),
                "lt" => Some("<".to_string()),
                "gt" => Some(">".to_string()),
                "quot" => Some("\"".to_string()),
                "apos" => Some("'".to_string()),
                "nbsp" => Some(" ".to_string()),
                n if n.starts_with("#x") => u32::from_str_radix(&n[2..], 16).ok().and_then(char::from_u32).map(String::from),
                n if n.starts_with('#') => n[1..].parse::<u32>().ok().and_then(char::from_u32).map(String::from),
                _ => None,
            }?;
            Some((text, j + 1))
        });
        match replaced {
            Some((text, used)) => {
                out.push_str(&text);
                rest = &rest[used..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// An episode as it is read, before it is known to be complete.
#[derive(Default)]
struct Draft {
    guid: String,
    title: String,
    url: String,
    enclosure_type: String,
    published: String,
    duration: String,
    image: Option<String>,
    season: Option<u32>,
    number: Option<u32>,
    trailer: bool,
}

impl Draft {
    fn take_enclosure(&mut self, e: &quick_xml::events::BytesStart) {
        let url = attribute(e, "url").unwrap_or_default();
        let kind = attribute(e, "type").unwrap_or_default();
        let playable = |k: &str| k.starts_with("audio") || k.starts_with("video");
        // The first audio or video file is the episode; a transcript or picture is not.
        if !url.is_empty() && (self.url.is_empty() || (!playable(&self.enclosure_type) && playable(&kind))) {
            self.url = url;
            self.enclosure_type = kind;
        }
    }

    fn take_text(&mut self, tag: &str, parent: &str, value: &str) {
        let own = parent == "item" || parent == "entry";
        match tag {
            "title" if own && self.title.is_empty() => self.title = value.to_string(),
            "guid" | "id" if own => self.guid = value.to_string(),
            "pubDate" | "published" | "updated" if own && self.published.is_empty() => self.published = value.to_string(),
            "itunes:duration" if own => self.duration = value.to_string(),
            "itunes:season" if own => self.season = value.parse().ok(),
            "itunes:episode" if own => self.number = value.parse().ok(),
            "itunes:episodeType" if own => self.trailer = value.eq_ignore_ascii_case("trailer"),
            "itunes:image" if own && self.image.is_none() && value.starts_with("http") => self.image = Some(value.to_string()),
            _ => {}
        }
    }

    fn finish(self, feed_url: &str) -> Option<Episode> {
        if self.url.is_empty() || self.title.is_empty() {
            return None;
        }
        Some(Episode {
            id: episode_id(feed_url, &self.guid, &self.url),
            title: self.title,
            url: self.url,
            published_ms: parse_date(&self.published).unwrap_or(0),
            duration_ms: parse_duration(&self.duration),
            image: self.image,
            season: self.season,
            number: self.number,
            trailer: self.trailer,
        })
    }
}

/// A stable id for an episode: its guid, or its file's address. A short guid such as "1" is
/// one show's numbering, so the feed is mixed in to keep shows apart.
pub fn episode_id(feed_url: &str, guid: &str, enclosure_url: &str) -> String {
    let key = if guid.is_empty() { enclosure_url } else { guid };
    if key.contains("://") || key.len() >= 20 { fnv(key) } else { fnv(&format!("{feed_url}\n{key}")) }
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
        "nbsp" => " ",
        _ => "",
    }
    .to_string()
}

/// A date as feeds give it: RSS's own form, or ISO 8601 with or without seconds or a time.
pub fn parse_date(s: &str) -> Option<i64> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    if let Some(ms) = parse_rfc2822(s) {
        return Some(ms);
    }
    if let Some(ms) = sdk::parse_iso8601(s) {
        return Some(ms);
    }
    // "2026-11-09T10:00Z" lacks seconds; "2026-11-09" lacks a time.
    if s.len() >= 16 && s.is_char_boundary(16) && s.as_bytes()[10] == b'T' && s.as_bytes().get(16).is_none_or(|c| !c.is_ascii_digit() && *c != b':') {
        return sdk::parse_iso8601(&format!("{}:00{}", &s[..16], &s[16..]));
    }
    if s.len() == 10 {
        return sdk::parse_iso8601(&format!("{s}T00:00:00Z"));
    }
    None
}

/// A date as RSS gives it: `Mon, 09 Nov 2026 10:00:00 +0000`, with or without the weekday,
/// the month in full or short, and a zone as an offset or a name.
pub fn parse_rfc2822(s: &str) -> Option<i64> {
    let mut parts = s.split([' ', ',']).filter(|p| !p.is_empty()).peekable();
    if parts.peek().is_some_and(|p| p.chars().all(|c| c.is_ascii_alphabetic())) {
        parts.next();
    }
    let day: u32 = parts.next()?.parse().ok()?;
    let month_name = parts.next()?.to_ascii_lowercase();
    let month = ["jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec"].iter().position(|m| month_name.starts_with(m))? as u32 + 1;
    let year: i64 = parts.next()?.parse().ok()?;
    let year = if year < 100 { year + 2000 } else { year };
    let mut clock = parts.next().unwrap_or("00:00:00").split(':').map(|p| p.parse::<u32>().unwrap_or(0));
    let (hour, minute, second) = (clock.next().unwrap_or(0), clock.next().unwrap_or(0), clock.next().unwrap_or(0));
    let zone = parts.next().unwrap_or("+0000");
    let offset_minutes = zone_offset(zone)?;
    if !(1..=31).contains(&day) || hour > 23 || minute > 59 || second > 60 {
        return None;
    }
    Some(sdk::epoch_ms(year, month, day, hour, minute, second.min(59), offset_minutes))
}

/// A zone as feeds write it: `+0000`, `+05:30`, `GMT+1000`, or a name.
fn zone_offset(zone: &str) -> Option<i32> {
    let zone = zone.trim().trim_start_matches("GMT").trim_start_matches("UTC");
    let named = match zone {
        "" | "UT" | "Z" => Some(0),
        "BST" | "IST" | "CET" | "WEST" | "WAT" => Some(60),
        "CEST" | "EET" | "SAST" | "CAT" => Some(120),
        "EEST" | "MSK" | "EAT" => Some(180),
        "AEST" => Some(600),
        "AEDT" => Some(660),
        "NZST" => Some(720),
        "NZDT" => Some(780),
        "JST" | "KST" => Some(540),
        "HKT" | "SGT" | "AWST" => Some(480),
        "AST" => Some(-240),
        "EST" => Some(-300),
        "EDT" => Some(-240),
        "CST" => Some(-360),
        "CDT" => Some(-300),
        "MST" => Some(-420),
        "MDT" => Some(-360),
        "PST" => Some(-480),
        "PDT" => Some(-420),
        "AKST" => Some(-540),
        "AKDT" => Some(-480),
        "HST" => Some(-600),
        _ => None,
    };
    if let Some(minutes) = named {
        return Some(minutes);
    }
    let (sign, rest) = match zone.as_bytes().first()? {
        b'+' => (1, &zone[1..]),
        b'-' => (-1, &zone[1..]),
        _ => return None,
    };
    let digits: String = rest.chars().filter(|c| c.is_ascii_digit()).collect();
    let n: i32 = match digits.len() {
        4 => digits.parse().ok()?,
        2 => digits.parse::<i32>().ok()? * 100,
        _ => return None,
    };
    Some(sign * (n / 100 * 60 + n % 100))
}

/// A duration as feeds give it: seconds, `MM:SS` or `HH:MM:SS`.
pub fn parse_duration(s: &str) -> Option<i64> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let mut total = 0i64;
    for part in s.split(':') {
        total = total * 60 + part.trim().parse::<f64>().ok()? as i64;
    }
    (total > 0).then_some(total * 1000)
}

/// A short, stable id for a feed or episode: FNV-1a over its text, in hex.
pub fn fnv(s: &str) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.bytes() {
        hash ^= b as u64;
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{hash:016x}")
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

    const FEED: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<rss version="2.0" xmlns:itunes="http://www.itunes.com/dtds/podcast-1.0.dtd" xmlns:atom="http://www.w3.org/2005/Atom">
 <channel>
  <title>Radiolab</title>
  <image><url>https://img.example/radiolab-small.jpg</url><title>Radiolab</title></image>
  <itunes:image href="https://img.example/radiolab.jpg"/>
  <description>Q&A about <br/> everything</description>
  <item>
   <title><![CDATA[The Other Latif &amp; Co]]></title>
   <guid isPermaLink="false">ep-2</guid>
   <pubDate>Fri, 06 November 2026 10:00:00 -0500</pubDate>
   <enclosure url="https://cdn.example/ep2.mp3" type="audio/mpeg" length="1234"></enclosure>
   <itunes:duration>2:25:12</itunes:duration>
   <itunes:image href="https://img.example/ep2.jpg"/>
   <itunes:season>3</itunes:season><itunes:episode>7</itunes:episode>
  </item>
  <item>
   <title>Older one</title>
   <guid>ep-1</guid>
   <pubDate>Mon, 02 Nov 2026 09:00:00 GMT</pubDate>
   <enclosure url="https://cdn.example/ep1-transcript.txt" type="text/plain"/>
   <enclosure url="https://cdn.example/ep1.mp3" type="audio/mpeg"/>
   <itunes:duration>3600</itunes:duration>
  </item>
  <item>
   <title>Bonus &#8217; newest</title>
   <guid>https://cdn.example/guid/ep-3</guid>
   <pubDate>Sun, 08 Nov 2026 20:30:00 +0000</pubDate>
   <enclosure url="https://cdn.example/ep3.m4a?a=1&amp;b=2&c=3" type="audio/x-m4a"/>
  </item>
  <item>
   <title>Coming soon</title>
   <guid>trailer-1</guid>
   <pubDate>Mon, 09 Nov 2026 08:00:00 +0000</pubDate>
   <itunes:episodeType>trailer</itunes:episodeType>
   <enclosure url="https://cdn.example/trailer.mp3" type="audio/mpeg"/>
  </item>
  <item>
   <title>No file</title>
   <guid>ep-4</guid>
   <pubDate>Sun, 08 Nov 2026 21:30:00 +0000</pubDate>
  </item>
  <item>
   <title>Older one</title>
   <guid>ep-1</guid>
   <enclosure url="https://cdn.example/ep1.mp3" type="audio/mpeg"/>
  </item>
 </channel>
</rss>"#;

    #[test]
    fn a_feed_becomes_a_show_with_its_episodes_newest_first() {
        let show = parse_feed(FEED, "https://feeds.example/radiolab");
        assert_eq!(show.title, "Radiolab");
        assert_eq!(show.image.as_deref(), Some("https://img.example/radiolab.jpg"), "the iTunes picture wins over the RSS one");
        let titles: Vec<&str> = show.episodes.iter().map(|e| e.title.as_str()).collect();
        assert_eq!(titles, ["Coming soon", "Bonus \u{2019} newest", "The Other Latif & Co", "Older one"]);
        assert_eq!(show.latest().map(|e| e.title.as_str()), Some("Bonus \u{2019} newest"), "a trailer is not the latest episode");
        let latif = &show.episodes[2];
        assert_eq!(latif.url, "https://cdn.example/ep2.mp3", "an enclosure that is not self-closing still counts");
        assert_eq!(latif.published_ms, sdk::parse_iso8601("2026-11-06T15:00:00Z").unwrap(), "a month in full is read");
        assert_eq!(latif.duration_ms, Some(8_712_000));
        assert_eq!(latif.image.as_deref(), Some("https://img.example/ep2.jpg"));
        assert_eq!((latif.season, latif.number), (Some(3), Some(7)));
        assert_eq!(latif.id, episode_id("https://feeds.example/radiolab", "ep-2", ""));
        assert_ne!(latif.id, episode_id("https://feeds.example/other", "ep-2", ""), "short guids are one show's own");
        assert_eq!(show.episodes[1].id, fnv("https://cdn.example/guid/ep-3"), "a guid that is an address stands alone");
        assert_eq!(show.episodes[1].url, "https://cdn.example/ep3.m4a?a=1&b=2&c=3", "a bare ampersand in an address is kept");
        let older = &show.episodes[3];
        assert_eq!(older.url, "https://cdn.example/ep1.mp3", "the transcript is not the episode");
        assert_eq!(older.duration_ms, Some(3_600_000));
        assert_eq!(show.episodes.iter().filter(|e| e.title == "Older one").count(), 1);
    }

    #[test]
    fn atom_feeds_and_text_images_are_read_too() {
        let atom = r#"<feed xmlns="http://www.w3.org/2005/Atom" xmlns:itunes="http://www.itunes.com/dtds/podcast-1.0.dtd"><title>Atomic</title><itunes:image>https://img.example/atomic.jpg</itunes:image>
          <entry><title>One</title><id>urn:uuid:12345678-1234-1234-1234-123456789abc</id><updated>2026-11-09T10:00Z</updated><link rel="enclosure" href="https://cdn.example/one.mp3" type="audio/mpeg"></link></entry></feed>"#;
        let show = parse_feed(atom, "https://feeds.example/atomic");
        assert_eq!(show.title, "Atomic");
        assert_eq!(show.image.as_deref(), Some("https://img.example/atomic.jpg"));
        assert_eq!(show.episodes.len(), 1);
        assert_eq!(show.episodes[0].url, "https://cdn.example/one.mp3");
        assert_eq!(show.episodes[0].published_ms, sdk::parse_iso8601("2026-11-09T10:00:00Z").unwrap());
        assert_eq!(parse_feed("<html>not a feed</html>", "https://x").episodes.len(), 0);
    }

    #[test]
    fn episodes_and_shows_become_tiles() {
        let show = parse_feed(FEED, "https://feeds.example/radiolab");
        let item = episode_item(&show, &show.episodes[2], 3, true);
        assert_eq!(item.title, "The Other Latif & Co");
        assert_eq!(item.subtitle.as_deref(), Some("Radiolab, 2 h 25 min"));
        assert_eq!((item.season.as_deref(), item.episode.as_deref()), (Some("3"), Some("7")));
        assert_eq!(item.thumbnail.as_deref(), Some("https://img.example/ep2.jpg"));
        assert_eq!(item.action.play.as_deref(), Some("https://cdn.example/ep2.mp3"));
        assert_eq!(item.action.title.as_deref(), Some("The Other Latif & Co"));
        assert!(item.is_new);
        assert_eq!(item.row.as_deref(), Some("new"));
        let plain = episode_item(&show, &show.episodes[1], 1, false);
        assert_eq!(plain.subtitle.as_deref(), Some("Radiolab"));
        assert_eq!(plain.thumbnail.as_deref(), Some("https://img.example/radiolab.jpg"));
        assert_eq!((plain.season, plain.episode), (None, None));
        let tile = show_item(&show, 1).unwrap();
        assert_eq!((tile.title.as_str(), tile.subtitle.as_deref(), tile.row.as_deref()), ("Radiolab", Some("Latest: Bonus \u{2019} newest"), Some("shows")));
        assert_eq!(tile.action.play.as_deref(), Some("https://cdn.example/ep3.m4a?a=1&b=2&c=3"));
        assert_eq!(length_text(48 * 60_000), "48 min");
        assert_eq!(length_text(2 * 3_600_000), "2 h");
        assert_eq!(length_text(20_000), "1 min");
    }

    #[test]
    fn dates_and_durations_come_in_every_shape() {
        let expect = sdk::parse_iso8601("2026-11-09T10:00:00Z").unwrap();
        assert_eq!(parse_date("Mon, 09 Nov 2026 10:00:00 +0000"), Some(expect));
        assert_eq!(parse_date("09 Nov 2026 10:00:00 GMT"), Some(expect));
        assert_eq!(parse_date("Mon, 9 Nov 2026 05:00:00 EST"), Some(expect));
        assert_eq!(parse_date("Mon, 09 Nov 2026 20:00:00 +1000"), Some(expect));
        assert_eq!(parse_date("Mon, 09 Nov 2026 20:00:00 GMT+1000"), Some(expect));
        assert_eq!(parse_date("Mon, 09 Nov 2026 15:30:00 +05:30"), Some(expect));
        assert_eq!(parse_date("Mon, 09 Nov 2026 21:00:00 AEDT"), Some(expect));
        assert_eq!(parse_date("Mon, 09 Nov 2026 11:00:00 CET"), Some(expect));
        assert_eq!(parse_date("Wed, 04 Sept 2026 10:00:00 +0000"), sdk::parse_iso8601("2026-09-04T10:00:00Z"));
        assert_eq!(parse_date("Mon, 09 Nov 26 10:00 +0000"), Some(expect));
        assert_eq!(parse_date("2026-11-09T10:00:00Z"), Some(expect));
        assert_eq!(parse_date("2026-11-09T10:00Z"), Some(expect));
        assert_eq!(parse_date("2026-11-09T10:00+00:00"), Some(expect));
        assert_eq!(parse_date("2026-11-09"), sdk::parse_iso8601("2026-11-09T00:00:00Z"));
        assert_eq!(parse_date("Mon, 32 Nov 2026 10:00:00 +0000"), None);
        assert_eq!(parse_date("Mon, 09 Nov 2026 10:00:00 XYZ"), None);
        assert_eq!(parse_date(""), None);
        // Text that is no date is refused wherever its letters fall.
        assert_eq!(parse_date("aaaaaaaaaaTaaaa\u{e9}"), None);
        assert_eq!(parse_duration("48:12"), Some(2_892_000));
        assert_eq!(parse_duration("1:02:03"), Some(3_723_000));
        assert_eq!(parse_duration("3723.5"), Some(3_723_000));
        assert_eq!(parse_duration("0"), None);
        assert_eq!(parse_duration("soon"), None);
        assert_eq!(fnv("ep-2").len(), 16);
    }

    #[test]
    fn broken_entities_are_read_as_written() {
        assert_eq!(unescape_soft("Tom&nbsp;and Jerry &amp; friends &#8217; &#x2019; &unknown; Q&A"), "Tom and Jerry & friends \u{2019} \u{2019} &unknown; Q&A");
        assert_eq!(unescape_soft("plain &amp; simple"), "plain & simple");
        assert_eq!(unescape_soft("https://x/?a=1&b=2"), "https://x/?a=1&b=2");
    }

    #[test]
    fn sources_are_feeds_apple_links_or_opml() {
        let s = parse_sources("https://feeds.example/a\nhttps://podcasts.apple.com/au/podcast/radiolab/id152249110?uo=4, feed://feeds.example/b\nfeeds.example/c https://feeds.example/a HTTPS://Feeds.Example/D pcast://feeds.example/e https://overcast.fm/itunes394775318/99-invisible https://pod.link/152249110 https://files.example/subs.opml").unwrap();
        assert_eq!(
            s,
            vec![
                Source::Feed("https://feeds.example/a".into()),
                Source::Apple("152249110".into()),
                Source::Feed("https://feeds.example/b".into()),
                Source::Feed("https://feeds.example/c".into()),
                Source::Feed("https://Feeds.Example/D".into()),
                Source::Feed("https://feeds.example/e".into()),
                Source::Apple("394775318".into()),
                Source::Opml("https://files.example/subs.opml".into()),
            ]
        );
        assert!(parse_sources("https://podcasts.apple.com/us/browse").unwrap_err().contains("without a podcast id"));
        assert!(parse_sources("radiolab").unwrap_err().contains("not a feed link"));
        // Only Apple's own names are Apple links, and a comma inside an address stays in it.
        assert_eq!(parse_sources("https://pineapple.com/feed.xml").unwrap(), vec![Source::Feed("https://pineapple.com/feed.xml".into())]);
        assert!(parse_sources("https://music.apple.com/us/album/x").unwrap_err().contains("without a podcast id"));
        assert_eq!(parse_sources("https://feeds.example/show?tags=a,b;c").unwrap(), vec![Source::Feed("https://feeds.example/show?tags=a,b;c".into())]);
        assert_eq!(parse_sources("feeds.example/a,feeds.example/b;").unwrap(), vec![Source::Feed("https://feeds.example/a".into()), Source::Feed("https://feeds.example/b".into())]);
        assert!(parse_sources("ftp://x.example/feed").unwrap_err().contains("not a link Lounge can fetch"));
        assert_eq!(apple_id("https://podcasts.apple.com/gb/podcast/id152249110"), Some("152249110".into()));
        assert_eq!(apple_id("https://itunes.apple.com/lookup?id=152249110"), Some("152249110".into()));
        assert_eq!(apple_storefront("https://podcasts.apple.com/au/podcast/x/id1"), Some("au".into()));
        assert_eq!(apple_storefront("https://podcasts.apple.com/podcast/x/id1"), None);
        let opml = r#"<?xml version="1.0"?><opml version="2.0"><body><outline text="Tech"><outline type="rss" text="Syntax" xmlUrl="https://feed.syntax.fm/rss?a=1&amp;b=2" htmlUrl="https://syntax.fm"/><outline type="rss" text="Dup" xmlUrl='https://feed.syntax.fm/rss?a=1&amp;b=2'/></outline></body></opml>"#;
        assert_eq!(parse_sources(opml).unwrap(), vec![Source::Feed("https://feed.syntax.fm/rss?a=1&b=2".into())]);
        let d: Directory = serde_json::from_str(r#"{"resultCount":1,"results":[{"wrapperType":"track","collectionId":152249110,"collectionName":"Radiolab","artistName":"WNYC Studios","feedUrl":"https://feeds.simplecast.com/EmVW7VGp","artworkUrl600":"https://is1-ssl.mzstatic.com/x/600x600bb.jpg"}]}"#).unwrap();
        assert_eq!(d.results[0].feed_url, "https://feeds.simplecast.com/EmVW7VGp");
    }

    #[test]
    fn feeds_are_asked_for_longest_ago_first_and_kept_when_they_fail() {
        let mut cache = FeedCache::default();
        let kept = parse_feed(FEED, "https://feeds.example/kept");
        cache.shows.insert("https://feeds.example/kept".into(), Show { fetched_ms: 500, attempted_ms: 500, ..kept });
        cache.shows.insert("https://feeds.example/gone".into(), Show { fetched_ms: 900, ..Default::default() });
        cache.shows.insert("https://feeds.example/huge".into(), Show { oversized_ms: 950, attempted_ms: 950, ..Default::default() });
        let feeds: Vec<FeedRef> = ["https://feeds.example/kept", "https://feeds.example/new", "https://feeds.example/huge"].iter().map(|u| FeedRef { url: u.to_string(), apple_id: None, art: None }).collect();
        let mut budget = Budget { fetches: FEEDS_PER_REFRESH - 1, ..Default::default() };
        let mut failures = Vec::new();
        // One fetch left: the never-asked feed goes first and, off the TV, fails; the kept one stays.
        let gone = cache.refresh(&feeds, 1_000, &mut budget, &mut failures);
        assert!(gone.is_empty());
        assert_eq!(budget.fetches, FEEDS_PER_REFRESH);
        assert_eq!(failures.len(), 1);
        assert!(failures[0].starts_with("feeds.example/new didn't answer."), "{}", failures[0]);
        assert!(!cache.shows.contains_key("https://feeds.example/gone"));
        assert_eq!(cache.shows["https://feeds.example/kept"].episodes.len(), 4);
        assert_eq!(cache.shows["https://feeds.example/kept"].fetched_ms, 500);
        assert_eq!(cache.shows["https://feeds.example/new"].attempted_ms, 1_000, "a failed feed is not first in line again");
        // A feed found too big yesterday is left alone today.
        let mut budget = Budget::default();
        cache.refresh(&feeds[2..], 1_000 + DAY_MS / 2, &mut budget, &mut failures);
        assert_eq!(budget.fetches, 0);
    }

    #[test]
    fn errors_say_what_to_check() {
        assert_eq!(<Podcasts as sdk::Plugin>::resolve("latest:https://feeds.example/x").err().as_deref(), Some("feeds.example/x didn't answer. Check the link in the plugin's settings and the TV's connection.".replace(" Check the link in the plugin's settings.", "").as_str()));
        assert_eq!(<Podcasts as sdk::Plugin>::resolve("other").err().as_deref(), Some(OUT_OF_DATE));
        assert_eq!(<Podcasts as sdk::Plugin>::search("  ", 12).unwrap().len(), 0);
        assert!(<Podcasts as sdk::Plugin>::manifest().settings.iter().all(|s| !s.required));
        assert_eq!(zone_offset("PDT"), Some(-420));
        assert_eq!(zone_offset("+0530"), Some(330));
        assert_eq!(zone_offset("-03"), Some(-180));
        assert_eq!(zone_offset("nope"), None);
    }
}
