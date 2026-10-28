//! Trakt: what you paused, the next episode of every show you are watching and your watchlist,
//! whichever service they live on. Trakt knows the title, not the app, so selecting an item
//! searches for it: Lounge's own search, or the TV's. Sign-in is Trakt's device code, shown on
//! the plugin's page until it has been entered at trakt.tv/activate.
use lounge_plugin_sdk::{self as sdk, host, parse_iso8601, Action, HttpRequest, Item, ItemKind, Kind, Manifest, RefreshContext, RowSpec, RowStyle, SettingKind, SettingSpec};
use serde::{Deserialize, Serialize};

pub struct Trakt;

const API: &str = "https://api.trakt.tv";
const TOKEN_KEY: &str = "token";
const DEVICE_KEY: &str = "device";
const SIGNED_OUT: &str = "Trakt no longer accepts this sign-in; it is renewed on the next refresh, or press Refresh now.";
const PAUSE: &str = "Trakt is asking for a pause between requests. It tries again on the next refresh.";
/// Most shows in Up next.
const UP_NEXT_SHOWS: usize = 12;
/// Shows looked at to fill Up next, so ones already caught up on do not leave it short; well
/// inside the host's 64 fetches a refresh.
const UP_NEXT_LOOKUPS: usize = 36;
const WATCHLIST_ITEMS: usize = 24;
const WEEK_MS: i64 = 7 * 86_400_000;
const HOUR_MS: i64 = 3_600_000;

impl sdk::Plugin for Trakt {
    fn manifest() -> Manifest {
        Manifest {
            id: "tv.lounge.trakt".into(),
            name: "Trakt".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            description: "Continue watching, up next and your watchlist from Trakt, across every service.".into(),
            publisher_name: "Lounge".into(),
            kinds: vec![Kind::Content],
            fetch_hosts: vec!["api.trakt.tv".into(), "*.trakt.tv".into()],
            refresh_minutes: 15,
            rows: vec![
                RowSpec { id: "upnext".into(), title: "Up next".into(), style: RowStyle::Landscape },
                RowSpec { id: "watchlist".into(), title: "Watchlist".into(), style: RowStyle::Poster },
            ],
            settings: vec![
                SettingSpec {
                    key: "client_id".into(),
                    label: "Client ID".into(),
                    kind: SettingKind::Text,
                    required: true,
                    help: Some("At trakt.tv/oauth/applications create an app (any name, redirect urn:ietf:wg:oauth:2.0:oob) and copy its Client ID and Client Secret.".into()),
                    default: None,
                },
                SettingSpec { key: "client_secret".into(), label: "Client secret".into(), kind: SettingKind::Password, required: true, help: None, default: None },
                SettingSpec { key: "upnext".into(), label: "Up next row".into(), kind: SettingKind::Toggle, required: false, help: Some("The next episode of each show you are part-way through.".into()), default: Some("true".into()) },
                SettingSpec { key: "watchlist".into(), label: "Watchlist row".into(), kind: SettingKind::Toggle, required: false, help: None, default: Some("true".into()) },
                SettingSpec {
                    key: "tv_search".into(),
                    label: "Open with the TV's own search".into(),
                    kind: SettingKind::Toggle,
                    required: false,
                    help: Some("Off: Lounge searches your apps and plugins for the title. On: the TV's search, which on Google TV lists where it streams.".into()),
                    default: Some("false".into()),
                },
            ],
            homepage: Some("https://trakt.tv".into()),
        }
    }

