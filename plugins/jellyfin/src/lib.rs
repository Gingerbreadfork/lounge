//! Jellyfin plugin: signs in with Quick Connect or the user's credentials, publishes resume points and the next
//! episode of each show into Continue watching, the newest library additions and the user's
//! favourites into rows of their own, and searches the library. Items open on their page in the
//! Jellyfin Android TV app.

use lounge_plugin_sdk::{self as sdk, host, parse_iso8601, Action, HttpRequest, Item, ItemKind, Kind, Manifest, RefreshContext, RowStyle, RowSpec, SettingKind, SettingSpec};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub struct Jellyfin;

const APP_PACKAGE: &str = "org.jellyfin.androidtv";
const QUICK_KEY: &str = "quick";
const QUICK_OFF: &str = "Quick Connect is turned off on this Jellyfin server. Turn it on in the Dashboard under General, or enter a username and password in the plugin's settings.";
/// The client header, sent as `Authorization`, which every Jellyfin reads; newer servers ignore
/// `X-Emby-Authorization` and `api_key` unless legacy authorization is switched on. It names this
/// TV by an id of its own, since the server signs out an older session of the same user and id.
fn auth_header(device_id: &str) -> String {
    format!("MediaBrowser Client=\"Lounge\", Device=\"Android TV\", DeviceId=\"{device_id}\", Version=\"0.1\"")
}

/// This TV's device id, made the first time and kept; the clock is the only thing that differs
/// between two TVs a plugin can read.
fn device_id() -> String {
    host::kv_get_string("device_id").unwrap_or_else(|| {
        let id = format!("lounge-{:x}", host::now_ms());
        host::kv_set("device_id", id.as_bytes());
        id
    })
}

impl sdk::Plugin for Jellyfin {
    fn manifest() -> Manifest {
        Manifest {
            id: "tv.lounge.jellyfin".into(),
            name: "Jellyfin".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            description: "Continue watching, recently added and favourites from your Jellyfin server.".into(),
            publisher_name: "Lounge".into(),
            kinds: vec![Kind::Content, Kind::Search],
            fetch_hosts: vec!["$server".into()],
            refresh_minutes: 30,
            rows: vec![
                RowSpec { id: "recent".into(), title: "Recently added on Jellyfin".into(), style: RowStyle::Poster },
                RowSpec { id: "favourites".into(), title: "Favourites on Jellyfin".into(), style: RowStyle::Poster },
            ],
            settings: vec![
                SettingSpec { key: "server".into(), label: "Server address".into(), kind: SettingKind::Url, required: true, help: Some("Like http://192.168.0.10:8096".into()), default: None },
                SettingSpec {
                    key: "username".into(),
                    label: "Username".into(),
                    kind: SettingKind::Text,
                    required: false,
                    help: Some("Leave empty to sign in with Quick Connect: this page shows a code to enter in Jellyfin on a phone or computer that is already signed in.".into()),
                    default: None,
                },
                SettingSpec { key: "password".into(), label: "Password".into(), kind: SettingKind::Password, required: false, help: None, default: None },
            ],
            homepage: Some("https://jellyfin.org".into()),
            self_signed_servers: false,
            for_apps: vec![APP_PACKAGE.into()],
            sent_to: Default::default(),
        }
    }

