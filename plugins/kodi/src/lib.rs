//! Kodi: what is part-way through, the next episode of each show, what was added lately and
//! Live TV, read from Kodi's library over JSON-RPC. Selecting something tells Kodi to play it,
//! and then brings Kodi to the front on this TV, or leaves it be if Kodi is a box across the room.
use lounge_plugin_sdk::{self as sdk, host, redact_url, Action, HttpRequest, Item, ItemKind, Kind, Manifest, RefreshContext, RowSpec, RowStyle, SettingKind, SettingSpec};
use serde::Deserialize;
use serde_json::{json, Value};

pub struct Kodi;

const DEFAULT_APP: &str = "org.xbmc.kodi";
/// Shows whose next episode is looked up, one request each.
const UP_NEXT_SHOWS: usize = 12;
const RECENT: usize = 20;
const IN_PROGRESS: usize = 30;

/// The error for a call Kodi answers with an error of its own.
const TURNED_DOWN: &str = "Kodi turned down the plugin's request. Check that Kodi is up to date, then try again.";
/// The error for a tile whose action this version of the plugin does not know.
const OUT_OF_DATE: &str = "This tile is out of date. Refresh the plugin, then try again.";

impl sdk::Plugin for Kodi {
    fn manifest() -> Manifest {
        Manifest {
            id: "tv.lounge.kodi".into(),
            name: "Kodi".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            description: "Continue watching, up next, recently added and Live TV from your Kodi library; Kodi plays them.".into(),
            publisher_name: "Lounge".into(),
            kinds: vec![Kind::Content, Kind::Epg, Kind::Search],
            // Loopback is refused unless named, and Kodi often runs on the TV itself.
            fetch_hosts: vec!["$server".into(), "localhost".into(), "127.0.0.1".into()],
            refresh_minutes: 10,
            rows: vec![
                RowSpec { id: "upnext".into(), title: "Up next in Kodi".into(), style: RowStyle::Landscape },
                RowSpec { id: "recent".into(), title: "New in Kodi".into(), style: RowStyle::Poster },
            ],
            settings: vec![
                SettingSpec {
                    key: "server".into(),
                    label: "Kodi address".into(),
                    kind: SettingKind::Url,
                    required: true,
                    help: Some("Like http://192.168.0.20:8080, or http://localhost:8080 when Kodi runs on this TV. In Kodi turn on Settings → Services → Control → Allow remote control via HTTP.".into()),
                    default: None,
                },
                SettingSpec { key: "username".into(), label: "Username".into(), kind: SettingKind::Text, required: false, help: Some("Kodi's default is kodi.".into()), default: Some("kodi".into()) },
                SettingSpec { key: "password".into(), label: "Password".into(), kind: SettingKind::Password, required: false, help: None, default: None },
                SettingSpec { key: "app".into(), label: "Kodi app".into(), kind: SettingKind::Text, required: false, help: Some("The package brought to the front after Kodi starts playing.".into()), default: Some(DEFAULT_APP.into()) },
                SettingSpec {
                    key: "front".into(),
                    label: "Bring Kodi to the front".into(),
                    kind: SettingKind::Toggle,
                    required: false,
                    help: Some("On when Kodi runs on this TV. Off when it is another box: it starts playing and this TV stays where it is.".into()),
                    default: Some("true".into()),
                },
                SettingSpec { key: "pvr".into(), label: "Live TV channels".into(), kind: SettingKind::Toggle, required: false, help: Some("Kodi's PVR channels in Live now and the guide.".into()), default: Some("true".into()) },
            ],
            homepage: Some("https://kodi.tv".into()),
        }
    }

