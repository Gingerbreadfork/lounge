//! Plex plugin: On Deck becomes Continue watching, the newest arrivals a "Recently added on
//! Plex" row, and the library answers search. Items open in the Plex app. With a pasted token it
//! talks to the server alone. Without one the person signs in with a code at plex.tv, which
//! gives the token and says where the account's servers are; everything else still comes from
//! the server.
use lounge_plugin_sdk::{self as sdk, host, Action, HttpRequest, Item, ItemKind, Kind, Manifest, RefreshContext, RowSpec, RowStyle, SettingKind, SettingSpec};
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub struct Plex;

const APP_PACKAGE: &str = "com.plexapp.android";
const PLEX_TV: &str = "https://plex.tv/api/v2";
const CLIENT_KEY: &str = "client";
const PIN_KEY: &str = "pin";
const ACCOUNT_KEY: &str = "account";
const SERVERS_KEY: &str = "servers";
/// How long a sign-in code lasts when plex.tv does not say.
const PIN_FOR_MS: i64 = 15 * 60_000;
/// Servers named in a message, so it stays short enough to read on the TV.
const SERVERS_NAMED: usize = 3;

impl sdk::Plugin for Plex {
    fn manifest() -> Manifest {
        Manifest {
            id: "tv.lounge.plex".into(),
            name: "Plex".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            description: "On Deck and recently added from your Plex Media Server.".into(),
            publisher_name: "Lounge".into(),
            kinds: vec![Kind::Content, Kind::Search],
            fetch_hosts: vec!["$server".into(), "plex.tv".into()],
            refresh_minutes: 30,
            rows: vec![RowSpec { id: "recent".into(), title: "Recently added on Plex".into(), style: RowStyle::Poster }],
            settings: vec![
                SettingSpec {
                    key: "server".into(),
                    label: "Server address".into(),
                    kind: SettingKind::Url,
                    required: false,
                    help: Some("Like http://192.168.0.10:32400. Turn the plugin on and sign in with the code it shows, and this page says what yours is.".into()),
                    default: None,
                },
                SettingSpec {
                    key: "token".into(),
                    label: "Plex token".into(),
                    kind: SettingKind::Password,
                    required: false,
                    help: Some("Not needed with a sign-in by code. To skip plex.tv, or for an address of your own, paste one: in Plex Web open any item, Get Info, View XML: the address ends in X-Plex-Token=…, copy that value.".into()),
                    default: None,
                },
                SettingSpec { key: "libraries".into(), label: "Libraries for recently added".into(), kind: SettingKind::Text, required: false, help: Some("Library names separated by commas. Empty means all of them.".into()), default: None },
            ],
            homepage: Some("https://www.plex.tv".into()),
            self_signed_servers: false,
            for_apps: vec![APP_PACKAGE.into()],
        }
    }

