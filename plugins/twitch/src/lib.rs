//! Twitch: the channels you follow that are live right now become channels in Live now and the
//! guide, each with what is being streamed and since when, and open in the Twitch app. Sign-in is
//! Twitch's device code, shown on the plugin's page until it has been entered at twitch.tv/activate.
use lounge_plugin_sdk::{self as sdk, host, parse_iso8601, Action, HttpRequest, Item, ItemKind, Kind, Manifest, RefreshContext, SettingKind, SettingSpec};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub struct Twitch;

const API: &str = "https://api.twitch.tv/helix";
const ID: &str = "https://id.twitch.tv/oauth2";
const SCOPES: &str = "user:read:follows";
const TOKEN_KEY: &str = "token";
const DEVICE_KEY: &str = "device";
const USER_KEY: &str = "user";
const AVATARS_KEY: &str = "avatars";
const DEFAULT_APP: &str = "tv.twitch.android.app";
/// Previews change every few minutes; the address changes with them so the TV fetches a new one.
const PREVIEW_MS: i64 = 10 * 60_000;

impl sdk::Plugin for Twitch {
    fn manifest() -> Manifest {
        Manifest {
            id: "tv.lounge.twitch".into(),
            name: "Twitch".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            description: "The channels you follow that are live now, in Live now and the guide, opening in the Twitch app.".into(),
            publisher_name: "Lounge".into(),
            kinds: vec![Kind::Content, Kind::Epg],
            fetch_hosts: vec!["id.twitch.tv".into(), "api.twitch.tv".into(), "static-cdn.jtvnw.net".into()],
            refresh_minutes: 5,
            rows: vec![],
            settings: vec![
                SettingSpec {
                    key: "client_id".into(),
                    label: "Client ID".into(),
                    kind: SettingKind::Text,
                    required: true,
                    help: Some("At dev.twitch.tv/console register an application (category Other, type Public, redirect http://localhost) and copy its Client ID.".into()),
                    default: None,
                },
                SettingSpec { key: "app".into(), label: "Twitch app".into(), kind: SettingKind::Text, required: false, help: Some("The package that opens streams.".into()), default: Some(DEFAULT_APP.into()) },
            ],
            homepage: Some("https://www.twitch.tv".into()),
        }
    }

    fn refresh(ctx: &RefreshContext) -> Result<Vec<Item>, String> {
        let client = Client::connect(ctx.now_ms)?;
        let app = host::setting("app").map(|a| a.trim().to_string()).filter(|a| !a.is_empty()).unwrap_or_else(|| DEFAULT_APP.into());
        let user_id = client.user_id()?;
        let streams: Page<Stream> = client.get_json(&format!("/streams/followed?user_id={user_id}&first=100"))?;
        let mut avatars = Avatars::load();
        avatars.expire(ctx.now_ms);
        let missing: Vec<&str> = streams.data.iter().map(|s| s.user_id.as_str()).filter(|id| !avatars.has(id)).take(100).collect();
        if !missing.is_empty() {
            let query: Vec<String> = missing.iter().map(|id| format!("id={id}")).collect();
            match client.get_json::<Page<User>>(&format!("/users?{}", query.join("&"))) {
                Ok(users) => avatars.add(&users.data),
                Err(e) => sdk::warn!("profile pictures: {e}"),
            }
        }
        avatars.save();
        let mut items = Vec::new();
        for s in &streams.data {
            items.extend(stream_items(s, avatars.get(&s.user_id), &app, ctx.now_ms));
        }
        sdk::info!("{} channels live", streams.data.len());
        Ok(items)
    }
}

sdk::export_plugin!(Twitch);

// ---------------------------------------------------------------------------
// Signing in: Twitch's device code, one step per refresh
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize)]
struct Token {
    access: String,
    refresh: String,
    expires_ms: i64,
}

/// Why a renewal failed: Twitch refused the refresh token, or no usable answer came back.
enum RenewError {
    Refused(u16),
    Failed(String),
}

/// What follows a renewal: the token to use, a new sign-in, or an error until the next refresh.
enum Renewed {
    Use(Token),
    SignInAgain,
    TryLater(String),
}