    fn refresh(ctx: &RefreshContext) -> Result<Vec<Item>, String> {
        let Some(session) = Session::connect(true)? else { return Err("Jellyfin isn't signed in yet. Press Refresh now to sign in.".into()) };
        let mut items = Vec::new();
        let resume: ItemsResponse = session.get_json(&format!(
            "/Users/{}/Items/Resume?Limit=24&Recursive=true&MediaTypes=Video&Fields=Overview,ParentId,SeriesInfo,RunTimeTicks&EnableImageTypes=Primary,Backdrop,Thumb",
            session.user_id
        ))?;
        for it in resume.items {
            items.push(session.resume_item(&it));
        }
        let next_up: ItemsResponse = session.get_json(&format!(
            "/Shows/NextUp?UserId={}&Limit=24&EnableResumable=false&EnableImageTypes=Primary,Backdrop,Thumb",
            session.user_id
        ))?;
        let next_up = not_started(next_up.items);
        if !next_up.is_empty() {
            let played: ItemsResponse = session.get_json(&format!(
                "/Users/{}/Items?Recursive=true&IncludeItemTypes=Episode&IsPlayed=true&SortBy=DatePlayed&SortOrder=Descending&Limit=200&EnableImages=false",
                session.user_id
            ))?;
            let finished = Finished::from_plays(&played.items);
            for it in next_up {
                items.push(session.next_up_item(&it, &finished));
            }
        }
        let latest: Vec<MediaItem> = session.get_json(&format!(
            "/Users/{}/Items/Latest?Limit=20&IncludeItemTypes=Movie,Series&Fields=Overview,DateCreated&EnableImageTypes=Primary,Backdrop",
            session.user_id
        ))?;
        for it in latest {
            let mut item = session.latest_item(&it);
            item.is_new = sdk::new_this_week(item.added_ms, ctx.now_ms);
            items.push(item);
        }
        let favourites: ItemsResponse = session.get_json(&format!(
            "/Users/{}/Items?Recursive=true&IsFavorite=true&IncludeItemTypes=Movie,Series&SortBy=SortName&Limit=60&Fields=Overview,DateCreated&EnableImageTypes=Primary,Backdrop",
            session.user_id
        ))?;
        let count = favourites.items.len() as i32;
        for (i, it) in favourites.items.iter().enumerate() {
            items.push(session.favourite_item(it, count - i as i32));
        }
        sdk::dedup_ids(&mut items);
        sdk::info!("{} items from {}", items.len(), sdk::redact_url(&session.server));
        Ok(items)
    }

    /// A search never starts a Quick Connect sign-in; that is left to the refresh.
    fn search(query: &str, limit: u32) -> Result<Vec<Item>, String> {
        let Some(session) = Session::connect(false)? else { return Ok(Vec::new()) };
        let found: ItemsResponse = session.get_json(&format!(
            "/Users/{}/Items?searchTerm={}&Recursive=true&IncludeItemTypes=Movie,Series,Episode&Limit={}&Fields=Overview,SeriesInfo",
            session.user_id,
            url_encode(query),
            limit.clamp(1, 50)
        ))?;
        Ok(found.items.iter().map(|it| session.latest_item(it)).collect())
    }
}

sdk::export_plugin!(Jellyfin);

struct Session {
    server: String,
    token: String,
    user_id: String,
    device_id: String,
}

#[derive(Deserialize)]
struct AuthResponse {
    #[serde(rename = "AccessToken")]
    access_token: String,
    #[serde(rename = "User")]
    user: AuthUser,
}

#[derive(Deserialize)]
struct AuthUser {
    #[serde(rename = "Id")]
    id: String,
}

#[derive(Deserialize, Default)]
struct ItemsResponse {
    #[serde(rename = "Items", default)]
    items: Vec<MediaItem>,
}

#[derive(Deserialize, Default, Clone)]
pub struct MediaItem {
    #[serde(rename = "Id", default)]
    pub id: String,
    #[serde(rename = "Name", default)]
    pub name: String,
    #[serde(rename = "Type", default)]
    pub kind: String,
    #[serde(rename = "SeriesName")]
    pub series_name: Option<String>,
    #[serde(rename = "SeriesId")]
    pub series_id: Option<String>,
    #[serde(rename = "ParentIndexNumber")]
    pub season: Option<i64>,
    #[serde(rename = "IndexNumber")]
    pub episode: Option<i64>,
    #[serde(rename = "Overview")]
    pub overview: Option<String>,
    #[serde(rename = "RunTimeTicks")]
    pub runtime_ticks: Option<i64>,
    #[serde(rename = "UserData")]
    pub user_data: Option<UserData>,
    #[serde(rename = "BackdropImageTags", default)]
    pub backdrop_tags: Vec<String>,
    #[serde(rename = "ImageTags", default)]
    pub image_tags: HashMap<String, String>,
    #[serde(rename = "DateCreated")]
    pub date_created: Option<String>,
    #[serde(rename = "ProductionYear")]
    pub year: Option<i64>,
}

#[derive(Deserialize, Default, Clone)]
pub struct UserData {
    #[serde(rename = "PlaybackPositionTicks")]
    pub position_ticks: Option<i64>,
    #[serde(rename = "LastPlayedDate")]
    pub last_played: Option<String>,
    #[serde(rename = "Played", default)]
    pub played: bool,
}

