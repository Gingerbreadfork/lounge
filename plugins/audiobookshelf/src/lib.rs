//! Audiobookshelf plugin: books and podcast episodes in progress join Continue watching, with the
//! next book of a series as up next, and recently added books and new episodes get rows of their
//! own. A press starts a stream on the server for a player app such as VLC, from the place the
//! server keeps; the player never tells the server where it stopped.

use lounge_plugin_sdk::{self as sdk, host, Action, HttpRequest, HttpResponse, Item, ItemKind, Kind, Manifest, RefreshContext, RowSpec, RowStyle, SettingKind, SettingSpec};
use serde::Deserialize;
use std::collections::{BTreeMap, HashMap};

pub struct Audiobookshelf;

const SERVICE: &str = "Audiobookshelf";
const APP_PACKAGE: &str = "com.audiobookshelf.app";
/// How many items each list asks the server for.
const LIMIT: u32 = 24;
/// The id of the stream the last press started.
const STREAM_KEY: &str = "stream";
const NO_SIGN_IN: &str = "Add a username and password, or an API key, in the plugin's settings.";
const REJECTED: &str = "Audiobookshelf rejected the username or password. Check them in the plugin's settings.";
const KEY_REFUSED: &str = "Audiobookshelf didn't accept the API key. Check it in the plugin's settings, or make a new one.";
const TOO_MANY: &str = "Audiobookshelf is turning sign-ins away after too many tries. Wait a few minutes, then try again.";
const GONE: &str = "Audiobookshelf no longer has that. Press Refresh now on the plugin's page to bring its rows up to date.";

impl sdk::Plugin for Audiobookshelf {
    fn manifest() -> Manifest {
        Manifest {
            id: "tv.lounge.audiobookshelf".into(),
            name: "Audiobookshelf".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            description: "Continue listening and what's new on your Audiobookshelf server, played in a player app such as VLC, though your place on the server doesn't move when you listen on the TV.".into(),
            publisher_name: "Lounge".into(),
            kinds: vec![Kind::Content, Kind::Search],
            fetch_hosts: vec!["$server".into()],
            refresh_minutes: 30,
            rows: vec![
                RowSpec { id: "recent".into(), title: "Recently added on Audiobookshelf".into(), style: RowStyle::Landscape },
                RowSpec { id: "episodes".into(), title: "New episodes on Audiobookshelf".into(), style: RowStyle::Landscape },
            ],
            settings: vec![
                SettingSpec { key: "server".into(), label: "Server address".into(), kind: SettingKind::Url, required: true, help: Some("Like http://192.168.0.10:13378".into()), default: None },
                SettingSpec { key: "username".into(), label: "Username".into(), kind: SettingKind::Text, required: false, help: Some("Leave it empty when you use an API key instead.".into()), default: None },
                SettingSpec { key: "password".into(), label: "Password".into(), kind: SettingKind::Password, required: false, help: None, default: None },
                SettingSpec {
                    key: "api_key".into(),
                    label: "API key".into(),
                    kind: SettingKind::Password,
                    required: false,
                    help: Some("Instead of a username and password. An admin makes one in Audiobookshelf under Settings, then API Keys, and it is easiest pasted from the phone companion.".into()),
                    default: None,
                },
            ],
            homepage: Some("https://www.audiobookshelf.org".into()),
            self_signed_servers: false,
            for_apps: vec![APP_PACKAGE.into()],
            sent_to: Default::default(),
        }
    }

    fn refresh(ctx: &RefreshContext) -> Result<Vec<Item>, String> {
        let mut s = Session::connect(true)?.ok_or_else(|| NO_SIGN_IN.to_string())?;
        let me: Me = s.get_json("/api/me")?;
        let progress = Progresses::new(each(me.media_progress));
        let in_progress: InProgress = s.get_json(&format!("/api/me/items-in-progress?limit={LIMIT}"))?;
        let mut items: Vec<Item> = each::<LibraryItem>(in_progress.library_items).iter().filter_map(|it| s.resume_item(it, &progress)).collect();
        let libraries: Libraries = s.get_json("/api/libraries")?;
        for library in &libraries.libraries {
            let shelves: Vec<Shelf> = s.get_json(&format!("/api/libraries/{}/personalized?limit={LIMIT}", enc(&library.id)))?;
            for shelf in shelves {
                // Shelves of authors and series hold other things, so each entity is read on its own.
                let entities = each::<LibraryItem>(shelf.entities).into_iter();
                match (library.media_type.as_str(), shelf.id.as_str()) {
                    ("book", "continue-series") => items.extend(entities.filter_map(|it| s.next_item(&it))),
                    ("book", "recently-added") => items.extend(entities.filter_map(|it| s.recent_item(&it, ctx.now_ms))),
                    ("podcast", "newest-episodes") => items.extend(entities.filter_map(|it| s.episode_item(&it, ctx.now_ms))),
                    _ => {}
                }
            }
        }
        sdk::dedup_ids(&mut items);
        sdk::info!("{} items from {}", items.len(), sdk::redact_url(&s.server));
        Ok(items)
    }

