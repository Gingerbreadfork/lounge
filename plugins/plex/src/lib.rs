//! Plex plugin: On Deck becomes Continue watching, the newest arrivals a "Recently added on
//! Plex" row, and the library answers search. Items open in the Plex app. It talks to the
//! server directly with a Plex token, so nothing goes through plex.tv.
use lounge_plugin_sdk::{self as sdk, host, Action, HttpRequest, Item, ItemKind, Kind, Manifest, RefreshContext, RowSpec, RowStyle, SettingKind, SettingSpec};
use serde::Deserialize;

pub struct Plex;

const APP_PACKAGE: &str = "com.plexapp.android";

impl sdk::Plugin for Plex {
    fn manifest() -> Manifest {
        Manifest {
            id: "tv.lounge.plex".into(),
            name: "Plex".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            description: "On Deck and recently added from your Plex Media Server.".into(),
            publisher_name: "Lounge".into(),
            kinds: vec![Kind::Content, Kind::Search],
            fetch_hosts: vec!["$server".into()],
            refresh_minutes: 30,
            rows: vec![RowSpec { id: "recent".into(), title: "Recently added on Plex".into(), style: RowStyle::Poster }],
            settings: vec![
                SettingSpec { key: "server".into(), label: "Server address".into(), kind: SettingKind::Url, required: true, help: Some("Like http://192.168.0.10:32400".into()), default: None },
                SettingSpec {
                    key: "token".into(),
                    label: "Plex token".into(),
                    kind: SettingKind::Password,
                    required: true,
                    help: Some("In Plex Web open any item, Get Info, View XML: the address ends in X-Plex-Token=…, copy that value.".into()),
                    default: None,
                },
                SettingSpec { key: "libraries".into(), label: "Libraries for recently added".into(), kind: SettingKind::Text, required: false, help: Some("Library names separated by commas. Empty means all of them.".into()), default: None },
            ],
            homepage: Some("https://www.plex.tv".into()),
        }
    }

    fn refresh(ctx: &RefreshContext) -> Result<Vec<Item>, String> {
        let server = Server::connect()?;
        let mut items = Vec::new();
        let on_deck: Container = server.get_json("/library/onDeck")?;
        let mut deck: Vec<Item> = on_deck.media_container.metadata.iter().map(|m| server.resume_item(m)).collect();
        date_in_order(&mut deck);
        items.extend(deck);
        // Asked of each movie and TV library, so music and photos never take the row's places.
        let wanted = libraries_setting();
        let sections: Container = server.get_json("/library/sections")?;
        let mut chosen = video_sections(&sections.media_container.directory, &wanted);
        if chosen.is_empty() && !wanted.is_empty() {
            sdk::warn!("no movie or TV library is called {}, so every one is used; check the names in the plugin's settings", wanted.join(" or "));
            chosen = video_sections(&sections.media_container.directory, &[]);
        }
        let mut recent: Vec<Metadata> = Vec::new();
        for section in chosen {
            match server.get_json::<Container>(&format!("/library/sections/{}/recentlyAdded?X-Plex-Container-Start=0&X-Plex-Container-Size={RECENT}", url_encode(&section.key))) {
                Ok(c) => recent.extend(c.media_container.metadata),
                Err(e) => sdk::warn!("{}: {e}", section.title),
            }
        }
        recent.sort_by_key(|m| std::cmp::Reverse(m.added_at.unwrap_or(0)));
        recent.truncate(RECENT);
        for m in &recent {
            if let Some(mut item) = server.recent_item(m) {
                item.is_new = sdk::new_this_week(item.added_ms, ctx.now_ms) && m.view_count.unwrap_or(0) == 0;
                items.push(item);
            }
        }
        sdk::dedup_ids(&mut items);
        sdk::info!("{} items from {}", items.len(), server.name);
        Ok(items)
    }