fn after_renewal(current: Token, outcome: Result<Token, RenewError>, now_ms: i64) -> Renewed {
    match outcome {
        Ok(fresh) => Renewed::Use(fresh),
        // Only a refresh token Twitch refuses needs the person to sign in again.
        Err(RenewError::Refused(status)) => {
            sdk::warn!("Twitch refused to renew the sign-in (HTTP {status}); signing in again");
            Renewed::SignInAgain
        }
        // Anything else may pass, so the current token serves while it lasts.
        Err(RenewError::Failed(e)) if current.expires_ms > now_ms => {
            sdk::warn!("could not renew the sign-in ({e}); using the current one");
            Renewed::Use(current)
        }
        Err(RenewError::Failed(e)) => {
            sdk::warn!("could not renew the sign-in ({e})");
            Renewed::TryLater("Could not renew the Twitch sign-in. It tries again on the next refresh.".into())
        }
    }
}

/// Marks the saved token as spent, so the next refresh renews it with the refresh token.
fn expire_token() {
    if let Some(mut t) = host::kv_get(TOKEN_KEY).and_then(|v| serde_json::from_slice::<Token>(&v).ok()) {
        t.expires_ms = 0;
        host::kv_set(TOKEN_KEY, serde_json::to_string(&t).unwrap_or_default().as_bytes());
    }
}

#[derive(Serialize, Deserialize)]
struct Device {
    device_code: String,
    user_code: String,
    verification_uri: String,
    expires_ms: i64,
}

#[derive(Deserialize)]
struct DeviceAnswer {
    device_code: String,
    user_code: String,
    verification_uri: String,
    expires_in: i64,
}

#[derive(Deserialize)]
struct TokenAnswer {
    access_token: String,
    #[serde(default)]
    refresh_token: String,
    #[serde(default)]
    expires_in: i64,
}

#[derive(Deserialize, Default)]
struct Failure {
    #[serde(default)]
    message: String,
}

struct Client {
    client_id: String,
    token: String,
}

impl Client {
    fn connect(now_ms: i64) -> Result<Client, String> {
        let client_id = host::required_setting("client_id", "Client ID")?.trim().to_string();
        let saved: Option<Token> = host::kv_get(TOKEN_KEY).and_then(|v| serde_json::from_slice(&v).ok());
        let token = match saved {
            Some(t) if t.expires_ms > now_ms + 10 * 60_000 => t,
            Some(t) => {
                let outcome = renew(&client_id, &t, now_ms);
                match after_renewal(t, outcome, now_ms) {
                    Renewed::Use(t) => t,
                    Renewed::SignInAgain => {
                        host::kv_delete(TOKEN_KEY);
                        host::kv_delete(USER_KEY);
                        device_step(&client_id, now_ms)?
                    }
                    Renewed::TryLater(e) => return Err(e),
                }
            }
            None => device_step(&client_id, now_ms)?,
        };
        Ok(Client { client_id, token: token.access })
    }

    fn get_json<T: serde::de::DeserializeOwned>(&self, path: &str) -> Result<T, String> {
        let req = HttpRequest::get(format!("{API}{path}")).header("Client-Id", &self.client_id).header("Authorization", &format!("Bearer {}", self.token));
        let r = sdk::fetch_online("Twitch", &req)?;
        let path = path.split('?').next().unwrap_or(path);
        match r.status {
            401 => {
                expire_token();
                return Err("Twitch no longer accepts this sign-in; it is renewed on the next refresh, or press Refresh now.".into());
            }
            s if !(200..300).contains(&s) => {
                let why = serde_json::from_str::<Failure>(&r.body).map(|f| f.message).unwrap_or_default();
                sdk::warn!("HTTP {s} for {path}: {why}");
                return Err(sdk::online_error("Twitch", s));
            }
            _ => {}
        }
        serde_json::from_str(&r.body).map_err(|e| {
            sdk::warn!("unexpected answer for {path}: {e}");
            sdk::unreadable("Twitch", "twitch.tv")
        })
    }

    /// The signed-in user's id, looked up once and kept.
    fn user_id(&self) -> Result<String, String> {
        if let Some(id) = host::kv_get_string(USER_KEY).filter(|s| !s.is_empty()) {
            return Ok(id);
        }
        let me: Page<User> = self.get_json("/users")?;
        let id = me.data.first().map(|u| u.id.clone()).filter(|s| !s.is_empty()).ok_or("Twitch did not say who is signed in. It tries again on the next refresh.")?;
        host::kv_set(USER_KEY, id.as_bytes());
        Ok(id)
    }
}