    /// A search never signs in, so typing cannot run into the server's limit on sign-in attempts;
    /// that is left to the refresh.
    fn search(query: &str, limit: u32) -> Result<Vec<Item>, String> {
        let Some(mut s) = Session::connect(false)? else { return Ok(Vec::new()) };
        let libraries: Libraries = s.get_json("/api/libraries")?;
        let now = host::now_ms();
        let mut items = Vec::new();
        let mut failed = None;
        for library in &libraries.libraries {
            match s.get_json::<Found>(&format!("/api/libraries/{}/search?q={}&limit={}", enc(&library.id), enc(query), limit.clamp(1, 25))) {
                Ok(found) => {
                    items.extend(found.book.iter().filter_map(|hit| s.recent_item(&hit.library_item, now)));
                    items.extend(found.episodes.iter().filter_map(|hit| s.episode_item(&hit.library_item, now)));
                }
                Err(e) => {
                    sdk::warn!("one library wasn't searched: {e}");
                    failed.get_or_insert(e);
                }
            }
        }
        if items.is_empty()
            && let Some(e) = failed
        {
            return Err(e);
        }
        items.truncate(limit as usize);
        Ok(items)
    }

    /// Starts a stream of the whole book or episode on the server, from the place it keeps. One
    /// address plays a book of many files straight through, and it needs no sign-in, so no secret
    /// goes to the player.
    fn resolve(action: &str) -> Result<Action, String> {
        let path = play_path(action).ok_or_else(|| GONE.to_string())?;
        let mut s = Session::connect(true)?.ok_or_else(|| NO_SIGN_IN.to_string())?;
        s.close_last_stream();
        let body = serde_json::json!({
            "deviceInfo": { "clientName": "Lounge", "clientVersion": env!("CARGO_PKG_VERSION"), "deviceId": device_id() },
            "forceTranscode": true,
            "mediaPlayer": "Lounge",
        });
        let r = s.call("POST", &path, Some(body.to_string()))?;
        match r.status {
            404 => return Err(GONE.into()),
            _ if !r.ok() => return Err(sdk::answered_error(SERVICE, &s.server, r.status)),
            _ => {}
        }
        let session: PlaySession = serde_json::from_str(&r.body).map_err(|e| {
            sdk::warn!("unexpected playback session: {e}");
            sdk::unreadable(SERVICE, &s.server)
        })?;
        let Some(track) = session.audio_tracks.first().filter(|t| t.content_url.starts_with('/')) else {
            sdk::warn!("the playback session has no stream on the server");
            return Err(sdk::unreadable(SERVICE, &s.server));
        };
        host::kv_set(STREAM_KEY, session.id.as_bytes());
        Ok(Action {
            play: Some(format!("{}{}", s.server, track.content_url)),
            title: session.display_title.clone(),
            start_ms: session.current_time.map(seconds_ms).filter(|&t| t > 0),
            ..Default::default()
        })
    }
}

sdk::export_plugin!(Audiobookshelf);

/// This TV's device id for the server's list of devices, made the first time and kept.
fn device_id() -> String {
    host::kv_get_string("device_id").unwrap_or_else(|| {
        let id = format!("lounge-{:x}", host::now_ms());
        host::kv_set("device_id", id.as_bytes());
        id
    })
}

/// How the plugin signs in: with an API key, or as a user whose tokens it keeps and renews.
enum Auth {
    Key,
    User { username: String, owner: String },
}

struct Session {
    server: String,
    bearer: String,
    auth: Auth,
    /// Whether a password sign-in may be tried when the kept tokens no longer serve.
    may_sign_in: bool,
}

#[derive(Deserialize)]
struct SignedIn {
    user: Tokens,
}

/// What a sign-in hands back: an access token and a refresh token from 2.26 on, and before that
/// one token that lasts.
#[derive(Deserialize, Default)]
struct Tokens {
    #[serde(rename = "accessToken")]
    access_token: Option<String>,
    #[serde(rename = "refreshToken")]
    refresh_token: Option<String>,
    token: Option<String>,
}

impl Session {
    /// An API key is used as it is. A user's kept tokens are used while they belong to the same
    /// server and username; otherwise the plugin signs in when [may_sign_in] allows, and there is
    /// no session when it does not.
    fn connect(may_sign_in: bool) -> Result<Option<Session>, String> {
        let server = host::server_setting("server", "server address")?;
        if let Some(key) = host::secret("api_key").map(|k| k.trim().to_string()).filter(|k| !k.is_empty()) {
            return Ok(Some(Session { server, bearer: key, auth: Auth::Key, may_sign_in }));
        }
        let Some(username) = host::setting("username").map(|u| u.trim().to_string()) else { return Ok(None) };
        let owner = format!("{server}\n{username}");
        let kept = if host::kv_get_string("session_for").as_deref() == Some(owner.as_str()) { host::kv_get_string("access") } else { None };
        let mut s = Session { server, bearer: kept.clone().unwrap_or_default(), auth: Auth::User { username, owner }, may_sign_in };
        if kept.is_none() {
            if !may_sign_in {
                return Ok(None);
            }
            s.sign_in()?;
        }
        Ok(Some(s))
    }

