//! Nuvio: what you paused, what is up next and the library of your Nuvio account, read from
//! the sync backend the Nuvio apps share, on nuvio.tv or self-hosted. Nuvio keeps only ids and
//! positions there, so names and pictures come from the library, Cinemeta and, for TMDB ids,
//! TMDB. Items open on their page in the Nuvio TV app; search asks Cinemeta, so a title found
//! here opens there too.

use lounge_plugin_sdk::{self as sdk, host, Action, HttpRequest, Item, ItemKind, Kind, Manifest, RefreshContext, RowSpec, RowStyle, SettingKind, SettingSpec};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};

pub struct Nuvio;

const OFFICIAL: &str = "https://api.nuvio.tv";
const CINEMETA: &str = "https://v3-cinemeta.strem.io";
const TMDB: &str = "https://api.themoviedb.org/3";
const TMDB_IMAGES: &str = "https://image.tmdb.org/t/p";
const GITHUB_PACKAGE: &str = "com.nuvio.tv";
const PLAY_PACKAGE: &str = "com.nuvio.app";
const BACKEND_KEY: &str = "backend";
const SESSION_KEY: &str = "session";
const META_KEY: &str = "meta_v2";
const RESUME_ITEMS: usize = 24;
/// Most shows in up next.
const UP_NEXT_SHOWS: usize = 12;
const LIBRARY_ITEMS: usize = 60;
/// Most lookups of names and pictures in one refresh; the rest come with the next.
const LOOKUPS: usize = 20;
/// Nuvio counts a title finished from here, and drops it from Continue watching.
const FINISHED: f32 = 0.9;
/// Nuvio's own home stops showing a position older than this.
const KEPT_DAYS_MS: i64 = 60 * 86_400_000;
/// The key-value store takes 256 KB a key; the cache stays well under it.
const CACHE_BYTES: usize = 200 * 1024;
const MINUTE_MS: i64 = 60_000;
const REJECTED: &str = "Nuvio rejected the email or password. Check them in the plugin's settings.";

impl sdk::Plugin for Nuvio {
    fn manifest() -> Manifest {
        Manifest {
            id: "tv.lounge.nuvio".into(),
            name: "Nuvio".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            description: "Continue watching, up next and your library from your Nuvio account's sync, opened in Nuvio.".into(),
            publisher_name: "Lounge".into(),
            kinds: vec![Kind::Content, Kind::Search],
            fetch_hosts: vec!["api.nuvio.tv".into(), "$server".into(), "v3-cinemeta.strem.io".into(), "api.themoviedb.org".into(), "*".into()],
            refresh_minutes: 15,
            rows: vec![RowSpec { id: "library".into(), title: "Library on Nuvio".into(), style: RowStyle::Poster }],
            settings: vec![
                SettingSpec {
                    key: "email".into(),
                    label: "Email".into(),
                    kind: SettingKind::Text,
                    required: true,
                    help: Some("The email of your Nuvio account. Positions come from Nuvio's own sync: a profile that follows Trakt, Simkl or MDBList for its progress keeps none there, so use the Trakt plugin for that one.".into()),
                    default: None,
                },
                SettingSpec { key: "password".into(), label: "Password".into(), kind: SettingKind::Password, required: true, help: None, default: None },
                SettingSpec {
                    key: "profile".into(),
                    label: "Profile".into(),
                    kind: SettingKind::Text,
                    required: false,
                    help: Some("Leave empty for the first profile, or type another profile's name as Nuvio shows it.".into()),
                    default: None,
                },
                SettingSpec { key: "library".into(), label: "Library row".into(), kind: SettingKind::Toggle, required: false, help: Some("The movies and shows saved in your Nuvio library, newest first.".into()), default: Some("true".into()) },
                SettingSpec {
                    key: "tmdb_key".into(),
                    label: "TMDB API key".into(),
                    kind: SettingKind::Password,
                    required: false,
                    help: Some("Optional, for when tiles show a code like tmdb:1234 instead of a name. The API Key (v3) from themoviedb.org/settings/api, not the Read Access Token.".into()),
                    default: None,
                },
                SettingSpec {
                    key: "server".into(),
                    label: "Self-hosted server".into(),
                    kind: SettingKind::Url,
                    required: false,
                    help: Some("Leave empty for Nuvio's own. For a self-hosted backend, its address, like https://nuvio.example.com.".into()),
                    default: None,
                },
                SettingSpec {
                    key: "play_store".into(),
                    label: "Installed from Google Play".into(),
                    kind: SettingKind::Toggle,
                    required: false,
                    help: Some("On if you installed Nuvio from Google Play, off if from GitHub or a downloaded APK.".into()),
                    default: Some("false".into()),
                },
            ],
            homepage: Some("https://nuvio.tv".into()),
            self_signed_servers: false,
        }
    }

    fn refresh(ctx: &RefreshContext) -> Result<Vec<Item>, String> {
        let mut client = Client::connect(ctx.now_ms)?;
        let profile = resolve_profile(&mut client)?;
        let package = app_package();
        let tmdb_key = host::secret("tmdb_key").map(|k| k.trim().to_string()).filter(|k| !k.is_empty());
        let mut items = Vec::new();

        let progress: Vec<Progress> = client.rpc("sync_pull_watch_progress", serde_json::json!({ "p_profile_id": profile, "p_since_last_watched": ctx.now_ms - KEPT_DAYS_MS, "p_limit": 300 }))?;
        let watching = continue_watching(&progress);
        let watched: Vec<Watched> = match client.rpc("sync_pull_watched_items", serde_json::json!({ "p_profile_id": profile, "p_page": 1, "p_page_size": 300 })) {
            Ok(watched) => watched,
            Err(e) => {
                sdk::warn!("watched episodes: {e}");
                Vec::new()
            }
        };
        let show_library = host::setting("library").map(|v| v != "false").unwrap_or(true);
        let saved: Vec<Saved> = match client.rpc("sync_pull_library", serde_json::json!({ "p_profile_id": profile, "p_limit": LIBRARY_ITEMS, "p_offset": 0 })) {
            Ok(saved) => saved,
            // The positions are worth more than a library that did not come.
            Err(e) => {
                sdk::warn!("library: {e}");
                Vec::new()
            }
        };
        let by_id: HashMap<&str, &Saved> = saved.iter().map(|s| (s.content_id.as_str(), s)).collect();

        let mut meta = MetaCache::load();
        let mut lookups = 0;
        for p in &watching {
            let episode = p.episode();
            let known_from_library = by_id.contains_key(p.content_id.as_str());
            if lookups < LOOKUPS && meta.needs(&p.content_id, episode, known_from_library) {
                lookups += meta.fetch(&p.content_id, &p.content_type, episode, tmdb_key.as_deref());
            }
            items.push(resume_item(p, by_id.get(p.content_id.as_str()).copied(), meta.get(&p.content_id), &package));
        }

        let finished = last_finished(&watched, &watching);
        let mut up_next = 0;
        for w in &finished {
            let after = (w.season.unwrap_or(0) as u32, w.episode.unwrap_or(0) as u32);
            if lookups < LOOKUPS && meta.needs_next(&w.content_id, after) {
                lookups += meta.fetch_next(w, after, ctx.now_ms, tmdb_key.as_deref());
            }
            if let Some(item) = up_next_item(w, by_id.get(w.content_id.as_str()).copied(), meta.get(&w.content_id), &package) {
                items.push(item);
                up_next += 1;
            }
        }
        meta.keep(watching.iter().map(|p| p.content_id.as_str()).chain(finished.iter().map(|w| w.content_id.as_str())));
        meta.save();

        let mut listed = 0;
        if show_library {
            let count = saved.len() as i32;
            for (i, s) in saved.iter().enumerate() {
                items.push(library_item(s, count - i as i32, ctx.now_ms, &package));
                listed += 1;
            }
        }
        if progress.is_empty() && watched.is_empty() {
            sdk::warn!("No positions for profile {profile} on Nuvio's sync. A profile that follows Trakt, Simkl or MDBList for its progress keeps none here.");
        }
        sdk::dedup_ids(&mut items);
        sdk::info!("{} positions, {} to continue, {up_next} up next, {listed} in the library", progress.len(), watching.len());
        Ok(items)
    }

