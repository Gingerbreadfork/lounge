//! Emby plugin: signs in with the user's credentials, publishes resume points into Continue
//! watching and the newest library additions into a "Recently added" row, and searches the
//! library. Items open in the Emby for Android TV app. Emby and Jellyfin share one API, so this
//! is the Jellyfin plugin with Emby's names, app and paths.

use lounge_plugin_sdk::{self as sdk, host, parse_iso8601, Action, HttpRequest, Item, ItemKind, Kind, Manifest, RefreshContext, RowStyle, RowSpec, SettingKind, SettingSpec};
use serde::Deserialize;

pub struct Emby;

const APP_PACKAGE: &str = "tv.emby.embyatv";
/// The client header. It names this TV by an id of its own, since the server signs out an older
/// session of the same user and id.
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

impl sdk::Plugin for Emby {
    fn manifest() -> Manifest {
        Manifest {
            id: "tv.lounge.emby".into(),
            name: "Emby".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            description: "Continue watching and recently added from your Emby server.".into(),
            publisher_name: "Lounge".into(),
            kinds: vec![Kind::Content, Kind::Search],
            fetch_hosts: vec!["$server".into(), "*".into()],
            refresh_minutes: 30,
            rows: vec![RowSpec { id: "recent".into(), title: "Recently added on Emby".into(), style: RowStyle::Poster }],
            settings: vec![
                SettingSpec { key: "server".into(), label: "Server address".into(), kind: SettingKind::Url, required: true, help: Some("Like http://192.168.0.10:8096".into()), default: None },
                SettingSpec { key: "username".into(), label: "Username".into(), kind: SettingKind::Text, required: true, help: None, default: None },
                SettingSpec { key: "password".into(), label: "Password".into(), kind: SettingKind::Password, required: false, help: None, default: None },
            ],
            homepage: Some("https://emby.media".into()),
        }
    }

    fn refresh(_ctx: &RefreshContext) -> Result<Vec<Item>, String> {
        let session = Session::connect()?;
        let mut items = Vec::new();
        let resume: ItemsResponse = session.get_json(&format!(
            "/Users/{}/Items/Resume?Limit=24&Recursive=true&MediaTypes=Video&Fields=Overview,ParentId,SeriesInfo,RunTimeTicks&EnableImageTypes=Primary,Backdrop,Thumb",
            session.user_id
        ))?;
        for it in resume.items {
            items.push(session.resume_item(&it));
        }
        let latest: Vec<MediaItem> = session.get_json(&format!(
            "/Users/{}/Items/Latest?Limit=20&IncludeItemTypes=Movie,Series&Fields=Overview,DateCreated&EnableImageTypes=Primary,Backdrop",
            session.user_id
        ))?;
        for it in latest {
            items.push(session.latest_item(&it));
        }
        sdk::dedup_ids(&mut items);
        sdk::info!("{} items from {}", items.len(), sdk::redact_url(&session.server));
        Ok(items)
    }

    fn search(query: &str, limit: u32) -> Result<Vec<Item>, String> {
        let session = Session::connect()?;
        let found: ItemsResponse = session.get_json(&format!(
            "/Users/{}/Items?searchTerm={}&Recursive=true&IncludeItemTypes=Movie,Series,Episode&Limit={}&Fields=Overview,SeriesInfo",
            session.user_id,
            url_encode(query),
            limit.clamp(1, 50)
        ))?;
        Ok(found.items.iter().map(|it| session.latest_item(it)).collect())
    }
}

sdk::export_plugin!(Emby);

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
    pub image_tags: std::collections::HashMap<String, String>,
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