    fn refresh(ctx: &RefreshContext) -> Result<Vec<Item>, String> {
        let kodi = Server::connect()?;
        let offset = ctx.utc_offset_minutes;
        let mut items = Vec::new();

        let movies: Movies = kodi.call("VideoLibrary.GetMovies", json!({
            "filter": {"field": "inprogress", "operator": "true", "value": ""},
            "properties": MOVIE_PROPS, "sort": {"method": "lastplayed", "order": "descending"}, "limits": {"end": IN_PROGRESS}
        }))?;
        items.extend(movies.movies.iter().map(|m| kodi.movie_item(m, true, offset)));
        let episodes: Episodes = kodi.call("VideoLibrary.GetEpisodes", json!({
            "filter": {"field": "inprogress", "operator": "true", "value": ""},
            "properties": EPISODE_PROPS, "sort": {"method": "lastplayed", "order": "descending"}, "limits": {"end": IN_PROGRESS}
        }))?;
        items.extend(episodes.episodes.iter().map(|e| kodi.episode_item(e, true, offset)));
        let paused = items.len();
        let in_progress: Vec<u64> = episodes.episodes.iter().map(|e| e.episodeid).collect();

        let mut up_next = 0;
        let shows: Shows = kodi.call("VideoLibrary.GetTVShows", json!({
            "filter": {"field": "inprogress", "operator": "true", "value": ""},
            "properties": ["title", "art", "lastplayed"], "sort": {"method": "lastplayed", "order": "descending"}, "limits": {"end": UP_NEXT_SHOWS}
        }))?;
        for (i, show) in shows.tvshows.iter().enumerate() {
            let next = match kodi.next_episode(show.tvshowid) {
                Ok(n) => n,
                Err(e) if e.contains("budget") => break,
                Err(e) => {
                    sdk::warn!("{}: {e}", show.title);
                    continue;
                }
            };
            let Some(e) = next.filter(|e| !in_progress.contains(&e.episodeid)) else { continue };
            let mut item = kodi.up_next_item(&e, show, offset);
            item.weight = (UP_NEXT_SHOWS - i) as i32;
            items.push(item);
            up_next += 1;
        }

        let recent_movies: Movies = kodi.call("VideoLibrary.GetRecentlyAddedMovies", json!({"properties": MOVIE_PROPS, "limits": {"end": RECENT}}))?;
        let recent_episodes: Episodes = kodi.call("VideoLibrary.GetRecentlyAddedEpisodes", json!({"properties": EPISODE_PROPS, "limits": {"end": RECENT}}))?;
        let recent = kodi.recent_items(&recent_movies.movies, &recent_episodes.episodes, offset, ctx.now_ms);
        let added = recent.len();
        items.extend(recent);

        let mut channels = 0;
        if host::setting("pvr").map(|v| v != "false").unwrap_or(true) {
            match kodi.call::<Channels>("PVR.GetChannels", json!({"channelgroupid": "alltv", "properties": ["channel", "channelnumber", "thumbnail", "hidden", "broadcastnow", "broadcastnext"]})) {
                Ok(list) => {
                    for c in list.channels.iter().filter(|c| !c.hidden) {
                        items.extend(kodi.channel_items(c));
                        channels += 1;
                    }
                }
                // Kodi without a PVR client answers with an error, which is not a problem here.
                Err(e) if e == TURNED_DOWN => sdk::info!("no Live TV"),
                Err(e) => sdk::info!("no Live TV: {e}"),
            }
        }
        sdk::dedup_ids(&mut items);
        sdk::info!("{paused} paused, {up_next} up next, {added} new, {channels} channels");
        Ok(items)
    }

    fn search(query: &str, limit: u32) -> Result<Vec<Item>, String> {
        let kodi = Server::connect()?;
        let limit = limit.clamp(1, 50) as usize;
        let movies: Movies = kodi.call("VideoLibrary.GetMovies", json!({
            "filter": {"field": "title", "operator": "contains", "value": query}, "properties": MOVIE_PROPS, "limits": {"end": limit}
        }))?;
        let shows: Shows = kodi.call("VideoLibrary.GetTVShows", json!({
            "filter": {"field": "title", "operator": "contains", "value": query}, "properties": ["title", "art", "lastplayed", "year"], "limits": {"end": limit}
        }))?;
        // Turn about, so a word that matches many films still finds the shows.
        let mut items = take_turns(movies.movies.iter().map(|m| kodi.movie_item(m, false, 0)), shows.tvshows.iter().map(|s| kodi.show_item(s)));
        items.truncate(limit);
        Ok(items)
    }