    fn search(query: &str, limit: u32) -> Result<Vec<Item>, String> {
        let server = Server::connect()?;
        let hubs: Container = server.get_json(&format!("/hubs/search?query={}&limit={}&includeCollections=0", url_encode(query), limit.clamp(1, 30)))?;
        let mut out: Vec<Item> = Vec::new();
        for hub in hubs.media_container.hub {
            if !matches!(hub.kind.as_str(), "movie" | "show" | "episode") {
                continue;
            }
            for m in hub.metadata {
                if let Some(item) = server.recent_item(&m) {
                    out.push(item);
                }
            }
        }
        sdk::dedup_ids(&mut out);
        Ok(out)
    }
}

sdk::export_plugin!(Plex);

/// On Deck comes most recent first, but a show's next episode, not yet started, has no time of
/// its own. Each takes a millisecond less than the one above it, so it keeps its place instead of
/// jumping to the front of Continue watching the first time Lounge sees it.
fn date_in_order(deck: &mut [Item]) {
    let first = deck.iter().position(|i| i.last_used_ms.is_some());
    let Some(first) = first else { return };
    let mut above = deck[first].last_used_ms.map(|t| t + first as i64 + 1);
    for item in deck.iter_mut() {
        match item.last_used_ms {
            Some(t) => above = Some(t),
            None => {
                above = above.map(|t| t - 1);
                item.last_used_ms = above;
            }
        }
    }
}

/// Items in the Recently added row.
const RECENT: usize = 40;
/// Libraries asked for recently added items, which keeps a refresh well inside its fetch budget.
const MAX_SECTIONS: usize = 12;

/// The movie and TV libraries named in `wanted`, or all of them when it is empty.
fn video_sections<'a>(sections: &'a [Section], wanted: &[String]) -> Vec<&'a Section> {
    sections
        .iter()
        .filter(|s| matches!(s.kind.as_str(), "movie" | "show"))
        .filter(|s| wanted.is_empty() || wanted.iter().any(|w| w.eq_ignore_ascii_case(s.title.trim())))
        .take(MAX_SECTIONS)
        .collect()
}

fn libraries_setting() -> Vec<String> {
    host::setting("libraries").unwrap_or_default().split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()
}

struct Server {
    base: String,
    token: String,
    machine: String,
    name: String,
}

#[derive(Deserialize, Default)]
struct Container {
    #[serde(rename = "MediaContainer", default)]
    media_container: MediaContainer,
}

#[derive(Deserialize, Default)]
struct MediaContainer {
    #[serde(rename = "machineIdentifier", default)]
    machine_identifier: Option<String>,
    #[serde(rename = "friendlyName", default)]
    friendly_name: Option<String>,
    #[serde(rename = "Metadata", default)]
    metadata: Vec<Metadata>,
    #[serde(rename = "Hub", default)]
    hub: Vec<Hub>,
    #[serde(rename = "Directory", default)]
    directory: Vec<Section>,
}

/// A library, as `/library/sections` lists it.
#[derive(Deserialize, Default)]
struct Section {
    #[serde(default)]
    key: String,
    #[serde(default)]
    title: String,
    #[serde(rename = "type", default)]
    kind: String,
}

#[derive(Deserialize, Default)]
struct Hub {
    #[serde(rename = "type", default)]
    kind: String,
    #[serde(rename = "Metadata", default)]
    metadata: Vec<Metadata>,
}

/// One Plex item, whatever its type; the fields a tile needs.
#[derive(Deserialize, Default, Clone, Debug)]
pub struct Metadata {
    #[serde(rename = "ratingKey", default)]
    pub rating_key: String,
    #[serde(rename = "type", default)]
    pub kind: String,
    #[serde(default)]
    pub title: String,
    #[serde(rename = "grandparentTitle")]
    pub grandparent_title: Option<String>,
    #[serde(rename = "parentTitle")]
    pub parent_title: Option<String>,
    #[serde(rename = "parentIndex")]
    pub parent_index: Option<i64>,
    #[serde(default)]
    pub index: Option<i64>,
    #[serde(default)]
    pub year: Option<i64>,
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default)]
    pub duration: Option<i64>,
    #[serde(rename = "viewOffset")]
    pub view_offset: Option<i64>,
    #[serde(rename = "lastViewedAt")]
    pub last_viewed_at: Option<i64>,
    #[serde(rename = "addedAt")]
    pub added_at: Option<i64>,
    #[serde(rename = "viewCount")]
    pub view_count: Option<i64>,
    #[serde(default)]
    pub thumb: Option<String>,
    #[serde(default)]
    pub art: Option<String>,
    #[serde(rename = "grandparentThumb")]
    pub grandparent_thumb: Option<String>,
    #[serde(rename = "grandparentArt")]
    pub grandparent_art: Option<String>,
    #[serde(rename = "parentThumb")]
    pub parent_thumb: Option<String>,
}

