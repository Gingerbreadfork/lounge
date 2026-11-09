//! Moonlight: the gaming PCs you stream from and the games on them, read from Sunshine's own
//! interfaces, as tiles that start a stream in Moonlight. A game left running on a PC comes back
//! in Continue watching, and a PC that is asleep is woken by Moonlight when a tile is chosen.
use lounge_plugin_sdk::{self as sdk, host, Action, HttpRequest, Item, ItemKind, Kind, Manifest, RefreshContext, RowSpec, RowStyle, SettingKind, SettingSpec};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

pub struct Moonlight;

const DEFAULT_APP: &str = "com.limelight";
/// The activity every Moonlight build exports for shortcuts, under its original package name.
const TRAMPOLINE: &str = "com.limelight.ShortcutTrampoline";
const GAMESTREAM_PORT: u16 = 47989;
const SERVERINFO_TIMEOUT_MS: u32 = 5_000;
const WEB_TIMEOUT_MS: u32 = 8_000;
/// Covers read per refresh to learn which app a PC is running, one request each.
const COVERS_PER_REFRESH: usize = 12;
const PCS_KEY: &str = "pcs";
/// A client id for the status request; Sunshine answers any, and never pairs over plain HTTP.
const CLIENT_ID: &str = "0123456789ABCDEF";

const REFUSED: &str = "Sunshine refused the username or password. Check them in the plugin's settings.";

impl sdk::Plugin for Moonlight {
    fn manifest() -> Manifest {
        Manifest {
            id: "tv.lounge.moonlight".into(),
            name: "Moonlight".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            description: "Your gaming PCs and the games on them, streamed with Moonlight; a game left running comes back in Continue watching.".into(),
            publisher_name: "Lounge".into(),
            kinds: vec![Kind::Content, Kind::Search],
            fetch_hosts: vec!["$hosts".into(), "local-network".into()],
            refresh_minutes: 5,
            rows: vec![
                RowSpec { id: "games".into(), title: "Play on Moonlight".into(), style: RowStyle::Poster },
                RowSpec { id: "pcs".into(), title: "Gaming PCs".into(), style: RowStyle::Compact },
            ],
            settings: vec![
                SettingSpec {
                    key: "hosts".into(),
                    label: "Gaming PCs".into(),
                    kind: SettingKind::Text,
                    required: true,
                    help: Some("Names or addresses of the PCs running Sunshine, separated by spaces or commas, like gaming-pc or 192.168.0.50. Add :port if Sunshine's port was changed from 47989. Pair each PC in Moonlight and open it there once, so Moonlight knows its games.".into()),
                    default: None,
                },
                SettingSpec {
                    key: "username".into(),
                    label: "Sunshine username".into(),
                    kind: SettingKind::Text,
                    required: false,
                    help: Some("The sign-in for Sunshine's web page, at https://192.168.0.50:47990 for a PC at 192.168.0.50; the same one is used on every PC. It lists the games. Without it only the PCs are shown. Apollo's page has no sign-in a plugin can use, so an Apollo PC is shown without its games.".into()),
                    default: None,
                },
                SettingSpec { key: "password".into(), label: "Sunshine password".into(), kind: SettingKind::Password, required: false, help: None, default: None },
                SettingSpec {
                    key: "app".into(),
                    label: "Moonlight app".into(),
                    kind: SettingKind::Text,
                    required: false,
                    help: Some("The app the tiles open: com.limelight for Moonlight, com.limelight.noir for Artemis.".into()),
                    default: Some(DEFAULT_APP.into()),
                },
            ],
            homepage: Some("https://moonlight-stream.org".into()),
            self_signed_servers: true,
            for_apps: Vec::new(),
            sent_to: Default::default(),
        }
    }

