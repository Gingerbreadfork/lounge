//! YouTube: the public feed of each channel or playlist you list becomes a "New on YouTube"
//! row, newest first, opened in the YouTube app. With a Google sign-in, made with a code on
//! the phone, the row takes in your subscriptions as well, and your playlists and liked videos
//! get rows of their own. The feeds are the ones YouTube publishes for every channel, so the
//! sign-in only ever asks the API which channels you follow.
use lounge_plugin_sdk::{self as sdk, host, Action, HttpRequest, Item, Kind, Manifest, RefreshContext, RowSpec, RowStyle, SettingKind, SettingSpec};
use quick_xml::events::Event;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub struct YouTube;

const APP_PACKAGE: &str = "com.google.android.youtube.tv";
const API: &str = "https://www.googleapis.com/youtube/v3";
const DEVICE_CODE_URL: &str = "https://oauth2.googleapis.com/device/code";
const TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
const SCOPE: &str = "https://www.googleapis.com/auth/youtube.readonly";
const TOKEN_KEY: &str = "token";
const DEVICE_KEY: &str = "device";
const SUBS_KEY: &str = "subscriptions";
const ART_KEY: &str = "hd_art";
const HANDLES_KEY: &str = "handles";
/// Feeds from the channels setting; subscriptions take what is left.
const MAX_FEEDS: usize = 40;
/// Fetches spent in one refresh, inside the host's 64.
const FETCH_BUDGET: usize = 60;
/// Fetches kept back from feeds so a few pictures can still be checked.
const ART_RESERVE: usize = 8;
const ART_CHECKS: usize = 20;
/// Pages of fifty subscriptions read, so a very long list does not use the refresh up.
const MAX_SUB_PAGES: usize = 6;
/// Videos kept per subscribed channel between refreshes.
const STORED_PER_CHANNEL: usize = 5;
const MOST_ITEMS: usize = 200;
/// The key-value store takes 256 KB a key; the cache stays well under it.
const CACHE_BYTES: usize = 200 * 1024;
const MINUTE_MS: i64 = 60_000;
const SIGNED_OUT: &str = "Google no longer accepts this sign-in. Press Refresh now to sign in again.";
const QUOTA: &str = "YouTube's daily allowance for this app is used up. It comes back at midnight, Pacific time.";
const API_OFF: &str = "The YouTube Data API v3 is not enabled for this Google Cloud project. Enable it, then try again.";

impl sdk::Plugin for YouTube {
    fn manifest() -> Manifest {
        Manifest {
            id: "tv.lounge.youtube".into(),
            name: "YouTube".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            description: "New videos from the channels you follow, and with a Google sign-in your subscriptions, playlists and liked videos.".into(),
            publisher_name: "Lounge".into(),
            kinds: vec![Kind::Content],
            fetch_hosts: vec!["www.youtube.com".into(), "m.youtube.com".into(), "*.ytimg.com".into(), "oauth2.googleapis.com".into(), "www.googleapis.com".into()],
            refresh_minutes: 30,
            rows: vec![
                RowSpec { id: "new".into(), title: "New on YouTube".into(), style: RowStyle::Landscape },
                RowSpec { id: "playlists".into(), title: "Your YouTube playlists".into(), style: RowStyle::Landscape },
                RowSpec { id: "liked".into(), title: "Liked on YouTube".into(), style: RowStyle::Landscape },
            ],
            settings: vec![
                SettingSpec {
                    key: "channels".into(),
                    label: "Channels".into(),
                    kind: SettingKind::List,
                    required: false,
                    help: Some("Channel links, @handles or ids (UC…), playlist links or ids (PL…), one per line or separated by commas. A Google Takeout export of your subscriptions, the CSV file, can be pasted in whole. Not needed with a sign-in below.".into()),
                    default: None,
                },
                SettingSpec {
                    key: "client_id".into(),
                    label: "Google Client ID".into(),
                    kind: SettingKind::Text,
                    required: false,
                    help: Some("For your subscriptions, playlists and likes. At console.cloud.google.com make a project, enable the YouTube Data API v3, set the OAuth consent screen to External and publish it, then make an OAuth client of the type TVs and Limited Input devices and copy its Client ID and Client secret here. This page then shows a code to enter on your phone.".into()),
                    default: None,
                },
                SettingSpec { key: "client_secret".into(), label: "Google Client secret".into(), kind: SettingKind::Password, required: false, help: None, default: None },
                SettingSpec { key: "subscriptions".into(), label: "Subscriptions".into(), kind: SettingKind::Toggle, required: false, help: Some("New videos from the channels you subscribe to, in New on YouTube.".into()), default: Some("true".into()) },
                SettingSpec { key: "playlists".into(), label: "Playlists row".into(), kind: SettingKind::Toggle, required: false, help: Some("Your own playlists. Watch later is not one YouTube hands out.".into()), default: Some("true".into()) },
                SettingSpec { key: "liked".into(), label: "Liked videos row".into(), kind: SettingKind::Toggle, required: false, help: None, default: Some("false".into()) },
                SettingSpec { key: "per_source".into(), label: "Videos per channel".into(), kind: SettingKind::Number, required: false, help: None, default: Some("4".into()) },
                SettingSpec { key: "days".into(), label: "Only the last … days".into(), kind: SettingKind::Number, required: false, help: Some("0 shows the feed's newest videos whatever their age.".into()), default: Some("30".into()) },
                SettingSpec { key: "shorts".into(), label: "Include Shorts".into(), kind: SettingKind::Toggle, required: false, help: None, default: Some("false".into()) },
            ],
            homepage: None,
            self_signed_servers: false,
            for_apps: Vec::new(),
            sent_to: Default::default(),
        }
    }