    fn refresh(ctx: &RefreshContext) -> Result<Vec<Item>, String> {
        let client = Client::connect(ctx.now_ms)?;
        let tv_search = host::setting("tv_search").map(|v| v == "true").unwrap_or(false);
        let open = |title: &str| if tv_search { tv_search_action(title) } else { Action::search(title) };
        let mut items = Vec::new();

        let playback: Vec<Playback> = client.get_json("/sync/playback?extended=full,images&limit=50")?;
        let now = ctx.now_ms;
        for p in &playback {
            if let Some(item) = resume_item(p, &open) {
                items.push(item);
            }
        }
        let paused = items.len();
        // An episode paused part-way is in Continue watching; Up next would show it twice.
        let paused_episodes: Vec<u64> = playback.iter().filter_map(|p| p.episode.as_ref()).map(|e| e.ids.trakt).filter(|&id| id > 0).collect();

        let mut up_next = 0;
        if host::setting("upnext").map(|v| v != "false").unwrap_or(true) {
            let watched: Vec<Watched> = client.get_json("/sync/watched/shows?extended=noseasons,images")?;
            let mut shows: Vec<&Watched> = watched.iter().filter(|w| w.show.ids.trakt > 0).collect();
            shows.sort_by_key(|w| std::cmp::Reverse(w.last_watched_at.as_deref().and_then(parse_iso8601).unwrap_or(0)));
            for (i, w) in shows.iter().take(UP_NEXT_LOOKUPS).enumerate() {
                if up_next >= UP_NEXT_SHOWS {
                    break;
                }
                let progress: Progress = match client.get_json(&format!("/shows/{}/progress/watched?hidden=false&specials=false&count_specials=false", w.show.ids.trakt)) {
                    Ok(p) => p,
                    // Every request after one of these fails the same way.
                    Err(e) if e.contains("budget") || e == SIGNED_OUT || e == PAUSE => break,
                    Err(e) => {
                        sdk::warn!("{}: {e}", w.show.title);
                        continue;
                    }
                };
                let next = progress.next_episode.as_ref().filter(|e| !paused_episodes.contains(&e.ids.trakt));
                if let Some(item) = up_next_item(w, next, (UP_NEXT_LOOKUPS - i) as i32, &open) {
                    items.push(item);
                    up_next += 1;
                }
            }
        }

        let mut listed = 0;
        if host::setting("watchlist").map(|v| v != "false").unwrap_or(true) {
            let watchlist = || -> Result<Vec<Listed>, String> {
                let mut entries: Vec<Listed> = client.get_json("/sync/watchlist/movies/added?extended=full,images")?;
                entries.extend(client.get_json::<Vec<Listed>>("/sync/watchlist/shows/added?extended=full,images")?);
                Ok(entries)
            };
            let mut entries = match watchlist() {
                Ok(entries) => entries,
                // What is already gathered is worth more than a watchlist that did not come.
                Err(e) if !items.is_empty() => {
                    sdk::warn!("watchlist: {e}");
                    Vec::new()
                }
                Err(e) => return Err(e),
            };
            entries.sort_by_key(|l| std::cmp::Reverse(l.listed_at.as_deref().and_then(parse_iso8601).unwrap_or(0)));
            for (i, l) in entries.iter().take(WATCHLIST_ITEMS).enumerate() {
                if let Some(item) = watchlist_item(l, now, (WATCHLIST_ITEMS - i) as i32, &open) {
                    items.push(item);
                    listed += 1;
                }
            }
        }
        sdk::dedup_ids(&mut items);
        sdk::info!("{paused} paused, {up_next} up next, {listed} on the watchlist");
        Ok(items)
    }
}

sdk::export_plugin!(Trakt);

// ---------------------------------------------------------------------------
// Signing in: Trakt's device code, one step per refresh
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize)]
struct Token {
    access: String,
    refresh: String,
    expires_ms: i64,
    /// How long Trakt said the token lasts; 0 for a token saved before this was kept.
    #[serde(default)]
    lifetime_ms: i64,
}

impl Token {
    /// Renewal is due once a fifth of the token's life is left, and always an hour ahead.
    fn due(&self, now_ms: i64) -> bool {
        let ahead = if self.lifetime_ms > 0 { (self.lifetime_ms / 5).max(HOUR_MS) } else { WEEK_MS };
        self.expires_ms - now_ms < ahead
    }
}

/// Why a renewal failed: Trakt refused the refresh token, or no usable answer came back.
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
        // Only a refresh token Trakt refuses needs the person to sign in again.
        Err(RenewError::Refused(status)) => {
            sdk::warn!("Trakt refused to renew the sign-in (HTTP {status}); signing in again");
            Renewed::SignInAgain
        }
        // Anything else may pass, so the current token serves while it lasts.
        Err(RenewError::Failed(e)) if current.expires_ms > now_ms => {
            sdk::warn!("could not renew the sign-in ({e}); using the current one");
            Renewed::Use(current)
        }
        Err(RenewError::Failed(e)) => {
            sdk::warn!("could not renew the sign-in ({e})");
            Renewed::TryLater("Could not renew the Trakt sign-in. It tries again on the next refresh.".into())
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
    verification_url: String,
    expires_ms: i64,
}