impl Server {
    /// Checks the address and token once per refresh and remembers the server's identity. The
    /// server's root needs the token, unlike `/identity`, so a wrong token is caught here.
    fn connect() -> Result<Server, String> {
        let base = host::server_setting("server", "Server address")?;
        let token = host::secret("token").filter(|t| !t.trim().is_empty()).ok_or_else(|| "The Plex token is missing: paste it in the plugin's settings".to_string())?;
        let probe = Server { base, token, machine: String::new(), name: "Plex".into() };
        let r = sdk::fetch_from("Plex", &probe.base, &probe.request("/"))?;
        match r.status {
            401 => return Err("Plex rejected the token. Copy a fresh X-Plex-Token from Plex Web.".into()),
            404 => return Err(sdk::wrong_server("Plex", &probe.base)),
            _ if !r.ok() => return Err(sdk::answered_error("Plex", &probe.base, r.status)),
            _ => {}
        }
        let (machine, name) = identity(&r.body).map_err(|e| {
            sdk::warn!("{e}");
            sdk::wrong_server("Plex", &probe.base)
        })?;
        Ok(Server { machine, name, ..probe })
    }

    fn request(&self, path: &str) -> HttpRequest {
        HttpRequest::get(format!("{}{}", self.base, path))
            .header("X-Plex-Token", &self.token)
            .header("X-Plex-Client-Identifier", "lounge-launcher")
            .header("X-Plex-Product", "Lounge")
            .header("Accept", "application/json")
    }

    fn get_json<T: serde::de::DeserializeOwned>(&self, path: &str) -> Result<T, String> {
        let r = sdk::fetch_from("Plex", &self.base, &self.request(path))?;
        if !r.ok() {
            sdk::warn!("HTTP {} for {path}", r.status);
            return Err(sdk::answered_error("Plex", &self.base, r.status));
        }
        serde_json::from_str(&r.body).map_err(|e| {
            sdk::warn!("unexpected answer for {path}: {e}");
            sdk::unreadable("Plex", &self.base)
        })
    }

    /// A picture through the server's resizer, so a 4000 px poster arrives at TV size.
    fn image(&self, path: &str, width: u32, height: u32) -> String {
        format!("{}/photo/:/transcode?width={width}&height={height}&minSize=1&upscale=1&url={}&X-Plex-Token={}", self.base, url_encode(path), self.token)
    }

    fn open(&self, m: &Metadata) -> Action {
        let key = url_encode(&format!("/library/metadata/{}", m.rating_key));
        Action {
            view: Some(format!("plex://preplay/?metadataKey={key}&server={}", self.machine)),
            fallback_launch: Some(APP_PACKAGE.into()),
            title: Some(m.title.clone()),
            ..Default::default()
        }
    }

    fn pictures(&self, m: &Metadata) -> (Option<String>, Option<String>) {
        let is_episode = m.kind == "episode";
        let poster = if is_episode { m.grandparent_thumb.as_deref().or(m.parent_thumb.as_deref()) } else { m.thumb.as_deref() }.map(|p| self.image(p, 600, 900));
        let thumbnail = if is_episode { m.thumb.as_deref().or(m.grandparent_art.as_deref()) } else { m.art.as_deref() }.map(|p| self.image(p, 1280, 720));
        (poster, thumbnail)
    }