    fn refresh(ctx: &RefreshContext) -> Result<Vec<Item>, String> {
        let mut budget = Budget::new(FETCH_BUDGET);
        let sources = match host::setting("channels") {
            Some(raw) => parse_sources(&raw)?,
            None => Vec::new(),
        };
        let account = Account::connect(ctx.now_ms, &mut budget)?;
        if sources.is_empty() && account.is_none() {
            return Err("Nothing to show yet: add a channel link or id, or a Google Client ID, in the plugin's settings".into());
        }
        let per_source = number_setting("per_source", 4).clamp(1, 15) as usize;
        let days = number_setting("days", 30).clamp(0, 3_650);
        let since = if days == 0 { 0 } else { ctx.now_ms - days * 86_400_000 };
        let shorts = host::setting("shorts").is_some_and(|v| v == "true");
        let wanted = |v: &Video| v.published_ms >= since && (shorts || !v.short);
        let mut videos = Vec::new();
        let mut failures = Vec::new();
        let sources = resolve_named(sources, &mut budget, &mut failures);
        if sources.len() > MAX_FEEDS {
            sdk::warn!("{} channels and playlists are listed; only the first {MAX_FEEDS} are read", sources.len());
        }
        for source in sources.iter().take(MAX_FEEDS) {
            budget.spend();
            match feed(source) {
                Ok(xml) => {
                    let mut parsed = parse_feed(&xml);
                    parsed.retain(wanted);
                    parsed.truncate(per_source);
                    videos.extend(parsed);
                }
                Err(e) => failures.push((source.label(), e)),
            }
        }

        let mut items = Vec::new();
        let mut subscribed = 0;
        if let Some(client) = &account {
            if toggle("subscriptions", true) {
                let mut cache = SubsCache::load();
                match client.subscriptions(&mut budget) {
                    Ok(subs) => {
                        subscribed = subs.len();
                        cache.refresh(&subs, ctx.now_ms, &mut budget);
                        cache.save();
                    }
                    // The channels read last time stand in until the list can be read again.
                    Err(e) => {
                        subscribed = cache.channels.len();
                        failures.push(("subscriptions".into(), e));
                    }
                }
                for channel in cache.channels.values() {
                    let mut from_channel: Vec<Video> = channel.videos.iter().filter(|v| wanted(v)).take(per_source).cloned().collect();
                    videos.append(&mut from_channel);
                }
            }
            if toggle("playlists", true) {
                match client.playlists(&mut budget) {
                    Ok(playlists) => items.extend(playlists),
                    Err(e) => failures.push(("playlists".into(), e)),
                }
            }
            if toggle("liked", false) {
                match client.liked(&mut budget, ctx.now_ms) {
                    Ok(liked) => items.extend(liked),
                    Err(e) => failures.push(("liked videos".into(), e)),
                }
            }
        }
        for (label, e) in &failures {
            sdk::warn!("{label}: {e}");
        }
        if videos.is_empty()
            && items.is_empty()
            && let Some((_, e)) = failures.into_iter().next()
        {
            return Err(e);
        }
        videos.sort_by_key(|v| std::cmp::Reverse(v.published_ms));
        let mut seen = std::collections::HashSet::new();
        videos.retain(|v| seen.insert(v.id.clone()));
        videos.truncate(MOST_ITEMS);
        let recent = ctx.now_ms - 3 * 86_400_000;
        let count = videos.len();
        let hd = hd_art(&videos, budget.left().min(ART_CHECKS));
        sdk::info!("{count} videos from {} feeds and {subscribed} subscriptions, {} with large pictures, {} other tiles", sources.len().min(MAX_FEEDS), hd.values().filter(|v| **v).count(), items.len());
        items.extend(videos.into_iter().enumerate().map(|(i, v)| Item {
            id: format!("video:{}", v.id),
            title: v.title,
            subtitle: Some(v.channel),
            added_ms: Some(v.published_ms),
            is_new: v.published_ms >= recent,
            thumbnail: Some(thumbnail(&v.id, hd.get(&v.id) == Some(&true))),
            row: Some("new".into()),
            weight: (count - i) as i32,
            action: watch(&v.id),
            ..Default::default()
        }));
        sdk::dedup_ids(&mut items);
        Ok(items)
    }
}

sdk::export_plugin!(YouTube);

fn toggle(key: &str, default: bool) -> bool {
    host::setting(key).map(|v| v == "true").unwrap_or(default)
}

fn watch(video_id: &str) -> Action {
    Action { view: Some(format!("https://www.youtube.com/watch?v={video_id}")), fallback_launch: Some(APP_PACKAGE.into()), ..Default::default() }
}

/// The fetches one refresh may still make.
struct Budget {
    left: usize,
}

impl Budget {
    fn new(left: usize) -> Budget {
        Budget { left }
    }

    fn left(&self) -> usize {
        self.left
    }

    /// Takes one fetch; false once there are none, which a caller treats as "next time".
    fn spend(&mut self) -> bool {
        if self.left == 0 {
            return false;
        }
        self.left -= 1;
        true
    }
}