/// A Quick Connect request waiting to be approved where the person is signed in.
#[derive(Serialize, Deserialize)]
struct Quick {
    server: String,
    secret: String,
    code: String,
}

/// What the server says of a Quick Connect request.
#[derive(Deserialize, Default)]
pub struct QuickAnswer {
    #[serde(rename = "Secret", default)]
    pub secret: String,
    #[serde(rename = "Code", default)]
    pub code: String,
    #[serde(rename = "Authenticated", default)]
    pub authenticated: bool,
}

fn quick_request(method: &str, url: String, device_id: &str, body: Option<String>) -> HttpRequest {
    let mut headers: std::collections::BTreeMap<String, String> = [("Authorization".to_string(), auth_header(device_id)), ("Accept".to_string(), "application/json".to_string())].into_iter().collect();
    if body.is_some() {
        headers.insert("Content-Type".to_string(), "application/json".to_string());
    }
    HttpRequest { method: method.into(), url, headers, body, timeout_ms: 0 }
}

/// The session once the code has been approved, so the refresh goes on with it, or else the
/// sentence the plugin's page shows meanwhile. A refresh asks about the code once.
fn quick_connect(server: &str, device_id: &str) -> Result<AuthResponse, String> {
    let unreadable = |e: serde_json::Error| {
        sdk::warn!("unexpected Quick Connect answer: {e}");
        sdk::wrong_server("Jellyfin", server)
    };
    let pending: Option<Quick> = host::kv_get(QUICK_KEY).and_then(|v| serde_json::from_slice(&v).ok()).filter(|q: &Quick| q.server == server);
    if let Some(quick) = pending {
        let ask = quick_request("GET", format!("{server}/QuickConnect/Connect?secret={}", url_encode(&quick.secret)), device_id, None);
        // A server that does not answer, or answers badly for a moment, leaves the code good,
        // so it stays on the screen.
        let Ok(r) = sdk::fetch_from("Jellyfin", server, &ask) else { return Err(instruction(&quick.code)) };
        match r.status {
            200 => {
                let answer: QuickAnswer = serde_json::from_str(&r.body).map_err(unreadable)?;
                if !answer.authenticated {
                    return Err(instruction(&quick.code));
                }
                let take = quick_request("POST", format!("{server}/Users/AuthenticateWithQuickConnect"), device_id, Some(serde_json::json!({ "Secret": quick.secret }).to_string()));
                let r = sdk::fetch_from("Jellyfin", server, &take)?;
                // An approved request is kept until it has been taken, or the server refuses it.
                if r.status >= 500 {
                    return Err(sdk::answered_error("Jellyfin", server, r.status));
                }
                host::kv_delete(QUICK_KEY);
                if !r.ok() {
                    return Err(sdk::answered_error("Jellyfin", server, r.status));
                }
                return serde_json::from_str(&r.body).map_err(unreadable);
            }
            401 => return Err(QUICK_OFF.into()),
            // The code has run out; a new one follows.
            404 => host::kv_delete(QUICK_KEY),
            s if s >= 500 => {
                sdk::warn!("HTTP {s} when asked about the Quick Connect code");
                return Err(instruction(&quick.code));
            }
            s => return Err(sdk::answered_error("Jellyfin", server, s)),
        }
    }
    // Servers before 10.9 start a request with GET.
    let start = |method: &str| sdk::fetch_from("Jellyfin", server, &quick_request(method, format!("{server}/QuickConnect/Initiate"), device_id, None));
    let mut r = start("POST")?;
    if r.status == 405 {
        r = start("GET")?;
    }
    match r.status {
        401 => return Err(QUICK_OFF.into()),
        404 => return Err(sdk::wrong_server("Jellyfin", server)),
        _ if !r.ok() => return Err(sdk::answered_error("Jellyfin", server, r.status)),
        _ => {}
    }
    let answer: QuickAnswer = serde_json::from_str(&r.body).map_err(unreadable)?;
    if answer.secret.is_empty() || answer.code.is_empty() {
        return Err(sdk::wrong_server("Jellyfin", server));
    }
    let quick = Quick { server: server.to_string(), secret: answer.secret, code: answer.code };
    host::kv_set(QUICK_KEY, serde_json::to_string(&quick).unwrap_or_default().as_bytes());
    Err(instruction(&quick.code))
}

