//! Stremio: what you paused, what is up next and the library of your Stremio account, read from
//! Stremio's own API, with episode names and pictures filled in from Cinemeta. Items open on
//! their page in the Stremio app; search asks Cinemeta, so a title found here opens there too.
//! Sign-in is a code entered on the phone at link.stremio.com, or an email and password.

use lounge_plugin_sdk::{self as sdk, host, parse_iso8601, Action, HttpRequest, Item, ItemKind, Kind, Manifest, RefreshContext, RowSpec, RowStyle, SettingKind, SettingSpec};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};

pub struct Stremio;

const API: &str = "https://api.strem.io/api";
const LINK_API: &str = "https://link.stremio.com/api/v2";
const CINEMETA: &str = "https://v3-cinemeta.strem.io";
const APP_PACKAGE: &str = "com.stremio.one";
const KEY_KEY: &str = "auth_key";
const OWNER_KEY: &str = "auth_for";
const LINK_KEY: &str = "link_code";
const META_KEY: &str = "meta_v2";
const NEW_EPISODES_KEY: &str = "new_episodes";
const RESUME_ITEMS: usize = 24;
const LIBRARY_ITEMS: usize = 60;
/// Most shows asked about for new episodes, Stremio's own number.
const NEW_EPISODE_SHOWS: usize = 100;
/// How long the last answer about new episodes serves; Stremio asks about as often.
const NEW_EPISODES_EVERY_MS: i64 = 6 * 3_600_000;
/// How long a sign-in code is shown before a new one is made.
const LINK_CODE_MS: i64 = 30 * 60_000;
/// Most Cinemeta lookups in one refresh; the rest come with the next, from the cache onwards.
const LOOKUPS: usize = 20;
/// The key-value store takes 256 KB a key; the cache stays well under it.
const CACHE_BYTES: usize = 200 * 1024;
/// The owner of a sign-in made with a code rather than an email.
const LINKED: &str = "link";
const REJECTED: &str = "Stremio rejected the email or password. Check them in the plugin's settings.";
const SIGNED_OUT: &str = "Stremio no longer knows this sign-in. Press Refresh now to sign in again.";

impl sdk::Plugin for Stremio {
    fn manifest() -> Manifest {
        Manifest {
            id: "tv.lounge.stremio".into(),
            name: "Stremio".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            description: "Continue watching, new episodes and your library from your Stremio account, opened in Stremio.".into(),
            publisher_name: "Lounge".into(),
            kinds: vec![Kind::Content, Kind::Search],
            fetch_hosts: vec!["api.strem.io".into(), "link.stremio.com".into(), "v3-cinemeta.strem.io".into(), "*".into()],
            refresh_minutes: 15,
            rows: vec![RowSpec { id: "library".into(), title: "Library on Stremio".into(), style: RowStyle::Poster }],
            settings: vec![
                SettingSpec {
                    key: "email".into(),
                    label: "Email".into(),
                    kind: SettingKind::Text,
                    required: false,
                    help: Some("Leave both empty to sign in with a code on your phone instead; this page shows the code. Accounts made with Facebook or Apple have no password, so they sign in that way.".into()),
                    default: None,
                },
                SettingSpec { key: "password".into(), label: "Password".into(), kind: SettingKind::Password, required: false, help: Some("Only with an email above.".into()), default: None },
                SettingSpec {
                    key: "new_episodes".into(),
                    label: "New episodes".into(),
                    kind: SettingKind::Toggle,
                    required: false,
                    help: Some("A show in your library with an episode out since you last watched it joins Continue watching as up next.".into()),
                    default: Some("true".into()),
                },
                SettingSpec { key: "library".into(), label: "Library row".into(), kind: SettingKind::Toggle, required: false, help: Some("The movies and shows in your Stremio library, most recently watched first.".into()), default: Some("true".into()) },
            ],
            homepage: Some("https://www.stremio.com".into()),
            self_signed_servers: false,
            for_apps: vec![APP_PACKAGE.into()],
        }
    }

    fn refresh(ctx: &RefreshContext) -> Result<Vec<Item>, String> {
        let library = fetch_library(ctx.now_ms)?;
        let mut meta = MetaCache::load();
        let mut items = Vec::new();

        let mut watching: Vec<&LibraryItem> = library.iter().filter(|it| it.in_continue_watching()).collect();
        watching.sort_by_key(|it| std::cmp::Reverse(it.last_used_ms()));
        watching.truncate(RESUME_ITEMS);
        let mut lookups = 0;
        for it in &watching {
            let video = it.video_id().filter(|_| it.kind == "series");
            if lookups < LOOKUPS && meta.needs(&it.id, video) {
                lookups += meta.fetch(&it.kind, &it.id, video);
            }
            items.push(resume_item(it, &meta));
        }
        meta.keep(watching.iter().map(|it| it.id.as_str()));
        meta.save();

        let mut fresh = 0;
        if host::setting("new_episodes").map(|v| v != "false").unwrap_or(true) {
            let mut shows: Vec<&LibraryItem> = library.iter().filter(|it| it.wants_new_episodes() && !watching.iter().any(|w| w.id == it.id)).collect();
            shows.sort_by_key(|it| std::cmp::Reverse(it.modified_ms()));
            shows.truncate(NEW_EPISODE_SHOWS);
            match NewEpisodes::for_shows(&shows, ctx.now_ms) {
                Ok(latest) => {
                    for it in &shows {
                        if let Some(item) = latest.item_for(it, ctx.now_ms) {
                            items.push(item);
                            fresh += 1;
                        }
                    }
                }
                // What is already gathered is worth more than the new episodes that did not come.
                Err(e) => sdk::warn!("new episodes: {e}"),
            }
        }

        let mut listed = 0;
        if host::setting("library").map(|v| v != "false").unwrap_or(true) {
            let mut saved: Vec<&LibraryItem> = library.iter().filter(|it| it.in_library()).collect();
            saved.sort_by_key(|it| std::cmp::Reverse(it.last_used_ms().or_else(|| it.added_ms())));
            saved.truncate(LIBRARY_ITEMS);
            let count = saved.len() as i32;
            for (i, it) in saved.iter().enumerate() {
                items.push(library_item(it, count - i as i32));
                listed += 1;
            }
        }
        sdk::dedup_ids(&mut items);
        sdk::info!("{} items in the library, {} to continue, {fresh} with a new episode, {listed} listed", library.len(), watching.len());
        Ok(items)
    }