/// A source's feed, or what went wrong reading it as a sentence for the TV. YouTube answers
/// 404 for an id it does not know, and now and then for one it does, so the sentence allows
/// for both.
fn feed(source: &Source) -> Result<String, String> {
    let r = sdk::fetch_online("YouTube", &HttpRequest::get(source.feed_url()))?;
    match r.status {
        404 => Err(format!("YouTube gave no feed for {}. If the id is right, it comes with a later refresh.", source.label())),
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
fn hd_art(videos: &[Video], most_checks: usize) -> BTreeMap<String, bool> {
    let mut known: BTreeMap<String, bool> = host::kv_get_string(ART_KEY).and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
    known.retain(|id, _| videos.iter().any(|v| &v.id == id));
    let mut checks = 0;
    for v in videos {
        if known.contains_key(&v.id) || checks >= most_checks {
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

// ---------------------------------------------------------------------------
// Signing in to Google: a code on the phone, one step per refresh
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize)]
struct Token {
    access: String,
    refresh: String,
    expires_ms: i64,
}

#[derive(Serialize, Deserialize)]
struct Device {
    device_code: String,
    user_code: String,
    verification_url: String,
    expires_ms: i64,
}

#[derive(Deserialize)]
struct DeviceAnswer {
    device_code: String,
    user_code: String,
    verification_url: String,
    #[serde(default)]
    expires_in: i64,
}

#[derive(Deserialize)]
struct TokenAnswer {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    expires_in: i64,
}

#[derive(Deserialize, Default)]
struct OAuthError {
    #[serde(default)]
    error: String,
    #[serde(default)]
    error_description: String,
}

/// A signed-in client for the account's own lists.
struct Client {
    token: String,
}

/// The sign-in the settings ask for: none without a Client ID, else a client once the code
/// has been entered, with the sentence to show meanwhile as the error.
struct Account;

impl Account {
    fn connect(now_ms: i64, budget: &mut Budget) -> Result<Option<Client>, String> {
        let Some(client_id) = host::setting("client_id").map(|v| v.trim().to_string()).filter(|v| !v.is_empty()) else {
            return Ok(None);
        };
        let secret = host::secret("client_secret").map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).ok_or("Google Client secret is not set")?;
        let saved: Option<Token> = host::kv_get(TOKEN_KEY).and_then(|v| serde_json::from_slice(&v).ok());
        let token = match saved {
            Some(t) if t.expires_ms - now_ms > 5 * MINUTE_MS => t,
            Some(t) => {
                budget.spend();
                match renew(&client_id, &secret, &t, now_ms) {
                    Ok(t) => t,
                    Err(Refusal::Refused(why)) => {
                        sdk::warn!("Google refused to renew the sign-in ({why}); signing in again");
                        host::kv_delete(TOKEN_KEY);
                        budget.spend();
                        device_step(&client_id, &secret, now_ms)?
                    }
                    Err(Refusal::Failed(e)) if t.expires_ms > now_ms => {
                        sdk::warn!("could not renew the sign-in ({e}); using the current one");
                        t
                    }
                    Err(Refusal::Failed(e)) => {
                        sdk::warn!("could not renew the sign-in ({e})");
                        return Err("Could not renew the Google sign-in. It tries again on the next refresh.".into());
                    }
                }
            }
            None => {
                budget.spend();
                device_step(&client_id, &secret, now_ms)?
            }
        };
        Ok(Some(Client { token: token.access }))
    }
}

/// Why a renewal did not give a token: Google refused the refresh token, or nothing usable
/// came back.
enum Refusal {
    Refused(String),
    Failed(String),
}

fn post_form(url: &str, fields: &[(&str, &str)]) -> Result<(u16, String), String> {
    let body = fields.iter().map(|(k, v)| format!("{}={}", url_encode(k), url_encode(v))).collect::<Vec<_>>().join("&");
    let req = HttpRequest {
        method: "POST".into(),
        url: url.into(),
        headers: [("Content-Type".to_string(), "application/x-www-form-urlencoded".to_string()), ("Accept".to_string(), "application/json".to_string())].into_iter().collect(),
        body: Some(body),
        timeout_ms: 0,
    };
    let r = sdk::fetch_online("Google", &req)?;
    Ok((r.status, r.body))
}

fn oauth_error(body: &str) -> OAuthError {
    serde_json::from_str(body).unwrap_or_default()
}

fn save_token(t: TokenAnswer, keep_refresh: &str, now_ms: i64) -> Token {
    let lifetime_ms = if t.expires_in > 0 { t.expires_in * 1000 } else { 3_600_000 };
    let token = Token { access: t.access_token, refresh: t.refresh_token.filter(|r| !r.is_empty()).unwrap_or_else(|| keep_refresh.to_string()), expires_ms: now_ms + lifetime_ms };
    host::kv_set(TOKEN_KEY, serde_json::to_string(&token).unwrap_or_default().as_bytes());
    token
}

const UNKNOWN_CLIENT: &str = "Google does not know this Client ID. Check it, and that the OAuth client is of the type TVs and Limited Input devices.";

/// The token once the code has been entered, so the refresh goes on with it, or else the
/// sentence the plugin's page shows meanwhile. One poll per refresh is all the host's rhythm
/// allows, and Google's codes last long enough for that.
fn device_step(client_id: &str, secret: &str, now_ms: i64) -> Result<Token, String> {
    let pending: Option<Device> = host::kv_get(DEVICE_KEY).and_then(|v| serde_json::from_slice(&v).ok()).filter(|d: &Device| d.expires_ms > now_ms);
    if let Some(d) = pending {
        let (status, body) = post_form(TOKEN_URL, &[("client_id", client_id), ("client_secret", secret), ("device_code", &d.device_code), ("grant_type", "urn:ietf:params:oauth:grant-type:device_code")])?;
        if (200..300).contains(&status) {
            let t: TokenAnswer = serde_json::from_str(&body).map_err(|e| {
                sdk::warn!("unexpected token answer: {e}");
                sdk::unreadable("Google", TOKEN_URL)
            })?;
            host::kv_delete(DEVICE_KEY);
            return Ok(save_token(t, "", now_ms));
        }
        let e = oauth_error(&body);
        match e.error.as_str() {
            "authorization_pending" | "slow_down" => return Err(instruction(&d)),
            "access_denied" => {
                host::kv_delete(DEVICE_KEY);
                return Err("The sign-in was declined on the phone. Press Refresh now for a new code.".into());
            }
            "invalid_client" => return Err(UNKNOWN_CLIENT.into()),
            _ => {
                sdk::warn!("the sign-in code is no longer good: HTTP {status} {} {}", e.error, e.error_description);
                host::kv_delete(DEVICE_KEY);
            }
        }
    }
    let (status, body) = post_form(DEVICE_CODE_URL, &[("client_id", client_id), ("scope", SCOPE)])?;
    if !(200..300).contains(&status) {
        let e = oauth_error(&body);
        sdk::warn!("HTTP {status} when asked for a sign-in code: {} {}", e.error, e.error_description);
        return Err(if e.error == "invalid_client" || status == 401 { UNKNOWN_CLIENT.into() } else { sdk::online_error("Google", status) });
    }
    let a: DeviceAnswer = serde_json::from_str(&body).map_err(|e| {
        sdk::warn!("unexpected sign-in code answer: {e}");
        sdk::unreadable("Google", DEVICE_CODE_URL)
    })?;
    let d = Device { device_code: a.device_code, user_code: a.user_code, verification_url: a.verification_url, expires_ms: now_ms + a.expires_in.max(300) * 1000 };
    host::kv_set(DEVICE_KEY, serde_json::to_string(&d).unwrap_or_default().as_bytes());
    Err(instruction(&d))
}

fn instruction(d: &Device) -> String {
    format!("Sign in: on your phone open {} and enter {}, then press Refresh now here.", d.verification_url.trim_start_matches("https://").trim_end_matches('/'), d.user_code)
}

fn renew(client_id: &str, secret: &str, old: &Token, now_ms: i64) -> Result<Token, Refusal> {
    if old.refresh.is_empty() {
        return Err(Refusal::Refused("no refresh token".into()));
    }
    let (status, body) = post_form(TOKEN_URL, &[("client_id", client_id), ("client_secret", secret), ("refresh_token", &old.refresh), ("grant_type", "refresh_token")]).map_err(Refusal::Failed)?;
    if (200..300).contains(&status) {
        let t: TokenAnswer = serde_json::from_str(&body).map_err(|e| Refusal::Failed(e.to_string()))?;
        return Ok(save_token(t, &old.refresh, now_ms));
    }
    let e = oauth_error(&body);
    // A refresh token Google has let lapse, which a consent screen left in Testing does after
    // a week, means signing in again.
    if matches!(status, 400 | 401) {
        return Err(Refusal::Refused(format!("HTTP {status} {} {}", e.error, e.error_description)));
    }
    Err(Refusal::Failed(format!("HTTP {status}")))
}

/// Marks the saved token as spent, so the next refresh renews it.
fn expire_token() {
    if let Some(mut t) = host::kv_get(TOKEN_KEY).and_then(|v| serde_json::from_slice::<Token>(&v).ok()) {
        t.expires_ms = 0;
        host::kv_set(TOKEN_KEY, serde_json::to_string(&t).unwrap_or_default().as_bytes());
    }
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

// ---------------------------------------------------------------------------
// The account's lists
// ---------------------------------------------------------------------------

#[derive(Deserialize, Default)]
struct Page<T> {
    #[serde(default)]
    items: Vec<T>,
    #[serde(rename = "nextPageToken", default)]
    next: Option<String>,
}

#[derive(Deserialize, Default)]
struct ApiError {
    #[serde(default)]
    error: ApiErrorBody,
}

#[derive(Deserialize, Default)]
struct ApiErrorBody {
    #[serde(default)]
    message: String,
    #[serde(default)]
    errors: Vec<ApiErrorDetail>,
}

#[derive(Deserialize, Default)]
struct ApiErrorDetail {
    #[serde(default)]
    reason: String,
}

#[derive(Deserialize, Default)]
struct Subscription {
    #[serde(default)]
    snippet: SubscriptionSnippet,
}

#[derive(Deserialize, Default)]
struct SubscriptionSnippet {
    #[serde(default)]
    title: String,
    #[serde(rename = "resourceId", default)]
    resource: ResourceId,
}

#[derive(Deserialize, Default)]
struct ResourceId {
    #[serde(rename = "channelId", default)]
    channel_id: Option<String>,
}

#[derive(Deserialize, Default)]
struct Playlist {
    #[serde(default)]
    id: String,
    #[serde(default)]
    snippet: PlaylistSnippet,
    #[serde(rename = "contentDetails", default)]
    details: PlaylistDetails,
}

#[derive(Deserialize, Default)]
struct PlaylistSnippet {
    #[serde(default)]
    title: String,
    #[serde(default)]
    thumbnails: Thumbnails,
}

#[derive(Deserialize, Default)]
struct PlaylistDetails {
    #[serde(rename = "itemCount", default)]
    item_count: Option<u32>,
}

#[derive(Deserialize, Default)]
struct LikedVideo {
    #[serde(default)]
    id: String,
    #[serde(default)]
    snippet: VideoSnippet,
}

#[derive(Deserialize, Default)]
struct VideoSnippet {
    #[serde(default)]
    title: String,
    #[serde(rename = "channelTitle", default)]
    channel: String,
    #[serde(rename = "publishedAt", default)]
    published: Option<String>,
    #[serde(default)]
    thumbnails: Thumbnails,
}

#[derive(Deserialize, Default)]
struct Thumbnails {
    #[serde(default)]
    maxres: Option<Thumb>,
    #[serde(default)]
    standard: Option<Thumb>,
    #[serde(default)]
    high: Option<Thumb>,
    #[serde(default)]
    medium: Option<Thumb>,
    #[serde(default)]
    default: Option<Thumb>,
}

#[derive(Deserialize, Default)]
struct Thumb {
    #[serde(default)]
    url: String,
}

impl Thumbnails {
    /// The largest picture YouTube has; the 1280-wide one is only there for videos that have it.
    fn best(&self) -> Option<String> {
        [&self.maxres, &self.standard, &self.high, &self.medium, &self.default].into_iter().flatten().map(|t| t.url.clone()).find(|u| !u.is_empty())
    }
}

/// A subscribed channel, as the API names it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Subscribed {
    pub channel_id: String,
    pub title: String,
}

impl Client {
    /// One page of the API, or the sentence for what stopped it.
    fn get_json<T: serde::de::DeserializeOwned>(&self, path: &str, budget: &mut Budget) -> Result<T, String> {
        if !budget.spend() {
            return Err("fetch budget for this refresh is used up".into());
        }
        let r = sdk::fetch_online("YouTube", &HttpRequest::get(format!("{API}/{path}")).header("Authorization", &format!("Bearer {}", self.token)).header("Accept", "application/json"))?;
        let name = path.split('?').next().unwrap_or(path);
        match r.status {
            200..=299 => serde_json::from_str(&r.body).map_err(|e| {
                sdk::warn!("unexpected answer for {name}: {e}");
                sdk::unreadable("YouTube", API)
            }),
            401 => {
                expire_token();
                Err(SIGNED_OUT.into())
            }
            s => {
                let e: ApiError = serde_json::from_str(&r.body).unwrap_or_default();
                let reason = e.error.errors.first().map(|d| d.reason.as_str()).unwrap_or("");
                sdk::warn!("HTTP {s} for {name}: {reason} {}", e.error.message);
                Err(match reason {
                    "quotaExceeded" | "dailyLimitExceeded" | "rateLimitExceeded" => QUOTA.into(),
                    "accessNotConfigured" => API_OFF.into(),
                    _ => sdk::online_error("YouTube", s),
                })
            }
        }
    }

    /// The channels the account subscribes to, a few pages at most.
    fn subscriptions(&self, budget: &mut Budget) -> Result<Vec<Subscribed>, String> {
        let mut out = Vec::new();
        let mut page_token: Option<String> = None;
        for _ in 0..MAX_SUB_PAGES {
            let token = page_token.as_deref().map(|t| format!("&pageToken={}", url_encode(t))).unwrap_or_default();
            let page: Page<Subscription> = self.get_json(&format!("subscriptions?part=snippet&mine=true&maxResults=50&order=alphabetical{token}"), budget)?;
            out.extend(page.items.into_iter().filter_map(|s| Some(Subscribed { channel_id: s.snippet.resource.channel_id.filter(|c| !c.is_empty())?, title: s.snippet.title })));
            page_token = page.next.filter(|t| !t.is_empty());
            if page_token.is_none() {
                break;
            }
        }
        Ok(out)
    }

    /// The account's own playlists as tiles, in YouTube's order.
    fn playlists(&self, budget: &mut Budget) -> Result<Vec<Item>, String> {
        let page: Page<Playlist> = self.get_json("playlists?part=snippet,contentDetails&mine=true&maxResults=50", budget)?;
        let count = page.items.len() as i32;
        Ok(page
            .items
            .into_iter()
            .enumerate()
            .filter(|(_, p)| !p.id.is_empty())
            .map(|(i, p)| Item {
                id: format!("playlist:{}", p.id),
                title: p.snippet.title.clone(),
                subtitle: p.details.item_count.map(|n| if n == 1 { "1 video".to_string() } else { format!("{n} videos") }),
                thumbnail: p.snippet.thumbnails.best(),
                row: Some("playlists".into()),
                weight: count - i as i32,
                action: Action { view: Some(format!("https://www.youtube.com/playlist?list={}", url_encode(&p.id))), fallback_launch: Some(APP_PACKAGE.into()), ..Default::default() },
                ..Default::default()
            })
            .collect())
    }

    /// The videos the account liked, most recent like first.
    fn liked(&self, budget: &mut Budget, now_ms: i64) -> Result<Vec<Item>, String> {
        let page: Page<LikedVideo> = self.get_json("videos?part=snippet&myRating=like&maxResults=50", budget)?;
        let count = page.items.len() as i32;
        let recent = now_ms - 3 * 86_400_000;
        Ok(page
            .items
            .into_iter()
            .enumerate()
            .filter(|(_, v)| !v.id.is_empty())
            .map(|(i, v)| {
                let published_ms = v.snippet.published.as_deref().and_then(sdk::parse_iso8601);
                Item {
                    id: format!("liked:{}", v.id),
                    title: v.snippet.title.clone(),
                    subtitle: Some(v.snippet.channel.clone()).filter(|c| !c.is_empty()),
                    added_ms: published_ms,
                    is_new: published_ms.is_some_and(|p| p >= recent),
                    thumbnail: v.snippet.thumbnails.best().or_else(|| Some(thumbnail(&v.id, false))),
                    row: Some("liked".into()),
                    weight: count - i as i32,
                    action: watch(&v.id),
                    ..Default::default()
                }
            })
            .collect())
    }
}

// ---------------------------------------------------------------------------
// Subscriptions' feeds, spread over refreshes
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize, Default, Clone)]
struct CachedChannel {
    title: String,
    fetched_ms: i64,
    /// When the feed was last asked for, whether or not it came.
    #[serde(default)]
    attempted_ms: i64,
    #[serde(default)]
    videos: Vec<Video>,
}