    fn sign_in(&mut self) -> Result<(), String> {
        let Auth::User { username, owner } = &self.auth else { return Err(KEY_REFUSED.into()) };
        let owner = owner.clone();
        let body = serde_json::json!({ "username": username, "password": host::secret("password").unwrap_or_default() }).to_string();
        let mut req = request("POST", format!("{}/login", self.server), None, Some(body));
        req.headers.insert("x-return-tokens".into(), "true".into());
        let r = sdk::fetch_from(SERVICE, &self.server, &req)?;
        match r.status {
            401 => return Err(REJECTED.into()),
            404 => return Err(sdk::wrong_server(SERVICE, &self.server)),
            429 => return Err(TOO_MANY.into()),
            _ if !r.ok() => return Err(sdk::answered_error(SERVICE, &self.server, r.status)),
            _ => {}
        }
        let signed: SignedIn = serde_json::from_str(&r.body).map_err(|e| {
            sdk::warn!("unexpected sign-in answer: {e}");
            sdk::wrong_server(SERVICE, &self.server)
        })?;
        self.keep(signed.user, &owner)
    }

    /// A new access token from the refresh token, or failing that a fresh sign-in.
    fn renew(&mut self) -> Result<(), String> {
        let Auth::User { owner, .. } = &self.auth else { return Err(KEY_REFUSED.into()) };
        let owner = owner.clone();
        if let Some(refresh) = host::kv_get_string("refresh") {
            let mut req = request("POST", format!("{}/auth/refresh", self.server), None, None);
            req.headers.insert("x-refresh-token".into(), refresh);
            let r = sdk::fetch_from(SERVICE, &self.server, &req)?;
            if r.ok()
                && let Ok(signed) = serde_json::from_str::<SignedIn>(&r.body)
            {
                return self.keep(signed.user, &owner);
            }
            sdk::warn!("the refresh token was turned away with HTTP {}", r.status);
        }
        if !self.may_sign_in {
            return Err(REJECTED.into());
        }
        self.sign_in()
    }

    /// Remembers a sign-in for the server and user it was made for.
    fn keep(&mut self, tokens: Tokens, owner: &str) -> Result<(), String> {
        let Some(access) = tokens.access_token.or(tokens.token).filter(|t| !t.is_empty()) else {
            return Err(sdk::wrong_server(SERVICE, &self.server));
        };
        host::kv_set("access", access.as_bytes());
        match tokens.refresh_token.filter(|t| !t.is_empty()) {
            Some(refresh) => host::kv_set("refresh", refresh.as_bytes()),
            None => host::kv_delete("refresh"),
        }
        host::kv_set("session_for", owner.as_bytes());
        self.bearer = access;
        Ok(())
    }

    /// A request to the server. An access token that has run out is renewed once and the request
    /// sent again.
    fn call(&mut self, method: &str, path: &str, body: Option<String>) -> Result<HttpResponse, String> {
        let url = format!("{}{path}", self.server);
        let r = sdk::fetch_from(SERVICE, &self.server, &request(method, url.clone(), Some(&self.bearer), body.clone()))?;
        if r.status != 401 {
            return Ok(r);
        }
        if matches!(self.auth, Auth::Key) {
            return Err(KEY_REFUSED.into());
        }
        self.renew()?;
        let r = sdk::fetch_from(SERVICE, &self.server, &request(method, url, Some(&self.bearer), body))?;
        if r.status == 401 {
            return Err(sdk::answered_error(SERVICE, &self.server, r.status));
        }
        Ok(r)
    }

    fn get_json<T: serde::de::DeserializeOwned>(&mut self, path: &str) -> Result<T, String> {
        let r = self.call("GET", path, None)?;
        match r.status {
            404 => return Err(sdk::wrong_server(SERVICE, &self.server)),
            _ if !r.ok() => {
                sdk::warn!("HTTP {} for {}", r.status, without_query(path));
                return Err(sdk::answered_error(SERVICE, &self.server, r.status));
            }
            _ => {}
        }
        serde_json::from_str(&r.body).map_err(|e| {
            sdk::warn!("unexpected answer for {}: {e}", without_query(path));
            sdk::unreadable(SERVICE, &self.server)
        })
    }

    /// Closes the stream the last press started, so the server drops its copy. Closing without a
    /// position leaves the place the server keeps alone, and a stream already gone is no matter.
    fn close_last_stream(&mut self) {
        let Some(id) = host::kv_get_string(STREAM_KEY) else { return };
        host::kv_delete(STREAM_KEY);
        if let Err(e) = self.call("POST", &format!("/api/session/{}/close", enc(&id)), None) {
            sdk::warn!("the last stream wasn't closed: {e}");
        }
    }

    /// Covers need no sign-in. They go in `poster`, which the Stage frames whole where it would
    /// crop a `thumbnail` to 16:9, and the item's last change is in the address so a new cover
    /// is fetched again.
    fn cover(&self, it: &LibraryItem) -> Option<String> {
        it.media.cover_path.as_ref()?;
        Some(format!("{}/api/items/{}/cover?width=800&format=jpeg&ts={}", self.server, enc(&it.id), it.updated_at.unwrap_or(0)))
    }