    fn search(query: &str, limit: u32) -> Result<Vec<Item>, String> {
        cinemeta_search(query, limit as usize, &|m| open(&m.kind, &m.id, None, &m.name))
    }
}

sdk::export_plugin!(Stremio);

// ---------------------------------------------------------------------------
// Stremio's API: one sign-in, then the library in one call
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct ApiAnswer<T> {
    result: Option<T>,
    error: Option<ApiError>,
}

#[derive(Deserialize, Debug)]
struct ApiError {
    #[serde(default)]
    code: i64,
    #[serde(default)]
    message: String,
}

#[derive(Deserialize)]
struct AuthResult {
    #[serde(rename = "authKey")]
    auth_key: String,
}

#[derive(Deserialize)]
struct LinkCode {
    code: String,
}

#[derive(Serialize, Deserialize)]
struct PendingLink {
    code: String,
    expires_ms: i64,
}

#[derive(Deserialize, Default, Clone, Debug)]
pub struct LibraryItem {
    #[serde(rename = "_id", default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(rename = "type", default)]
    pub kind: String,
    #[serde(default)]
    pub poster: Option<String>,
    #[serde(default)]
    pub removed: bool,
    #[serde(default)]
    pub temp: bool,
    #[serde(rename = "_ctime", default)]
    pub ctime: Option<String>,
    #[serde(rename = "_mtime", default)]
    pub mtime: Option<String>,
    #[serde(default)]
    pub state: State,
    #[serde(rename = "behaviorHints", default)]
    pub hints: Hints,
}

#[derive(Deserialize, Default, Clone, Debug)]
pub struct State {
    #[serde(rename = "lastWatched", default)]
    pub last_watched: Option<String>,
    #[serde(rename = "timeOffset", default)]
    pub time_offset: u64,
    #[serde(default)]
    pub duration: u64,
    #[serde(default)]
    pub video_id: Option<String>,
    #[serde(rename = "noNotif", default)]
    pub no_notif: bool,
}

#[derive(Deserialize, Default, Clone, Debug)]
pub struct Hints {
    #[serde(rename = "isLive", default)]
    pub is_live: bool,
    #[serde(rename = "defaultVideoId", default)]
    pub default_video_id: Option<String>,
}

impl LibraryItem {
    /// Stremio's own rule for its Continue watching row, less live channels, which have no
    /// position to come back to.
    fn in_continue_watching(&self) -> bool {
        self.kind != "other" && (!self.removed || self.temp) && self.state.time_offset > 0 && !self.is_live()
    }

    /// Stremio moves a show to its next episode with a position of 1 ms: not started yet.
    fn is_up_next(&self) -> bool {
        self.state.time_offset <= 1
    }

    fn in_library(&self) -> bool {
        self.kind != "other" && !self.removed && !self.temp
    }

    /// Stremio's own rule for the shows it watches for new episodes, among those watched at
    /// least once; one never started would otherwise come up with every episode it ever had.
    fn wants_new_episodes(&self) -> bool {
        self.kind == "series" && !self.removed && !self.temp && !self.state.no_notif && self.hints.default_video_id.is_none() && self.last_watched_ms().is_some()
    }

    fn is_live(&self) -> bool {
        self.hints.is_live || self.kind == "tv"
    }

    fn video_id(&self) -> Option<&str> {
        self.state.video_id.as_deref().map(str::trim).filter(|v| !v.is_empty())
    }

    fn last_watched_ms(&self) -> Option<i64> {
        self.state.last_watched.as_deref().and_then(parse_iso8601)
    }

    fn last_used_ms(&self) -> Option<i64> {
        self.last_watched_ms().or_else(|| self.modified_ms())
    }

    fn added_ms(&self) -> Option<i64> {
        self.ctime.as_deref().and_then(parse_iso8601)
    }

    fn modified_ms(&self) -> Option<i64> {
        self.mtime.as_deref().and_then(parse_iso8601)
    }

    fn poster(&self) -> Option<String> {
        self.poster.as_deref().filter(|p| !p.trim().is_empty()).map(larger_metahub_poster)
    }
}

fn post(path: &str, body: serde_json::Value) -> Result<String, String> {
    let req = HttpRequest {
        method: "POST".into(),
        url: format!("{API}/{path}"),
        headers: [("Content-Type".to_string(), "application/json".to_string()), ("Accept".to_string(), "application/json".to_string())].into_iter().collect(),
        body: Some(body.to_string()),
        timeout_ms: 0,
    };
    let r = sdk::fetch_online("Stremio", &req)?;
    if !r.ok() {
        sdk::warn!("HTTP {} for {path}", r.status);
        return Err(sdk::online_error("Stremio", r.status));
    }
    Ok(r.body)
}

/// Reads Stremio's envelope, whose errors come back with HTTP 200.
fn answer<T: serde::de::DeserializeOwned>(body: &str, path: &str) -> Result<Result<T, ApiError>, String> {
    let a: ApiAnswer<T> = serde_json::from_str(body).map_err(|e| {
        sdk::warn!("unexpected answer for {path}: {e}");
        sdk::unreadable("Stremio", API)
    })?;
    match (a.result, a.error) {
        (Some(r), _) => Ok(Ok(r)),
        (None, Some(e)) => Ok(Err(e)),
        (None, None) => {
            sdk::warn!("neither a result nor an error for {path}");
            Err(sdk::unreadable("Stremio", API))
        }
    }
}

/// How the person signs in: with the email and password they typed, or with a code.
enum SignIn {
    Password(String),
    Code,
}

fn sign_in_method() -> SignIn {
    match host::setting("email").map(|e| e.trim().to_string()).filter(|e| !e.is_empty()) {
        Some(email) => SignIn::Password(email),
        None => SignIn::Code,
    }
}

fn save_auth_key(key: &str, owner: &str) {
    host::kv_set(KEY_KEY, key.as_bytes());
    host::kv_set(OWNER_KEY, owner.as_bytes());
    host::kv_delete(LINK_KEY);
}