    /// Tells Kodi to play the item, then opens Kodi here or leaves it where it is.
    fn resolve(action: &str) -> Result<Action, String> {
        let kodi = Server::connect()?;
        let front = host::setting("front").map(|v| v != "false").unwrap_or(true);
        // Kodi's web server runs only while Kodi does, and Android stops it in the background,
        // so a Kodi on this TV that does not answer is opened instead.
        if front && kodi.on_this_tv() && !kodi.answers() {
            return Ok(Action { launch: Some(app()), ..Default::default() });
        }
        let unknown = || {
            sdk::warn!("unknown action {action}");
            OUT_OF_DATE.to_string()
        };
        let (what, id) = action.split_once(':').ok_or_else(unknown)?;
        let id: u64 = id.parse().map_err(|_| unknown())?;
        let item = match what {
            "movie" => json!({"movieid": id}),
            "episode" => json!({"episodeid": id}),
            "channel" => json!({"channelid": id}),
            "show" => {
                let next = kodi.next_episode(id)?.or_else(|| kodi.first_episode(id).ok().flatten()).ok_or("Kodi has no episodes of this show")?;
                json!({"episodeid": next.episodeid})
            }
            _ => return Err(unknown()),
        };
        let resume = what != "channel";
        kodi.call::<Value>("Player.Open", json!({"item": item, "options": {"resume": resume}}))
            .map_err(|e| if e == TURNED_DOWN { "Kodi would not play it. Refresh the plugin, then try again.".into() } else { e })?;
        Ok(if front { Action { launch: Some(app()), ..Default::default() } } else { Action::default() })
    }
}

sdk::export_plugin!(Kodi);

/// `base` with its host swapped for localhost, the way to Kodi on this TV.
fn on_localhost(base: &str) -> String {
    let (scheme, rest) = base.split_once("://").unwrap_or(("http", base));
    let (authority, path) = rest.split_at(rest.find('/').unwrap_or(rest.len()));
    let port = authority.rsplit_once(':').map(|(_, p)| p).filter(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()));
    format!("{scheme}://localhost{}{path}", port.map(|p| format!(":{p}")).unwrap_or_default())
}

fn app() -> String {
    host::setting("app").map(|a| a.trim().to_string()).filter(|a| !a.is_empty()).unwrap_or_else(|| DEFAULT_APP.into())
}

const MOVIE_PROPS: [&str; 8] = ["title", "year", "runtime", "resume", "art", "lastplayed", "playcount", "dateadded"];
const EPISODE_PROPS: [&str; 11] = ["title", "showtitle", "season", "episode", "runtime", "resume", "art", "lastplayed", "playcount", "dateadded", "tvshowid"];

// ---------------------------------------------------------------------------
// Kodi's answers
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct Reply<T> {
    #[serde(default = "Option::default")]
    result: Option<T>,
    #[serde(default)]
    error: Option<RpcError>,
}

#[derive(Deserialize, Default)]
struct RpcError {
    #[serde(default)]
    code: i64,
    #[serde(default)]
    message: String,
}

#[derive(Deserialize, Default)]
pub struct Movies {
    #[serde(default)]
    pub movies: Vec<Movie>,
}

#[derive(Deserialize, Default)]
pub struct Episodes {
    #[serde(default)]
    pub episodes: Vec<Episode>,
}

#[derive(Deserialize, Default)]
pub struct Shows {
    #[serde(default)]
    pub tvshows: Vec<Show>,
}

#[derive(Deserialize, Default)]
pub struct Channels {
    #[serde(default)]
    pub channels: Vec<Channel>,
}

#[derive(Deserialize, Default, Clone, Copy)]
pub struct Resume {
    #[serde(default)]
    pub position: f64,
    #[serde(default)]
    pub total: f64,
}

#[derive(Deserialize, Default)]
pub struct Movie {
    #[serde(default)]
    pub movieid: u64,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub year: u32,
    /// Seconds.
    #[serde(default)]
    pub runtime: f64,
    #[serde(default)]
    pub resume: Resume,
    #[serde(default)]
    pub art: serde_json::Map<String, Value>,
    #[serde(default)]
    pub lastplayed: Option<String>,
    #[serde(default)]
    pub playcount: u32,
    #[serde(default)]
    pub dateadded: Option<String>,
}

#[derive(Deserialize, Default)]
pub struct Episode {
    #[serde(default)]
    pub episodeid: u64,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub showtitle: String,
    #[serde(default)]
    pub season: i32,
    #[serde(default)]
    pub episode: i32,
    #[serde(default)]
    pub runtime: f64,
    #[serde(default)]
    pub resume: Resume,
    #[serde(default)]
    pub art: serde_json::Map<String, Value>,
    #[serde(default)]
    pub lastplayed: Option<String>,
    #[serde(default)]
    pub playcount: u32,
    #[serde(default)]
    pub dateadded: Option<String>,
    #[serde(default)]
    pub tvshowid: u64,
}

#[derive(Deserialize, Default)]
pub struct Show {
    #[serde(default)]
    pub tvshowid: u64,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub year: u32,
    #[serde(default)]
    pub art: serde_json::Map<String, Value>,
    #[serde(default)]
    pub lastplayed: Option<String>,
}