    fn refresh(ctx: &RefreshContext) -> Result<Vec<Item>, String> {
        let addresses = parse_hosts(&host::required_setting("hosts", "Gaming PCs")?);
        if addresses.is_empty() {
            return Err("Gaming PCs needs a name or address, like gaming-pc or 192.168.0.50.".into());
        }
        let login = Login::from_settings();
        let mut cache = Cache::load();
        let mut pcs: Vec<Pc> = addresses.iter().map(|a| Pc { address: a.clone(), ..cache.pcs.remove(&a.key()).unwrap_or_default() }).collect();

        // Every PC's status and game list first, so a PC late in the list is never starved by
        // the covers of one before it.
        let mut budget_left = true;
        let (mut answered, mut refused) = (0, 0);
        for pc in pcs.iter_mut() {
            if !budget_left {
                break;
            }
            match server_info(&pc.address) {
                Ok(info) => {
                    pc.online = true;
                    pc.reach = Reach::Fine;
                    pc.unique_id = Some(info.unique_id);
                    pc.hostname = Some(info.hostname);
                    if info.running != pc.running {
                        pc.running = info.running;
                        pc.running_since_ms = info.running.map(|_| ctx.now_ms);
                    }
                    pc.seen_ms = Some(ctx.now_ms);
                }
                Err(e) if e.contains("budget") => {
                    sdk::warn!("{}: {e}", pc.address.key());
                    budget_left = false;
                    continue;
                }
                Err(e) => {
                    sdk::info!("{}: {e}", pc.address.key());
                    pc.online = false;
                    pc.running = None;
                    pc.running_since_ms = None;
                    pc.reach = if e.contains("allow list") || e.contains("home network") { Reach::NotAllowed } else { Reach::Fine };
                }
            }
            match &login {
                Some(login) if pc.online => match app_list(&pc.address, login) {
                    Ok(apps) => {
                        pc.take_apps(apps);
                        pc.web = Web::Fine;
                        answered += 1;
                    }
                    Err(WebError::Budget(e)) => {
                        sdk::warn!("{}: {e}", pc.address.key());
                        budget_left = false;
                    }
                    Err(WebError::State(web)) => {
                        sdk::info!("{}: {}", pc.address.key(), web.log_line());
                        pc.web = web;
                        answered += 1;
                        refused += usize::from(web == Web::Refused);
                    }
                    Err(WebError::Other(e)) => sdk::warn!("{}: {e}", pc.address.key()),
                },
                Some(_) => {}
                None => {
                    pc.apps.clear();
                    pc.web = Web::Unknown;
                }
            }
        }

        // Then, for a PC running something its tiles cannot name yet, the covers that name it.
        if let Some(login) = &login {
            let mut covers_left = if budget_left { COVERS_PER_REFRESH } else { 0 };
            for pc in pcs.iter_mut() {
                let Some(running) = pc.running.filter(|_| pc.online && pc.running_app().is_none() && !pc.apps.is_empty()) else { continue };
                learn_ids(pc, login, &mut covers_left);
                if pc.running_app().is_none() && pc.apps.iter().all(|a| !a.ids.is_empty()) {
                    // Nothing left to learn and still no match: covers may have changed, so learn
                    // them again next time, once per running app.
                    if pc.unmatched != Some(running) {
                        pc.unmatched = Some(running);
                        for a in pc.apps.iter_mut().filter(|a| a.has_cover()) {
                            a.ids.clear();
                        }
                    }
                    sdk::info!("{}: running app {running} matches none of its {} apps", pc.address.key(), pc.apps.len());
                }
            }
        }

        cache.pcs = pcs.iter().map(|p| (p.address.key(), p.clone())).collect();
        cache.save();
        // Only the answers of this refresh count; what a PC said before it went offline does not.
        if answered > 0 && refused == answered {
            return Err(REFUSED.into());
        }
        if !pcs.iter().any(|p| p.unique_id.is_some()) {
            return Err(pcs[0].unreachable_error());
        }

        let items = items(&pcs, &app(), ctx.now_ms);
        let online = pcs.iter().filter(|p| p.online).count();
        let running = pcs.iter().filter(|p| p.running.is_some()).count();
        let games: usize = pcs.iter().map(|p| p.apps.len()).sum();
        sdk::info!("{} {}, {online} online, {running} running a game, {games} games", pcs.len(), if pcs.len() == 1 { "PC" } else { "PCs" });
        Ok(items)
    }

    /// Games and PCs by name, from the last refresh.
    fn search(query: &str, limit: u32) -> Result<Vec<Item>, String> {
        let query = query.trim().to_lowercase();
        if query.is_empty() {
            return Ok(Vec::new());
        }
        let cache = Cache::load();
        let pcs: Vec<&Pc> = cache.pcs.values().collect();
        let app = app();
        let app_ref = app.as_str();
        let many = pcs.len() > 1;
        let mut found: Vec<Item> = pcs
            .iter()
            .flat_map(|pc| pc.apps.iter().filter(|a| a.title().to_lowercase().contains(&query)).map(move |a| game_item(pc, a, app_ref, many, 0)))
            .collect();
        found.extend(pcs.iter().filter(|pc| pc.name().to_lowercase().contains(&query)).map(|pc| pc_item(pc, &app, 0)));
        found.truncate(limit.clamp(1, 50) as usize);
        Ok(found)
    }
}

sdk::export_plugin!(Moonlight);

fn app() -> String {
    host::setting("app").map(|a| a.trim().to_string()).filter(|a| !a.is_empty()).unwrap_or_else(|| DEFAULT_APP.into())
}

// ---------------------------------------------------------------------------
// Addresses and sign-in
// ---------------------------------------------------------------------------

/// A PC as typed: a host and the GameStream port Sunshine's other ports are counted from.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Address {
    pub host: String,
    pub port: u16,
}

impl Address {
    pub fn key(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }

    fn host_part(&self) -> String {
        if self.host.contains(':') { format!("[{}]", self.host) } else { self.host.clone() }
    }

    /// Sunshine's status interface, plain HTTP.
    pub fn status_url(&self) -> String {
        format!("http://{}:{}/serverinfo?uniqueid={CLIENT_ID}", self.host_part(), self.port)
    }

    /// Sunshine's web interface, one port up from the GameStream port, over its own certificate.
    pub fn web_url(&self, path: &str, login: Option<&Login>) -> String {
        let credentials = login.map(|l| format!("{}:{}@", url_encode(&l.username), url_encode(&l.password))).unwrap_or_default();
        format!("https://{credentials}{}:{}{path}", self.host_part(), self.port.saturating_add(1))
    }
}

