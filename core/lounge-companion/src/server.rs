//! axum on a tokio runtime, on its own thread. Every `/api` route but `POST /api/pair` needs a
//! paired token, checked before the body is read, and the `GET` routes a page loads by URL also
//! take the session cookie, so no token sits in a URL. Snapshots are pushed over `/api/ws`, and
//! every error the page can show is a sentence with the cause and the fix.

use crate::auth::{PairingInfo, TokenStore};
use crate::{reads_as_sentence, CompanionActions, CompanionError, LayoutPatch};
use axum::body::Bytes;
use axum::extract::ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade};
use axum::extract::{ConnectInfo, DefaultBodyLimit, FromRequest, Path as AxPath, Query, Request, State};
use axum::http::{header, HeaderMap, Method, StatusCode, Uri};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post, put};
use axum::{Extension, Json, Router};
use rust_embed::RustEmbed;
use serde::Deserialize;
use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tower::ServiceExt;

#[derive(RustEmbed)]
#[folder = "$CARGO_MANIFEST_DIR/../../companion/dist/"]
struct Assets;

struct App {
    actions: Arc<dyn CompanionActions>,
    tokens: Arc<TokenStore>,
    revision: tokio::sync::watch::Sender<u64>,
    /// One upload at a time. An upload reads its body only once it holds this, so a bundle or a
    /// backup waiting its turn is not yet in memory.
    uploads: tokio::sync::Semaphore,
}

const SESSION_COOKIE: &str = "lounge_session";
/// The live socket's close code for a phone the TV has forgotten; the page unpairs on it.
const CLOSE_UNPAIRED: u16 = 4401;
/// Request bodies are small JSON except on the two upload routes.
const BODY_LIMIT: usize = 64 * 1024;
/// A setting may hold a pasted list, such as another podcast app's OPML export.
const SETTING_LIMIT: usize = 512 * 1024;
const BUNDLE_LIMIT: usize = 16 * 1024 * 1024;
const BACKUP_LIMIT: usize = 2 * 1024 * 1024;
/// Most connections at once, and from one address; a phone's browser opens a handful.
const MAX_CONNECTIONS: usize = 48;
const MAX_PER_ADDRESS: usize = 12;
/// A request's head must arrive within this, and a connection idle for this long is closed.
const HEAD_TIMEOUT: Duration = Duration::from_secs(10);
/// What one connection may hold while its request's head is still arriving.
const MAX_BUFFER: usize = 16 * 1024;
/// How long an upload's body may take to arrive, so a stalled one gives up its turn.
const UPLOAD_TIMEOUT: Duration = Duration::from_secs(120);
/// How long open connections get to finish once the server is told to stop.
const GRACE: Duration = Duration::from_secs(2);
/// How often the live socket says the TV is still there. A page cannot see protocol pings, so
/// this is a text message, and it is not JSON so a page that expects only snapshots skips it.
const HEARTBEAT: Duration = Duration::from_secs(5);
const HEARTBEAT_TEXT: &str = "ping";

type Shared = Arc<App>;

/// A running server. Dropping it stops the runtime.
pub struct Companion {
    pub port: u16,
    tokens: Arc<TokenStore>,
    revision: tokio::sync::watch::Sender<u64>,
    counter: AtomicU64,
    shutdown: Option<tokio::sync::oneshot::Sender<()>>,
    /// Set while the port is held, so stopping can wait for it to be free and no longer.
    listening: Arc<AtomicBool>,
}