/// The newest videos of each subscribed channel, from the public feeds. A refresh reads the
/// channels not read for longest, as many as the budget allows, and keeps the rest from last
/// time, so a long list of subscriptions still fits the host's fetches.
#[derive(Serialize, Deserialize, Default)]
struct SubsCache {
    channels: BTreeMap<String, CachedChannel>,
}

impl SubsCache {
    fn load() -> SubsCache {
        host::kv_get(SUBS_KEY).and_then(|v| serde_json::from_slice(&v).ok()).unwrap_or_default()
    }

    fn save(&self) {
        let mut trimmed = SubsCache { channels: self.channels.clone() };
        for per_channel in [STORED_PER_CHANNEL, 3, 2, 1] {
            for c in trimmed.channels.values_mut() {
                c.videos.truncate(per_channel);
            }
            if let Ok(json) = serde_json::to_vec(&trimmed)
                && json.len() <= CACHE_BYTES
            {
                host::kv_set(SUBS_KEY, &json);
                return;
            }
        }
        sdk::warn!("too many subscriptions to remember between refreshes; starting over");
        host::kv_delete(SUBS_KEY);
    }

    /// Reads the feeds due, channels not asked for longest first, keeping `ART_RESERVE` fetches
    /// back, and forgets channels no longer subscribed to.
    fn refresh(&mut self, subs: &[Subscribed], now_ms: i64, budget: &mut Budget) {
        self.channels.retain(|id, _| subs.iter().any(|s| &s.channel_id == id));
        let mut due: Vec<&Subscribed> = subs.iter().collect();
        due.sort_by_key(|s| self.channels.get(&s.channel_id).map_or(0, |c| c.attempted_ms.max(c.fetched_ms)));
        let mut read = 0;
        for s in due {
            if budget.left() <= ART_RESERVE || !budget.spend() {
                break;
            }
            read += 1;
            let entry = self.channels.entry(s.channel_id.clone()).or_default();
            entry.title = s.title.clone();
            entry.attempted_ms = now_ms;
            match feed(&Source::Channel(s.channel_id.clone())) {
                Ok(xml) => {
                    let mut videos = parse_feed(&xml);
                    videos.truncate(STORED_PER_CHANNEL);
                    entry.videos = videos;
                    entry.fetched_ms = now_ms;
                }
                // A feed that did not come keeps last time's videos and waits its turn again.
                Err(e) => sdk::warn!("{}: {e}", s.title),
            }
        }
        sdk::info!("{read} of {} subscriptions' feeds read this time", subs.len());
    }
}