/// The saved sign-in for this method, or a fresh one.
fn auth_key(method: &SignIn, now_ms: i64) -> Result<String, String> {
    let owner = match method {
        SignIn::Password(email) => email.as_str(),
        SignIn::Code => LINKED,
    };
    if host::kv_get_string(OWNER_KEY).as_deref() == Some(owner)
        && let Some(key) = host::kv_get_string(KEY_KEY)
    {
        return Ok(key);
    }
    sign_in(method, now_ms)
}

fn sign_in(method: &SignIn, now_ms: i64) -> Result<String, String> {
    match method {
        SignIn::Password(email) => sign_in_with_password(email),
        SignIn::Code => link_step(now_ms),
    }
}

fn sign_in_with_password(email: &str) -> Result<String, String> {
    let password = host::secret("password").filter(|p| !p.is_empty()).ok_or("Password is not set. Leave the email empty too to sign in with a code instead.")?;
    let body = post("login", serde_json::json!({ "type": "Login", "email": email, "password": password, "facebook": false }))?;
    match answer::<AuthResult>(&body, "login")? {
        Ok(auth) => {
            save_auth_key(&auth.auth_key, email);
            Ok(auth.auth_key)
        }
        Err(e) => {
            sdk::warn!("sign-in refused: {} (code {})", e.message, e.code);
            Err(REJECTED.into())
        }
    }
}

fn link_get(path: &str) -> Result<String, String> {
    let r = sdk::fetch_online("Stremio", &HttpRequest::get(format!("{LINK_API}/{path}")).header("Accept", "application/json"))?;
    if !r.ok() {
        sdk::warn!("HTTP {} for the sign-in code", r.status);
        return Err(sdk::online_error("Stremio", r.status));
    }
    Ok(r.body)
}

/// The sign-in once the code has been entered, so the refresh goes on, or else the sentence
/// the plugin's page shows meanwhile. Stremio answers an unentered code the way it answers an
/// unknown one, so the code is kept for a while and then made again.
fn link_step(now_ms: i64) -> Result<String, String> {
    let pending: Option<PendingLink> = host::kv_get(LINK_KEY).and_then(|v| serde_json::from_slice(&v).ok()).filter(|p: &PendingLink| p.expires_ms > now_ms);
    if let Some(p) = pending {
        let body = link_get(&format!("read?type=Read&code={}", encode_component(&p.code)))?;
        match answer::<AuthResult>(&body, "read")? {
            Ok(auth) => {
                save_auth_key(&auth.auth_key, LINKED);
                return Ok(auth.auth_key);
            }
            Err(_) => return Err(instruction(&p.code)),
        }
    }
    let body = link_get("create?type=Create")?;
    match answer::<LinkCode>(&body, "create")? {
        Ok(link) => {
            let p = PendingLink { code: link.code, expires_ms: now_ms + LINK_CODE_MS };
            host::kv_set(LINK_KEY, serde_json::to_string(&p).unwrap_or_default().as_bytes());
            Err(instruction(&p.code))
        }
        Err(e) => {
            sdk::warn!("no sign-in code: {} (code {})", e.message, e.code);
            Err(sdk::online_error("Stremio", 400))
        }
    }
}

fn instruction(code: &str) -> String {
    format!("Sign in: on your phone open link.stremio.com/{code} and sign in to Stremio, then press Refresh now here.")
}

/// Every item of the account's library. A sign-in Stremio has forgotten is made again once.
fn fetch_library(now_ms: i64) -> Result<Vec<LibraryItem>, String> {
    let method = sign_in_method();
    let mut key = auth_key(&method, now_ms)?;
    for attempt in 0..2 {
        let body = post("datastoreGet", serde_json::json!({ "authKey": key, "collection": "libraryItem", "ids": [], "all": true }))?;
        match answer::<Vec<serde_json::Value>>(&body, "datastoreGet")? {
            Ok(values) => return Ok(parse_items(values)),
            // Code 1 is a session Stremio has forgotten; anything else is not mended by signing in.
            Err(e) if e.code == 1 && attempt == 0 => {
                sdk::warn!("the saved sign-in is no longer valid; signing in again");
                host::kv_delete(KEY_KEY);
                host::kv_delete(OWNER_KEY);
                key = sign_in(&method, now_ms)?;
            }
            Err(e) if e.code == 1 => return Err(SIGNED_OUT.into()),
            Err(e) => {
                sdk::warn!("datastoreGet refused: {} (code {})", e.message, e.code);
                return Err(sdk::online_error("Stremio", 400));
            }
        }
    }
    Err(SIGNED_OUT.into())
}

/// Items one at a time, so one entry this plugin cannot read costs only itself.
fn parse_items(values: Vec<serde_json::Value>) -> Vec<LibraryItem> {
    let total = values.len();
    let items: Vec<LibraryItem> = values.into_iter().filter_map(|v| serde_json::from_value::<LibraryItem>(v).ok()).filter(|it| !it.id.is_empty()).collect();
    if items.len() < total {
        sdk::warn!("{} library entries could not be read", total - items.len());
    }
    items
}

// ---------------------------------------------------------------------------
// Tiles
// ---------------------------------------------------------------------------

fn resume_item(it: &LibraryItem, meta: &MetaCache) -> Item {
    let video_id = it.video_id();
    let episode = video_id.filter(|_| it.kind == "series").and_then(episode_of);
    let known = meta.get(&it.id);
    let video = video_id.and_then(|v| known.and_then(|m| m.videos.get(v)));
    // An episode Stremio has only moved on to has no position; the host shows it as up next.
    let (duration_ms, position_ms) = if it.is_up_next() { (None, None) } else { ((it.state.duration > 0).then_some(it.state.duration as i64), Some(it.state.time_offset as i64)) };
    let progress = duration_ms.map(|d| (it.state.time_offset as f32 / d as f32).clamp(0.0, 1.0));
    Item {
        id: format!("resume:{}", it.id),
        kind: ItemKind::Resume,
        title: it.name.clone(),
        subtitle: video.map(|v| v.title.clone()).filter(|t| !t.is_empty()),
        season: episode.map(|(s, _)| s.to_string()),
        episode: episode.map(|(_, e)| e.to_string()),
        duration_ms,
        position_ms,
        progress,
        last_used_ms: it.last_used_ms(),
        poster: it.poster().or_else(|| known.and_then(|m| m.poster.clone())),
        thumbnail: video.and_then(|v| v.thumbnail.clone()).or_else(|| known.and_then(|m| m.background.clone())),
        action: open(&it.kind, &it.id, video_id, &it.name),
        ..Default::default()
    }
}