impl Session {
    /// Reuse the cached token; sign in again when there is none or the server rejects it.
    fn connect() -> Result<Session, String> {
        let server = host::required_setting("server", "Server address")?.trim_end_matches('/').to_string();
        let username = host::required_setting("username", "Username")?;
        let device_id = device_id();
        // The saved session belongs to one server and user; another of either signs in afresh.
        let owner = format!("{server}\n{username}");
        if host::kv_get_string("session_for").as_deref() == Some(owner.as_str())
            && let (Some(token), Some(user_id)) = (host::kv_get_string("token"), host::kv_get_string("user_id"))
        {
            let s = Session { server: server.clone(), token, user_id, device_id: device_id.clone() };
            let probe = host::fetch(&s.request(&format!("/Users/{}", s.user_id)))?;
            if probe.ok() {
                return Ok(s);
            }
            host::kv_delete("token");
            host::kv_delete("user_id");
        }
        let password = host::secret("password").unwrap_or_default();
        let body = serde_json::json!({ "Username": username, "Pw": password }).to_string();
        let req = HttpRequest {
            method: "POST".into(),
            url: format!("{server}/emby/Users/AuthenticateByName"),
            headers: [
                ("Content-Type".to_string(), "application/json".to_string()),
                ("X-Emby-Authorization".to_string(), auth_header(&device_id)),
            ]
            .into_iter()
            .collect(),
            body: Some(body),
            timeout_ms: 0,
        };
        let response = host::fetch(&req)?;
        if response.status == 401 {
            return Err("Emby rejected the username or password".into());
        }
        if !response.ok() {
            return Err(format!("Emby answered HTTP {}", response.status));
        }
        let auth: AuthResponse = serde_json::from_str(&response.body).map_err(|e| format!("unexpected sign-in response: {e}"))?;
        host::kv_set("token", auth.access_token.as_bytes());
        host::kv_set("user_id", auth.user.id.as_bytes());
        host::kv_set("session_for", owner.as_bytes());
        Ok(Session { server, token: auth.access_token, user_id: auth.user.id, device_id })
    }

    fn request(&self, path: &str) -> HttpRequest {
        HttpRequest::get(format!("{}/emby{}", self.server, path))
            .header("X-Emby-Authorization", &format!("{}, Token=\"{}\"", auth_header(&self.device_id), self.token))
            .header("Accept", "application/json")
    }

    fn get_json<T: serde::de::DeserializeOwned>(&self, path: &str) -> Result<T, String> {
        let r = host::fetch(&self.request(path))?;
        if !r.ok() {
            return Err(format!("HTTP {} for {path}", r.status));
        }
        serde_json::from_str(&r.body).map_err(|e| format!("unexpected response for {path}: {e}"))
    }

    fn image(&self, item_id: &str, kind: &str, size: &str) -> String {
        format!("{}/emby/Items/{item_id}/Images/{kind}?{size}&quality=90&api_key={}", self.server, self.token)
    }

    fn open(&self, item: &MediaItem) -> Action {
        Action {
            view: Some(format!("{}/web/index.html#!/item?id={}", self.server, item.id)),
            fallback_launch: Some(APP_PACKAGE.into()),
            title: Some(item.name.clone()),
            ..Default::default()
        }
    }

    fn resume_item(&self, it: &MediaItem) -> Item {
        let is_episode = it.kind == "Episode";
        let title = if is_episode { it.series_name.clone().unwrap_or_else(|| it.name.clone()) } else { it.name.clone() };
        let subtitle = if is_episode { Some(it.name.clone()) } else { it.overview.as_ref().map(|o| first_sentence(o)) };
        let duration_ms = it.runtime_ticks.map(|t| t / 10_000);
        let position_ms = it.user_data.as_ref().and_then(|u| u.position_ticks).map(|t| t / 10_000);
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
            action: self.open(it),
            ..Default::default()
        }
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
            action: self.open(it),
            is_new: true,
            ..Default::default()
        }
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
        assert!(item.thumbnail.unwrap().contains("/Items/e1/Images/Primary"));
        assert!(item.poster.unwrap().contains("/Items/s1/Images/Primary"));
        assert_eq!(item.action.fallback_launch.as_deref(), Some(APP_PACKAGE));
        assert_eq!(url_encode("the bear & co"), "the+bear+%26+co");
    }
}