#[derive(Deserialize)]
struct DeviceAnswer {
    device_code: String,
    user_code: String,
    verification_url: String,
    expires_in: i64,
}

#[derive(Deserialize)]
struct TokenAnswer {
    access_token: String,
    refresh_token: String,
    expires_in: i64,
}

struct Client {
    client_id: String,
    token: String,
}

impl Client {
    /// A signed-in client, or the one sentence the plugin's page should show to get there.
    fn connect(now_ms: i64) -> Result<Client, String> {
        let client_id = host::required_setting("client_id", "Client ID")?.trim().to_string();
        let secret = host::secret("client_secret").map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).ok_or("Client secret is not set")?;
        let saved: Option<Token> = host::kv_get(TOKEN_KEY).and_then(|v| serde_json::from_slice(&v).ok());
        let token = match saved {
            Some(t) if !t.due(now_ms) => t,
            Some(t) => {
                let outcome = renew(&client_id, &secret, &t, now_ms);
                match after_renewal(t, outcome, now_ms) {
                    Renewed::Use(t) => t,
                    Renewed::SignInAgain => {
                        host::kv_delete(TOKEN_KEY);
                        device_step(&client_id, &secret, now_ms)?
                    }
                    Renewed::TryLater(e) => return Err(e),
                }
            }
            None => device_step(&client_id, &secret, now_ms)?,
        };
        Ok(Client { client_id, token: token.access })
    }

    fn request(&self, path: &str) -> HttpRequest {
        HttpRequest::get(format!("{API}{path}"))
            .header("Content-Type", "application/json")
            .header("trakt-api-version", "2")
            .header("trakt-api-key", &self.client_id)
            .header("Authorization", &format!("Bearer {}", self.token))
    }

    fn get_json<T: serde::de::DeserializeOwned>(&self, path: &str) -> Result<T, String> {
        let r = sdk::fetch_online("Trakt", &self.request(path))?;
        let path = path.split('?').next().unwrap_or(path);
        match r.status {
            401 => {
                expire_token();
                return Err(SIGNED_OUT.into());
            }
            429 => return Err(PAUSE.into()),
            s if !(200..300).contains(&s) => {
                sdk::warn!("HTTP {s} for {path}");
                return Err(sdk::online_error("Trakt", s));
            }
            _ => {}
        }
        serde_json::from_str(&r.body).map_err(|e| {
            sdk::warn!("unexpected answer for {path}: {e}");
            sdk::unreadable("Trakt", API)
        })
    }
}

fn post(path: &str, body: serde_json::Value, client_id: &str) -> Result<(u16, String), String> {
    let req = HttpRequest {
        method: "POST".into(),
        url: format!("{API}{path}"),
        headers: [("Content-Type".to_string(), "application/json".to_string()), ("trakt-api-version".to_string(), "2".to_string()), ("trakt-api-key".to_string(), client_id.to_string())].into_iter().collect(),
        body: Some(body.to_string()),
        timeout_ms: 0,
    };
    let r = sdk::fetch_online("Trakt", &req)?;
    Ok((r.status, r.body))
}

/// The error for a sign-in answer that cannot be read; the cause goes to the log.
fn unreadable_sign_in(e: serde_json::Error) -> String {
    sdk::warn!("unexpected sign-in answer: {e}");
    sdk::unreadable("Trakt", API)
}

