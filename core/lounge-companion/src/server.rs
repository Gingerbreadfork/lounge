//! axum on a tokio runtime, on its own thread. Every `/api` route needs a paired token in
//! `Authorization: Bearer …`, checked before a byte of the body is read, except `POST /api/pair`.
//! Pictures, the snapshot socket and the backup load by URL, so those `GET` routes also accept
//! the session cookie `POST /api/session` sets, and no token ever sits in a URL.
//! Snapshots are pushed over `/api/ws` whenever the core says one changed.

use crate::auth::{PairingInfo, TokenStore};
use crate::{CompanionActions, CompanionError, LayoutPatch};
use axum::body::Bytes;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{ConnectInfo, DefaultBodyLimit, Path as AxPath, Query, Request, State};
use axum::http::{header, HeaderMap, Method, StatusCode, Uri};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post, put};
use axum::{Extension, Json, Router};
use rust_embed::RustEmbed;
use serde::Deserialize;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

#[derive(RustEmbed)]
#[folder = "$CARGO_MANIFEST_DIR/../../companion/dist/"]
struct Assets;

struct App {
    actions: Arc<dyn CompanionActions>,
    tokens: Arc<TokenStore>,
    revision: tokio::sync::watch::Sender<u64>,
    /// One upload at a time: a bundle or a backup is held in memory while it is handled.
    uploads: tokio::sync::Semaphore,
}

const SESSION_COOKIE: &str = "lounge_session";
/// Request bodies are small JSON except on the two upload routes.
const BODY_LIMIT: usize = 64 * 1024;
const BUNDLE_LIMIT: usize = 16 * 1024 * 1024;
const BACKUP_LIMIT: usize = 2 * 1024 * 1024;

type Shared = Arc<App>;

/// A running server. Dropping it stops the runtime.
pub struct Companion {
    pub port: u16,
    tokens: Arc<TokenStore>,
    revision: tokio::sync::watch::Sender<u64>,
    counter: AtomicU64,
    shutdown: Option<tokio::sync::oneshot::Sender<()>>,
    thread: Option<std::thread::JoinHandle<()>>,
    bound: Arc<parking_lot::Mutex<Option<Result<u16, String>>>>,
}