#[derive(Deserialize, Default)]
pub struct Channel {
    #[serde(default)]
    pub channelid: u64,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub channel: String,
    #[serde(default)]
    pub channelnumber: u32,
    #[serde(default)]
    pub thumbnail: String,
    #[serde(default)]
    pub hidden: bool,
    #[serde(default)]
    pub broadcastnow: Option<Broadcast>,
    #[serde(default)]
    pub broadcastnext: Option<Broadcast>,
}

#[derive(Deserialize, Default)]
pub struct Broadcast {
    #[serde(default)]
    pub broadcastid: u64,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub starttime: String,
    #[serde(default)]
    pub endtime: String,
    #[serde(default)]
    pub thumbnail: String,
}

// ---------------------------------------------------------------------------
// The server
// ---------------------------------------------------------------------------

pub struct Server {
    base: String,
    /// The address pictures are fetched through, with the sign-in in it.
    picture_base: String,
    auth: Option<String>,
}

impl Server {
    fn connect() -> Result<Server, String> {
        let base = host::server_setting("server", "Kodi address")?;
        let username = host::setting("username").map(|u| u.trim().to_string()).filter(|u| !u.is_empty());
        let password = host::secret("password").unwrap_or_default();
        Ok(Server::new(&base, username.as_deref(), &password))
    }

    pub fn new(base: &str, username: Option<&str>, password: &str) -> Server {
        let auth = username.map(|u| format!("Basic {}", base64(format!("{u}:{password}").as_bytes())));
        let picture_base = match username {
            Some(u) => {
                let (scheme, rest) = base.split_once("://").unwrap_or(("http", base));
                format!("{scheme}://{}:{}@{rest}", url_encode(u), url_encode(password))
            }
            None => base.to_string(),
        };
        Server { base: base.to_string(), picture_base, auth }
    }