/// "gaming-pc, 192.168.0.50:47989 https://den-pc:47990/" → the PCs named, in order, each once.
/// The web page's address, with `https` or its port, names the PC as well as the GameStream one.
pub fn parse_hosts(value: &str) -> Vec<Address> {
    let mut out: Vec<Address> = Vec::new();
    for token in value.split([' ', ',', '\n', '\t', ';']).map(str::trim).filter(|t| !t.is_empty()) {
        let (scheme, token) = token.split_once("://").map_or((None, token), |(s, rest)| (Some(s.to_ascii_lowercase()), rest));
        let token = token.split(['/', '?', '#']).next().unwrap_or("").trim_end_matches('.');
        let (host, port) = match token.strip_prefix('[') {
            Some(rest) => {
                let (inside, after) = rest.split_once(']').unwrap_or((rest, ""));
                (inside, after.strip_prefix(':'))
            }
            None if token.matches(':').count() == 1 => {
                let (h, p) = token.split_once(':').unwrap_or((token, ""));
                (h, Some(p))
            }
            None => (token, None),
        };
        let host = host.trim().to_ascii_lowercase();
        if host.is_empty() {
            continue;
        }
        let web = scheme.as_deref() == Some("https");
        let port = match port {
            Some(p) => match p.parse::<u16>() {
                Ok(p) if p > 1 && (web || p == GAMESTREAM_PORT + 1) => p - 1,
                Ok(p) if p > 0 => p,
                _ => {
                    sdk::warn!("{token}: not a port, using {GAMESTREAM_PORT}");
                    GAMESTREAM_PORT
                }
            },
            None => GAMESTREAM_PORT,
        };
        let address = Address { host, port };
        if !out.contains(&address) {
            out.push(address);
        }
    }
    out
}

pub struct Login {
    pub username: String,
    pub password: String,
}

impl Login {
    fn from_settings() -> Option<Login> {
        let username = host::setting("username")?.trim().to_string();
        let password = host::secret("password").unwrap_or_default();
        (!username.is_empty()).then_some(Login { username, password })
    }

    fn header(&self) -> String {
        format!("Basic {}", sdk::base64_encode(format!("{}:{}", self.username, self.password).as_bytes()))
    }
}

// ---------------------------------------------------------------------------
// What Sunshine says
// ---------------------------------------------------------------------------

pub struct ServerInfo {
    pub hostname: String,
    pub unique_id: String,
    /// The id of the app being run, when one is.
    pub running: Option<u32>,
}

/// The PC's status over plain HTTP, which Sunshine and GameStream answer without pairing.
fn server_info(address: &Address) -> Result<ServerInfo, String> {
    let url = address.status_url();
    let request = HttpRequest { timeout_ms: SERVERINFO_TIMEOUT_MS, ..HttpRequest::get(&url) };
    let r = host::fetch(&request).map_err(|e| {
        if e.contains("budget") { e } else { format!("didn't answer ({})", sdk::redact(&e)) }
    })?;
    if !r.ok() {
        return Err(format!("answered with error {}", r.status));
    }
    parse_server_info(&r.body).ok_or_else(|| "didn't answer like a Sunshine or GameStream PC".to_string())
}

/// Sunshine's `<root status_code="200">` with `hostname`, `uniqueid`, `state` and `currentgame`.
pub fn parse_server_info(xml: &str) -> Option<ServerInfo> {
    let hostname = xml_text(xml, "hostname").map(unescape).filter(|h| !h.is_empty())?;
    let unique_id = xml_text(xml, "uniqueid").map(str::trim).filter(|u| !u.is_empty())?.to_string();
    let busy = xml_text(xml, "state").is_some_and(|s| s.trim().ends_with("_SERVER_BUSY"));
    let running = xml_text(xml, "currentgame").and_then(|g| g.trim().parse::<u32>().ok()).filter(|&g| g != 0 && busy);
    Some(ServerInfo { hostname, unique_id, running })
}

/// The text of the first `<name>…</name>` in a flat document.
fn xml_text<'a>(xml: &'a str, name: &str) -> Option<&'a str> {
    let open = format!("<{name}>");
    let start = xml.find(&open)? + open.len();
    let end = xml[start..].find(&format!("</{name}>"))? + start;
    Some(&xml[start..end])
}

fn unescape(text: &str) -> String {
    text.trim().replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'").replace("&amp;", "&")
}

/// One entry of Sunshine's app list, as its web interface gives it.
#[derive(Deserialize)]
struct WebApp {
    #[serde(default)]
    name: String,
    #[serde(default, rename = "image-path")]
    image_path: Option<String>,
}

#[derive(Deserialize)]
struct WebApps {
    apps: Vec<WebApp>,
}

/// How the web interface answered the last time it was asked.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub enum Web {
    #[default]
    Unknown,
    Fine,
    /// Wrong username or password.
    Refused,
    /// A sign-in page instead of the Basic challenge: Apollo, whose page a plugin cannot sign into.
    NoBasic,
    /// The page allows only addresses Sunshine was told to.
    Forbidden,
    /// Sunshine has no username set up yet, so its page is the welcome page.
    Welcome,
}

impl Web {
    fn log_line(self) -> &'static str {
        match self {
            Web::Unknown | Web::Fine => "web interface answered",
            Web::Refused => "web interface refused the username or password",
            Web::NoBasic => "web interface wants a sign-in on its page rather than a username and password, as Apollo's does; the games cannot be listed",
            Web::Forbidden => "web interface refused this TV's address; allow it under origin_web_ui_allowed in Sunshine's settings",
            Web::Welcome => "web interface has no username set up yet; open it in a browser to create one",
        }
    }
}