    /// A book or episode in progress, for Continue watching. One that is finished, hidden from
    /// Continue Listening, only read as an ebook, without audio or without a title is left out.
    fn resume_item(&self, it: &LibraryItem, progress: &Progresses) -> Option<Item> {
        if !it.playable() {
            return None;
        }
        let episode = it.recent_episode.as_ref().filter(|_| it.media_type == "podcast");
        if it.media_type == "podcast" && episode.is_none() {
            return None;
        }
        let p = progress.get(&it.id, episode.map(|e| e.id.as_str()));
        if p.is_some_and(|p| p.is_finished || p.hidden || p.only_read()) {
            return None;
        }
        let (play, title, subtitle, length) = match episode {
            Some(e) => (format!("episode:{}:{}", it.id, e.id), e.title.clone()?, it.media.metadata.title.clone(), e.length()),
            None => (format!("book:{}", it.id), it.media.metadata.title.clone()?, it.media.metadata.author(), it.media.duration),
        };
        Some(Item {
            id: format!("resume:{play}"),
            kind: ItemKind::Resume,
            subtitle,
            duration_ms: p.and_then(|p| p.duration).or(length).map(seconds_ms).filter(|&d| d > 0),
            position_ms: p.and_then(|p| p.current_time).map(seconds_ms).filter(|&t| t > 0),
            last_used_ms: it.progress_last_update.or(p.and_then(|p| p.last_update)),
            poster: self.cover(it),
            action: play_action(&play, &title),
            title,
            ..Default::default()
        })
    }

    /// The next book of a series being listened to, which joins Continue watching as up next.
    fn next_item(&self, it: &LibraryItem) -> Option<Item> {
        if !it.playable() {
            return None;
        }
        let title = it.media.metadata.title.clone()?;
        let play = format!("book:{}", it.id);
        Some(Item {
            id: format!("next:{play}"),
            kind: ItemKind::Resume,
            subtitle: it.media.metadata.series().or_else(|| it.media.metadata.author()),
            duration_ms: it.media.duration.map(seconds_ms).filter(|&d| d > 0),
            poster: self.cover(it),
            action: play_action(&play, &title),
            title,
            ..Default::default()
        })
    }

    fn recent_item(&self, it: &LibraryItem, now_ms: i64) -> Option<Item> {
        if !it.playable() {
            return None;
        }
        let title = it.media.metadata.title.clone()?;
        let play = format!("book:{}", it.id);
        Some(Item {
            id: format!("new:{play}"),
            kind: ItemKind::Program,
            subtitle: it.media.metadata.author(),
            added_ms: it.added_at,
            is_new: sdk::new_this_week(it.added_at, now_ms),
            poster: self.cover(it),
            row: Some("recent".into()),
            action: play_action(&play, &title),
            title,
            ..Default::default()
        })
    }

    /// A podcast's newest episode, dated by when it was published.
    fn episode_item(&self, it: &LibraryItem, now_ms: i64) -> Option<Item> {
        let e = it.recent_episode.as_ref()?;
        let title = e.title.clone()?;
        let play = format!("episode:{}:{}", it.id, e.id);
        let show = it.media.metadata.title.clone().filter(|s| !s.is_empty());
        let subtitle = match (show, e.length().map(seconds_ms).filter(|&d| d > 0)) {
            (Some(show), Some(ms)) => Some(format!("{show}, {}", length_text(ms))),
            (show, ms) => show.or_else(|| ms.map(length_text)),
        };
        Some(Item {
            id: format!("new:{play}"),
            kind: ItemKind::Program,
            subtitle,
            added_ms: e.published_at,
            is_new: sdk::new_this_week(e.published_at, now_ms),
            poster: self.cover(it),
            row: Some("episodes".into()),
            action: play_action(&play, &title),
            title,
            ..Default::default()
        })
    }
}

fn request(method: &str, url: String, bearer: Option<&str>, body: Option<String>) -> HttpRequest {
    let mut headers = BTreeMap::new();
    headers.insert("Accept".to_string(), "application/json".to_string());
    if let Some(token) = bearer {
        headers.insert("Authorization".to_string(), format!("Bearer {token}"));
    }
    if body.is_some() {
        headers.insert("Content-Type".to_string(), "application/json".to_string());
    }
    HttpRequest { method: method.into(), url, headers, body, timeout_ms: 0 }
}

fn play_action(play: &str, title: &str) -> Action {
    Action { resolve: Some(play.to_string()), title: Some(title.to_string()), ..Default::default() }
}

/// Where on the server a tile's action starts a stream, or nothing for an action this plugin did not make.
fn play_path(action: &str) -> Option<String> {
    let ok = |id: &&str| !id.is_empty();
    match action.split(':').collect::<Vec<_>>().as_slice() {
        ["book", id] if ok(id) => Some(format!("/api/items/{}/play", enc(id))),
        ["episode", id, episode] if ok(id) && ok(episode) => Some(format!("/api/items/{}/play/{}", enc(id), enc(episode))),
        _ => None,
    }
}

/// Each entry of a list that reads as `T`; one that does not is passed over rather than failing the rest.
fn each<T: serde::de::DeserializeOwned>(entries: Vec<serde_json::Value>) -> Vec<T> {
    entries.into_iter().filter_map(|e| serde_json::from_value(e).ok()).collect()
}

fn without_query(path: &str) -> &str {
    path.split('?').next().unwrap_or(path)
}

fn seconds_ms(seconds: f64) -> i64 {
    if seconds.is_finite() && seconds > 0.0 { (seconds * 1000.0).round() as i64 } else { 0 }
}