    fn request(&self, method: &str, params: Value) -> HttpRequest {
        let body = json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}).to_string();
        let mut req = HttpRequest { method: "POST".into(), url: format!("{}/jsonrpc", self.base), headers: Default::default(), body: Some(body), timeout_ms: 0 };
        req.headers.insert("Content-Type".into(), "application/json".into());
        if let Some(a) = &self.auth {
            req.headers.insert("Authorization".into(), a.clone());
        }
        req
    }

    /// Whether anything answers at Kodi's address at all.
    fn answers(&self) -> bool {
        host::fetch(&self.request("JSONRPC.Ping", json!({}))).is_ok()
    }

    fn on_this_tv(&self) -> bool {
        lounge_plugin_sdk::host_in_setting(&self.base).is_some_and(|h| h == "localhost" || h == "::1" || h.starts_with("127."))
    }

    fn call<T: serde::de::DeserializeOwned>(&self, method: &str, params: Value) -> Result<T, String> {
        let r = host::fetch(&self.request(method, params)).map_err(|e| {
            if e.contains("budget") {
                return e;
            }
            sdk::warn!("{e}");
            if e.contains("leads to this device itself") {
                format!("{} is this TV's own address, which a plugin may not use. For Kodi on this TV, set the address to {}.", redact_url(&self.base), on_localhost(&self.base))
            } else {
                format!("Kodi didn't answer at {}. Check that it is running and allows remote control over HTTP, then try again.", redact_url(&self.base))
            }
        })?;
        match r.status {
            401 => return Err("Kodi refused the username or password. Check them in the plugin's settings.".into()),
            s if !(200..300).contains(&s) => {
                sdk::warn!("HTTP {s} for {method}");
                return Err(sdk::answered_error("Kodi", &self.base, s));
            }
            _ => {}
        }
        let unreadable = |why: &str| {
            sdk::warn!("unexpected answer to {method}: {why}");
            sdk::unreadable("Kodi", &self.base)
        };
        let reply: Reply<T> = serde_json::from_str(&r.body).map_err(|e| unreadable(&e.to_string()))?;
        if let Some(e) = reply.error {
            // Kodi without a PVR client turns down the PVR calls, which is no cause for a warning.
            sdk::info!("{method}: {} ({})", e.message, e.code);
            return Err(TURNED_DOWN.into());
        }
        reply.result.ok_or_else(|| unreadable("no result"))
    }

    /// The first episode not yet watched, in season and episode order, specials left out.
    fn next_episode(&self, tvshowid: u64) -> Result<Option<Episode>, String> {
        let list: Episodes = self.call("VideoLibrary.GetEpisodes", json!({
            "tvshowid": tvshowid,
            "filter": {"and": [{"field": "playcount", "operator": "is", "value": "0"}, {"field": "season", "operator": "greaterthan", "value": "0"}]},
            "properties": EPISODE_PROPS, "sort": {"method": "episode", "order": "ascending"}, "limits": {"end": 1}
        }))?;
        Ok(list.episodes.into_iter().next())
    }

    fn first_episode(&self, tvshowid: u64) -> Result<Option<Episode>, String> {
        let list: Episodes = self.call("VideoLibrary.GetEpisodes", json!({
            "tvshowid": tvshowid, "properties": EPISODE_PROPS, "sort": {"method": "episode", "order": "ascending"}, "limits": {"end": 1}
        }))?;
        Ok(list.episodes.into_iter().next())
    }

    /// A Kodi art reference as an address the TV can fetch, through Kodi's own image service.
    pub fn art(&self, value: Option<&Value>) -> Option<String> {
        let v = value?.as_str()?.trim();
        if v.is_empty() {
            return None;
        }
        Some(format!("{}/image/{}", self.picture_base, url_encode(v)))
    }

    pub fn movie_item(&self, m: &Movie, resume: bool, offset: i32) -> Item {
        let title = if m.title.trim().is_empty() { m.label.clone() } else { m.title.clone() };
        let duration = (m.runtime > 0.0).then(|| (m.runtime * 1000.0) as i64);
        Item {
            id: format!("movie:{}", m.movieid),
            kind: if resume { ItemKind::Resume } else { ItemKind::Program },
            title: title.clone(),
            subtitle: (m.year > 0).then(|| m.year.to_string()),
            duration_ms: duration,
            position_ms: resume.then(|| (m.resume.position * 1000.0) as i64),
            progress: progress(m.resume, resume),
            last_used_ms: parse_local(m.lastplayed.as_deref().unwrap_or(""), offset),
            added_ms: parse_local(m.dateadded.as_deref().unwrap_or(""), offset),
            poster: self.art(m.art.get("poster")),
            thumbnail: self.art(m.art.get("fanart")).or_else(|| self.art(m.art.get("thumb"))),
            action: Action { resolve: Some(format!("movie:{}", m.movieid)), title: Some(title), ..Default::default() },
            ..Default::default()
        }
    }

    pub fn episode_item(&self, e: &Episode, resume: bool, offset: i32) -> Item {
        let show = if e.showtitle.trim().is_empty() { e.label.clone() } else { e.showtitle.clone() };
        let duration = (e.runtime > 0.0).then(|| (e.runtime * 1000.0) as i64);
        Item {
            id: format!("episode:{}", e.episodeid),
            kind: if resume { ItemKind::Resume } else { ItemKind::Program },
            title: show.clone(),
            subtitle: Some(e.title.trim()).filter(|t| !t.is_empty()).map(str::to_string),
            season: (e.season >= 0).then(|| e.season.to_string()),
            episode: (e.episode > 0).then(|| e.episode.to_string()),
            duration_ms: duration,
            position_ms: resume.then(|| (e.resume.position * 1000.0) as i64),
            progress: progress(e.resume, resume),
            last_used_ms: parse_local(e.lastplayed.as_deref().unwrap_or(""), offset),
            added_ms: parse_local(e.dateadded.as_deref().unwrap_or(""), offset),
            poster: self.art(e.art.get("tvshow.poster")).or_else(|| self.art(e.art.get("season.poster"))),
            thumbnail: self.art(e.art.get("thumb")).or_else(|| self.art(e.art.get("tvshow.fanart"))),
            action: Action { resolve: Some(format!("episode:{}", e.episodeid)), title: Some(show), ..Default::default() },
            ..Default::default()
        }
    }

    /// The next episode of `show`. Up next and the New row give their tiles ids of their own, so
    /// an episode in both keeps a tile in each.
    pub fn up_next_item(&self, e: &Episode, show: &Show, offset: i32) -> Item {
        let mut item = self.episode_item(e, false, offset);
        item.id = format!("next:{}", item.id);
        item.row = Some("upnext".into());
        item.added_ms = parse_local(show.lastplayed.as_deref().unwrap_or(""), offset);
        if item.poster.is_none() {
            item.poster = self.art(show.art.get("poster"));
        }
        item
    }

    /// The unwatched films and episodes added lately, newest first. A film part-way through is
    /// still new, so these tiles carry ids apart from its resume point.
    pub fn recent_items(&self, movies: &[Movie], episodes: &[Episode], offset: i32, now_ms: i64) -> Vec<Item> {
        let mut recent: Vec<Item> = movies.iter().filter(|m| m.playcount == 0).map(|m| self.movie_item(m, false, offset)).collect();
        recent.extend(episodes.iter().filter(|e| e.playcount == 0).map(|e| self.episode_item(e, false, offset)));
        recent.sort_by_key(|i| std::cmp::Reverse(i.added_ms.unwrap_or(0)));
        recent.truncate(RECENT);
        for (i, item) in recent.iter_mut().enumerate() {
            item.id = format!("recent:{}", item.id);
            item.row = Some("recent".into());
            item.weight = (RECENT - i) as i32;
            item.is_new = item.added_ms.is_some_and(|t| now_ms - t < 7 * 86_400_000);
        }
        recent
    }

    pub fn show_item(&self, s: &Show) -> Item {
        let title = if s.title.trim().is_empty() { s.label.clone() } else { s.title.clone() };
        Item {
            id: format!("show:{}", s.tvshowid),
            kind: ItemKind::Program,
            title: title.clone(),
            subtitle: (s.year > 0).then(|| s.year.to_string()),
            poster: self.art(s.art.get("poster")),
            thumbnail: self.art(s.art.get("fanart")),
            action: Action { resolve: Some(format!("show:{}", s.tvshowid)), title: Some(title), ..Default::default() },
            ..Default::default()
        }
    }

    /// The channel, and what is on it now and next.
    pub fn channel_items(&self, c: &Channel) -> Vec<Item> {
        let name = if c.channel.trim().is_empty() { c.label.clone() } else { c.channel.clone() };
        let id = c.channelid.to_string();
        let action = Action { resolve: Some(format!("channel:{}", c.channelid)), title: Some(name.clone()), ..Default::default() };
        let logo = self.art(Some(&Value::String(c.thumbnail.clone())));
        let mut items = vec![Item {
            id: format!("ch:{id}"),
            kind: ItemKind::Channel,
            title: name.clone(),
            channel_id: Some(id.clone()),
            channel_name: Some(name.clone()),
            channel_number: (c.channelnumber > 0).then(|| c.channelnumber.to_string()),
            logo,
            action: action.clone(),
            ..Default::default()
        }];
        // Kodi keeps its guide in UTC, unlike its library's dates, which are in the TV's own time.
        for b in [&c.broadcastnow, &c.broadcastnext].into_iter().flatten() {
            let (Some(start), Some(end)) = (parse_local(&b.starttime, 0), parse_local(&b.endtime, 0)) else { continue };
            if b.title.trim().is_empty() || end <= start {
                continue;
            }
            items.push(Item {
                id: format!("b:{}:{start}", b.broadcastid),
                kind: ItemKind::Schedule,
                title: b.title.trim().to_string(),
                channel_id: Some(id.clone()),
                channel_name: Some(name.clone()),
                start_ms: Some(start),
                end_ms: Some(end),
                thumbnail: self.art(Some(&Value::String(b.thumbnail.clone()))),
                action: action.clone(),
                ..Default::default()
            });
        }
        items
    }
}