    fn resume_item(&self, m: &Metadata) -> Item {
        let is_episode = m.kind == "episode";
        let (poster, thumbnail) = self.pictures(m);
        let progress = match (m.view_offset, m.duration) {
            (Some(p), Some(d)) if d > 0 => Some((p as f32 / d as f32).clamp(0.0, 1.0)),
            _ => None,
        };
        Item {
            id: format!("resume:{}", m.rating_key),
            kind: ItemKind::Resume,
            title: if is_episode { m.grandparent_title.clone().unwrap_or_else(|| m.title.clone()) } else { m.title.clone() },
            subtitle: if is_episode { Some(m.title.clone()) } else { m.summary.as_deref().map(first_sentence) },
            season: if is_episode { m.parent_index.map(|n| n.to_string()) } else { None },
            episode: if is_episode { m.index.map(|n| n.to_string()) } else { None },
            duration_ms: m.duration,
            position_ms: m.view_offset,
            progress,
            last_used_ms: m.last_viewed_at.map(|s| s * 1000),
            poster,
            thumbnail,
            action: self.open(m),
            ..Default::default()
        }
    }

    fn recent_item(&self, m: &Metadata) -> Option<Item> {
        let (title, subtitle) = match m.kind.as_str() {
            "movie" => (m.title.clone(), m.year.map(|y| y.to_string())),
            "show" => (m.title.clone(), m.year.map(|y| y.to_string())),
            "season" => (m.parent_title.clone().unwrap_or_else(|| m.title.clone()), Some(m.title.clone())),
            "episode" => (m.grandparent_title.clone().unwrap_or_else(|| m.title.clone()), Some(m.title.clone())),
            _ => return None,
        };
        let (poster, thumbnail) = self.pictures(m);
        let poster = poster.or_else(|| m.parent_thumb.as_deref().or(m.thumb.as_deref()).map(|p| self.image(p, 600, 900)));
        Some(Item {
            id: format!("recent:{}", m.rating_key),
            kind: ItemKind::Program,
            title,
            subtitle,
            season: if m.kind == "episode" { m.parent_index.map(|n| n.to_string()) } else { None },
            episode: if m.kind == "episode" { m.index.map(|n| n.to_string()) } else { None },
            added_ms: m.added_at.map(|s| s * 1000),
            poster,
            thumbnail,
            row: Some("recent".into()),
            action: self.open(m),
            ..Default::default()
        })
    }
}

