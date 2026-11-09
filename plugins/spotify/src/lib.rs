//! Spotify: what you listened to lately, your playlists, saved albums and podcasts from your
//! Spotify account, and any Spotify links you paste, opened in the Spotify app. Spotify gives
//! third-party apps no sign-in by code, so the sign-in is a link opened on the phone and the
//! address it lands on pasted back; the plugin keeps the session going from there, until
//! Spotify ends it, which it does six months after every sign-in.

use lounge_plugin_sdk::{self as sdk, host, parse_iso8601, Action, HttpRequest, HttpResponse, Item, ItemKind, Kind, Manifest, RefreshContext, RowSpec, RowStyle, SettingKind, SettingSpec};
use serde::{Deserialize, Deserializer, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

pub struct Spotify;

const API: &str = "https://api.spotify.com/v1";
const ACCOUNTS: &str = "https://accounts.spotify.com";
const OEMBED: &str = "https://open.spotify.com/oembed";
const REDIRECT_URI: &str = "http://127.0.0.1:8888/callback";
const SCOPES: &str = "user-read-recently-played playlist-read-private playlist-read-collaborative user-library-read";
const APP_PACKAGE: &str = "com.spotify.tv.android";
const TOKEN_KEY: &str = "token";
const USED_CODE_KEY: &str = "used_code";
const VERIFIER_KEY: &str = "verifier";
const CONTEXTS_KEY: &str = "contexts";
const PICKS_KEY: &str = "picks";
const PAUSE_KEY: &str = "pause_until";
/// Most tiles in Recently played: one per playlist, album or artist listened to.
const RECENT_ITEMS: usize = 20;
/// Most lookups of names and covers in one refresh; the rest come with the next.
const LOOKUPS: usize = 15;
/// How long a name and cover serve before being asked for again; Spotify calls cover
/// addresses temporary.
const KNOWN_FOR_MS: i64 = 7 * 86_400_000;
/// How long a title Spotify refused is left alone before being asked about again.
const REFUSED_FOR_MS: i64 = 86_400_000;
/// How long one sign-in link stays good for, so the link a person opened still matches.
const VERIFIER_FOR_MS: i64 = 86_400_000;
const MINUTE_MS: i64 = 60_000;
const DAY_MS: i64 = 86_400_000;
const PAUSE: &str = "Spotify is asking for a pause between requests. It tries again on the next refresh.";
const QUOTA: &str = "Spotify's daily allowance for this app is used up. It comes back tomorrow.";
const SIGNED_OUT: &str = "Spotify no longer accepts this sign-in; it is renewed on the next refresh, or press Refresh now.";

impl sdk::Plugin for Spotify {
    fn manifest() -> Manifest {
        Manifest {
            id: "tv.lounge.spotify".into(),
            name: "Spotify".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            description: "Recently played, your playlists, saved albums and podcasts from your Spotify account, or any Spotify links you paste, opened in Spotify.".into(),
            publisher_name: "Lounge".into(),
            kinds: vec![Kind::Content],
            fetch_hosts: vec!["api.spotify.com".into(), "accounts.spotify.com".into(), "open.spotify.com".into(), "*.scdn.co".into(), "*.spotifycdn.com".into()],
            refresh_minutes: 15,
            rows: vec![
                RowSpec { id: "recent".into(), title: "Recently played on Spotify".into(), style: RowStyle::Landscape },
                RowSpec { id: "playlists".into(), title: "Your Spotify playlists".into(), style: RowStyle::Landscape },
                RowSpec { id: "albums".into(), title: "Saved albums on Spotify".into(), style: RowStyle::Landscape },
                RowSpec { id: "podcasts".into(), title: "Your podcasts on Spotify".into(), style: RowStyle::Landscape },
                RowSpec { id: "picks".into(), title: "Your picks on Spotify".into(), style: RowStyle::Landscape },
            ],
            settings: vec![
                SettingSpec {
                    key: "links".into(),
                    label: "Spotify links".into(),
                    kind: SettingKind::Text,
                    required: false,
                    help: Some("Playlists, albums, artists or podcasts to keep on the home, one share link per line. Needs no account. Share, Copy link in Spotify gives you one.".into()),
                    default: None,
                },
                SettingSpec {
                    key: "client_id".into(),
                    label: "Spotify Client ID".into(),
                    kind: SettingKind::Text,
                    required: false,
                    help: Some(format!("For your own listening, playlists, albums and podcasts. At developer.spotify.com/dashboard, signed in with a Premium account, create an app: tick Web API and give it the redirect URI {REDIRECT_URI}, exactly. Paste its Client ID here. If another account will listen, add it under User Management. The sign-in link then appears here and on the TV.")),
                    default: None,
                },
                SettingSpec {
                    key: "code".into(),
                    label: "Sign-in address".into(),
                    kind: SettingKind::Text,
                    required: false,
                    help: Some("After you agree on the sign-in page, Spotify sends the browser to a page that can't be reached. That is expected: copy the whole address of that page and paste it here.".into()),
                    default: None,
                },
                SettingSpec { key: "recent".into(), label: "Recently played row".into(), kind: SettingKind::Toggle, required: false, help: None, default: Some("true".into()) },
                SettingSpec { key: "playlists".into(), label: "Playlists row".into(), kind: SettingKind::Toggle, required: false, help: None, default: Some("true".into()) },
                SettingSpec { key: "albums".into(), label: "Saved albums row".into(), kind: SettingKind::Toggle, required: false, help: None, default: Some("true".into()) },
                SettingSpec { key: "podcasts".into(), label: "Podcasts row".into(), kind: SettingKind::Toggle, required: false, help: Some("The shows you follow.".into()), default: Some("true".into()) },
            ],
            homepage: Some("https://www.spotify.com".into()),
            self_signed_servers: false,
            for_apps: Vec::new(),
        }
    }

    fn refresh(ctx: &RefreshContext) -> Result<Vec<Item>, String> {
        let links = parse_links(&host::setting("links").unwrap_or_default());
        let account = Account::connect(ctx.now_ms)?;
        if account.is_none() && links.is_empty() {
            return Err("Nothing to show yet: paste some Spotify links, or a Client ID for your account, in the plugin's settings".into());
        }
        let mut items = Vec::new();
        let mut failures = Vec::new();

        if !links.is_empty() {
            let mut picks = Picks::load();
            items.extend(picks.items_for(&links, ctx.now_ms));
            picks.save();
        }

        if let Some(mut client) = account {
            let mut contexts = Contexts::load(ctx.now_ms);
            if toggle("playlists") || toggle("recent") {
                match client.get_json::<Page<Playlist>>("/me/playlists?limit=50") {
                    Ok(page) => {
                        let playlists = page.present();
                        contexts.learn_playlists(&playlists, ctx.now_ms);
                        if toggle("playlists") {
                            let count = playlists.len() as i32;
                            items.extend(playlists.into_iter().enumerate().filter_map(|(i, p)| playlist_item(p, count - i as i32)));
                        }
                    }
                    Err(e) => failures.push(e),
                }
            }
            if toggle("recent") {
                match client.get_json::<Page<Play>>("/me/player/recently-played?limit=50") {
                    Ok(page) => {
                        let listened = listened_to(page.present());
                        contexts.fill(&mut client, &listened, ctx.now_ms);
                        let count = listened.len() as i32;
                        items.extend(listened.into_iter().enumerate().filter_map(|(i, l)| contexts.item_for(&l, count - i as i32)));
                    }
                    Err(e) => failures.push(e),
                }
            }
            contexts.save();
            if toggle("albums") {
                match client.get_json::<Page<SavedAlbum>>("/me/albums?limit=50") {
                    Ok(page) => {
                        let albums = page.present();
                        let count = albums.len() as i32;
                        items.extend(albums.into_iter().enumerate().filter_map(|(i, a)| album_item(a, count - i as i32)));
                    }
                    Err(e) => failures.push(e),
                }
            }
            if toggle("podcasts") {
                match client.get_json::<Page<SavedShow>>("/me/shows?limit=50") {
                    Ok(page) => {
                        let shows = page.present();
                        let count = shows.len() as i32;
                        items.extend(shows.into_iter().enumerate().filter_map(|(i, s)| show_item(s, count - i as i32)));
                    }
                    Err(e) => failures.push(e),
                }
            }
            if let Some(left) = client.token.days_left(ctx.now_ms)
                && left <= 14
            {
                sdk::warn!("Spotify ends this sign-in in {left} days; it asks for a new one every six months");
            }
        }
        for e in &failures {
            sdk::warn!("{e}");
        }
        if items.is_empty()
            && let Some(e) = failures.into_iter().next()
        {
            return Err(e);
        }
        sdk::dedup_ids(&mut items);
        sdk::info!("{} tiles from Spotify", items.len());
        Ok(items)
    }
}

sdk::export_plugin!(Spotify);

fn toggle(key: &str) -> bool {
    host::setting(key).map(|v| v != "false").unwrap_or(true)
}

/// Reads a value that Spotify may send as `null` as its default instead.
fn or_default<'de, D: Deserializer<'de>, T: Default + Deserialize<'de>>(d: D) -> Result<T, D::Error> {
    Option::<T>::deserialize(d).map(Option::unwrap_or_default)
}