enum WebError {
    State(Web),
    Budget(String),
    Other(String),
}

/// The PC's apps from Sunshine's web interface, in Sunshine's own order. Names are kept as they
/// are, since Sunshine hashes them and Moonlight matches them exactly.
fn app_list(address: &Address, login: &Login) -> Result<Vec<App>, WebError> {
    let r = web_get(address, login, "/api/apps")?;
    if r.status == 404 {
        return Err(WebError::Other("web interface has no app list at /api/apps".into()));
    }
    if r.headers.get("content-type").is_some_and(|t| t.contains("text/html")) {
        return Err(WebError::State(Web::Welcome));
    }
    let list: WebApps = serde_json::from_str(&r.body).map_err(|e| WebError::Other(format!("unreadable app list: {e}")))?;
    Ok(list
        .apps
        .into_iter()
        .enumerate()
        .filter(|(_, a)| !a.name.trim().is_empty())
        .map(|(index, a)| App { name: a.name, image_path: a.image_path.unwrap_or_default().trim().to_string(), index, ids: Vec::new() })
        .collect())
}

/// A GET from the web interface. A 404 comes back as it is, since it has a meaning of its own on
/// the cover path.
fn web_get(address: &Address, login: &Login, path: &str) -> Result<sdk::HttpResponse, WebError> {
    let url = address.web_url(path, None);
    let mut request = HttpRequest { timeout_ms: WEB_TIMEOUT_MS, ..HttpRequest::get(&url) };
    request.headers.insert("Authorization".into(), login.header());
    request.headers.insert("Accept".into(), "application/json, image/png".into());
    let r = host::fetch(&request).map_err(|e| if e.contains("budget") { WebError::Budget(e) } else { WebError::Other(format!("web interface didn't answer ({})", sdk::redact(&e))) })?;
    match r.status {
        401 if r.headers.get("www-authenticate").is_some_and(|w| w.to_ascii_lowercase().starts_with("basic")) => Err(WebError::State(Web::Refused)),
        401 => Err(WebError::State(Web::NoBasic)),
        403 => Err(WebError::State(Web::Forbidden)),
        s if !(200..300).contains(&s) && s != 404 => Err(WebError::Other(format!("web interface answered {s} for {path}"))),
        _ => Ok(r),
    }
}

/// Learns the ids Sunshine gives the PC's apps, so the one it is running can be named. An id is
/// the CRC-32 of the name and the cover's SHA-256, or of the name alone for the stock cover,
/// with the app's position added when two would otherwise collide; older versions counted.
/// Apps without a cover of their own need no request; the rest are read until one matches.
fn learn_ids(pc: &mut Pc, login: &Login, covers_left: &mut usize) {
    let Some(running) = pc.running else { return };
    for app in pc.apps.iter_mut().filter(|a| a.ids.is_empty() && (!a.has_cover() || pc.no_covers)) {
        app.ids = app_ids(&app.name, None, app.index);
        if pc.no_covers && app.has_cover() {
            // The cover cannot be read, so only the count of old versions is known.
            app.ids = vec![app.index as u32 + 1];
        }
    }
    if pc.running_app().is_some() || pc.no_covers {
        return;
    }
    let address = pc.address.clone();
    let mut no_covers = false;
    for app in pc.apps.iter_mut().filter(|a| a.ids.is_empty()) {
        if *covers_left == 0 {
            return;
        }
        *covers_left -= 1;
        match web_get(&address, login, &format!("/api/covers/{}", app.index)) {
            // The cover route says so when an app has only the stock cover; an older Sunshine
            // has no cover route and says only that nothing is there.
            Ok(r) if r.status == 404 && r.body.contains("Cover image") => app.ids = app_ids(&app.name, None, app.index),
            Ok(r) if r.status == 404 => {
                no_covers = true;
                break;
            }
            Ok(r) => app.ids = app_ids(&app.name, Some(&sdk::body_bytes(r)), app.index),
            Err(WebError::Budget(e)) => {
                sdk::info!("{}: {e}", address.key());
                *covers_left = 0;
                return;
            }
            Err(WebError::Other(e)) => sdk::info!("{}: {e}", address.key()),
            Err(WebError::State(_)) => return,
        }
        if app.ids.contains(&running) {
            return;
        }
    }
    if no_covers {
        sdk::info!("{}: Sunshine has no cover route, so apps with covers keep the stock tile and cannot be told apart when running", address.key());
        pc.no_covers = true;
        for app in pc.apps.iter_mut().filter(|a| a.ids.is_empty()) {
            app.ids = vec![app.index as u32 + 1];
        }
    }
}

/// The ids Sunshine may give an app: without and with its position, and the old count.
pub fn app_ids(name: &str, cover: Option<&[u8]>, index: usize) -> Vec<u32> {
    let mut input = name.to_string();
    if let Some(bytes) = cover {
        input.push_str(&hex(&Sha256::digest(bytes)));
    }
    let id = |s: &str| (crc32(s.as_bytes()) as i32).unsigned_abs();
    vec![id(&input), id(&format!("{input}{index}")), index as u32 + 1]
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// CRC-32 as zlib and Boost compute it.
pub fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xffff_ffffu32;
    for &b in bytes {
        crc ^= u32::from(b);
        for _ in 0..8 {
            crc = if crc & 1 == 1 { (crc >> 1) ^ 0xedb8_8320 } else { crc >> 1 };
        }
    }
    !crc
}