/// The machine identifier and friendly name from the server's root.
fn identity(body: &str) -> Result<(String, String), String> {
    let root: Container = serde_json::from_str(body).map_err(|e| format!("unexpected answer from Plex: {e}"))?;
    let c = root.media_container;
    Ok((c.machine_identifier.unwrap_or_default(), c.friendly_name.filter(|n| !n.is_empty()).unwrap_or_else(|| "Plex".into())))
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
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn server() -> Server {
        Server { base: "http://plex:32400".into(), token: "tok".into(), machine: "abc".into(), name: "Den".into() }
    }

    const ON_DECK: &str = r#"{"MediaContainer": {"size": 1, "Metadata": [{
        "ratingKey": "1201", "type": "episode", "title": "Honeydew", "grandparentTitle": "The Bear", "parentIndex": 2, "index": 4,
        "duration": 1800000, "viewOffset": 900000, "lastViewedAt": 1760800000, "thumb": "/library/metadata/1201/thumb/1", "grandparentThumb": "/library/metadata/1100/thumb/1", "grandparentArt": "/library/metadata/1100/art/1"
    }]}}"#;

    #[test]
    fn recently_added_comes_from_the_movie_and_tv_libraries_named() {
        let c: Container = serde_json::from_str(r#"{"MediaContainer": {"Directory": [
            {"key": "1", "title": "Films", "type": "movie"},
            {"key": "2", "title": "Music", "type": "artist"},
            {"key": "3", "title": "TV Shows", "type": "show"},
            {"key": "4", "title": "Photos", "type": "photo"}
        ]}}"#)
        .unwrap();
        let keys = |wanted: &[&str]| -> Vec<String> {
            let wanted: Vec<String> = wanted.iter().map(|w| w.to_string()).collect();
            video_sections(&c.media_container.directory, &wanted).iter().map(|s| s.key.clone()).collect()
        };
        assert_eq!(keys(&[]), vec!["1", "3"]);
        assert_eq!(keys(&["tv shows"]), vec!["3"]);
        assert!(keys(&["Music"]).is_empty());
    }

    #[test]
    fn undated_next_episodes_keep_their_place_on_deck() {
        let mut deck: Vec<Item> = [None, Some(900), None, None, Some(500), None].into_iter().map(|t| Item { last_used_ms: t, ..Default::default() }).collect();
        date_in_order(&mut deck);
        assert_eq!(deck.iter().map(|i| i.last_used_ms).collect::<Vec<_>>(), [Some(901), Some(900), Some(899), Some(898), Some(500), Some(499)]);
        let mut none = vec![Item::default()];
        date_in_order(&mut none);
        assert_eq!(none[0].last_used_ms, None);
    }

    #[test]
    fn on_deck_becomes_a_resume_point() {
        let c: Container = serde_json::from_str(ON_DECK).unwrap();
        let item = server().resume_item(&c.media_container.metadata[0]);
        assert_eq!(item.kind, ItemKind::Resume);
        assert_eq!(item.title, "The Bear");
        assert_eq!(item.subtitle.as_deref(), Some("Honeydew"));
        assert_eq!((item.season.as_deref(), item.episode.as_deref()), (Some("2"), Some("4")));
        assert_eq!(item.progress, Some(0.5));
        assert_eq!(item.last_used_ms, Some(1760800000000));
        assert!(item.poster.unwrap().contains("url=%2Flibrary%2Fmetadata%2F1100%2Fthumb%2F1&X-Plex-Token=tok"));
        assert!(item.thumbnail.unwrap().contains("width=1280"));
        assert_eq!(item.action.view.as_deref(), Some("plex://preplay/?metadataKey=%2Flibrary%2Fmetadata%2F1201&server=abc"));
        assert_eq!(item.action.fallback_launch.as_deref(), Some(APP_PACKAGE));
    }

    #[test]
    fn recently_added_handles_every_type() {
        let c: Container = serde_json::from_str(r#"{"MediaContainer": {"Metadata": [
            {"ratingKey": "1", "type": "movie", "title": "Charade", "year": 1963, "thumb": "/t1", "art": "/a1", "addedAt": 1760000000},
            {"ratingKey": "2", "type": "season", "title": "Season 2", "parentTitle": "The Bear", "thumb": "/t2", "parentThumb": "/pt2"},
            {"ratingKey": "3", "type": "artist", "title": "Someone"}
        ]}}"#).unwrap();
        let s = server();
        let items: Vec<Item> = c.media_container.metadata.iter().filter_map(|m| s.recent_item(m)).collect();
        assert_eq!(items.len(), 2);
        assert_eq!((items[0].title.as_str(), items[0].subtitle.as_deref()), ("Charade", Some("1963")));
        assert_eq!(items[0].added_ms, Some(1760000000000));
        assert_eq!((items[1].title.as_str(), items[1].subtitle.as_deref()), ("The Bear", Some("Season 2")));
        assert!(items[1].poster.is_some());
    }

    #[test]
    fn the_name_comes_from_the_root_not_identity() {
        let root = r#"{"MediaContainer": {"size": 25, "allowSync": true, "friendlyName": "Den", "machineIdentifier": "abc123", "version": "1.41.0", "Directory": [{"key": "library", "title": "library"}]}}"#;
        assert_eq!(identity(root).unwrap(), ("abc123".to_string(), "Den".to_string()));
        let identity_only = r#"{"MediaContainer": {"size": 0, "claimed": true, "machineIdentifier": "abc123", "version": "1.41.0"}}"#;
        assert_eq!(identity(identity_only).unwrap().1, "Plex");
        assert!(identity("<html>").is_err());
    }

    #[test]
    fn hubs_search_flattens_the_useful_hubs() {
        let c: Container = serde_json::from_str(r#"{"MediaContainer": {"Hub": [
            {"type": "movie", "Metadata": [{"ratingKey": "9", "type": "movie", "title": "Nosferatu", "year": 1922}]},
            {"type": "actor", "Metadata": [{"ratingKey": "10", "type": "person", "title": "Max"}]}
        ]}}"#).unwrap();
        let s = server();
        let found: Vec<Item> = c.media_container.hub.iter().filter(|h| matches!(h.kind.as_str(), "movie" | "show" | "episode")).flat_map(|h| h.metadata.iter().filter_map(|m| s.recent_item(m))).collect();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].title, "Nosferatu");
    }
}