    fn search(query: &str, limit: u32) -> Result<Vec<Item>, String> {
        let package = app_package();
        cinemeta_search(query, limit as usize, &|m| open(&m.kind, &m.id, &m.name, &package))
    }
}

sdk::export_plugin!(Nuvio);

fn app_package() -> String {
    if host::setting("play_store").map(|v| v == "true").unwrap_or(false) { PLAY_PACKAGE.into() } else { GITHUB_PACKAGE.into() }
}

#[derive(Deserialize, Default)]
struct Profile {
    #[serde(default)]
    profile_index: i64,
    #[serde(default)]
    name: String,
}

/// The profile to follow: the first unless a name, or a number, says otherwise. Nuvio hands
/// out profile numbers as slots, so a name is what a person knows.
fn resolve_profile(client: &mut Client) -> Result<i64, String> {
    let Some(wanted) = host::setting("profile").map(|v| v.trim().to_string()).filter(|v| !v.is_empty()) else {
        return Ok(1);
    };
    if let Ok(n) = wanted.parse::<i64>()
        && n >= 1
    {
        return Ok(n);
    }
    let profiles: Vec<Profile> = client.rpc("sync_pull_profiles", serde_json::json!({}))?;
    match profiles.iter().find(|p| p.name.trim().eq_ignore_ascii_case(&wanted)) {
        Some(p) => Ok(p.profile_index),
        None => {
            let names: Vec<&str> = profiles.iter().map(|p| p.name.trim()).filter(|n| !n.is_empty()).collect();
            Err(if names.is_empty() {
                format!("Nuvio has no profile called {wanted}. Leave the field empty for the first profile.")
            } else {
                format!("Nuvio has no profile called {wanted}. Besides the first, it has: {}.", names.join(", "))
            })
        }
    }
}

// ---------------------------------------------------------------------------
// The backend: a Supabase project found through its discovery document
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct Discovery {
    #[serde(default)]
    service: String,
    #[serde(default)]
    backend_url: String,
    #[serde(default)]
    publishable_key: String,
    #[serde(default)]
    capabilities: Capabilities,
}

#[derive(Deserialize, Default)]
struct Capabilities {
    #[serde(default)]
    email_password_auth: bool,
}

#[derive(Serialize, Deserialize, Clone)]
struct Backend {
    /// The address the user gave, which the saved key belongs to.
    server: String,
    url: String,
    key: String,
}

#[derive(Serialize, Deserialize, Clone)]
struct Session {
    access: String,
    refresh: String,
    expires_ms: i64,
    /// Whose session this is: server and email, so a change of either signs in afresh.
    owner: String,
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
struct AuthError {
    #[serde(default)]
    msg: Option<String>,
    #[serde(default)]
    error_description: Option<String>,
    #[serde(default)]
    error: Option<String>,
}

#[derive(Deserialize, Default)]
struct RpcError {
    #[serde(default)]
    message: Option<String>,
    #[serde(default)]
    code: Option<String>,
}

struct Client {
    backend: Backend,
    session: Session,
    email: String,
    now_ms: i64,
}

/// The server setting as an address, or Nuvio's own when it is empty.
fn server_setting() -> String {
    host::setting("server").map(|s| sdk::server_address(&s)).filter(|s| s != "http://").unwrap_or_else(|| OFFICIAL.to_string())
}

impl Client {
    fn connect(now_ms: i64) -> Result<Client, String> {
        let email = host::required_setting("email", "Email")?.trim().to_string();
        let server = server_setting();
        let backend = backend_for(&server)?;
        let owner = format!("{}\n{email}", backend.url);
        let saved: Option<Session> = host::kv_get(SESSION_KEY).and_then(|v| serde_json::from_slice(&v).ok()).filter(|s: &Session| s.owner == owner);
        let session = match saved {
            Some(s) if s.expires_ms - now_ms > 2 * MINUTE_MS => s,
            Some(s) => match renew(&backend, &s, now_ms) {
                Ok(s) => s,
                Err(e) => {
                    sdk::warn!("could not renew the sign-in ({e}); signing in again");
                    sign_in(&backend, &email, now_ms)?
                }
            },
            None => sign_in(&backend, &email, now_ms)?,
        };
        Ok(Client { backend, session, email, now_ms })
    }