    fn refresh(ctx: &RefreshContext) -> Result<Vec<Item>, String> {
        let server = Server::connect(Access::for_refresh(ctx.now_ms)?)?;
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
                item.is_new = m.is_new(ctx.now_ms);
                items.push(item);
            }
        }
        sdk::dedup_ids(&mut items);
        sdk::info!("{} items from {}", items.len(), server.name);
        Ok(items)
    }

    /// Search never starts a sign-in; that is left to the refresh.
    fn search(query: &str, limit: u32) -> Result<Vec<Item>, String> {
        let Some(access) = Access::known() else { return Ok(Vec::new()) };
        let server = Server::connect(access)?;
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
    #[serde(rename = "viewedLeafCount")]
    pub viewed_leaf_count: Option<i64>,
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

impl Metadata {
    /// Whether it earns a New badge: added in the past week and none of it watched. A season or
    /// show has no `viewCount` and counts its watched episodes in `viewedLeafCount`.
    fn is_new(&self, now_ms: i64) -> bool {
        let watched = self.view_count.unwrap_or(0) > 0 || self.viewed_leaf_count.unwrap_or(0) > 0;
        sdk::new_this_week(self.added_at.map(|s| s * 1000), now_ms) && !watched
    }
}

/// Where the server is and the token to show it: both from the settings, or the address from
/// the settings and the token from a sign-in by code.
struct Access {
    base: String,
    token: String,
    by_code: bool,
}

impl Access {
    /// For a refresh, which signs in and asks plex.tv for the account's servers when it must.
    /// Without an address yet, the error says which one to enter.
    fn for_refresh(now_ms: i64) -> Result<Access, String> {
        if let Some(token) = pasted_token() {
            return Ok(Access { base: host::server_setting("server", "Server address")?, token, by_code: false });
        }
        let Some(base) = typed_server() else { return Err(where_servers_are(&list_servers(now_ms)?)) };
        let kept = kept_servers().and_then(|servers| token_for(&servers, &base));
        let token = match kept {
            Some(token) => token,
            None => {
                let servers = list_servers(now_ms)?;
                token_for(&servers, &base).ok_or_else(|| unknown_address(&servers, &base))?
            }
        };
        Ok(Access { base, token, by_code: true })
    }

    /// From what is already settled, without asking plex.tv anything.
    fn known() -> Option<Access> {
        let base = typed_server()?;
        match pasted_token() {
            Some(token) => Some(Access { base, token, by_code: false }),
            None => Some(Access { token: token_for(&kept_servers()?, &base)?, base, by_code: true }),
        }
    }
}

fn pasted_token() -> Option<String> {
    host::secret("token").map(|t| t.trim().to_string()).filter(|t| !t.is_empty())
}

fn typed_server() -> Option<String> {
    host::setting("server").filter(|v| !v.trim().is_empty()).map(|v| sdk::server_address(&v))
}

fn kept_servers() -> Option<Vec<Found>> {
    host::kv_get(SERVERS_KEY).and_then(|v| serde_json::from_slice(&v).ok())
}

/// A sign-in code waiting to be entered at plex.tv/link.
#[derive(Serialize, Deserialize)]
struct Pin {
    id: u64,
    code: String,
    expires_ms: i64,
}

/// What plex.tv says of a sign-in code: the token once the person has entered it.
#[derive(Deserialize)]
pub struct PinAnswer {
    pub id: u64,
    #[serde(default)]
    pub code: String,
    #[serde(rename = "expiresIn", default)]
    pub expires_in: Option<i64>,
    #[serde(rename = "authToken", default)]
    pub auth_token: Option<String>,
}

/// A server the account may use, as plex.tv lists it, with the token that server takes.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Found {
    pub name: String,
    pub token: String,
    pub owned: bool,
    pub https_required: bool,
    pub connections: Vec<Connection>,
}

/// One way to a server: its address on a network, and the `plex.direct` name for the same.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Connection {
    pub address: String,
    pub port: u16,
    pub uri: String,
    pub local: bool,
    pub relay: bool,
}

/// The name this TV goes by at plex.tv, made once and kept.
fn client_id(now_ms: i64) -> String {
    if let Some(id) = host::kv_get_string(CLIENT_KEY).filter(|id| !id.is_empty()) {
        return id;
    }
    let id = format!("lounge-{now_ms:x}");
    host::kv_set(CLIENT_KEY, id.as_bytes());
    id
}

fn plex_tv(request: HttpRequest, client: &str) -> HttpRequest {
    request.header("Accept", "application/json").header("X-Plex-Product", "Lounge").header("X-Plex-Client-Identifier", client)
}

/// The error for an answer from plex.tv that cannot be read; the cause goes to the log.
fn unreadable_sign_in(e: serde_json::Error) -> String {
    sdk::warn!("unexpected answer from plex.tv: {e}");
    sdk::unreadable("Plex", PLEX_TV)
}