// ---------------------------------------------------------------------------
// What is kept between refreshes
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct App {
    /// As Sunshine has it, spaces and all.
    pub name: String,
    #[serde(default)]
    pub image_path: String,
    pub index: usize,
    /// Learnt from the cover; empty until it was.
    #[serde(default)]
    pub ids: Vec<u32>,
}

impl App {
    pub fn title(&self) -> &str {
        self.name.trim()
    }

    /// Whether Sunshine serves a cover for it: only a PNG of its own, never the stock one.
    pub fn has_cover(&self) -> bool {
        !self.image_path.is_empty() && self.image_path.to_ascii_lowercase().ends_with(".png")
    }
}

/// Whether the PC's address could be asked at all.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub enum Reach {
    #[default]
    Fine,
    /// The TV refused the name: a public-looking one that only the home network answers for.
    NotAllowed,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Pc {
    pub address: Address,
    #[serde(default)]
    pub unique_id: Option<String>,
    #[serde(default)]
    pub hostname: Option<String>,
    #[serde(default)]
    pub online: bool,
    #[serde(default)]
    pub reach: Reach,
    /// The id of the app being run.
    #[serde(default)]
    pub running: Option<u32>,
    /// When that app was first seen running, which keeps its place in Continue watching.
    #[serde(default)]
    pub running_since_ms: Option<i64>,
    /// A running app whose covers were learnt again without a match, so they are not again.
    #[serde(default)]
    pub unmatched: Option<u32>,
    #[serde(default)]
    pub web: Web,
    /// Sunshine too old to serve covers.
    #[serde(default)]
    pub no_covers: bool,
    #[serde(default)]
    pub apps: Vec<App>,
    #[serde(default)]
    pub seen_ms: Option<i64>,
}

impl Pc {
    /// The name Moonlight knows the PC by, or what was typed before it has answered.
    pub fn name(&self) -> String {
        self.hostname.clone().unwrap_or_else(|| self.address.host.clone())
    }

    /// A fresh app list, keeping the ids learnt for apps whose name, cover and place are the same.
    fn take_apps(&mut self, apps: Vec<App>) {
        let known: BTreeMap<(String, String, usize), Vec<u32>> = self.apps.drain(..).filter(|a| !a.ids.is_empty()).map(|a| ((a.name, a.image_path, a.index), a.ids)).collect();
        self.apps = apps
            .into_iter()
            .map(|mut a| {
                if let Some(ids) = known.get(&(a.name.clone(), a.image_path.clone(), a.index)) {
                    a.ids = ids.clone();
                }
                a
            })
            .collect();
    }

    fn running_app(&self) -> Option<&App> {
        let id = self.running?;
        self.apps.iter().find(|a| a.ids.contains(&id))
    }

    /// The error for a PC that has never answered, with what to check.
    fn unreachable_error(&self) -> String {
        match self.reach {
            Reach::NotAllowed => format!("{} can't be reached by that name. Use the PC's IP address, or a name ending in .local or .lan.", self.address.host),
            Reach::Fine => format!("{} didn't answer. Check that the PC is on and Sunshine is running, then try again.", self.address.key()),
        }
    }
}

#[derive(Default, Serialize, Deserialize)]
struct Cache {
    #[serde(default)]
    pcs: BTreeMap<String, Pc>,
}

impl Cache {
    fn load() -> Cache {
        host::kv_get(PCS_KEY).and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
    }

    fn save(&self) {
        if let Ok(bytes) = serde_json::to_vec(self) {
            host::kv_set(PCS_KEY, &bytes);
        }
    }
}

// ---------------------------------------------------------------------------
// Tiles
// ---------------------------------------------------------------------------

/// Every PC's tiles in the order the PCs were typed: what is running, the PC itself, then its
/// games in Sunshine's order.
pub fn items(pcs: &[Pc], app: &str, now_ms: i64) -> Vec<Item> {
    let many = pcs.len() > 1;
    let games: usize = pcs.iter().map(|p| p.apps.len()).sum();
    let mut placed = 0;
    let mut items = Vec::new();
    for (n, pc) in pcs.iter().enumerate() {
        if pc.running.is_some() {
            items.push(running_item(pc, app, now_ms));
        }
        items.push(pc_item(pc, app, (pcs.len() - n) as i32));
        for a in &pc.apps {
            items.push(game_item(pc, a, app, many, (games - placed) as i32));
            placed += 1;
        }
    }
    sdk::dedup_ids(&mut items);
    items
}