    /// Calls one of the backend's sync functions. A session the backend no longer accepts is
    /// renewed once, then made again.
    fn rpc<T: serde::de::DeserializeOwned>(&mut self, name: &str, params: serde_json::Value) -> Result<T, String> {
        for attempt in 0..2 {
            let req = HttpRequest {
                method: "POST".into(),
                url: format!("{}/rest/v1/rpc/{name}", self.backend.url),
                headers: [
                    ("apikey".to_string(), self.backend.key.clone()),
                    ("Authorization".to_string(), format!("Bearer {}", self.session.access)),
                    ("Content-Type".to_string(), "application/json".to_string()),
                    ("Accept".to_string(), "application/json".to_string()),
                ]
                .into_iter()
                .collect(),
                body: Some(params.to_string()),
                timeout_ms: 0,
            };
            let r = fetch(&self.backend, &req)?;
            match r.status {
                401 | 403 if attempt == 0 => {
                    let e: RpcError = serde_json::from_str(&r.body).unwrap_or_default();
                    sdk::warn!("{name}: HTTP {} {} {}; signing in again", r.status, e.code.unwrap_or_default(), e.message.unwrap_or_default());
                    self.session = match renew(&self.backend, &self.session, self.now_ms) {
                        Ok(s) => s,
                        Err(_) => sign_in(&self.backend, &self.email, self.now_ms)?,
                    };
                }
                s if !(200..300).contains(&s) => {
                    let e: RpcError = serde_json::from_str(&r.body).unwrap_or_default();
                    sdk::warn!("{name}: HTTP {s} {} {}", e.code.unwrap_or_default(), e.message.unwrap_or_default());
                    return Err(if s == 404 { sdk::wrong_server("Nuvio", &self.backend.url) } else { error_status(&self.backend, s) });
                }
                _ => {
                    return serde_json::from_str(&r.body).map_err(|e| {
                        sdk::warn!("unexpected answer for {name}: {e}");
                        sdk::unreadable("Nuvio", &self.backend.url)
                    });
                }
            }
        }
        Err(REJECTED.into())
    }
}

fn is_official(backend: &Backend) -> bool {
    backend.server == OFFICIAL
}

/// A fetch from the backend, worded for Nuvio's own service or for a server the user named.
fn fetch(backend: &Backend, req: &HttpRequest) -> Result<lounge_plugin_sdk::HttpResponse, String> {
    if is_official(backend) { sdk::fetch_online("Nuvio", req) } else { sdk::fetch_from("Nuvio", &backend.url, req) }
}

fn error_status(backend: &Backend, status: u16) -> String {
    if is_official(backend) { sdk::online_error("Nuvio", status) } else { sdk::answered_error("Nuvio", &backend.url, status) }
}

/// The backend behind `server`: its address and public key, from the discovery document every
/// Nuvio server publishes, kept until the server setting changes.
fn backend_for(server: &str) -> Result<Backend, String> {
    if let Some(b) = host::kv_get(BACKEND_KEY).and_then(|v| serde_json::from_slice::<Backend>(&v).ok()).filter(|b| b.server == server) {
        return Ok(b);
    }
    let probe = Backend { server: server.to_string(), url: server.to_string(), key: String::new() };
    let r = fetch(&probe, &HttpRequest::get(format!("{server}/.well-known/nuvio")).header("Accept", "application/json"))?;
    if !r.ok() {
        sdk::warn!("HTTP {} for the discovery document", r.status);
        return Err(sdk::wrong_server("Nuvio", server));
    }
    let d: Discovery = serde_json::from_str(&r.body).map_err(|e| {
        sdk::warn!("unexpected discovery document: {e}");
        sdk::wrong_server("Nuvio", server)
    })?;
    if !d.service.eq_ignore_ascii_case("nuvio") || d.publishable_key.trim().is_empty() {
        return Err(sdk::wrong_server("Nuvio", server));
    }
    if !d.capabilities.email_password_auth {
        return Err("This Nuvio server doesn't allow signing in with an email and password, which is the only way this plugin can.".into());
    }
    let url = sdk::server_address(&d.backend_url);
    let url = if url == "http://" { server.to_string() } else { url };
    let backend = Backend { server: server.to_string(), url, key: d.publishable_key.trim().to_string() };
    host::kv_set(BACKEND_KEY, serde_json::to_string(&backend).unwrap_or_default().as_bytes());
    Ok(backend)
}

fn token_request(backend: &Backend, grant: &str, body: serde_json::Value) -> HttpRequest {
    HttpRequest {
        method: "POST".into(),
        url: format!("{}/auth/v1/token?grant_type={grant}", backend.url),
        headers: [("apikey".to_string(), backend.key.clone()), ("Content-Type".to_string(), "application/json".to_string()), ("Accept".to_string(), "application/json".to_string())].into_iter().collect(),
        body: Some(body.to_string()),
        timeout_ms: 0,
    }
}

fn save_session(backend: &Backend, email: &str, t: TokenAnswer, now_ms: i64) -> Session {
    let lifetime_ms = if t.expires_in > 0 { t.expires_in * 1000 } else { 3_600_000 };
    let session = Session { access: t.access_token, refresh: t.refresh_token, expires_ms: now_ms + lifetime_ms, owner: format!("{}\n{email}", backend.url) };
    host::kv_set(SESSION_KEY, serde_json::to_string(&session).unwrap_or_default().as_bytes());
    session
}

fn sign_in(backend: &Backend, email: &str, now_ms: i64) -> Result<Session, String> {
    let password = host::secret("password").filter(|p| !p.is_empty()).ok_or("Password is not set")?;
    let r = fetch(backend, &token_request(backend, "password", serde_json::json!({ "email": email, "password": password })))?;
    match r.status {
        200..=299 => {}
        // The server's own key was refused, not the person's: it is looked up again next time.
        401 => {
            host::kv_delete(BACKEND_KEY);
            sdk::warn!("the server's key was refused: HTTP 401 {}", r.body.chars().take(200).collect::<String>());
            return Err("The Nuvio server turned the sign-in away. Lounge checks the server's settings again on the next refresh.".into());
        }
        400 | 403 | 422 => {
            let e: AuthError = serde_json::from_str(&r.body).unwrap_or_default();
            sdk::warn!("sign-in refused: HTTP {} {}", r.status, e.msg.or(e.error_description).or(e.error).unwrap_or_default());
            return Err(REJECTED.into());
        }
        s => {
            sdk::warn!("HTTP {s} signing in");
            return Err(error_status(backend, s));
        }
    }
    let t: TokenAnswer = serde_json::from_str(&r.body).map_err(|e| {
        sdk::warn!("unexpected sign-in answer: {e}");
        sdk::unreadable("Nuvio", &backend.url)
    })?;
    Ok(save_session(backend, email, t, now_ms))
}

/// A fresh access token from the session's refresh token.
fn renew(backend: &Backend, s: &Session, now_ms: i64) -> Result<Session, String> {
    if s.refresh.is_empty() {
        return Err("no refresh token".into());
    }
    let r = fetch(backend, &token_request(backend, "refresh_token", serde_json::json!({ "refresh_token": s.refresh })))?;
    if !r.ok() {
        return Err(format!("HTTP {}", r.status));
    }
    let t: TokenAnswer = serde_json::from_str(&r.body).map_err(|e| e.to_string())?;
    let email = s.owner.split_once('\n').map(|(_, e)| e).unwrap_or_default();
    Ok(save_session(backend, email, t, now_ms))
}

// ---------------------------------------------------------------------------
// What the backend keeps
// ---------------------------------------------------------------------------

#[derive(Deserialize, Default, Clone, Debug)]
pub struct Progress {
    #[serde(default)]
    pub content_id: String,
    #[serde(default)]
    pub content_type: String,
    #[serde(default)]
    pub video_id: String,
    #[serde(default)]
    pub season: Option<i64>,
    #[serde(default)]
    pub episode: Option<i64>,
    #[serde(default)]
    pub position: i64,
    #[serde(default)]
    pub duration: i64,
    #[serde(default)]
    pub last_watched: i64,
}

/// Nuvio calls a show `series` or `tv`.
fn is_show(content_type: &str) -> bool {
    content_type == "series" || content_type == "tv"
}

impl Progress {
    fn is_series(&self) -> bool {
        is_show(&self.content_type)
    }

    fn progress(&self) -> Option<f32> {
        (self.duration > 1 && self.position > 0).then(|| (self.position as f32 / self.duration as f32).clamp(0.0, 1.0))
    }

    /// Nuvio's rule for a title still being watched: any position short of finished, with a
    /// duration or without one.
    fn in_progress(&self) -> bool {
        !self.content_id.is_empty() && self.position > 0 && self.progress().is_none_or(|p| p < FINISHED)
    }

    fn episode(&self) -> Option<(u32, u32)> {
        if !self.is_series() {
            return None;
        }
        match (self.season, self.episode) {
            (Some(s), Some(e)) if s >= 0 && e >= 0 => Some((s as u32, e as u32)),
            _ => episode_of(&self.video_id),
        }
    }
}

#[derive(Deserialize, Default, Clone, Debug)]
pub struct Watched {
    #[serde(default)]
    pub content_id: String,
    #[serde(default)]
    pub content_type: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub season: Option<i64>,
    #[serde(default)]
    pub episode: Option<i64>,
    #[serde(default)]
    pub watched_at: i64,
}

#[derive(Deserialize, Default, Clone, Debug)]
pub struct Saved {
    #[serde(default)]
    pub content_id: String,
    #[serde(default)]
    pub content_type: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub poster: Option<String>,
    #[serde(default)]
    pub background: Option<String>,
    #[serde(default)]
    pub release_info: Option<String>,
    #[serde(default)]
    pub added_at: i64,
}

/// The newest position of each title still being watched, most recent first.
fn continue_watching(progress: &[Progress]) -> Vec<&Progress> {
    let mut newest: HashMap<&str, &Progress> = HashMap::new();
    for p in progress.iter().filter(|p| p.in_progress()) {
        newest.entry(p.content_id.as_str()).and_modify(|n| if p.last_watched > n.last_watched { *n = p }).or_insert(p);
    }
    let mut watching: Vec<&Progress> = newest.into_values().collect();
    watching.sort_by_key(|p| std::cmp::Reverse((p.last_watched, p.content_id.clone())));
    watching.truncate(RESUME_ITEMS);
    watching
}

/// The last episode finished of each show not part-way through another, most recent first,
/// as Nuvio's own home works out what is up next. Specials never lead anywhere.
fn last_finished<'a>(watched: &'a [Watched], watching: &[&Progress]) -> Vec<&'a Watched> {
    let mut latest: HashMap<&str, &Watched> = HashMap::new();
    for w in watched.iter().filter(|w| !w.content_id.is_empty() && is_show(&w.content_type) && w.season.is_some_and(|s| s > 0) && w.episode.is_some_and(|e| e > 0)) {
        if watching.iter().any(|p| p.content_id == w.content_id) {
            continue;
        }
        latest.entry(w.content_id.as_str()).and_modify(|l| if (w.watched_at, w.season, w.episode) > (l.watched_at, l.season, l.episode) { *l = w }).or_insert(w);
    }
    let mut finished: Vec<&Watched> = latest.into_values().collect();
    finished.sort_by_key(|w| std::cmp::Reverse((w.watched_at, w.content_id.clone())));
    finished.truncate(UP_NEXT_SHOWS);
    finished
}