impl Companion {
    pub fn start(port: u16, data_dir: &std::path::Path, actions: Arc<dyn CompanionActions>) -> Result<Companion, CompanionError> {
        let tokens = Arc::new(TokenStore::open(data_dir));
        let (revision, _) = tokio::sync::watch::channel(0u64);
        let app = Arc::new(App { actions, tokens: Arc::clone(&tokens), revision: revision.clone(), uploads: tokio::sync::Semaphore::new(1) });
        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
        let bound = Arc::new(parking_lot::Mutex::new(None));
        let bound_for_thread = Arc::clone(&bound);
        let thread = std::thread::Builder::new()
            .name("lounge-companion".into())
            .spawn(move || {
                let rt = match tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build() {
                    Ok(rt) => rt,
                    Err(e) => {
                        *bound_for_thread.lock() = Some(Err(e.to_string()));
                        return;
                    }
                };
                rt.block_on(async move {
                    let listener = match tokio::net::TcpListener::bind(SocketAddr::from(([0, 0, 0, 0], port))).await {
                        Ok(l) => l,
                        Err(e) => {
                            *bound_for_thread.lock() = Some(Err(e.to_string()));
                            return;
                        }
                    };
                    let actual = listener.local_addr().map(|a| a.port()).unwrap_or(port);
                    *bound_for_thread.lock() = Some(Ok(actual));
                    let router = routes(app);
                    let _ = axum::serve(listener, router.into_make_service_with_connect_info::<SocketAddr>())
                        .with_graceful_shutdown(async { let _ = shutdown_rx.await; })
                        .await;
                });
            })?;
        // Wait briefly for the bind result so a port clash is reported to the caller.
        let started = std::time::Instant::now();
        let actual = loop {
            if let Some(r) = bound.lock().clone() {
                break r;
            }
            if started.elapsed() > std::time::Duration::from_secs(5) {
                break Err("timed out starting".to_string());
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        };
        let actual = actual.map_err(CompanionError::Other)?;
        Ok(Companion { port: actual, tokens, revision, counter: AtomicU64::new(0), shutdown: Some(shutdown_tx), thread: Some(thread), bound })
    }

    /// Tell connected phones the snapshot changed.
    pub fn notify(&self) {
        let n = self.counter.fetch_add(1, Ordering::Relaxed) + 1;
        let _ = self.revision.send(n);
    }

    pub fn new_pairing(&self, base_url: &str, now_ms: i64) -> PairingInfo {
        self.tokens.new_pairing(base_url, now_ms)
    }

    pub fn paired_devices(&self) -> usize {
        self.tokens.count()
    }

    pub fn forget_all(&self) {
        self.tokens.revoke_all();
    }
}

impl Drop for Companion {
    fn drop(&mut self) {
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
        let _ = &self.bound;
    }
}

fn routes(app: Shared) -> Router {
    let paired = Router::new()
        .route("/api/session", post(session))
        .route("/api/state", get(state))
        .route("/api/ws", get(ws))
        .route("/api/key", post(key))
        .route("/api/text", post(text))
        .route("/api/open", post(open))
        .route("/api/launch", post(launch))
        .route("/api/search", post(search))
        .route("/api/layout", get(layout).put(set_layout))
        .route("/api/plugins", get(plugins))
        .route("/api/plugins/install", post(install_plugin).layer(DefaultBodyLimit::max(BUNDLE_LIMIT)))
        .route("/api/plugins/install-url", post(install_plugin_url))
        .route("/api/plugins/{id}/settings", put(plugin_setting))
        .route("/api/plugins/{id}/log", get(plugin_log))
        .route("/api/plugins/{id}/{command}", post(plugin_command))
        .route("/api/diagnostics", get(diagnostics))
        .route("/api/devices", get(devices))
        .route("/api/devices/{id}", delete(revoke_device))
        .route("/api/backup", get(backup))
        .route("/api/restore", post(restore).layer(DefaultBodyLimit::max(BACKUP_LIMIT)))
        .route("/api/art", get(art))
        .route_layer(middleware::from_fn_with_state(Arc::clone(&app), require_token));
    Router::new()
        .route("/api/pair", post(pair).layer(DefaultBodyLimit::max(1024)))
        .merge(paired)
        .fallback(get(static_file))
        .layer(DefaultBodyLimit::max(BODY_LIMIT))
        .layer(middleware::from_fn(known_host))
        .with_state(app)
}

// ---------------------------------------------------------------------------

fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

/// The paired token a request carries, found by `require_token`.
#[derive(Clone)]
struct Paired(String);

fn bearer(headers: &HeaderMap) -> Option<String> {
    headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(|s| s.trim().to_string())
}

fn cookie(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .filter_map(|pair| pair.trim().split_once('='))
        .find(|(k, _)| *k == name)
        .map(|(_, v)| v.trim().to_string())
}

/// Routes a page loads by URL, which cannot carry a header; read-only, so a cookie suffices.
fn cookie_route(method: &Method, path: &str) -> bool {
    method == Method::GET && matches!(path, "/api/art" | "/api/ws" | "/api/backup")
}

/// Every paired route: the token is checked here, before the handler reads the body.
async fn require_token(State(app): State<Shared>, mut req: Request, next: Next) -> Response {
    let headers = req.headers();
    let token = bearer(headers).or_else(|| cookie_route(req.method(), req.uri().path()).then(|| cookie(headers, SESSION_COOKIE)).flatten());
    let Some(token) = token else {
        return ApiError(StatusCode::UNAUTHORIZED, "pair this phone with the TV first".into()).into_response();
    };
    if !app.tokens.authenticate(&token, now_ms()) {
        return ApiError(StatusCode::UNAUTHORIZED, "this phone is no longer paired".into()).into_response();
    }
    req.extensions_mut().insert(Paired(token));
    next.run(req).await
}

/// Answer only to names a person would type or scan for a device on their own network, so a web
/// page cannot rebind its own domain to the TV and talk to it.
async fn known_host(req: Request, next: Next) -> Response {
    let host = req.headers().get(header::HOST).and_then(|v| v.to_str().ok()).unwrap_or("");
    if !local_name(host) {
        return (StatusCode::MISDIRECTED_REQUEST, "unknown host").into_response();
    }
    next.run(req).await
}

fn local_name(host_header: &str) -> bool {
    let host = if let Some(rest) = host_header.strip_prefix('[') {
        rest.split(']').next().unwrap_or("")
    } else {
        host_header.rsplit_once(':').map(|(h, _)| h).unwrap_or(host_header)
    };
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    if host.is_empty() {
        return false;
    }
    host.parse::<std::net::IpAddr>().is_ok()
        || !host.contains('.')
        || [".local", ".lan", ".home", ".home.arpa", ".internal", ".localhost"].iter().any(|s| host.ends_with(s))
}

struct ApiError(StatusCode, String);

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(serde_json::json!({ "error": self.1 }))).into_response()
    }
}