// ---------------------------------------------------------------------------
// Signing in: a link on the phone, the address pasted back, then refresh tokens
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize, Clone)]
struct Token {
    access: String,
    refresh: String,
    expires_ms: i64,
    /// The app the token belongs to; a new Client ID signs in afresh.
    client_id: String,
    /// When the person signed in; Spotify ends the session six months on.
    #[serde(default)]
    signed_in_ms: i64,
}

impl Token {
    fn days_left(&self, now_ms: i64) -> Option<i64> {
        (self.signed_in_ms > 0).then(|| (self.signed_in_ms + 182 * DAY_MS - now_ms) / DAY_MS)
    }
}

#[derive(Serialize, Deserialize)]
struct Verifier {
    value: String,
    client_id: String,
    made_ms: i64,
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

/// What the person pasted into the sign-in setting.
#[derive(Debug, PartialEq, Eq)]
pub enum Pasted {
    Nothing,
    Code(String),
    Declined,
}

/// The code in a pasted address, a bare code, or that the sign-in was declined.
pub fn pasted_code(value: &str) -> Pasted {
    let value = value.trim();
    if value.is_empty() {
        return Pasted::Nothing;
    }
    let param = |key: &str| {
        value.find(key).map(|i| &value[i + key.len()..]).map(|rest| rest.split(['&', '#', ' ', '\n']).next().unwrap_or("").to_string()).filter(|v| !v.is_empty())
    };
    if let Some(code) = param("code=") {
        return Pasted::Code(code);
    }
    if param("error=").is_some() {
        return Pasted::Declined;
    }
    if value.contains("://") || value.contains('/') || value.contains('?') {
        return Pasted::Nothing;
    }
    Pasted::Code(value.to_string())
}

/// The PKCE verifier behind the current sign-in link, kept for a day so a link already opened
/// still matches. The sandbox has no source of chance, so it is hashed from the moment, the
/// app and whatever came before; the code it protects only ever travels through the phone.
fn verifier(client_id: &str, now_ms: i64) -> String {
    if let Some(v) = host::kv_get(VERIFIER_KEY).and_then(|b| serde_json::from_slice::<Verifier>(&b).ok())
        && v.client_id == client_id
        && now_ms - v.made_ms < VERIFIER_FOR_MS
    {
        return v.value;
    }
    let previous = host::kv_get_string(VERIFIER_KEY).unwrap_or_default();
    let mut material = Sha256::digest(format!("{now_ms}:{client_id}:{previous}").as_bytes()).to_vec();
    material.extend(Sha256::digest(&material));
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-._~";
    let value: String = material.iter().map(|b| ALPHABET[*b as usize % ALPHABET.len()] as char).collect();
    host::kv_set(VERIFIER_KEY, serde_json::to_string(&Verifier { value: value.clone(), client_id: client_id.to_string(), made_ms: now_ms }).unwrap_or_default().as_bytes());
    value
}

/// The S256 challenge for a verifier: its SHA-256, in base64url without padding.
pub fn challenge(verifier: &str) -> String {
    base64url(&Sha256::digest(verifier.as_bytes()))
}

/// The link the person opens on the phone to let the app in.
pub fn authorize_url(client_id: &str, verifier: &str) -> String {
    format!(
        "{ACCOUNTS}/authorize?response_type=code&client_id={}&scope={}&redirect_uri={}&code_challenge_method=S256&code_challenge={}&show_dialog=true",
        url_encode(client_id),
        url_encode(SCOPES),
        url_encode(REDIRECT_URI),
        challenge(verifier)
    )
}

/// What to do to sign in. The link goes without its scheme, since the host cuts an address
/// written with one down to its path.
fn instruction(client_id: &str, verifier: &str, again: bool) -> String {
    let why = if again { "Spotify ends every sign-in after six months, so it is time to sign in again. " } else { "" };
    let link = authorize_url(client_id, verifier);
    format!(
        "{why}Sign in on your phone: in Lounge's companion app open Plugins, then Spotify, and tap the link there, or open {}. After you agree, copy the whole address of the page that can't be reached into the Sign-in address setting.",
        link.strip_prefix("https://").unwrap_or(&link)
    )
}

struct Client {
    client_id: String,
    token: Token,
    now_ms: i64,
    /// Set once Spotify has asked for a pause; nothing more is asked this refresh.
    paused: bool,
}

/// The sign-in the settings ask for: none without a Client ID, else a client, with the
/// sentence to show meanwhile as the error.
struct Account;

impl Account {
    fn connect(now_ms: i64) -> Result<Option<Client>, String> {
        let Some(client_id) = host::setting("client_id").map(|v| v.trim().to_string()).filter(|v| !v.is_empty()) else {
            return Ok(None);
        };
        if host::kv_get_string(PAUSE_KEY).and_then(|v| v.parse::<i64>().ok()).is_some_and(|until| until > now_ms) {
            return Err(PAUSE.into());
        }
        let pasted = pasted_code(&host::setting("code").unwrap_or_default());
        let used = host::kv_get_string(USED_CODE_KEY);
        let fresh_code = match &pasted {
            Pasted::Code(code) if used.as_deref() != Some(code.as_str()) => Some(code.clone()),
            _ => None,
        };
        let saved: Option<Token> = host::kv_get(TOKEN_KEY).and_then(|v| serde_json::from_slice(&v).ok()).filter(|t: &Token| t.client_id == client_id);
        let token = if let Some(code) = fresh_code {
            exchange_code(&client_id, &code, now_ms)?
        } else {
            match saved {
                Some(t) if t.expires_ms - now_ms > 2 * MINUTE_MS => t,
                Some(t) => match renew(&client_id, &t, now_ms) {
                    Ok(t) => t,
                    Err(Refusal::Refused(why)) => {
                        sdk::warn!("Spotify refused to renew the sign-in ({why})");
                        host::kv_delete(TOKEN_KEY);
                        return Err(instruction(&client_id, &verifier(&client_id, now_ms), true));
                    }
                    Err(Refusal::Failed(e)) if t.expires_ms > now_ms => {
                        sdk::warn!("could not renew the sign-in ({e}); using the current one");
                        t
                    }
                    Err(Refusal::Failed(e)) => {
                        sdk::warn!("could not renew the sign-in ({e})");
                        return Err(format!("Could not renew the Spotify sign-in ({e}). It tries again on the next refresh."));
                    }
                },
                None if pasted == Pasted::Declined => {
                    return Err(format!("The sign-in was declined on the phone. {}", instruction(&client_id, &verifier(&client_id, now_ms), false)));
                }
                None => return Err(instruction(&client_id, &verifier(&client_id, now_ms), false)),
            }
        };
        Ok(Some(Client { client_id, token, now_ms, paused: false }))
    }
}

/// Why a renewal did not give a token: Spotify refused the refresh token, or nothing usable
/// came back.
enum Refusal {
    Refused(String),
    Failed(String),
}

fn token_post(fields: &[(&str, &str)]) -> Result<(u16, String), String> {
    let body = fields.iter().map(|(k, v)| format!("{}={}", url_encode(k), url_encode(v))).collect::<Vec<_>>().join("&");
    let req = HttpRequest {
        method: "POST".into(),
        url: format!("{ACCOUNTS}/api/token"),
        headers: [("Content-Type".to_string(), "application/x-www-form-urlencoded".to_string()), ("Accept".to_string(), "application/json".to_string())].into_iter().collect(),
        body: Some(body),
        timeout_ms: 0,
    };
    let r = sdk::fetch_online("Spotify", &req)?;
    Ok((r.status, r.body))
}

fn save_token(t: TokenAnswer, keep_refresh: &str, client_id: &str, signed_in_ms: i64, now_ms: i64) -> Token {
    let lifetime_ms = if t.expires_in > 0 { t.expires_in * 1000 } else { 3_600_000 };
    let token = Token {
        access: t.access_token,
        refresh: t.refresh_token.filter(|r| !r.is_empty()).unwrap_or_else(|| keep_refresh.to_string()),
        expires_ms: now_ms + lifetime_ms,
        client_id: client_id.to_string(),
        signed_in_ms,
    };
    host::kv_set(TOKEN_KEY, serde_json::to_string(&token).unwrap_or_default().as_bytes());
    token
}

/// Turns the code the person pasted into a session, or says what went wrong. A code Spotify
/// took, or refused as spent, is remembered so it is not tried again; one refused for another
/// reason is kept for after the fix.
fn exchange_code(client_id: &str, code: &str, now_ms: i64) -> Result<Token, String> {
    let verifier = verifier(client_id, now_ms);
    let (status, body) = token_post(&[("client_id", client_id), ("grant_type", "authorization_code"), ("code", code), ("redirect_uri", REDIRECT_URI), ("code_verifier", &verifier)])?;
    if (200..300).contains(&status) {
        let t: TokenAnswer = serde_json::from_str(&body).map_err(|e| {
            sdk::warn!("unexpected token answer: {e}");
            sdk::unreadable("Spotify", ACCOUNTS)
        })?;
        host::kv_set(USED_CODE_KEY, code.as_bytes());
        host::kv_delete(VERIFIER_KEY);
        return Ok(save_token(t, "", client_id, now_ms, now_ms));
    }
    let e: OAuthError = serde_json::from_str(&body).unwrap_or_default();
    sdk::warn!("the sign-in code was refused: HTTP {status} {} {}", e.error, e.error_description);
    let description = e.error_description.to_lowercase();
    Err(match e.error.as_str() {
        "invalid_client" => "Spotify does not know this Client ID. Check it against the app in the developer dashboard.".into(),
        "invalid_grant" if description.contains("redirect") => format!("The app's redirect URI must be {REDIRECT_URI}, exactly. Set it in the app's dashboard, then sign in again."),
        "invalid_grant" if description.contains("verifier") || description.contains("challenge") => {
            host::kv_set(USED_CODE_KEY, code.as_bytes());
            host::kv_delete(VERIFIER_KEY);
            format!("That sign-in came from an earlier link. {}", instruction(client_id, &self::verifier(client_id, now_ms), false))
        }
        "invalid_grant" => {
            host::kv_set(USED_CODE_KEY, code.as_bytes());
            format!("That sign-in address did not work; a code is good for a few minutes and once. {}", instruction(client_id, &verifier, false))
        }
        _ => sdk::online_error("Spotify", status),
    })
}

fn renew(client_id: &str, old: &Token, now_ms: i64) -> Result<Token, Refusal> {
    if old.refresh.is_empty() {
        return Err(Refusal::Refused("no refresh token".into()));
    }
    let (status, body) = token_post(&[("client_id", client_id), ("grant_type", "refresh_token"), ("refresh_token", &old.refresh)]).map_err(Refusal::Failed)?;
    if (200..300).contains(&status) {
        let t: TokenAnswer = serde_json::from_str(&body).map_err(|e| Refusal::Failed(e.to_string()))?;
        return Ok(save_token(t, &old.refresh, client_id, old.signed_in_ms, now_ms));
    }
    let e: OAuthError = serde_json::from_str(&body).unwrap_or_default();
    // Only a refresh token Spotify no longer honours means signing in again; a Client ID it
    // does not know is a setting to fix while the session waits.
    match e.error.as_str() {
        "invalid_grant" => Err(Refusal::Refused(format!("HTTP {status} {}", e.error_description))),
        "invalid_client" => Err(Refusal::Failed("Spotify does not know this Client ID; check it in the plugin's settings".into())),
        _ => Err(Refusal::Failed(format!("HTTP {status} {} {}", e.error, e.error_description))),
    }
}

impl Client {
    /// One answer from the API. A session Spotify no longer accepts is renewed once; a request
    /// for a pause stops everything else this refresh.
    fn get(&mut self, path: &str) -> Result<HttpResponse, String> {
        if self.paused {
            return Err(PAUSE.into());
        }
        for attempt in 0..2 {
            let r = sdk::fetch_online("Spotify", &HttpRequest::get(format!("{API}{path}")).header("Authorization", &format!("Bearer {}", self.token.access)).header("Accept", "application/json"))?;
            match r.status {
                401 if attempt == 0 => match renew(&self.client_id, &self.token, self.now_ms) {
                    Ok(t) => self.token = t,
                    Err(Refusal::Refused(why)) => {
                        sdk::warn!("Spotify refused to renew the sign-in ({why})");
                        host::kv_delete(TOKEN_KEY);
                        return Err(SIGNED_OUT.into());
                    }
                    Err(Refusal::Failed(e)) => return Err(format!("Could not renew the Spotify sign-in ({e}). It tries again on the next refresh.")),
                },
                401 => return Err(SIGNED_OUT.into()),
                429 => {
                    self.paused = true;
                    let wait_s = r.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case("retry-after")).and_then(|(_, v)| v.trim().parse::<i64>().ok()).unwrap_or(60);
                    host::kv_set(PAUSE_KEY, (self.now_ms + wait_s.clamp(1, 86_400) * 1000).to_string().as_bytes());
                    return Err(if r.body.to_uppercase().contains("QUOTA") { QUOTA.into() } else { PAUSE.into() });
                }
                _ => return Ok(r),
            }
        }
        Err(SIGNED_OUT.into())
    }