/// The season and episode in a Stremio-style video id such as `tt0903747:2:4`.
fn episode_of(video_id: &str) -> Option<(u32, u32)> {
    let mut parts = video_id.rsplit(':');
    let episode = parts.next()?.parse().ok()?;
    let season = parts.next()?.parse().ok()?;
    parts.next()?;
    Some((season, episode))
}

// ---------------------------------------------------------------------------
// Tiles
// ---------------------------------------------------------------------------

/// What the library and the lookups together know of a title: its name and pictures.
fn title_of(id: &str, saved: Option<&Saved>, known: Option<&KnownMeta>) -> (String, Option<String>, Option<String>) {
    let name = saved.map(|s| s.name.clone()).filter(|n| !n.is_empty()).or_else(|| known.and_then(|m| m.name.clone())).unwrap_or_else(|| id.to_string());
    let poster = saved.and_then(|s| picture(&s.poster)).or_else(|| known.and_then(|m| m.poster.clone()));
    let background = saved.and_then(|s| picture(&s.background)).or_else(|| known.and_then(|m| m.background.clone()));
    (name, poster, background)
}

fn resume_item(p: &Progress, saved: Option<&Saved>, known: Option<&KnownMeta>, package: &str) -> Item {
    let episode = p.episode();
    let video = episode.and_then(|(s, e)| known.and_then(|m| m.videos.get(&episode_key(s, e))));
    let (title, poster, background) = title_of(&p.content_id, saved, known);
    Item {
        id: format!("resume:{}", p.content_id),
        kind: ItemKind::Resume,
        title: title.clone(),
        subtitle: video.map(|v| v.title.clone()).filter(|t| !t.is_empty()),
        season: episode.map(|(s, _)| s.to_string()),
        episode: episode.map(|(_, e)| e.to_string()),
        duration_ms: (p.duration > 1).then_some(p.duration),
        position_ms: Some(p.position),
        progress: p.progress(),
        last_used_ms: (p.last_watched > 0).then_some(p.last_watched),
        poster,
        thumbnail: video.and_then(|v| v.thumbnail.clone()).or(background),
        action: open(&p.content_type, &p.content_id, &title, package),
        ..Default::default()
    }
}

/// The episode after the one last finished, once a lookup has named it.
fn up_next_item(w: &Watched, saved: Option<&Saved>, known: Option<&KnownMeta>, package: &str) -> Option<Item> {
    let after = episode_key(w.season? as u32, w.episode? as u32);
    let next = known?.next.as_ref().filter(|n| n.after == after)?.episode.as_ref()?;
    let (title, poster, background) = title_of(&w.content_id, saved, known);
    let title = if title == w.content_id && !w.title.is_empty() { w.title.clone() } else { title };
    Some(Item {
        id: format!("next:{}", w.content_id),
        kind: ItemKind::Resume,
        title: title.clone(),
        subtitle: Some(next.title.clone()).filter(|t| !t.is_empty()),
        season: Some(next.season.to_string()),
        episode: Some(next.episode.to_string()),
        last_used_ms: (w.watched_at > 0).then_some(w.watched_at),
        poster,
        thumbnail: next.thumbnail.clone().or(background),
        action: open(&w.content_type, &w.content_id, &title, package),
        ..Default::default()
    })
}

fn library_item(s: &Saved, weight: i32, now_ms: i64, package: &str) -> Item {
    let added_ms = (s.added_at > 0).then_some(s.added_at);
    Item {
        id: format!("lib:{}", s.content_id),
        kind: ItemKind::Program,
        title: s.name.clone(),
        subtitle: s.release_info.clone().filter(|r| !r.is_empty()),
        added_ms,
        is_new: sdk::new_this_week(added_ms, now_ms),
        poster: picture(&s.poster),
        thumbnail: picture(&s.background),
        row: Some("library".into()),
        weight,
        action: open(&s.content_type, &s.content_id, &s.name, package),
        ..Default::default()
    }
}

fn picture(url: &Option<String>) -> Option<String> {
    url.as_deref().map(str::trim).filter(|u| u.starts_with("http://") || u.starts_with("https://")).map(larger_metahub_poster)
}

/// Opens the title's page in Nuvio, which reads `nuvio://detail/{type}/{id}` with an IMDb or
/// `tmdb:` id. Only Nuvio answers the scheme, so the app's name is kept for the fallback.
fn open(kind: &str, id: &str, title: &str, package: &str) -> Action {
    Action {
        view: Some(format!("nuvio://detail/{}/{}", encode_segment(kind), encode_segment(id))),
        fallback_launch: Some(package.to_string()),
        title: Some(title.to_string()),
        ..Default::default()
    }
}