impl Companion {
    pub fn start(port: u16, data_dir: &std::path::Path, actions: Arc<dyn CompanionActions>) -> Result<Companion, CompanionError> {
        let tokens = Arc::new(TokenStore::open(data_dir));
        let (revision, _) = tokio::sync::watch::channel(0u64);
        let app = Arc::new(App { actions, tokens: Arc::clone(&tokens), revision: revision.clone(), uploads: tokio::sync::Semaphore::new(1) });
        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
        let bound = Arc::new(parking_lot::Mutex::new(None));
        let bound_for_thread = Arc::clone(&bound);
        let listening = Arc::new(AtomicBool::new(false));
        let listening_for_thread = Arc::clone(&listening);
        std::thread::Builder::new()
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
                    listening_for_thread.store(true, Ordering::Release);
                    *bound_for_thread.lock() = Some(Ok(actual));
                    serve(listener, routes(app), shutdown_rx, &listening_for_thread).await;
                });
                // A handler may still be inside a call into the launcher; leave it rather than wait.
                rt.shutdown_timeout(Duration::from_secs(1));
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
        Ok(Companion { port: actual, tokens, revision, counter: AtomicU64::new(0), shutdown: Some(shutdown_tx), listening })
    }

    /// Tell connected phones the snapshot changed.
    pub fn notify(&self) {
        let n = self.counter.fetch_add(1, Ordering::Relaxed) + 1;
        let _ = self.revision.send(n);
    }

    pub fn new_pairing(&self, base_url: &str, now_ms: i64) -> Result<PairingInfo, String> {
        self.tokens.new_pairing(base_url, now_ms)
    }

    /// The code that pairs a phone now; a used one has already been replaced.
    pub fn live_pairing(&self, now_ms: i64) -> Option<PairingInfo> {
        self.tokens.live_pairing(now_ms)
    }

    pub fn paired_devices(&self) -> usize {
        self.tokens.count()
    }

    /// Forgets every paired phone at once; an error means the change is not yet saved.
    pub fn forget_all(&self) -> Result<(), String> {
        self.tokens.revoke_all()
    }
}