/// The code is entered in Jellyfin's own settings, which the server's address does not lead
/// to, so the ask has no link. Jellyfin's TV app has no place to enter one.
fn instruction(code: &str) -> String {
    sdk::ask(sdk::Ask {
        say: format!("Sign in: on a phone or computer signed in to Jellyfin, open Settings, then Quick Connect, and enter {code}, then press Refresh now here."),
        link: None,
        code: Some(code.to_string()),
        after: Some("On a phone or computer signed in to Jellyfin, open Settings, then Quick Connect, and enter this code.".into()),
        watch: true,
    })
}

impl Session {
    /// Reuse the cached token; sign in again when there is none or the server rejects it. With
    /// no username that is by Quick Connect, which only `may_ask` starts; without it there is
    /// no session yet.
    fn connect(may_ask: bool) -> Result<Option<Session>, String> {
        let server = host::server_setting("server", "server address")?;
        let username = host::setting("username").map(|u| u.trim().to_string()).unwrap_or_default();
        let device_id = device_id();
        // The saved session belongs to one server and user; another of either signs in afresh.
        let owner = format!("{server}\n{username}");
        if host::kv_get_string("session_for").as_deref() == Some(owner.as_str())
            && let (Some(token), Some(user_id)) = (host::kv_get_string("token"), host::kv_get_string("user_id"))
        {
            let s = Session { server: server.clone(), token, user_id, device_id: device_id.clone() };
            let probe = sdk::fetch_from("Jellyfin", &server, &s.request(&format!("/Users/{}", s.user_id)))?;
            match probe.status {
                _ if probe.ok() => return Ok(Some(s)),
                // The server no longer takes the token, or no longer has the user. Anything else
                // is the server having a bad moment, which is no reason to sign in again.
                401 | 403 | 404 => {
                    host::kv_delete("token");
                    host::kv_delete("user_id");
                }
                status => return Err(sdk::answered_error("Jellyfin", &server, status)),
            }
        }
        if username.is_empty() {
            if !may_ask {
                return Ok(None);
            }
            let auth = quick_connect(&server, &device_id)?;
            return Ok(Some(Session::keep(server, auth, &owner, device_id)));
        }
        let password = host::secret("password").unwrap_or_default();
        let body = serde_json::json!({ "Username": username, "Pw": password }).to_string();
        let req = HttpRequest {
            method: "POST".into(),
            url: format!("{server}/Users/AuthenticateByName"),
            headers: [
                ("Content-Type".to_string(), "application/json".to_string()),
                ("Authorization".to_string(), auth_header(&device_id)),
            ]
            .into_iter()
            .collect(),
            body: Some(body),
            timeout_ms: 0,
        };
        let response = sdk::fetch_from("Jellyfin", &server, &req)?;
        match response.status {
            401 => return Err("Jellyfin rejected the username or password. Check them in the plugin's settings.".into()),
            404 => return Err(sdk::wrong_server("Jellyfin", &server)),
            _ if !response.ok() => return Err(sdk::answered_error("Jellyfin", &server, response.status)),
            _ => {}
        }
        let auth: AuthResponse = serde_json::from_str(&response.body).map_err(|e| {
            sdk::warn!("unexpected sign-in response: {e}");
            sdk::wrong_server("Jellyfin", &server)
        })?;
        Ok(Some(Session::keep(server, auth, &owner, device_id)))
    }

    /// Remembers a sign-in for the server and user it was made for.
    fn keep(server: String, auth: AuthResponse, owner: &str, device_id: String) -> Session {
        host::kv_set("token", auth.access_token.as_bytes());
        host::kv_set("user_id", auth.user.id.as_bytes());
        host::kv_set("session_for", owner.as_bytes());
        Session { server, token: auth.access_token, user_id: auth.user.id, device_id }
    }

    fn request(&self, path: &str) -> HttpRequest {
        HttpRequest::get(format!("{}{}", self.server, path))
            .header("Authorization", &format!("{}, Token=\"{}\"", auth_header(&self.device_id), self.token))
            .header("Accept", "application/json")
    }