/// The app a PC is running, which Moonlight resumes when the PC is opened. A position of zero
/// makes it something to continue rather than something up next.
fn running_item(pc: &Pc, app: &str, now_ms: i64) -> Item {
    let game = pc.running_app();
    let title = game.map_or_else(|| "Game in progress".to_string(), |g| g.title().to_string());
    Item {
        id: format!("running:{}", pc.address.key()),
        kind: ItemKind::Resume,
        title: title.clone(),
        subtitle: Some(format!("Running on {}", pc.name())),
        position_ms: Some(0),
        last_used_ms: Some(pc.running_since_ms.unwrap_or(now_ms)),
        poster: game.and_then(|g| cover_url(pc, g)),
        action: open(app, pc, None, title),
        ..Default::default()
    }
}

fn pc_item(pc: &Pc, app: &str, weight: i32) -> Item {
    let subtitle = match (pc.online, pc.running_app(), pc.running) {
        (true, Some(game), _) => format!("{} running", game.title()),
        (true, None, Some(_)) => "Game running".to_string(),
        (true, None, None) => match pc.web {
            Web::Refused => "Online, sign-in refused".to_string(),
            Web::NoBasic => "Online, Apollo lists no games".to_string(),
            Web::Forbidden => "Online, web page refused this TV".to_string(),
            Web::Welcome => "Online, Sunshine has no sign-in yet".to_string(),
            Web::Fine | Web::Unknown if pc.apps.len() == 1 => "Online, 1 game".to_string(),
            Web::Fine | Web::Unknown if !pc.apps.is_empty() => format!("Online, {} games", pc.apps.len()),
            Web::Fine | Web::Unknown => "Online".to_string(),
        },
        (false, ..) if pc.reach == Reach::NotAllowed => "Not reachable by this name".to_string(),
        (false, ..) => "Not answering".to_string(),
    };
    Item {
        id: format!("pc:{}", pc.address.key()),
        kind: ItemKind::Program,
        title: pc.name(),
        subtitle: Some(subtitle),
        row: Some("pcs".into()),
        weight,
        action: open(app, pc, None, pc.name()),
        ..Default::default()
    }
}

fn game_item(pc: &Pc, game: &App, app: &str, many: bool, weight: i32) -> Item {
    Item {
        id: format!("game:{}:{}", pc.address.key(), game.name),
        kind: ItemKind::Program,
        title: game.title().to_string(),
        subtitle: many.then(|| pc.name()),
        row: Some("games".into()),
        weight,
        poster: cover_url(pc, game),
        action: open(app, pc, Some(&game.name), game.title().to_string()),
        ..Default::default()
    }
}

/// The cover through Sunshine's web interface, with the sign-in in the address for the TV to
/// use; an app with the stock cover has none to fetch.
fn cover_url(pc: &Pc, game: &App) -> Option<String> {
    if !game.has_cover() || pc.no_covers {
        return None;
    }
    let login = Login::from_settings()?;
    Some(pc.address.web_url(&cover_path(game), Some(&login)))
}

/// Where Sunshine serves the app's cover. Sunshine numbers covers by position, so a fragment
/// marks which app the address was made for, and a picture the TV kept for one app never
/// stands in for another that took its position.
pub fn cover_path(game: &App) -> String {
    let mark = crc32(format!("{}\n{}", game.name, game.image_path).as_bytes());
    format!("/api/covers/{}#{mark:08x}", game.index)
}

/// Opening Moonlight at the PC, or at a game on it. A PC that has never answered has no id
/// Moonlight would know it by, so Moonlight is simply opened.
fn open(app: &str, pc: &Pc, game: Option<&str>, title: String) -> Action {
    match &pc.unique_id {
        Some(id) => Action { view: Some(intent(app, id, game)), fallback_launch: Some(app.to_string()), title: Some(title), ..Default::default() },
        None => Action { launch: Some(app.to_string()), title: Some(title), ..Default::default() },
    }
}