    fn get_json<T: serde::de::DeserializeOwned>(&mut self, path: &str) -> Result<T, String> {
        let r = self.get(path)?;
        let what = path.split('?').next().unwrap_or(path);
        match r.status {
            200..=299 => serde_json::from_str(&r.body).map_err(|e| {
                sdk::warn!("unexpected answer for {what}: {e}");
                sdk::unreadable("Spotify", API)
            }),
            403 => Err(refused_why(&r.body)),
            s => {
                sdk::warn!("HTTP {s} for {what}");
                Err(sdk::online_error("Spotify", s))
            }
        }
    }

    /// As [`Client::get_json`], but nothing for a title Spotify does not have or will not
    /// give this app, which is how it answers for its own playlists in development mode.
    fn get_optional<T: serde::de::DeserializeOwned>(&mut self, path: &str) -> Result<Option<T>, String> {
        let r = self.get(path)?;
        match r.status {
            200..=299 => serde_json::from_str(&r.body).map(Some).map_err(|e| e.to_string()),
            403 | 404 => Ok(None),
            s => Err(format!("HTTP {s}")),
        }
    }
}

/// Spotify's 403 says why in plain text, or in JSON with a message.
fn refused_why(body: &str) -> String {
    #[derive(Deserialize, Default)]
    struct ApiError {
        #[serde(default)]
        error: Inner,
    }
    #[derive(Deserialize, Default)]
    struct Inner {
        #[serde(default)]
        message: String,
    }
    let said = serde_json::from_str::<ApiError>(body).map(|e| e.error.message).ok().filter(|m| !m.is_empty()).unwrap_or_else(|| body.trim().chars().take(200).collect());
    sdk::warn!("HTTP 403: {said}");
    let lower = said.to_lowercase();
    if lower.contains("registered") {
        "Spotify's app is in development mode and this account is not on its list. Add the account under User Management in the app's dashboard.".into()
    } else if lower.contains("premium") {
        "Spotify needs the account that made the app to have Premium.".into()
    } else if said.is_empty() {
        "Spotify refused the request. Check the app in the developer dashboard.".into()
    } else {
        format!("Spotify refused: {said}")
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

/// base64url without padding, as PKCE wants its challenge.
pub fn base64url(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = (chunk[0] as u32) << 16 | (*chunk.get(1).unwrap_or(&0) as u32) << 8 | *chunk.get(2).unwrap_or(&0) as u32;
        out.push(TABLE[(n >> 18) as usize & 63] as char);
        out.push(TABLE[(n >> 12) as usize & 63] as char);
        if chunk.len() > 1 {
            out.push(TABLE[(n >> 6) as usize & 63] as char);
        }
        if chunk.len() > 2 {
            out.push(TABLE[n as usize & 63] as char);
        }
    }
    out
}

// ---------------------------------------------------------------------------
// What Spotify sends
// ---------------------------------------------------------------------------

/// A page of a list. Spotify puts `null` in place of what it will not give this app, such as
/// its own playlists in development mode, so each entry may be missing.
#[derive(Deserialize, Default)]
#[serde(bound(deserialize = "T: Deserialize<'de>"))]
pub struct Page<T> {
    #[serde(default, deserialize_with = "or_default")]
    pub items: Vec<Option<T>>,
}

impl<T> Page<T> {
    pub fn present(self) -> Vec<T> {
        self.items.into_iter().flatten().collect()
    }
}

#[derive(Deserialize, Default, Clone, Debug)]
pub struct ImageRef {
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub width: Option<u32>,
}

/// The largest picture, which for Spotify is a 640-pixel square; of pictures with no stated
/// size the first, which Spotify lists largest first.
pub fn largest(images: &[ImageRef]) -> Option<String> {
    images.iter().rev().filter(|i| i.url.starts_with("https://")).max_by_key(|i| i.width.unwrap_or(0)).map(|i| i.url.clone())
}

#[derive(Deserialize, Default, Clone, Debug)]
pub struct Named {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub uri: String,
}

#[derive(Deserialize, Default, Clone, Debug)]
pub struct Album {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub uri: String,
    #[serde(default, deserialize_with = "or_default")]
    pub images: Vec<ImageRef>,
    #[serde(default, deserialize_with = "or_default")]
    pub artists: Vec<Named>,
}

impl Album {
    fn artist_names(&self) -> String {
        self.artists.iter().map(|a| a.name.as_str()).filter(|n| !n.is_empty()).collect::<Vec<_>>().join(", ")
    }
}

impl PartialEq for Album {
    fn eq(&self, other: &Album) -> bool {
        self.uri == other.uri
    }
}

impl Eq for Album {}

#[derive(Deserialize, Default, Clone, Debug)]
pub struct Track {
    #[serde(default)]
    pub name: String,
    #[serde(default, deserialize_with = "or_default")]
    pub album: Album,
}

#[derive(Deserialize, Default, Clone, Debug)]
pub struct Context {
    #[serde(rename = "type", default)]
    pub kind: String,
    #[serde(default)]
    pub uri: String,
}

#[derive(Deserialize, Default, Clone, Debug)]
pub struct Play {
    #[serde(default, deserialize_with = "or_default")]
    pub track: Track,
    #[serde(default)]
    pub played_at: Option<String>,
    #[serde(default)]
    pub context: Option<Context>,
}

#[derive(Deserialize, Default, Clone, Debug)]
pub struct Owner {
    #[serde(default)]
    pub display_name: Option<String>,
}

#[derive(Deserialize, Default, Clone, Debug)]
pub struct Count {
    #[serde(default)]
    pub total: Option<u32>,
}

#[derive(Deserialize, Default, Clone, Debug)]
pub struct Playlist {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub uri: String,
    #[serde(default, deserialize_with = "or_default")]
    pub images: Vec<ImageRef>,
    #[serde(default, deserialize_with = "or_default")]
    pub owner: Owner,
    /// The songs, under the name Spotify gave the field before 2026.
    #[serde(default, deserialize_with = "or_default")]
    pub tracks: Count,
    /// The songs, under the name Spotify gives the field since 2026.
    #[serde(default, deserialize_with = "or_default")]
    pub items: Count,
}

impl Playlist {
    fn song_count(&self) -> Option<u32> {
        self.items.total.or(self.tracks.total)
    }

    fn subtitle(&self) -> String {
        playlist_subtitle(self.owner.display_name.as_deref(), self.song_count())
    }
}

#[derive(Deserialize, Default, Clone, Debug)]
pub struct SavedAlbum {
    #[serde(default)]
    pub added_at: Option<String>,
    #[serde(default, deserialize_with = "or_default")]
    pub album: Album,
}

#[derive(Deserialize, Default, Clone, Debug)]
pub struct Show {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub uri: String,
    #[serde(default)]
    pub publisher: Option<String>,
    #[serde(default)]
    pub total_episodes: Option<u32>,
    #[serde(default, deserialize_with = "or_default")]
    pub images: Vec<ImageRef>,
}

#[derive(Deserialize, Default, Clone, Debug)]
pub struct SavedShow {
    #[serde(default, deserialize_with = "or_default")]
    pub show: Show,
}

#[derive(Deserialize, Default, Clone)]
struct Artist {
    #[serde(default)]
    name: String,
    #[serde(default, deserialize_with = "or_default")]
    images: Vec<ImageRef>,
}

#[derive(Deserialize, Default)]
struct Oembed {
    #[serde(default)]
    title: String,
    #[serde(default)]
    thumbnail_url: Option<String>,
}

/// The public card for a Spotify link, which needs no account: a title and a cover.
fn oembed(uri: &str) -> Result<Option<Oembed>, String> {
    let r = sdk::fetch_online("Spotify", &HttpRequest::get(format!("{OEMBED}?url={}", url_encode(uri))).header("Accept", "application/json"))?;
    match r.status {
        200..=299 => serde_json::from_str::<Oembed>(&r.body).map(Some).map_err(|e| e.to_string()),
        400 | 404 => Ok(None),
        s => Err(format!("HTTP {s}")),
    }
}

// ---------------------------------------------------------------------------
// Recently played: the playlists, albums and artists behind the tracks
// ---------------------------------------------------------------------------

/// Something listened to lately: the context the tracks were played from, or the album of a
/// track played on its own.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listened {
    /// A Spotify URI, which is also what opens it.
    pub uri: String,
    pub last_played_ms: i64,
    /// What is known without asking: an album's name, cover and artists.
    pub album: Option<Album>,
}