/// Percent-encodes a path segment, leaving the colon of a `tmdb:` id readable.
fn encode_segment(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b':' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Nuvio keeps the small metahub poster; the medium one suits a TV.
fn larger_metahub_poster(url: &str) -> String {
    match url.strip_prefix("https://images.metahub.space/poster/small/") {
        Some(rest) => format!("https://images.metahub.space/poster/medium/{rest}"),
        None => url.to_string(),
    }
}

// ---------------------------------------------------------------------------
// Names and pictures: Cinemeta for IMDb ids, TMDB for the rest
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct MetaAnswer {
    meta: Option<CinemetaMeta>,
}

#[derive(Deserialize, Default)]
struct CinemetaMeta {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    poster: Option<String>,
    #[serde(default)]
    background: Option<String>,
    #[serde(default)]
    videos: Vec<CinemetaVideo>,
}

#[derive(Deserialize, Default, Clone)]
struct CinemetaVideo {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    season: Option<u32>,
    #[serde(default)]
    episode: Option<u32>,
    #[serde(default)]
    thumbnail: Option<String>,
    #[serde(default)]
    released: Option<String>,
}

impl CinemetaVideo {
    /// Cinemeta names an episode in `name`; other add-ons use `title`.
    fn title(&self) -> String {
        self.name.clone().or_else(|| self.title.clone()).unwrap_or_default()
    }

    fn number(&self) -> Option<(u32, u32)> {
        Some((self.season?, self.episode?))
    }

    /// Whether the episode is out by `now_ms`; one without a date is taken to be.
    fn out_by(&self, now_ms: i64) -> bool {
        self.released.as_deref().and_then(sdk::parse_iso8601).is_none_or(|r| r <= now_ms)
    }
}

/// What a Cinemeta lookup gave: the title's details, nothing for a title it has never heard
/// of, or an error for an answer that may be better next time.
fn cinemeta_meta(content_type: &str, id: &str) -> Result<Option<CinemetaMeta>, String> {
    let kind = if is_show(content_type) { "series" } else { "movie" };
    let url = format!("{CINEMETA}/meta/{kind}/{}.json", encode_segment(id));
    let r = sdk::fetch_online("Cinemeta", &HttpRequest::get(&url))?;
    match r.status {
        404 => Ok(None),
        s if !(200..300).contains(&s) => Err(format!("HTTP {s}")),
        _ => serde_json::from_str::<MetaAnswer>(&r.body).map(|a| a.meta).map_err(|e| e.to_string()),
    }
}

#[derive(Deserialize, Default)]
struct TmdbTitle {
    #[serde(default, alias = "name")]
    title: Option<String>,
    #[serde(default)]
    poster_path: Option<String>,
    #[serde(default)]
    backdrop_path: Option<String>,
    #[serde(default)]
    seasons: Vec<TmdbSeason>,
    #[serde(default)]
    last_episode_to_air: Option<TmdbAired>,
}

#[derive(Deserialize, Default)]
struct TmdbSeason {
    #[serde(default)]
    season_number: u32,
    #[serde(default)]
    episode_count: u32,
}

#[derive(Deserialize, Default)]
struct TmdbAired {
    #[serde(default)]
    season_number: u32,
    #[serde(default)]
    episode_number: u32,
}

#[derive(Deserialize, Default)]
struct TmdbEpisode {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    still_path: Option<String>,
}

impl TmdbTitle {
    /// The episode after `after` that has aired, by TMDB's season lengths.
    fn next_after(&self, after: (u32, u32)) -> Option<(u32, u32)> {
        let (s, e) = after;
        let mut seasons: Vec<&TmdbSeason> = self.seasons.iter().filter(|x| x.season_number > 0 && x.episode_count > 0).collect();
        seasons.sort_by_key(|x| x.season_number);
        let next = match seasons.iter().find(|x| x.season_number == s) {
            Some(this) if e < this.episode_count => (s, e + 1),
            _ => (seasons.iter().find(|x| x.season_number > s)?.season_number, 1),
        };
        match &self.last_episode_to_air {
            Some(last) if next > (last.season_number, last.episode_number) => None,
            _ => Some(next),
        }
    }
}

/// What a TMDB lookup gave, as [`cinemeta_meta`] does. The key stays out of the log.
fn tmdb_get<T: serde::de::DeserializeOwned + Default>(path: &str, key: &str) -> Result<Option<T>, String> {
    let r = sdk::fetch_online("TMDB", &HttpRequest::get(format!("{TMDB}/{path}?api_key={}", encode_segment(key))))?;
    match r.status {
        404 => Ok(None),
        401 => Err("the TMDB API key was refused".into()),
        s if !(200..300).contains(&s) => Err(format!("HTTP {s}")),
        _ => serde_json::from_str::<T>(&r.body).map(Some).map_err(|e| e.to_string()),
    }
}

#[derive(Deserialize)]
struct CatalogAnswer {
    #[serde(default)]
    metas: Vec<CatalogMeta>,
}

#[derive(Deserialize, Default, Clone)]
pub struct CatalogMeta {
    #[serde(default)]
    pub id: String,
    #[serde(rename = "type", default)]
    pub kind: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub poster: Option<String>,
    #[serde(default)]
    pub background: Option<String>,
    #[serde(rename = "releaseInfo", default)]
    pub release_info: Option<String>,
}

#[derive(Serialize, Deserialize, Default, Clone)]
struct KnownMeta {
    name: Option<String>,
    poster: Option<String>,
    background: Option<String>,
    /// The episode last asked about, by season and episode; a show is only ever at one.
    #[serde(default)]
    videos: BTreeMap<String, KnownVideo>,
    /// What follows the episode last finished, once asked.
    #[serde(default)]
    next: Option<NextEpisode>,
}

#[derive(Serialize, Deserialize, Default, Clone)]
struct KnownVideo {
    title: String,
    thumbnail: Option<String>,
}

#[derive(Serialize, Deserialize, Default, Clone)]
struct NextEpisode {
    /// The finished episode this answers for, as `season:episode`.
    after: String,
    /// None when the show has nothing out past that episode.
    episode: Option<NextKnown>,
}

#[derive(Serialize, Deserialize, Default, Clone)]
struct NextKnown {
    season: u32,
    episode: u32,
    title: String,
    thumbnail: Option<String>,
}

fn episode_key(season: u32, episode: u32) -> String {
    format!("{season}:{episode}")
}

/// The kind of id Nuvio has for a title, which decides who is asked about it.
enum IdKind<'a> {
    Imdb,
    Tmdb(&'a str),
    Unknown,
}

fn id_kind(id: &str) -> IdKind<'_> {
    if id.starts_with("tt") {
        IdKind::Imdb
    } else if let Some(num) = id.strip_prefix("tmdb:") {
        IdKind::Tmdb(num)
    } else {
        IdKind::Unknown
    }
}

/// What Cinemeta and TMDB have said about the titles being watched, kept between refreshes so
/// each is asked about once, and once more for each new episode. Only answers are kept, so a
/// lookup that failed is tried again next time.
#[derive(Default)]
struct MetaCache {
    known: HashMap<String, KnownMeta>,
}

impl MetaCache {
    fn load() -> MetaCache {
        MetaCache { known: host::kv_get(META_KEY).and_then(|v| serde_json::from_slice(&v).ok()).unwrap_or_default() }
    }

    fn save(&self) {
        match serde_json::to_vec(&self.known) {
            Ok(json) if json.len() <= CACHE_BYTES => host::kv_set(META_KEY, &json),
            Ok(json) => {
                sdk::warn!("{META_KEY}: {} bytes is too much to keep; starting over", json.len());
                host::kv_delete(META_KEY);
            }
            Err(_) => {}
        }
    }

    fn get(&self, id: &str) -> Option<&KnownMeta> {
        self.known.get(id)
    }

    /// Whether a lookup is due: a title not seen yet and not in the library, or an episode of
    /// a show not seen yet. The library has the title's name and pictures, never an episode's.
    fn needs(&self, id: &str, episode: Option<(u32, u32)>, in_library: bool) -> bool {
        match (self.known.get(id), episode) {
            (None, None) => !in_library,
            (None, Some(_)) => true,
            (Some(m), Some((s, e))) => !m.videos.contains_key(&episode_key(s, e)),
            (Some(_), None) => false,
        }
    }

    /// Whether what follows `after` is still to be asked.
    fn needs_next(&self, id: &str, after: (u32, u32)) -> bool {
        !self.known.get(id).and_then(|m| m.next.as_ref()).is_some_and(|n| n.after == episode_key(after.0, after.1))
    }

    /// Asks about one title, and about its episode when there is one; returns how many fetches
    /// that took.
    fn fetch(&mut self, id: &str, content_type: &str, episode: Option<(u32, u32)>, tmdb_key: Option<&str>) -> usize {
        match id_kind(id) {
            IdKind::Imdb => match cinemeta_meta(content_type, id) {
                Ok(meta) => {
                    let meta = meta.unwrap_or_default();
                    let entry = self.known.entry(id.to_string()).or_default();
                    entry.take_cinemeta(&meta);
                    if let Some((s, e)) = episode {
                        let video = meta.videos.iter().find(|v| v.number() == Some((s, e))).map(|v| KnownVideo { title: v.title(), thumbnail: v.thumbnail.clone() }).unwrap_or_default();
                        entry.videos.clear();
                        entry.videos.insert(episode_key(s, e), video);
                    }
                    1
                }
                Err(e) => {
                    sdk::warn!("Cinemeta did not answer for {id}: {e}");
                    1
                }
            },
            IdKind::Tmdb(num) => {
                let Some(key) = tmdb_key else { return 0 };
                let kind = if is_show(content_type) { "tv" } else { "movie" };
                let title: TmdbTitle = match tmdb_get(&format!("{kind}/{}", encode_segment(num)), key) {
                    Ok(t) => t.unwrap_or_default(),
                    Err(e) => {
                        sdk::warn!("TMDB did not answer for {id}: {e}");
                        return 1;
                    }
                };
                let entry = self.known.entry(id.to_string()).or_default();
                entry.take_tmdb(&title);
                let Some((s, e)) = episode else { return 1 };
                match tmdb_get::<TmdbEpisode>(&format!("tv/{}/season/{s}/episode/{e}", encode_segment(num)), key) {
                    Ok(ep) => {
                        let ep = ep.unwrap_or_default();
                        entry.videos.clear();
                        entry.videos.insert(episode_key(s, e), KnownVideo { title: ep.name.unwrap_or_default(), thumbnail: ep.still_path.map(|p| format!("{TMDB_IMAGES}/w780{p}")) });
                    }
                    Err(err) => sdk::warn!("TMDB has no episode {s}:{e} for {id}: {err}"),
                }
                2
            }
            // Nothing this plugin can ask knows the id; it is noted, so it is not asked again.
            IdKind::Unknown => {
                self.known.entry(id.to_string()).or_default();
                0
            }
        }
    }