fn bad(e: String) -> ApiError {
    ApiError(StatusCode::BAD_REQUEST, e)
}

fn json_raw(s: String) -> Response {
    ([(header::CONTENT_TYPE, "application/json")], s).into_response()
}

#[derive(Deserialize)]
struct PairBody {
    code: String,
}

async fn pair(State(app): State<Shared>, ConnectInfo(peer): ConnectInfo<SocketAddr>, headers: HeaderMap, Json(body): Json<PairBody>) -> Result<Response, ApiError> {
    let name = headers
        .get(header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .map(device_name)
        .unwrap_or_else(|| "Phone".to_string());
    match app.tokens.redeem(body.code.trim(), &name, peer.ip(), now_ms()) {
        Ok(token) => Ok(Json(serde_json::json!({ "token": token, "device": app.actions.device_info().name })).into_response()),
        Err(crate::auth::RedeemError::LockedOut) => {
            tracing::warn!("companion: pairing locked out for {} after repeated wrong codes", peer.ip());
            Err(ApiError(StatusCode::TOO_MANY_REQUESTS, "too many wrong codes; wait a moment and ask the TV for a new one".into()))
        }
        Err(crate::auth::RedeemError::Wrong) => {
            tracing::warn!("companion: wrong pairing code from {name}");
            Err(ApiError(StatusCode::FORBIDDEN, "wrong or expired code".into()))
        }
    }
}

/// Hands the page a cookie for the routes it loads by URL. Strict same-site, never readable by
/// script, scoped to the API.
async fn session(Extension(Paired(token)): Extension<Paired>) -> Response {
    let cookie = format!("{SESSION_COOKIE}={token}; Path=/api; HttpOnly; SameSite=Strict; Max-Age=31536000");
    (StatusCode::NO_CONTENT, [(header::SET_COOKIE, cookie)]).into_response()
}

/// "Pixel 8 · Chrome" from a user agent, roughly.
fn device_name(ua: &str) -> String {
    let device = if ua.contains("iPhone") {
        "iPhone"
    } else if ua.contains("iPad") {
        "iPad"
    } else if ua.contains("Android") {
        ua.split(';').nth(2).map(|s| s.split(')').next().unwrap_or("").split(" Build").next().unwrap_or("").trim()).filter(|s| !s.is_empty()).unwrap_or("Android phone")
    } else if ua.contains("Macintosh") {
        "Mac"
    } else if ua.contains("Windows") {
        "Windows PC"
    } else {
        "Phone"
    };
    let browser = if ua.contains("Firefox") { "Firefox" } else if ua.contains("Edg/") { "Edge" } else if ua.contains("Chrome") { "Chrome" } else if ua.contains("Safari") { "Safari" } else { "browser" };
    format!("{device} · {browser}")
}

async fn state(State(app): State<Shared>) -> Response {
    json_raw(state_json(&app))
}

fn state_json(app: &App) -> String {
    format!(
        "{{\"device\":{},\"snapshot\":{}}}",
        serde_json::to_string(&app.actions.device_info()).unwrap_or_else(|_| "{}".into()),
        app.actions.snapshot_json()
    )
}

async fn ws(State(app): State<Shared>, headers: HeaderMap, upgrade: WebSocketUpgrade) -> Result<Response, ApiError> {
    // A socket opened by some other site's page is refused even with the cookie.
    let origin_host = headers.get(header::ORIGIN).and_then(|v| v.to_str().ok()).map(|o| o.split_once("://").map(|(_, h)| h).unwrap_or(o).to_string());
    let host = headers.get(header::HOST).and_then(|v| v.to_str().ok()).unwrap_or("");
    if origin_host.is_some_and(|o| !o.eq_ignore_ascii_case(host)) {
        return Err(ApiError(StatusCode::FORBIDDEN, "cross-site socket".into()));
    }
    Ok(upgrade.on_upgrade(move |socket| ws_loop(app, socket)))
}

async fn ws_loop(app: Shared, mut socket: WebSocket) {
    let mut rx = app.revision.subscribe();
    if socket.send(Message::Text(state_json(&app).into())).await.is_err() {
        return;
    }
    loop {
        tokio::select! {
            changed = rx.changed() => {
                if changed.is_err() { break; }
                if socket.send(Message::Text(state_json(&app).into())).await.is_err() { break; }
            }
            incoming = socket.recv() => {
                match incoming {
                    Some(Ok(Message::Ping(p))) => { let _ = socket.send(Message::Pong(p)).await; }
                    Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
                    _ => {}
                }
            }
            _ = tokio::time::sleep(std::time::Duration::from_secs(25)) => {
                if socket.send(Message::Ping(Bytes::new())).await.is_err() { break; }
            }
        }
    }
}

#[derive(Deserialize)]
struct KeyBody {
    key: String,
}
async fn key(State(app): State<Shared>, Json(b): Json<KeyBody>) -> Result<StatusCode, ApiError> {
    app.actions.send_key(&b.key).map_err(bad)?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct TextBody {
    text: String,
}
async fn text(State(app): State<Shared>, Json(b): Json<TextBody>) -> Result<StatusCode, ApiError> {
    app.actions.type_text(&b.text).map_err(bad)?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct ActionBody {
    action: String,
}
async fn open(State(app): State<Shared>, Json(b): Json<ActionBody>) -> Result<StatusCode, ApiError> {
    app.actions.perform(&b.action).map_err(bad)?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct LaunchBody {
    package: String,
}
async fn launch(State(app): State<Shared>, Json(b): Json<LaunchBody>) -> Result<StatusCode, ApiError> {
    app.actions.launch(&b.package).map_err(bad)?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct SearchBody {
    query: String,
}
async fn search(State(app): State<Shared>, Json(b): Json<SearchBody>) -> Result<StatusCode, ApiError> {
    app.actions.search(&b.query).map_err(bad)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn layout(State(app): State<Shared>) -> Result<Response, ApiError> {
    Ok(Json(app.actions.layout()).into_response())
}

async fn set_layout(State(app): State<Shared>, Json(patch): Json<LayoutPatch>) -> Result<Response, ApiError> {
    Ok(Json(app.actions.set_layout(patch).map_err(bad)?).into_response())
}

async fn plugins(State(app): State<Shared>) -> Result<Response, ApiError> {
    Ok(json_raw(app.actions.plugins_json()))
}

async fn install_plugin(State(app): State<Shared>, body: Bytes) -> Result<Response, ApiError> {
    let _turn = app.uploads.acquire().await.map_err(|e| bad(e.to_string()))?;
    if body.is_empty() {
        return Err(bad("empty upload".into()));
    }
    let path = app.actions.temp_dir().join(format!("upload-{}.lgp", now_ms()));
    tokio::fs::create_dir_all(path.parent().unwrap_or(std::path::Path::new("."))).await.map_err(|e| bad(e.to_string()))?;
    tokio::fs::write(&path, &body).await.map_err(|e| bad(e.to_string()))?;
    let result = app.actions.install_plugin(&path);
    let _ = tokio::fs::remove_file(&path).await;
    Ok(json_raw(result.map_err(bad)?))
}

#[derive(Deserialize)]
struct UrlBody {
    url: String,
}
async fn install_plugin_url(State(app): State<Shared>, Json(b): Json<UrlBody>) -> Result<Response, ApiError> {
    let _turn = app.uploads.acquire().await.map_err(|e| bad(e.to_string()))?;
    let path = app.actions.temp_dir().join(format!("download-{}.lgp", now_ms()));
    let actions = Arc::clone(&app.actions);
    let url = b.url.clone();
    let p = path.clone();
    tokio::task::spawn_blocking(move || actions.download(&url, &p)).await.map_err(|e| bad(e.to_string()))?.map_err(bad)?;
    let result = app.actions.install_plugin(&path);
    let _ = tokio::fs::remove_file(&path).await;
    Ok(json_raw(result.map_err(bad)?))
}

#[derive(Deserialize)]
struct SettingBody {
    key: String,
    value: Option<String>,
}
async fn plugin_setting(State(app): State<Shared>, AxPath(id): AxPath<String>, Json(b): Json<SettingBody>) -> Result<StatusCode, ApiError> {
    app.actions.set_plugin_setting(&id, &b.key, b.value).map_err(bad)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn plugin_log(State(app): State<Shared>, AxPath(id): AxPath<String>) -> Result<Response, ApiError> {
    Ok(Json(app.actions.plugin_log(&id)).into_response())
}

async fn plugin_command(State(app): State<Shared>, AxPath((id, command)): AxPath<(String, String)>) -> Result<StatusCode, ApiError> {
    app.actions.plugin_command(&id, &command).map_err(bad)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn diagnostics(State(app): State<Shared>) -> Result<Response, ApiError> {
    Ok(json_raw(app.actions.diagnostics_json()))
}

async fn devices(State(app): State<Shared>, Extension(Paired(token)): Extension<Paired>) -> Response {
    Json(app.tokens.devices(Some(&token))).into_response()
}

async fn revoke_device(State(app): State<Shared>, AxPath(id): AxPath<String>) -> Result<StatusCode, ApiError> {
    if app.tokens.revoke(&id) { Ok(StatusCode::NO_CONTENT) } else { Err(ApiError(StatusCode::NOT_FOUND, "no such device".into())) }
}

async fn backup(State(app): State<Shared>) -> Result<Response, ApiError> {
    Ok((
        [(header::CONTENT_TYPE, "application/json"), (header::CONTENT_DISPOSITION, "attachment; filename=\"lounge-backup.json\"")],
        app.actions.export_json(),
    )
        .into_response())
}

async fn restore(State(app): State<Shared>, body: Bytes) -> Result<StatusCode, ApiError> {
    let _turn = app.uploads.acquire().await.map_err(|e| bad(e.to_string()))?;
    let json = std::str::from_utf8(&body).map_err(|_| bad("backup is not UTF-8".into()))?;
    app.actions.import_json(json).map_err(bad)?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct ArtQuery {
    path: String,
}
async fn art(State(app): State<Shared>, Query(q): Query<ArtQuery>) -> Result<Response, ApiError> {
    let requested = PathBuf::from(&q.path);
    let canonical = tokio::fs::canonicalize(&requested).await.map_err(|_| ApiError(StatusCode::NOT_FOUND, "no such image".into()))?;
    let mut inside = false;
    for root in app.actions.art_roots() {
        let root_canonical = tokio::fs::canonicalize(&root).await.unwrap_or(root);
        if canonical.starts_with(&root_canonical) {
            inside = true;
            break;
        }
    }
    if !inside {
        return Err(ApiError(StatusCode::FORBIDDEN, "not an artwork file".into()));
    }
    let bytes = tokio::fs::read(&canonical).await.map_err(|_| ApiError(StatusCode::NOT_FOUND, "no such image".into()))?;
    let mime = if bytes.starts_with(&[0x89, b'P', b'N', b'G']) { "image/png" } else if bytes.starts_with(&[0xFF, 0xD8]) { "image/jpeg" } else { "application/octet-stream" };
    Ok(([(header::CONTENT_TYPE, mime), (header::CACHE_CONTROL, "private, max-age=86400")], bytes).into_response())
}

async fn static_file(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    let path = if path.is_empty() { "index.html" } else { path };
    // App routes without a dot ("/share") are the single page itself.
    let served = if Assets::get(path).is_some() || path.contains('.') { path } else { "index.html" };
    match Assets::get(served) {
        Some(file) => {
            let mime = mime_guess::from_path(served).first_or_octet_stream();
            let cache = if path.starts_with("assets/") || path.starts_with("fonts/") { "public, max-age=31536000, immutable" } else { "no-cache" };
            ([(header::CONTENT_TYPE, mime.as_ref().to_string()), (header::CACHE_CONTROL, cache.to_string())], file.data.into_owned()).into_response()
        }
        None => (StatusCode::NOT_FOUND, "not found").into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_local_names_are_answered() {
        for ok in ["192.168.0.84:8787", "[fe80::1]:8787", "10.0.0.5", "tv:8787", "living-room.local", "tv.lan:8787", "localhost:8787"] {
            assert!(local_name(ok), "{ok}");
        }
        for no in ["attacker.example", "evil.com:8787", "", "rebind.attacker.example:8787"] {
            assert!(!local_name(no), "{no}");
        }
    }

    #[test]
    fn cookies_are_read_by_name() {
        let mut h = HeaderMap::new();
        h.insert(header::COOKIE, "a=1; lounge_session=tok; b=2".parse().unwrap());
        assert_eq!(cookie(&h, SESSION_COOKIE).as_deref(), Some("tok"));
        assert!(cookie_route(&Method::GET, "/api/art"));
        assert!(!cookie_route(&Method::POST, "/api/backup"));
        assert!(!cookie_route(&Method::GET, "/api/state"));
    }

    #[test]
    fn device_names_read_well() {
        assert_eq!(device_name("Mozilla/5.0 (iPhone; CPU iPhone OS 17_0 like Mac OS X) AppleWebKit/605 (KHTML, like Gecko) Version/17.0 Mobile/15E148 Safari/604.1"), "iPhone · Safari");
        assert_eq!(device_name("Mozilla/5.0 (Linux; Android 14; Pixel 8 Build/UQ1A) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120 Mobile Safari/537.36"), "Pixel 8 · Chrome");
    }
}