    fn get_json<T: serde::de::DeserializeOwned>(&self, path: &str) -> Result<T, String> {
        let r = sdk::fetch_from("Jellyfin", &self.server, &self.request(path))?;
        if !r.ok() {
            sdk::warn!("HTTP {} for {path}", r.status);
            return Err(sdk::answered_error("Jellyfin", &self.server, r.status));
        }
        serde_json::from_str(&r.body).map_err(|e| {
            sdk::warn!("unexpected response for {path}: {e}");
            sdk::unreadable("Jellyfin", &self.server)
        })
    }

    /// Item pictures need no sign-in, so the token stays out of every picture's address. They are
    /// asked for as JPEG: a poster kept as PNG is many times the bytes and far slower to decode.
    fn image(&self, item_id: &str, kind: &str, size: &str) -> String {
        format!("{}/Items/{item_id}/Images/{kind}?{size}&quality=90&format=jpg", self.server)
    }

    fn resume_item(&self, it: &MediaItem) -> Item {
        let is_episode = it.kind == "Episode";
        let title = if is_episode { it.series_name.clone().unwrap_or_else(|| it.name.clone()) } else { it.name.clone() };
        let subtitle = if is_episode { Some(it.name.clone()) } else { it.overview.as_ref().map(|o| first_sentence(o)) };
        let duration_ms = it.runtime_ticks.map(|t| t / 10_000);
        let position_ms = it.user_data.as_ref().and_then(|u| u.position_ticks).map(|t| t / 10_000).filter(|&p| p > 0);
        let progress = match (position_ms, duration_ms) {
            (Some(p), Some(d)) if d > 0 => Some((p as f32 / d as f32).clamp(0.0, 1.0)),
            _ => None,
        };
        let (poster, thumbnail) = if is_episode {
            (
                it.series_id.as_ref().map(|s| self.image(s, "Primary", "maxHeight=600")),
                Some(self.image(&it.id, "Primary", "maxWidth=960")),
            )
        } else {
            (
                it.image_tags.contains_key("Primary").then(|| self.image(&it.id, "Primary", "maxHeight=600")),
                (!it.backdrop_tags.is_empty()).then(|| self.image(&it.id, "Backdrop/0", "maxWidth=1280")),
            )
        };
        Item {
            id: format!("resume:{}", it.id),
            kind: ItemKind::Resume,
            title,
            subtitle,
            season: it.season.map(|n| n.to_string()),
            episode: it.episode.map(|n| n.to_string()),
            duration_ms,
            position_ms,
            progress,
            last_used_ms: it.user_data.as_ref().and_then(|u| u.last_played.as_deref()).and_then(parse_iso8601),
            poster,
            thumbnail,
            action: open(it),
            ..Default::default()
        }
    }

    /// The next episode of a show, dated by the last time an episode of its series was finished.
    fn next_up_item(&self, it: &MediaItem, finished: &Finished) -> Item {
        let mut item = self.resume_item(it);
        item.id = format!("next:{}", it.id);
        item.last_used_ms = finished.at(it.series_id.as_deref());
        item
    }

    fn latest_item(&self, it: &MediaItem) -> Item {
        let is_episode = it.kind == "Episode";
        Item {
            id: format!("latest:{}", it.id),
            kind: ItemKind::Program,
            title: if is_episode { it.series_name.clone().unwrap_or_else(|| it.name.clone()) } else { it.name.clone() },
            subtitle: if is_episode { Some(it.name.clone()) } else { it.year.map(|y| y.to_string()) },
            season: it.season.map(|n| n.to_string()),
            episode: it.episode.map(|n| n.to_string()),
            added_ms: it.date_created.as_deref().and_then(parse_iso8601),
            poster: Some(self.image(if is_episode { it.series_id.as_deref().unwrap_or(&it.id) } else { &it.id }, "Primary", "maxHeight=600")),
            thumbnail: (!it.backdrop_tags.is_empty()).then(|| self.image(&it.id, "Backdrop/0", "maxWidth=1280")),
            row: Some("recent".into()),
            action: open(it),
            ..Default::default()
        }
    }