    /// Asks what follows the episode `after` of a show, and returns the fetches that took.
    fn fetch_next(&mut self, w: &Watched, after: (u32, u32), now_ms: i64, tmdb_key: Option<&str>) -> usize {
        let id = w.content_id.as_str();
        match id_kind(id) {
            IdKind::Imdb => match cinemeta_meta(&w.content_type, id) {
                Ok(meta) => {
                    let meta = meta.unwrap_or_default();
                    let mut later: Vec<&CinemetaVideo> = meta.videos.iter().filter(|v| v.number().is_some_and(|(s, e)| s > 0 && (s, e) > after) && v.out_by(now_ms)).collect();
                    later.sort_by_key(|v| v.number());
                    let next = later.first().map(|v| NextKnown { season: v.season.unwrap_or_default(), episode: v.episode.unwrap_or_default(), title: v.title(), thumbnail: v.thumbnail.clone() });
                    let entry = self.known.entry(id.to_string()).or_default();
                    entry.take_cinemeta(&meta);
                    entry.next = Some(NextEpisode { after: episode_key(after.0, after.1), episode: next });
                    1
                }
                Err(e) => {
                    sdk::warn!("Cinemeta did not answer for {id}: {e}");
                    1
                }
            },
            IdKind::Tmdb(num) => {
                let Some(key) = tmdb_key else { return 0 };
                let title: TmdbTitle = match tmdb_get(&format!("tv/{}", encode_segment(num)), key) {
                    Ok(t) => t.unwrap_or_default(),
                    Err(e) => {
                        sdk::warn!("TMDB did not answer for {id}: {e}");
                        return 1;
                    }
                };
                let next = title.next_after(after);
                let entry = self.known.entry(id.to_string()).or_default();
                entry.take_tmdb(&title);
                let Some((s, e)) = next else {
                    entry.next = Some(NextEpisode { after: episode_key(after.0, after.1), episode: None });
                    return 1;
                };
                match tmdb_get::<TmdbEpisode>(&format!("tv/{}/season/{s}/episode/{e}", encode_segment(num)), key) {
                    Ok(ep) => {
                        let ep = ep.unwrap_or_default();
                        entry.next = Some(NextEpisode {
                            after: episode_key(after.0, after.1),
                            episode: Some(NextKnown { season: s, episode: e, title: ep.name.unwrap_or_default(), thumbnail: ep.still_path.map(|p| format!("{TMDB_IMAGES}/w780{p}")) }),
                        });
                    }
                    Err(err) => sdk::warn!("TMDB has no episode {s}:{e} for {id}: {err}"),
                }
                2
            }
            IdKind::Unknown => {
                let entry = self.known.entry(id.to_string()).or_default();
                entry.next = Some(NextEpisode { after: episode_key(after.0, after.1), episode: None });
                0
            }
        }
    }

    /// Forgets titles no longer on the home, so the store stays the size of the rows.
    fn keep<'a>(&mut self, ids: impl Iterator<Item = &'a str>) {
        let wanted: Vec<String> = ids.map(str::to_string).collect();
        self.known.retain(|id, _| wanted.iter().any(|w| w == id));
    }
}

impl KnownMeta {
    fn take_cinemeta(&mut self, meta: &CinemetaMeta) {
        self.name = meta.name.clone().filter(|n| !n.is_empty()).or(self.name.take());
        self.poster = meta.poster.as_deref().map(larger_metahub_poster).or(self.poster.take());
        self.background = meta.background.clone().or(self.background.take());
    }

    fn take_tmdb(&mut self, title: &TmdbTitle) {
        self.name = title.title.clone().filter(|n| !n.is_empty()).or(self.name.take());
        self.poster = title.poster_path.as_ref().map(|p| format!("{TMDB_IMAGES}/w500{p}")).or(self.poster.take());
        self.background = title.backdrop_path.as_ref().map(|p| format!("{TMDB_IMAGES}/w1280{p}")).or(self.background.take());
    }
}

/// Movies, then shows, matching `query` in Cinemeta's catalogue, each opened with `open`.
pub fn cinemeta_search(query: &str, limit: usize, open: &dyn Fn(&CatalogMeta) -> Action) -> Result<Vec<Item>, String> {
    let query = query.trim();
    if query.is_empty() {
        return Ok(Vec::new());
    }
    let mut items = Vec::new();
    for kind in ["movie", "series"] {
        let url = format!("{CINEMETA}/catalog/{kind}/top/search={}.json", encode_segment(query));
        let r = sdk::fetch_online("Cinemeta", &HttpRequest::get(&url))?;
        if !r.ok() {
            sdk::warn!("HTTP {} searching {kind}", r.status);
            continue;
        }
        let found: CatalogAnswer = serde_json::from_str(&r.body).map_err(|e| {
            sdk::warn!("unexpected search answer: {e}");
            sdk::unreadable("Cinemeta", CINEMETA)
        })?;
        for m in found.metas.into_iter().filter(|m| !m.id.is_empty()) {
            items.push(Item {
                id: format!("found:{}:{}", m.kind, m.id),
                kind: ItemKind::Program,
                title: m.name.clone(),
                subtitle: m.release_info.clone().filter(|r| !r.is_empty()),
                poster: m.poster.as_deref().map(larger_metahub_poster),
                thumbnail: m.background.clone(),
                action: open(&m),
                ..Default::default()
            });
        }
    }
    rank_matches(&mut items, query);
    items.truncate(limit.max(1));
    Ok(items)
}