/// The account's token once the code has been entered, so the refresh goes on with it, or else
/// the sentence the plugin's page shows meanwhile. A refresh asks about the code once.
fn pin_step(now_ms: i64) -> Result<String, String> {
    let client = client_id(now_ms);
    let pending: Option<Pin> = host::kv_get(PIN_KEY).and_then(|v| serde_json::from_slice(&v).ok()).filter(|p: &Pin| p.expires_ms > now_ms);
    if let Some(pin) = pending {
        let r = sdk::fetch_online("Plex", &plex_tv(HttpRequest::get(format!("{PLEX_TV}/pins/{}", pin.id)), &client))?;
        match r.status {
            200 => {
                let answer: PinAnswer = serde_json::from_str(&r.body).map_err(unreadable_sign_in)?;
                let Some(token) = answer.auth_token.filter(|t| !t.is_empty()) else { return Err(instruction(&pin)) };
                host::kv_delete(PIN_KEY);
                host::kv_set(ACCOUNT_KEY, token.as_bytes());
                return Ok(token);
            }
            429 => return Err(instruction(&pin)),
            s if s >= 500 => return Err(sdk::online_error("Plex", s)),
            // The code has run out or is unknown; a new one follows.
            _ => host::kv_delete(PIN_KEY),
        }
    }
    let ask = HttpRequest { method: "POST".into(), body: Some(String::new()), ..HttpRequest::get(format!("{PLEX_TV}/pins")) };
    let r = sdk::fetch_online("Plex", &plex_tv(ask, &client))?;
    if !r.ok() {
        sdk::warn!("HTTP {} when asked for a sign-in code", r.status);
        return Err(sdk::online_error("Plex", r.status));
    }
    let answer: PinAnswer = serde_json::from_str(&r.body).map_err(unreadable_sign_in)?;
    let life_ms = answer.expires_in.filter(|s| *s > 0).map_or(PIN_FOR_MS, |s| s * 1000);
    let pin = Pin { id: answer.id, code: answer.code, expires_ms: now_ms + life_ms };
    host::kv_set(PIN_KEY, serde_json::to_string(&pin).unwrap_or_default().as_bytes());
    Err(instruction(&pin))
}

fn instruction(pin: &Pin) -> String {
    format!("Sign in: on your phone open plex.tv/link and enter {}, then press Refresh now here.", pin.code)
}

/// The account's servers from plex.tv, kept for later refreshes. Signs in first when no one
/// has, and again when plex.tv no longer takes the token.
fn list_servers(now_ms: i64) -> Result<Vec<Found>, String> {
    let token = match host::kv_get_string(ACCOUNT_KEY).filter(|t| !t.is_empty()) {
        Some(token) => token,
        None => pin_step(now_ms)?,
    };
    let ask = plex_tv(HttpRequest::get(format!("{PLEX_TV}/resources?includeHttps=1&includeRelay=1")), &client_id(now_ms)).header("X-Plex-Token", &token);
    let r = sdk::fetch_online("Plex", &ask)?;
    match r.status {
        401 => {
            host::kv_delete(ACCOUNT_KEY);
            host::kv_delete(SERVERS_KEY);
            Err(pin_step(now_ms).err().unwrap_or_else(|| sdk::online_error("Plex", 401)))
        }
        _ if !r.ok() => {
            sdk::warn!("HTTP {} for the account's servers", r.status);
            Err(sdk::online_error("Plex", r.status))
        }
        _ => {
            let servers = servers_in(&r.body).map_err(unreadable_sign_in)?;
            host::kv_set(SERVERS_KEY, serde_json::to_string(&servers).unwrap_or_default().as_bytes());
            Ok(servers)
        }
    }
}