impl Drop for Companion {
    /// Stops accepting and waits only until the port is free, so a stop never blocks the caller
    /// on a connection that will not finish; the server's thread winds down on its own.
    fn drop(&mut self) {
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
        let started = std::time::Instant::now();
        while self.listening.load(Ordering::Acquire) && started.elapsed() < Duration::from_secs(1) {
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}

/// Accepts connections until told to stop, within the limits above, then gives open ones the
/// grace period to finish.
async fn serve(listener: tokio::net::TcpListener, router: Router, mut shutdown: tokio::sync::oneshot::Receiver<()>, listening: &AtomicBool) {
    let open = Arc::new(tokio::sync::Semaphore::new(MAX_CONNECTIONS));
    let per_address = Arc::new(parking_lot::Mutex::new(HashMap::new()));
    let (stop, stopped) = tokio::sync::watch::channel(false);
    let mut connections = tokio::task::JoinSet::new();
    loop {
        let (stream, peer) = tokio::select! {
            _ = &mut shutdown => break,
            accepted = listener.accept() => match accepted {
                Ok(accepted) => accepted,
                // Out of file descriptors and the like: wait rather than spin.
                Err(_) => {
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    continue;
                }
            },
        };
        while connections.try_join_next().is_some() {}
        // Over either limit, the connection is dropped unanswered.
        let Ok(permit) = Arc::clone(&open).try_acquire_owned() else { continue };
        let Some(slot) = AddressSlot::take(&per_address, peer.ip()) else { continue };
        let router = router.clone();
        let mut stopped = stopped.clone();
        connections.spawn(async move {
            let _held = (permit, slot);
            let service = hyper::service::service_fn(move |mut req: Request<hyper::body::Incoming>| {
                req.extensions_mut().insert(ConnectInfo(peer));
                router.clone().oneshot(req)
            });
            let conn = hyper::server::conn::http1::Builder::new()
                .timer(hyper_util::rt::TokioTimer::new())
                .header_read_timeout(HEAD_TIMEOUT)
                .max_buf_size(MAX_BUFFER)
                .serve_connection(hyper_util::rt::TokioIo::new(stream), service)
                .with_upgrades();
            tokio::pin!(conn);
            tokio::select! {
                _ = conn.as_mut() => {}
                _ = stopped.changed() => {
                    conn.as_mut().graceful_shutdown();
                    let _ = conn.await;
                }
            }
        });
    }
    drop(listener);
    listening.store(false, Ordering::Release);
    let _ = stop.send(true);
    let _ = tokio::time::timeout(GRACE, async { while connections.join_next().await.is_some() {} }).await;
}

/// One connection's place in its address's count, given back when the connection ends.
struct AddressSlot {
    counts: Arc<parking_lot::Mutex<HashMap<IpAddr, usize>>>,
    ip: IpAddr,
}

impl AddressSlot {
    fn take(counts: &Arc<parking_lot::Mutex<HashMap<IpAddr, usize>>>, ip: IpAddr) -> Option<AddressSlot> {
        let mut c = counts.lock();
        let n = c.entry(ip).or_insert(0);
        if *n >= MAX_PER_ADDRESS {
            return None;
        }
        *n += 1;
        Some(AddressSlot { counts: Arc::clone(counts), ip })
    }
}

impl Drop for AddressSlot {
    fn drop(&mut self) {
        let mut c = self.counts.lock();
        if let Some(n) = c.get_mut(&self.ip) {
            *n -= 1;
            if *n == 0 {
                c.remove(&self.ip);
            }
        }
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
        .route("/api/plugins/{id}/settings", put(plugin_setting).layer(DefaultBodyLimit::max(SETTING_LIMIT)))
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
        .layer(middleware::from_fn(local_only))
        .with_state(app)
}

// ---------------------------------------------------------------------------

fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

/// The id of the paired device a request comes from, found by `require_token`.
#[derive(Clone)]
struct Paired(String);

fn bearer(headers: &HeaderMap) -> Option<String> {
    let (scheme, token) = headers.get(header::AUTHORIZATION)?.to_str().ok()?.split_once(' ')?;
    scheme.eq_ignore_ascii_case("Bearer").then(|| token.trim().to_string())
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

/// Every paired route: the token is checked here, before the handler reads the body. The session
/// cookie stands in for it on the cookie routes only.
async fn require_token(State(app): State<Shared>, mut req: Request, next: Next) -> Response {
    let headers = req.headers();
    let device = if let Some(token) = bearer(headers) {
        app.tokens.authenticate(&token, now_ms())
    } else if let Some(session) = cookie_route(req.method(), req.uri().path()).then(|| cookie(headers, SESSION_COOKIE)).flatten() {
        app.tokens.authenticate_session(&session, now_ms())
    } else {
        return ApiError(StatusCode::UNAUTHORIZED, "Pair this phone with the TV first.".into()).into_response();
    };
    let Some(device) = device else {
        return ApiError(StatusCode::UNAUTHORIZED, "The TV has forgotten this phone. Pair it again to carry on.".into()).into_response();
    };
    req.extensions_mut().insert(Paired(device));
    next.run(req).await
}

/// Answer only to devices on the TV's own network, and only to names a person would type or scan
/// for a device there, so a web page cannot rebind its own domain to the TV and talk to it.
async fn local_only(req: Request, next: Next) -> Response {
    let peer = req.extensions().get::<ConnectInfo<SocketAddr>>().map(|c| c.0.ip());
    if !peer.is_some_and(local_peer) {
        return (StatusCode::FORBIDDEN, "The TV answers only phones on its own network. Join the TV's Wi-Fi, then try again.").into_response();
    }
    let host = req.headers().get(header::HOST).and_then(|v| v.to_str().ok()).unwrap_or("");
    if !local_name(host) {
        return (StatusCode::MISDIRECTED_REQUEST, "Open the phone companion at the address the TV shows. In Settings on the TV, open Phone companion to find it.").into_response();
    }
    next.run(req).await
}

/// Loopback, private, link-local or unique local: the TV itself or a device on its network.
fn local_peer(ip: IpAddr) -> bool {
    match ip.to_canonical() {
        IpAddr::V4(v4) => v4.is_loopback() || v4.is_private() || v4.is_link_local(),
        IpAddr::V6(v6) => v6.is_loopback() || v6.is_unique_local() || v6.is_unicast_link_local(),
    }
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

/// A failure the launcher reported. It words its own, so a sentence passes as it is; anything
/// else goes to the log and the page reads `fallback`.
fn refused(fallback: &'static str) -> impl FnOnce(String) -> ApiError {
    move |e| {
        if reads_as_sentence(&e) {
            return bad(e);
        }
        tracing::warn!("companion: {e}");
        bad(fallback.to_string())
    }
}

const USED_CODE: &str = "That code has been used or has expired. Choose New code on the TV and try again.";

fn json_raw(s: String) -> Response {
    ([(header::CONTENT_TYPE, "application/json")], s).into_response()
}

/// A typed code, or the key from a QR link.
#[derive(Deserialize)]
struct PairBody {
    code: Option<String>,
    link: Option<String>,
}

async fn pair(State(app): State<Shared>, ConnectInfo(peer): ConnectInfo<SocketAddr>, headers: HeaderMap, Json(body): Json<PairBody>) -> Result<Response, ApiError> {
    let name = headers
        .get(header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .map(device_name)
        .unwrap_or_else(|| "Phone".to_string());
    let token = match (body.link, body.code) {
        (Some(link), _) => app.tokens.redeem_link(link.trim(), &name, now_ms()).map_err(|e| match e {
            crate::auth::RedeemError::NotSaved(e) => not_saved(e),
            _ => {
                tracing::warn!("companion: expired pairing link from {name}");
                ApiError(StatusCode::FORBIDDEN, USED_CODE.into())
            }
        }),
        (None, Some(code)) => app.tokens.redeem(code.trim(), &name, peer.ip(), now_ms()).map_err(|e| match e {
            crate::auth::RedeemError::LockedOut => {
                tracing::warn!("companion: pairing locked out for {} after repeated wrong codes", peer.ip());
                ApiError(StatusCode::TOO_MANY_REQUESTS, "Too many wrong codes. Scan the QR code on the TV instead, or wait a few minutes and try again.".into())
            }
            crate::auth::RedeemError::Wrong => {
                tracing::warn!("companion: wrong pairing code from {name}");
                ApiError(StatusCode::FORBIDDEN, USED_CODE.into())
            }
            crate::auth::RedeemError::NotSaved(e) => not_saved(e),
        }),
        (None, None) => Err(bad("Type the six digits the TV shows.".into())),
    }?;
    Ok(Json(serde_json::json!({ "token": token, "device": app.actions.device_info().name })).into_response())
}

fn not_saved(e: String) -> ApiError {
    tracing::warn!("companion: a paired phone could not be saved ({e})");
    ApiError(StatusCode::SERVICE_UNAVAILABLE, "The TV couldn't save this phone, so it isn't paired. Check the TV has free space, then try again.".into())
}

/// Hands the page a cookie for the routes it loads by URL. It holds a value of its own, not the
/// token, because a browser sends it to every port on the TV's address. Strict same-site, never
/// readable by script, scoped to the API.
async fn session(State(app): State<Shared>, Extension(Paired(device)): Extension<Paired>) -> Result<Response, ApiError> {
    let session = app.tokens.new_session(&device).ok_or_else(|| ApiError(StatusCode::SERVICE_UNAVAILABLE, "The TV couldn't start a session for this phone. Try again in a moment.".into()))?;
    let cookie = format!("{SESSION_COOKIE}={session}; Path=/api; HttpOnly; SameSite=Strict; Max-Age=31536000");
    Ok((StatusCode::NO_CONTENT, [(header::SET_COOKIE, cookie)]).into_response())
}

/// "Chrome on Pixel 8" from a user agent, roughly.
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
    let browser = if ua.contains("Firefox") { "Firefox" } else if ua.contains("Edg/") { "Edge" } else if ua.contains("Chrome") { "Chrome" } else if ua.contains("Safari") { "Safari" } else { "A browser" };
    format!("{browser} on {device}")
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

async fn ws(State(app): State<Shared>, Extension(Paired(device)): Extension<Paired>, headers: HeaderMap, upgrade: WebSocketUpgrade) -> Result<Response, ApiError> {
    // A socket opened by some other site's page is refused even with the cookie.
    let origin_host = headers.get(header::ORIGIN).and_then(|v| v.to_str().ok()).map(|o| o.split_once("://").map(|(_, h)| h).unwrap_or(o).to_string());
    let host = headers.get(header::HOST).and_then(|v| v.to_str().ok()).unwrap_or("");
    if origin_host.is_some_and(|o| !o.eq_ignore_ascii_case(host)) {
        return Err(ApiError(StatusCode::FORBIDDEN, "cross-site socket".into()));
    }
    // The phone only ever sends pings here, so a large message is refused before it is read.
    Ok(upgrade.max_message_size(4096).max_frame_size(4096).on_upgrade(move |socket| ws_loop(app, socket, device)))
}

/// Pushes each new snapshot for as long as the device stays paired, and a heartbeat between
/// them so the page can tell a quiet TV from a gone one. Once the device is forgotten the socket
/// closes with `CLOSE_UNPAIRED`.
async fn ws_loop(app: Shared, mut socket: WebSocket, device: String) {
    let mut rx = app.revision.subscribe();
    let mut revoked = app.tokens.revocations();
    if socket.send(Message::Text(state_json(&app).into())).await.is_err() {
        return;
    }
    let mut heartbeat = tokio::time::interval_at(tokio::time::Instant::now() + HEARTBEAT, HEARTBEAT);
    heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            changed = rx.changed() => {
                if changed.is_err() { return; }
                if !app.tokens.is_paired(&device) { break; }
                if socket.send(Message::Text(state_json(&app).into())).await.is_err() { return; }
            }
            changed = revoked.changed() => {
                if changed.is_err() { return; }
                if !app.tokens.is_paired(&device) { break; }
            }
            incoming = socket.recv() => {
                match incoming {
                    Some(Ok(Message::Ping(p))) => { let _ = socket.send(Message::Pong(p)).await; }
                    Some(Ok(Message::Close(_))) | None | Some(Err(_)) => return,
                    _ => {}
                }
            }
            _ = heartbeat.tick() => {
                if socket.send(Message::Text(HEARTBEAT_TEXT.into())).await.is_err() { return; }
            }
        }
    }
    let _ = socket.send(Message::Close(Some(CloseFrame { code: CLOSE_UNPAIRED, reason: "this phone is no longer paired".into() }))).await;
}

/// Runs a call into the launcher on tokio's blocking threads. The server has two workers, and a
/// resolve, an install or a store write waiting behind a big ingest would otherwise hold them.
async fn on_core<T: Send + 'static>(app: &Shared, f: impl FnOnce(&dyn CompanionActions) -> T + Send + 'static) -> Result<T, ApiError> {
    let actions = Arc::clone(&app.actions);
    tokio::task::spawn_blocking(move || f(actions.as_ref())).await.map_err(|e| {
        tracing::warn!("companion: a call into the launcher failed ({e})");
        ApiError(StatusCode::INTERNAL_SERVER_ERROR, "Lounge on the TV ran into a problem. Try again in a moment.".into())
    })
}

#[derive(Deserialize)]
struct KeyBody {
    key: String,
}
async fn key(State(app): State<Shared>, Json(b): Json<KeyBody>) -> Result<StatusCode, ApiError> {
    on_core(&app, move |a| a.send_key(&b.key)).await?.map_err(refused("The TV didn't take that key. Try again."))?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct TextBody {
    text: String,
}
async fn text(State(app): State<Shared>, Json(b): Json<TextBody>) -> Result<StatusCode, ApiError> {
    on_core(&app, move |a| a.type_text(&b.text)).await?.map_err(refused("The TV didn't take the text. Try again."))?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct ActionBody {
    action: String,
}
async fn open(State(app): State<Shared>, Json(b): Json<ActionBody>) -> Result<StatusCode, ApiError> {
    on_core(&app, move |a| a.perform(&b.action)).await?.map_err(refused("The TV couldn't open that. Try it from the TV."))?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct LaunchBody {
    package: String,
}
async fn launch(State(app): State<Shared>, Json(b): Json<LaunchBody>) -> Result<StatusCode, ApiError> {
    on_core(&app, move |a| a.launch(&b.package)).await?.map_err(refused("The TV couldn't open that app. Is it still installed?"))?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct SearchBody {
    query: String,
}
async fn search(State(app): State<Shared>, Json(b): Json<SearchBody>) -> Result<StatusCode, ApiError> {
    on_core(&app, move |a| a.search(&b.query)).await?.map_err(refused("The TV couldn't start the search. Try again."))?;
    Ok(StatusCode::NO_CONTENT)
}

async fn layout(State(app): State<Shared>) -> Result<Response, ApiError> {
    Ok(Json(on_core(&app, |a| a.layout()).await?).into_response())
}

async fn set_layout(State(app): State<Shared>, Json(patch): Json<LayoutPatch>) -> Result<Response, ApiError> {
    Ok(Json(on_core(&app, move |a| a.set_layout(patch)).await?.map_err(refused("The TV couldn't save that change. Try again."))?).into_response())
}

async fn plugins(State(app): State<Shared>) -> Result<Response, ApiError> {
    Ok(json_raw(on_core(&app, |a| a.plugins_json()).await?))
}

/// The body of an upload that holds its turn, within the route's body limit and time.
async fn upload_body(req: Request) -> Result<Bytes, ApiError> {
    match tokio::time::timeout(UPLOAD_TIMEOUT, Bytes::from_request(req, &())).await {
        Ok(body) => body.map_err(|e| {
            tracing::warn!("companion: upload refused ({})", e.body_text());
            let said = if e.status() == StatusCode::PAYLOAD_TOO_LARGE { "That file is too big for the TV." } else { "The file didn't arrive in full. Try again." };
            ApiError(e.status(), said.into())
        }),
        Err(_) => Err(ApiError(StatusCode::REQUEST_TIMEOUT, "The file stopped arriving. Check this phone's Wi-Fi, then try again.".into())),
    }
}

/// The TV could not keep an upload long enough to read it.
fn not_stored(e: std::io::Error) -> ApiError {
    tracing::warn!("companion: an upload could not be stored ({e})");
    ApiError(StatusCode::INSUFFICIENT_STORAGE, "The TV couldn't store the file. Check it has free space, then try again.".into())
}

/// Waits for the one upload turn.
async fn upload_turn(app: &Shared) -> Result<tokio::sync::SemaphorePermit<'_>, ApiError> {
    app.uploads.acquire().await.map_err(|_| ApiError(StatusCode::SERVICE_UNAVAILABLE, "Lounge on the TV is stopping. Try again in a moment.".into()))
}

async fn install_plugin(State(app): State<Shared>, req: Request) -> Result<Response, ApiError> {
    let _turn = upload_turn(&app).await?;
    let body = upload_body(req).await?;
    if body.is_empty() {
        return Err(bad("That file is empty. Choose the plugin's .lgp file.".into()));
    }
    let path = app.actions.temp_dir().join(format!("upload-{}.lgp", now_ms()));
    tokio::fs::create_dir_all(path.parent().unwrap_or(std::path::Path::new("."))).await.map_err(not_stored)?;
    tokio::fs::write(&path, &body).await.map_err(not_stored)?;
    let p = path.clone();
    let result = on_core(&app, move |a| a.install_plugin(&p)).await;
    let _ = tokio::fs::remove_file(&path).await;
    Ok(json_raw(result?.map_err(refused("The TV couldn't install that plugin. Check it is a Lounge plugin, then try again."))?))
}

#[derive(Deserialize)]
struct UrlBody {
    url: String,
}
async fn install_plugin_url(State(app): State<Shared>, Json(b): Json<UrlBody>) -> Result<Response, ApiError> {
    let _turn = upload_turn(&app).await?;
    let path = app.actions.temp_dir().join(format!("download-{}.lgp", now_ms()));
    let p = path.clone();
    let result = on_core(&app, move |a| a.download(&b.url, &p).and_then(|()| a.install_plugin(&p))).await;
    let _ = tokio::fs::remove_file(&path).await;
    Ok(json_raw(result?.map_err(refused("The TV couldn't install from that link. Check the address, then try again."))?))
}

#[derive(Deserialize)]
struct SettingBody {
    key: String,
    value: Option<String>,
}
async fn plugin_setting(State(app): State<Shared>, AxPath(id): AxPath<String>, Json(b): Json<SettingBody>) -> Result<StatusCode, ApiError> {
    on_core(&app, move |a| a.set_plugin_setting(&id, &b.key, b.value)).await?.map_err(refused("The TV couldn't save that setting. Try again."))?;
    Ok(StatusCode::NO_CONTENT)
}

async fn plugin_log(State(app): State<Shared>, AxPath(id): AxPath<String>) -> Result<Response, ApiError> {
    Ok(Json(on_core(&app, move |a| a.plugin_log(&id)).await?).into_response())
}

async fn plugin_command(State(app): State<Shared>, AxPath((id, command)): AxPath<(String, String)>) -> Result<StatusCode, ApiError> {
    on_core(&app, move |a| a.plugin_command(&id, &command)).await?.map_err(refused("The TV couldn't do that with the plugin. Try again."))?;
    Ok(StatusCode::NO_CONTENT)
}

async fn diagnostics(State(app): State<Shared>) -> Result<Response, ApiError> {
    Ok(json_raw(on_core(&app, |a| a.diagnostics_json()).await?))
}

async fn devices(State(app): State<Shared>, Extension(Paired(device)): Extension<Paired>) -> Response {
    Json(app.tokens.devices(Some(&device))).into_response()
}

async fn revoke_device(State(app): State<Shared>, AxPath(id): AxPath<String>) -> Result<StatusCode, ApiError> {
    match app.tokens.revoke(&id) {
        Ok(true) => Ok(StatusCode::NO_CONTENT),
        Ok(false) => Err(ApiError(StatusCode::NOT_FOUND, "The TV had already forgotten that phone.".into())),
        Err(e) => {
            tracing::warn!("companion: a forgotten phone could not be saved ({e})");
            Err(ApiError(StatusCode::INTERNAL_SERVER_ERROR, "Forgotten, but the TV couldn't save that, so a restart could bring it back. Try again.".into()))
        }
    }
}

async fn backup(State(app): State<Shared>) -> Result<Response, ApiError> {
    let json = on_core(&app, |a| a.export_json()).await?;
    Ok((
        [(header::CONTENT_TYPE, "application/json"), (header::CONTENT_DISPOSITION, "attachment; filename=\"lounge-backup.json\"")],
        json,
    )
        .into_response())
}

async fn restore(State(app): State<Shared>, req: Request) -> Result<StatusCode, ApiError> {
    let _turn = upload_turn(&app).await?;
    let body = upload_body(req).await?;
    let json = String::from_utf8(body.to_vec()).map_err(|_| bad(NOT_A_BACKUP.into()))?;
    on_core(&app, move |a| a.import_json(&json)).await?.map_err(refused(NOT_A_BACKUP))?;
    Ok(StatusCode::NO_CONTENT)
}

const NOT_A_BACKUP: &str = "That file isn't a Lounge backup. Choose a file that Download backup saved.";

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
    fn only_devices_on_the_home_network_are_answered() {
        for ok in ["192.168.0.20", "10.1.2.3", "172.16.5.4", "169.254.3.4", "127.0.0.1", "::1", "fe80::1", "fd12:3456::1", "::ffff:192.168.1.9"] {
            assert!(local_peer(ok.parse().unwrap()), "{ok}");
        }
        for no in ["8.8.8.8", "172.32.0.1", "100.64.0.1", "203.0.113.7", "2001:db8::1", "::ffff:8.8.8.8"] {
            assert!(!local_peer(no.parse().unwrap()), "{no}");
        }

        let router = Router::new().route("/", get(|| async { "ok" })).layer(middleware::from_fn(local_only));
        let runtime = tokio::runtime::Builder::new_current_thread().build().unwrap();
        let status = |peer: Option<&str>| {
            let mut req = Request::builder().uri("/").header(header::HOST, "tv.local").body(axum::body::Body::empty()).unwrap();
            if let Some(peer) = peer {
                req.extensions_mut().insert(ConnectInfo(SocketAddr::new(peer.parse().unwrap(), 50_000)));
            }
            runtime.block_on(router.clone().oneshot(req)).unwrap().status()
        };
        assert_eq!(status(Some("192.168.1.9")), StatusCode::OK);
        assert_eq!(status(Some("203.0.113.7")), StatusCode::FORBIDDEN);
        assert_eq!(status(None), StatusCode::FORBIDDEN);
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
    fn the_bearer_scheme_is_read_in_any_case() {
        let with = |value: &str| {
            let mut h = HeaderMap::new();
            h.insert(header::AUTHORIZATION, value.parse().unwrap());
            bearer(&h)
        };
        assert_eq!(with("Bearer tok").as_deref(), Some("tok"));
        assert_eq!(with("bearer tok").as_deref(), Some("tok"));
        assert_eq!(with("Basic tok"), None);
        assert_eq!(with("Bearertok"), None);
    }

    #[test]
    fn device_names_read_well() {
        assert_eq!(device_name("Mozilla/5.0 (iPhone; CPU iPhone OS 17_0 like Mac OS X) AppleWebKit/605 (KHTML, like Gecko) Version/17.0 Mobile/15E148 Safari/604.1"), "Safari on iPhone");
        assert_eq!(device_name("Mozilla/5.0 (Linux; Android 14; Pixel 8 Build/UQ1A) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120 Mobile Safari/537.36"), "Chrome on Pixel 8");
    }
}