fn library_item(it: &LibraryItem, weight: i32) -> Item {
    Item {
        id: format!("lib:{}", it.id),
        kind: ItemKind::Program,
        title: it.name.clone(),
        added_ms: it.added_ms(),
        poster: it.poster(),
        row: Some("library".into()),
        weight,
        action: open(&it.kind, &it.id, None, &it.name),
        ..Default::default()
    }
}

/// The season and episode in a Stremio video id such as `tt0903747:2:4`.
fn episode_of(video_id: &str) -> Option<(u32, u32)> {
    let mut parts = video_id.rsplit(':');
    let episode = parts.next()?.parse().ok()?;
    let season = parts.next()?.parse().ok()?;
    parts.next()?;
    Some((season, episode))
}

/// Opens the item's page in Stremio, the way Stremio's own deep links name it: with the video
/// to the streams for it, without to the list of episodes. The intent names the app, since
/// another app on the TV may answer `stremio://` links too.
fn open(kind: &str, id: &str, video_id: Option<&str>, title: &str) -> Action {
    let mut path = format!("/detail/{}/{}", encode_component(kind), encode_component(id));
    if let Some(v) = video_id {
        path.push('/');
        path.push_str(&encode_component(v));
    }
    Action {
        view: Some(format!("intent://{path}#Intent;scheme=stremio;package={APP_PACKAGE};end")),
        fallback_launch: Some(APP_PACKAGE.into()),
        title: Some(title.to_string()),
        ..Default::default()
    }
}