fn post_form(url: &str, form: &[(&str, &str)]) -> Result<(u16, String), String> {
    let body: Vec<String> = form.iter().map(|(k, v)| format!("{k}={}", url_encode(v))).collect();
    let req = HttpRequest {
        method: "POST".into(),
        url: url.to_string(),
        headers: [("Content-Type".to_string(), "application/x-www-form-urlencoded".to_string())].into_iter().collect(),
        body: Some(body.join("&")),
        timeout_ms: 0,
    };
    let r = sdk::fetch_online("Twitch", &req)?;
    Ok((r.status, r.body))
}

/// The error for a sign-in answer that cannot be read; the cause goes to the log.
fn unreadable_sign_in(e: serde_json::Error) -> String {
    sdk::warn!("unexpected sign-in answer: {e}");
    sdk::unreadable("Twitch", "twitch.tv")
}

/// The token once the sign-in has gone through, so the refresh goes on with it, or else the
/// sentence the plugin's page shows meanwhile. One poll per refresh is all the host's rhythm
/// allows.
fn device_step(client_id: &str, now_ms: i64) -> Result<Token, String> {
    let pending: Option<Device> = host::kv_get(DEVICE_KEY).and_then(|v| serde_json::from_slice(&v).ok()).filter(|d: &Device| d.expires_ms > now_ms);
    if let Some(d) = pending {
        let (status, body) = post_form(&format!("{ID}/token"), &[("client_id", client_id), ("scopes", SCOPES), ("device_code", &d.device_code), ("grant_type", "urn:ietf:params:oauth:grant-type:device_code")])?;
        if (200..300).contains(&status) {
            let t: TokenAnswer = serde_json::from_str(&body).map_err(unreadable_sign_in)?;
            host::kv_delete(DEVICE_KEY);
            return Ok(save_token(&t, now_ms));
        }
        let why = serde_json::from_str::<Failure>(&body).map(|f| f.message).unwrap_or_default();
        if why.contains("authorization_pending") || why.contains("slow_down") {
            return Err(instruction(&d));
        }
        host::kv_delete(DEVICE_KEY);
        if why.contains("access_denied") {
            return Err("The sign-in was declined on twitch.tv. Press Refresh now for a new code.".into());
        }
    }
    let (status, body) = post_form(&format!("{ID}/device"), &[("client_id", client_id), ("scopes", SCOPES)])?;
    if !(200..300).contains(&status) {
        let why = serde_json::from_str::<Failure>(&body).map(|f| f.message).unwrap_or_default();
        if status == 400 && why.contains("client") {
            return Err("Twitch does not know this Client ID. Check it against your application at dev.twitch.tv/console.".into());
        }
        sdk::warn!("HTTP {status} when asked for a sign-in code: {why}");
        return Err(sdk::online_error("Twitch", status));
    }
    let a: DeviceAnswer = serde_json::from_str(&body).map_err(unreadable_sign_in)?;
    let d = Device { device_code: a.device_code, user_code: a.user_code, verification_uri: a.verification_uri, expires_ms: now_ms + a.expires_in * 1000 };
    host::kv_set(DEVICE_KEY, serde_json::to_string(&d).unwrap_or_default().as_bytes());
    Err(instruction(&d))
}

fn instruction(d: &Device) -> String {
    // The address Twitch gives already carries the code; the short form is what people read out.
    let page = d.verification_uri.trim_start_matches("https://").split('?').next().unwrap_or("twitch.tv/activate");
    format!("Sign in: on your phone open {page} and enter {}, then press Refresh now here.", d.user_code)
}

fn renew(client_id: &str, old: &Token, now_ms: i64) -> Result<Token, RenewError> {
    let (status, body) = post_form(&format!("{ID}/token"), &[("client_id", client_id), ("grant_type", "refresh_token"), ("refresh_token", &old.refresh)]).map_err(RenewError::Failed)?;
    if matches!(status, 400 | 401) {
        return Err(RenewError::Refused(status));
    }
    if !(200..300).contains(&status) {
        return Err(RenewError::Failed(format!("HTTP {status}")));
    }
    let t: TokenAnswer = serde_json::from_str(&body).map_err(|e| RenewError::Failed(e.to_string()))?;
    Ok(save_token(&t, now_ms))
}