    /// A favourite movie or show; `weight` keeps the row in the server's alphabetical order.
    fn favourite_item(&self, it: &MediaItem, weight: i32) -> Item {
        let mut item = self.latest_item(it);
        item.id = format!("fav:{}", it.id);
        item.row = Some("favourites".into());
        item.weight = weight;
        item
    }
}

/// Opens the item's page in the Jellyfin app, whose start screen takes an item id the way the
/// app's own Watch Next entries pass it.
fn open(item: &MediaItem) -> Action {
    Action {
        view: Some(format!(
            "intent:#Intent;component={APP_PACKAGE}/org.jellyfin.androidtv.ui.startup.StartupActivity;S.ItemId={};end",
            url_encode(&item.id)
        )),
        fallback_launch: Some(APP_PACKAGE.into()),
        title: Some(item.name.clone()),
        ..Default::default()
    }
}

/// The next episodes not yet started. Older servers ignore `EnableResumable=false`, and a started
/// episode already has a resume tile.
fn not_started(next_up: Vec<MediaItem>) -> Vec<MediaItem> {
    next_up.into_iter().filter(|it| !it.user_data.as_ref().and_then(|u| u.position_ticks).is_some_and(|t| t > 0)).collect()
}

/// When an episode of each series was last finished, from the most recent plays.
struct Finished {
    by_series: HashMap<String, i64>,
    /// The oldest of those plays: a series missing from them was last watched before it.
    oldest: Option<i64>,
}

impl Finished {
    fn from_plays(played: &[MediaItem]) -> Finished {
        let mut by_series = HashMap::new();
        let mut oldest: Option<i64> = None;
        for it in played {
            let Some(at) = it.user_data.as_ref().and_then(|u| u.last_played.as_deref()).and_then(parse_iso8601) else {
                continue;
            };
            if let Some(series) = &it.series_id {
                by_series.entry(series.clone()).and_modify(|t: &mut i64| *t = (*t).max(at)).or_insert(at);
            }
            oldest = Some(oldest.map_or(at, |o| o.min(at)));
        }
        Finished { by_series, oldest }
    }

    fn at(&self, series_id: Option<&str>) -> Option<i64> {
        series_id.and_then(|s| self.by_series.get(s).copied()).or(self.oldest)
    }
}

fn first_sentence(s: &str) -> String {
    let end = s.find(". ").map(|i| i + 1).unwrap_or(s.len()).min(160);
    let cut = s.char_indices().take_while(|(i, _)| *i < end).map(|(i, c)| i + c.len_utf8()).last().unwrap_or(0);
    s[..cut].trim().to_string()
}