/// Groups plays by what they were played from, newest first.
pub fn listened_to(plays: Vec<Play>) -> Vec<Listened> {
    let mut by_uri: BTreeMap<String, Listened> = BTreeMap::new();
    for p in plays {
        let played_ms = p.played_at.as_deref().and_then(parse_iso8601).unwrap_or(0);
        let (uri, album) = match p.context.filter(|c| !c.uri.is_empty()) {
            Some(c) if c.kind == "album" => (c.uri, Some(p.track.album)),
            Some(c) => (c.uri, None),
            None if !p.track.album.uri.is_empty() => (p.track.album.uri.clone(), Some(p.track.album)),
            None => continue,
        };
        let entry = by_uri.entry(uri.clone()).or_insert(Listened { uri, last_played_ms: 0, album: None });
        entry.last_played_ms = entry.last_played_ms.max(played_ms);
        if entry.album.is_none() {
            entry.album = album;
        }
    }
    let mut listened: Vec<Listened> = by_uri.into_values().collect();
    listened.sort_by_key(|l| std::cmp::Reverse(l.last_played_ms));
    listened.truncate(RECENT_ITEMS);
    listened
}

/// What a context is called and looks like, as found once, or that nothing would say.
#[derive(Serialize, Deserialize, Default, Clone)]
struct KnownContext {
    name: String,
    subtitle: Option<String>,
    image: Option<String>,
    fetched_ms: i64,
    #[serde(default)]
    refused: bool,
}