fn save_token(t: &TokenAnswer, now_ms: i64) -> Token {
    let expires = if t.expires_in > 0 { t.expires_in } else { 4 * 3600 };
    let token = Token { access: t.access_token.clone(), refresh: t.refresh_token.clone(), expires_ms: now_ms + expires * 1000 };
    host::kv_set(TOKEN_KEY, serde_json::to_string(&token).unwrap_or_default().as_bytes());
    token
}

// ---------------------------------------------------------------------------
// What Twitch sends
// ---------------------------------------------------------------------------

#[derive(Deserialize, Default)]
struct Page<T> {
    #[serde(default)]
    data: Vec<T>,
}

#[derive(Deserialize, Default, Clone, Debug)]
pub struct Stream {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub user_id: String,
    #[serde(default)]
    pub user_login: String,
    #[serde(default)]
    pub user_name: String,
    #[serde(default)]
    pub game_name: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub viewer_count: u64,
    #[serde(default)]
    pub started_at: String,
    /// A template with `{width}` and `{height}` in it.
    #[serde(default)]
    pub thumbnail_url: String,
}

#[derive(Deserialize, Default, Clone, Debug)]
pub struct User {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub profile_image_url: String,
}

/// Profile pictures by user id, kept so each is asked for once a week.
#[derive(Serialize, Deserialize, Default)]
struct Avatars {
    urls: BTreeMap<String, String>,
    #[serde(default)]
    fetched_ms: i64,
}

impl Avatars {
    fn load() -> Avatars {
        host::kv_get(AVATARS_KEY).and_then(|v| serde_json::from_slice(&v).ok()).unwrap_or_default()
    }
    fn has(&self, id: &str) -> bool {
        self.urls.contains_key(id)
    }
    fn get(&self, id: &str) -> Option<&str> {
        self.urls.get(id).map(String::as_str).filter(|u| !u.is_empty())
    }
    /// Forgets pictures a week old, before the refresh decides which to fetch, so it fetches them
    /// all again rather than showing none.
    fn expire(&mut self, now_ms: i64) {
        if now_ms - self.fetched_ms > 7 * 86_400_000 {
            self.urls.clear();
            self.fetched_ms = now_ms;
        }
    }
    fn add(&mut self, users: &[User]) {
        for u in users {
            self.urls.insert(u.id.clone(), u.profile_image_url.clone());
        }
        while self.urls.len() > 500 {
            let first = self.urls.keys().next().cloned();
            if let Some(k) = first {
                self.urls.remove(&k);
            }
        }
    }
    fn save(&self) {
        host::kv_set(AVATARS_KEY, serde_json::to_string(self).unwrap_or_default().as_bytes());
    }
}

/// A live stream as a channel that is on, and the stream itself as what is on it.
pub fn stream_items(s: &Stream, avatar: Option<&str>, app: &str, now_ms: i64) -> Vec<Item> {
    if s.user_login.is_empty() {
        return Vec::new();
    }
    let name = if s.user_name.trim().is_empty() { s.user_login.clone() } else { s.user_name.clone() };
    let preview = (!s.thumbnail_url.is_empty()).then(|| format!("{}?r={}", s.thumbnail_url.replace("{width}", "1280").replace("{height}", "720"), now_ms / PREVIEW_MS));
    let action = Action { view: Some(format!("twitch://stream/{}", s.user_login)), fallback_launch: Some(app.to_string()), title: Some(name.clone()), ..Default::default() };
    let channel = Item {
        id: format!("ch:{}", s.user_login),
        kind: ItemKind::Channel,
        title: name.clone(),
        channel_id: Some(s.user_login.clone()),
        channel_name: Some(name.clone()),
        logo: avatar.map(str::to_string),
        thumbnail: preview.clone(),
        action: action.clone(),
        ..Default::default()
    };
    let mut lines = Vec::new();
    if !s.game_name.trim().is_empty() {
        lines.push(s.game_name.trim().to_string());
    }
    lines.push(watching(s.viewer_count));
    let live = Item {
        id: format!("live:{}", if s.id.is_empty() { s.user_login.clone() } else { s.id.clone() }),
        kind: ItemKind::Schedule,
        title: if s.title.trim().is_empty() { name.clone() } else { clip(s.title.trim(), 90) },
        subtitle: Some(lines.join(", ")),
        channel_id: Some(s.user_login.clone()),
        channel_name: Some(name),
        start_ms: Some(parse_iso8601(&s.started_at).unwrap_or(now_ms)),
        thumbnail: preview,
        action,
        ..Default::default()
    };
    vec![channel, live]
}