/// A channel or a playlist, as the user wrote it. A channel named by its handle or an old-style
/// link is looked up on its page before its feed can be read.
#[derive(Debug, PartialEq, Eq)]
pub enum Source {
    Channel(String),
    Playlist(String),
    Named(String),
}

impl Source {
    fn feed_url(&self) -> String {
        match self {
            Source::Channel(id) => format!("https://www.youtube.com/feeds/videos.xml?channel_id={id}"),
            Source::Playlist(id) => format!("https://www.youtube.com/feeds/videos.xml?playlist_id={id}"),
            Source::Named(name) => page_url(name),
        }
    }

    fn label(&self) -> String {
        match self {
            Source::Channel(id) => format!("channel {id}"),
            Source::Playlist(id) => format!("playlist {id}"),
            Source::Named(name) => format!("channel {name}"),
        }
    }
}

/// Accepts channel ids, `/channel/UC…` links, @handles, `/c/` and `/user/` links, playlist ids
/// and `list=` links, separated by line breaks, spaces, commas or semicolons. A line that names
/// a source may carry other words; one that names nothing is refused, naming it so the user can
/// fix it. A Google Takeout subscriptions export gives its channel links, whether or not the
/// field it was pasted into kept its line breaks.
pub fn parse_sources(raw: &str) -> Result<Vec<Source>, String> {
    if raw.trim_start().get(..10).is_some_and(|heading| heading.eq_ignore_ascii_case("channel id")) {
        let mut out: Vec<Source> = raw.split("/channel/").skip(1).map(id_token).filter(|id| !id.is_empty()).map(Source::Channel).collect();
        out.dedup();
        return Ok(out);
    }
    let mut out = Vec::new();
    for line in raw.lines().map(str::trim).filter(|l| !l.is_empty()) {
        let before = out.len();
        let mut unknown: Option<&str> = None;
        for piece in line.split([',', ';']).map(str::trim).filter(|s| !s.is_empty()) {
            let found = out.len();
            out.extend(piece.split_whitespace().filter_map(parse_piece));
            if out.len() == found {
                unknown = unknown.or(Some(piece));
            }
        }
        if out.len() == before
            && let Some(piece) = unknown
        {
            return Err(format!("\"{piece}\" is not a channel or playlist id or link. Check the channels in the plugin's settings."));
        }
    }
    out.dedup();
    Ok(out)
}