fn progress(r: Resume, resume: bool) -> Option<f32> {
    (resume && r.total > 0.0 && r.position > 0.0).then(|| (r.position / r.total).clamp(0.0, 1.0) as f32)
}

/// "2026-10-20 21:14:33" in the TV's own time, which is Kodi's: it is in the same house.
pub fn parse_local(s: &str, utc_offset_minutes: i32) -> Option<i64> {
    let s = s.trim();
    if s.len() < 19 {
        return None;
    }
    let num = |a: usize, b: usize| s.get(a..b)?.parse::<u32>().ok();
    let (year, month, day) = (num(0, 4)?, num(5, 7)?, num(8, 10)?);
    let (hour, minute, second) = (num(11, 13)?, num(14, 16)?, num(17, 19)?);
    if year == 0 || month == 0 || day == 0 {
        return None;
    }
    Some(sdk::epoch_ms(i64::from(year), month, day, hour, minute, second, utc_offset_minutes))
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

fn base64(bytes: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = chunk.iter().enumerate().fold(0u32, |acc, (i, b)| acc | (u32::from(*b) << (16 - 8 * i)));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(T[((n >> (18 - 6 * i)) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// One from `a`, one from `b`, and so on, then whatever is left of the longer.
fn take_turns<T>(a: impl IntoIterator<Item = T>, b: impl IntoIterator<Item = T>) -> Vec<T> {
    let (mut a, mut b) = (a.into_iter(), b.into_iter());
    let mut out = Vec::new();
    loop {
        match (a.next(), b.next()) {
            (None, None) => return out,
            (x, y) => out.extend(x.into_iter().chain(y)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kodi_on_this_tv_is_reached_through_localhost() {
        assert_eq!(on_localhost("http://192.168.0.30:8080"), "http://localhost:8080");
        assert_eq!(on_localhost("https://tv.home/kodi"), "https://localhost/kodi");
        assert!(Server::new("http://127.0.0.1:8080", None, "").on_this_tv());
        assert!(!Server::new("http://192.168.0.30:8080", None, "").on_this_tv());
    }

    #[test]
    fn search_results_take_turns() {
        assert_eq!(take_turns(["m1", "m2", "m3"], ["s1"]), ["m1", "s1", "m2", "m3"]);
        assert_eq!(take_turns(Vec::<&str>::new(), ["s1", "s2"]), ["s1", "s2"]);
    }

    fn kodi() -> Server {
        Server::new("http://kodi.lan:8080", Some("kodi"), "s3cret")
    }

    #[test]
    fn a_paused_movie_is_a_resume_point_with_its_art_through_kodi() {
        let reply: Reply<Movies> = serde_json::from_str(r#"{"id":1,"jsonrpc":"2.0","result":{"limits":{"end":1,"start":0,"total":1},"movies":[
            {"movieid":12,"label":"Charade","title":"Charade","year":1963,"runtime":6780,"playcount":0,"lastplayed":"2026-10-20 21:14:33","dateadded":"2026-09-01 10:00:00",
             "resume":{"position":1700.5,"total":6780},
             "art":{"poster":"image://http%3a%2f%2fimage.tmdb.org%2ft%2fp%2foriginal%2fabc.jpg/","fanart":"image://smb%3a%2f%2fnas%2fmovies%2fCharade%2ffanart.jpg/"}}]}}"#).unwrap();
        let m = &reply.result.unwrap().movies[0];
        let item = kodi().movie_item(m, true, 600);
        assert_eq!(item.id, "movie:12");
        assert_eq!(item.kind, ItemKind::Resume);
        assert_eq!(item.duration_ms, Some(6_780_000));
        assert_eq!(item.position_ms, Some(1_700_500));
        assert!((item.progress.unwrap() - 0.2508).abs() < 0.001);
        assert_eq!(item.last_used_ms, Some(sdk::epoch_ms(2026, 10, 20, 21, 14, 33, 600)));
        assert_eq!(item.poster.as_deref(), Some("http://kodi:s3cret@kodi.lan:8080/image/image%3A%2F%2Fhttp%253a%252f%252fimage.tmdb.org%252ft%252fp%252foriginal%252fabc.jpg%2F"));
        assert!(item.thumbnail.as_deref().unwrap().contains("fanart.jpg"));
        assert_eq!(item.action.resolve.as_deref(), Some("movie:12"));
        let plain = kodi().movie_item(m, false, 0);
        assert_eq!(plain.kind, ItemKind::Program);
        assert_eq!(plain.progress, None);
        assert_eq!(Server::new("http://kodi.lan:8080", None, "").art(Some(&Value::String("image://x/".into()))).as_deref(), Some("http://kodi.lan:8080/image/image%3A%2F%2Fx%2F"));
    }

    #[test]
    fn an_episode_names_its_show_and_takes_the_shows_poster() {
        let list: Episodes = serde_json::from_str(r#"{"episodes":[{"episodeid":77,"label":"2x04. Honeydew","title":"Honeydew","showtitle":"The Bear","season":2,"episode":4,"runtime":1800,"playcount":0,"tvshowid":5,
            "resume":{"position":0,"total":0},"dateadded":"2026-10-21 08:00:00","art":{"thumb":"image://a/","tvshow.poster":"image://p/","tvshow.fanart":"image://f/"}}]}"#).unwrap();
        let item = kodi().episode_item(&list.episodes[0], false, 0);
        assert_eq!(item.title, "The Bear");
        assert_eq!(item.subtitle.as_deref(), Some("Honeydew"));
        assert_eq!((item.season.as_deref(), item.episode.as_deref()), (Some("2"), Some("4")));
        assert!(item.poster.as_deref().unwrap().ends_with("/image/image%3A%2F%2Fp%2F"));
        assert!(item.thumbnail.as_deref().unwrap().ends_with("/image/image%3A%2F%2Fa%2F"));
        assert_eq!(item.added_ms, Some(sdk::epoch_ms(2026, 10, 21, 8, 0, 0, 0)));
        assert_eq!(item.progress, None);
    }

    #[test]
    fn a_new_film_part_watched_keeps_its_tile_in_the_new_row() {
        let movies: Movies = serde_json::from_str(r#"{"movies":[{"movieid":12,"title":"Charade","runtime":6780,"playcount":0,"dateadded":"2026-10-20 10:00:00",
            "resume":{"position":1700,"total":6780}}]}"#).unwrap();
        let episodes: Episodes = serde_json::from_str(r#"{"episodes":[{"episodeid":77,"title":"Honeydew","showtitle":"The Bear","season":2,"episode":4,"playcount":0,
            "dateadded":"2026-10-21 08:00:00","tvshowid":5}]}"#).unwrap();
        let show: Show = serde_json::from_str(r#"{"tvshowid":5,"title":"The Bear","lastplayed":"2026-10-19 20:00:00"}"#).unwrap();
        let now = sdk::epoch_ms(2026, 10, 22, 0, 0, 0, 0);
        let k = kodi();
        let mut items = vec![k.movie_item(&movies.movies[0], true, 0), k.up_next_item(&episodes.episodes[0], &show, 0)];
        items.extend(k.recent_items(&movies.movies, &episodes.episodes, 0, now));
        assert_eq!(sdk::dedup_ids(&mut items), 0);
        let ids: Vec<&str> = items.iter().map(|i| i.id.as_str()).collect();
        assert_eq!(ids, ["movie:12", "next:episode:77", "recent:episode:77", "recent:movie:12"]);
        assert_eq!(items[1].row.as_deref(), Some("upnext"));
        assert!(items[2..].iter().all(|i| i.row.as_deref() == Some("recent") && i.is_new));
        assert_eq!(items[3].action.resolve.as_deref(), Some("movie:12"));
    }

    #[test]
    fn a_pvr_channel_brings_now_and_next() {
        let list: Channels = serde_json::from_str(r#"{"channels":[{"channelid":3,"label":"BBC One","channel":"BBC One","channelnumber":1,"hidden":false,"thumbnail":"image://pvr/bbc.png/",
            "broadcastnow":{"broadcastid":900,"title":"The One Show","starttime":"2026-10-22 19:00:00","endtime":"2026-10-22 19:30:00","thumbnail":""},
            "broadcastnext":{"broadcastid":901,"title":"EastEnders","starttime":"2026-10-22 19:30:00","endtime":"2026-10-22 20:00:00","thumbnail":""}},
            {"channelid":4,"label":"Hidden","channel":"Hidden","channelnumber":99,"hidden":true}]}"#).unwrap();
        let items = kodi().channel_items(&list.channels[0]);
        assert_eq!(items.len(), 3);
        assert_eq!(items[0].kind, ItemKind::Channel);
        assert_eq!(items[0].channel_number.as_deref(), Some("1"));
        assert_eq!(items[0].action.resolve.as_deref(), Some("channel:3"));
        assert!(items[0].logo.as_deref().unwrap().contains("bbc.png"));
        assert_eq!(items[1].title, "The One Show");
        assert_eq!(items[1].start_ms, Some(sdk::epoch_ms(2026, 10, 22, 19, 0, 0, 0)));
        assert_eq!(items[1].end_ms, Some(sdk::epoch_ms(2026, 10, 22, 19, 30, 0, 0)));
        assert_eq!(items[2].title, "EastEnders");
        assert!(items[2].thumbnail.is_none());
        assert!(list.channels[1].hidden);
    }

    #[test]
    fn errors_from_kodi_are_read() {
        let reply: Reply<Channels> = serde_json::from_str(r#"{"id":1,"jsonrpc":"2.0","error":{"code":-32100,"message":"Failed to execute method."}}"#).unwrap();
        assert_eq!(reply.error.unwrap().code, -32100);
        assert!(reply.result.is_none());
        assert_eq!(parse_local("", 0), None);
        assert_eq!(parse_local("0000-00-00 00:00:00", 0), None);
        assert_eq!(base64(b"kodi:s3cret"), "a29kaTpzM2NyZXQ=");
    }

    #[test]
    fn kodi_on_this_tv_is_within_reach() {
        let hosts = <Kodi as sdk::Plugin>::manifest().fetch_hosts;
        assert!(hosts.iter().any(|h| h == "localhost") && hosts.iter().any(|h| h == "127.0.0.1"));
    }
}