fn watching(n: u64) -> String {
    match n {
        0 => "nobody watching yet".into(),
        1 => "1 watching".into(),
        n if n < 1000 => format!("{n} watching"),
        n => format!("{:.1}k watching", n as f64 / 1000.0),
    }
}

fn clip(s: &str, max: usize) -> String {
    if s.chars().count() <= max { s.to_string() } else { format!("{}…", s.chars().take(max - 1).collect::<String>().trim_end()) }
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
    fn a_live_stream_is_a_channel_with_something_on() {
        let page: Page<Stream> = serde_json::from_str(r#"{"data": [{"id": "41375541868", "user_id": "459331509", "user_login": "summoningsalt", "user_name": "SummoningSalt",
            "game_id": "509658", "game_name": "Super Mario 64", "type": "live", "title": "16 star speedruns, then history video research", "viewer_count": 2742,
            "started_at": "2026-10-21T19:00:00Z", "language": "en", "thumbnail_url": "https://static-cdn.jtvnw.net/previews-ttv/live_user_summoningsalt-{width}x{height}.jpg"}], "pagination": {}}"#).unwrap();
        let now = parse_iso8601("2026-10-21T21:00:00Z").unwrap();
        let items = stream_items(&page.data[0], Some("https://static-cdn.jtvnw.net/jtv_user_pictures/x.png"), DEFAULT_APP, now);
        assert_eq!(items.len(), 2);
        let ch = &items[0];
        assert_eq!(ch.kind, ItemKind::Channel);
        assert_eq!(ch.title, "SummoningSalt");
        assert_eq!(ch.channel_id.as_deref(), Some("summoningsalt"));
        assert_eq!(ch.logo.as_deref(), Some("https://static-cdn.jtvnw.net/jtv_user_pictures/x.png"));
        assert_eq!(ch.action.view.as_deref(), Some("twitch://stream/summoningsalt"));
        assert_eq!(ch.action.fallback_launch.as_deref(), Some(DEFAULT_APP));
        let live = &items[1];
        assert_eq!(live.kind, ItemKind::Schedule);
        assert_eq!(live.title, "16 star speedruns, then history video research");
        assert_eq!(live.subtitle.as_deref(), Some("Super Mario 64, 2.7k watching"));
        assert_eq!(live.start_ms, parse_iso8601("2026-10-21T19:00:00Z"));
        assert_eq!(live.end_ms, None);
        let preview = live.thumbnail.as_deref().unwrap();
        assert!(preview.starts_with("https://static-cdn.jtvnw.net/previews-ttv/live_user_summoningsalt-1280x720.jpg?r="));
        assert_eq!(ch.thumbnail, live.thumbnail);
    }

    #[test]
    fn viewer_counts_read_well() {
        assert_eq!(watching(0), "nobody watching yet");
        assert_eq!(watching(1), "1 watching");
        assert_eq!(watching(42), "42 watching");
        assert_eq!(watching(12_345), "12.3k watching");
    }

    #[test]
    fn the_sign_in_instruction_names_the_code() {
        let d = Device { device_code: "x".into(), user_code: "ABCDEFGH".into(), verification_uri: "https://www.twitch.tv/activate?public=true&device-code=ABCDEFGH".into(), expires_ms: 0 };
        assert_eq!(instruction(&d), "Sign in: on your phone open www.twitch.tv/activate and enter ABCDEFGH, then press Refresh now here.");
        assert_eq!(url_encode("urn:ietf:params:oauth:grant-type:device_code"), "urn%3Aietf%3Aparams%3Aoauth%3Agrant-type%3Adevice_code");
    }

    #[test]
    fn only_a_refused_renewal_asks_for_a_new_sign_in() {
        let token = |expires_ms| Token { access: "old".into(), refresh: "r".into(), expires_ms };
        assert!(matches!(after_renewal(token(10), Err(RenewError::Refused(400)), 0), Renewed::SignInAgain));
        assert!(matches!(after_renewal(token(10), Err(RenewError::Failed("HTTP 503".into())), 0), Renewed::Use(t) if t.access == "old"));
        assert!(matches!(after_renewal(token(10), Err(RenewError::Failed("timed out".into())), 20), Renewed::TryLater(e) if e == "Could not renew the Twitch sign-in. It tries again on the next refresh."));
    }
}