fn parse_piece(piece: &str) -> Option<Source> {
    // A bare id pasted from a share link can carry "?si=…" after it.
    let token = id_token(piece);
    if let Some(id) = find_param(piece, "list=") {
        Some(Source::Playlist(id))
    } else if let Some(id) = find_param(piece, "channel_id=") {
        Some(Source::Channel(id))
    } else if let Some(rest) = piece.split("/channel/").nth(1) {
        Some(Source::Channel(id_token(rest)))
    } else if let Some(rest) = piece.split("/playlist/").nth(1) {
        Some(Source::Playlist(id_token(rest)))
    } else if token.starts_with("UC") && token.len() == 24 && !piece.contains('/') {
        Some(Source::Channel(token))
    } else if ["PL", "UU", "OL", "FL", "RD"].iter().any(|p| token.starts_with(p)) && token.len() >= 13 && !piece.contains('/') {
        Some(Source::Playlist(token))
    } else if let Some(rest) = piece.strip_prefix('@').or_else(|| piece.split("/@").nth(1)) {
        let handle = name_token(rest);
        (!handle.is_empty()).then(|| Source::Named(format!("@{handle}")))
    } else if let Some(rest) = piece.split("/c/").nth(1) {
        let name = name_token(rest);
        (!name.is_empty()).then(|| Source::Named(format!("c/{name}")))
    } else if let Some(rest) = piece.split("/user/").nth(1) {
        let name = name_token(rest);
        (!name.is_empty()).then(|| Source::Named(format!("user/{name}")))
    } else {
        None
    }
}

fn page_url(name: &str) -> String {
    let mut path = String::new();
    for b in name.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'/' | b'@' => path.push(b as char),
            _ => path.push_str(&format!("%{b:02X}")),
        }
    }
    format!("https://www.youtube.com/{path}")
}

/// Turns channels named by a handle or an old-style link into channel ids from the channel's own
/// page, each looked up once and remembered. One that cannot be found this time is left out and
/// said so.
fn resolve_named(sources: Vec<Source>, budget: &mut Budget, failures: &mut Vec<(String, String)>) -> Vec<Source> {
    let mut known: BTreeMap<String, String> = host::kv_get_string(HANDLES_KEY).and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
    let mut learned = false;
    let mut out = Vec::new();
    for source in sources {
        let Source::Named(name) = source else {
            out.push(source);
            continue;
        };
        if let Some(id) = known.get(&name) {
            out.push(Source::Channel(id.clone()));
            continue;
        }
        if !budget.spend() {
            failures.push((format!("channel {name}"), "no fetches left this time; it is looked up next time".into()));
            continue;
        }
        match channel_page_id(&name) {
            Ok(id) => {
                known.insert(name, id.clone());
                learned = true;
                out.push(Source::Channel(id));
            }
            Err(e) => failures.push((format!("channel {name}"), e)),
        }
    }
    if learned {
        host::kv_set(HANDLES_KEY, serde_json::to_string(&known).unwrap_or_default().as_bytes());
    }
    out
}

/// The channel id on the page at youtube.com/{name}. The consent cookie keeps YouTube from
/// answering with its cookie notice instead of the page, and a desktop browser's name keeps it
/// from sending the TV to the mobile site.
fn channel_page_id(name: &str) -> Result<String, String> {
    let req = HttpRequest::get(page_url(name))
        .header("Accept", "text/html")
        .header("Accept-Language", "en")
        .header("User-Agent", "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0 Safari/537.36")
        .header("Cookie", "SOCS=CAI; CONSENT=YES+");
    let r = sdk::fetch_online("YouTube", &req)?;
    match r.status {
        404 => Err(format!("YouTube has no channel {name}. Check the channels in the plugin's settings.")),
        s if !(200..300).contains(&s) => Err(sdk::online_error("YouTube", s)),
        _ => {
            let html = if r.body_base64.is_some() { String::from_utf8_lossy(&sdk::body_bytes(r)).into_owned() } else { r.body };
            channel_id_in(&html).ok_or_else(|| format!("YouTube's page for {name} does not say which channel it is. Use the channel's id (starts with UC) instead."))
        }
    }
}

/// The page's own channel id: its canonical link, else its identifier, else its external id.
pub fn channel_id_in(html: &str) -> Option<String> {
    let canonical = html.find("rel=\"canonical\" href=\"").and_then(|start| {
        let link = &html[start..];
        let end = link[23..].find('"')? + 23;
        let channel = link[..end].find("/channel/")? + 9;
        Some(id_token(&link[channel..end]))
    });
    let marked = |marker: &str| html.find(marker).map(|start| id_token(&html[start + marker.len()..]));
    [canonical, marked("itemprop=\"identifier\" content=\""), marked("\"externalId\":\"")].into_iter().flatten().find(|id| id.starts_with("UC") && id.len() == 24)
}

fn find_param(s: &str, key: &str) -> Option<String> {
    let start = s.find(key)? + key.len();
    Some(id_token(&s[start..]))
}

fn id_token(s: &str) -> String {
    s.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-').collect()
}