/// The servers in plex.tv's list of the account's devices, the account's own first. Players
/// and servers it gives no token for are left out.
pub fn servers_in(json: &str) -> Result<Vec<Found>, serde_json::Error> {
    let devices: Vec<Value> = serde_json::from_str(json)?;
    let mut servers: Vec<Found> = devices
        .iter()
        .filter(|d| text(&d["provides"]).split(',').any(|p| p.trim() == "server"))
        .filter_map(|d| {
            let connections = d["connections"].as_array().map(|list| list.iter().filter_map(connection).collect()).unwrap_or_default();
            Some(Found { name: text(&d["name"]), token: Some(text(&d["accessToken"])).filter(|t| !t.is_empty())?, owned: flag(&d["owned"]), https_required: flag(&d["httpsRequired"]), connections })
        })
        .collect();
    servers.sort_by_key(|s| !s.owned);
    Ok(servers)
}

fn connection(c: &Value) -> Option<Connection> {
    let port = c["port"].as_u64().or_else(|| c["port"].as_str()?.parse().ok()).and_then(|p| u16::try_from(p).ok())?;
    Some(Connection { address: text(&c["address"]), port, uri: text(&c["uri"]), local: flag(&c["local"]), relay: flag(&c["relay"]) })
}

/// A value plex.tv gives as text or as a number.
fn text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        _ => String::new(),
    }
}

/// A value plex.tv gives as true, 1 or "1".
fn flag(v: &Value) -> bool {
    match v {
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_i64() != Some(0),
        Value::String(s) => s == "1" || s.eq_ignore_ascii_case("true"),
        _ => false,
    }
}

/// The address to enter for a server: the plain one on the home network when the server takes
/// it, else its `plex.direct` name, with a relay only as the last resort.
pub fn address_to_enter(server: &Found) -> Option<String> {
    let direct = |local: bool| server.connections.iter().find(move |c| c.local == local && !c.relay);
    let plain = direct(true).filter(|c| !server.https_required && !c.address.is_empty() && !c.address.contains(':')).map(|c| format!("http://{}:{}", c.address, c.port));
    let named = || [direct(true), direct(false), server.connections.iter().find(|c| c.relay)].into_iter().flatten().map(|c| c.uri.clone()).find(|uri| !uri.is_empty());
    plain.or_else(named)
}

/// "Den is at http://192.168.0.10:32400, Cabin is at …" for the first few servers.
fn named_addresses(servers: &[Found]) -> String {
    let named: Vec<String> = servers.iter().filter_map(|s| Some(format!("{} is at {}", if s.name.is_empty() { "Your server" } else { &s.name }, address_to_enter(s)?))).take(SERVERS_NAMED).collect();
    named.join(", ")
}

/// What to do once signed in with no address entered yet.
pub fn where_servers_are(servers: &[Found]) -> String {
    match named_addresses(servers) {
        named if named.is_empty() => "Signed in to Plex, but this account has no server. Sign the server in to the same account, then press Refresh now.".into(),
        named => format!("Signed in to Plex. Now enter your server's address in the plugin's settings: {named}."),
    }
}

/// The error for an address that is none of the account's servers. A token from the sign-in
/// only goes to an address plex.tv gives for the server it belongs to.
fn unknown_address(servers: &[Found], base: &str) -> String {
    let named = named_addresses(servers);
    let yours = if named.is_empty() { String::new() } else { format!(" {named}.") };
    format!("Plex knows no server of yours at {}. Check the server address in the plugin's settings, or paste a Plex token to use this one.{yours}", sdk::address(base))
}

/// The token for the server at `base`: that of the server plex.tv lists at the same host. A
/// relay's bare address is shared between servers, so a relay counts by its name alone.
pub fn token_for(servers: &[Found], base: &str) -> Option<String> {
    let host = sdk::host_of(base)?;
    let leads_there = |c: &Connection| (!c.relay && c.address.eq_ignore_ascii_case(&host)) || sdk::host_of(&c.uri).as_deref() == Some(host.as_str());
    servers.iter().find(|s| s.connections.iter().any(leads_there)).map(|s| s.token.clone())
}