/// The token once the sign-in has gone through, so the refresh goes on with it, or else the
/// sentence the plugin's page shows meanwhile. One poll per refresh is all the host's rhythm
/// allows.
fn device_step(client_id: &str, secret: &str, now_ms: i64) -> Result<Token, String> {
    let pending: Option<Device> = host::kv_get(DEVICE_KEY).and_then(|v| serde_json::from_slice(&v).ok()).filter(|d: &Device| d.expires_ms > now_ms);
    if let Some(d) = pending {
        let (status, body) = post("/oauth/device/token", serde_json::json!({ "code": d.device_code, "client_id": client_id, "client_secret": secret }), client_id)?;
        match status {
            200 => {
                let t: TokenAnswer = serde_json::from_str(&body).map_err(unreadable_sign_in)?;
                host::kv_delete(DEVICE_KEY);
                return Ok(save_token(&t, now_ms));
            }
            400 | 429 => return Err(instruction(&d)),
            418 => {
                host::kv_delete(DEVICE_KEY);
                return Err("The sign-in was declined on trakt.tv. Press Refresh now for a new code.".into());
            }
            _ => host::kv_delete(DEVICE_KEY),
        }
    }
    let (status, body) = post("/oauth/device/code", serde_json::json!({ "client_id": client_id }), client_id)?;
    if status == 403 || status == 401 {
        return Err("Trakt does not know this Client ID. Check it against your app at trakt.tv/oauth/applications.".into());
    }
    if !(200..300).contains(&status) {
        sdk::warn!("HTTP {status} when asked for a sign-in code");
        return Err(sdk::online_error("Trakt", status));
    }
    let a: DeviceAnswer = serde_json::from_str(&body).map_err(unreadable_sign_in)?;
    let d = Device { device_code: a.device_code, user_code: a.user_code, verification_url: a.verification_url, expires_ms: now_ms + a.expires_in * 1000 };
    host::kv_set(DEVICE_KEY, serde_json::to_string(&d).unwrap_or_default().as_bytes());
    Err(instruction(&d))
}

fn instruction(d: &Device) -> String {
    format!("Sign in: on your phone open {} and enter {}, then press Refresh now here.", d.verification_url.trim_start_matches("https://"), d.user_code)
}

fn renew(client_id: &str, secret: &str, old: &Token, now_ms: i64) -> Result<Token, RenewError> {
    let (status, body) = post(
        "/oauth/token",
        serde_json::json!({ "refresh_token": old.refresh, "client_id": client_id, "client_secret": secret, "redirect_uri": "urn:ietf:wg:oauth:2.0:oob", "grant_type": "refresh_token" }),
        client_id,
    )
    .map_err(RenewError::Failed)?;
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
    let lifetime_ms = t.expires_in * 1000;
    let token = Token { access: t.access_token.clone(), refresh: t.refresh_token.clone(), expires_ms: now_ms + lifetime_ms, lifetime_ms };
    host::kv_set(TOKEN_KEY, serde_json::to_string(&token).unwrap_or_default().as_bytes());
    token
}

// ---------------------------------------------------------------------------
// What Trakt sends
// ---------------------------------------------------------------------------

#[derive(Deserialize, Default, Clone, Debug)]
pub struct Ids {
    #[serde(default)]
    pub trakt: u64,
    #[serde(default)]
    pub slug: Option<String>,
}

#[derive(Deserialize, Default, Clone, Debug)]
pub struct Images {
    #[serde(default)]
    pub poster: Vec<String>,
    #[serde(default)]
    pub fanart: Vec<String>,
    #[serde(default)]
    pub screenshot: Vec<String>,
    #[serde(default)]
    pub thumb: Vec<String>,
}

impl Images {
    fn first(list: &[String]) -> Option<String> {
        list.first().map(|u| if u.contains("://") { u.clone() } else { format!("https://{u}") })
    }
    fn poster(&self) -> Option<String> {
        Self::first(&self.poster)
    }
    fn wide(&self) -> Option<String> {
        Self::first(&self.fanart).or_else(|| Self::first(&self.screenshot)).or_else(|| Self::first(&self.thumb))
    }
}

#[derive(Deserialize, Default, Clone, Debug)]
pub struct Media {
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub year: Option<u32>,
    #[serde(default)]
    pub ids: Ids,
    /// Minutes.
    #[serde(default)]
    pub runtime: Option<u32>,
    #[serde(default)]
    pub images: Images,
}

#[derive(Deserialize, Default, Clone, Debug)]
pub struct Episode {
    #[serde(default)]
    pub season: u32,
    #[serde(default)]
    pub number: u32,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub ids: Ids,
    #[serde(default)]
    pub runtime: Option<u32>,
    #[serde(default)]
    pub images: Images,
}