/// A handle or channel name as it appears in a link, which may carry dots.
fn name_token(s: &str) -> String {
    s.chars().take_while(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.')).collect()
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Video {
    pub id: String,
    pub title: String,
    pub channel: String,
    pub published_ms: i64,
    /// A vertical Short rather than a video.
    #[serde(default)]
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
        assert_eq!(parse_sources("PLx1234567890abc?si=Zx9").unwrap(), vec![Source::Playlist("PLx1234567890abc".into())]);
        assert_eq!(parse_sources("not a thing").unwrap_err(), "\"not a thing\" is not a channel or playlist id or link. Check the channels in the plugin's settings.");
        // A line that names a channel may carry other words, as an export's title column does.
        assert_eq!(parse_sources("UCBJycsmduvYEL83R_U4JriQ, not a thing").unwrap(), vec![Source::Channel("UCBJycsmduvYEL83R_U4JriQ".into())]);
    }

    #[test]
    fn handles_and_old_links_are_channels_to_look_up() {
        let s = parse_sources("https://www.youtube.com/@mkbhd/videos\n@Veritasium\nhttps://youtube.com/c/SomeName?si=1\nhttps://www.youtube.com/user/oldname").unwrap();
        assert_eq!(s, vec![Source::Named("@mkbhd".into()), Source::Named("@Veritasium".into()), Source::Named("c/SomeName".into()), Source::Named("user/oldname".into())]);
        assert_eq!(Source::Named("@mkbhd".into()).feed_url(), "https://www.youtube.com/@mkbhd");
        assert_eq!(page_url("@a b"), "https://www.youtube.com/@a%20b");
        assert_eq!(Source::Named("c/x".into()).label(), "channel c/x");
        assert!(parse_sources("@").is_err());
        // Line breaks pasted into a one-line field arrive as spaces.
        let spaced = parse_sources("@one @two https://www.youtube.com/@three").unwrap();
        assert_eq!(spaced, vec![Source::Named("@one".into()), Source::Named("@two".into()), Source::Named("@three".into())]);
    }

    #[test]
    fn a_takeout_export_can_be_pasted_whole() {
        let csv = "Channel Id,Channel Url,Channel Title\nUCBJycsmduvYEL83R_U4JriQ,http://www.youtube.com/channel/UCBJycsmduvYEL83R_U4JriQ,Marques Brownlee\nUCHnyfMqiRRG1u-2MsSQLbXA,http://www.youtube.com/channel/UCHnyfMqiRRG1u-2MsSQLbXA,\"Smith, John\"\n";
        let s = parse_sources(csv).unwrap();
        assert_eq!(s, vec![Source::Channel("UCBJycsmduvYEL83R_U4JriQ".into()), Source::Channel("UCHnyfMqiRRG1u-2MsSQLbXA".into())]);
        // A one-line field turns the line breaks into spaces, or drops them.
        assert_eq!(parse_sources(&csv.replace('\n', " ")).unwrap(), s);
        assert_eq!(parse_sources(&csv.replace('\n', "")).unwrap(), s);
        // Rows without the heading still name their channels, and a title's words name none.
        let rows = "UCBJycsmduvYEL83R_U4JriQ,http://www.youtube.com/channel/UCBJycsmduvYEL83R_U4JriQ,Me@Work UCHnyfMqiRRG1u-2MsSQLbXA,http://www.youtube.com/channel/UCHnyfMqiRRG1u-2MsSQLbXA,Other";
        assert_eq!(parse_sources(rows).unwrap(), s);
    }

    #[test]
    fn a_channel_page_names_its_channel() {
        let page = r#"<html><link rel="canonical" href="https://www.youtube.com/channel/UCBJycsmduvYEL83R_U4JriQ"><meta itemprop="identifier" content="UCother000000000000000x">"#;
        assert_eq!(channel_id_in(page).as_deref(), Some("UCBJycsmduvYEL83R_U4JriQ"));
        assert_eq!(channel_id_in(r#"<link rel="canonical" href="https://m.youtube.com/channel/UCBJycsmduvYEL83R_U4JriQ/videos">"#).as_deref(), Some("UCBJycsmduvYEL83R_U4JriQ"));
        assert_eq!(channel_id_in(r#"<link rel="canonical" href="https://www.youtube.com/@mkbhd"><meta itemprop="identifier" content="UCBJycsmduvYEL83R_U4JriQ">"#).as_deref(), Some("UCBJycsmduvYEL83R_U4JriQ"));
        assert_eq!(channel_id_in(r#"<link rel="canonical" href="https://www.youtube.com/channel/UCBJycsmduvYEL83R_U4JriQ"#), None);
        assert_eq!(channel_id_in(r#"<meta itemprop="identifier" content="UCHnyfMqiRRG1u-2MsSQLbXA">"#).as_deref(), Some("UCHnyfMqiRRG1u-2MsSQLbXA"));
        assert_eq!(channel_id_in(r#"{"externalId":"UCHnyfMqiRRG1u-2MsSQLbXA","channelId":"UCBJycsmduvYEL83R_U4JriQ"}"#).as_deref(), Some("UCHnyfMqiRRG1u-2MsSQLbXA"));
        assert_eq!(channel_id_in(r#"{"channelId":"UCBJycsmduvYEL83R_U4JriQ"}"#), None);
        assert_eq!(channel_id_in("nothing here"), None);
    }

    #[test]
    fn a_handle_that_cannot_be_looked_up_is_reported_not_fatal() {
        let mut budget = Budget::new(1);
        let mut failures = Vec::new();
        let sources = resolve_named(vec![Source::Named("@one".into()), Source::Channel("UCx".into()), Source::Named("@two".into())], &mut budget, &mut failures);
        assert_eq!(sources, vec![Source::Channel("UCx".into())]);
        assert_eq!(budget.left(), 0);
        assert_eq!(failures.len(), 2);
        assert_eq!(failures[0].0, "channel @one");
        assert_eq!(failures[1].1, "no fetches left this time; it is looked up next time");
    }

    #[test]
    fn feed_urls_point_at_youtube() {
        assert_eq!(Source::Channel("UCx".into()).feed_url(), "https://www.youtube.com/feeds/videos.xml?channel_id=UCx");
        assert_eq!(Source::Playlist("PLy".into()).feed_url(), "https://www.youtube.com/feeds/videos.xml?playlist_id=PLy");
    }

    #[test]
    fn the_apis_pages_are_read() {
        let page: Page<Subscription> = serde_json::from_str(
            r#"{"kind":"youtube#subscriptionListResponse","nextPageToken":"CDIQAA","items":[
                {"id":"s1","snippet":{"title":"Marques Brownlee","resourceId":{"kind":"youtube#channel","channelId":"UCBJycsmduvYEL83R_U4JriQ"},"thumbnails":{"default":{"url":"https://yt3.ggpht.com/a"}}}},
                {"id":"s2","snippet":{"title":"Odd","resourceId":{"kind":"youtube#channel"}}}]}"#,
        )
        .unwrap();
        assert_eq!(page.next.as_deref(), Some("CDIQAA"));
        let subs: Vec<Subscribed> = page.items.into_iter().filter_map(|s| Some(Subscribed { channel_id: s.snippet.resource.channel_id.filter(|c| !c.is_empty())?, title: s.snippet.title })).collect();
        assert_eq!(subs, vec![Subscribed { channel_id: "UCBJycsmduvYEL83R_U4JriQ".into(), title: "Marques Brownlee".into() }]);

        let playlists: Page<Playlist> = serde_json::from_str(
            r#"{"items":[{"id":"PLabc","snippet":{"title":"Cooking","thumbnails":{"default":{"url":"d"},"high":{"url":"h"},"maxres":{"url":"m"}}},"contentDetails":{"itemCount":12}},
                        {"id":"PLone","snippet":{"title":"One","thumbnails":{"default":{"url":"d"}}},"contentDetails":{"itemCount":1}}]}"#,
        )
        .unwrap();
        assert_eq!(playlists.items[0].snippet.thumbnails.best().as_deref(), Some("m"));
        assert_eq!(playlists.items[1].snippet.thumbnails.best().as_deref(), Some("d"));
        assert_eq!(playlists.items[0].details.item_count, Some(12));

        let liked: Page<LikedVideo> = serde_json::from_str(r#"{"items":[{"id":"abc123DEF45","snippet":{"title":"Liked one","channelTitle":"Someone","publishedAt":"2026-10-10T10:00:00Z","thumbnails":{"high":{"url":"h"}}}}]}"#).unwrap();
        assert_eq!((liked.items[0].id.as_str(), liked.items[0].snippet.channel.as_str()), ("abc123DEF45", "Someone"));
        assert_eq!(liked.items[0].snippet.published.as_deref().and_then(sdk::parse_iso8601), sdk::parse_iso8601("2026-10-10T10:00:00Z"));

        let e: ApiError = serde_json::from_str(r#"{"error":{"code":403,"message":"The request cannot be completed because you have exceeded your quota.","errors":[{"message":"…","domain":"youtube.quota","reason":"quotaExceeded"}]}}"#).unwrap();
        assert_eq!(e.error.errors[0].reason, "quotaExceeded");
    }

    #[test]
    fn googles_sign_in_answers_are_read() {
        let d: DeviceAnswer = serde_json::from_str(r#"{"device_code":"AH-1Ng","user_code":"GQVQ-JKEC","expires_in":1800,"interval":5,"verification_url":"https://www.google.com/device"}"#).unwrap();
        let device = Device { device_code: d.device_code, user_code: d.user_code, verification_url: d.verification_url, expires_ms: 0 };
        assert_eq!(instruction(&device), "Sign in: on your phone open www.google.com/device and enter GQVQ-JKEC, then press Refresh now here.");
        let t: TokenAnswer = serde_json::from_str(r#"{"access_token":"ya29.a","expires_in":3599,"refresh_token":"1//0g","scope":"https://www.googleapis.com/auth/youtube.readonly","token_type":"Bearer"}"#).unwrap();
        assert_eq!((t.access_token.as_str(), t.refresh_token.as_deref(), t.expires_in), ("ya29.a", Some("1//0g"), 3599));
        let e = oauth_error(r#"{"error":"authorization_pending","error_description":"Precondition Required"}"#);
        assert_eq!(e.error, "authorization_pending");
        assert_eq!(oauth_error("nope").error, "");
        assert_eq!(url_encode("urn:ietf:params:oauth:grant-type:device_code"), "urn%3Aietf%3Aparams%3Aoauth%3Agrant-type%3Adevice_code");
    }

    #[test]
    fn subscriptions_are_read_longest_ago_first_within_the_budget() {
        let subs: Vec<Subscribed> = (1..=5).map(|i| Subscribed { channel_id: format!("UC{i}"), title: format!("Channel {i}") }).collect();
        let mut cache = SubsCache::default();
        cache.channels.insert("UC1".into(), CachedChannel { title: "Channel 1".into(), fetched_ms: 500, ..Default::default() });
        cache.channels.insert("UC2".into(), CachedChannel { title: "Channel 2".into(), fetched_ms: 100, ..Default::default() });
        cache.channels.insert("UCgone".into(), CachedChannel { title: "Unsubscribed".into(), fetched_ms: 900, ..Default::default() });
        // Two fetches above the reserve: the three never-read channels come before the two read ones.
        let mut budget = Budget::new(ART_RESERVE + 2);
        cache.refresh(&subs, 1_000, &mut budget);
        assert_eq!(budget.left(), ART_RESERVE);
        assert!(!cache.channels.contains_key("UCgone"));
        // Off the TV the feeds fail, so the entries exist with their titles but no read time.
        let read: Vec<&str> = cache.channels.iter().filter(|(_, c)| c.fetched_ms == 0).map(|(id, _)| id.as_str()).collect();
        assert_eq!(read.len(), 2);
        assert!(read.iter().all(|id| ["UC3", "UC4", "UC5"].contains(id)));
        assert_eq!(cache.channels["UC2"].fetched_ms, 100);
        // A feed that did not come waits its turn: next the channel never asked, then the oldest read.
        let mut budget = Budget::new(ART_RESERVE + 2);
        cache.refresh(&subs, 2_000, &mut budget);
        assert_eq!(cache.channels.values().filter(|c| c.attempted_ms == 1_000).count(), 2);
        assert_eq!(cache.channels["UC2"].attempted_ms, 2_000);
        assert_eq!(cache.channels["UC1"].attempted_ms, 0);
    }

    #[test]
    fn a_budget_runs_out_quietly() {
        let mut b = Budget::new(2);
        assert!(b.spend());
        assert!(b.spend());
        assert!(!b.spend());
        assert_eq!(b.left(), 0);
        let client = Client { token: "t".into() };
        assert_eq!(client.get_json::<Page<Playlist>>("playlists?mine=true", &mut b).err().as_deref(), Some("fetch budget for this refresh is used up"));
    }

    #[test]
    fn without_a_client_id_there_is_no_account() {
        let mut budget = Budget::new(FETCH_BUDGET);
        assert!(Account::connect(0, &mut budget).unwrap().is_none());
        assert_eq!(budget.left(), FETCH_BUDGET);
        assert!(<YouTube as sdk::Plugin>::manifest().settings.iter().all(|s| !s.required));
    }
}