/// Percent-encodes as a URI component, as Stremio does for the ids in its links.
fn encode_component(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'!' | b'~' | b'*' | b'\'' | b'(' | b')' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Stremio keeps the small metahub poster; the medium one suits a TV.
fn larger_metahub_poster(url: &str) -> String {
    match url.strip_prefix("https://images.metahub.space/poster/small/") {
        Some(rest) => format!("https://images.metahub.space/poster/medium/{rest}"),
        None => url.to_string(),
    }
}

// ---------------------------------------------------------------------------
// Cinemeta: episode names, pictures, new episodes and search
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct MetaAnswer {
    meta: Option<CinemetaMeta>,
}

#[derive(Deserialize, Default)]
struct CinemetaMeta {
    #[serde(default)]
    poster: Option<String>,
    #[serde(default)]
    background: Option<String>,
    #[serde(default)]
    videos: Vec<CinemetaVideo>,
}

#[derive(Deserialize, Default, Clone)]
pub struct CinemetaVideo {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    pub season: Option<u32>,
    #[serde(default)]
    pub episode: Option<u32>,
    #[serde(default)]
    pub thumbnail: Option<String>,
    #[serde(default)]
    pub released: Option<String>,
}

impl CinemetaVideo {
    /// Cinemeta names an episode in `name`; other add-ons use `title`.
    fn title(&self) -> String {
        self.name.clone().or_else(|| self.title.clone()).unwrap_or_default()
    }

    fn released_ms(&self) -> Option<i64> {
        self.released.as_deref().and_then(parse_iso8601)
    }
}

/// What a Cinemeta lookup gave: the title's details, nothing for a title it has never heard
/// of, or an error for an answer that may be better next time.
fn cinemeta_meta(kind: &str, id: &str) -> Result<Option<CinemetaMeta>, String> {
    let url = format!("{CINEMETA}/meta/{}/{}.json", encode_component(kind), encode_component(id));
    let r = sdk::fetch_online("Cinemeta", &HttpRequest::get(&url))?;
    match r.status {
        404 => Ok(None),
        s if !(200..300).contains(&s) => Err(format!("HTTP {s}")),
        _ => serde_json::from_str::<MetaAnswer>(&r.body).map(|a| a.meta).map_err(|e| e.to_string()),
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
    poster: Option<String>,
    background: Option<String>,
    /// The episode last asked about, by video id; a show is only ever at one.
    #[serde(default)]
    videos: BTreeMap<String, KnownVideo>,
}

#[derive(Serialize, Deserialize, Default, Clone)]
struct KnownVideo {
    title: String,
    thumbnail: Option<String>,
}

/// What Cinemeta has said about the items being watched, kept between refreshes so each title
/// is asked about once, and once more for each new episode.
#[derive(Default)]
struct MetaCache {
    known: HashMap<String, KnownMeta>,
}

impl MetaCache {
    fn load() -> MetaCache {
        MetaCache { known: host::kv_get(META_KEY).and_then(|v| serde_json::from_slice(&v).ok()).unwrap_or_default() }
    }

    fn save(&self) {
        save_within_limit(META_KEY, &self.known);
    }

    fn get(&self, id: &str) -> Option<&KnownMeta> {
        self.known.get(id)
    }

    /// Whether Cinemeta should be asked: an IMDb title not seen yet, or seen without this episode.
    fn needs(&self, id: &str, video_id: Option<&str>) -> bool {
        if !id.starts_with("tt") {
            return false;
        }
        match (self.known.get(id), video_id) {
            (None, _) => true,
            (Some(m), Some(v)) => !m.videos.contains_key(v),
            (Some(_), None) => false,
        }
    }

    /// Asks Cinemeta about one title and returns the fetches that took. Only an answer is
    /// kept, so a lookup that failed is tried again next time.
    fn fetch(&mut self, kind: &str, id: &str, video_id: Option<&str>) -> usize {
        let meta = match cinemeta_meta(kind, id) {
            Ok(meta) => meta.unwrap_or_default(),
            Err(e) => {
                sdk::warn!("Cinemeta did not answer for {id}: {e}");
                return 1;
            }
        };
        let entry = self.known.entry(id.to_string()).or_default();
        entry.poster = meta.poster.map(|p| larger_metahub_poster(&p)).or(entry.poster.take());
        entry.background = meta.background.or(entry.background.take());
        if let Some(v) = video_id {
            let video = meta.videos.into_iter().find(|c| c.id == v).map(|c| KnownVideo { title: c.title(), thumbnail: c.thumbnail }).unwrap_or_default();
            entry.videos.clear();
            entry.videos.insert(v.to_string(), video);
        }
        1
    }

    /// Forgets titles no longer being watched, so the store stays the size of the row.
    fn keep<'a>(&mut self, ids: impl Iterator<Item = &'a str>) {
        let wanted: Vec<String> = ids.map(str::to_string).collect();
        self.known.retain(|id, _| wanted.iter().any(|w| w == id));
    }
}

/// Saves `value` under `key` when it fits the store's limit for one value, and otherwise
/// drops it, since a value too large would end the whole refresh.
fn save_within_limit<T: Serialize>(key: &str, value: &T) {
    match serde_json::to_vec(value) {
        Ok(json) if json.len() <= CACHE_BYTES => host::kv_set(key, &json),
        Ok(json) => {
            sdk::warn!("{key}: {} bytes is too much to keep; starting over", json.len());
            host::kv_delete(key);
        }
        Err(_) => {}
    }
}

#[derive(Deserialize)]
struct LastVideosAnswer {
    #[serde(default, rename = "metasDetailed", alias = "metas")]
    metas_detailed: Vec<LastVideosMeta>,
}

#[derive(Deserialize, Default)]
struct LastVideosMeta {
    #[serde(default)]
    id: String,
    #[serde(default)]
    videos: Vec<CinemetaVideo>,
}

#[derive(Serialize, Deserialize, Default, Clone)]
pub struct NewVideo {
    pub id: String,
    pub season: Option<u32>,
    pub episode: Option<u32>,
    pub title: String,
    pub thumbnail: Option<String>,
    pub released_ms: i64,
}

/// The latest episodes of the shows in the library, from one Cinemeta call that answers for
/// up to a hundred shows at once and serves for some hours.
#[derive(Serialize, Deserialize, Default)]
pub struct NewEpisodes {
    fetched_ms: i64,
    /// The ids asked about, so a changed library asks again.
    #[serde(default)]
    ids: Vec<String>,
    #[serde(default)]
    latest: HashMap<String, Vec<NewVideo>>,
}

impl NewEpisodes {
    fn for_shows(shows: &[&LibraryItem], now_ms: i64) -> Result<NewEpisodes, String> {
        let ids: Vec<String> = shows.iter().map(|it| it.id.clone()).collect();
        if ids.is_empty() {
            return Ok(NewEpisodes::default());
        }
        if let Some(kept) = host::kv_get(NEW_EPISODES_KEY).and_then(|v| serde_json::from_slice::<NewEpisodes>(&v).ok())
            && kept.ids == ids
            && now_ms - kept.fetched_ms < NEW_EPISODES_EVERY_MS
        {
            return Ok(kept);
        }
        let url = format!("{CINEMETA}/catalog/series/last-videos/lastVideosIds={}.json", ids.iter().map(|id| encode_component(id)).collect::<Vec<_>>().join(","));
        let r = sdk::fetch_online("Cinemeta", &HttpRequest::get(&url))?;
        if !r.ok() {
            return Err(format!("HTTP {}", r.status));
        }
        let a: LastVideosAnswer = serde_json::from_str(&r.body).map_err(|e| format!("unexpected answer: {e}"))?;
        let mut fresh = NewEpisodes { fetched_ms: now_ms, ids, latest: HashMap::new() };
        for m in a.metas_detailed.into_iter().filter(|m| !m.id.is_empty()) {
            // Nothing is new for a show never watched.
            let Some(since_ms) = shows.iter().find(|it| it.id == m.id).and_then(|it| it.last_watched_ms()) else { continue };
            fresh.latest.insert(m.id, latest_videos(m.videos, since_ms, now_ms + NEW_EPISODES_EVERY_MS));
        }
        save_within_limit(NEW_EPISODES_KEY, &fresh);
        Ok(fresh)
    }

    /// The first episode out since the show was last watched, as an up-next tile.
    fn item_for(&self, it: &LibraryItem, now_ms: i64) -> Option<Item> {
        let last_watched = it.last_watched_ms()?;
        let video = self.latest.get(&it.id)?.iter().find(|v| v.released_ms > last_watched && v.released_ms <= now_ms)?;
        Some(Item {
            id: format!("new:{}", it.id),
            kind: ItemKind::Resume,
            title: it.name.clone(),
            subtitle: Some(video.title.clone()).filter(|t| !t.is_empty()),
            season: video.season.map(|s| s.to_string()),
            episode: video.episode.map(|e| e.to_string()),
            last_used_ms: Some(video.released_ms),
            added_ms: Some(video.released_ms),
            is_new: sdk::new_this_week(Some(video.released_ms), now_ms),
            poster: it.poster(),
            thumbnail: video.thumbnail.clone(),
            action: open(&it.kind, &it.id, Some(&video.id), &it.name),
            ..Default::default()
        })
    }
}

/// A show's first few dated episodes out after `since_ms` and by `until_ms`, specials left out,
/// oldest first: all it takes to know what came out since it was last watched. Episodes
/// scheduled for later wait for the next answer.
fn latest_videos(videos: Vec<CinemetaVideo>, since_ms: i64, until_ms: i64) -> Vec<NewVideo> {
    let mut videos: Vec<NewVideo> = videos
        .into_iter()
        .filter(|v| v.season.unwrap_or(1) > 0)
        .filter_map(|v| Some(NewVideo { released_ms: v.released_ms()?, title: v.title(), id: v.id, season: v.season, episode: v.episode, thumbnail: v.thumbnail }))
        .filter(|v| v.released_ms > since_ms && v.released_ms <= until_ms)
        .collect();
    videos.sort_by_key(|v| v.released_ms);
    videos.truncate(5);
    videos
}

/// Movies, then shows, matching `query` in Cinemeta's catalogue, each opened with `open`.
pub fn cinemeta_search(query: &str, limit: usize, open: &dyn Fn(&CatalogMeta) -> Action) -> Result<Vec<Item>, String> {
    let query = query.trim();
    if query.is_empty() {
        return Ok(Vec::new());
    }
    let mut items = Vec::new();
    for kind in ["movie", "series"] {
        let url = format!("{CINEMETA}/catalog/{kind}/top/search={}.json", encode_component(query));
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

    fn library(json: &str) -> Vec<LibraryItem> {
        parse_items(serde_json::from_str(json).unwrap())
    }

    #[test]
    fn the_title_asked_for_comes_first() {
        let mut items: Vec<Item> = ["Breaking Bad Wolf", "Bad Breaking", "Breaking Bad", "Breaking Bad: Minisodes"].into_iter().map(|t| Item { title: t.into(), ..Default::default() }).collect();
        rank_matches(&mut items, "Breaking bad");
        let titles: Vec<&str> = items.iter().map(|it| it.title.as_str()).collect();
        assert_eq!(titles, ["Breaking Bad", "Breaking Bad Wolf", "Breaking Bad: Minisodes", "Bad Breaking"]);
    }

    #[test]
    fn stremios_continue_watching_rule_is_followed() {
        let items = library(
            r#"[{"_id":"tt1","name":"Paused film","type":"movie","removed":false,"temp":false,"_mtime":"2026-10-20T00:00:00Z","state":{"timeOffset":600000,"duration":6000000}},
                {"_id":"tt2","name":"Removed but temp","type":"movie","removed":true,"temp":true,"_mtime":"2026-10-19T00:00:00Z","state":{"timeOffset":1}},
                {"_id":"tt3","name":"Removed","type":"movie","removed":true,"temp":false,"_mtime":"2026-10-19T00:00:00Z","state":{"timeOffset":1}},
                {"_id":"tt4","name":"Never started","type":"series","removed":false,"temp":false,"_mtime":"2026-10-19T00:00:00Z","state":{"timeOffset":0}},
                {"_id":"ch1","name":"A channel","type":"tv","removed":false,"temp":false,"_mtime":"2026-10-19T00:00:00Z","state":{"timeOffset":5}},
                {"_id":"x1","name":"Other","type":"other","removed":false,"temp":false,"state":{"timeOffset":5}},
                {"_id":"bad","name":42}]"#,
        );
        assert_eq!(items.len(), 6);
        let watching: Vec<&str> = items.iter().filter(|it| it.in_continue_watching()).map(|it| it.id.as_str()).collect();
        assert_eq!(watching, ["tt1", "tt2"]);
        let saved: Vec<&str> = items.iter().filter(|it| it.in_library()).map(|it| it.id.as_str()).collect();
        assert_eq!(saved, ["tt1", "tt4", "ch1"]);
    }

    #[test]
    fn a_paused_episode_becomes_a_resume_tile() {
        let it = &library(
            r#"[{"_id":"tt0903747","name":"Breaking Bad","type":"series","removed":false,"temp":false,"_ctime":"2026-01-01T00:00:00Z","_mtime":"2026-10-20T00:00:00Z",
                 "poster":"https://images.metahub.space/poster/small/tt0903747/img",
                 "state":{"lastWatched":"2026-10-21T10:00:00Z","timeOffset":900000,"duration":1800000,"video_id":"tt0903747:2:4"}}]"#,
        )[0];
        let mut meta = MetaCache::default();
        meta.known.insert(
            "tt0903747".into(),
            KnownMeta {
                poster: None,
                background: Some("https://images.metahub.space/background/medium/tt0903747/img".into()),
                videos: [("tt0903747:2:4".to_string(), KnownVideo { title: "Down".into(), thumbnail: Some("https://episodes.metahub.space/tt0903747/2/4/w780.jpg".into()) })].into_iter().collect(),
            },
        );
        let item = resume_item(it, &meta);
        assert_eq!(item.id, "resume:tt0903747");
        assert_eq!(item.kind, ItemKind::Resume);
        assert_eq!(item.title, "Breaking Bad");
        assert_eq!(item.subtitle.as_deref(), Some("Down"));
        assert_eq!((item.season.as_deref(), item.episode.as_deref()), (Some("2"), Some("4")));
        assert_eq!(item.progress, Some(0.5));
        assert_eq!(item.position_ms, Some(900_000));
        assert_eq!(item.last_used_ms, parse_iso8601("2026-10-21T10:00:00Z"));
        assert_eq!(item.poster.as_deref(), Some("https://images.metahub.space/poster/medium/tt0903747/img"));
        assert_eq!(item.thumbnail.as_deref(), Some("https://episodes.metahub.space/tt0903747/2/4/w780.jpg"));
        assert_eq!(item.action.view.as_deref(), Some("intent:///detail/series/tt0903747/tt0903747%3A2%3A4#Intent;scheme=stremio;package=com.stremio.one;end"));
        assert_eq!(item.action.fallback_launch.as_deref(), Some(APP_PACKAGE));
    }

    #[test]
    fn an_episode_only_moved_on_to_is_up_next_not_half_watched() {
        let it = &library(r#"[{"_id":"tt1","name":"Show","type":"series","removed":false,"temp":false,"_mtime":"2026-10-20T00:00:00Z","state":{"timeOffset":1,"duration":2820000,"video_id":"tt1:1:8","lastWatched":"2026-10-20T00:00:00Z"}}]"#)[0];
        assert!(it.in_continue_watching());
        let item = resume_item(it, &MetaCache::default());
        assert_eq!(item.kind, ItemKind::Resume);
        assert_eq!((item.position_ms, item.duration_ms, item.progress), (None, None, None));
        assert_eq!((item.season.as_deref(), item.episode.as_deref()), (Some("1"), Some("8")));
        assert!(item.action.view.unwrap().contains("/tt1/tt1%3A1%3A8#"));
    }

    #[test]
    fn a_paused_film_opens_on_its_streams_and_an_empty_video_id_is_none() {
        let items = library(
            r#"[{"_id":"tt0133093","name":"The Matrix","type":"movie","removed":false,"temp":false,"_mtime":"2026-10-20T00:00:00Z","poster":"https://example.com/matrix.jpg","state":{"timeOffset":100,"duration":0,"video_id":"tt0133093"}},
                {"_id":"tt9","name":"Blank","type":"series","removed":false,"temp":false,"state":{"timeOffset":100,"video_id":""}}]"#,
        );
        let item = resume_item(&items[0], &MetaCache::default());
        assert_eq!(item.season, None);
        assert_eq!(item.subtitle, None);
        assert_eq!(item.duration_ms, None);
        assert_eq!(item.progress, None);
        assert_eq!(item.position_ms, Some(100));
        assert_eq!(item.poster.as_deref(), Some("https://example.com/matrix.jpg"));
        assert_eq!(item.thumbnail, None);
        assert_eq!(item.last_used_ms, parse_iso8601("2026-10-20T00:00:00Z"));
        assert_eq!(item.action.view.as_deref(), Some("intent:///detail/movie/tt0133093/tt0133093#Intent;scheme=stremio;package=com.stremio.one;end"));
        assert_eq!(items[1].video_id(), None);
        assert_eq!(resume_item(&items[1], &MetaCache::default()).action.view.as_deref(), Some("intent:///detail/series/tt9#Intent;scheme=stremio;package=com.stremio.one;end"));
    }

    #[test]
    fn library_tiles_are_most_recently_watched_first() {
        let items = library(
            r#"[{"_id":"tt5","name":"Old favourite","type":"movie","removed":false,"temp":false,"_ctime":"2025-01-01T00:00:00Z","_mtime":"2026-10-01T00:00:00Z","state":{"lastWatched":"2026-10-19T00:00:00Z"}},
                {"_id":"tt6","name":"Just saved","type":"movie","removed":false,"temp":false,"_ctime":"2026-10-18T00:00:00Z","_mtime":"2026-10-18T00:00:00Z","state":{}}]"#,
        );
        let mut saved: Vec<&LibraryItem> = items.iter().collect();
        saved.sort_by_key(|it| std::cmp::Reverse(it.last_used_ms().or_else(|| it.added_ms())));
        assert_eq!(saved[0].id, "tt5");
        let item = library_item(saved[0], 7);
        assert_eq!(item.id, "lib:tt5");
        assert_eq!(item.row.as_deref(), Some("library"));
        assert_eq!(item.weight, 7);
        assert!(!item.is_new);
        assert_eq!(item.added_ms, parse_iso8601("2025-01-01T00:00:00Z"));
        assert!(<Stremio as sdk::Plugin>::manifest().rows.iter().any(|r| r.id == "library"));
    }

    #[test]
    fn only_shows_watched_once_and_not_muted_are_watched_for_new_episodes() {
        let items = library(
            r#"[{"_id":"tt1","name":"Watched","type":"series","removed":false,"temp":false,"state":{"lastWatched":"2026-10-01T00:00:00Z"}},
                {"_id":"tt2","name":"Never watched","type":"series","removed":false,"temp":false,"state":{}},
                {"_id":"tt3","name":"Muted","type":"series","removed":false,"temp":false,"state":{"lastWatched":"2026-10-01T00:00:00Z","noNotif":true}},
                {"_id":"tt4","name":"Film","type":"movie","removed":false,"temp":false,"state":{"lastWatched":"2026-10-01T00:00:00Z"}},
                {"_id":"tt5","name":"Removed","type":"series","removed":true,"temp":false,"state":{"lastWatched":"2026-10-01T00:00:00Z"}},
                {"_id":"tt6","name":"One video","type":"series","removed":false,"temp":false,"state":{"lastWatched":"2026-10-01T00:00:00Z"},"behaviorHints":{"defaultVideoId":"tt6"}}]"#,
        );
        let wanted: Vec<&str> = items.iter().filter(|it| it.wants_new_episodes()).map(|it| it.id.as_str()).collect();
        assert_eq!(wanted, ["tt1"]);
    }

    #[test]
    fn a_new_episode_since_the_last_watch_is_up_next() {
        let it = &library(r#"[{"_id":"tt1","name":"Show","type":"series","removed":false,"temp":false,"poster":"https://p/1.jpg","state":{"lastWatched":"2026-10-10T00:00:00Z"}}]"#)[0];
        let a: LastVideosAnswer = serde_json::from_str(
            r#"{"metasDetailed":[{"id":"tt1","name":"Show","videos":[
                {"id":"tt1:1:1","name":"Old","season":1,"number":1,"episode":1,"released":"2026-10-01T00:00:00Z"},
                {"id":"tt1:1:2","name":"Fresh","season":1,"number":2,"episode":2,"released":"2026-10-15T00:00:00Z","thumbnail":"https://t/2.jpg"},
                {"id":"tt1:1:3","name":"Soon","season":1,"number":3,"episode":3,"released":"2026-12-01T00:00:00Z"},
                {"id":"tt1:0:1","name":"Special","season":0,"number":1,"episode":1,"released":"2026-10-16T00:00:00Z"}]}]}"#,
        )
        .unwrap();
        let now = parse_iso8601("2026-10-20T00:00:00Z").unwrap();
        let since = it.last_watched_ms().unwrap();
        let mut latest = NewEpisodes::default();
        for m in a.metas_detailed {
            latest.latest.insert(m.id, latest_videos(m.videos, since, now + NEW_EPISODES_EVERY_MS));
        }
        assert_eq!(latest.latest["tt1"].iter().map(|v| v.id.as_str()).collect::<Vec<_>>(), ["tt1:1:2"]);
        let item = latest.item_for(it, now).unwrap();
        assert_eq!(item.id, "new:tt1");
        assert_eq!(item.kind, ItemKind::Resume);
        assert_eq!(item.subtitle.as_deref(), Some("Fresh"));
        assert_eq!((item.season.as_deref(), item.episode.as_deref()), (Some("1"), Some("2")));
        assert_eq!((item.position_ms, item.progress), (None, None));
        assert_eq!(item.last_used_ms, parse_iso8601("2026-10-15T00:00:00Z"));
        assert!(item.is_new);
        assert_eq!(item.thumbnail.as_deref(), Some("https://t/2.jpg"));
        assert!(item.action.view.unwrap().contains("/tt1/tt1%3A1%3A2#"));
        let caught_up = &library(r#"[{"_id":"tt1","name":"Show","type":"series","removed":false,"temp":false,"state":{"lastWatched":"2026-10-16T00:00:00Z"}}]"#)[0];
        assert!(latest.item_for(caught_up, now).is_none());

        // Mid-season, the weeks already scheduled do not push out the episode that is out.
        let weekly: Vec<String> = ["2026-10-15", "2026-10-22", "2026-10-29", "2026-11-05", "2026-11-12", "2026-11-19", "2026-11-26"]
            .iter()
            .enumerate()
            .map(|(i, day)| format!(r#"{{"id":"tt1:1:{n}","season":1,"episode":{n},"released":"{day}T00:00:00Z"}}"#, n = i + 2))
            .collect();
        let videos: Vec<CinemetaVideo> = serde_json::from_str(&format!("[{}]", weekly.join(","))).unwrap();
        let kept = latest_videos(videos, since, now + NEW_EPISODES_EVERY_MS);
        assert_eq!(kept.iter().map(|v| v.id.as_str()).collect::<Vec<_>>(), ["tt1:1:2"]);
    }

    #[test]
    fn cinemetas_videos_are_read_with_both_their_episode_fields() {
        let a: MetaAnswer = serde_json::from_str(r#"{"meta":{"id":"tt1","poster":"p","videos":[{"id":"tt1:1:1","name":"Pilot","season":1,"number":1,"episode":1,"thumbnail":"t","released":"2008-01-21T05:00:00.000Z"}]}}"#).unwrap();
        let v = &a.meta.unwrap().videos[0];
        assert_eq!((v.title().as_str(), v.season, v.episode), ("Pilot", Some(1), Some(1)));
        assert_eq!(v.released_ms(), parse_iso8601("2008-01-21T05:00:00Z"));
        let other: CinemetaVideo = serde_json::from_str(r#"{"id":"x","title":"Named otherwise"}"#).unwrap();
        assert_eq!(other.title(), "Named otherwise");
    }

    #[test]
    fn video_ids_give_season_and_episode() {
        assert_eq!(episode_of("tt0903747:2:4"), Some((2, 4)));
        assert_eq!(episode_of("kitsu:1234:1:12"), Some((1, 12)));
        assert_eq!(episode_of("tt0133093"), None);
        assert_eq!(episode_of("2:4"), None);
        assert_eq!(episode_of("tt1:a:4"), None);
    }

    #[test]
    fn an_id_cannot_add_to_the_intent() {
        let view = open("movie", "tt1;component=x/.Y;end", None, "x").view.unwrap();
        assert_eq!(view, "intent:///detail/movie/tt1%3Bcomponent%3Dx%2F.Y%3Bend#Intent;scheme=stremio;package=com.stremio.one;end");
        assert_eq!(encode_component("a b#c?d"), "a%20b%23c%3Fd");
    }

    #[test]
    fn the_cache_asks_once_per_title_and_once_per_new_episode() {
        let mut cache = MetaCache::default();
        assert!(cache.needs("tt1", Some("tt1:1:1")));
        assert!(!cache.needs("kitsu:1", Some("kitsu:1:1:1")));
        cache.known.insert("tt1".into(), KnownMeta { videos: [("tt1:1:1".to_string(), KnownVideo::default())].into_iter().collect(), ..Default::default() });
        assert!(!cache.needs("tt1", Some("tt1:1:1")));
        assert!(cache.needs("tt1", Some("tt1:1:2")));
        assert!(!cache.needs("tt1", None));
        cache.known.insert("tt2".into(), KnownMeta::default());
        cache.keep(["tt1"].into_iter());
        assert!(cache.get("tt2").is_none());
        assert!(cache.get("tt1").is_some());
        // Off the TV every fetch fails, and a failed lookup is not remembered as an answer.
        assert_eq!(cache.fetch("series", "tt3", Some("tt3:1:1")), 1);
        assert!(cache.get("tt3").is_none());
    }

    #[test]
    fn stremios_envelope_is_read_either_way() {
        let ok: Result<AuthResult, ApiError> = answer(r#"{"result":{"authKey":"k","user":{}}}"#, "login").unwrap();
        assert_eq!(ok.unwrap().auth_key, "k");
        let refused: Result<AuthResult, ApiError> = answer(r#"{"error":{"code":2,"message":"User not found","wrongEmail":true}}"#, "login").unwrap();
        let e = refused.err().unwrap();
        assert_eq!((e.code, e.message.as_str()), (2, "User not found"));
        let code: Result<LinkCode, ApiError> = answer(r#"{"result":{"success":true,"code":"47X0","link":"https://link.stremio.com/47X0","qrcode":"https://link.stremio.com/qr"}}"#, "create").unwrap();
        assert_eq!(code.unwrap().code, "47X0");
        assert!(answer::<AuthResult>("{}", "login").is_err());
        assert!(answer::<AuthResult>("not json", "login").is_err());
    }

    #[test]
    fn metahub_posters_are_asked_for_in_medium() {
        assert_eq!(larger_metahub_poster("https://images.metahub.space/poster/small/tt1/img"), "https://images.metahub.space/poster/medium/tt1/img");
        assert_eq!(larger_metahub_poster("https://example.com/p.jpg"), "https://example.com/p.jpg");
    }

    #[test]
    fn errors_say_what_to_check() {
        // Off the TV there is no host, so a fetch fails as an unreachable service would.
        assert_eq!(post("login", serde_json::json!({})).err().as_deref(), Some("Stremio didn't answer. Check the TV's internet connection, then try again."));
        assert_eq!(cinemeta_search("  ", 12, &|_| Action::default()).unwrap().len(), 0);
        assert_eq!(instruction("47X0"), "Sign in: on your phone open link.stremio.com/47X0 and sign in to Stremio, then press Refresh now here.");
        assert!(matches!(sign_in_method(), SignIn::Code));
        assert_eq!(sign_in_with_password("a@b.c").err().as_deref(), Some("Password is not set. Leave the email empty too to sign in with a code instead."));
    }
}