/// Names and covers for the playlists and artists listened to, from the person's own
/// playlists, Spotify's API or its public cards, kept between refreshes so each is asked
/// about once in a while.
#[derive(Default)]
struct Contexts {
    known: BTreeMap<String, KnownContext>,
    now_ms: i64,
}

impl Contexts {
    fn load(now_ms: i64) -> Contexts {
        Contexts { known: host::kv_get(CONTEXTS_KEY).and_then(|v| serde_json::from_slice(&v).ok()).unwrap_or_default(), now_ms }
    }

    fn save(&self) {
        if let Ok(json) = serde_json::to_vec(&self.known) {
            host::kv_set(CONTEXTS_KEY, &json);
        }
    }

    /// The person's own playlists name themselves, at no cost.
    fn learn_playlists(&mut self, playlists: &[Playlist], now_ms: i64) {
        for p in playlists.iter().filter(|p| !p.uri.is_empty() && !p.name.is_empty()) {
            self.known.insert(p.uri.clone(), KnownContext { name: p.name.clone(), subtitle: Some(p.subtitle()), image: largest(&p.images), fetched_ms: now_ms, refused: false });
        }
    }

    fn is_due(&self, uri: &str) -> bool {
        match self.known.get(uri) {
            None => true,
            Some(k) if k.refused => self.now_ms - k.fetched_ms > REFUSED_FOR_MS,
            Some(k) => self.now_ms - k.fetched_ms > KNOWN_FOR_MS,
        }
    }

    /// Asks about the contexts not known: Spotify's API first, its public card when the API
    /// will not say, a few a refresh.
    fn fill(&mut self, client: &mut Client, listened: &[Listened], now_ms: i64) {
        self.known.retain(|uri, _| listened.iter().any(|l| &l.uri == uri));
        let mut lookups = 0;
        for l in listened.iter().filter(|l| l.album.is_none()) {
            if lookups >= LOOKUPS || client.paused {
                break;
            }
            if !self.is_due(&l.uri) {
                continue;
            }
            let found = if let Some(id) = l.uri.strip_prefix("spotify:playlist:") {
                lookups += 1;
                match client.get_optional::<Playlist>(&format!("/playlists/{}?fields=name,uri,images,owner(display_name),items(total),tracks(total)", url_encode(id))) {
                    Ok(Some(p)) => Some(KnownContext { name: p.name.clone(), subtitle: Some(p.subtitle()), image: largest(&p.images), fetched_ms: now_ms, refused: false }),
                    Ok(None) => None,
                    Err(e) => {
                        sdk::warn!("playlist {id}: {e}");
                        continue;
                    }
                }
            } else if let Some(id) = l.uri.strip_prefix("spotify:artist:") {
                lookups += 1;
                match client.get_optional::<Artist>(&format!("/artists/{}", url_encode(id))) {
                    Ok(Some(a)) => Some(KnownContext { name: a.name, subtitle: Some("Artist".into()), image: largest(&a.images), fetched_ms: now_ms, refused: false }),
                    Ok(None) => None,
                    Err(e) => {
                        sdk::warn!("artist {id}: {e}");
                        continue;
                    }
                }
            } else if l.uri.ends_with(":collection") {
                lookups += 1;
                let count = client.get_json::<Count>("/me/tracks?limit=1").ok().and_then(|c| c.total);
                Some(KnownContext { name: "Liked Songs".into(), subtitle: count.map(|n| format!("{n} songs")), image: None, fetched_ms: now_ms, refused: false })
            } else {
                continue;
            };
            // What the API would not give, the public card for the link usually will.
            let found = match found {
                Some(k) => k,
                None => {
                    lookups += 1;
                    match oembed(&l.uri) {
                        Ok(Some(card)) if !card.title.is_empty() => KnownContext { name: card.title, subtitle: Some(kind_word(&l.uri).into()), image: card.thumbnail_url, fetched_ms: now_ms, refused: false },
                        Ok(_) => KnownContext { fetched_ms: now_ms, refused: true, ..Default::default() },
                        Err(e) => {
                            sdk::warn!("card for {}: {e}", l.uri);
                            continue;
                        }
                    }
                }
            };
            self.known.insert(l.uri.clone(), found);
        }
    }

    /// The tile for something listened to, once its name is known.
    fn item_for(&self, l: &Listened, weight: i32) -> Option<Item> {
        let (name, subtitle, image) = match (&l.album, self.known.get(&l.uri)) {
            (Some(album), _) => (album.name.clone(), Some(album.artist_names()).filter(|a| !a.is_empty()), largest(&album.images)),
            (None, Some(k)) if !k.refused => (k.name.clone(), k.subtitle.clone(), k.image.clone()),
            _ => return None,
        };
        if name.is_empty() {
            return None;
        }
        Some(Item {
            id: format!("recent:{}", l.uri),
            kind: ItemKind::Program,
            title: name.clone(),
            subtitle,
            last_used_ms: (l.last_played_ms > 0).then_some(l.last_played_ms),
            thumbnail: image.clone(),
            logo: image,
            row: Some("recent".into()),
            weight,
            action: open(&l.uri, &name),
            ..Default::default()
        })
    }
}