impl Server {
    /// Checks the address and token once per refresh and remembers the server's identity. The
    /// server's root needs the token, unlike `/identity`, so a wrong token is caught here.
    fn connect(access: Access) -> Result<Server, String> {
        let probe = Server { base: access.base, token: access.token, machine: String::new(), name: "Plex".into() };
        let r = sdk::fetch_from("Plex", &probe.base, &probe.request("/"))?;
        match r.status {
            // The server's token may have changed, so the account's servers are read again next time.
            401 if access.by_code => {
                host::kv_delete(SERVERS_KEY);
                return Err("Plex rejected the sign-in at this server. Press Refresh now to check it again.".into());
            }
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

    /// What plex.tv lists for an account: its own server, a friend's, and a player.
    const RESOURCES: &str = r#"[
        {"name": "Living room TV", "product": "Plex for Android (TV)", "provides": "client,player", "accessToken": null, "owned": true, "connections": []},
        {"name": "Cabin", "product": "Plex Media Server", "provides": "server", "accessToken": "cabin-token", "owned": false, "httpsRequired": true, "connections": [
            {"protocol": "https", "address": "203.0.113.20", "port": 32400, "uri": "https://203-0-113-20.ffee0011.plex.direct:32400", "local": false, "relay": false, "IPv6": false}]},
        {"name": "Office NAS", "product": "Plex Media Server", "provides": "server", "publicAddress": "203.0.113.7", "accessToken": "nas-token", "owned": true, "httpsRequired": false, "connections": [
            {"protocol": "https", "address": "192.168.0.10", "port": 32400, "uri": "https://192-168-0-10.0123abcd.plex.direct:32400", "local": true, "relay": false, "IPv6": false},
            {"protocol": "https", "address": "203.0.113.7", "port": 32400, "uri": "https://203-0-113-7.0123abcd.plex.direct:32400", "local": false, "relay": false, "IPv6": false},
            {"protocol": "https", "address": "198.51.100.9", "port": 8443, "uri": "https://198-51-100-9.0123abcd.plex.direct:8443", "local": false, "relay": true, "IPv6": false}]}
    ]"#;

    #[test]
    fn a_sign_in_code_is_shown_until_it_is_entered() {
        let waiting: PinAnswer = serde_json::from_str(
            r#"{"id": 479540091, "code": "25Q4", "product": "Lounge", "trusted": false, "qr": "https://plex.tv/api/v2/pins/qr/25Q4", "clientIdentifier": "lounge-1", "location": {"code": "AU"},
                "expiresIn": 899, "createdAt": "2026-11-09T21:03:22Z", "expiresAt": "2026-11-09T21:18:22Z", "authToken": null, "newRegistration": null}"#,
        )
        .unwrap();
        assert_eq!((waiting.id, waiting.code.as_str(), waiting.expires_in, waiting.auth_token.as_deref()), (479540091, "25Q4", Some(899), None));
        let pin = Pin { id: waiting.id, code: waiting.code, expires_ms: 0 };
        assert_eq!(instruction(&pin), "Sign in: on your phone open plex.tv/link and enter 25Q4, then press Refresh now here.");
        let entered: PinAnswer = serde_json::from_str(r#"{"id": 479540091, "code": "25Q4", "authToken": "account-token"}"#).unwrap();
        assert_eq!(entered.auth_token.as_deref(), Some("account-token"));
    }

    #[test]
    fn the_account_s_servers_say_which_address_to_enter() {
        let servers = servers_in(RESOURCES).unwrap();
        // The account's own server first; the player is no server.
        assert_eq!(servers.iter().map(|s| (s.name.as_str(), s.owned)).collect::<Vec<_>>(), [("Office NAS", true), ("Cabin", false)]);
        assert_eq!(servers[0].connections.len(), 3);
        // At home the plain address is short enough to type; a server that insists on HTTPS, or
        // one elsewhere, is entered by its plex.direct name.
        assert_eq!(address_to_enter(&servers[0]).as_deref(), Some("http://192.168.0.10:32400"));
        assert_eq!(address_to_enter(&servers[1]).as_deref(), Some("https://203-0-113-20.ffee0011.plex.direct:32400"));
        let strict = Found { https_required: true, ..servers[0].clone() };
        assert_eq!(address_to_enter(&strict).as_deref(), Some("https://192-168-0-10.0123abcd.plex.direct:32400"));
        assert_eq!(
            where_servers_are(&servers),
            "Signed in to Plex. Now enter your server's address in the plugin's settings: Office NAS is at http://192.168.0.10:32400, Cabin is at https://203-0-113-20.ffee0011.plex.direct:32400."
        );
        assert!(where_servers_are(&[]).starts_with("Signed in to Plex, but this account has no server."));
        // Older answers give flags and ports as text.
        let texty = servers_in(r#"[{"name": "Den", "provides": "server", "accessToken": "t", "owned": "1", "connections": [{"address": "10.0.0.2", "port": "32400", "uri": "", "local": "1", "relay": "0"}]}]"#).unwrap();
        assert_eq!((texty[0].owned, texty[0].connections[0].port, texty[0].connections[0].local, texty[0].connections[0].relay), (true, 32400, true, false));
        assert!(servers_in("{}").is_err());
    }

    #[test]
    fn a_server_s_token_only_goes_where_plex_says_that_server_is() {
        let servers = servers_in(RESOURCES).unwrap();
        assert_eq!(token_for(&servers, "http://192.168.0.10:32400").as_deref(), Some("nas-token"));
        assert_eq!(token_for(&servers, "https://192-168-0-10.0123ABCD.plex.direct:32400").as_deref(), Some("nas-token"));
        assert_eq!(token_for(&servers, "https://203-0-113-20.ffee0011.plex.direct:32400").as_deref(), Some("cabin-token"));
        // Another address gets nothing, and neither does a relay by its bare address.
        assert_eq!(token_for(&servers, "http://192.168.0.11:32400"), None);
        assert_eq!(token_for(&servers, "http://198.51.100.9:8443"), None);
        assert_eq!(token_for(&servers, "https://198-51-100-9.0123abcd.plex.direct:8443").as_deref(), Some("nas-token"));
        let said = unknown_address(&servers, "http://192.168.0.11:32400");
        assert!(said.starts_with("Plex knows no server of yours at 192.168.0.11:32400."), "{said}");
        assert!(said.ends_with("Office NAS is at http://192.168.0.10:32400, Cabin is at https://203-0-113-20.ffee0011.plex.direct:32400."), "{said}");
    }

    #[test]
    fn the_plugin_can_be_turned_on_with_nothing_filled_in() {
        let m = <Plex as sdk::Plugin>::manifest();
        assert!(m.validate().is_ok());
        assert!(m.settings.iter().all(|s| !s.required));
        assert_eq!(m.fetch_hosts, ["$server", "plex.tv"]);
        assert_eq!(m.for_apps, [APP_PACKAGE]);
    }

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
    fn a_new_season_with_episodes_watched_loses_its_new_badge() {
        let c: Container = serde_json::from_str(r#"{"MediaContainer": {"Metadata": [
            {"ratingKey": "2", "type": "season", "title": "Season 2", "parentTitle": "The Bear", "addedAt": 1760000000, "leafCount": 10, "viewedLeafCount": 10},
            {"ratingKey": "5", "type": "season", "title": "Season 1", "parentTitle": "Shogun", "addedAt": 1760000000, "leafCount": 10, "viewedLeafCount": 0},
            {"ratingKey": "1", "type": "movie", "title": "Charade", "addedAt": 1760000000, "viewCount": 1},
            {"ratingKey": "6", "type": "movie", "title": "Nosferatu", "addedAt": 1760000000}
        ]}}"#).unwrap();
        let now = 1_760_000_000_000 + 86_400_000;
        let new: Vec<bool> = c.media_container.metadata.iter().map(|m| m.is_new(now)).collect();
        assert_eq!(new, [false, true, false, true]);
        assert!(!c.media_container.metadata[3].is_new(now + 7 * 86_400_000));
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