/// A length in words, such as "1 hour 5 minutes".
fn length_text(ms: i64) -> String {
    let minutes = (ms + 30_000) / 60_000;
    let count = |n: i64, one: &str, many: &str| format!("{n} {}", if n == 1 { one } else { many });
    match (minutes / 60, minutes % 60) {
        (0, m) => count(m.max(1), "minute", "minutes"),
        (h, 0) => count(h, "hour", "hours"),
        (h, m) => format!("{} {}", count(h, "hour", "hours"), count(m, "minute", "minutes")),
    }
}

fn enc(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[derive(Deserialize, Default)]
struct Me {
    #[serde(rename = "mediaProgress", default)]
    media_progress: Vec<serde_json::Value>,
}

#[derive(Deserialize, Default, Clone)]
pub struct Progress {
    #[serde(rename = "libraryItemId")]
    library_item_id: Option<String>,
    #[serde(rename = "episodeId")]
    episode_id: Option<String>,
    #[serde(rename = "currentTime")]
    current_time: Option<f64>,
    duration: Option<f64>,
    #[serde(rename = "isFinished", default)]
    is_finished: bool,
    #[serde(rename = "hideFromContinueListening", default)]
    hidden: bool,
    #[serde(rename = "ebookProgress")]
    ebook_progress: Option<f64>,
    #[serde(rename = "lastUpdate")]
    last_update: Option<i64>,
}

impl Progress {
    /// Whether the place is in the book's text alone, with nothing listened to yet.
    fn only_read(&self) -> bool {
        self.current_time.unwrap_or(0.0) <= 0.0 && self.ebook_progress.unwrap_or(0.0) > 0.0
    }
}

/// The user's place in each book and episode.
struct Progresses(HashMap<(String, String), Progress>);

impl Progresses {
    fn new(all: Vec<Progress>) -> Progresses {
        Progresses(all.into_iter().filter_map(|p| Some(((p.library_item_id.clone()?, p.episode_id.clone().unwrap_or_default()), p))).collect())
    }

    fn get(&self, item: &str, episode: Option<&str>) -> Option<&Progress> {
        self.0.get(&(item.to_string(), episode.unwrap_or_default().to_string()))
    }
}

#[derive(Deserialize, Default)]
struct Libraries {
    #[serde(default)]
    libraries: Vec<Library>,
}

#[derive(Deserialize)]
struct Library {
    id: String,
    #[serde(rename = "mediaType", default)]
    media_type: String,
}

#[derive(Deserialize, Default)]
struct InProgress {
    #[serde(rename = "libraryItems", default)]
    library_items: Vec<serde_json::Value>,
}

#[derive(Deserialize)]
struct Shelf {
    id: String,
    #[serde(default)]
    entities: Vec<serde_json::Value>,
}

#[derive(Deserialize, Default)]
struct Found {
    #[serde(default)]
    book: Vec<Hit>,
    #[serde(default)]
    episodes: Vec<Hit>,
}

#[derive(Deserialize)]
struct Hit {
    #[serde(rename = "libraryItem")]
    library_item: LibraryItem,
}

#[derive(Deserialize, Default, Clone)]
pub struct LibraryItem {
    #[serde(default)]
    id: String,
    #[serde(rename = "mediaType", default)]
    media_type: String,
    #[serde(default)]
    media: Media,
    #[serde(rename = "addedAt")]
    added_at: Option<i64>,
    #[serde(rename = "updatedAt")]
    updated_at: Option<i64>,
    #[serde(rename = "progressLastUpdate")]
    progress_last_update: Option<i64>,
    #[serde(rename = "recentEpisode")]
    recent_episode: Option<Episode>,
    #[serde(rename = "isMissing", default)]
    is_missing: bool,
}

impl LibraryItem {
    /// Whether there is audio to play: a book's tracks are counted in the short form the lists
    /// give and listed in the long form search gives, and one known to have none is left out.
    fn playable(&self) -> bool {
        if self.is_missing {
            return false;
        }
        if self.media_type == "podcast" {
            return true;
        }
        let m = &self.media;
        match (m.num_tracks.or(m.num_audio_files), &m.audio_files) {
            (Some(n), _) => n > 0,
            (None, Some(files)) => !files.is_empty(),
            (None, None) => true,
        }
    }
}

#[derive(Deserialize, Default, Clone)]
struct Media {
    #[serde(default)]
    metadata: Metadata,
    #[serde(rename = "coverPath")]
    cover_path: Option<String>,
    duration: Option<f64>,
    #[serde(rename = "numTracks")]
    num_tracks: Option<i64>,
    #[serde(rename = "numAudioFiles")]
    num_audio_files: Option<i64>,
    #[serde(rename = "audioFiles")]
    audio_files: Option<Vec<serde_json::Value>>,
}

#[derive(Deserialize, Default, Clone)]
struct Metadata {
    title: Option<String>,
    #[serde(rename = "authorName")]
    author_name: Option<String>,
    /// The expanded form, as search gives it, names the authors one by one.
    #[serde(default)]
    authors: Vec<Named>,
    #[serde(rename = "seriesName")]
    series_name: Option<String>,
    /// One series as an object on the continue-series shelf, a list of them in search.
    #[serde(default)]
    series: serde_json::Value,
}

impl Metadata {
    /// The series and the book's place in it, as "Wilde Tales, book 2".
    fn series(&self) -> Option<String> {
        let one = match &self.series {
            serde_json::Value::Array(all) => all.first(),
            other => Some(other),
        };
        let name = one.and_then(|s| s.get("name")).and_then(|n| n.as_str()).filter(|n| !n.is_empty());
        let place = one.and_then(|s| s.get("sequence")).and_then(|n| n.as_str()).filter(|n| !n.is_empty());
        match (name, place) {
            (Some(name), Some(place)) => Some(format!("{name}, book {place}")),
            (Some(name), None) => Some(name.to_string()),
            _ => self.series_name.as_deref().filter(|s| !s.is_empty()).map(|s| match s.rsplit_once(" #") {
                Some((name, place)) if !name.is_empty() && !place.is_empty() && !name.contains(',') => format!("{name}, book {place}"),
                _ => s.to_string(),
            }),
        }
    }

    fn author(&self) -> Option<String> {
        let joined = self.authors.iter().map(|a| a.name.as_str()).filter(|n| !n.is_empty()).collect::<Vec<_>>().join(", ");
        self.author_name.clone().filter(|a| !a.is_empty()).or_else(|| Some(joined).filter(|a| !a.is_empty()))
    }
}

#[derive(Deserialize, Default, Clone)]
struct Named {
    #[serde(default)]
    name: String,
}

#[derive(Deserialize, Default, Clone)]
struct Episode {
    #[serde(default)]
    id: String,
    title: Option<String>,
    #[serde(rename = "publishedAt")]
    published_at: Option<i64>,
    #[serde(rename = "audioFile")]
    audio_file: Option<AudioFile>,
}

impl Episode {
    fn length(&self) -> Option<f64> {
        self.audio_file.as_ref().and_then(|a| a.duration)
    }
}

#[derive(Deserialize, Default, Clone)]
struct AudioFile {
    duration: Option<f64>,
}

#[derive(Deserialize)]
struct PlaySession {
    id: String,
    #[serde(rename = "audioTracks", default)]
    audio_tracks: Vec<Track>,
    #[serde(rename = "currentTime")]
    current_time: Option<f64>,
    #[serde(rename = "displayTitle")]
    display_title: Option<String>,
}

#[derive(Deserialize)]
struct Track {
    #[serde(rename = "contentUrl", default)]
    content_url: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session() -> Session {
        Session { server: "http://abs:13378".into(), bearer: "tok".into(), auth: Auth::Key, may_sign_in: true }
    }

    /// A book, a podcast with an episode in progress and a finished book, as the server lists what is in progress.
    const IN_PROGRESS: &str = r#"{"libraryItems":[
        {"id":"b1","mediaType":"book","addedAt":1794392600000,"updatedAt":1794392700000,"progressLastUpdate":1794392948123,
         "media":{"coverPath":"/audiobooks/b1/cover.jpg","duration":6101.8,"metadata":{"title":"The Happy Prince","authorName":"Oscar Wilde","seriesName":null}}},
        {"id":"p1","mediaType":"podcast","updatedAt":5,"progressLastUpdate":1794393096398,
         "media":{"coverPath":"/podcasts/p1/cover.jpg","metadata":{"title":"The Daily","author":"The New York Times"}},
         "recentEpisode":{"id":"e1","title":"Election Day","publishedAt":1604400900000,"season":"","episode":"","audioFile":{"duration":39.2}}},
        {"id":"b2","mediaType":"book","media":{"duration":100,"metadata":{"title":"Done"}}}
    ]}"#;

    const PROGRESS: &str = r#"[
        {"libraryItemId":"b1","episodeId":null,"duration":6102,"progress":0.098,"currentTime":600,"isFinished":false,"lastUpdate":1794392948123},
        {"libraryItemId":"p1","episodeId":"e1","duration":39,"progress":0.3,"currentTime":12,"isFinished":false,"lastUpdate":1794393096398},
        {"libraryItemId":"b2","episodeId":null,"duration":100,"currentTime":100,"isFinished":true,"lastUpdate":1}
    ]"#;

    #[test]
    fn books_and_episodes_in_progress_join_continue_watching() {
        let s = session();
        let progress = Progresses::new(serde_json::from_str(PROGRESS).unwrap());
        let listed: InProgress = serde_json::from_str(IN_PROGRESS).unwrap();
        let items: Vec<Item> = each::<LibraryItem>(listed.library_items).iter().filter_map(|it| s.resume_item(it, &progress)).collect();
        assert_eq!(items.len(), 2, "a finished book is left out");
        let book = &items[0];
        assert_eq!((book.id.as_str(), book.kind), ("resume:book:b1", ItemKind::Resume));
        assert_eq!((book.title.as_str(), book.subtitle.as_deref()), ("The Happy Prince", Some("Oscar Wilde")));
        assert_eq!((book.position_ms, book.duration_ms, book.last_used_ms), (Some(600_000), Some(6_102_000), Some(1_794_392_948_123)));
        assert_eq!(book.poster.as_deref(), Some("http://abs:13378/api/items/b1/cover?width=800&format=jpeg&ts=1794392700000"));
        assert_eq!(book.action.resolve.as_deref(), Some("book:b1"));
        let episode = &items[1];
        assert_eq!(episode.id, "resume:episode:p1:e1");
        assert_eq!((episode.title.as_str(), episode.subtitle.as_deref()), ("Election Day", Some("The Daily")));
        assert_eq!((episode.position_ms, episode.duration_ms), (Some(12_000), Some(39_000)));
        assert_eq!(episode.action.resolve.as_deref(), Some("episode:p1:e1"));
    }

    #[test]
    fn a_book_without_its_place_is_up_next() {
        let s = session();
        // The continue-series shelf gives the series as an object and leaves its name empty.
        let it: LibraryItem = serde_json::from_str(
            r#"{"id":"b3","mediaType":"book","media":{"duration":3600,"metadata":{"title":"Book Two","authorName":"A. Writer","seriesName":"","series":{"id":"s","name":"Wilde Tales","sequence":"2"}}}}"#,
        )
        .unwrap();
        let item = s.next_item(&it).unwrap();
        assert_eq!((item.id.as_str(), item.kind, item.position_ms), ("next:book:b3", ItemKind::Resume, None));
        assert_eq!(item.subtitle.as_deref(), Some("Wilde Tales, book 2"));
        let listed: Metadata = serde_json::from_str(r#"{"title":"T","seriesName":"Wilde Tales #1"}"#).unwrap();
        assert_eq!(listed.series().as_deref(), Some("Wilde Tales, book 1"));
        let two: Metadata = serde_json::from_str(r#"{"title":"T","seriesName":"One #1, Two #3"}"#).unwrap();
        assert_eq!(two.series().as_deref(), Some("One #1, Two #3"), "several series are left as written");
        let found: Metadata = serde_json::from_str(r#"{"title":"T","series":[{"id":"s","name":"Wilde Tales","sequence":null}]}"#).unwrap();
        assert_eq!(found.series().as_deref(), Some("Wilde Tales"));
        let none: Metadata = serde_json::from_str(r#"{"title":"T","authorName":"A"}"#).unwrap();
        assert_eq!((none.series(), none.author().as_deref()), (None, Some("A")));
        assert_eq!(item.poster, None, "a book without a cover gets no picture address");
        // With no position and no progress the TV takes it as up next, not as something to continue.
        assert_eq!(item.progress, None);
    }

    #[test]
    fn new_books_and_episodes_get_rows_of_their_own() {
        let s = session();
        let now = 1_794_400_000_000;
        let listed: Vec<LibraryItem> = each(serde_json::from_str::<InProgress>(IN_PROGRESS).unwrap().library_items);
        let book = s.recent_item(&listed[0], now).unwrap();
        assert_eq!((book.id.as_str(), book.row.as_deref(), book.kind, book.is_new), ("new:book:b1", Some("recent"), ItemKind::Program, true));
        let episode = s.episode_item(&listed[1], now).unwrap();
        assert_eq!((episode.row.as_deref(), episode.subtitle.as_deref()), (Some("episodes"), Some("The Daily, 1 minute")));
        assert!(!episode.is_new, "an episode from years ago is not new");
        assert_eq!(s.episode_item(&listed[0], now).map(|i| i.id), None, "a book has no episode");
        // Search gives authors one by one rather than as one name.
        let found: Found = serde_json::from_str(r#"{"book":[{"libraryItem":{"id":"b4","media":{"metadata":{"title":"Two Hands","authors":[{"id":"a","name":"Ann"},{"id":"b","name":"Bo"}]}}}}],"authors":[]}"#).unwrap();
        assert_eq!(s.recent_item(&found.book[0].library_item, now).unwrap().subtitle.as_deref(), Some("Ann, Bo"));
    }

    #[test]
    fn only_books_with_audio_become_tiles() {
        let s = session();
        let item = |json: &str| serde_json::from_str::<LibraryItem>(json).unwrap();
        let ebook = item(r#"{"id":"e","mediaType":"book","media":{"numTracks":0,"numAudioFiles":0,"metadata":{"title":"Text only"}}}"#);
        let missing = item(r#"{"id":"m","mediaType":"book","isMissing":true,"media":{"numTracks":3,"metadata":{"title":"Gone from disk"}}}"#);
        let found_ebook = item(r#"{"id":"f","mediaType":"book","media":{"audioFiles":[],"metadata":{"title":"Found as text"}}}"#);
        let audio = item(r#"{"id":"a","mediaType":"book","media":{"numTracks":5,"metadata":{"title":"Heard"}}}"#);
        let unknown = item(r#"{"id":"u","mediaType":"book","media":{"metadata":{"title":"Untold"}}}"#);
        for it in [&ebook, &missing, &found_ebook] {
            assert!(s.recent_item(it, 0).is_none() && s.next_item(it).is_none(), "{}", it.id);
        }
        assert!(s.recent_item(&audio, 0).is_some() && s.recent_item(&unknown, 0).is_some());
        let progress = Progresses::new(vec![]);
        assert!(s.resume_item(&ebook, &progress).is_none());
        // A podcast listed without the episode to play has nothing to press.
        assert!(s.resume_item(&item(r#"{"id":"p","mediaType":"podcast","media":{"metadata":{"title":"Show"}}}"#), &progress).is_none());
    }

    #[test]
    fn hidden_and_read_only_books_stay_out_of_continue_watching() {
        let s = session();
        let book = serde_json::from_str::<LibraryItem>(r#"{"id":"b","mediaType":"book","media":{"numTracks":1,"metadata":{"title":"T"}}}"#).unwrap();
        let with = |p: &str| Progresses::new(each(serde_json::from_str::<Vec<serde_json::Value>>(p).unwrap()));
        assert!(s.resume_item(&book, &with(r#"[{"libraryItemId":"b","currentTime":60,"hideFromContinueListening":true}]"#)).is_none());
        assert!(s.resume_item(&book, &with(r#"[{"libraryItemId":"b","currentTime":0,"ebookProgress":0.4}]"#)).is_none());
        assert!(s.resume_item(&book, &with(r#"[{"libraryItemId":"b","currentTime":60,"ebookProgress":0.4}]"#)).is_some());
        // An entry the server writes for an item that is gone neither fails the rest nor counts.
        let odd = with(r#"[{"libraryItemId":null,"currentTime":5},{"libraryItemId":"b","currentTime":60},"not an entry"]"#);
        assert_eq!(odd.0.len(), 1);
        assert_eq!(s.resume_item(&book, &odd).and_then(|i| i.position_ms), Some(60_000));
        assert_eq!(without_query("/api/libraries/l/search?q=private"), "/api/libraries/l/search");
    }

    #[test]
    fn shelves_of_other_things_are_passed_over() {
        let shelves: Vec<Shelf> = serde_json::from_str(
            r#"[{"id":"newest-authors","type":"authors","entities":[{"id":"a1","name":"Oscar Wilde","numBooks":2,"addedAt":"not a number"}]},
                {"id":"recently-added","type":"book","entities":[{"id":"b1","mediaType":"book","media":{"metadata":{"title":"T"}}}]}]"#,
        )
        .unwrap();
        let read = |shelf: &Shelf| shelf.entities.iter().filter_map(|e| serde_json::from_value::<LibraryItem>(e.clone()).ok()).count();
        assert_eq!((read(&shelves[0]), read(&shelves[1])), (0, 1));
    }

    #[test]
    fn only_this_plugins_actions_start_a_stream() {
        assert_eq!(play_path("book:75080997-7463").as_deref(), Some("/api/items/75080997-7463/play"));
        assert_eq!(play_path("episode:p1:e1").as_deref(), Some("/api/items/p1/play/e1"));
        for odd in ["book:", "book:a:b", "episode:p1", "episode::e1", "film:x", ""] {
            assert_eq!(play_path(odd), None, "{odd:?}");
        }
        assert_eq!(play_path("book:a/../users").as_deref(), Some("/api/items/a%2F..%2Fusers/play"));
    }

    #[test]
    fn a_stream_starts_where_the_server_left_off() {
        let session: PlaySession = serde_json::from_str(
            r#"{"id":"s1","currentTime":1800.4,"displayTitle":"The Happy Prince","playMethod":2,
                "audioTracks":[{"index":1,"startOffset":0,"duration":6101.8,"contentUrl":"/hls/s1/output.m3u8","mimeType":"application/vnd.apple.mpegurl"}]}"#,
        )
        .unwrap();
        assert_eq!(session.audio_tracks[0].content_url, "/hls/s1/output.m3u8");
        assert_eq!(session.current_time.map(seconds_ms), Some(1_800_400));
        assert_eq!((seconds_ms(f64::NAN), seconds_ms(-3.0)), (0, 0));
    }

    #[test]
    fn a_sign_in_prefers_the_access_token_over_the_old_one() {
        let new: SignedIn = serde_json::from_str(r#"{"user":{"id":"u","token":"old","accessToken":"acc","refreshToken":"ref"}}"#).unwrap();
        assert_eq!((new.user.access_token.as_deref(), new.user.refresh_token.as_deref()), (Some("acc"), Some("ref")));
        let old: SignedIn = serde_json::from_str(r#"{"user":{"id":"u","token":"old"}}"#).unwrap();
        assert_eq!(old.user.access_token.or(old.user.token).as_deref(), Some("old"));
        let r = request("GET", "http://abs:13378/api/me".into(), Some("tok"), None);
        assert_eq!(r.headers.get("Authorization").map(String::as_str), Some("Bearer tok"));
        assert!(!r.headers.contains_key("Content-Type"));
    }

    #[test]
    fn the_manifest_asks_only_for_the_server() {
        let m = <Audiobookshelf as sdk::Plugin>::manifest();
        assert!(m.validate().is_ok(), "{:?}", m.validate());
        assert_eq!(m.settings.iter().filter(|s| s.required).map(|s| s.key.as_str()).collect::<Vec<_>>(), ["server"]);
        assert_eq!(m.rows.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(), ["recent", "episodes"]);
        assert_eq!(<Audiobookshelf as sdk::Plugin>::refresh(&RefreshContext::default()).err().as_deref(), Some("Add the server address in the plugin's settings."));
        assert_eq!(<Audiobookshelf as sdk::Plugin>::resolve("film:x").err().as_deref(), Some(GONE));
    }

    #[test]
    fn lengths_read_as_words() {
        assert_eq!(length_text(39_000), "1 minute");
        assert_eq!(length_text(3_600_000), "1 hour");
        assert_eq!(length_text(6_102_000), "1 hour 42 minutes");
        assert_eq!(enc("the bear & co"), "the%20bear%20%26%20co");
    }
}