fn playlist_subtitle(owner: Option<&str>, songs: Option<u32>) -> String {
    let count = songs.map(|n| if n == 1 { "1 song".to_string() } else { format!("{n} songs") });
    match (owner.filter(|o| !o.is_empty()), count) {
        (Some(owner), Some(count)) => format!("{owner}, {count}"),
        (Some(owner), None) => owner.to_string(),
        (None, Some(count)) => count,
        (None, None) => "Playlist".to_string(),
    }
}

// ---------------------------------------------------------------------------
// Tiles
// ---------------------------------------------------------------------------

fn playlist_item(p: Playlist, weight: i32) -> Option<Item> {
    if p.uri.is_empty() || p.name.is_empty() {
        return None;
    }
    let image = largest(&p.images);
    Some(Item {
        id: format!("playlist:{}", p.id),
        kind: ItemKind::Program,
        title: p.name.clone(),
        subtitle: Some(p.subtitle()),
        thumbnail: image.clone(),
        logo: image,
        row: Some("playlists".into()),
        weight,
        action: open(&p.uri, &p.name),
        ..Default::default()
    })
}

fn album_item(s: SavedAlbum, weight: i32) -> Option<Item> {
    let a = s.album;
    if a.uri.is_empty() || a.name.is_empty() {
        return None;
    }
    let image = largest(&a.images);
    Some(Item {
        id: format!("album:{}", a.id),
        kind: ItemKind::Program,
        title: a.name.clone(),
        subtitle: Some(a.artist_names()).filter(|n| !n.is_empty()),
        added_ms: s.added_at.as_deref().and_then(parse_iso8601),
        thumbnail: image.clone(),
        logo: image,
        row: Some("albums".into()),
        weight,
        action: open(&a.uri, &a.name),
        ..Default::default()
    })
}

fn show_item(s: SavedShow, weight: i32) -> Option<Item> {
    let show = s.show;
    if show.uri.is_empty() || show.name.is_empty() {
        return None;
    }
    let image = largest(&show.images);
    let subtitle = show.publisher.clone().filter(|p| !p.is_empty()).or_else(|| show.total_episodes.map(|n| if n == 1 { "1 episode".to_string() } else { format!("{n} episodes") }));
    Some(Item {
        id: format!("show:{}", show.id),
        kind: ItemKind::Program,
        title: show.name.clone(),
        subtitle,
        thumbnail: image.clone(),
        logo: image,
        row: Some("podcasts".into()),
        weight,
        action: open(&show.uri, &show.name),
        ..Default::default()
    })
}

/// Opens a Spotify URI in whichever Spotify is on the TV, or starts the TV app.
fn open(uri: &str, title: &str) -> Action {
    Action { view: Some(uri.to_string()), fallback_launch: Some(APP_PACKAGE.into()), title: Some(title.to_string()), ..Default::default() }
}

/// What kind of thing a Spotify URI names, as a tile's second line.
fn kind_word(uri: &str) -> &'static str {
    match uri.split(':').nth(1).unwrap_or("") {
        "playlist" => "Playlist",
        "album" => "Album",
        "artist" => "Artist",
        "show" => "Podcast",
        "episode" => "Episode",
        "track" => "Song",
        _ => "Spotify",
    }
}

// ---------------------------------------------------------------------------
// Pasted links: the public card for each, no account needed
// ---------------------------------------------------------------------------

/// Spotify URIs from pasted share links or URIs, one or more a line, in the order given.
pub fn parse_links(raw: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for piece in raw.split(['\n', ',', ';', ' ']).map(str::trim).filter(|s| !s.is_empty()) {
        let uri = if let Some(rest) = piece.strip_prefix("spotify:") {
            let mut parts = rest.split(':');
            match (parts.next(), parts.next()) {
                (Some(kind), Some(id)) if !id.is_empty() => Some(format!("spotify:{kind}:{}", id_token(id))),
                _ => None,
            }
        } else if let Some(i) = piece.find("open.spotify.com/") {
            let path = &piece[i + "open.spotify.com/".len()..];
            let mut segments = path.split('/').filter(|s| !s.is_empty());
            let mut kind = segments.next().unwrap_or("");
            if kind.starts_with("intl-") {
                kind = segments.next().unwrap_or("");
            }
            let id = id_token(segments.next().unwrap_or(""));
            (!id.is_empty() && !kind.is_empty()).then(|| format!("spotify:{kind}:{id}"))
        } else {
            None
        };
        if let Some(uri) = uri.filter(|u| matches!(u.split(':').nth(1), Some("playlist" | "album" | "artist" | "show" | "episode" | "track")))
            && !out.contains(&uri)
        {
            out.push(uri);
        }
    }
    out
}

fn id_token(s: &str) -> String {
    s.chars().take_while(|c| c.is_ascii_alphanumeric()).collect()
}

#[derive(Serialize, Deserialize, Default, Clone)]
struct KnownPick {
    title: String,
    image: Option<String>,
    fetched_ms: i64,
    #[serde(default)]
    refused: bool,
}

/// The cards for the links the person pasted, kept between refreshes.
#[derive(Default)]
struct Picks {
    known: BTreeMap<String, KnownPick>,
}

impl Picks {
    fn load() -> Picks {
        Picks { known: host::kv_get(PICKS_KEY).and_then(|v| serde_json::from_slice(&v).ok()).unwrap_or_default() }
    }

    fn save(&self) {
        if let Ok(json) = serde_json::to_vec(&self.known) {
            host::kv_set(PICKS_KEY, &json);
        }
    }