#[derive(Deserialize, Default, Clone, Debug)]
pub struct Playback {
    #[serde(default)]
    pub progress: f32,
    #[serde(default)]
    pub paused_at: Option<String>,
    #[serde(rename = "type", default)]
    pub kind: String,
    #[serde(default)]
    pub movie: Option<Media>,
    #[serde(default)]
    pub show: Option<Media>,
    #[serde(default)]
    pub episode: Option<Episode>,
}

#[derive(Deserialize, Default, Clone, Debug)]
pub struct Watched {
    #[serde(default)]
    pub last_watched_at: Option<String>,
    #[serde(default)]
    pub show: Media,
}

#[derive(Deserialize, Default, Clone, Debug)]
pub struct Progress {
    #[serde(default)]
    pub next_episode: Option<Episode>,
}

#[derive(Deserialize, Default, Clone, Debug)]
pub struct Listed {
    #[serde(default)]
    pub listed_at: Option<String>,
    #[serde(rename = "type", default)]
    pub kind: String,
    #[serde(default)]
    pub movie: Option<Media>,
    #[serde(default)]
    pub show: Option<Media>,
}

/// The TV's own search for a title: the intent its global search provider answers.
pub fn tv_search_action(title: &str) -> Action {
    Action { view: Some(format!("intent:#Intent;action=android.search.action.GLOBAL_SEARCH;S.query={};end", uri_encode(title))), ..Default::default() }
}

pub fn resume_item(p: &Playback, open: &dyn Fn(&str) -> Action) -> Option<Item> {
    let paused = p.paused_at.as_deref().and_then(parse_iso8601);
    let progress = (p.progress / 100.0).clamp(0.0, 1.0);
    let item = match p.kind.as_str() {
        "movie" => {
            let m = p.movie.as_ref()?;
            Item {
                id: format!("resume:movie:{}", m.ids.trakt),
                kind: ItemKind::Resume,
                title: m.title.clone(),
                subtitle: m.year.map(|y| y.to_string()),
                duration_ms: m.runtime.map(|r| i64::from(r) * 60_000),
                poster: m.images.poster(),
                thumbnail: m.images.wide(),
                action: open(&m.title),
                ..Default::default()
            }
        }
        "episode" => {
            let (s, e) = (p.show.as_ref()?, p.episode.as_ref()?);
            Item {
                id: format!("resume:episode:{}", e.ids.trakt),
                kind: ItemKind::Resume,
                title: s.title.clone(),
                subtitle: e.title.clone().filter(|t| !t.is_empty()),
                season: Some(e.season.to_string()),
                episode: Some(e.number.to_string()),
                duration_ms: e.runtime.or(s.runtime).map(|r| i64::from(r) * 60_000),
                poster: s.images.poster(),
                thumbnail: e.images.wide().or_else(|| s.images.wide()),
                action: open(&s.title),
                ..Default::default()
            }
        }
        _ => return None,
    };
    if item.title.trim().is_empty() {
        return None;
    }
    Some(Item {
        progress: Some(progress),
        position_ms: item.duration_ms.map(|d| (d as f32 * progress) as i64),
        last_used_ms: paused,
        ..item
    })
}

pub fn up_next_item(w: &Watched, next: Option<&Episode>, weight: i32, open: &dyn Fn(&str) -> Action) -> Option<Item> {
    let e = next?;
    if w.show.title.trim().is_empty() {
        return None;
    }
    Some(Item {
        id: format!("next:{}", w.show.ids.trakt),
        kind: ItemKind::Program,
        title: w.show.title.clone(),
        subtitle: e.title.clone().filter(|t| !t.is_empty()),
        season: Some(e.season.to_string()),
        episode: Some(e.number.to_string()),
        added_ms: w.last_watched_at.as_deref().and_then(parse_iso8601),
        poster: w.show.images.poster(),
        thumbnail: e.images.wide().or_else(|| w.show.images.wide()),
        row: Some("upnext".into()),
        weight,
        action: open(&w.show.title),
        ..Default::default()
    })
}