/// An intent for Moonlight's shortcut activity: the PC by the id Moonlight paired with and the
/// game by its exact name; without a game it opens the PC's list, or resumes what is running.
pub fn intent(app: &str, unique_id: &str, game: Option<&str>) -> String {
    let mut uri = format!("intent:#Intent;component={app}/{TRAMPOLINE};S.UUID={};", url_encode(unique_id));
    if let Some(game) = game {
        uri.push_str(&format!("S.AppName={};", url_encode(game)));
    }
    uri.push_str("end");
    uri
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
    fn addresses_are_read_as_typed() {
        let pcs = parse_hosts("gaming-pc, 192.168.0.50:47989 http://[fd00::5]/ Den-PC:48000; gaming-pc");
        assert_eq!(pcs.iter().map(Address::key).collect::<Vec<_>>(), ["gaming-pc:47989", "192.168.0.50:47989", "fd00::5:47989", "den-pc:48000"]);
        assert_eq!(pcs[2].status_url(), "http://[fd00::5]:47989/serverinfo?uniqueid=0123456789ABCDEF");
        assert_eq!(pcs[3].web_url("/api/apps", None), "https://den-pc:48001/api/apps");
        let login = Login { username: "me".into(), password: "p@ss word".into() };
        assert_eq!(pcs[0].web_url("/api/covers/2", Some(&login)), "https://me:p%40ss%20word@gaming-pc:47990/api/covers/2");
        assert_eq!(parse_hosts("pc:notaport")[0].port, GAMESTREAM_PORT);
        assert!(parse_hosts(" , ").is_empty());
        // The web page's address names the PC too.
        assert_eq!(parse_hosts("https://gaming-pc:47990/")[0].key(), "gaming-pc:47989");
        assert_eq!(parse_hosts("gaming-pc:47990")[0].key(), "gaming-pc:47989");
        assert_eq!(parse_hosts("https://gaming-pc")[0].key(), "gaming-pc:47989");
        assert_eq!(parse_hosts("https://den:48001")[0].key(), "den:48000");
    }

    #[test]
    fn the_status_names_the_pc_and_what_runs() {
        let xml = r#"<?xml version="1.0" encoding="utf-8"?><root status_code="200"><hostname>GAMING &amp; PC</hostname><appversion>2025.628.4510</appversion><GfeVersion>3.23.0.74</GfeVersion><uniqueid>0F3A7E5C-9F4D-4B6E-8A9A-1C2D3E4F5A6B</uniqueid><HttpsPort>47984</HttpsPort><ExternalPort>47989</ExternalPort><mac>00:00:00:00:00:00</mac><PairStatus>0</PairStatus><currentgame>1178646165</currentgame><state>SUNSHINE_SERVER_BUSY</state></root>"#;
        let info = parse_server_info(xml).unwrap();
        assert_eq!(info.hostname, "GAMING & PC");
        assert_eq!(info.unique_id, "0F3A7E5C-9F4D-4B6E-8A9A-1C2D3E4F5A6B");
        assert_eq!(info.running, Some(1178646165));
        let idle = parse_server_info(&xml.replace("SUNSHINE_SERVER_BUSY", "SUNSHINE_SERVER_FREE").replace("1178646165", "0")).unwrap();
        assert_eq!(idle.running, None);
        // A GameStream PC says busy without naming the app id.
        let gfe = parse_server_info("<root status_code=\"200\"><hostname>DEN</hostname><uniqueid>abc</uniqueid><state>MJOLNIR_SERVER_BUSY</state></root>").unwrap();
        assert_eq!(gfe.running, None);
        assert!(parse_server_info("<html>router</html>").is_none());
    }

    #[test]
    fn app_ids_follow_sunshines_hashing() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        // Boost's checksum cast to a signed int and made positive.
        assert_eq!(app_ids("123456789", None, 0)[0], 873_187_034);
        let with_cover = app_ids("Desktop", Some(b"png bytes"), 3);
        let digest = hex(&Sha256::digest(b"png bytes"));
        assert_eq!(with_cover[0], (crc32(format!("Desktop{digest}").as_bytes()) as i32).unsigned_abs());
        assert_eq!(with_cover[1], (crc32(format!("Desktop{digest}3").as_bytes()) as i32).unsigned_abs());
        assert_eq!(with_cover[2], 4);
        assert_ne!(app_ids("Desktop", None, 0)[0], with_cover[0]);
        // A name keeps its spaces, since Sunshine hashed them.
        assert_ne!(app_ids(" Desktop", None, 0)[0], app_ids("Desktop", None, 0)[0]);
    }

    #[test]
    fn a_cover_address_changes_with_the_app_at_its_position() {
        let app = |name: &str, image: &str| App { name: name.into(), image_path: image.into(), index: 1, ids: Vec::new() };
        let path = cover_path(&app("Hades", "hades.png"));
        assert!(path.starts_with("/api/covers/1#") && path.len() == "/api/covers/1#".len() + 8, "{path}");
        assert_eq!(path, cover_path(&app("Hades", "hades.png")));
        assert_ne!(path, cover_path(&app("Celeste", "celeste.png")));
        assert_ne!(path, cover_path(&app("Hades", "hades-2.png")));
    }

    fn pc() -> Pc {
        Pc {
            address: Address { host: "gaming-pc".into(), port: 47989 },
            unique_id: Some("0F3A7E5C-9F4D-4B6E-8A9A-1C2D3E4F5A6B".into()),
            hostname: Some("GAMING-PC".into()),
            online: true,
            running: Some(77),
            running_since_ms: Some(500),
            web: Web::Fine,
            apps: vec![
                App { name: "Desktop".into(), image_path: "desktop.png".into(), index: 0, ids: vec![5, 1] },
                App { name: "Elden Ring ".into(), image_path: "".into(), index: 1, ids: vec![77, 2] },
                App { name: "Steam Big Picture".into(), image_path: "steam.png".into(), index: 2, ids: Vec::new() },
            ],
            ..Pc::default()
        }
    }

    #[test]
    fn a_running_game_resumes_and_the_rest_launch_by_name() {
        let items = items(&[pc()], "com.limelight", 1_000);
        assert_eq!(items.iter().map(|i| i.id.as_str()).collect::<Vec<_>>(), ["running:gaming-pc:47989", "pc:gaming-pc:47989", "game:gaming-pc:47989:Desktop", "game:gaming-pc:47989:Elden Ring ", "game:gaming-pc:47989:Steam Big Picture"]);
        assert_eq!(items[0].kind, ItemKind::Resume);
        assert_eq!(items[0].title, "Elden Ring");
        assert_eq!(items[0].subtitle.as_deref(), Some("Running on GAMING-PC"));
        assert_eq!((items[0].position_ms, items[0].last_used_ms), (Some(0), Some(500)));
        assert_eq!(items[0].action.view.as_deref(), Some("intent:#Intent;component=com.limelight/com.limelight.ShortcutTrampoline;S.UUID=0F3A7E5C-9F4D-4B6E-8A9A-1C2D3E4F5A6B;end"));
        assert_eq!(items[1].subtitle.as_deref(), Some("Elden Ring running"));
        assert_eq!(items[1].row.as_deref(), Some("pcs"));
        // The name goes to Moonlight exactly as Sunshine has it, trailing space and all.
        assert_eq!(items[3].title, "Elden Ring");
        assert_eq!(items[3].action.view.as_deref(), Some("intent:#Intent;component=com.limelight/com.limelight.ShortcutTrampoline;S.UUID=0F3A7E5C-9F4D-4B6E-8A9A-1C2D3E4F5A6B;S.AppName=Elden%20Ring%20;end"));
        assert_eq!(items[4].action.fallback_launch.as_deref(), Some("com.limelight"));
        assert_eq!(items[2].weight, 3);
        assert_eq!(items[4].weight, 1);
        // One PC needs no name under each game; a second does, and its games follow the first's.
        assert_eq!(items[3].subtitle, None);
        let mut other = pc();
        other.address.host = "den".into();
        other.running = None;
        other.unique_id = None;
        other.hostname = None;
        other.online = false;
        let two = super::items(&[pc(), other], "com.limelight.noir", 0);
        let den = two.iter().find(|i| i.id == "pc:den:47989").unwrap();
        assert_eq!(den.subtitle.as_deref(), Some("Not answering"));
        assert_eq!(den.weight, 1);
        // A PC Moonlight cannot yet be pointed at just opens Moonlight.
        assert_eq!((den.action.view.as_deref(), den.action.launch.as_deref()), (None, Some("com.limelight.noir")));
        let den_game = two.iter().find(|i| i.id == "game:den:47989:Desktop").unwrap();
        assert_eq!(den_game.subtitle.as_deref(), Some("den"));
        assert_eq!(den_game.weight, 3);
        assert_eq!(two.iter().find(|i| i.id == "game:gaming-pc:47989:Desktop").unwrap().weight, 6);
    }

    #[test]
    fn a_fresh_app_list_keeps_what_was_learnt() {
        let mut pc = pc();
        pc.take_apps(vec![
            App { name: "Elden Ring ".into(), image_path: "".into(), index: 1, ids: Vec::new() },
            App { name: "Desktop".into(), image_path: "desktop.png".into(), index: 0, ids: Vec::new() },
            App { name: "Moved".into(), image_path: "".into(), index: 2, ids: Vec::new() },
        ]);
        assert_eq!(pc.apps[0].ids, [77, 2]);
        assert_eq!(pc.apps[1].ids, [5, 1]);
        assert!(pc.apps[2].ids.is_empty());
        assert_eq!(pc.running_app().map(App::title), Some("Elden Ring"));
        // An app that moved to another place is learnt again, since its ids may include it.
        let mut moved = pc.clone();
        moved.take_apps(vec![App { name: "Desktop".into(), image_path: "desktop.png".into(), index: 4, ids: Vec::new() }]);
        assert!(moved.apps[0].ids.is_empty());
        let idle = Pc { running: None, ..pc.clone() };
        assert_eq!(pc_item(&idle, "com.limelight", 0).subtitle.as_deref(), Some("Online, 3 games"));
        for (web, text) in [(Web::Refused, "Online, sign-in refused"), (Web::NoBasic, "Online, Apollo lists no games"), (Web::Forbidden, "Online, web page refused this TV"), (Web::Welcome, "Online, Sunshine has no sign-in yet")] {
            let pc = Pc { running: None, web, apps: Vec::new(), ..pc.clone() };
            assert_eq!(pc_item(&pc, "com.limelight", 0).subtitle.as_deref(), Some(text), "{web:?}");
        }
        let refused_name = Pc { online: false, reach: Reach::NotAllowed, ..pc.clone() };
        assert_eq!(pc_item(&refused_name, "com.limelight", 0).subtitle.as_deref(), Some("Not reachable by this name"));
        assert!(refused_name.unreachable_error().contains("IP address"));
    }

    #[test]
    fn the_web_interface_answer_is_read() {
        let list: WebApps = serde_json::from_str(r#"{"env":{"PATH":"$(PATH)"},"apps":[{"name":"Desktop","image-path":"desktop.png","output":"","cmd":"","exclude-global-prep-cmd":false,"elevated":false,"auto-detach":true,"wait-all":true,"exit-timeout":5,"prep-cmd":[]},{"name":"Steam Big Picture","image-path":null,"detached":["steam://open/bigpicture"]},{"name":"  "}]}"#).unwrap();
        assert_eq!(list.apps.len(), 3);
        assert_eq!(list.apps[0].image_path.as_deref(), Some("desktop.png"));
        assert_eq!(list.apps[1].image_path, None);
        // A body without the list is not an empty list.
        assert!(serde_json::from_str::<WebApps>(r#"{"error":"Not Found"}"#).is_err());
        assert!(App { name: "a".into(), image_path: "Cover.PNG".into(), ..Default::default() }.has_cover());
        assert!(!App { name: "a".into(), image_path: "cover.jpg".into(), ..Default::default() }.has_cover());
        assert!(!App { name: "a".into(), ..Default::default() }.has_cover());
    }
}