    fn items_for(&mut self, links: &[String], now_ms: i64) -> Vec<Item> {
        self.known.retain(|uri, _| links.contains(uri));
        let mut lookups = 0;
        let count = links.len() as i32;
        let mut items = Vec::new();
        for (i, uri) in links.iter().enumerate() {
            let due = match self.known.get(uri) {
                None => true,
                Some(k) if k.refused => now_ms - k.fetched_ms > REFUSED_FOR_MS,
                Some(k) => now_ms - k.fetched_ms > KNOWN_FOR_MS,
            };
            if due && lookups < LOOKUPS {
                lookups += 1;
                match oembed(uri) {
                    Ok(Some(card)) if !card.title.is_empty() => {
                        self.known.insert(uri.clone(), KnownPick { title: card.title, image: card.thumbnail_url, fetched_ms: now_ms, refused: false });
                    }
                    Ok(_) => {
                        sdk::warn!("Spotify has nothing for {uri}; check the link in the plugin's settings");
                        self.known.insert(uri.clone(), KnownPick { fetched_ms: now_ms, refused: true, ..Default::default() });
                    }
                    Err(e) => sdk::warn!("card for {uri}: {e}"),
                }
            }
            let Some(k) = self.known.get(uri).filter(|k| !k.refused && !k.title.is_empty()) else { continue };
            items.push(Item {
                id: format!("pick:{uri}"),
                kind: ItemKind::Program,
                title: k.title.clone(),
                subtitle: Some(kind_word(uri).into()),
                thumbnail: k.image.clone(),
                logo: k.image.clone(),
                row: Some("picks".into()),
                weight: count - i as i32,
                action: open(uri, &k.title),
                ..Default::default()
            });
        }
        items
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plays_are_grouped_by_what_they_came_from_newest_first() {
        let page: Page<Play> = serde_json::from_str(
            r#"{"items":[
                {"played_at":"2026-11-09T08:00:00Z","context":{"type":"playlist","uri":"spotify:playlist:p1"},"track":{"name":"A","album":{"id":"a1","name":"Album One","uri":"spotify:album:a1","images":[{"url":"https://i.scdn.co/a1-300","width":300},{"url":"https://i.scdn.co/a1","width":640}],"artists":[{"name":"Band"}]}}},
                {"played_at":"2026-11-09T07:00:00Z","context":{"type":"album","uri":"spotify:album:a1"},"track":{"name":"B","album":{"id":"a1","name":"Album One","uri":"spotify:album:a1","images":[{"url":"https://i.scdn.co/a1","width":640}],"artists":[{"name":"Band"},{"name":"Guest"}]}}},
                {"played_at":"2026-11-09T09:00:00Z","context":{"type":"playlist","uri":"spotify:playlist:p1"},"track":{"name":"C","album":{"uri":"spotify:album:a2","name":"Two"}}},
                {"played_at":"2026-11-08T09:00:00Z","context":null,"track":{"name":"D","album":{"id":"a3","name":"Alone","uri":"spotify:album:a3","artists":null,"images":null}}},
                {"played_at":"2026-11-08T10:00:00Z","context":{"type":"artist","uri":"spotify:artist:x1"},"track":{"name":"E","album":{"uri":"spotify:album:a4"}}},
                {"played_at":"2026-11-08T11:00:00Z","context":{"type":"collection","uri":"spotify:user:me:collection"},"track":null},
                null]}"#,
        )
        .unwrap();
        let listened = listened_to(page.present());
        let uris: Vec<&str> = listened.iter().map(|l| l.uri.as_str()).collect();
        assert_eq!(uris, ["spotify:playlist:p1", "spotify:album:a1", "spotify:user:me:collection", "spotify:artist:x1", "spotify:album:a3"]);
        assert_eq!(listened[0].last_played_ms, parse_iso8601("2026-11-09T09:00:00Z").unwrap());
        assert!(listened[0].album.is_none());
        assert_eq!(listened[1].album.as_ref().map(|a| a.artist_names()), Some("Band, Guest".into()));