/// Cinemeta ranks loosely; a title that is the query, or begins with it, comes first.
fn rank_matches(items: &mut [Item], query: &str) {
    let q = query.to_lowercase();
    items.sort_by_key(|it| {
        let t = it.title.to_lowercase();
        if t == q { 0 } else if t.starts_with(&q) { 1 } else { 2 }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn progress(json: &str) -> Vec<Progress> {
        serde_json::from_str(json).unwrap()
    }

    fn watched(json: &str) -> Vec<Watched> {
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn the_title_asked_for_comes_first() {
        let mut items: Vec<Item> = ["Breaking Bad Wolf", "Bad Breaking", "Breaking Bad", "Breaking Bad: Minisodes"].into_iter().map(|t| Item { title: t.into(), ..Default::default() }).collect();
        rank_matches(&mut items, "Breaking bad");
        let titles: Vec<&str> = items.iter().map(|it| it.title.as_str()).collect();
        assert_eq!(titles, ["Breaking Bad", "Breaking Bad Wolf", "Breaking Bad: Minisodes", "Bad Breaking"]);
    }

    #[test]
    fn only_titles_part_way_through_are_continued_once_each() {
        let rows = progress(
            r#"[{"content_id":"tt1","content_type":"movie","video_id":"tt1","position":600000,"duration":6000000,"last_watched":100},
                {"content_id":"tt2","content_type":"series","video_id":"tt2:1:1","season":1,"episode":1,"position":1700000,"duration":1800000,"last_watched":300},
                {"content_id":"tt3","content_type":"series","video_id":"tt3:1:1","season":1,"episode":1,"position":100000,"duration":1800000,"last_watched":200},
                {"content_id":"tt3","content_type":"series","video_id":"tt3:1:2","season":1,"episode":2,"position":200000,"duration":1800000,"last_watched":400},
                {"content_id":"tt4","content_type":"movie","video_id":"tt4","position":0,"duration":6000000,"last_watched":500},
                {"content_id":"tt5","content_type":"movie","video_id":"tt5","position":1,"duration":1,"last_watched":600},
                {"content_id":"tt6","content_type":"movie","video_id":"tt6","position":120000,"duration":0,"last_watched":50},
                {"content_id":"","content_type":"movie","video_id":"","position":10,"duration":100,"last_watched":700}]"#,
        );
        let watching: Vec<(&str, &str)> = continue_watching(&rows).into_iter().map(|p| (p.content_id.as_str(), p.video_id.as_str())).collect();
        assert_eq!(watching, [("tt5", "tt5"), ("tt3", "tt3:1:2"), ("tt1", "tt1"), ("tt6", "tt6")]);
        let unknown_length = resume_item(&rows[6], None, None, GITHUB_PACKAGE);
        assert_eq!((unknown_length.duration_ms, unknown_length.progress, unknown_length.position_ms), (None, None, Some(120_000)));
    }

    #[test]
    fn a_paused_episode_becomes_a_resume_tile() {
        let p = &progress(r#"[{"content_id":"tt0903747","content_type":"series","video_id":"tt0903747:2:4","season":2,"episode":4,"position":900000,"duration":1800000,"last_watched":1761040800000}]"#)[0];
        let known = KnownMeta {
            name: Some("Breaking Bad".into()),
            poster: Some("https://images.metahub.space/poster/medium/tt0903747/img".into()),
            background: Some("https://images.metahub.space/background/medium/tt0903747/img".into()),
            videos: [("2:4".to_string(), KnownVideo { title: "Down".into(), thumbnail: Some("https://episodes.metahub.space/tt0903747/2/4/w780.jpg".into()) })].into_iter().collect(),
            next: None,
        };
        let item = resume_item(p, None, Some(&known), GITHUB_PACKAGE);
        assert_eq!(item.id, "resume:tt0903747");
        assert_eq!(item.kind, ItemKind::Resume);
        assert_eq!(item.title, "Breaking Bad");
        assert_eq!(item.subtitle.as_deref(), Some("Down"));
        assert_eq!((item.season.as_deref(), item.episode.as_deref()), (Some("2"), Some("4")));
        assert_eq!(item.progress, Some(0.5));
        assert_eq!(item.duration_ms, Some(1_800_000));
        assert_eq!(item.last_used_ms, Some(1_761_040_800_000));
        assert_eq!(item.thumbnail.as_deref(), Some("https://episodes.metahub.space/tt0903747/2/4/w780.jpg"));
        assert_eq!(item.action.view.as_deref(), Some("nuvio://detail/series/tt0903747"));
        assert_eq!(item.action.fallback_launch.as_deref(), Some("com.nuvio.tv"));
    }

    #[test]
    fn the_library_names_a_title_before_anything_else() {
        let p = &progress(r#"[{"content_id":"tmdb:603","content_type":"movie","video_id":"tmdb:603","position":100,"duration":1000,"last_watched":5}]"#)[0];
        let saved: Saved = serde_json::from_str(r#"{"content_id":"tmdb:603","content_type":"movie","name":"The Matrix","poster":"https://images.metahub.space/poster/small/tt0133093/img","background":" https://example.com/bg.jpg ","release_info":"1999","added_at":1760000000000}"#).unwrap();
        let item = resume_item(p, Some(&saved), None, PLAY_PACKAGE);
        assert_eq!(item.title, "The Matrix");
        assert_eq!(item.poster.as_deref(), Some("https://images.metahub.space/poster/medium/tt0133093/img"));
        assert_eq!(item.thumbnail.as_deref(), Some("https://example.com/bg.jpg"));
        assert_eq!(item.season, None);
        assert_eq!(item.action.view.as_deref(), Some("nuvio://detail/movie/tmdb:603"));
        assert_eq!(item.action.fallback_launch.as_deref(), Some("com.nuvio.app"));

        let unnamed = resume_item(p, None, None, PLAY_PACKAGE);
        assert_eq!(unnamed.title, "tmdb:603");
        assert_eq!(unnamed.poster, None);

        let lib = library_item(&saved, 3, 1_760_300_000_000, GITHUB_PACKAGE);
        assert_eq!(lib.id, "lib:tmdb:603");
        assert_eq!(lib.row.as_deref(), Some("library"));
        assert_eq!(lib.subtitle.as_deref(), Some("1999"));
        assert!(lib.is_new);
        assert_eq!(lib.weight, 3);
        assert!(<Nuvio as sdk::Plugin>::manifest().rows.iter().any(|r| r.id == "library"));
    }

    #[test]
    fn episodes_come_from_the_columns_or_the_video_id_for_series_and_tv() {
        let rows = progress(
            r#"[{"content_id":"tt1","content_type":"series","video_id":"tt1:3:7","position":1,"duration":10},
                {"content_id":"tt2","content_type":"tv","video_id":"tt2:3:7","season":1,"episode":2,"position":1,"duration":10},
                {"content_id":"tt3","content_type":"movie","video_id":"tt3:3:7","season":3,"episode":7,"position":1,"duration":10}]"#,
        );
        assert_eq!(rows[0].episode(), Some((3, 7)));
        assert_eq!(rows[1].episode(), Some((1, 2)));
        assert_eq!(rows[2].episode(), None);
        assert_eq!(episode_of("tmdb:1396:5:14"), Some((5, 14)));
        assert_eq!(episode_of("tt1"), None);
    }

    #[test]
    fn the_cache_asks_only_where_the_library_cannot_answer() {
        let mut cache = MetaCache::default();
        assert!(!cache.needs("tt1", None, true));
        assert!(cache.needs("tt1", None, false));
        assert!(cache.needs("tt1", Some((1, 1)), true));
        cache.known.insert("tt1".into(), KnownMeta { videos: [("1:1".to_string(), KnownVideo::default())].into_iter().collect(), ..Default::default() });
        assert!(!cache.needs("tt1", Some((1, 1)), false));
        assert!(cache.needs("tt1", Some((1, 2)), false));
        assert!(!cache.needs("tt1", None, false));
        assert!(cache.needs_next("tt1", (1, 1)));
        cache.known.get_mut("tt1").unwrap().next = Some(NextEpisode { after: "1:1".into(), episode: None });
        assert!(!cache.needs_next("tt1", (1, 1)));
        assert!(cache.needs_next("tt1", (1, 2)));
        cache.known.insert("tt2".into(), KnownMeta::default());
        cache.keep(["tt1"].into_iter());
        assert!(cache.get("tt2").is_none() && cache.get("tt1").is_some());
    }

    #[test]
    fn a_failed_lookup_is_not_remembered_but_an_unknowable_id_is() {
        let mut cache = MetaCache::default();
        // Off the TV every fetch fails; nothing is kept, so it is asked again next time.
        assert_eq!(cache.fetch("tt1", "series", Some((1, 1)), None), 1);
        assert!(cache.get("tt1").is_none());
        assert_eq!(cache.fetch("tmdb:1396", "series", Some((1, 1)), None), 0);
        assert!(cache.get("tmdb:1396").is_none());
        assert_eq!(cache.fetch("tmdb:1396", "series", Some((1, 1)), Some("k")), 1);
        assert!(cache.get("tmdb:1396").is_none());
        assert_eq!(cache.fetch("kitsu:1", "series", Some((1, 1)), None), 0);
        assert!(cache.get("kitsu:1").is_some());
        assert!(!cache.needs("kitsu:1", None, false));
    }

    #[test]
    fn the_episode_after_the_last_finished_is_up_next() {
        let rows = watched(
            r#"[{"content_id":"tt1","content_type":"series","title":"Show","season":1,"episode":2,"watched_at":300},
                {"content_id":"tt1","content_type":"series","title":"Show","season":1,"episode":1,"watched_at":200},
                {"content_id":"tt2","content_type":"series","title":"Paused show","season":2,"episode":1,"watched_at":400},
                {"content_id":"tt3","content_type":"series","title":"Special","season":0,"episode":1,"watched_at":500},
                {"content_id":"tt4","content_type":"movie","title":"Film","watched_at":600},
                {"content_id":"tmdb:5","content_type":"tv","title":"TMDB show","season":3,"episode":3,"watched_at":100}]"#,
        );
        let paused = progress(r#"[{"content_id":"tt2","content_type":"series","video_id":"tt2:2:2","season":2,"episode":2,"position":5,"duration":10,"last_watched":450}]"#);
        let watching = continue_watching(&paused);
        let finished: Vec<&str> = last_finished(&rows, &watching).iter().map(|w| w.content_id.as_str()).collect();
        assert_eq!(finished, ["tt1", "tmdb:5"]);

        let w = &rows[0];
        assert!(up_next_item(w, None, None, GITHUB_PACKAGE).is_none());
        let known = KnownMeta {
            name: Some("Show".into()),
            poster: Some("https://p/1.jpg".into()),
            next: Some(NextEpisode { after: "1:2".into(), episode: Some(NextKnown { season: 1, episode: 3, title: "Third".into(), thumbnail: Some("https://t/3.jpg".into()) }) }),
            ..Default::default()
        };
        let item = up_next_item(w, None, Some(&known), GITHUB_PACKAGE).unwrap();
        assert_eq!(item.id, "next:tt1");
        assert_eq!(item.kind, ItemKind::Resume);
        assert_eq!(item.title, "Show");
        assert_eq!(item.subtitle.as_deref(), Some("Third"));
        assert_eq!((item.season.as_deref(), item.episode.as_deref()), (Some("1"), Some("3")));
        assert_eq!((item.position_ms, item.progress), (None, None));
        assert_eq!(item.last_used_ms, Some(300));
        assert_eq!(item.thumbnail.as_deref(), Some("https://t/3.jpg"));
        let stale = KnownMeta { next: Some(NextEpisode { after: "1:1".into(), episode: Some(NextKnown::default()) }), ..Default::default() };
        assert!(up_next_item(w, None, Some(&stale), GITHUB_PACKAGE).is_none());
        let nothing_more = KnownMeta { next: Some(NextEpisode { after: "1:2".into(), episode: None }), ..Default::default() };
        assert!(up_next_item(w, None, Some(&nothing_more), GITHUB_PACKAGE).is_none());
        let unnamed = KnownMeta { next: known.next.clone(), ..Default::default() };
        assert_eq!(up_next_item(w, None, Some(&unnamed), GITHUB_PACKAGE).unwrap().title, "Show");
    }

    #[test]
    fn cinemeta_names_the_next_episode_out() {
        let meta: MetaAnswer = serde_json::from_str(
            r#"{"meta":{"id":"tt1","name":"Show","videos":[
                {"id":"tt1:1:1","name":"One","season":1,"number":1,"episode":1,"released":"2026-01-01T00:00:00Z"},
                {"id":"tt1:1:3","name":"Three","season":1,"number":3,"episode":3,"released":"2026-03-01T00:00:00Z"},
                {"id":"tt1:1:2","name":"Two","season":1,"number":2,"episode":2,"released":"2026-02-01T00:00:00Z","thumbnail":"t2"},
                {"id":"tt1:2:1","name":"Later","season":2,"number":1,"episode":1,"released":"2027-01-01T00:00:00Z"},
                {"id":"tt1:0:1","name":"Special","season":0,"number":1,"episode":1}]}}"#,
        )
        .unwrap();
        let meta = meta.meta.unwrap();
        let now = sdk::parse_iso8601("2026-06-01T00:00:00Z").unwrap();
        let after = |a: (u32, u32)| {
            let mut later: Vec<&CinemetaVideo> = meta.videos.iter().filter(|v| v.number().is_some_and(|(s, e)| s > 0 && (s, e) > a) && v.out_by(now)).collect();
            later.sort_by_key(|v| v.number());
            later.first().map(|v| (v.number().unwrap(), v.title()))
        };
        assert_eq!(after((1, 1)), Some(((1, 2), "Two".to_string())));
        assert_eq!(after((1, 3)), None);
        assert_eq!(meta.videos[2].thumbnail.as_deref(), Some("t2"));
    }

    #[test]
    fn tmdb_counts_the_next_episode_from_its_season_lengths() {
        let t: TmdbTitle = serde_json::from_str(r#"{"name":"Show","seasons":[{"season_number":0,"episode_count":3},{"season_number":1,"episode_count":2},{"season_number":2,"episode_count":8}],"last_episode_to_air":{"season_number":2,"episode_number":3}}"#).unwrap();
        assert_eq!(t.next_after((1, 1)), Some((1, 2)));
        assert_eq!(t.next_after((1, 2)), Some((2, 1)));
        assert_eq!(t.next_after((2, 3)), None);
        assert_eq!(t.next_after((2, 8)), None);
        assert_eq!(t.title.as_deref(), Some("Show"));
        let open_ended: TmdbTitle = serde_json::from_str(r#"{"title":"Film","seasons":[{"season_number":1,"episode_count":2}]}"#).unwrap();
        assert_eq!(open_ended.next_after((1, 1)), Some((1, 2)));
        assert_eq!(open_ended.next_after((3, 1)), None);
    }

    #[test]
    fn links_keep_ids_readable_and_harmless() {
        assert_eq!(open("series", "tmdb:1396", "x", GITHUB_PACKAGE).view.as_deref(), Some("nuvio://detail/series/tmdb:1396"));
        assert_eq!(encode_segment("a b/c?d#e"), "a%20b%2Fc%3Fd%23e");
        assert_eq!(picture(&Some("javascript:alert(1)".into())), None);
        assert_eq!(picture(&Some("".into())), None);
    }

    #[test]
    fn the_server_setting_falls_back_to_nuvios_own() {
        assert_eq!(server_setting(), OFFICIAL);
        let official = Backend { server: OFFICIAL.into(), url: OFFICIAL.into(), key: "k".into() };
        assert_eq!(error_status(&official, 500), "Nuvio answered with error 500. Try again later.");
        let own = Backend { server: "https://nuvio.example.com".into(), url: "https://nuvio.example.com".into(), key: "k".into() };
        assert_eq!(error_status(&own, 500), "Nuvio at nuvio.example.com answered with error 500. Check the server, then try again.");
        // Off the TV there is no host, so a fetch fails as an unreachable server would.
        assert_eq!(backend_for("https://nuvio.example.com").err().as_deref(), Some("Nuvio didn't answer at nuvio.example.com. Check the server, then try again."));
        assert_eq!(backend_for(OFFICIAL).err().as_deref(), Some("Nuvio didn't answer. Check the TV's internet connection, then try again."));
    }

    #[test]
    fn the_discovery_document_and_tokens_are_read_as_nuvio_sends_them() {
        let d: Discovery = serde_json::from_str(r#"{"version":1,"service":"nuvio","self_hosted":true,"backend_url":"https://api.nuvio.tv","publishable_key":"eyJ","capabilities":{"email_password_auth":true,"tv_login":true,"watch_party":true}}"#).unwrap();
        assert_eq!((d.service.as_str(), d.backend_url.as_str(), d.publishable_key.as_str(), d.capabilities.email_password_auth), ("nuvio", "https://api.nuvio.tv", "eyJ", true));
        let t: TokenAnswer = serde_json::from_str(r#"{"access_token":"a","token_type":"bearer","expires_in":3600,"refresh_token":"r","user":{}}"#).unwrap();
        assert_eq!((t.access_token.as_str(), t.refresh_token.as_str(), t.expires_in), ("a", "r", 3600));
        let e: AuthError = serde_json::from_str(r#"{"code":400,"error_code":"invalid_credentials","msg":"Invalid login credentials"}"#).unwrap();
        assert_eq!(e.msg.as_deref(), Some("Invalid login credentials"));
        let e: AuthError = serde_json::from_str(r#"{"error":"invalid_grant","error_description":"Invalid login credentials"}"#).unwrap();
        assert_eq!(e.error_description.as_deref(), Some("Invalid login credentials"));
        let profiles: Vec<Profile> = serde_json::from_str(r#"[{"id":"x","user_id":"u","profile_index":2,"name":"Kids","avatar_color_hex":"fff"}]"#).unwrap();
        assert_eq!((profiles[0].profile_index, profiles[0].name.as_str()), (2, "Kids"));
    }
}