pub fn watchlist_item(l: &Listed, now_ms: i64, weight: i32, open: &dyn Fn(&str) -> Action) -> Option<Item> {
    let m = match l.kind.as_str() {
        "movie" => l.movie.as_ref()?,
        "show" => l.show.as_ref()?,
        _ => return None,
    };
    if m.title.trim().is_empty() {
        return None;
    }
    let listed = l.listed_at.as_deref().and_then(parse_iso8601);
    Some(Item {
        id: format!("list:{}:{}", l.kind, m.ids.trakt),
        kind: ItemKind::Program,
        title: m.title.clone(),
        subtitle: m.year.map(|y| y.to_string()),
        added_ms: listed,
        is_new: listed.is_some_and(|t| now_ms - t < WEEK_MS),
        poster: m.images.poster(),
        thumbnail: m.images.wide(),
        row: Some("watchlist".into()),
        weight,
        action: open(&m.title),
        ..Default::default()
    })
}

/// Percent-encoding as `Intent.parseUri` undoes it: no plus signs for spaces.
fn uri_encode(s: &str) -> String {
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

    const PLAYBACK: &str = r#"[
      {"progress": 25.5, "paused_at": "2026-10-20T22:01:32.000Z", "id": 13, "type": "movie",
       "movie": {"title": "Charade", "year": 1963, "runtime": 113, "ids": {"trakt": 100, "slug": "charade-1963"},
                 "images": {"fanart": ["walter.trakt.tv/images/movies/000/000/100/fanarts/medium/a.jpg"], "poster": ["walter.trakt.tv/images/movies/000/000/100/posters/thumb/p.jpg"]}}},
      {"progress": 60, "paused_at": "2026-10-21T20:00:00.000Z", "id": 14, "type": "episode",
       "episode": {"season": 2, "number": 4, "title": "Honeydew", "runtime": 30, "ids": {"trakt": 555}, "images": {"screenshot": ["walter.trakt.tv/images/episodes/000/000/555/screenshots/medium/s.jpg"]}},
       "show": {"title": "The Bear", "year": 2022, "ids": {"trakt": 1390}, "images": {"poster": ["walter.trakt.tv/images/shows/000/001/390/posters/thumb/p.jpg"]}}},
      {"progress": 10, "type": "podcast"}
    ]"#;

    #[test]
    fn paused_playback_becomes_resume_points() {
        let list: Vec<Playback> = serde_json::from_str(PLAYBACK).unwrap();
        let open = |t: &str| Action::search(t);
        let items: Vec<Item> = list.iter().filter_map(|p| resume_item(p, &open)).collect();
        assert_eq!(items.len(), 2);
        let movie = &items[0];
        assert_eq!(movie.id, "resume:movie:100");
        assert_eq!(movie.kind, ItemKind::Resume);
        assert_eq!(movie.progress, Some(0.255));
        assert_eq!(movie.duration_ms, Some(113 * 60_000));
        assert_eq!(movie.position_ms, Some((113.0 * 60_000.0 * 0.255) as i64));
        assert_eq!(movie.last_used_ms, parse_iso8601("2026-10-20T22:01:32.000Z"));
        assert_eq!(movie.thumbnail.as_deref(), Some("https://walter.trakt.tv/images/movies/000/000/100/fanarts/medium/a.jpg"));
        assert_eq!(movie.action.search.as_deref(), Some("Charade"));
        let ep = &items[1];
        assert_eq!(ep.title, "The Bear");
        assert_eq!(ep.subtitle.as_deref(), Some("Honeydew"));
        assert_eq!((ep.season.as_deref(), ep.episode.as_deref()), (Some("2"), Some("4")));
        assert!(ep.thumbnail.as_deref().unwrap().contains("screenshots"));
        assert!(ep.poster.as_deref().unwrap().contains("posters"));
    }

    #[test]
    fn the_next_episode_and_the_watchlist_fill_their_rows() {
        let w: Watched = serde_json::from_str(r#"{"plays": 56, "last_watched_at": "2026-10-19T09:00:00.000Z", "show": {"title": "Severance", "year": 2022, "ids": {"trakt": 7}, "images": {"fanart": ["walter.trakt.tv/f.jpg"]}}}"#).unwrap();
        let p: Progress = serde_json::from_str(r#"{"aired": 19, "completed": 12, "next_episode": {"season": 2, "number": 3, "title": "Who Is Alive?", "ids": {"trakt": 9}}}"#).unwrap();
        let open = |t: &str| tv_search_action(t);
        let item = up_next_item(&w, p.next_episode.as_ref(), 5, &open).unwrap();
        assert_eq!(item.row.as_deref(), Some("upnext"));
        assert_eq!(item.subtitle.as_deref(), Some("Who Is Alive?"));
        assert_eq!(item.thumbnail.as_deref(), Some("https://walter.trakt.tv/f.jpg"));
        assert_eq!(item.action.view.as_deref(), Some("intent:#Intent;action=android.search.action.GLOBAL_SEARCH;S.query=Severance;end"));
        let done: Progress = serde_json::from_str(r#"{"aired": 19, "completed": 19, "next_episode": null}"#).unwrap();
        assert!(up_next_item(&w, done.next_episode.as_ref(), 5, &open).is_none());

        let l: Vec<Listed> = serde_json::from_str(r#"[{"rank": 1, "listed_at": "2026-10-21T10:00:00.000Z", "type": "movie", "movie": {"title": "Metropolis", "year": 1927, "ids": {"trakt": 3}}},
            {"rank": 2, "listed_at": "2026-01-01T10:00:00.000Z", "type": "show", "show": {"title": "Patience", "year": 2024, "ids": {"trakt": 4}}}]"#).unwrap();
        let now = parse_iso8601("2026-10-22T00:00:00Z").unwrap();
        let a = watchlist_item(&l[0], now, 2, &open).unwrap();
        assert_eq!(a.id, "list:movie:3");
        assert!(a.is_new);
        assert_eq!(a.row.as_deref(), Some("watchlist"));
        let b = watchlist_item(&l[1], now, 1, &open).unwrap();
        assert_eq!(b.id, "list:show:4");
        assert!(!b.is_new);
        assert_eq!(uri_encode("The Bear & co"), "The%20Bear%20%26%20co");
    }

    #[test]
    fn the_sign_in_instruction_names_the_code() {
        let d = Device { device_code: "x".into(), user_code: "5055CC52".into(), verification_url: "https://trakt.tv/activate".into(), expires_ms: 0 };
        assert_eq!(instruction(&d), "Sign in: on your phone open trakt.tv/activate and enter 5055CC52, then press Refresh now here.");
    }

    fn token(expires_ms: i64, lifetime_ms: i64) -> Token {
        Token { access: "old".into(), refresh: "r".into(), expires_ms, lifetime_ms }
    }

    #[test]
    fn renewal_is_due_when_a_fifth_of_the_life_is_left() {
        let day = 86_400_000;
        assert!(!token(day, day).due(0));
        assert!(!token(day, day).due(day - 5 * HOUR_MS));
        assert!(token(day, day).due(day - 4 * HOUR_MS));
        assert!(!token(90 * day, 90 * day).due(60 * day));
        assert!(token(90 * day, 90 * day).due(75 * day));
        // A token saved before its lifetime was kept renews a week ahead, as it did.
        let old: Token = serde_json::from_str(r#"{"access": "a", "refresh": "r", "expires_ms": 1000000000}"#).unwrap();
        assert!(old.due(1_000_000_000 - WEEK_MS + 1));
        assert!(!old.due(1_000_000_000 - WEEK_MS - 1));
    }

    #[test]
    fn only_a_refused_renewal_asks_for_a_new_sign_in() {
        assert!(matches!(after_renewal(token(10, 10), Err(RenewError::Refused(401)), 0), Renewed::SignInAgain));
        assert!(matches!(after_renewal(token(10, 10), Err(RenewError::Failed("HTTP 503".into())), 0), Renewed::Use(t) if t.access == "old"));
        assert!(matches!(after_renewal(token(10, 10), Err(RenewError::Failed("timed out".into())), 20), Renewed::TryLater(e) if e == "Could not renew the Trakt sign-in. It tries again on the next refresh."));
        assert!(matches!(after_renewal(token(10, 10), Ok(token(99, 99)), 0), Renewed::Use(t) if t.expires_ms == 99));
    }
}