        let mut contexts = Contexts::default();
        contexts.known.insert("spotify:playlist:p1".into(), KnownContext { name: "Morning".into(), subtitle: Some("Me, 42 songs".into()), image: Some("https://mosaic.scdn.co/p1".into()), fetched_ms: 1, refused: false });
        contexts.known.insert("spotify:artist:x1".into(), KnownContext { fetched_ms: 1, refused: true, ..Default::default() });
        let playlist = contexts.item_for(&listened[0], 5).unwrap();
        assert_eq!((playlist.id.as_str(), playlist.title.as_str(), playlist.subtitle.as_deref()), ("recent:spotify:playlist:p1", "Morning", Some("Me, 42 songs")));
        assert_eq!(playlist.action.view.as_deref(), Some("spotify:playlist:p1"));
        assert_eq!(playlist.action.fallback_launch.as_deref(), Some(APP_PACKAGE));
        assert_eq!(playlist.logo.as_deref(), Some("https://mosaic.scdn.co/p1"));
        let album = contexts.item_for(&listened[1], 4).unwrap();
        assert_eq!((album.title.as_str(), album.subtitle.as_deref(), album.thumbnail.as_deref()), ("Album One", Some("Band, Guest"), Some("https://i.scdn.co/a1")));
        assert!(contexts.item_for(&listened[2], 3).is_none(), "liked songs wait for their count");
        assert!(contexts.item_for(&listened[3], 2).is_none(), "an artist Spotify refused has no tile");
        assert_eq!(contexts.item_for(&listened[4], 1).unwrap().title, "Alone");
    }

    #[test]
    fn the_cache_asks_again_after_a_week_or_a_day_when_refused() {
        let mut c = Contexts { now_ms: 10 * DAY_MS, ..Default::default() };
        assert!(c.is_due("spotify:playlist:x"));
        c.known.insert("spotify:playlist:x".into(), KnownContext { fetched_ms: 4 * DAY_MS, ..Default::default() });
        assert!(!c.is_due("spotify:playlist:x"));
        c.known.insert("spotify:playlist:x".into(), KnownContext { fetched_ms: 2 * DAY_MS, ..Default::default() });
        assert!(c.is_due("spotify:playlist:x"));
        c.known.insert("spotify:playlist:y".into(), KnownContext { fetched_ms: 9 * DAY_MS + 1, refused: true, ..Default::default() });
        assert!(!c.is_due("spotify:playlist:y"));
        c.known.insert("spotify:playlist:y".into(), KnownContext { fetched_ms: 8 * DAY_MS, refused: true, ..Default::default() });
        assert!(c.is_due("spotify:playlist:y"));
        let page: Page<Playlist> = serde_json::from_str(r#"{"items":[{"id":"p1","name":"Morning","uri":"spotify:playlist:p1","images":null,"owner":{"display_name":"Me"},"items":{"total":42}}]}"#).unwrap();
        c.learn_playlists(&page.present(), 10 * DAY_MS);
        assert_eq!(c.known["spotify:playlist:p1"].subtitle.as_deref(), Some("Me, 42 songs"));
    }

    #[test]
    fn playlists_albums_and_shows_become_tiles_despite_nulls() {
        let page: Page<Playlist> = serde_json::from_str(
            r#"{"items":[{"id":"p1","name":"Morning","uri":"spotify:playlist:p1","images":[{"url":"https://mosaic.scdn.co/640/p1","width":640}],"owner":{"display_name":"Me"},"tracks":{"total":1}},null,{"id":"","name":"","uri":""}]}"#,
        )
        .unwrap();
        let items: Vec<Item> = page.present().into_iter().enumerate().filter_map(|(i, p)| playlist_item(p, 2 - i as i32)).collect();
        assert_eq!(items.len(), 1);
        assert_eq!((items[0].id.as_str(), items[0].subtitle.as_deref(), items[0].row.as_deref()), ("playlist:p1", Some("Me, 1 song"), Some("playlists")));
        assert_eq!(playlist_subtitle(None, None), "Playlist");

        let saved: Page<SavedAlbum> = serde_json::from_str(r#"{"items":[{"added_at":"2026-11-08T10:00:00Z","album":{"id":"a1","name":"Album One","uri":"spotify:album:a1","images":[{"url":"https://i.scdn.co/a1","width":640}],"artists":[{"name":"Band"}]}}]}"#).unwrap();
        let album = album_item(saved.present().into_iter().next().unwrap(), 1).unwrap();
        assert_eq!((album.id.as_str(), album.subtitle.as_deref(), album.is_new, album.row.as_deref()), ("album:a1", Some("Band"), false, Some("albums")));
        assert_eq!(album.added_ms, parse_iso8601("2026-11-08T10:00:00Z"));

        let shows: Page<SavedShow> = serde_json::from_str(r#"{"items":[{"added_at":"2026-01-01T00:00:00Z","show":{"id":"s1","name":"A Show","uri":"spotify:show:s1","total_episodes":120,"images":[{"url":"https://i.scdn.co/s1","width":640}]}}]}"#).unwrap();
        let show = show_item(shows.present().into_iter().next().unwrap(), 1).unwrap();
        assert_eq!((show.id.as_str(), show.subtitle.as_deref(), show.action.view.as_deref()), ("show:s1", Some("120 episodes"), Some("spotify:show:s1")));
        assert_eq!(largest(&[ImageRef { url: "https://first".into(), width: None }, ImageRef { url: "https://second".into(), width: None }]).as_deref(), Some("https://first"));
        assert_eq!(largest(&[ImageRef { url: "http://insecure".into(), width: Some(900) }, ImageRef { url: "https://ok".into(), width: Some(64) }]).as_deref(), Some("https://ok"));
        let nothing: Page<Playlist> = serde_json::from_str(r#"{"items":null}"#).unwrap();
        assert!(nothing.present().is_empty());
    }

    #[test]
    fn the_pasted_address_gives_up_its_code() {
        assert_eq!(pasted_code("http://127.0.0.1:8888/callback?code=AQB-xyz_123&state=s"), Pasted::Code("AQB-xyz_123".into()));
        assert_eq!(pasted_code("  127.0.0.1:8888/callback?code=AQB\n"), Pasted::Code("AQB".into()));
        assert_eq!(pasted_code("AQB-xyz_123"), Pasted::Code("AQB-xyz_123".into()));
        assert_eq!(pasted_code("http://127.0.0.1:8888/callback?error=access_denied"), Pasted::Declined);
        assert_eq!(pasted_code("http://127.0.0.1:8888/callback"), Pasted::Nothing);
        assert_eq!(pasted_code("   "), Pasted::Nothing);
    }

    #[test]
    fn the_sign_in_link_carries_a_pkce_challenge() {
        // RFC 7636's own example.
        assert_eq!(challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"), "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM");
        assert_eq!(base64url(&[0xfb, 0xff]), "-_8");
        assert_eq!(base64url(b"a"), "YQ");
        let url = authorize_url("abc123", "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk");
        assert!(url.starts_with("https://accounts.spotify.com/authorize?response_type=code&client_id=abc123&scope=user-read-recently-played%20"));
        assert!(url.contains("&redirect_uri=http%3A%2F%2F127.0.0.1%3A8888%2Fcallback&code_challenge_method=S256&code_challenge=E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM&show_dialog=true"));
        assert!(instruction("abc123", "v", true).starts_with("Spotify ends every sign-in after six months, so it is time to sign in again. Sign in on your phone: in Lounge's companion app"));
        // The host cuts an address with a scheme down to its path and keeps 1000 bytes of an error.
        let said = instruction("0123456789abcdef0123456789abcdef", "v", true);
        assert!(!said.contains("://"), "{said}");
        assert!(said.contains(" or open accounts.spotify.com/authorize?response_type=code&client_id=0123456789abcdef0123456789abcdef&scope="), "{said}");
        assert!(said.contains("&show_dialog=true. After you agree"), "{said}");
        assert!(said.len() + "That sign-in address did not work; a code is good for a few minutes and once. ".len() < 1000);
        let t: TokenAnswer = serde_json::from_str(r#"{"access_token":"BQ","token_type":"Bearer","scope":"user-read-recently-played","expires_in":3600,"refresh_token":"AQ"}"#).unwrap();
        assert_eq!((t.access_token.as_str(), t.refresh_token.as_deref(), t.expires_in), ("BQ", Some("AQ"), 3600));
        let token = Token { access: "a".into(), refresh: "r".into(), expires_ms: 0, client_id: "c".into(), signed_in_ms: 0 };
        assert_eq!(token.days_left(100 * DAY_MS), None);
        let token = Token { signed_in_ms: 10 * DAY_MS, ..token };
        assert_eq!(token.days_left(100 * DAY_MS), Some(92));
    }

    #[test]
    fn spotifys_refusals_are_read_as_text_or_json() {
        assert_eq!(refused_why("Check settings on https://developer.spotify.com/dashboard, the user may not be registered."), "Spotify's app is in development mode and this account is not on its list. Add the account under User Management in the app's dashboard.");
        assert_eq!(refused_why("Active premium subscription required for the owner of the app"), "Spotify needs the account that made the app to have Premium.");
        assert_eq!(refused_why(r#"{"error":{"status":403,"message":"Forbidden"}}"#), "Spotify refused: Forbidden");
        assert_eq!(refused_why(""), "Spotify refused the request. Check the app in the developer dashboard.");
    }

    #[test]
    fn pasted_links_become_uris_in_order() {
        let links = parse_links("https://open.spotify.com/playlist/37i9dQZF1DXcBWIGoYBM5M?si=abc\nspotify:album:4aawyAB9vmqN3uQ7FjRGTy, https://open.spotify.com/intl-de/artist/0OdUWJ0sBjDrqHygGUXeCF\nhttps://open.spotify.com/show/5CfCWKI5pZ28U0uOzXkDHe https://example.com/x spotify:track:11dFghVXANMlKmJXsNCbNl spotify:playlist:37i9dQZF1DXcBWIGoYBM5M");
        assert_eq!(links, ["spotify:playlist:37i9dQZF1DXcBWIGoYBM5M", "spotify:album:4aawyAB9vmqN3uQ7FjRGTy", "spotify:artist:0OdUWJ0sBjDrqHygGUXeCF", "spotify:show:5CfCWKI5pZ28U0uOzXkDHe", "spotify:track:11dFghVXANMlKmJXsNCbNl"]);
        assert_eq!(kind_word("spotify:show:x"), "Podcast");
        let mut picks = Picks::default();
        picks.known.insert("spotify:album:a".into(), KnownPick { title: "An Album".into(), image: Some("https://i.scdn.co/a".into()), fetched_ms: 5, refused: false });
        picks.known.insert("spotify:album:gone".into(), KnownPick { title: "Old".into(), image: None, fetched_ms: 5, refused: false });
        // Off the TV every fetch fails, so only what is already known becomes a tile.
        let items = picks.items_for(&["spotify:playlist:new".to_string(), "spotify:album:a".to_string()], 6);
        assert_eq!(items.len(), 1);
        assert_eq!((items[0].id.as_str(), items[0].title.as_str(), items[0].subtitle.as_deref(), items[0].weight), ("pick:spotify:album:a", "An Album", Some("Album"), 1));
        assert!(!picks.known.contains_key("spotify:album:gone"));
    }

    #[test]
    fn errors_say_what_to_do() {
        // Off the TV there is no host, so a fetch fails as an unreachable service would.
        let mut client = Client { client_id: "c".into(), token: Token { access: "t".into(), refresh: "r".into(), expires_ms: i64::MAX, client_id: "c".into(), signed_in_ms: 0 }, now_ms: 0, paused: false };
        assert_eq!(client.get_json::<Page<Playlist>>("/me/playlists").err().as_deref(), Some("Spotify didn't answer. Check the TV's internet connection, then try again."));
        client.paused = true;
        assert_eq!(client.get_json::<Page<Playlist>>("/me/playlists").err().as_deref(), Some(PAUSE));
        assert!(Account::connect(0).unwrap().is_none());
        let manifest = <Spotify as sdk::Plugin>::manifest();
        assert!(manifest.settings.iter().all(|s| !s.required));
        assert!(manifest.fetch_hosts.iter().any(|h| h == "open.spotify.com"));
    }
}