fn url_encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iso_dates() {
        assert_eq!(parse_iso8601("2026-10-19T00:15:00.0000000Z"), Some(1_792_368_900_000));
        assert_eq!(parse_iso8601("2026-10-19T10:15:00+10:00"), Some(1_792_368_900_000));
        assert_eq!(parse_iso8601("nope"), None);
        for odd in ["2026-1é-19T00:15:00Z", "2026-10-19T00:15:0ü", "2026-10-19T00:15:00+1€:00", "ＸＸＸＸＸＸＸＸＸＸＸ"] {
            assert_eq!(parse_iso8601(odd), None, "{odd:?}");
        }
        let it: MediaItem = serde_json::from_str(r#"{"Id": "m", "Name": "Film", "Type": "Movie", "UserData": {"LastPlayedDate": "2026-10-1é"}, "DateCreated": "２０２６"}"#).unwrap();
        assert_eq!(it.user_data.as_ref().and_then(|u| u.last_played.as_deref()).and_then(parse_iso8601), None);
        assert_eq!(it.date_created.as_deref().and_then(parse_iso8601), None);
    }

    #[test]
    fn media_items_map_to_resume_tiles() {
        let s = Session { server: "http://jf:8096".into(), token: "t".into(), user_id: "u".into(), device_id: "tv-1".into() };
        let it: MediaItem = serde_json::from_str(
            r#"{"Id":"e1","Name":"Honeydew","Type":"Episode","SeriesName":"The Bear","SeriesId":"s1","ParentIndexNumber":2,"IndexNumber":4,
                 "RunTimeTicks":18000000000,"UserData":{"PlaybackPositionTicks":9000000000,"LastPlayedDate":"2026-10-18T12:00:00Z"}}"#,
        )
        .unwrap();
        let item = s.resume_item(&it);
        assert_eq!(item.title, "The Bear");
        assert_eq!(item.subtitle.as_deref(), Some("Honeydew"));
        assert_eq!(item.season.as_deref(), Some("2"));
        assert_eq!(item.progress, Some(0.5));
        assert_eq!(item.duration_ms, Some(1_800_000));
        assert_eq!(item.thumbnail.as_deref(), Some("http://jf:8096/Items/e1/Images/Primary?maxWidth=960&quality=90&format=jpg"));
        assert!(item.poster.unwrap().contains("/Items/s1/Images/Primary"));
        assert_eq!(
            item.action.view.as_deref(),
            Some("intent:#Intent;component=org.jellyfin.androidtv/org.jellyfin.androidtv.ui.startup.StartupActivity;S.ItemId=e1;end")
        );
        assert_eq!(item.action.fallback_launch.as_deref(), Some(APP_PACKAGE));
        assert_eq!(url_encode("the bear & co"), "the+bear+%26+co");
    }

    #[test]
    fn an_item_id_cannot_add_to_the_intent() {
        let it = MediaItem { id: "e1;component=x/.Y;end".into(), ..Default::default() };
        let view = open(&it).view.unwrap();
        assert!(view.ends_with(";S.ItemId=e1%3Bcomponent%3Dx%2F.Y%3Bend;end"), "{view}");
    }

    #[test]
    fn next_episodes_take_the_time_their_series_was_last_finished() {
        let s = Session { server: "http://jf:8096".into(), token: "t".into(), user_id: "u".into(), device_id: "tv-1".into() };
        let played: Vec<MediaItem> = serde_json::from_str(
            r#"[{"Id":"a2","SeriesId":"sa","UserData":{"Played":true,"LastPlayedDate":"2026-10-20T00:00:00Z"}},
                {"Id":"b1","SeriesId":"sb","UserData":{"Played":true,"LastPlayedDate":"2026-10-18T00:00:00Z"}},
                {"Id":"a1","SeriesId":"sa","UserData":{"Played":true,"LastPlayedDate":"2026-10-10T00:00:00Z"}},
                {"Id":"m1","UserData":{"Played":true}}]"#,
        )
        .unwrap();
        let finished = Finished::from_plays(&played);
        let next: MediaItem = serde_json::from_str(
            r#"{"Id":"a3","Name":"Three","Type":"Episode","SeriesName":"Show A","SeriesId":"sa","ParentIndexNumber":1,"IndexNumber":3,"RunTimeTicks":18000000000,
                 "UserData":{"PlaybackPositionTicks":0,"Played":false}}"#,
        )
        .unwrap();
        let item = s.next_up_item(&next, &finished);
        assert_eq!(item.id, "next:a3");
        assert_eq!(item.kind, ItemKind::Resume);
        assert_eq!(item.title, "Show A");
        assert_eq!(item.progress, None);
        assert_eq!(item.position_ms, None);
        assert_eq!(item.last_used_ms, parse_iso8601("2026-10-20T00:00:00Z"));
        assert_eq!(finished.at(Some("sb")), parse_iso8601("2026-10-18T00:00:00Z"));
        assert_eq!(finished.at(Some("older")), parse_iso8601("2026-10-10T00:00:00Z"));
        assert_eq!(Finished::from_plays(&[]).at(Some("sa")), None);
    }

    #[test]
    fn a_started_episode_up_next_is_left_to_its_resume_tile() {
        let next_up: ItemsResponse = serde_json::from_str(
            r#"{"Items":[{"Id":"a3","Name":"Three","Type":"Episode","SeriesId":"sa","UserData":{"PlaybackPositionTicks":9000000000,"Played":false}},
                {"Id":"b2","Name":"Two","Type":"Episode","SeriesId":"sb","UserData":{"PlaybackPositionTicks":0,"Played":false}},
                {"Id":"c1","Name":"One","Type":"Episode","SeriesId":"sc"}]}"#,
        )
        .unwrap();
        let ids: Vec<String> = not_started(next_up.items).into_iter().map(|it| it.id).collect();
        assert_eq!(ids, ["b2", "c1"]);
    }

    #[test]
    fn favourites_go_in_their_own_row_without_the_new_badge() {
        let s = Session { server: "http://jf:8096".into(), token: "t".into(), user_id: "u".into(), device_id: "tv-1".into() };
        let it: MediaItem = serde_json::from_str(
            r#"{"Id":"m1","Name":"Static Bloom","Type":"Movie","ProductionYear":2021,"DateCreated":"2024-03-01T00:00:00Z","ImageTags":{"Primary":"x"}}"#,
        )
        .unwrap();
        let item = s.favourite_item(&it, 7);
        assert_eq!(item.id, "fav:m1");
        assert_eq!(item.row.as_deref(), Some("favourites"));
        assert!(!item.is_new);
        assert_eq!(item.weight, 7);
        assert_eq!(item.added_ms, parse_iso8601("2024-03-01T00:00:00Z"));
        assert_eq!(item.subtitle.as_deref(), Some("2021"));
        assert!(<Jellyfin as sdk::Plugin>::manifest().rows.iter().any(|r| r.id == "favourites"));
    }

    #[test]
    fn quick_connect_shows_a_code_until_it_is_approved() {
        let waiting: QuickAnswer = serde_json::from_str(
            r#"{"Authenticated":false,"Secret":"0123abcd","Code":"482913","DeviceId":"lounge-1","DeviceName":"Android TV","AppName":"Lounge","AppVersion":"0.1","DateAdded":"2026-11-09T21:03:22Z"}"#,
        )
        .unwrap();
        assert_eq!((waiting.authenticated, waiting.secret.as_str(), waiting.code.as_str()), (false, "0123abcd", "482913"));
        assert!(serde_json::from_str::<QuickAnswer>(r#"{"Authenticated":true,"Secret":"0123abcd","Code":"482913"}"#).unwrap().authenticated);
        assert_eq!(
            instruction("482913"),
            "Sign in: on a phone or computer signed in to Jellyfin, open Settings, then Quick Connect, and enter 482913, then press Refresh now here."
        );
        let request = quick_request("POST", "http://jf:8096/Users/AuthenticateWithQuickConnect".into(), "tv-1", Some(r#"{"Secret":"0123abcd"}"#.into()));
        assert_eq!(request.headers.get("Authorization").map(String::as_str), Some(auth_header("tv-1").as_str()));
        assert_eq!(request.headers.get("Content-Type").map(String::as_str), Some("application/json"));
        let auth: AuthResponse = serde_json::from_str(r#"{"User":{"Name":"me","Id":"u1"},"AccessToken":"tok","ServerId":"s1"}"#).unwrap();
        assert_eq!((auth.access_token.as_str(), auth.user.id.as_str()), ("tok", "u1"));
        // Only the server address is needed to turn the plugin on.
        let m = <Jellyfin as sdk::Plugin>::manifest();
        assert!(m.validate().is_ok());
        assert_eq!(m.settings.iter().filter(|s| s.required).map(|s| s.key.as_str()).collect::<Vec<_>>(), ["server"]);
    }

    #[test]
    fn the_token_goes_in_the_authorization_header() {
        let s = Session { server: "http://jf:8096".into(), token: "secret".into(), user_id: "u".into(), device_id: "tv-1".into() };
        let req = s.request("/Users/u");
        assert_eq!(req.headers.get("Authorization").map(String::as_str), Some(format!("{}, Token=\"secret\"", auth_header("tv-1")).as_str()));
        assert!(!req.headers.contains_key("X-Emby-Authorization"));
    }

    #[test]
    fn errors_name_the_server_and_what_to_check() {
        let s = Session { server: "http://192.168.0.20:8096".into(), token: "t".into(), user_id: "u".into(), device_id: "tv-1".into() };
        // Off the TV there is no host, so every fetch fails as an unreachable server would.
        assert_eq!(
            s.get_json::<serde_json::Value>("/Users/u").err().as_deref(),
            Some("Jellyfin didn't answer at 192.168.0.20:8096. Check the server, then try again.")
        );
        assert_eq!(<Jellyfin as sdk::Plugin>::refresh(&RefreshContext::default()).err().as_deref(), Some("Add the server address in the plugin's settings."));
    }
}
