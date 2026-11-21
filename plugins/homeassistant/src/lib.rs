//! Home Assistant: the lights, scenes, blinds, heating and locks of the room this TV is in, the
//! other rooms, the home's scenes and cameras, and a row that says how things stand around the
//! home. A tile switches the thing it shows, and a sheet at the side adjusts it.
use lounge_plugin_sdk::{self as sdk, host, Action, Ask, HttpRequest, HttpResponse, Item, Kind, Manifest, RefreshContext, RowSpec, RowStyle, SettingKind, SettingSpec, Sheet, SheetRow};
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;
use serde_json::{json, Value};
use std::borrow::Cow;
use std::collections::{HashMap, HashSet};

pub struct HomeAssistant;

const SERVICE: &str = "Home Assistant";
/// Asks Home Assistant for its rooms and what is in them, in one answer.
const TEMPLATE: &str = include_str!("registry.jinja");
/// Asks for the rooms alone, for a Home Assistant that turns the first one down.
const PLAIN_TEMPLATE: &str = "{%- set ns = namespace(areas=[]) -%}{%- for a in areas() -%}{%- set ns.areas = ns.areas + [{'id': a, 'name': area_name(a), 'e': area_entities(a) | select('match', dom) | list}] -%}{%- endfor -%}{{ {'areas': ns.areas} | tojson }}";
/// Changes when the template's answer does, so an answer kept from before is asked for again.
const TEMPLATE_VERSION: u32 = 1;
/// The kinds of thing the rooms are asked about, as the template matches their ids.
const KINDS: &str = "(light|switch|input_boolean|fan|cover|lock|climate|media_player|vacuum|scene|script|humidifier|water_heater|alarm_control_panel|camera|binary_sensor|sensor)[.]";

const HERE_KEY: &str = "here";
const REGISTRY_KEY: &str = "registry";
const SNAP_KEY: &str = "snap";
const RECENT_KEY: &str = "recent";
const REFUSED_KEY: &str = "refused";
const NO_ADMIN_KEY: &str = "no_admin";
const NO_CALLS_KEY: &str = "no_calls";
const SESSION_KEY: &str = "session";
const SAID_KEY: &str = "said";
const SIGN_IN_KEY: &str = "sign_in";
/// Room for one stored value, under the 256 KB a key may hold.
const KV_ROOM: usize = 200 * 1024;
/// Most stored values the kept states may take between them.
const SNAP_PARTS: usize = 8;

/// How long the rooms are kept before Home Assistant is asked for them again.
const ROOMS_LAST_MS: i64 = 5 * 60_000;
/// How long the kept states serve a sheet before they are read again.
const STATES_LAST_MS: i64 = 45_000;
/// How long what a press found is kept for a refresh to take in.
const RECENT_LASTS_MS: i64 = 2 * 60_000;
/// How long the kept states are fresh enough for a search result to say them.
const SAID_IN_SEARCH_MS: i64 = 60_000;
const READ_TIMEOUT_MS: u32 = 2_500;
const PRESS_TIMEOUT_MS: u32 = 9_000;
const SHEET_TIMEOUT_MS: u32 = 8_000;
/// Short, so a renewal still leaves a press or a sheet time to finish.
const GRANT_TIMEOUT_MS: u32 = 4_000;

const HERE: &str = "here";
const ROOMS: &str = "rooms";
const SCENES: &str = "scenes";
const CAMERAS: &str = "cameras";
const AROUND: &str = "around";
/// Tiles the TV shows in one row.
const ROW_TILES: usize = 60;
/// Scenes that come before the lights in the room's row.
const SCENES_FIRST: usize = 8;
const CAMERA_TILES: usize = 20;
/// Rows the TV keeps of one sheet.
const SHEET_ROWS: usize = 60;
/// A battery below this many percent is low.
const LOW_BATTERY: f64 = 15.0;

const REFUSED: &str = "Home Assistant didn't accept the access token. Check it in the plugin's settings, or make a new one.";
const SIGNED_OUT: &str = "Home Assistant no longer accepts this TV's sign-in. Refresh the plugin to sign in again.";
const SIGN_IN: &str = "Sign in to Home Assistant on your phone: scan the code on the plugin's page, or tap the link in the phone companion.";
const SIGN_IN_AFTER: &str = "Sign in there as you do in Home Assistant. This TV carries on by itself.";
const NO_COMPANION: &str = "To sign in on your phone, turn on Phone companion in Lounge's settings. Or paste an access token in this plugin's settings.";
const BLOCKED: &str = "Home Assistant has blocked this TV after too many refused sign-ins. Remove the TV's address from ip_bans.yaml there, then restart Home Assistant.";
const NOT_ALLOWED: &str = "Home Assistant lets this access token read the home but not control it. Use an administrator's token in the plugin's settings.";
const TOO_LARGE: &str = "Home Assistant's list of devices is larger than the 8 MB a plugin may read, so this plugin can't show a home that large yet.";
const NO_LABELS: &str = "Labels need Home Assistant 2024.4 or newer. Clear the label in the plugin's settings, or update Home Assistant.";
const LABELS_NEED_ADMIN: &str = "Home Assistant lets only an administrator's access token read labels. Clear the label in the plugin's settings, or use an administrator's token.";
/// The error for a tile whose action this version of the plugin does not know.
const OUT_OF_DATE: &str = "This tile is out of date. Refresh the plugin, then try again.";

fn no_label(label: &str) -> String {
    format!("Home Assistant has no label called {}. Check the label in the plugin's settings.", brief(label))
}

fn still_starting(base: &str) -> String {
    format!("Home Assistant at {} is still starting. The tiles come back by themselves in a minute.", sdk::address(base))
}

fn gone(name: &str) -> String {
    format!("Home Assistant no longer has {}. If it was renamed there, its tile catches up in a few minutes.", brief(name))
}

fn not_responding(name: &str) -> String {
    format!("Home Assistant can't reach {}. Check its power and connection, then try again.", brief(name))
}

fn turned_down(name: &str) -> String {
    format!("Home Assistant turned that down for {}. Check the device in Home Assistant.", brief(name))
}

fn could_not(name: &str) -> String {
    format!("Home Assistant couldn't get {} to do that. Check the device there, then try again.", brief(name))
}

fn still_working(name: &str) -> String {
    format!("Home Assistant is still working on {}. Look at its tile in a moment.", brief(name))
}

/// A name short enough to sit inside a sentence.
fn brief(name: &str) -> String {
    if name.chars().count() > 60 { format!("{}…", name.chars().take(59).collect::<String>().trim_end()) } else { name.to_string() }
}

impl sdk::Plugin for HomeAssistant {
    fn manifest() -> Manifest {
        Manifest {
            id: "tv.lounge.homeassistant".into(),
            name: "Home Assistant".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            description: "Lights, scenes, blinds, heating, locks and cameras from Home Assistant, starting with the room this TV is in.".into(),
            publisher_name: "Lounge".into(),
            kinds: vec![Kind::Content, Kind::Search],
            fetch_hosts: vec!["$server".into()],
            refresh_minutes: 5,
            refresh_seconds: 30,
            rows: vec![
                RowSpec { id: HERE.into(), title: "This room".into(), style: RowStyle::Compact },
                RowSpec { id: ROOMS.into(), title: "Rooms".into(), style: RowStyle::Compact },
                RowSpec { id: SCENES.into(), title: "Scenes".into(), style: RowStyle::Compact },
                RowSpec { id: CAMERAS.into(), title: "Cameras".into(), style: RowStyle::Landscape },
                RowSpec { id: AROUND.into(), title: "Around the home".into(), style: RowStyle::Compact },
            ],
            settings: vec![
                SettingSpec {
                    key: "server".into(),
                    label: "Home Assistant address".into(),
                    kind: SettingKind::Url,
                    required: true,
                    help: Some("Like http://192.168.0.20:8123 or http://homeassistant.local:8123.".into()),
                    default: None,
                },
                SettingSpec {
                    key: "token".into(),
                    label: "Access token".into(),
                    kind: SettingKind::Password,
                    required: false,
                    help: Some("Leave it empty to sign in on your phone. Or make a long-lived access token in an administrator's Home Assistant profile, under Security, and paste it here.".into()),
                    default: None,
                },
                SettingSpec {
                    key: "label".into(),
                    label: "Label".into(),
                    kind: SettingKind::Text,
                    required: false,
                    help: Some("Shows only what carries this label in Home Assistant, on an entity, a device or a whole area. Left empty, everything shows.".into()),
                    default: None,
                },
                SettingSpec {
                    key: "scripts".into(),
                    label: "Scripts".into(),
                    kind: SettingKind::Toggle,
                    required: false,
                    help: Some("Adds scripts to the Scenes row. Leave it off when most of yours are parts of automations.".into()),
                    default: Some("false".into()),
                },
            ],
            homepage: Some("https://www.home-assistant.io".into()),
            self_signed_servers: true,
            for_apps: Vec::new(),
            sent_to: Default::default(),
        }
    }

    fn refresh(ctx: &RefreshContext) -> Result<Vec<Item>, String> {
        let server = Server::connect()?;
        let options = Options::read();
        let fresh = server.states(0)?;
        let recent = Recent::load();
        let seen_ms = recent.at_ms;
        let kept = Snap::stored().with(recent);
        // A Home Assistant that has just restarted lists only some of its things at first.
        if fresh.len() * 2 < kept.entities.len() && !server.running() {
            return Err(still_starting(&server.base));
        }
        let (registry, asked) = Registry::current(&server, &options, &fresh, ctx.now_ms)?;
        let chosen = host::kv_get_string(HERE_KEY);
        let here = chosen.clone().filter(|area| registry.has(area));
        // A room that has gone from Home Assistant is forgotten once its rooms have been read again.
        if asked && registry.mode == Mode::Rooms && chosen.is_some() && here.is_none() {
            host::kv_delete(HERE_KEY);
        }
        let snap = kept.renewed(fresh, ctx.now_ms);
        snap.save();
        Recent::forget(seen_ms);
        let home = Home::build(snap.entities, &registry, here.as_deref(), &options, ctx.now_ms);
        let mut items = home.items(&server.base);
        sdk::dedup_ids(&mut items);
        // The log says how the home stands when that changes, not on every refresh.
        let line = home.log_line();
        if host::kv_get_string(SAID_KEY).as_deref() != Some(line.as_str()) {
            sdk::info!("{line}");
            host::kv_set(SAID_KEY, line.as_bytes());
        }
        Ok(items)
    }

    /// Rooms, scenes and devices by name, from what the last refresh read.
    fn search(query: &str, limit: u32) -> Result<Vec<Item>, String> {
        let now_ms = host::now_ms();
        let snap = Snap::load();
        let fresh = (0..SAID_IN_SEARCH_MS).contains(&(now_ms - snap.at_ms));
        Ok(Home::from_kept(snap, now_ms).search(query, limit.clamp(1, 50) as usize, fresh))
    }

    /// Does what a tile or a sheet row asks, with one call to Home Assistant.
    fn resolve(action: &str) -> Result<Action, String> {
        let unknown = || {
            sdk::warn!("unknown action {action}");
            OUT_OF_DATE.to_string()
        };
        let press = Press::parse(action).ok_or_else(unknown)?;
        let now_ms = host::now_ms();
        if let Press::Here(area) = press {
            return choose_room(&Registry::load().unwrap_or_default(), area).ok_or_else(unknown);
        }
        let server = Server::connect()?;
        if !matches!(press, Press::Camera(_)) && server.barred(host::kv_get_string(NO_CALLS_KEY).as_deref()) {
            return Err(NOT_ALLOWED.into());
        }
        let fresh = match press.entity().filter(|_| press.reads_first()) {
            Some(id) => {
                // The kept home is read only to name something that has gone from Home Assistant.
                let known = || Snap::load().entities.iter().find(|e| e.id == id).map(Entity::name).unwrap_or_else(|| name_from_id(id));
                let entity = server.state(id)?.ok_or_else(|| gone(&known()))?;
                if entity.unavailable() {
                    return Err(not_responding(&entity.name()));
                }
                Some(entity)
            }
            None => None,
        };
        if let (Press::Camera(_), Some(camera)) = (&press, &fresh) {
            let picture = camera_picture(&server.base, camera, false).ok_or_else(|| not_responding(&camera.name()))?;
            return Ok(Action::live(picture, camera.name()));
        }
        let call = match press {
            Press::AllOff(area) | Press::AllOn(area) => Home::from_kept(Snap::load(), now_ms).all_lights(area, matches!(press, Press::AllOn(_))),
            _ => plan(&press, fresh.as_ref()),
        }
        .ok_or_else(unknown)?;
        let name = fresh.as_ref().map(Entity::name).or_else(|| press.entity().map(name_from_id)).unwrap_or_else(|| "the lights".into());
        let changed = server.call(&call, &name, fresh.is_some())?;
        // A device that has not said what became of it is read again by the next sheet.
        let told = press.entity().is_some_and(|id| changed.iter().any(|e| e.id == id));
        Recent::note(fresh.into_iter().chain(changed).collect(), !told, now_ms);
        Ok(match call.say {
            Some(words) => Action::default().said(words),
            None => Action::default(),
        })
    }

    fn sheet(id: &str) -> Result<Sheet, String> {
        let unknown = || {
            sdk::warn!("unknown sheet {id}");
            OUT_OF_DATE.to_string()
        };
        let server = Server::connect()?;
        let now_ms = host::now_ms();
        let mut snap = Snap::load();
        match id.strip_prefix("dev:").or_else(|| id.strip_prefix("src:")) {
            // One device is always read again, since a lock or a door has to show the truth.
            Some(entity) => {
                let entity = entity_id(entity).ok_or_else(unknown)?;
                let known = snap.entities.iter().find(|e| e.id == entity).map(Entity::name).unwrap_or_else(|| name_from_id(entity));
                let fresh = server.state(entity)?.ok_or_else(|| gone(&known))?;
                snap.patch(vec![fresh]);
            }
            // The list of rooms is the same whatever state things are in.
            None if id != ROOMS && !(0..STATES_LAST_MS).contains(&(now_ms - snap.at_ms)) => {
                snap = snap.renewed(server.states(SHEET_TIMEOUT_MS)?, now_ms);
                snap.save();
            }
            None => {}
        }
        Home::from_kept(snap, now_ms).sheet(id).ok_or_else(unknown)
    }
}

sdk::export_plugin!(HomeAssistant);

/// Remembers the room this TV is in, or that it has none.
fn choose_room(registry: &Registry, area: &str) -> Option<Action> {
    if area.is_empty() {
        host::kv_delete(HERE_KEY);
        return Some(Action::default().said("This TV has no room"));
    }
    let room = registry.areas.iter().find(|a| a.id == area)?;
    host::kv_set(HERE_KEY, area.as_bytes());
    Some(Action::default().said(format!("This TV is in {}", room.shown_name())))
}

/// What the person chose in the plugin's settings.
#[derive(Clone, Debug, Default, PartialEq)]
struct Options {
    label: Option<String>,
    scripts: bool,
}

impl Options {
    fn read() -> Options {
        Options { label: host::setting("label").map(|l| l.trim().to_string()).filter(|l| !l.is_empty()), scripts: host::setting("scripts").is_some_and(|v| v == "true") }
    }
}

// ---------------------------------------------------------------------------
// The server
// ---------------------------------------------------------------------------

/// How the plugin proves to Home Assistant who it is.
#[derive(Clone, Debug, PartialEq)]
enum Auth {
    /// A long-lived access token made in a person's profile.
    Token(String),
    /// The half-hour token of a sign-in made on a phone, and a name for the sign-in that stays
    /// the same each time that token is renewed.
    Session(String, String),
}

impl Auth {
    /// The `Authorization` header that goes with every request.
    fn header(&self) -> String {
        match self {
            Auth::Token(token) | Auth::Session(token, _) => format!("Bearer {token}"),
        }
    }
}

/// A sign-in made on a phone, kept between refreshes. Its access token lasts half an hour and
/// is renewed with the other for as long as Home Assistant allows.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
struct Session {
    /// The server it was made on, so a new address starts a new sign-in.
    base: String,
    /// The name this TV gave itself, which every renewal has to repeat.
    client: String,
    access: String,
    expires_ms: i64,
    refresh: String,
}

/// A sign-in that has been started on this TV and not yet finished on a phone.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
struct Started {
    base: String,
    client: String,
    landing: String,
    /// Sent with the link and checked when it comes back, so only this sign-in can finish it.
    state: String,
}

impl Started {
    /// The sign-in to offer for `base`, landing at `landing`: the one already offered while
    /// neither has changed, so the link on screen stays good while someone is using it.
    fn offer(kept: Option<Started>, base: &str, landing: &str, now_ms: i64) -> Started {
        match kept {
            Some(kept) if kept.base == base && kept.landing == landing => kept,
            _ => Started { base: base.into(), client: format!("{}/", origin(landing)), landing: landing.into(), state: fnv(&format!("{now_ms}\n{base}\n{landing}")) },
        }
    }

    /// The page on Home Assistant where the person signs in.
    fn link(&self) -> String {
        format!("{}/auth/authorize?client_id={}&redirect_uri={}&state={}", self.base, encoded(&self.client), encoded(&self.landing), self.state)
    }

    /// The code in what the landing page was opened with, when it answers this sign-in.
    fn code(&self, query: &str) -> Option<String> {
        let field = |name: &str| query.split('&').filter_map(|pair| pair.split_once('=')).find(|(key, _)| *key == name).map(|(_, value)| decoded(value));
        (field("state").as_deref() == Some(self.state.as_str())).then(|| field("code")).flatten().filter(|code| !code.is_empty())
    }
}

/// What Home Assistant answers when it hands out or renews a token.
#[derive(Deserialize)]
struct Granted {
    access_token: String,
    #[serde(default)]
    expires_in: i64,
    #[serde(default)]
    refresh_token: Option<String>,
}

/// What became of asking Home Assistant for a token.
enum Grant {
    Given(Granted),
    /// It will not give one for this code or this sign-in again.
    Ended,
    Failed(String),
}

/// Asks for a token with the fields of `form`, which no sign-in goes with.
fn grant(base: &str, form: &[(&str, &str)]) -> Grant {
    let body = form.iter().map(|(key, value)| format!("{key}={}", encoded(value))).collect::<Vec<_>>().join("&");
    let mut request = HttpRequest { method: "POST".into(), url: format!("{base}/auth/token"), headers: Default::default(), body: Some(body), timeout_ms: GRANT_TIMEOUT_MS };
    request.headers.insert("Content-Type".into(), "application/x-www-form-urlencoded".into());
    let response = match host::fetch(&request) {
        Ok(response) => response,
        Err(e) => {
            sdk::warn!("{e}");
            return Grant::Failed(sdk::didnt_answer(SERVICE, base));
        }
    };
    match response.status {
        200 => match serde_json::from_str::<Granted>(&response.body) {
            Ok(granted) if !granted.access_token.is_empty() => Grant::Given(granted),
            _ => Grant::Failed(sdk::unreadable(SERVICE, base)),
        },
        400 | 401 => {
            sdk::warn!("HTTP {} for /auth/token: {}", response.status, said(&response.body));
            Grant::Ended
        }
        403 => Grant::Failed(BLOCKED.into()),
        404 => Grant::Failed(sdk::wrong_server(SERVICE, base)),
        status => Grant::Failed(sdk::answered_error(SERVICE, base, status)),
    }
}

impl Session {
    fn load(base: &str) -> Option<Session> {
        host::kv_get_string(SESSION_KEY).and_then(|kept| serde_json::from_str::<Session>(&kept).ok()).filter(|session| session.base == base && !session.refresh.is_empty())
    }

    fn save(&self) {
        host::kv_set(SESSION_KEY, serde_json::to_string(self).unwrap_or_default().as_bytes());
    }

    /// Names this sign-in without holding its tokens, the same after every renewal.
    fn name(&self) -> String {
        fnv(&format!("{}\n{}", self.client, self.refresh))
    }

    /// Marks the access token as spent, so the next request renews it first.
    fn spend() {
        if let Some(mut session) = host::kv_get_string(SESSION_KEY).and_then(|kept| serde_json::from_str::<Session>(&kept).ok()) {
            session.expires_ms = 0;
            session.save();
        }
    }

    /// When a token granted at `now_ms` for `seconds` should be renewed: a minute early.
    fn expiry(now_ms: i64, seconds: i64) -> i64 {
        now_ms + (seconds.clamp(120, 86_400) - 60) * 1000
    }

    /// The sign-in for `base` with a token that is good now: the one kept, renewed when it has
    /// run out, or the one a phone has just finished. With none, the error asks for one.
    fn current(base: &str, now_ms: i64) -> Result<Session, String> {
        if let Some(mut session) = Session::load(base) {
            if now_ms < session.expires_ms {
                return Ok(session);
            }
            match grant(base, &[("grant_type", "refresh_token"), ("refresh_token", &session.refresh), ("client_id", &session.client)]) {
                Grant::Given(granted) => {
                    session.access = granted.access_token;
                    session.expires_ms = Session::expiry(now_ms, granted.expires_in);
                    session.save();
                    return Ok(session);
                }
                Grant::Failed(why) => return Err(why),
                // Signed out in Home Assistant, or left unused for too long.
                Grant::Ended => host::kv_delete(SESSION_KEY),
            }
        }
        let kept = host::kv_get_string(SIGN_IN_KEY).and_then(|kept| serde_json::from_str::<Started>(&kept).ok());
        if let (Some(started), Some(query)) = (kept.as_ref().filter(|started| started.base == base), host::take_callback()) {
            match started.code(&query).map(|code| grant(base, &[("grant_type", "authorization_code"), ("code", &code), ("client_id", &started.client)])) {
                Some(Grant::Given(Granted { access_token, expires_in, refresh_token: Some(refresh) })) => {
                    let session = Session { base: base.into(), client: started.client.clone(), access: access_token, expires_ms: Session::expiry(now_ms, expires_in), refresh };
                    session.save();
                    host::kv_delete(SIGN_IN_KEY);
                    return Ok(session);
                }
                Some(Grant::Failed(why)) => return Err(why),
                // A sign-in that was not this one, or a code that has run out: it is offered again.
                _ => sdk::warn!("the sign-in that came back was not finished"),
            }
        }
        let Some(landing) = host::callback() else {
            return Err(sdk::ask(Ask { say: NO_COMPANION.into(), ..Default::default() }));
        };
        let started = Started::offer(kept, base, &landing, now_ms);
        host::kv_set(SIGN_IN_KEY, serde_json::to_string(&started).unwrap_or_default().as_bytes());
        Err(sdk::ask(Ask { say: SIGN_IN.into(), link: Some(started.link()), code: None, after: Some(SIGN_IN_AFTER.into()), watch: true }))
    }
}

/// `text` as it goes in a form or a query: letters, digits and `-._~` as they are.
fn encoded(text: &str) -> String {
    text.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-._~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") }).collect()
}

/// A query's value back as text.
fn decoded(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        let hex = (bytes[at] == b'%').then(|| value.get(at + 1..at + 3).and_then(|pair| u8::from_str_radix(pair, 16).ok())).flatten();
        match hex {
            Some(byte) => {
                out.push(byte);
                at += 3;
            }
            None => {
                out.push(if bytes[at] == b'+' { b' ' } else { bytes[at] });
                at += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[derive(Clone, Debug, PartialEq)]
struct Server {
    /// The scheme, host and port, with no path.
    base: String,
    auth: Auth,
}

/// Why a request got no answer to read.
#[derive(Debug, PartialEq)]
enum Failure {
    /// Nothing answered.
    Away,
    /// It did not answer in time.
    Slow,
    /// The sign-in was refused, which Home Assistant counts against this TV's address.
    Refused,
    /// This TV's address is banned.
    Blocked,
    /// One of the host's own limits, in its words.
    Limit(String),
}

impl Server {
    /// The server with the access token from the settings, or with a sign-in made on a phone
    /// when there is none.
    fn connect() -> Result<Server, String> {
        let address = host::server_setting("server", "Home Assistant address")?;
        let token = host::secret("token").filter(|token| !tidy_token(token).is_empty());
        if token.is_some() {
            return Server::open(&address, token.as_deref(), host::kv_get_string(REFUSED_KEY).as_deref());
        }
        let base = origin(&address);
        let session = Session::current(&base, host::now_ms())?;
        let sign_in = session.name();
        Ok(Server { base, auth: Auth::Session(session.access, sign_in) })
    }

    /// The server at `address` with the token as pasted. A sign-in Home Assistant has already
    /// refused is not sent again, since every refusal counts towards a ban of this TV.
    fn open(address: &str, token: Option<&str>, refused: Option<&str>) -> Result<Server, String> {
        let token = token.map(tidy_token).filter(|t| !t.is_empty()).ok_or_else(|| sdk::not_set("access token"))?;
        let server = Server { base: origin(address), auth: Auth::Token(token) };
        if server.barred(refused) {
            return Err(REFUSED.into());
        }
        Ok(server)
    }

    /// Stands for this server and sign-in without holding either. A sign-in made on a phone
    /// keeps its mark when its token is renewed, so what it was refused is not asked again.
    fn mark(&self) -> String {
        match &self.auth {
            Auth::Token(_) => fnv(&format!("{}\n{}", self.base, self.auth.header())),
            Auth::Session(_, sign_in) => fnv(&format!("{}\nsigned in\n{sign_in}", self.base)),
        }
    }

    /// Whether `kept` is the mark of this server and sign-in, left by something it was refused.
    fn barred(&self, kept: Option<&str>) -> bool {
        kept == Some(self.mark().as_str())
    }

    fn request(&self, method: &str, path: &str, body: Option<String>, timeout_ms: u32) -> HttpRequest {
        let mut request = HttpRequest { method: method.into(), url: format!("{}{path}", self.base), headers: Default::default(), body, timeout_ms };
        request.headers.insert("Authorization".into(), self.auth.header());
        if request.body.is_some() {
            request.headers.insert("Content-Type".into(), "application/json".into());
        }
        request
    }

    /// The answer to `request`, whatever its status, unless nothing came or the sign-in failed.
    fn send(&self, request: &HttpRequest) -> Result<HttpResponse, Failure> {
        let response = host::fetch(request).map_err(|e| {
            if e == host::BUDGET_SPENT {
                return Failure::Limit(e);
            }
            if e == host::TOO_LARGE {
                return Failure::Limit(TOO_LARGE.into());
            }
            sdk::warn!("{e}");
            if e.contains("took too long") { Failure::Slow } else { Failure::Away }
        })?;
        match response.status {
            401 => Err(Failure::Refused),
            403 => Err(Failure::Blocked),
            _ => Ok(response),
        }
    }

    /// A failure as the sentence a read shows for it. A refused read is remembered.
    fn explain(&self, failure: Failure) -> String {
        match failure {
            Failure::Away | Failure::Slow => sdk::didnt_answer(SERVICE, &self.base),
            // A sign-in is renewed before the next request, and asked for again if that fails.
            Failure::Refused if matches!(self.auth, Auth::Session(..)) => {
                Session::spend();
                SIGNED_OUT.into()
            }
            Failure::Refused => {
                host::kv_set(REFUSED_KEY, self.mark().as_bytes());
                REFUSED.into()
            }
            Failure::Blocked => BLOCKED.into(),
            Failure::Limit(why) => why,
        }
    }

    /// A status that should have been 200, as a sentence.
    fn wrong_status(&self, status: u16, path: &str) -> String {
        sdk::warn!("HTTP {status} for {path}");
        if status == 404 { sdk::wrong_server(SERVICE, &self.base) } else { sdk::answered_error(SERVICE, &self.base, status) }
    }

    /// Every state of a kind this plugin shows, read one at a time so the list is never held whole.
    fn states(&self, timeout_ms: u32) -> Result<Vec<Entity>, String> {
        let path = "/api/states";
        let response = self.send(&self.request("GET", path, None, timeout_ms)).map_err(|f| self.explain(f))?;
        if response.status != 200 {
            return Err(self.wrong_status(response.status, path));
        }
        read_states(&response.body).map_err(|e| {
            sdk::warn!("unexpected answer for {path}: {e}");
            if response.body.trim_start().starts_with('<') { sdk::wrong_server(SERVICE, &self.base) } else { sdk::unreadable(SERVICE, &self.base) }
        })
    }

    /// One thing's state now, or nothing when Home Assistant no longer has it.
    fn state(&self, entity: &str) -> Result<Option<Entity>, String> {
        let path = format!("/api/states/{entity}");
        let response = self.send(&self.request("GET", &path, None, READ_TIMEOUT_MS)).map_err(|f| self.explain(f))?;
        match response.status {
            200 => {}
            404 if response.body.contains("Entity not found") => return Ok(None),
            status => return Err(self.wrong_status(status, &path)),
        }
        let raw: Raw = serde_json::from_str(&response.body).map_err(|e| {
            sdk::warn!("unexpected answer for {path}: {e}");
            sdk::unreadable(SERVICE, &self.base)
        })?;
        Ok(Entity::read(raw))
    }

    fn config(&self) -> Option<Config> {
        let response = self.send(&self.request("GET", "/api/config", None, READ_TIMEOUT_MS)).ok().filter(HttpResponse::ok)?;
        serde_json::from_str(&response.body).ok()
    }

    /// Whether Home Assistant says it has finished starting. A server that will not say counts as running.
    fn running(&self) -> bool {
        self.config().is_none_or(|config| config.state == "RUNNING")
    }

    /// Renders `template` on the server, which only an administrator's token may ask for.
    fn render(&self, template: &str, label: &str) -> Result<HttpResponse, Failure> {
        let body = json!({"template": template, "variables": {"dom": KINDS, "label": label}}).to_string();
        self.send(&self.request("POST", "/api/template", Some(body), 0))
    }

    /// Makes the call and returns the states Home Assistant says it changed. `just_read` says
    /// this sign-in read something a moment ago, so a refusal here is of the call alone.
    fn call(&self, call: &Call, name: &str, just_read: bool) -> Result<Vec<Entity>, String> {
        let path = format!("/api/services/{}", call.service);
        let response = self.send(&self.request("POST", &path, Some(call.data.to_string()), PRESS_TIMEOUT_MS)).map_err(|failure| match failure {
            Failure::Slow => still_working(name),
            // A token that still reads may not make calls, which is remembered so it is not
            // refused again. One that reads nothing has been taken away. It is only asked which
            // when no read has just worked, since every refusal counts against this TV.
            Failure::Refused => match (!just_read).then(|| self.send(&self.request("GET", "/api/config", None, READ_TIMEOUT_MS))) {
                Some(Err(Failure::Refused)) => self.explain(Failure::Refused),
                _ => {
                    host::kv_set(NO_CALLS_KEY, self.mark().as_bytes());
                    NOT_ALLOWED.into()
                }
            },
            other => self.explain(other),
        })?;
        match response.status {
            200 => Ok(read_states(&response.body).unwrap_or_default()),
            400..500 => {
                sdk::warn!("HTTP {} for {path}: {}", response.status, said(&response.body));
                Err(turned_down(name))
            }
            status => {
                sdk::warn!("HTTP {status} for {path}: {}", said(&response.body));
                Err(could_not(name))
            }
        }
    }
}

/// An address cut to its scheme, host and port, so a pasted page address still works.
fn origin(address: &str) -> String {
    let (scheme, rest) = address.split_once("://").unwrap_or(("http", address));
    let authority = rest.split(['/', '?', '#']).next().unwrap_or(rest);
    format!("{scheme}://{authority}")
}

/// A token as pasted, without the word that goes before it in a header or any line breaks.
fn tidy_token(token: &str) -> String {
    let token = token.trim();
    let bare = if token.get(..7).is_some_and(|word| word.eq_ignore_ascii_case("bearer ")) { &token[7..] } else { token };
    bare.split_whitespace().collect()
}

/// FNV-1a over `text`, as sixteen hex digits.
fn fnv(text: &str) -> String {
    fnv_of(text.as_bytes())
}

fn fnv_of(bytes: &[u8]) -> String {
    let hash = bytes.iter().fold(0xcbf2_9ce4_8422_2325_u64, |h, b| (h ^ u64::from(*b)).wrapping_mul(0x0000_0100_0000_01b3));
    format!("{hash:016x}")
}

/// What Home Assistant said went wrong, for the log.
fn said(body: &str) -> String {
    #[derive(Deserialize)]
    struct Message {
        message: String,
    }
    let text = serde_json::from_str::<Message>(body).map(|m| m.message).unwrap_or_else(|_| body.trim().to_string());
    text.chars().take(200).collect()
}

// ---------------------------------------------------------------------------
// What Home Assistant says
// ---------------------------------------------------------------------------

/// One state as it arrives. The attributes are read only for a kind the plugin shows.
#[derive(Deserialize)]
struct Raw<'a> {
    #[serde(borrow)]
    entity_id: Cow<'a, str>,
    #[serde(borrow)]
    state: Cow<'a, str>,
    #[serde(borrow, default)]
    attributes: Option<&'a RawValue>,
    #[serde(borrow, default)]
    last_changed: Option<Cow<'a, str>>,
    #[serde(borrow, default)]
    last_updated: Option<Cow<'a, str>>,
}

#[derive(Deserialize, Default)]
struct Config {
    #[serde(default)]
    unit_system: Units,
    /// "RUNNING" once everything has loaded.
    #[serde(default)]
    state: String,
    #[serde(default)]
    location_name: Option<String>,
}

#[derive(Deserialize, Default)]
struct Units {
    #[serde(default)]
    temperature: String,
}

/// The kinds of thing the plugin reads, in the order a room lists them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum Domain {
    Scene,
    Script,
    Light,
    Cover,
    Climate,
    Humidifier,
    WaterHeater,
    Fan,
    Media,
    Switch,
    Toggle,
    Lock,
    Vacuum,
    Alarm,
    Camera,
    Person,
    Weather,
    BinarySensor,
    Sensor,
}

impl Domain {
    fn of(entity: &str) -> Option<Domain> {
        Some(match entity.split_once('.')?.0 {
            "scene" => Domain::Scene,
            "script" => Domain::Script,
            "light" => Domain::Light,
            "cover" => Domain::Cover,
            "climate" => Domain::Climate,
            "fan" => Domain::Fan,
            "media_player" => Domain::Media,
            "switch" => Domain::Switch,
            "input_boolean" => Domain::Toggle,
            "lock" => Domain::Lock,
            "vacuum" => Domain::Vacuum,
            "humidifier" => Domain::Humidifier,
            "water_heater" => Domain::WaterHeater,
            "alarm_control_panel" => Domain::Alarm,
            "camera" => Domain::Camera,
            "person" => Domain::Person,
            "weather" => Domain::Weather,
            "binary_sensor" => Domain::BinarySensor,
            "sensor" => Domain::Sensor,
            _ => return None,
        })
    }

    /// Whether a room lists it among what can be done there.
    fn placed(self) -> bool {
        self <= Domain::Camera
    }

    /// Whether it gets a tile in its room's row. Scenes come before those, and alarms and
    /// cameras have rows of their own.
    fn tiled(self) -> bool {
        (Domain::Light..=Domain::Vacuum).contains(&self)
    }

    /// What one is called when its own name says no more than its room's.
    fn noun(self) -> &'static str {
        match self {
            Domain::Scene => "Scene",
            Domain::Script => "Script",
            Domain::Light => "Light",
            Domain::Cover => "Blind",
            Domain::Climate => "Thermostat",
            Domain::Fan => "Fan",
            Domain::Media => "Speaker",
            Domain::Switch | Domain::Toggle => "Switch",
            Domain::Lock => "Lock",
            Domain::Vacuum => "Vacuum",
            Domain::Humidifier => "Humidifier",
            Domain::WaterHeater => "Water heater",
            Domain::Alarm => "Alarm",
            Domain::Camera => "Camera",
            Domain::Person => "Person",
            Domain::Weather => "Weather",
            Domain::BinarySensor | Domain::Sensor => "Sensor",
        }
    }

    /// The heading its rows sit under in a room's sheet.
    fn heading(self) -> &'static str {
        match self {
            Domain::Scene | Domain::Script => "Scenes",
            Domain::Light => "Lights",
            Domain::Cover => "Blinds and doors",
            Domain::Climate | Domain::Humidifier | Domain::WaterHeater => "Climate",
            Domain::Fan => "Fans",
            Domain::Media => "Speakers and TVs",
            Domain::Switch | Domain::Toggle => "Switches",
            Domain::Lock => "Locks",
            Domain::Vacuum => "Vacuums",
            Domain::Alarm => "Alarm",
            Domain::Camera => "Cameras",
            _ => "Sensors",
        }
    }
}

/// Reads an attribute if it has the shape expected, and leaves it out otherwise.
fn lenient<'de, D: serde::Deserializer<'de>, T: serde::de::DeserializeOwned>(deserializer: D) -> Result<Option<T>, D::Error> {
    let raw: &RawValue = Deserialize::deserialize(deserializer)?;
    Ok(serde_json::from_str(raw.get()).ok())
}

/// Declares the attributes kept from a state. One of an unexpected shape is left out so the
/// rest still read, and none is written while it is absent.
macro_rules! attributes {
    ($($name:ident: $kind:ty,)*) => {
        #[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
        struct Attributes {
            $(#[serde(default, deserialize_with = "lenient", skip_serializing_if = "Option::is_none")]
            $name: Option<$kind>,)*
        }
    };
}

attributes! {
    friendly_name: String,
    device_class: String,
    unit_of_measurement: String,
    supported_features: u32,
    restored: bool,
    entity_id: Vec<String>,
    is_hue_group: bool,
    brightness: f64,
    rgb_color: [u8; 3],
    color_temp_kelvin: f64,
    min_color_temp_kelvin: f64,
    max_color_temp_kelvin: f64,
    supported_color_modes: Vec<String>,
    percentage: f64,
    percentage_step: f64,
    oscillating: bool,
    preset_mode: String,
    preset_modes: Vec<String>,
    current_position: f64,
    current_tilt_position: f64,
    code_format: String,
    code_arm_required: bool,
    current_temperature: f64,
    temperature: f64,
    target_temp_low: f64,
    target_temp_high: f64,
    target_temp_step: f64,
    min_temp: f64,
    max_temp: f64,
    hvac_action: String,
    hvac_modes: Vec<String>,
    fan_mode: String,
    fan_modes: Vec<String>,
    operation_mode: String,
    operation_list: Vec<String>,
    away_mode: String,
    humidity: f64,
    current_humidity: f64,
    min_humidity: f64,
    max_humidity: f64,
    mode: String,
    available_modes: Vec<String>,
    media_title: String,
    volume_level: f64,
    is_volume_muted: bool,
    source: String,
    source_list: Vec<String>,
    fan_speed: String,
    fan_speed_list: Vec<String>,
    access_token: String,
    temperature_unit: String,
}

/// One thing in the home, with what the plugin reads of it. This is also what is kept between
/// calls, so sheets and search need no request.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
struct Entity {
    id: String,
    state: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    changed_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    updated_ms: Option<i64>,
    #[serde(default)]
    a: Attributes,
}

const SMOKE: [&str; 3] = ["smoke", "gas", "carbon_monoxide"];
const OPENINGS: [&str; 4] = ["door", "window", "garage_door", "opening"];
const READ_OF_SENSORS: [&str; 3] = ["battery", "temperature", "humidity"];

impl Entity {
    /// The state as the plugin keeps it, or nothing for a kind it does not show. A sensor is
    /// kept only for the classes the plugin reads.
    fn read(raw: Raw) -> Option<Entity> {
        /// A sensor's class alone, read first since most sensors are of no class the plugin shows.
        #[derive(Deserialize)]
        struct Class<'a> {
            #[serde(borrow, default)]
            device_class: Option<Cow<'a, str>>,
        }
        let domain = Domain::of(&raw.entity_id)?;
        if matches!(domain, Domain::Sensor | Domain::BinarySensor) {
            let class = raw.attributes.and_then(|a| serde_json::from_str::<Class>(a.get()).ok()).and_then(|c| c.device_class).unwrap_or_default();
            let read = if domain == Domain::Sensor { READ_OF_SENSORS.contains(&&*class) } else { SMOKE.contains(&&*class) || OPENINGS.contains(&&*class) || class == "moisture" };
            if !read {
                return None;
            }
        }
        let a: Attributes = raw.attributes.and_then(|a| serde_json::from_str(a.get()).ok()).unwrap_or_default();
        // The registry remembers things that are gone, and Home Assistant lists them as restored.
        if a.restored == Some(true) {
            return None;
        }
        let time = |text: Option<Cow<str>>| text.and_then(|t| sdk::parse_iso8601(&t));
        Some(Entity { id: raw.entity_id.into_owned(), state: raw.state.into_owned(), changed_ms: time(raw.last_changed), updated_ms: time(raw.last_updated), a })
    }

    /// Only things of a kind the plugin shows are ever kept, so this always finds one.
    fn domain(&self) -> Domain {
        Domain::of(&self.id).unwrap_or(Domain::Sensor)
    }

    /// The name people gave it, tidied, or one made from its id.
    fn name(&self) -> String {
        let given = self.a.friendly_name.as_deref().map(tidy_name).filter(|n| !n.is_empty());
        given.unwrap_or_else(|| name_from_id(&self.id))
    }

    fn class(&self) -> &str {
        self.a.device_class.as_deref().unwrap_or("")
    }

    fn unavailable(&self) -> bool {
        self.state == "unavailable"
    }

    fn known(&self) -> bool {
        !matches!(self.state.as_str(), "unavailable" | "unknown" | "")
    }

    /// Whether the thing is doing what its kind does: lit, open, unlocked, playing, armed.
    fn is_on(&self) -> bool {
        let state = self.state.as_str();
        match self.domain() {
            Domain::Cover => self.known() && state != "closed",
            Domain::Lock => self.known() && state != "locked",
            Domain::Climate => matches!(self.a.hvac_action.as_deref(), Some("heating" | "preheating" | "cooling")),
            Domain::Media => matches!(state, "playing" | "buffering"),
            Domain::Vacuum => matches!(state, "cleaning" | "returning"),
            Domain::Alarm => self.known() && state != "disarmed",
            Domain::WaterHeater => self.known() && state != "off",
            Domain::Person => state == "home",
            Domain::Scene | Domain::Camera | Domain::Weather | Domain::Sensor => false,
            _ => state == "on",
        }
    }

    /// Whether Home Assistant says it supports the feature with this bit, when it says at all.
    fn supports(&self, bit: u32) -> bool {
        self.a.supported_features.is_none_or(|features| features & bit != 0)
    }

    /// A light that stands for several others.
    fn is_group(&self) -> bool {
        self.domain() == Domain::Light && (self.a.entity_id.as_ref().is_some_and(|members| !members.is_empty()) || self.a.is_hue_group == Some(true))
    }

    fn modes(&self) -> &[String] {
        self.a.supported_color_modes.as_deref().unwrap_or(&[])
    }

    fn dimmable(&self) -> bool {
        if self.a.supported_color_modes.is_none() {
            return self.a.brightness.is_some();
        }
        self.modes().iter().any(|mode| mode != "onoff")
    }

    fn takes_warmth(&self) -> bool {
        self.modes().iter().any(|mode| mode == "color_temp")
    }

    fn takes_colour(&self) -> bool {
        self.modes().iter().any(|mode| matches!(mode.as_str(), "hs" | "xy" | "rgb" | "rgbw" | "rgbww"))
    }

    /// A cover that lets people or cars through, which never moves on one press.
    fn garage_like(&self) -> bool {
        match self.class() {
            "garage" | "gate" | "door" => true,
            "" => {
                let named = format!("{} {}", self.id, self.name()).to_lowercase();
                named.contains("garage") || named.contains("gate")
            }
            _ => false,
        }
    }

    /// Whether a lock or an alarm asks for a code, which a remote cannot give.
    fn needs_code(&self) -> bool {
        self.a.code_format.as_deref().is_some_and(|format| !format.is_empty() && format != "no_code")
    }

    /// The state as a number, as a sensor gives a level or a temperature.
    fn number(&self) -> Option<f64> {
        self.state.trim().parse::<f64>().ok().filter(|n| n.is_finite())
    }
}

fn read_states(body: &str) -> Result<Vec<Entity>, String> {
    let mut entities = Vec::new();
    sdk::for_each_in_array::<Raw>(body, |raw| entities.extend(Entity::read(raw)))?;
    Ok(entities)
}

/// What the plugin last read, kept so sheets and search need no request. A large home's states
/// are more than one stored value holds, so they are kept as several lists.
#[derive(Clone, Debug, Default, PartialEq)]
struct Snap {
    at_ms: i64,
    entities: Vec<Entity>,
    /// How many lists the states were read from, so a shorter save leaves none behind.
    parts: usize,
}

/// What is stored under the snapshot's own key: when it was read, and how many lists hold it.
#[derive(Serialize, Deserialize)]
struct Kept {
    at_ms: i64,
    parts: usize,
    /// A sum of each list as stored, so a list that has not changed is not written again.
    #[serde(default)]
    sums: Vec<String>,
}

/// Whether list `part` is already stored as it is now.
fn unchanged(stored: &[String], sums: &[String], part: usize) -> bool {
    stored.get(part).is_some_and(|sum| Some(sum) == sums.get(part))
}

fn part_key(part: usize) -> String {
    format!("{SNAP_KEY}:{part}")
}

/// Takes into `entities` the states of `found` that are no older than the ones already there.
fn take_in(entities: &mut Vec<Entity>, found: Vec<Entity>) {
    for new in found {
        match entities.iter_mut().find(|e| e.id == new.id) {
            Some(old) if old.updated_ms > new.updated_ms => {}
            Some(old) => *old = new,
            None => entities.push(new),
        }
    }
}

/// What presses and single readings have found since the last refresh. It is kept apart from
/// the rest, so a press never has to store the whole home again.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
struct Recent {
    at_ms: i64,
    /// Whether something was changed that these states do not show yet.
    #[serde(default)]
    unsure: bool,
    #[serde(default)]
    entities: Vec<Entity>,
}

impl Recent {
    fn load() -> Recent {
        host::kv_get(RECENT_KEY).and_then(|v| serde_json::from_slice(&v).ok()).unwrap_or_default()
    }

    /// These with `found` taken in, as of now. What was found a while ago has been refreshed since.
    fn with(self, found: Vec<Entity>, unsure: bool, now_ms: i64) -> Recent {
        let mut recent = if (0..RECENT_LASTS_MS).contains(&(now_ms - self.at_ms)) { self } else { Recent::default() };
        take_in(&mut recent.entities, found);
        Recent { at_ms: now_ms, unsure: recent.unsure || unsure, entities: recent.entities }
    }

    /// As stored. Too many states to hold are left for the next reading to find.
    fn kept(&self) -> Vec<u8> {
        let whole = serde_json::to_vec(self).unwrap_or_default();
        if whole.len() <= KV_ROOM { whole } else { serde_json::to_vec(&Recent { at_ms: self.at_ms, unsure: true, entities: Vec::new() }).unwrap_or_default() }
    }

    fn note(found: Vec<Entity>, unsure: bool, now_ms: i64) {
        host::kv_set(RECENT_KEY, &Recent::load().with(found, unsure, now_ms).kept());
    }

    /// Clears what a refresh has taken in, unless a press has added to it since it was read at `seen_ms`.
    fn forget(seen_ms: i64) {
        if seen_ms != 0 && Recent::load().at_ms == seen_ms {
            host::kv_delete(RECENT_KEY);
        }
    }
}

impl Snap {
    /// The home as last read, with what has been found since.
    fn load() -> Snap {
        Snap::stored().with(Recent::load())
    }

    fn stored() -> Snap {
        let Some(kept) = host::kv_get(SNAP_KEY).and_then(|v| serde_json::from_slice::<Kept>(&v).ok()) else { return Snap::default() };
        Snap::read(kept.at_ms, (0..kept.parts.min(SNAP_PARTS)).map(|part| host::kv_get(&part_key(part)).unwrap_or_default()))
    }

    /// These states with what presses have found since they were read. A press whose outcome
    /// is not known yet makes them count as old, so a sheet reads them again.
    fn with(mut self, recent: Recent) -> Snap {
        if recent.at_ms < self.at_ms {
            return self;
        }
        take_in(&mut self.entities, recent.entities);
        if recent.unsure {
            self.at_ms = 0;
        }
        self
    }

    /// The states from the lists they were stored as. With a list missing, none of it is used.
    fn read(at_ms: i64, lists: impl Iterator<Item = Vec<u8>>) -> Snap {
        let mut snap = Snap { at_ms, ..Default::default() };
        for list in lists {
            let Ok(more) = serde_json::from_slice::<Vec<Entity>>(&list) else { return Snap::default() };
            snap.entities.extend(more);
            snap.parts += 1;
        }
        snap
    }

    /// The states just read. Where the kept state of a thing is the later one, as after a
    /// press that came between the read and now, the kept one stays.
    fn renewed(self, fresh: Vec<Entity>, now_ms: i64) -> Snap {
        let mut kept: HashMap<String, Entity> = self.entities.into_iter().map(|e| (e.id.clone(), e)).collect();
        let mut entities: Vec<Entity> = fresh
            .into_iter()
            .map(|new| match kept.remove(&new.id) {
                Some(old) if old.updated_ms > new.updated_ms => old,
                _ => new,
            })
            .collect();
        // The kinds a home shows first come first, so a home too large to keep whole loses its sensors.
        entities.sort_by(|a, b| (a.domain(), &a.id).cmp(&(b.domain(), &b.id)));
        Snap { at_ms: now_ms, entities, parts: self.parts }
    }

    /// Takes in states that are no older than the ones kept.
    fn patch(&mut self, changed: Vec<Entity>) {
        take_in(&mut self.entities, changed);
    }

    /// The states as the lists they are stored as, each small enough for one stored value.
    /// Past [SNAP_PARTS] lists the rest are left out.
    fn lists(&self) -> Vec<Vec<u8>> {
        let mut lists = Vec::new();
        let mut list = vec![b'['];
        for (n, entity) in self.entities.iter().enumerate() {
            let Ok(one) = serde_json::to_vec(entity) else { continue };
            if list.len() > 1 && list.len() + one.len() + 2 > KV_ROOM {
                list.push(b']');
                lists.push(std::mem::replace(&mut list, vec![b'[']));
                if lists.len() == SNAP_PARTS {
                    sdk::warn!("the home is too large to keep whole, so sheets and search know its first {n} things");
                    return lists;
                }
            }
            if list.len() > 1 {
                list.push(b',');
            }
            list.extend(one);
        }
        list.push(b']');
        lists.push(list);
        lists
    }

    /// Stores the lists that differ from what is stored, then when they were read. A home
    /// where nothing changed costs one small write.
    fn save(&self) {
        let lists = self.lists();
        let stored = host::kv_get(SNAP_KEY).and_then(|v| serde_json::from_slice::<Kept>(&v).ok()).map(|kept| kept.sums).unwrap_or_default();
        let sums: Vec<String> = lists.iter().map(|list| fnv_of(list)).collect();
        for (part, list) in lists.iter().enumerate() {
            if !unchanged(&stored, &sums, part) {
                host::kv_set(&part_key(part), list);
            }
        }
        for part in lists.len()..self.parts {
            host::kv_delete(&part_key(part));
        }
        if let Ok(kept) = serde_json::to_vec(&Kept { at_ms: self.at_ms, parts: lists.len(), sums }) {
            host::kv_set(SNAP_KEY, &kept);
        }
    }
}

/// `value` as JSON that fits the room one stored entry has, cut down with `shrink` until it
/// does. A store that is refused would end the whole call.
fn fitted<T: Serialize>(mut value: T, mut shrink: impl FnMut(&mut T) -> bool) -> Option<Vec<u8>> {
    loop {
        let bytes = serde_json::to_vec(&value).ok()?;
        if bytes.len() <= KV_ROOM {
            return Some(bytes);
        }
        if !shrink(&mut value) {
            return None;
        }
    }
}

// ---------------------------------------------------------------------------
// Rooms
// ---------------------------------------------------------------------------

/// What the template answers with.
#[derive(Deserialize, Default)]
struct Rendered {
    #[serde(default)]
    areas: Vec<Area>,
    #[serde(default)]
    hidden: Vec<String>,
    /// Whether this Home Assistant has labels at all.
    #[serde(default)]
    labels: bool,
    #[serde(default)]
    label_found: bool,
    #[serde(default)]
    label: Vec<String>,
    #[serde(default)]
    phones: Vec<String>,
}

/// A room: its entities, and those that share a device, device by device.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
struct Area {
    id: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    e: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    d: Vec<Vec<String>>,
}

impl Area {
    fn shown_name(&self) -> String {
        self.name.as_deref().map(tidy_name).filter(|n| !n.is_empty()).unwrap_or_else(|| name_from_id(&self.id))
    }
}

/// How much of the home's layout the plugin could read.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Mode {
    /// Rooms, each with what is in it.
    Rooms,
    /// No rooms: Home Assistant has none, or would not list them.
    #[default]
    Flat,
    /// No rooms, because the token is not an administrator's.
    NoAdmin,
}

/// The home's layout as last read, kept for a few minutes.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
struct Registry {
    /// The server, sign-in and label this was read for.
    sig: String,
    at_ms: i64,
    mode: Mode,
    #[serde(default)]
    fahrenheit: bool,
    /// What Home Assistant calls the home, which its own weather forecast is named after.
    #[serde(default)]
    home_name: Option<String>,
    #[serde(default)]
    areas: Vec<Area>,
    #[serde(default)]
    hidden: Vec<String>,
    #[serde(default)]
    label: Vec<String>,
    #[serde(default)]
    phones: Vec<String>,
}

impl Registry {
    fn load() -> Option<Registry> {
        host::kv_get(REGISTRY_KEY).and_then(|v| serde_json::from_slice(&v).ok())
    }

    fn has(&self, area: &str) -> bool {
        self.areas.iter().any(|a| a.id == area)
    }

    /// Whether this still stands for a server, sign-in and label signed `sig`.
    fn stands(&self, sig: &str, now_ms: i64) -> bool {
        self.sig == sig && (0..ROOMS_LAST_MS).contains(&(now_ms - self.at_ms))
    }

    /// The layout to use now, and whether Home Assistant was just asked for it.
    fn current(server: &Server, options: &Options, read: &[Entity], now_ms: i64) -> Result<(Registry, bool), String> {
        let label = options.label.as_deref().unwrap_or("");
        let sig = fnv(&format!("{}\n{label}\n{TEMPLATE_VERSION}", server.mark()));
        let kept = Registry::load();
        if let Some(registry) = kept.as_ref().filter(|r| r.stands(&sig, now_ms)) {
            return Ok((registry.clone(), false));
        }
        let config = server.config();
        let mut registry = Registry {
            sig: sig.clone(),
            at_ms: now_ms,
            fahrenheit: config.as_ref().is_some_and(|c| c.unit_system.temperature.contains('F')),
            home_name: config.and_then(|c| c.location_name),
            ..Default::default()
        };
        // A token that may not ask for the rooms is not made to ask twice.
        let no_admin = server.barred(host::kv_get_string(NO_ADMIN_KEY).as_deref());
        let rendered = if no_admin { Err(Failure::Refused) } else { server.render(TEMPLATE, label) };
        let rendered = match rendered {
            Ok(response) if response.status == 400 => {
                sdk::warn!("the rooms template was turned down: {}", said(&response.body));
                server.render(PLAIN_TEMPLATE, label)
            }
            other => other,
        };
        // An answer that is missing or cannot be read leaves the rooms as they were.
        let as_before = |registry: Registry| Ok((kept.clone().filter(|r| r.sig == sig).unwrap_or(registry), false));
        match rendered {
            Ok(response) if response.status == 200 => match serde_json::from_str::<Rendered>(&response.body) {
                Ok(rendered) if !label.is_empty() && !rendered.labels => return Err(NO_LABELS.into()),
                Ok(rendered) if !label.is_empty() && !rendered.label_found => return Err(no_label(label)),
                Ok(rendered) => registry.take(rendered),
                Err(e) => {
                    sdk::warn!("unexpected answer for the rooms: {e}");
                    return as_before(registry);
                }
            },
            // A Home Assistant too old for either template still shows the home, without rooms.
            Ok(response) if response.status == 400 => {
                sdk::warn!("the plain rooms template was turned down: {}", said(&response.body));
                if !label.is_empty() {
                    return Err(NO_LABELS.into());
                }
            }
            Ok(response) => {
                sdk::warn!("HTTP {} for the rooms: {}", response.status, said(&response.body));
                return as_before(registry);
            }
            // States read with this token a moment ago, so it is the rooms it may not ask for.
            Err(Failure::Refused) => {
                if !no_admin {
                    sdk::warn!("the access token is not an administrator's, so Home Assistant will not list the rooms");
                    host::kv_set(NO_ADMIN_KEY, server.mark().as_bytes());
                }
                if !label.is_empty() {
                    return Err(LABELS_NEED_ADMIN.into());
                }
                registry.mode = Mode::NoAdmin;
            }
            Err(Failure::Blocked) => return Err(BLOCKED.into()),
            Err(_) => return as_before(registry),
        }
        if let Some(bytes) = registry.stored(&read.iter().map(|e| e.id.as_str()).collect()) {
            host::kv_set(REGISTRY_KEY, &bytes);
        }
        Ok((registry, true))
    }

    fn take(&mut self, rendered: Rendered) {
        self.mode = if rendered.areas.is_empty() { Mode::Flat } else { Mode::Rooms };
        self.areas = rendered.areas;
        self.hidden = rendered.hidden;
        self.label = rendered.label;
        self.phones = rendered.phones;
    }

    /// The layout as it is stored: whole, or cut down a step at a time until one entry holds
    /// it. `read` names the things whose states the plugin keeps.
    fn stored(&self, read: &HashSet<&str>) -> Option<Vec<u8>> {
        let mut step = 0;
        fitted(self.clone(), |registry| {
            step += 1;
            registry.shed(step, read)
        })
    }

    /// Cuts the layout down a step at a time: first the sensors the plugin does not read, then
    /// the devices that group no switch or sensor, then every device, then the end of each
    /// room's list.
    fn shed(&mut self, step: usize, read: &HashSet<&str>) -> bool {
        let sensor = |id: &String| matches!(Domain::of(id), Some(Domain::Sensor | Domain::BinarySensor));
        let extra = |id: &String| sensor(id) || Domain::of(id) == Some(Domain::Switch);
        match step {
            1 => self.areas.iter_mut().for_each(|area| {
                area.e.retain(|id| !sensor(id) || read.contains(id.as_str()));
                area.d.iter_mut().for_each(|device| device.retain(|id| !sensor(id) || read.contains(id.as_str())));
                area.d.retain(|device| device.len() > 1);
            }),
            2 => self.areas.iter_mut().for_each(|area| area.d.retain(|device| device.iter().any(extra))),
            3 => self.areas.iter_mut().for_each(|area| area.d.clear()),
            _ => {
                let longest = self.areas.iter().map(|area| area.e.len()).max().unwrap_or(0);
                if longest == 0 {
                    return false;
                }
                let keep = longest - (longest / 8).max(1);
                sdk::warn!("the rooms are too many to keep whole, so each keeps its first {keep} things");
                self.areas.iter_mut().for_each(|area| {
                    area.e.sort_by_key(|id| Domain::of(id));
                    area.e.truncate(keep);
                });
            }
        }
        true
    }
}

// ---------------------------------------------------------------------------
// The home
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
struct Room {
    id: String,
    name: String,
    /// How warm the room is, by its thermostat or a sensor made to measure rooms.
    temperature: Option<f64>,
}

/// One thing with where it is and what it is called there.
#[derive(Clone, Debug, PartialEq)]
struct Thing {
    e: Entity,
    /// Its place in the home's list of rooms.
    room: Option<usize>,
    /// Its name without the room's, as it reads inside that room.
    name: String,
    /// A switch on a device that is mainly something else, kept off the home.
    extra: bool,
    /// A phone's own sensor, whose battery is its owner's business.
    phone: bool,
}

/// Where a thing goes, worked out before the thing itself is moved there.
struct Place {
    room: Option<usize>,
    name: String,
    extra: bool,
    phone: bool,
}

/// Everything the plugin shows, sorted out: the rooms by name, and each thing in the order a
/// row lists it, which never depends on its state.
#[derive(Clone, Debug, PartialEq)]
struct Home {
    now_ms: i64,
    mode: Mode,
    fahrenheit: bool,
    home_name: Option<String>,
    rooms: Vec<Room>,
    /// This TV's room, as a place in `rooms`.
    here: Option<usize>,
    things: Vec<Thing>,
    /// Each room's things as places in `things`, and after the last room those with no room.
    members: Vec<Vec<usize>>,
}

impl Home {
    fn build(entities: Vec<Entity>, registry: &Registry, here: Option<&str>, options: &Options, now_ms: i64) -> Home {
        let mut rooms: Vec<Room> = registry.areas.iter().map(|area| Room { id: area.id.clone(), name: area.shown_name(), temperature: None }).collect();
        rooms.sort_by_key(|room| (room.name.to_lowercase(), room.id.clone()));
        let place: HashMap<&str, usize> = rooms.iter().enumerate().map(|(i, room)| (room.id.as_str(), i)).collect();
        let mut room_of: HashMap<&str, usize> = HashMap::new();
        let mut device_of: HashMap<&str, &Vec<String>> = HashMap::new();
        for area in &registry.areas {
            for entity in &area.e {
                room_of.insert(entity, place[area.id.as_str()]);
            }
            for device in &area.d {
                for entity in device {
                    device_of.insert(entity, device);
                }
            }
        }
        let hidden: HashSet<&str> = registry.hidden.iter().map(String::as_str).collect();
        let labelled: Option<HashSet<&str>> = options.label.as_ref().map(|_| registry.label.iter().map(String::as_str).collect());
        let phones: HashSet<&str> = registry.phones.iter().map(String::as_str).collect();
        let by_id: HashMap<&str, &Entity> = entities.iter().map(|e| (e.id.as_str(), e)).collect();

        // First where each thing goes, while the others can still be looked up beside it.
        let mut measured: HashMap<usize, f64> = HashMap::new();
        let mut places: Vec<Option<Place>> = Vec::with_capacity(entities.len());
        for e in &entities {
            let domain = e.domain();
            let id = e.id.as_str();
            let carries_label = labelled.as_ref().is_none_or(|l| l.contains(id));
            // People and the weather are not devices, so a label does not put them away.
            let shown = !hidden.contains(id) && (carries_label || matches!(domain, Domain::Person | Domain::Weather)) && (domain != Domain::Script || options.scripts);
            let mut room = room_of.get(id).copied();
            // A scene made in Home Assistant has no room of its own, so it takes the one all its lights share.
            if room.is_none() && domain == Domain::Scene {
                let mut set = e.a.entity_id.iter().flatten().map(|member| room_of.get(member.as_str()).copied());
                let first = set.next().flatten();
                room = first.filter(|_| set.all(|other| other == first));
            }
            // A helper is shown only once someone has given it a room or the label.
            if !shown || (domain == Domain::Toggle && room.is_none() && labelled.is_none()) {
                places.push(None);
                continue;
            }
            let siblings = || device_of.get(id).into_iter().flat_map(|device| device.iter()).filter(move |other| other.as_str() != id);
            let main = [Domain::Light, Domain::Cover, Domain::Climate, Domain::Media, Domain::Lock, Domain::Vacuum, Domain::Fan, Domain::Camera];
            let extra = domain == Domain::Switch && siblings().any(|other| Domain::of(other).is_some_and(|d| main.contains(&d)));
            // A plug's own thermometer says nothing of the room. One that also reads humidity was made to.
            let reads_humidity = || siblings().any(|other| by_id.get(other.as_str()).is_some_and(|o| o.domain() == Domain::Sensor && o.class() == "humidity"));
            if let (Domain::Sensor, "temperature", Some(room), Some(reading)) = (domain, e.class(), room, e.number())
                && reads_humidity()
            {
                measured.entry(room).or_insert(reading);
            }
            let name = short_name(&e.name(), room.map(|i| rooms[i].name.as_str()), e);
            places.push(Some(Place { room, name, extra, phone: phones.contains(id) }));
        }
        drop(by_id);
        let mut things: Vec<Thing> = entities.into_iter().zip(places).filter_map(|(e, place)| place.map(|Place { room, name, extra, phone }| Thing { e, room, name, extra, phone })).collect();
        things.sort_by_cached_key(Thing::order);
        let here = here.and_then(|id| place.get(id).copied());
        // A room is as warm as its first thermostat says, or else as its own sensor does.
        for t in things.iter().filter(|t| t.e.domain() == Domain::Climate) {
            if let (Some(i), Some(reading)) = (t.room, t.e.a.current_temperature) {
                rooms[i].temperature.get_or_insert(reading);
            }
        }
        for (i, reading) in measured {
            rooms[i].temperature.get_or_insert(reading);
        }
        let mut members = vec![Vec::new(); rooms.len() + 1];
        for (n, t) in things.iter().enumerate() {
            members[t.room.unwrap_or(rooms.len())].push(n);
        }
        Home { now_ms, mode: registry.mode, fahrenheit: registry.fahrenheit, home_name: registry.home_name.clone(), rooms, here, things, members }
    }

    /// The home as the kept states and the kept layout make it, for a call that reads nothing new.
    fn from_kept(snap: Snap, now_ms: i64) -> Home {
        let registry = Registry::load().unwrap_or_default();
        let here = host::kv_get_string(HERE_KEY).filter(|area| registry.has(area));
        Home::build(snap.entities, &registry, here.as_deref(), &Options::read(), now_ms)
    }

    fn of(&self, domain: Domain) -> impl Iterator<Item = &Thing> {
        self.things.iter().filter(move |t| t.e.domain() == domain)
    }

    fn find(&self, entity: &str) -> Option<&Thing> {
        self.things.iter().find(|t| t.e.id == entity)
    }

    fn room_name(&self, t: &Thing) -> Option<&str> {
        t.room.map(|i| self.rooms[i].name.as_str())
    }

    /// The things of one room in their order, or with no room those that have none.
    fn within(&self, room: Option<usize>) -> impl Iterator<Item = &Thing> {
        self.members[room.unwrap_or(self.rooms.len())].iter().map(|&n| &self.things[n])
    }

    /// The devices a room lists, without its scenes and the switches kept under More.
    fn devices(&self, room: Option<usize>) -> impl Iterator<Item = &Thing> {
        self.within(room).filter(|t| t.e.domain().placed() && !t.extra && !matches!(t.e.domain(), Domain::Scene | Domain::Script))
    }

    fn scenes(&self, room: Option<usize>) -> impl Iterator<Item = &Thing> {
        self.within(room).filter(|t| matches!(t.e.domain(), Domain::Scene | Domain::Script))
    }

    /// The lights to count among `things`: each lamp once, so a group is counted only where
    /// there is nothing else.
    fn lamps<'a>(&self, things: impl Iterator<Item = &'a Thing>) -> Vec<&'a Thing> {
        let lights: Vec<&Thing> = things.filter(|t| t.e.domain() == Domain::Light && !t.extra).collect();
        let singles: Vec<&Thing> = lights.iter().copied().filter(|t| !t.e.is_group()).collect();
        if singles.is_empty() { lights } else { singles }
    }

    /// A room in a line: "2 lights on, 21°".
    fn summary(&self, room: Option<usize>) -> String {
        let lamps = self.lamps(self.within(room));
        let mut facts = Vec::new();
        if !lamps.is_empty() {
            facts.push(match lamps.iter().filter(|t| t.e.is_on()).count() {
                0 => "Lights off".to_string(),
                on => format!("{} on", counted(on, "light")),
            });
        }
        if let Some(reading) = room.and_then(|i| self.rooms[i].temperature) {
            facts.push(degrees(reading, self.fahrenheit));
        }
        if facts.is_empty() {
            facts.push(counted(self.devices(room).count(), "device"));
        }
        facts.join(", ")
    }

    /// The line for the plugin's log: "7 rooms, 41 devices, 5 lights on".
    fn log_line(&self) -> String {
        let devices = self.things.iter().filter(|t| t.e.domain().placed() && !matches!(t.e.domain(), Domain::Scene | Domain::Script)).count();
        let on = self.lamps(self.things.iter()).iter().filter(|t| t.e.is_on()).count();
        let rooms = match self.mode {
            Mode::Rooms => counted(self.rooms.len(), "room"),
            Mode::Flat => "No rooms".to_string(),
            Mode::NoAdmin => "No rooms without an administrator's token".to_string(),
        };
        format!("{rooms}, {}, {} on", counted(devices, "device"), counted(on, "light"))
    }
}

impl Thing {
    /// Where it comes among the others: by kind, a kind's leaders first, then room by room and
    /// by name within the room.
    fn order(&self) -> (Domain, bool, bool, Option<usize>, String, String) {
        let later = match self.e.domain() {
            Domain::Light => !self.e.is_group(),
            // Scenes made in Home Assistant itself come before those an integration brings by the dozen.
            Domain::Scene => self.e.a.entity_id.is_none(),
            _ => false,
        };
        (self.e.domain(), later, self.room.is_none(), self.room, self.name.to_lowercase(), self.e.id.clone())
    }
}

// ---------------------------------------------------------------------------
// Words
// ---------------------------------------------------------------------------

/// A name on one line: no control characters, single spaces, and without a last word that only
/// repeats an earlier one, as "Front door Door" does.
fn tidy_name(name: &str) -> String {
    let clean: String = name.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
    let mut words: Vec<&str> = clean.split_whitespace().collect();
    if let Some((last, rest)) = words.split_last()
        && rest.iter().any(|word| word.eq_ignore_ascii_case(last))
    {
        words.pop();
    }
    words.join(" ")
}

/// "light.bed_light" as "Bed light".
fn name_from_id(entity: &str) -> String {
    let object = entity.split_once('.').map_or(entity, |(_, object)| object);
    capitalised(&object.replace('_', " "))
}

fn capitalised(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

fn uncapitalised(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_lowercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// A thing's name inside its room: without the room's name in front, and the kind's own word
/// when nothing else is left.
fn short_name(name: &str, room: Option<&str>, e: &Entity) -> String {
    let Some(room) = room else { return name.to_string() };
    let starts = name.len() >= room.len() && name.is_char_boundary(room.len()) && name[..room.len()].eq_ignore_ascii_case(room);
    let rest = if starts { &name[room.len()..] } else { return name.to_string() };
    // "Kitchenette lamp" is not the kitchen's.
    if rest.chars().next().is_some_and(char::is_alphanumeric) {
        return name.to_string();
    }
    let rest = rest.trim_start_matches(|c: char| c.is_whitespace() || c == ':' || c == '-');
    if !rest.is_empty() {
        return capitalised(rest);
    }
    match e.domain() {
        Domain::Light if e.is_group() => "Lights".into(),
        Domain::Media if e.class() == "tv" => "TV".into(),
        domain => domain.noun().into(),
    }
}

/// "1 light", "3 lights".
fn counted(count: usize, noun: &str) -> String {
    format!("{count} {noun}{}", if count == 1 { "" } else { "s" })
}

/// A temperature with a true minus sign: whole degrees in Fahrenheit, and otherwise a decimal
/// only when there is one to show.
fn degrees(temperature: f64, fahrenheit: bool) -> String {
    let tenths = if fahrenheit { temperature.round() * 10.0 } else { (temperature * 10.0).round() };
    let (whole, tenth) = ((tenths.abs() / 10.0).trunc() as i64, (tenths.abs() % 10.0).round() as i64);
    let sign = if tenths < 0.0 { "\u{2212}" } else { "" };
    if tenth == 0 { format!("{sign}{whole}°") } else { format!("{sign}{whole}.{tenth}°") }
}

/// A share of a whole as "60%".
fn percent(value: f64) -> String {
    format!("{}%", value.round() as i64)
}

/// A lamp's brightness, which Home Assistant counts to 255, as a share. A lit lamp is never at 0%.
fn level(brightness: f64) -> String {
    percent((brightness / 255.0 * 100.0).max(1.0))
}

/// A length of time in the largest unit that fits: "40 minutes", "1 hour", "3 days".
fn span(ms: i64) -> String {
    let minutes = (ms / 60_000).max(1);
    let (count, unit) = match minutes {
        ..60 => (minutes, "minute"),
        60..2_880 => (minutes / 60, "hour"),
        _ => (minutes / 1_440, "day"),
    };
    format!("{count} {unit}{}", if count == 1 { "" } else { "s" })
}

/// One of Home Assistant's own words, such as "heat_cool" or "fan_only", as people read it.
fn worded(value: &str) -> String {
    match value {
        "heat_cool" => "Heat and cool".into(),
        other => capitalised(&other.replace('_', " ")),
    }
}

/// The names once each, in the order they first come.
fn once_each(names: impl Iterator<Item = String>) -> Vec<String> {
    let mut seen = Vec::new();
    for name in names {
        if !seen.contains(&name) {
            seen.push(name);
        }
    }
    seen
}

/// A list of names for a second line: "Ana and Ben", "Kitchen, Hall, Office".
fn listed(names: &[String]) -> String {
    match names {
        [one, two] => format!("{one} and {two}"),
        _ => names.join(", "),
    }
}

/// How a thing stands, as a tile's second line. A device that does not answer says so whatever it is.
fn line(e: &Entity, fahrenheit: bool) -> String {
    if e.unavailable() {
        return "Not responding".into();
    }
    let a = &e.a;
    let state = e.state.as_str();
    let temperature = |t: f64| degrees(t, fahrenheit);
    match e.domain() {
        Domain::Light => match (state, a.brightness.filter(|_| e.dimmable())) {
            ("on", Some(brightness)) => format!("On, {}", level(brightness)),
            _ => worded(state),
        },
        Domain::Fan => match (state, a.percentage, a.preset_mode.as_deref()) {
            ("on", _, Some(preset)) => format!("On, {}", uncapitalised(&worded(preset))),
            ("on", Some(speed), _) => format!("On, {}", percent(speed)),
            _ => worded(state),
        },
        Domain::Cover => match (state, a.current_position) {
            ("opening" | "closing", _) => worded(state),
            ("open", Some(position)) if position > 0.0 && position < 100.0 => format!("{} open", percent(position)),
            _ => worded(state),
        },
        Domain::Climate => {
            let now = a.current_temperature.map(temperature);
            let doing = match a.hvac_action.as_deref() {
                Some("heating" | "preheating") => "heating to",
                Some("cooling") => "cooling to",
                _ => "set to",
            };
            let aim = match (a.target_temp_low, a.target_temp_high, a.temperature) {
                (Some(low), Some(high), _) => Some(format!("{} to {}", temperature(low), temperature(high))),
                (_, _, Some(target)) => Some(format!("{doing} {}", temperature(target))),
                _ => None,
            };
            match (state, now, aim) {
                ("off", Some(now), _) => format!("Off, {now}"),
                ("off", None, _) => "Off".into(),
                (_, Some(now), Some(aim)) => format!("{now}, {aim}"),
                (_, Some(now), None) => now,
                (_, None, Some(aim)) => capitalised(&aim),
                _ => worded(state),
            }
        }
        Domain::Media => match (state, a.media_title.as_deref().map(str::trim).filter(|title| !title.is_empty())) {
            ("playing" | "paused", Some(title)) => format!("{}, {title}", worded(state)),
            _ => worded(state),
        },
        Domain::Vacuum => match state {
            "returning" => "Going home".into(),
            "error" => "Needs attention".into(),
            _ => worded(state),
        },
        Domain::Scene => "Scene".into(),
        Domain::Script => if state == "on" { "Running" } else { "Script" }.into(),
        Domain::Humidifier => match (state, a.current_humidity, a.humidity) {
            ("off", ..) => "Off".into(),
            (_, Some(now), Some(target)) => format!("{}, set to {}", percent(now), percent(target)),
            (_, None, Some(target)) => format!("Set to {}", percent(target)),
            _ => worded(state),
        },
        Domain::WaterHeater => match (state, a.current_temperature, a.temperature) {
            ("off", ..) => "Off".into(),
            (_, Some(now), Some(target)) => format!("{}, set to {}", temperature(now), temperature(target)),
            (_, None, Some(target)) => format!("Set to {}", temperature(target)),
            _ => worded(state),
        },
        Domain::Alarm => match state {
            "armed_home" | "armed_away" | "armed_night" | "armed_vacation" => format!("Armed, {}", &state[6..]),
            "armed_custom_bypass" => "Armed".into(),
            "pending" => "Entry delay".into(),
            _ => worded(state),
        },
        Domain::Person => match state {
            "home" => "Home".into(),
            "not_home" => "Away".into(),
            zone => worded(zone),
        },
        _ => worded(state),
    }
}

/// The glyph Lounge draws for a thing.
fn glyph(e: &Entity) -> &'static str {
    match e.domain() {
        Domain::Scene => "scene",
        Domain::Script => "script",
        Domain::Light => "light",
        Domain::Cover => match e.class() {
            "garage" | "gate" => "garage",
            "door" => "door",
            "window" => "window",
            _ if e.garage_like() => "garage",
            _ => "blind",
        },
        Domain::Climate | Domain::WaterHeater => "thermostat",
        Domain::Fan => "fan",
        Domain::Media if e.class() == "tv" => "tv",
        Domain::Media => "speaker",
        Domain::Switch => "plug",
        Domain::Toggle => "toggle",
        Domain::Lock => "lock",
        Domain::Vacuum => "vacuum",
        Domain::Humidifier => "humidity",
        Domain::Alarm => "shield",
        Domain::Camera => "camera",
        Domain::Person => "person",
        Domain::Weather => "sun",
        Domain::BinarySensor if SMOKE.contains(&e.class()) => "smoke",
        Domain::BinarySensor if e.class() == "moisture" => "drop",
        Domain::BinarySensor if e.class() == "window" => "window",
        Domain::BinarySensor => "door",
        Domain::Sensor if e.class() == "battery" => "battery",
        Domain::Sensor => "thermostat",
    }
}

/// The weather in a couple of words, for the conditions Home Assistant names.
fn sky(condition: &str) -> Option<&'static str> {
    Some(match condition {
        "clear-night" => "Clear",
        "cloudy" => "Cloudy",
        "fog" => "Fog",
        "hail" => "Hail",
        "lightning" => "Thunderstorm",
        "lightning-rainy" => "Thunder and rain",
        "partlycloudy" => "Partly cloudy",
        "pouring" => "Heavy rain",
        "rainy" => "Rain",
        "snowy" => "Snow",
        "snowy-rainy" => "Sleet",
        "sunny" => "Sunny",
        "windy" | "windy-variant" => "Windy",
        _ => return None,
    })
}

// ---------------------------------------------------------------------------
// Tiles
// ---------------------------------------------------------------------------

/// A tile with a glyph and no state of its own.
fn tile(id: impl Into<String>, title: impl Into<String>, subtitle: Option<String>, glyph: &str) -> Item {
    Item { id: id.into(), title: title.into(), subtitle, glyph: Some(glyph.into()), ..Default::default() }
}

/// Puts tiles in a row in the order given, which the TV keeps by their weights.
fn placed(row: &str, mut tiles: Vec<Item>) -> Vec<Item> {
    tiles.truncate(ROW_TILES);
    let count = tiles.len();
    for (i, tile) in tiles.iter_mut().enumerate() {
        tile.row = Some(row.into());
        tile.weight = (count - i) as i32;
    }
    tiles
}

fn pressing(action: String) -> Action {
    Action { resolve: Some(action), ..Default::default() }
}

fn device_sheet(e: &Entity) -> String {
    format!("dev:{}", e.id)
}

/// A camera's picture as it is now. The address carries the camera's own short-lived token,
/// never the access token.
fn camera_picture(base: &str, camera: &Entity, small: bool) -> Option<String> {
    let token = camera.a.access_token.as_deref().filter(|t| !t.is_empty() && t.bytes().all(|b| b.is_ascii_alphanumeric()))?;
    let size = if small { "&width=960&height=540" } else { "" };
    Some(format!("{base}/api/camera_proxy/{}?token={token}{size}", camera.id))
}

/// "#FFA452" for a lamp that is lit in a colour of its own.
fn tint(e: &Entity) -> Option<String> {
    let [r, g, b] = e.a.rgb_color.filter(|_| e.domain() == Domain::Light && e.is_on())?;
    Some(format!("#{r:02X}{g:02X}{b:02X}"))
}

/// What OK does on a thing's tile, and the sheet its menu offers. Anything that lets someone
/// in, or cannot be done in one safe press, opens its sheet.
fn press_of(e: &Entity) -> (Action, Option<String>) {
    let id = &e.id;
    let sheet = device_sheet(e);
    let opens = || (Action::sheet(sheet.clone()), None);
    match e.domain() {
        Domain::Scene => (pressing(format!("scene:{id}")), None),
        Domain::Script => (pressing(format!("script:{id}")), None),
        Domain::Light => (pressing(format!("toggle:{id}")), (e.dimmable() || e.is_group()).then_some(sheet)),
        Domain::Fan | Domain::Humidifier => (pressing(format!("toggle:{id}")), Some(sheet)),
        Domain::Switch | Domain::Toggle => (pressing(format!("toggle:{id}")), None),
        Domain::Cover if e.garage_like() => opens(),
        Domain::Cover => (pressing(format!("cover:{id}:toggle")), Some(sheet)),
        Domain::Lock if e.state == "unlocked" && !e.needs_code() => (pressing(format!("lock:{id}")), Some(sheet)),
        Domain::Media => (pressing(format!("media:{id}:play_pause")), Some(sheet)),
        Domain::Vacuum => (pressing(format!("vacuum:{id}:{}", if e.state == "cleaning" { "pause" } else { "start" })), Some(sheet)),
        Domain::Camera => (Action { title: Some(e.name()), ..pressing(format!("camera:{id}")) }, None),
        Domain::Lock | Domain::Climate | Domain::WaterHeater | Domain::Alarm => opens(),
        Domain::Person | Domain::Weather | Domain::BinarySensor | Domain::Sensor => (Action::default(), None),
    }
}

impl Home {
    /// A thing's tile, titled as given: its glyph, whether it is lit and in what colour, how it
    /// stands, and what a press does.
    fn tile(&self, t: &Thing, title: &str) -> Item {
        let (action, more) = press_of(&t.e);
        Item { on: (t.e.domain() != Domain::Scene).then(|| t.e.is_on()), tint: tint(&t.e), action, more, ..tile(format!("e:{}", t.e.id), title, Some(line(&t.e, self.fahrenheit)), glyph(&t.e)) }
    }

    fn items(&self, base: &str) -> Vec<Item> {
        let mut items = placed(HERE, self.here_row());
        items.extend(placed(ROOMS, self.rooms_row()));
        items.extend(placed(SCENES, self.scenes_row()));
        items.extend(placed(CAMERAS, self.cameras_row(base)));
        items.extend(placed(AROUND, self.around_row()));
        items
    }

    /// This TV's room: the room itself, its scenes, then what is in it. Before a room is
    /// chosen the row asks for one, and a home without rooms shows everything here.
    fn here_row(&self) -> Vec<Item> {
        match (self.mode, self.here) {
            (Mode::Rooms, Some(i)) => {
                let room = &self.rooms[i];
                let lit = self.lamps(self.within(Some(i))).iter().any(|t| t.e.is_on());
                let whole = Item { action: Action::sheet(format!("room:{}", room.id)), row_title: Some(room.name.clone()), on: Some(lit), ..tile(HERE, "Whole room", Some(self.summary(Some(i))), "home") };
                let scenes = self.scenes(Some(i)).take(SCENES_FIRST);
                let devices = self.devices(Some(i)).filter(|t| t.e.domain().tiled());
                std::iter::once(whole).chain(scenes.chain(devices).map(|t| self.tile(t, &t.name))).collect()
            }
            (Mode::Rooms, None) => vec![Item { action: Action::sheet(ROOMS), ..tile(HERE, "Choose this room", Some("See its lights here".into()), "home") }],
            (mode, _) => {
                let mut row: Vec<Item> = self.things.iter().filter(|t| t.e.domain().tiled() && !t.extra).map(|t| self.tile(t, &t.name)).collect();
                row.truncate(ROW_TILES - 1);
                let (title, why) = if mode == Mode::NoAdmin { ("No rooms shown", "Needs an admin token") } else { ("No rooms yet", "Areas show as rooms") };
                row.push(Item { row_title: Some("Home".into()), ..tile("norooms", title, Some(why.into()), "home") });
                row
            }
        }
    }

    /// The other rooms, each opening its sheet, and last whatever has no room.
    fn rooms_row(&self) -> Vec<Item> {
        if self.mode != Mode::Rooms {
            return Vec::new();
        }
        let mut row = Vec::new();
        for (i, room) in self.rooms.iter().enumerate() {
            let empty = self.devices(Some(i)).next().is_none() && self.scenes(Some(i)).next().is_none();
            if Some(i) == self.here || (empty && room.temperature.is_none()) {
                continue;
            }
            let lamps = self.lamps(self.within(Some(i)));
            let lit = (!lamps.is_empty()).then(|| lamps.iter().any(|t| t.e.is_on()));
            row.push(Item { action: Action::sheet(format!("room:{}", room.id)), on: lit, ..tile(format!("room:{}", room.id), &room.name, Some(self.summary(Some(i))), "home") });
        }
        let loose = self.devices(None).count();
        if loose > 0 {
            row.push(Item { action: Action::sheet("room:"), ..tile("room:", "Not in a room", Some(counted(loose, "device")), "home") });
        }
        if let (Some(first), Some(_)) = (row.first_mut(), self.here) {
            first.row_title = Some("Other rooms".into());
        }
        row
    }

    /// Scenes for the whole home, then other rooms' scenes under their room's name. This
    /// room's own are in its row.
    fn scenes_row(&self) -> Vec<Item> {
        let mut scenes: Vec<&Thing> = self.things.iter().filter(|t| matches!(t.e.domain(), Domain::Scene | Domain::Script) && (t.room.is_none() || t.room != self.here)).collect();
        scenes.sort_by_key(|t| (t.e.domain(), t.room.is_some(), t.room));
        scenes
            .into_iter()
            .map(|t| {
                let mut item = self.tile(t, &t.e.name());
                if let Some(room) = self.room_name(t).filter(|_| t.e.state != "on") {
                    item.subtitle = Some(room.to_string());
                }
                item
            })
            .collect()
    }

    /// A snapshot of each camera, this room's first. The address changes when the camera's
    /// token does, which is when the TV fetches the picture again.
    fn cameras_row(&self, base: &str) -> Vec<Item> {
        let mut cameras: Vec<&Thing> = self.of(Domain::Camera).collect();
        cameras.sort_by_key(|t| t.room.is_none() || t.room != self.here);
        cameras
            .into_iter()
            .take(CAMERA_TILES)
            .map(|t| {
                let (action, _) = press_of(&t.e);
                let words = if t.e.unavailable() { Some("Not responding".to_string()) } else { self.room_name(t).map(str::to_string) };
                Item { id: format!("e:{}", t.e.id), title: t.e.name(), subtitle: words, thumbnail: camera_picture(base, &t.e, true).filter(|_| !t.e.unavailable()), action, ..Default::default() }
            })
            .collect()
    }

    /// Where a thing is, for a line that has to say: its room, or its own name.
    fn whereabouts(&self, t: &Thing) -> String {
        self.room_name(t).map(str::to_string).unwrap_or_else(|| t.e.name())
    }

    /// How long a thing has stood as it does: "For 12 minutes".
    fn how_long(&self, t: &Thing) -> Option<String> {
        t.e.changed_ms.filter(|&since| since <= self.now_ms).map(|since| format!("For {}", span(self.now_ms - since)))
    }

    fn alerts(&self, classes: &[&str]) -> Vec<&Thing> {
        self.of(Domain::BinarySensor).filter(|t| classes.contains(&t.e.class()) && t.e.state == "on").collect()
    }

    /// Doors and windows with a sensor on them, and the garage doors and gates themselves.
    fn openings(&self) -> Vec<&Thing> {
        self.things.iter().filter(|t| (t.e.domain() == Domain::BinarySensor && OPENINGS.contains(&t.e.class())) || (t.e.domain() == Domain::Cover && t.e.garage_like())).filter(|t| !t.e.unavailable()).collect()
    }

    /// The batteries that report a level, lowest first. A phone's is its owner's to mind.
    fn batteries(&self) -> Vec<(&Thing, f64)> {
        let mut levels: Vec<(&Thing, f64)> = self.of(Domain::Sensor).filter(|t| t.e.class() == "battery" && !t.phone).filter_map(|t| Some((t, t.e.number()?))).collect();
        levels.sort_by(|a, b| a.1.total_cmp(&b.1));
        levels
    }

    /// The forecast named after the home when there is one, since that is the one for where it stands.
    fn weather(&self) -> Option<&Thing> {
        let named = self.of(Domain::Weather).find(|t| self.home_name.as_ref().is_some_and(|home| t.e.a.friendly_name.as_ref() == Some(home)));
        named.or_else(|| self.of(Domain::Weather).next()).filter(|t| t.e.known())
    }

    /// How things stand around the home, in an order that never changes: what is wrong first,
    /// then what a person checks before bed, then who is in and what it is like outside.
    fn around_row(&self) -> Vec<Item> {
        let mut row = Vec::new();
        let places = |things: &[&Thing]| listed(&once_each(things.iter().map(|t| self.whereabouts(t))));
        let smoke = self.alerts(&SMOKE);
        if let Some(first) = smoke.first() {
            let what = match first.e.class() {
                "gas" => "Gas detected",
                "carbon_monoxide" => "CO detected",
                _ => "Smoke detected",
            };
            row.push(Item { on: Some(true), ..tile("smoke", what, Some(places(&smoke)), "smoke") });
        }
        let leaks = self.alerts(&["moisture"]);
        if !leaks.is_empty() {
            row.push(Item { on: Some(true), ..tile("leak", "Water leak", Some(places(&leaks)), "drop") });
        }
        row.extend(self.of(Domain::Alarm).map(|t| self.tile(t, &t.e.name())));

        let locks: Vec<&Thing> = self.of(Domain::Lock).filter(|t| t.e.known()).collect();
        if !locks.is_empty() {
            let undone: Vec<&Thing> = locks.iter().copied().filter(|t| t.e.state != "locked").collect();
            let (title, words) = match undone[..] {
                [] => ("All locked".to_string(), Some(counted(locks.len(), "lock"))),
                [one] => (format!("{} {}", one.e.name(), uncapitalised(&line(&one.e, false))), self.how_long(one)),
                _ => (format!("{} unlocked", counted(undone.len(), "lock")), Some(listed(&undone.iter().map(|t| t.e.name()).collect::<Vec<_>>()))),
            };
            row.push(Item { on: Some(!undone.is_empty()), action: Action::sheet("locks"), ..tile("locks", title, words, "lock") });
        }

        let openings = self.openings();
        if !openings.is_empty() {
            let open: Vec<&Thing> = openings.iter().copied().filter(|t| t.e.is_on()).collect();
            let (title, words) = match open[..] {
                [] => ("All closed".to_string(), Some(if openings.len() == 1 { "1 door or window".to_string() } else { format!("{} doors and windows", openings.len()) })),
                [one] => (format!("{} open", one.e.name()), self.how_long(one)),
                _ => (format!("{} left open", open.len()), Some(listed(&open.iter().map(|t| t.e.name()).collect::<Vec<_>>()))),
            };
            row.push(Item { on: Some(!open.is_empty()), action: Action::sheet("open"), ..tile("openings", title, words, "door") });
        }

        let low: Vec<(&Thing, f64)> = self.batteries().into_iter().filter(|(_, level)| *level < LOW_BATTERY).collect();
        if let Some((first, level)) = low.first() {
            let (title, words) = match low.len() {
                1 => ("Battery low".to_string(), format!("{}, {}", battery_name(&first.e), percent(*level))),
                count => (format!("{count} batteries low"), listed(&low.iter().map(|(t, _)| battery_name(&t.e)).collect::<Vec<_>>())),
            };
            row.push(Item { on: Some(true), action: Action::sheet("batteries"), ..tile("batteries", title, Some(words), "battery") });
        }

        let people: Vec<&Thing> = self.of(Domain::Person).filter(|t| t.e.known()).collect();
        if !people.is_empty() {
            let names = |home: bool| people.iter().filter(|t| t.e.is_on() == home).map(|t| t.e.name()).collect::<Vec<_>>();
            let (home, away) = (names(true), names(false));
            let (title, words) = match home.len() {
                0 => ("Nobody home".to_string(), None),
                1 | 2 => (format!("{} home", listed(&home)), (!away.is_empty()).then(|| if away.len() > 2 { format!("{} away", away.len()) } else { format!("{} away", listed(&away)) })),
                count => (format!("{count} people home"), Some(listed(&home))),
            };
            row.push(Item { on: Some(!home.is_empty()), action: Action::sheet("people"), ..tile("people", title, words, "person") });
        }

        if let Some(weather) = self.weather()
            && let Some(outside) = weather.e.a.temperature
        {
            let scale = if weather.e.a.temperature_unit.as_deref().map_or(self.fahrenheit, |unit| unit.contains('F')) { "F" } else { "C" };
            row.push(tile("weather", format!("{}{scale} outside", degrees(outside.round(), false)), sky(&weather.e.state).map(str::to_string), "sun"));
        }

        let lamps = self.lamps(self.things.iter());
        if !lamps.is_empty() {
            let lit: Vec<&Thing> = lamps.iter().copied().filter(|t| t.e.is_on()).collect();
            let mut rooms: Vec<usize> = lit.iter().filter_map(|t| t.room).collect();
            rooms.sort_unstable();
            rooms.dedup();
            let rooms: Vec<String> = rooms.into_iter().map(|i| self.rooms[i].name.clone()).collect();
            let (title, words) = match lit.len() {
                0 => ("All lights off".to_string(), None),
                count => (format!("{} on", counted(count, "light")), Some(if rooms.is_empty() { listed(&lit.iter().take(3).map(|t| t.e.name()).collect::<Vec<_>>()) } else { rooms.join(", ") })),
            };
            let action = if lit.is_empty() { Action::default() } else { Action::sheet("lights") };
            row.push(Item { on: Some(!lit.is_empty()), action, ..tile("lights", title, words, "light") });
        }
        row
    }
}

/// What a battery is in, from its sensor's name: "Hall Sensor Battery" is the hall sensor's.
fn battery_name(e: &Entity) -> String {
    let name = e.name();
    let without = |tail: &str| {
        let cut = name.len().checked_sub(tail.len()).filter(|&cut| name.is_char_boundary(cut))?;
        name[cut..].eq_ignore_ascii_case(tail).then(|| name[..cut].trim_end()).filter(|rest| !rest.is_empty())
    };
    without("battery level").or_else(|| without("battery")).map(str::to_string).unwrap_or(name)
}

// ---------------------------------------------------------------------------
// Sheets
// ---------------------------------------------------------------------------

const WARMTHS: [(&str, f64); 5] = [("Candlelight", 2200.0), ("Warm", 2700.0), ("Neutral", 4000.0), ("Cool", 5000.0), ("Daylight", 6500.0)];
/// Colours around the wheel, each with its hue in degrees.
const COLOURS: [(&str, &str, f64); 9] = [
    ("Red", "FF0000", 0.0),
    ("Orange", "FF8000", 30.0),
    ("Yellow", "FFFF00", 60.0),
    ("Green", "00FF00", 120.0),
    ("Turquoise", "00FFD5", 170.0),
    ("Sky blue", "00AAFF", 200.0),
    ("Blue", "0000FF", 240.0),
    ("Purple", "8000FF", 270.0),
    ("Pink", "FF00AA", 320.0),
];

/// The hue of a colour, in degrees around the wheel, or nothing for a white or a grey.
fn hue([r, g, b]: [u8; 3]) -> Option<f64> {
    let (r, g, b) = (f64::from(r), f64::from(g), f64::from(b));
    let (max, min) = (r.max(g).max(b), r.min(g).min(b));
    if max - min < 8.0 {
        return None;
    }
    let sixth = if max == r { ((g - b) / (max - min)).rem_euclid(6.0) } else if max == g { (b - r) / (max - min) + 2.0 } else { (r - g) / (max - min) + 4.0 };
    Some(sixth * 60.0)
}

/// Which of `count` choices `at` is nearest to, given how far each is from it.
fn nearest(count: usize, far: impl Fn(usize) -> f64) -> usize {
    (0..count).min_by(|&a, &b| far(a).total_cmp(&far(b))).unwrap_or(0)
}

/// A row that steps through `choices`, left to the one before `current` and right to the one
/// after, coming round at the ends.
fn cycle(id: &str, title: &str, entity: &str, what: &str, choices: &[String], current: Option<&str>) -> Option<SheetRow> {
    if choices.is_empty() {
        return None;
    }
    let at = current.and_then(|c| choices.iter().position(|choice| choice == c));
    let count = choices.len();
    let (down, up) = match at {
        Some(at) => (&choices[(at + count - 1) % count], &choices[(at + 1) % count]),
        None => (&choices[count - 1], &choices[0]),
    };
    let row = SheetRow::step(id, title, format!("set:{entity}:{what}:{down}"), format!("set:{entity}:{what}:{up}"));
    Some(match current {
        Some(current) => row.value(worded(current)),
        None => row,
    })
}

fn on_off(on: bool) -> &'static str {
    if on { "On" } else { "Off" }
}

/// `row` with `words` at its end, when there are any.
fn valued(row: SheetRow, words: &str) -> SheetRow {
    if words.is_empty() { row } else { row.value(words) }
}

impl Home {
    fn sheet(&self, id: &str) -> Option<Sheet> {
        let mut sheet = match id {
            ROOMS => self.rooms_sheet(),
            "lights" => self.lights_sheet(),
            "open" => self.openings_sheet(),
            "locks" => self.locks_sheet(),
            "people" => self.people_sheet(),
            "batteries" => self.batteries_sheet(),
            _ => {
                let (kind, rest) = id.split_once(':')?;
                match kind {
                    "room" => self.room_sheet(area_id(rest)?)?,
                    "dev" => self.device_sheet(self.find(entity_id(rest)?)?),
                    "src" => self.sources_sheet(self.find(entity_id(rest)?)?),
                    _ => return None,
                }
            }
        };
        sheet.rows.truncate(SHEET_ROWS);
        Some(sheet)
    }

    /// One thing as a row in a list: a press for what its tile does, a step where it has a
    /// level, and its own sheet where it has more than a row can hold.
    fn row(&self, t: &Thing, title: &str) -> SheetRow {
        let e = &t.e;
        let id = e.id.as_str();
        let words = line(e, self.fahrenheit);
        let lit = e.is_on();
        let row = match e.domain() {
            Domain::Scene => return SheetRow::press(id, title, format!("scene:{id}")).glyph(glyph(e)),
            Domain::Script => valued(SheetRow::press(id, title, format!("script:{id}")), if lit { "Running" } else { "" }),
            Domain::Light if e.dimmable() && !e.unavailable() => {
                let shown = e.a.brightness.filter(|_| lit).map_or("Off".to_string(), level);
                SheetRow::step(id, title, format!("step:{id}:brightness:-10"), format!("step:{id}:brightness:+10")).pressing(format!("toggle:{id}")).value(shown)
            }
            Domain::Fan if e.a.percentage_step.is_some() && !e.unavailable() => {
                let speed = e.a.percentage.filter(|_| lit).map_or(on_off(lit).to_string(), percent);
                SheetRow::step(id, title, format!("step:{id}:speed:-1"), format!("step:{id}:speed:+1")).pressing(format!("toggle:{id}")).value(speed)
            }
            Domain::Light | Domain::Switch | Domain::Toggle | Domain::Fan => SheetRow::press(id, title, format!("toggle:{id}")).value(words),
            Domain::Cover if e.garage_like() => SheetRow::open(id, title, device_sheet(e)).value(words),
            Domain::Cover if e.a.current_position.is_some() && !e.unavailable() => SheetRow::step(id, title, format!("step:{id}:position:-10"), format!("step:{id}:position:+10")).pressing(format!("cover:{id}:toggle")).value(words),
            Domain::Cover => SheetRow::press(id, title, format!("cover:{id}:toggle")).value(words),
            Domain::Lock if e.needs_code() => SheetRow::text(id, title).value(words).subtitle("Needs its code, so use it in Home Assistant"),
            Domain::Lock if e.state == "locked" => SheetRow::confirm(id, title, format!("Yes, unlock {}", brief(&e.name())), format!("unlock:{id}")).value(words),
            Domain::Lock => SheetRow::press(id, title, format!("lock:{id}")).value(words),
            Domain::Camera => valued(SheetRow::press(id, title, format!("camera:{id}")), if e.unavailable() { &words } else { "" }),
            Domain::Media => {
                let state = words.split(", ").next().unwrap_or_default().to_string();
                let row = SheetRow::open(id, title, device_sheet(e)).value(state);
                match e.a.media_title.as_deref().filter(|_| matches!(e.state.as_str(), "playing" | "paused")) {
                    Some(playing) => row.subtitle(playing),
                    None => row,
                }
            }
            Domain::Person | Domain::Weather | Domain::BinarySensor | Domain::Sensor => SheetRow::text(id, title).value(words),
            _ => SheetRow::open(id, title, device_sheet(e)).value(words),
        };
        row.glyph(glyph(e)).on(lit)
    }

    /// The rooms to choose this TV's from, one press each.
    fn rooms_sheet(&self) -> Sheet {
        let mut rows: Vec<SheetRow> = self
            .rooms
            .iter()
            .enumerate()
            .map(|(i, room)| {
                let chosen = Some(i) == self.here;
                valued(SheetRow::press(format!("room:{}", room.id), &room.name, format!("here:{}", room.id)), if chosen { "This TV" } else { "" }).glyph("home").on(chosen)
            })
            .collect();
        rows.truncate(SHEET_ROWS - 1);
        if self.here.is_some() {
            rows.push(SheetRow::press("none", "None of these", "here:"));
        }
        let footnote = if self.rooms.is_empty() { "Home Assistant has no areas yet. Add some there and they show here as rooms." } else { "The first row of the home shows this room's lights and scenes." };
        Sheet { title: "This TV's room".into(), subtitle: Some("Choose the room this TV is in".into()), footnote: Some(footnote.into()), rows }
    }

    /// Everything in a room, kind by kind, with what its row cannot fit. No room at all lists
    /// what has none.
    fn room_sheet(&self, area: &str) -> Option<Sheet> {
        let room = if area.is_empty() { None } else { Some(self.rooms.iter().position(|room| room.id == area)?) };
        let mut rows = Vec::new();
        let lamps = self.lamps(self.within(room));
        if !lamps.is_empty() && room.is_some() {
            let on = lamps.iter().filter(|t| t.e.is_on()).count();
            let (does, value) = if on > 0 { ("all_off", format!("{on} on")) } else { ("all_on", "Off".to_string()) };
            rows.push(SheetRow::press("all", "All lights", format!("{does}:{area}")).glyph("light").on(on > 0).value(value));
        }
        let mut heading = "";
        let scenes = self.scenes(room).filter(|_| room.is_some());
        let more = self.within(room).filter(|t| t.extra);
        for (t, under) in scenes.chain(self.devices(room)).map(|t| (t, t.e.domain().heading())).chain(more.map(|t| (t, "More"))) {
            if under != heading {
                heading = under;
                rows.push(SheetRow::group(format!("g:{}", under.to_lowercase()), under));
            }
            rows.push(self.row(t, &t.name));
        }
        if let Some(i) = room {
            let here = Some(i) == self.here;
            rows.truncate(SHEET_ROWS - 2);
            rows.push(SheetRow::group("g:tv", "This TV"));
            rows.push(SheetRow::press("here", "This TV is in this room", format!("here:{}", if here { "" } else { area })).glyph("tv").on(here).value(on_off(here)));
        }
        let (title, footnote) = match room {
            Some(i) => (self.rooms[i].name.clone(), None),
            None => ("Not in a room".to_string(), Some("Give these an area in Home Assistant and they move to its room.".to_string())),
        };
        Some(Sheet { title, subtitle: Some(self.summary(room)), footnote, rows })
    }

    /// The lights that are on, room by room, with one row to turn them all off.
    fn lights_sheet(&self) -> Sheet {
        let lit: Vec<&Thing> = self.lamps(self.things.iter()).into_iter().filter(|t| t.e.is_on()).collect();
        let mut rooms: Vec<Option<usize>> = lit.iter().map(|t| t.room).collect();
        rooms.sort_by_key(|room| (room.is_none(), *room));
        rooms.dedup();
        let mut rows = Vec::new();
        if !lit.is_empty() {
            rows.push(SheetRow::confirm("all", "All lights off", format!("Yes, turn off {}", counted(lit.len(), "light")), "all_off:").glyph("light"));
        }
        for room in &rooms {
            let name = room.map_or("Not in a room", |i| self.rooms[i].name.as_str());
            rows.push(SheetRow::group(format!("g:{}", room.map_or("", |i| self.rooms[i].id.as_str())), name));
            rows.extend(lit.iter().filter(|t| t.room == *room).map(|t| self.row(t, &t.name)));
        }
        let placed = rooms.iter().flatten().count();
        let subtitle = match (lit.len(), placed) {
            (0, _) => "All lights are off".to_string(),
            (count, 0) => counted(count, "light"),
            (count, rooms) => format!("{} in {}", counted(count, "light"), counted(rooms, "room")),
        };
        Sheet { title: "Lights on".into(), subtitle: Some(subtitle), footnote: None, rows }
    }

    /// Doors and windows, the open ones first with how long they have been.
    fn openings_sheet(&self) -> Sheet {
        let mut openings = self.openings();
        openings.sort_by_key(|t| !t.e.is_on());
        let rows = openings
            .iter()
            .map(|t| {
                let open = t.e.is_on();
                let row = SheetRow::text(&t.e.id, t.e.name()).glyph(glyph(&t.e)).on(open).value(if open { "Open" } else { "Closed" });
                match self.how_long(t).filter(|_| open) {
                    Some(long) => row.subtitle(long),
                    None => row,
                }
            })
            .collect();
        Sheet { title: "Doors and windows".into(), subtitle: None, footnote: None, rows }
    }

    fn locks_sheet(&self) -> Sheet {
        Sheet { title: "Locks".into(), subtitle: None, footnote: None, rows: self.of(Domain::Lock).map(|t| self.row(t, &t.e.name())).collect() }
    }

    fn people_sheet(&self) -> Sheet {
        Sheet { title: "Who is home".into(), subtitle: None, footnote: None, rows: self.of(Domain::Person).filter(|t| t.e.known()).map(|t| self.row(t, &t.e.name())).collect() }
    }

    fn batteries_sheet(&self) -> Sheet {
        let rows = self.batteries().into_iter().map(|(t, level)| SheetRow::text(&t.e.id, battery_name(&t.e)).glyph("battery").on(level < LOW_BATTERY).value(percent(level))).collect();
        Sheet { title: "Batteries".into(), subtitle: None, footnote: None, rows }
    }

    /// A speaker's or a TV's sources, for a list too long to step through.
    fn sources_sheet(&self, t: &Thing) -> Sheet {
        let e = &t.e;
        let sources = e.a.source_list.as_deref().unwrap_or(&[]);
        let rows = sources.iter().enumerate().map(|(n, source)| SheetRow::press(format!("s{n}"), source, format!("set:{}:source:{source}", e.id)).on(e.a.source.as_ref() == Some(source))).collect();
        Sheet { title: "Source".into(), subtitle: Some(e.name()), footnote: None, rows }
    }

    /// All that can be done with one thing. A row is there only for what the thing can do.
    fn device_sheet(&self, t: &Thing) -> Sheet {
        let e = &t.e;
        let a = &e.a;
        let id = e.id.as_str();
        let name = e.name();
        let lit = e.is_on();
        let state = e.state.as_str();
        let press = |row: &str, title: &str, action: String| SheetRow::press(row, title, action);
        let mut rows: Vec<SheetRow> = Vec::new();
        let mut footnote = None;
        if e.unavailable() {
            rows.push(SheetRow::text("state", "Not responding").subtitle("Check its power and connection"));
        } else {
            match e.domain() {
                Domain::Light => {
                    rows.push(if e.dimmable() { SheetRow { id: "brightness".into(), title: "Brightness".into(), ..self.row(t, "") } } else { press("power", "Power", format!("toggle:{id}")).value(on_off(lit)).on(lit) });
                    if e.takes_warmth() {
                        let (low, high) = (a.min_color_temp_kelvin.unwrap_or(2000.0), a.max_color_temp_kelvin.unwrap_or(6500.0));
                        let reach = |n: usize| WARMTHS[n].1.clamp(low, high).round() as i64;
                        let at = a.color_temp_kelvin.map(|kelvin| nearest(WARMTHS.len(), |n| (WARMTHS[n].1 - kelvin).abs()));
                        let (down, up) = at.map_or((1, 3), |at| (at.saturating_sub(1), (at + 1).min(WARMTHS.len() - 1)));
                        let row = SheetRow::step("warmth", "Warmth", format!("set:{id}:kelvin:{}", reach(down)), format!("set:{id}:kelvin:{}", reach(up)));
                        rows.push(at.filter(|_| lit).map_or(row.clone(), |at| row.value(WARMTHS[at].0)));
                    }
                    if e.takes_colour() {
                        // A lamp set to a white says its warmth, and its colour only stands in for that.
                        let shown = a.rgb_color.filter(|_| lit && a.color_temp_kelvin.is_none()).and_then(hue);
                        let at = shown.map(|hue| nearest(COLOURS.len(), |n| ((COLOURS[n].2 - hue).abs()).min(360.0 - (COLOURS[n].2 - hue).abs())));
                        let count = COLOURS.len();
                        let (down, up) = at.map_or((count - 1, 0), |at| ((at + count - 1) % count, (at + 1) % count));
                        let row = SheetRow::step("colour", "Colour", format!("set:{id}:colour:{}", COLOURS[down].1), format!("set:{id}:colour:{}", COLOURS[up].1));
                        rows.push(at.map_or(row.clone(), |at| row.value(COLOURS[at].0)));
                    }
                    let members: Vec<&Thing> = a.entity_id.iter().flatten().filter_map(|member| self.find(member)).collect();
                    if !members.is_empty() {
                        rows.push(SheetRow::group("g:members", "In this group"));
                        rows.extend(members.into_iter().map(|member| self.row(member, &member.e.name())));
                    }
                }
                Domain::Fan => {
                    rows.push(if a.percentage_step.is_some() { SheetRow { id: "speed".into(), title: "Speed".into(), ..self.row(t, "") } } else { press("power", "Power", format!("toggle:{id}")).value(on_off(lit)).on(lit) });
                    if let Some(turning) = a.oscillating {
                        rows.push(press("oscillating", "Oscillate", format!("set:{id}:oscillating:{}", if turning { "off" } else { "on" })).value(on_off(turning)).on(turning));
                    }
                    rows.extend(cycle("preset", "Mode", id, "preset", a.preset_modes.as_deref().unwrap_or(&[]), a.preset_mode.as_deref()));
                }
                Domain::Cover if e.garage_like() => {
                    match state {
                        "opening" | "closing" => rows.push(press("stop", "Stop", format!("cover:{id}:stop")).value(worded(state))),
                        "closed" => rows.push(SheetRow::confirm("open", "Open", format!("Yes, open {}", brief(&name)), format!("cover:{id}:open"))),
                        _ => rows.push(SheetRow::confirm("close", "Close", format!("Yes, close {}", brief(&name)), format!("cover:{id}:close"))),
                    }
                    footnote = Some("Check that nothing is in its way first.".to_string());
                }
                Domain::Cover => {
                    if let Some(position) = a.current_position {
                        rows.push(SheetRow::step("position", "Position", format!("step:{id}:position:-10"), format!("step:{id}:position:+10")).pressing(format!("cover:{id}:toggle")).value(percent(position)).on(lit));
                    }
                    if e.supports(1) && e.supports(2) {
                        rows.push(press("open", "Open", format!("cover:{id}:open")));
                        rows.push(press("close", "Close", format!("cover:{id}:close")));
                    }
                    if e.supports(8) {
                        rows.push(press("stop", "Stop", format!("cover:{id}:stop")));
                    }
                    if let Some(tilt) = a.current_tilt_position {
                        rows.push(SheetRow::step("tilt", "Tilt", format!("step:{id}:tilt:-10"), format!("step:{id}:tilt:+10")).value(percent(tilt)));
                    }
                }
                Domain::Lock if e.needs_code() => rows.push(SheetRow::text("code", format!("Needs its code, so {} it in Home Assistant", if state == "locked" { "unlock" } else { "lock" }))),
                Domain::Lock => {
                    if state != "locked" {
                        rows.push(press("lock", "Lock", format!("lock:{id}")).glyph("lock"));
                    }
                    if matches!(state, "locked" | "jammed") {
                        rows.push(SheetRow::confirm("unlock", "Unlock", format!("Yes, unlock {}", brief(&name)), format!("unlock:{id}")).glyph("lock"));
                    }
                }
                Domain::Climate | Domain::WaterHeater => {
                    let step = a.target_temp_step.filter(|step| *step > 0.0).unwrap_or(if self.fahrenheit { 1.0 } else { 0.5 });
                    let (low, high) = (a.min_temp.unwrap_or(f64::MIN), a.max_temp.unwrap_or(f64::MAX));
                    let stepper = |row: &str, title: &str, what: &str, at: f64| {
                        let to = |target: f64| format!("set:{id}:{what}:{:.1}", target.clamp(low, high));
                        SheetRow::step(row, title, to(at - step), to(at + step)).value(degrees(at, self.fahrenheit))
                    };
                    if let Some(now) = a.current_temperature {
                        let now = degrees(now, self.fahrenheit);
                        rows.push(SheetRow::text("now", "Now").value(a.current_humidity.map_or(now.clone(), |humidity| format!("{now}, {}", percent(humidity)))));
                    }
                    match (a.target_temp_low, a.target_temp_high, a.temperature) {
                        (Some(heat), Some(cool), _) => {
                            rows.push(stepper("low", "Heat to", "low", heat));
                            rows.push(stepper("high", "Cool to", "high", cool));
                        }
                        (_, _, Some(target)) => rows.push(stepper("temperature", "Temperature", "temperature", target)),
                        _ => {}
                    }
                    if e.domain() == Domain::Climate {
                        rows.extend(cycle("mode", "Mode", id, "mode", a.hvac_modes.as_deref().unwrap_or(&[]), Some(state)));
                        rows.extend(cycle("preset", "Preset", id, "preset", a.preset_modes.as_deref().unwrap_or(&[]), a.preset_mode.as_deref()));
                        rows.extend(cycle("fan", "Fan", id, "fan", a.fan_modes.as_deref().unwrap_or(&[]), a.fan_mode.as_deref()));
                    } else {
                        rows.extend(cycle("mode", "Mode", id, "mode", a.operation_list.as_deref().unwrap_or(&[]), a.operation_mode.as_deref().or(Some(state))));
                        if let Some(away) = a.away_mode.as_deref().map(|mode| mode == "on") {
                            rows.push(press("away", "Away mode", format!("set:{id}:away:{}", if away { "off" } else { "on" })).value(on_off(away)).on(away));
                        }
                    }
                }
                Domain::Humidifier => {
                    let (low, high) = (a.min_humidity.unwrap_or(0.0), a.max_humidity.unwrap_or(100.0));
                    if let Some(now) = a.current_humidity {
                        rows.push(SheetRow::text("now", "Now").value(percent(now)));
                    }
                    let toggle = format!("toggle:{id}");
                    rows.push(match a.humidity {
                        Some(target) => {
                            let to = |humidity: f64| format!("set:{id}:humidity:{}", humidity.clamp(low, high).round() as i64);
                            SheetRow::step("humidity", "Humidity", to(target - 5.0), to(target + 5.0)).pressing(toggle).value(if lit { percent(target) } else { "Off".into() }).on(lit)
                        }
                        None => press("power", "Power", toggle).value(on_off(lit)).on(lit),
                    });
                    rows.extend(cycle("mode", "Mode", id, "mode", a.available_modes.as_deref().unwrap_or(&[]), a.mode.as_deref()));
                }
                Domain::Media => {
                    if matches!(state, "off" | "standby") {
                        rows.push(press("on", "Turn on", format!("media:{id}:on")));
                    } else {
                        if e.supports(1) || e.supports(16_384) {
                            rows.push(press("play", if lit { "Pause" } else { "Play" }, format!("media:{id}:play_pause")).value(worded(state)).on(lit));
                        }
                        if let Some(volume) = a.volume_level {
                            let muted = a.is_volume_muted == Some(true);
                            rows.push(SheetRow::step("volume", "Volume", format!("step:{id}:volume:-5"), format!("step:{id}:volume:+5")).pressing(format!("media:{id}:mute")).value(if muted { "Muted".into() } else { percent(volume * 100.0) }).on(!muted));
                        }
                        if a.supported_features.is_some_and(|features| features & 32 != 0) {
                            rows.push(press("next", "Next", format!("media:{id}:next")));
                        }
                        if a.supported_features.is_some_and(|features| features & 16 != 0) {
                            rows.push(press("previous", "Previous", format!("media:{id}:previous")));
                        }
                        let sources = a.source_list.as_deref().unwrap_or(&[]);
                        if sources.len() > 8 {
                            rows.push(valued(SheetRow::open("source", "Source", format!("src:{id}")), a.source.as_deref().unwrap_or("")));
                        } else if let Some(row) = cycle("source", "Source", id, "source", sources, a.source.as_deref()) {
                            rows.push(SheetRow { value: a.source.clone(), ..row });
                        }
                        if a.supported_features.is_some_and(|features| features & 256 != 0) {
                            rows.push(press("off", "Turn off", format!("media:{id}:off")));
                        }
                    }
                }
                Domain::Vacuum => {
                    let cleaning = state == "cleaning";
                    rows.push(press("start", if cleaning { "Pause" } else { "Start" }, format!("vacuum:{id}:{}", if cleaning { "pause" } else { "start" })).value(line(e, false)).on(lit));
                    if e.supports(16) {
                        rows.push(press("dock", "Send home", format!("vacuum:{id}:dock")));
                    }
                    if a.supported_features.is_some_and(|features| features & 512 != 0) {
                        rows.push(press("locate", "Find it", format!("vacuum:{id}:locate")));
                    }
                    rows.extend(cycle("power", "Power", id, "fan", a.fan_speed_list.as_deref().unwrap_or(&[]), a.fan_speed.as_deref()));
                }
                Domain::Alarm => {
                    let code = e.needs_code();
                    if state == "disarmed" {
                        if code && a.code_arm_required != Some(false) {
                            rows.push(SheetRow::text("code", "Needs its code, so arm it in Home Assistant"));
                        } else {
                            let ways = [("home", "Arm home", 1), ("away", "Arm away", 2), ("night", "Arm night", 4)];
                            rows.extend(ways.into_iter().filter(|(_, _, bit)| e.supports(*bit)).map(|(way, title, _)| press(way, title, format!("alarm:{id}:{way}"))));
                        }
                    } else if code {
                        rows.push(SheetRow::text("code", "Needs its code, so disarm it in Home Assistant"));
                    } else {
                        rows.push(SheetRow::confirm("disarm", "Disarm", format!("Yes, disarm {}", brief(&name)), format!("alarm:{id}:disarm")));
                    }
                }
                _ => rows.push(self.row(t, "State")),
            }
        }
        let words = line(e, self.fahrenheit);
        let subtitle = match self.room_name(t) {
            Some(room) => format!("{room}, {}", uncapitalised(&words)),
            None => words,
        };
        Sheet { title: name, subtitle: Some(subtitle), footnote, rows }
    }
}

// ---------------------------------------------------------------------------
// Presses
// ---------------------------------------------------------------------------

/// An entity id as Home Assistant writes one, and nothing that could reach another address.
fn entity_id(text: &str) -> Option<&str> {
    let plain = |part: &str| !part.is_empty() && part.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_');
    let (domain, object) = text.split_once('.')?;
    (plain(domain) && plain(object)).then_some(text)
}

/// An area id, or the empty one that stands for no room.
fn area_id(text: &str) -> Option<&str> {
    text.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_').then_some(text)
}

/// What a tile or a sheet row asks for, as its action spells it: a verb, the thing, and for
/// some what to change and to what.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Press<'a> {
    Toggle(&'a str),
    Scene(&'a str),
    Script(&'a str),
    Lock(&'a str),
    Unlock(&'a str),
    Camera(&'a str),
    Step { entity: &'a str, what: &'a str, by: f64 },
    Set { entity: &'a str, what: &'a str, value: &'a str },
    Cover { entity: &'a str, verb: &'a str },
    Media { entity: &'a str, verb: &'a str },
    Vacuum { entity: &'a str, verb: &'a str },
    Alarm { entity: &'a str, verb: &'a str },
    AllOff(&'a str),
    AllOn(&'a str),
    Here(&'a str),
}

impl<'a> Press<'a> {
    fn parse(action: &'a str) -> Option<Press<'a>> {
        let (verb, rest) = action.split_once(':')?;
        let of = |text: &'a str, kinds: &[Domain]| entity_id(text).filter(|e| Domain::of(e).is_some_and(|d| kinds.contains(&d)));
        let one = |kinds: &[Domain]| of(rest, kinds);
        // The thing and one of the verbs its kind knows.
        let two = |kind: Domain, verbs: &[&'static str]| {
            let (entity, verb) = rest.split_once(':')?;
            Some((of(entity, &[kind])?, *verbs.iter().find(|known| **known == verb)?))
        };
        // The thing, what to change, and the rest as the value, colons and all.
        let three = || {
            let mut parts = rest.splitn(3, ':');
            let entity = entity_id(parts.next()?).filter(|e| Domain::of(e).is_some())?;
            Some((entity, parts.next().filter(|what| !what.is_empty())?, parts.next().filter(|value| !value.is_empty())?))
        };
        Some(match verb {
            "toggle" => Press::Toggle(one(&[Domain::Light, Domain::Switch, Domain::Toggle, Domain::Fan, Domain::Humidifier, Domain::Lock])?),
            "scene" => Press::Scene(one(&[Domain::Scene])?),
            "script" => Press::Script(one(&[Domain::Script])?),
            "lock" => Press::Lock(one(&[Domain::Lock])?),
            "unlock" => Press::Unlock(one(&[Domain::Lock])?),
            "camera" => Press::Camera(one(&[Domain::Camera])?),
            "step" => {
                let (entity, what, by) = three()?;
                Press::Step { entity, what, by: by.parse::<f64>().ok().filter(|by| by.is_finite() && by.abs() <= 100.0)? }
            }
            "set" => {
                let (entity, what, value) = three()?;
                Press::Set { entity, what, value }
            }
            "cover" => {
                let (entity, verb) = two(Domain::Cover, &["open", "close", "stop", "toggle"])?;
                Press::Cover { entity, verb }
            }
            "media" => {
                let (entity, verb) = two(Domain::Media, &["play_pause", "next", "previous", "mute", "on", "off"])?;
                Press::Media { entity, verb }
            }
            "vacuum" => {
                let (entity, verb) = two(Domain::Vacuum, &["start", "pause", "dock", "locate"])?;
                Press::Vacuum { entity, verb }
            }
            "alarm" => {
                let (entity, verb) = two(Domain::Alarm, &["home", "away", "night", "disarm"])?;
                Press::Alarm { entity, verb }
            }
            "all_off" => Press::AllOff(area_id(rest)?),
            "all_on" => Press::AllOn(area_id(rest).filter(|area| !area.is_empty())?),
            "here" => Press::Here(area_id(rest)?),
            _ => return None,
        })
    }

    /// The one thing the press is about, when it is about one.
    fn entity(&self) -> Option<&'a str> {
        match *self {
            Press::Toggle(e) | Press::Scene(e) | Press::Script(e) | Press::Lock(e) | Press::Unlock(e) | Press::Camera(e) => Some(e),
            Press::Step { entity, .. } | Press::Set { entity, .. } | Press::Cover { entity, .. } | Press::Media { entity, .. } | Press::Vacuum { entity, .. } | Press::Alarm { entity, .. } => Some(entity),
            Press::AllOff(_) | Press::AllOn(_) | Press::Here(_) => None,
        }
    }

    /// Whether the thing is read before the call. A call on something gone or unreachable
    /// succeeds and does nothing, so only a read can tell. Dimming leaves the read out, to keep
    /// up with a held key.
    fn reads_first(&self) -> bool {
        !matches!(self, Press::Step { what: "brightness", .. })
    }
}

/// One call to a Home Assistant service, and what to say once it has worked.
#[derive(Clone, Debug, PartialEq)]
struct Call {
    /// The service as its address ends: "light/turn_on".
    service: String,
    data: Value,
    say: Option<String>,
}

impl Call {
    fn new(service: &str, data: Value) -> Call {
        Call { service: service.into(), data, say: None }
    }

    fn saying(mut self, words: String) -> Call {
        self.say = Some(words);
        self
    }
}

/// The call a press makes, given the thing as it stands now. Nothing comes back for a press
/// that does not suit the thing.
fn plan(press: &Press, fresh: Option<&Entity>) -> Option<Call> {
    let on = |entity: &str, service: &str| Call::new(service, json!({"entity_id": entity}));
    let with = |entity: &str, service: &str, key: &str, value: Value| Call::new(service, json!({"entity_id": entity, key: value}));
    if let Press::Step { entity, what: "brightness", by } = *press {
        return (Domain::of(entity) == Some(Domain::Light)).then(|| with(entity, "light/turn_on", "brightness_step_pct", json!(by.round() as i64)));
    }
    let e = fresh?;
    let a = &e.a;
    let name = e.name();
    let state = e.state.as_str();
    let said = |call: Call, words: &str| call.saying(format!("{name} {words}"));
    let flag = |value: &str| match value {
        "on" => Some(true),
        "off" => Some(false),
        _ => None,
    };
    Some(match *press {
        // A lock is only ever locked by a press that could go either way.
        Press::Toggle(id) if e.domain() == Domain::Lock => said(on(id, "lock/lock"), "locked"),
        Press::Toggle(id) => {
            let domain = id.split_once('.')?.0;
            if state == "on" { said(on(id, &format!("{domain}/turn_off")), "off") } else { said(on(id, &format!("{domain}/turn_on")), "on") }
        }
        Press::Scene(id) => said(on(id, "scene/turn_on"), "set"),
        // Calling a script by its own name waits for it to end, and turning it on does not.
        Press::Script(id) => said(on(id, "script/turn_on"), "started"),
        Press::Lock(id) => said(on(id, "lock/lock"), "locked"),
        Press::Unlock(id) => said(on(id, "lock/unlock"), "unlocked"),
        Press::Step { entity, what, by } => match (e.domain(), what) {
            (Domain::Media, "volume") => with(entity, "media_player/volume_set", "volume_level", json!(((a.volume_level? * 100.0 + by).clamp(0.0, 100.0)).round() / 100.0)),
            (Domain::Cover, "position") => with(entity, "cover/set_cover_position", "position", json!((a.current_position? + by).clamp(0.0, 100.0).round() as i64)),
            (Domain::Cover, "tilt") => with(entity, "cover/set_cover_tilt_position", "tilt_position", json!((a.current_tilt_position? + by).clamp(0.0, 100.0).round() as i64)),
            (Domain::Fan, "speed") => {
                let notch = a.percentage_step.filter(|step| *step > 0.0)?;
                let now = if state == "on" { a.percentage.unwrap_or(0.0) } else { 0.0 };
                let notches = (100.0 / notch).round();
                with(entity, "fan/set_percentage", "percentage", json!((((now / notch).round() + by.signum()).clamp(0.0, notches) * notch).min(100.0).round() as i64))
            }
            _ => return None,
        },
        Press::Set { entity, what, value } => {
            let number = || value.parse::<f64>().ok().filter(|n| n.is_finite());
            match (e.domain(), what) {
                (Domain::Climate, "temperature") => with(entity, "climate/set_temperature", "temperature", json!(number()?)),
                (Domain::WaterHeater, "temperature") => with(entity, "water_heater/set_temperature", "temperature", json!(number()?)),
                (Domain::Climate, "low") => Call::new("climate/set_temperature", json!({"entity_id": entity, "target_temp_low": number()?, "target_temp_high": a.target_temp_high?})),
                (Domain::Climate, "high") => Call::new("climate/set_temperature", json!({"entity_id": entity, "target_temp_low": a.target_temp_low?, "target_temp_high": number()?})),
                (Domain::Climate, "mode") => with(entity, "climate/set_hvac_mode", "hvac_mode", json!(value)),
                (Domain::Humidifier, "mode") => with(entity, "humidifier/set_mode", "mode", json!(value)),
                (Domain::WaterHeater, "mode") => with(entity, "water_heater/set_operation_mode", "operation_mode", json!(value)),
                (Domain::Climate, "preset") => with(entity, "climate/set_preset_mode", "preset_mode", json!(value)),
                (Domain::Fan, "preset") => with(entity, "fan/set_preset_mode", "preset_mode", json!(value)),
                (Domain::Climate, "fan") => with(entity, "climate/set_fan_mode", "fan_mode", json!(value)),
                (Domain::Vacuum, "fan") => with(entity, "vacuum/set_fan_speed", "fan_speed", json!(value)),
                (Domain::Light, "kelvin") => with(entity, "light/turn_on", "color_temp_kelvin", json!(number().filter(|k| (1000.0..=12000.0).contains(k))?.round() as i64)),
                (Domain::Light, "colour") => {
                    let rgb = u32::from_str_radix(value, 16).ok().filter(|_| value.len() == 6)?;
                    with(entity, "light/turn_on", "rgb_color", json!([rgb >> 16, (rgb >> 8) & 0xff, rgb & 0xff]))
                }
                (Domain::Media, "source") => with(entity, "media_player/select_source", "source", json!(value)),
                (Domain::Humidifier, "humidity") => with(entity, "humidifier/set_humidity", "humidity", json!(number().filter(|h| (0.0..=100.0).contains(h))?.round() as i64)),
                (Domain::Fan, "oscillating") => with(entity, "fan/oscillate", "oscillating", json!(flag(value)?)),
                (Domain::WaterHeater, "away") => with(entity, "water_heater/set_away_mode", "away_mode", json!(flag(value)?)),
                _ => return None,
            }
        }
        Press::Cover { entity, verb } => match (verb, state) {
            ("stop", _) | ("toggle", "opening" | "closing") => said(on(entity, "cover/stop_cover"), "stopped"),
            ("open", _) | ("toggle", "closed") => said(on(entity, "cover/open_cover"), "opening"),
            _ => said(on(entity, "cover/close_cover"), "closing"),
        },
        Press::Media { entity, verb } => match (verb, state) {
            ("on", _) | ("play_pause", "off" | "standby") => said(on(entity, "media_player/turn_on"), "on"),
            ("off", _) => said(on(entity, "media_player/turn_off"), "off"),
            ("play_pause", "playing" | "buffering") => said(on(entity, "media_player/media_pause"), "paused"),
            ("play_pause", _) => said(on(entity, "media_player/media_play"), "playing"),
            ("next", _) => on(entity, "media_player/media_next_track"),
            ("previous", _) => on(entity, "media_player/media_previous_track"),
            _ => with(entity, "media_player/volume_mute", "is_volume_muted", json!(a.is_volume_muted != Some(true))),
        },
        Press::Vacuum { entity, verb } => match verb {
            "start" => said(on(entity, "vacuum/start"), "cleaning"),
            "pause" => said(on(entity, "vacuum/pause"), "paused"),
            "dock" => said(on(entity, "vacuum/return_to_base"), "going home"),
            _ => on(entity, "vacuum/locate"),
        },
        Press::Alarm { entity, verb } => match verb {
            "home" => said(on(entity, "alarm_control_panel/alarm_arm_home"), "armed"),
            "away" => said(on(entity, "alarm_control_panel/alarm_arm_away"), "armed"),
            "night" => said(on(entity, "alarm_control_panel/alarm_arm_night"), "armed"),
            _ => said(on(entity, "alarm_control_panel/alarm_disarm"), "disarmed"),
        },
        Press::Camera(_) | Press::AllOff(_) | Press::AllOn(_) | Press::Here(_) => return None,
    })
}

impl Home {
    /// Every light of a room, or with no room every light of the home, turned on or off in one
    /// call that names each of them.
    fn all_lights(&self, area: &str, on: bool) -> Option<Call> {
        let room = if area.is_empty() { None } else { Some(self.rooms.iter().position(|room| room.id == area)?) };
        let lights: Vec<&Thing> = self.of(Domain::Light).filter(|t| !t.e.unavailable() && (room.is_none() || t.room == room)).collect();
        if lights.is_empty() {
            return None;
        }
        let ids: Vec<&str> = lights.iter().map(|t| t.e.id.as_str()).collect();
        let say = match room {
            Some(i) => format!("{} lights {}", self.rooms[i].name, if on { "on" } else { "off" }),
            None => match self.lamps(self.things.iter()).iter().filter(|t| t.e.is_on()).count() {
                0 => "All lights off".to_string(),
                lit => format!("{} off", counted(lit, "light")),
            },
        };
        Some(Call::new(if on { "light/turn_on" } else { "light/turn_off" }, json!({"entity_id": ids})).saying(say))
    }
}

// ---------------------------------------------------------------------------
// Search
// ---------------------------------------------------------------------------

/// Whether `query` starts a word of `name`, whatever the case.
fn matches_words(name: &str, query: &str) -> bool {
    let name = name.to_lowercase();
    name.match_indices(query).any(|(at, _)| !name[..at].chars().next_back().is_some_and(char::is_alphanumeric))
}

impl Home {
    /// Rooms first, then scenes, then devices. A result does what its tile does, so nothing
    /// that lets someone in moves on one press. `fresh` says whether the states are recent
    /// enough to show.
    fn search(&self, query: &str, limit: usize, fresh: bool) -> Vec<Item> {
        let query = query.trim().to_lowercase();
        if query.is_empty() {
            return Vec::new();
        }
        let mut found: Vec<Item> = self
            .rooms
            .iter()
            .enumerate()
            .filter(|(_, room)| matches_words(&room.name, &query))
            .map(|(i, room)| Item { action: Action::sheet(format!("room:{}", room.id)), ..tile(format!("room:{}", room.id), &room.name, fresh.then(|| self.summary(Some(i))), "home") })
            .collect();
        let scene = |t: &&Thing| matches!(t.e.domain(), Domain::Scene | Domain::Script);
        let named = |t: &&Thing| t.e.domain().placed() && !t.extra && matches_words(&t.e.name(), &query);
        let things = self.things.iter().filter(named);
        for t in things.clone().filter(scene).chain(things.filter(|t| !scene(t))) {
            let mut item = self.tile(t, &t.e.name());
            let room = self.room_name(t).map(str::to_string);
            let words = item.subtitle.take().filter(|_| fresh && t.e.domain() != Domain::Scene);
            item.subtitle = match (room, words) {
                (Some(room), Some(words)) => Some(format!("{room}, {}", uncapitalised(&words))),
                (Some(only), None) | (None, Some(only)) => Some(only),
                (None, None) => Some(t.e.domain().noun().to_string()),
            };
            if !fresh {
                item.on = None;
                item.tint = None;
            }
            found.push(item);
        }
        found.truncate(limit);
        found
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lounge_plugin_sdk::SheetRowKind;

    /// A home as Home Assistant lists it, with kinds the plugin does not show and one thing that is gone.
    const HOME: &str = r#"[
        {"entity_id":"sun.sun","state":"below_horizon","attributes":{"elevation":-6.74,"rising":false,"friendly_name":"Sun"},"last_changed":"2026-11-20T12:00:00.634546+00:00","last_reported":"2026-11-20T12:00:00.634546+00:00","last_updated":"2026-11-20T12:00:00.634546+00:00","context":{"id":"01M7WTEV82JPMHHH4VW4Q5PM17","parent_id":null,"user_id":null}},
        {"entity_id":"update.demo_add_on","state":"on","attributes":{"installed_version":"1.0.0","friendly_name":"Demo add-on"}},
        {"entity_id":"sensor.power_consumption","state":"100","attributes":{"device_class":"power","unit_of_measurement":"W","friendly_name":"Power consumption"}},
        {"entity_id":"binary_sensor.movement_backyard","state":"on","attributes":{"device_class":"motion","friendly_name":"Movement Backyard"}},
        {"entity_id":"light.ghost_lamp","state":"unavailable","attributes":{"restored":true,"supported_color_modes":["onoff"],"friendly_name":"Ghost Lamp","supported_features":0}},
        {"entity_id":"light.ceiling_lights","state":"on","attributes":{"min_color_temp_kelvin":2000,"max_color_temp_kelvin":6535,"supported_color_modes":["color_temp","hs"],"color_mode":"color_temp","brightness":180,"color_temp_kelvin":2631,"hs_color":[28.55,67.974],"rgb_color":[255,164,82],"friendly_name":"Ceiling Lights","supported_features":0},"last_changed":"2026-11-20T12:00:00+00:00","last_updated":"2026-11-20T15:00:00+00:00"},
        {"entity_id":"light.living_room_lamp","state":"on","attributes":{"supported_color_modes":["hs"],"color_mode":"hs","brightness":153,"rgb_color":[255,64,112],"friendly_name":"Living Room Lamp","supported_features":0}},
        {"entity_id":"light.living_room","state":"on","attributes":{"min_color_temp_kelvin":2000,"max_color_temp_kelvin":6535,"supported_color_modes":["color_temp","hs"],"brightness":166,"color_temp_kelvin":null,"rgb_color":[255,114,97],"entity_id":["light.ceiling_lights","light.living_room_lamp"],"friendly_name":"Living Room","supported_features":0}},
        {"entity_id":"cover.living_room_window","state":"open","attributes":{"current_position":70,"current_tilt_position":50,"friendly_name":"Living Room Window","supported_features":255}},
        {"entity_id":"climate.heatpump","state":"heat","attributes":{"hvac_modes":["heat","off"],"min_temp":7.0,"max_temp":35.0,"current_temperature":19.5,"temperature_unit":"°C","temperature":21.0,"hvac_action":"heating","friendly_name":"HeatPump","supported_features":385}},
        {"entity_id":"fan.living_room_fan","state":"off","attributes":{"preset_modes":["auto","smart","sleep","on"],"direction":"forward","oscillating":false,"percentage":null,"percentage_step":33.333333333333336,"preset_mode":null,"friendly_name":"Living Room Fan","supported_features":55}},
        {"entity_id":"media_player.living_room","state":"playing","attributes":{"volume_level":0.4,"is_volume_muted":false,"media_title":"Kind of Blue","source_list":["dvd","youtube"],"source":"dvd","friendly_name":"Living Room","supported_features":152509}},
        {"entity_id":"switch.living_room_child_lock","state":"off","attributes":{"friendly_name":"Living Room Child lock"}},
        {"entity_id":"switch.decorative_lights","state":"on","attributes":{"assumed_state":true,"friendly_name":"Decorative Lights"}},
        {"entity_id":"scene.movie_time","state":"unknown","attributes":{"entity_id":["light.ceiling_lights","light.living_room_lamp"],"id":"lounge_movie_time","friendly_name":"Movie time"}},
        {"entity_id":"scene.living_room_relax","state":"2026-11-19T20:00:00+00:00","attributes":{"group_name":"Living Room","is_dynamic":false,"friendly_name":"Living Room Relax"}},
        {"entity_id":"light.kitchen_lights","state":"off","attributes":{"supported_color_modes":["onoff"],"color_mode":null,"friendly_name":"Kitchen Lights","supported_features":0}},
        {"entity_id":"switch.kitchen_plug","state":"on","attributes":{"device_class":"outlet","friendly_name":"Kitchen Plug"}},
        {"entity_id":"sensor.kitchen_plug_temperature","state":"35.2","attributes":{"device_class":"temperature","unit_of_measurement":"°C","friendly_name":"Kitchen Plug Temperature"}},
        {"entity_id":"sensor.kitchen_temperature","state":"21.4","attributes":{"device_class":"temperature","unit_of_measurement":"°C","friendly_name":"Kitchen Temperature"}},
        {"entity_id":"sensor.kitchen_humidity","state":"48","attributes":{"device_class":"humidity","unit_of_measurement":"%","friendly_name":"Kitchen Humidity"}},
        {"entity_id":"lock.kitchen_door","state":"unlocked","attributes":{"friendly_name":"Kitchen Door","supported_features":0},"last_changed":"2026-11-20T14:03:00+00:00","last_updated":"2026-11-20T14:03:00+00:00"},
        {"entity_id":"vacuum.robo","state":"docked","attributes":{"fan_speed_list":["min","medium","high","max"],"fan_speed":"medium","cleaned_area":0,"friendly_name":"Robo","supported_features":32700}},
        {"entity_id":"binary_sensor.kitchen_window","state":"off","attributes":{"device_class":"window","friendly_name":"Kitchen Window"}},
        {"entity_id":"binary_sensor.kitchen_leak","state":"off","attributes":{"device_class":"moisture","friendly_name":"Kitchen Leak"}},
        {"entity_id":"light.office_lamp","state":"unavailable","attributes":{"supported_color_modes":["brightness"],"friendly_name":"Office Lamp","supported_features":0}},
        {"entity_id":"climate.ecobee","state":"heat_cool","attributes":{"hvac_modes":["off","cool","heat_cool","auto","dry","fan_only"],"min_temp":7,"max_temp":35,"fan_modes":["on_low","on_high","auto_low","auto_high","off"],"preset_modes":["home","eco","away"],"current_temperature":23,"temperature_unit":"°C","target_temp_high":24,"target_temp_low":21,"fan_mode":"auto_low","preset_mode":"home","friendly_name":"Ecobee","supported_features":442}},
        {"entity_id":"lock.front_door","state":"locked","attributes":{"friendly_name":"Front Door","supported_features":0}},
        {"entity_id":"binary_sensor.front_door","state":"on","attributes":{"device_class":"door","friendly_name":"Front door Door"},"last_changed":"2026-11-20T15:50:30.123456+00:00","last_updated":"2026-11-20T15:50:30.123456+00:00"},
        {"entity_id":"binary_sensor.hall_smoke","state":"off","attributes":{"device_class":"smoke","friendly_name":"Hall Smoke"}},
        {"entity_id":"sensor.hall_sensor_battery","state":"9","attributes":{"device_class":"battery","unit_of_measurement":"%","friendly_name":"Hall Sensor Battery"}},
        {"entity_id":"alarm_control_panel.house_alarm","state":"disarmed","attributes":{"code_format":null,"changed_by":null,"code_arm_required":false,"friendly_name":"House Alarm","supported_features":7}},
        {"entity_id":"camera.porch","state":"idle","attributes":{"access_token":"0123abcd","entity_picture":"/api/camera_proxy/camera.porch?token=0123abcd","friendly_name":"Porch","supported_features":0}},
        {"entity_id":"cover.garage_door","state":"closed","attributes":{"is_closed":true,"device_class":"garage","friendly_name":"Garage Door","supported_features":3}},
        {"entity_id":"lock.cellar_door","state":"locked","attributes":{"code_format":"\\d{4}","friendly_name":"Cellar Door","supported_features":0}},
        {"entity_id":"scene.good_night","state":"unknown","attributes":{"entity_id":["light.ceiling_lights","light.kitchen_lights"],"id":"lounge_good_night","friendly_name":"Good night"}},
        {"entity_id":"script.welcome_home","state":"off","attributes":{"last_triggered":null,"mode":"single","current":0,"friendly_name":"Welcome home"}},
        {"entity_id":"camera.drive","state":"streaming","attributes":{"access_token":"feed5678","entity_picture":"/api/camera_proxy/camera.drive?token=feed5678","friendly_name":"Drive","supported_features":3}},
        {"entity_id":"fan.hidden_fan","state":"off","attributes":{"friendly_name":"Hidden Fan","supported_features":0}},
        {"entity_id":"input_boolean.guest_mode","state":"off","attributes":{"editable":false,"friendly_name":"Guest mode"}},
        {"entity_id":"person.ana","state":"home","attributes":{"editable":false,"id":"ana","friendly_name":"Ana"}},
        {"entity_id":"person.ben","state":"home","attributes":{"editable":false,"id":"ben","friendly_name":"Ben"}},
        {"entity_id":"person.dee","state":"not_home","attributes":{"editable":false,"id":"dee","friendly_name":"Dee"}},
        {"entity_id":"weather.home","state":"cloudy","attributes":{"temperature":11.6,"temperature_unit":"°C","humidity":82,"friendly_name":"Home","supported_features":3}},
        {"entity_id":"sensor.ana_s_phone_battery_level","state":"7","attributes":{"device_class":"battery","unit_of_measurement":"%","friendly_name":"Ana's Phone Battery Level"}},
        {"entity_id":"sensor.outside_temperature_battery","state":"62","attributes":{"device_class":"battery","unit_of_measurement":"%","friendly_name":"Outside Temperature Battery"}}
    ]"#;

    /// What the rooms template answers for that home, as Home Assistant writes it.
    const ROOMS_ANSWER: &str = r#"{"areas": [
        {"d": [["media_player.living_room", "switch.living_room_child_lock"]], "e": ["fan.living_room_fan", "media_player.living_room", "switch.living_room_child_lock", "scene.living_room_relax", "light.ceiling_lights", "light.living_room_lamp", "light.living_room", "climate.heatpump", "switch.decorative_lights", "cover.living_room_window"], "id": "living_room", "name": "Living Room"},
        {"d": [["sensor.kitchen_temperature", "sensor.kitchen_humidity"], ["switch.kitchen_plug", "sensor.kitchen_plug_temperature"]], "e": ["binary_sensor.kitchen_window", "binary_sensor.kitchen_leak", "light.kitchen_lights", "switch.kitchen_plug", "sensor.kitchen_plug_temperature", "sensor.kitchen_temperature", "sensor.kitchen_humidity", "lock.kitchen_door", "vacuum.robo"], "id": "kitchen", "name": "Kitchen"},
        {"d": [], "e": ["light.office_lamp", "climate.ecobee"], "id": "buro_cafe", "name": "Büro & Café"},
        {"d": [], "e": ["lock.front_door", "binary_sensor.front_door", "binary_sensor.hall_smoke", "sensor.hall_sensor_battery", "alarm_control_panel.house_alarm", "camera.porch"], "id": "hall", "name": "Hall"},
        {"d": [], "e": ["lock.cellar_door", "cover.garage_door"], "id": "garage", "name": "Garage"},
        {"d": [], "e": [], "id": "attic", "name": "Attic"}],
        "hidden": ["fan.hidden_fan"], "label": ["light.ceiling_lights", "cover.living_room_window", "light.office_lamp", "climate.ecobee"], "label_found": true, "labels": true, "phones": ["sensor.ana_s_phone_battery_level"]}"#;

    const BASE: &str = "http://192.168.0.20:8123";
    const TOKEN: &str = "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.long.lived";

    /// Three minutes past four that afternoon, a little over twelve minutes after the front door opened.
    fn now() -> i64 {
        sdk::epoch_ms(2026, 11, 20, 16, 3, 0, 0)
    }

    fn states() -> Vec<Entity> {
        read_states(HOME).unwrap()
    }

    fn registry() -> Registry {
        let mut registry = Registry { sig: "s".into(), at_ms: now(), home_name: Some("Home".into()), ..Default::default() };
        registry.take(serde_json::from_str(ROOMS_ANSWER).unwrap());
        registry
    }

    /// The home with this TV in `here`, as a refresh would sort it out.
    fn home_in(here: Option<&str>) -> Home {
        Home::build(states(), &registry(), here, &Options::default(), now())
    }

    /// The same home after `change` has been made to one thing's state.
    fn home_after(here: Option<&str>, changes: &[(&str, &str)]) -> Home {
        let mut states = states();
        for (id, state) in changes {
            states.iter_mut().find(|e| e.id == *id).unwrap_or_else(|| panic!("no {id}")).state = state.to_string();
        }
        Home::build(states, &registry(), here, &Options::default(), now())
    }

    fn thing(id: &str) -> Entity {
        states().into_iter().find(|e| e.id == id).unwrap_or_else(|| panic!("no {id}"))
    }

    fn one(state: &str) -> Entity {
        Entity::read(serde_json::from_str(state).unwrap()).unwrap()
    }

    fn lines(items: &[Item]) -> Vec<(&str, &str, &str)> {
        items.iter().map(|i| (i.id.as_str(), i.title.as_str(), i.subtitle.as_deref().unwrap_or(""))).collect()
    }

    fn row<'a>(items: &'a [Item], row: &str) -> Vec<&'a Item> {
        items.iter().filter(|i| i.row.as_deref() == Some(row)).collect()
    }

    /// What a tile does: the press it sends, or the sheet it opens.
    fn does(item: &Item) -> String {
        match (&item.action.resolve, &item.action.sheet) {
            (Some(press), _) => press.clone(),
            (None, Some(sheet)) => format!("opens {sheet}"),
            (None, None) => String::new(),
        }
    }

    /// A sheet's rows, one line each: the kind, the title, the value, then what the keys do.
    fn rows(sheet: &Sheet) -> Vec<String> {
        sheet
            .rows
            .iter()
            .map(|r| {
                let keys = [("ok", &r.resolve), ("left", &r.down), ("right", &r.up), ("opens", &r.sheet), ("asks", &r.ask)];
                let does: Vec<String> = keys.into_iter().filter_map(|(key, action)| action.as_ref().map(|a| format!("{key} {a}"))).collect();
                format!("{:?} | {} | {} | {}", r.kind, r.title, r.value.as_deref().unwrap_or(""), does.join(", ")).trim_end_matches([' ', '|']).to_string()
            })
            .collect()
    }

    /// The TV's own rule for an error it shows as written.
    fn reads_as_sentence(text: &str) -> bool {
        text.chars().next().is_some_and(char::is_uppercase) && text.ends_with(['.', '?']) && text.len() <= 400
    }

    // Manifest and reading

    #[test]
    fn a_sign_in_keeps_its_link_and_takes_only_its_own_answer() {
        let landing = "http://192.168.0.30:8787/signin/tv.lounge.homeassistant";
        let started = Started::offer(None, BASE, landing, 1_000);
        assert_eq!(started.client, "http://192.168.0.30:8787/");
        assert_eq!(
            started.link(),
            format!("{BASE}/auth/authorize?client_id=http%3A%2F%2F192.168.0.30%3A8787%2F&redirect_uri=http%3A%2F%2F192.168.0.30%3A8787%2Fsignin%2Ftv.lounge.homeassistant&state={}", started.state)
        );
        // The link stays the same while someone may be using it, and changes with the server or the TV's address.
        assert_eq!(Started::offer(Some(started.clone()), BASE, landing, 9_000), started);
        assert_ne!(Started::offer(Some(started.clone()), "http://192.168.0.21:8123", landing, 9_000).state, started.state);
        assert_ne!(Started::offer(Some(started.clone()), BASE, "http://192.168.0.31:8787/signin/tv.lounge.homeassistant", 9_000).client, started.client);

        assert_eq!(started.code(&format!("code=abc%2F123&state={}", started.state)).as_deref(), Some("abc/123"));
        assert_eq!(started.code(&format!("state={}&code=abc", started.state)).as_deref(), Some("abc"));
        // An answer to some other sign-in, or one with no code, is not taken.
        assert_eq!(started.code("code=abc&state=0000000000000000"), None);
        assert_eq!(started.code("code=abc"), None);
        assert_eq!(started.code(&format!("state={}", started.state)), None);
        assert_eq!(started.code(&format!("code=&state={}", started.state)), None);
    }

    #[test]
    fn a_list_is_written_again_only_when_it_has_changed() {
        let sums = |lists: &[&str]| lists.iter().map(|list| fnv_of(list.as_bytes())).collect::<Vec<_>>();
        let stored = sums(&["one", "two"]);
        assert!(unchanged(&stored, &sums(&["one", "two"]), 0) && unchanged(&stored, &sums(&["one", "two"]), 1));
        assert!(unchanged(&stored, &sums(&["one", "three"]), 0) && !unchanged(&stored, &sums(&["one", "three"]), 1));
        // A list there was none of before is new, and a store from before sums were kept is all new.
        assert!(!unchanged(&stored, &sums(&["one", "two", "more"]), 2));
        assert!(!unchanged(&[], &sums(&["one"]), 0));
        assert_eq!(fnv("abc"), fnv_of(b"abc"));
    }

    #[test]
    fn a_sign_in_is_renewed_a_minute_before_it_runs_out() {
        assert_eq!(Session::expiry(1_000, 1800), 1_000 + 1_740_000);
        // An answer with no lifetime, or an absurd one, still leaves time to use the token.
        assert_eq!(Session::expiry(0, 0), 60_000);
        assert_eq!(Session::expiry(0, 10_000_000), 86_340_000);
        assert_eq!(Auth::Session("abc".into(), "one".into()).header(), "Bearer abc");
        // What a sign-in was refused stays refused when its token is renewed, and a new sign-in starts afresh.
        let signed_in = |access: &str, sign_in: &str| Server { base: BASE.into(), auth: Auth::Session(access.into(), sign_in.into()) };
        assert_eq!(signed_in("first.token", "one").mark(), signed_in("renewed.token", "one").mark());
        assert_ne!(signed_in("first.token", "one").mark(), signed_in("first.token", "two").mark());
        let session = Session { base: BASE.into(), client: "http://192.168.0.30:8787/".into(), access: "a".into(), expires_ms: 0, refresh: "r".into() };
        assert_eq!(session.name(), Session { access: "b".into(), expires_ms: 9, ..session.clone() }.name());
        assert_ne!(session.name(), Session { refresh: "other".into(), ..session.clone() }.name());
        assert_eq!((encoded("a b&c=d/é"), decoded("a+b%26c%3Dd%2F%C3%A9"), decoded("100%")), ("a%20b%26c%3Dd%2F%C3%A9".to_string(), "a b&c=d/é".to_string(), "100%".to_string()));
    }

    #[test]
    fn the_manifest_is_sound() {
        let manifest = <HomeAssistant as sdk::Plugin>::manifest();
        manifest.validate().unwrap();
        assert_eq!(manifest.settings.iter().filter(|s| s.required).map(|s| s.key.as_str()).collect::<Vec<_>>(), ["server"]);
        for setting in &manifest.settings {
            let help = setting.help.as_deref().unwrap_or("");
            assert!(help.chars().count() <= 160, "the TV shows the help for {} whole only up to 160 characters", setting.key);
        }
        assert_eq!(manifest.rows.iter().map(|r| (r.id.as_str(), r.title.as_str())).collect::<Vec<_>>(), [("here", "This room"), ("rooms", "Rooms"), ("scenes", "Scenes"), ("cameras", "Cameras"), ("around", "Around the home")]);
        // A server on the home network is asked every half minute, and may show its own certificate.
        let at = |key: &str| (key == "server").then(|| "http://192.168.0.20:8123".to_string());
        assert_eq!((manifest.refresh_every_ms(&at), manifest.self_signed_servers), (30_000, true));
        assert!(manifest.allows_host_with("192.168.0.20", &at) && !manifest.allows_host("192.168.0.20"));
        assert_eq!(<HomeAssistant as sdk::Plugin>::refresh(&RefreshContext::default()).err().as_deref(), Some("Add the Home Assistant address in the plugin's settings."));
        assert_eq!(<HomeAssistant as sdk::Plugin>::resolve("toggle:light.bed_light").err().as_deref(), Some("Add the Home Assistant address in the plugin's settings."));
    }

    #[test]
    fn states_are_read_one_at_a_time_and_only_the_kinds_shown_are_kept() {
        let states = states();
        let ids: Vec<&str> = states.iter().map(|e| e.id.as_str()).collect();
        // The sun, an update, a power meter and a motion sensor pass by, and so does a lamp Home Assistant only remembers.
        for left_out in ["sun.sun", "update.demo_add_on", "sensor.power_consumption", "binary_sensor.movement_backyard", "light.ghost_lamp"] {
            assert!(!ids.contains(&left_out), "{left_out}");
        }
        assert_eq!(ids.len(), 41);
        let ceiling = thing("light.ceiling_lights");
        assert_eq!((ceiling.state.as_str(), ceiling.a.brightness, ceiling.a.rgb_color, ceiling.a.color_temp_kelvin), ("on", Some(180.0), Some([255, 164, 82]), Some(2631.0)));
        assert_eq!((ceiling.changed_ms, ceiling.updated_ms), (Some(sdk::epoch_ms(2026, 11, 20, 12, 0, 0, 0)), Some(sdk::epoch_ms(2026, 11, 20, 15, 0, 0, 0))));
        // An attribute of an unexpected shape is left out, and the rest of the thing still reads.
        let odd = one(r#"{"entity_id":"light.odd","state":"on","attributes":{"brightness":"bright","rgb_color":[300,0,0],"supported_color_modes":"hs","friendly_name":"Odd"}}"#);
        assert_eq!((odd.name().as_str(), odd.a.brightness, odd.a.rgb_color, odd.a.supported_color_modes.clone()), ("Odd", None, None, None));
        assert!(one(r#"{"entity_id":"switch.bare","state":"on"}"#).a == Attributes::default());
        // A page that is not Home Assistant's, or an answer cut short, is no list of states.
        assert!(read_states("<html>").is_err() && read_states(r#"[{"entity_id":"light.a","state":"on"}"#).is_err());
        assert!(read_states("[]").unwrap().is_empty());
    }

    #[test]
    fn the_registry_answer_becomes_rooms() {
        let registry = registry();
        assert_eq!(registry.mode, Mode::Rooms);
        assert_eq!(registry.areas.iter().map(|a| (a.id.as_str(), a.shown_name())).collect::<Vec<_>>()[2], ("buro_cafe", "Büro & Café".to_string()));
        assert!(registry.has("attic") && !registry.has("cellar") && !registry.has(""));
        let home = home_in(None);
        assert_eq!(home.rooms.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(), ["Attic", "Büro & Café", "Garage", "Hall", "Kitchen", "Living Room"]);
        // A hidden fan is nowhere, and a helper nobody gave a room is not shown.
        assert!(home.find("fan.hidden_fan").is_none() && home.find("input_boolean.guest_mode").is_none());
        assert_eq!(home.find("light.office_lamp").and_then(|t| home.room_name(t)), Some("Büro & Café"));
        // With a label set, only what carries it is left, with the people and the weather.
        let labelled = Home::build(states(), &registry, None, &Options { label: Some("TV".into()), scripts: false }, now());
        assert_eq!(labelled.things.iter().map(|t| t.e.id.as_str()).collect::<Vec<_>>(), ["light.office_lamp", "light.ceiling_lights", "cover.living_room_window", "climate.ecobee", "person.ana", "person.ben", "person.dee", "weather.home"]);
        // A Home Assistant with no areas answers with none.
        let mut bare = Registry::default();
        bare.take(serde_json::from_str(r#"{"areas": [], "hidden": [], "labels": true, "label_found": false, "label": [], "phones": []}"#).unwrap());
        assert_eq!(bare.mode, Mode::Flat);
        // The plainer template's answer reads too.
        let plain: Rendered = serde_json::from_str(r#"{"areas": [{"id": "den", "name": null, "e": ["light.den"]}]}"#).unwrap();
        assert_eq!((plain.areas[0].shown_name().as_str(), plain.labels, plain.hidden.len()), ("Den", false, 0));
    }

    #[test]
    fn the_registry_is_trimmed_to_fit_one_entry() {
        let small = registry();
        let read: HashSet<&str> = HashSet::new();
        assert_eq!(serde_json::from_slice::<Registry>(&small.stored(&read).unwrap()).unwrap(), small);
        // Rooms of a hundred lights and a hundred sensors the plugin does not read, each light on a
        // device with one other thing, which for every fourth is a switch.
        let room = |n: usize| {
            let e = (0..200).map(|i| format!("{}.room_{n}_thing_number_{i}", if i % 2 == 0 { "light" } else { "sensor" })).collect();
            let d = (0..200).map(|i| vec![format!("light.room_{n}_thing_number_{i}"), format!("{}.room_{n}_other_{i}", if i % 4 == 0 { "switch" } else { "light" })]).collect();
            Area { id: format!("room_{n}"), name: Some(format!("Room {n}")), e, d }
        };
        let kept = |rooms: usize| {
            let home = Registry { areas: (0..rooms).map(room).collect(), mode: Mode::Rooms, ..small.clone() };
            assert!(serde_json::to_vec(&home).unwrap().len() > KV_ROOM, "{rooms} rooms fit as they are");
            let stored = home.stored(&read).unwrap();
            assert!(stored.len() <= KV_ROOM);
            let kept: Registry = serde_json::from_slice(&stored).unwrap();
            assert_eq!((kept.areas.len(), kept.mode), (rooms, Mode::Rooms));
            kept.areas
        };
        // The sensors nobody reads go first.
        assert!(kept(13).iter().all(|a| a.e.len() == 100 && a.e.iter().all(|id| id.starts_with("light.")) && a.d.len() == 200));
        // Then the devices that group no switch, then all of them.
        assert!(kept(20).iter().all(|a| a.e.len() == 100 && a.d.len() == 50 && a.d.iter().all(|device| device[1].starts_with("switch."))));
        assert!(kept(40).iter().all(|a| a.e.len() == 100 && a.d.is_empty()));
        // Last, each room keeps the front of its list, and every room is still there.
        let most = kept(80);
        assert!(most.iter().all(|a| a.d.is_empty() && a.e.len() == most[0].e.len() && (50..100).contains(&a.e.len())));
        // A sensor the plugin does read stays when the others go.
        let read: HashSet<&str> = ["sensor.room_0_thing_number_1"].into();
        let home = Registry { areas: (0..13).map(room).collect(), ..small.clone() };
        let kept: Registry = serde_json::from_slice(&home.stored(&read).unwrap()).unwrap();
        assert_eq!((kept.areas[0].e.len(), kept.areas[1].e.len()), (101, 100));
    }

    #[test]
    fn a_changed_setting_or_an_old_answer_asks_for_the_rooms_again() {
        let registry = registry();
        assert!(registry.stands("s", now()) && registry.stands("s", now() + 4 * 60_000));
        assert!(!registry.stands("s", now() + 5 * 60_000));
        // A clock set back does not keep an answer for ever.
        assert!(!registry.stands("s", now() - 1));
        assert!(!registry.stands("another server, token or label", now()));
        // The mark of a server and token changes with either, and gives neither away.
        let server = |address: &str, token: &str| Server::open(address, Some(token), None).unwrap();
        let mark = server(BASE, TOKEN).mark();
        assert_eq!(mark.len(), 16);
        assert!(mark != server("http://192.168.0.21:8123", TOKEN).mark() && mark != server(BASE, "another.token").mark());
        assert!(!mark.contains("eyJ") && !TOKEN.contains(&mark));
        assert_eq!(fnv(""), "cbf29ce484222325");
    }

    #[test]
    fn what_is_kept_comes_back_as_it_went() {
        let snap = Snap::default().renewed(states(), now());
        let lists = snap.lists();
        assert_eq!(lists.len(), 1);
        let back = Snap::read(snap.at_ms, lists.into_iter());
        assert_eq!((back.at_ms, back.entities.len(), back.parts), (now(), 41, 1));
        assert_eq!(back.entities, snap.entities);
        // Scenes and lights lead and sensors trail.
        assert_eq!((back.entities[0].id.as_str(), back.entities[40].id.as_str()), ("scene.good_night", "sensor.outside_temperature_battery"));
        // A home too large for one stored value is kept as several, and comes back whole and in order.
        let thing = |n: usize| Entity { id: format!("light.thing_{n:05}"), state: "on".into(), a: Attributes { friendly_name: Some(format!("A thing with a long name, number {n}")), ..Default::default() }, ..Default::default() };
        let large = Snap { at_ms: 1, entities: (0..5000).map(thing).collect(), parts: 0 };
        let lists = large.lists();
        assert!(lists.len() > 1 && lists.iter().all(|list| list.len() <= KV_ROOM), "{} lists", lists.len());
        let back = Snap::read(1, lists.clone().into_iter());
        assert_eq!((back.entities.len(), back.parts, &back.entities), (5000, lists.len(), &large.entities));
        // With a list gone missing nothing is trusted, and the next reading starts afresh.
        let torn = Snap::read(1, [lists[0].clone(), Vec::new()].into_iter());
        assert_eq!(torn, Snap::default());
        // There is an end to it: the front of an enormous home is kept, and the rest left out.
        let enormous = Snap { at_ms: 1, entities: (0..30_000).map(thing).collect(), parts: 0 };
        let lists = enormous.lists();
        let back = Snap::read(1, lists.clone().into_iter());
        assert_eq!(lists.len(), SNAP_PARTS);
        assert!(back.entities.len() < 30_000 && back.entities.len() > 10_000 && back.entities[..] == enormous.entities[..back.entities.len()]);
        assert_eq!(Snap::default().lists(), [b"[]".to_vec()]);
        assert_eq!((part_key(0), part_key(7)), ("snap:0".to_string(), "snap:7".to_string()));
        // What is absent is not written, so a switch takes a line and not a page.
        assert_eq!(serde_json::to_string(&one(r#"{"entity_id":"switch.ac","state":"off","attributes":{"friendly_name":"AC"}}"#)).unwrap(), r#"{"id":"switch.ac","state":"off","a":{"friendly_name":"AC"}}"#);
        // A thing stored with nothing but its state reads back with nothing else.
        assert_eq!(Snap::read(5, [br#"[{"id": "light.a", "state": "on"}]"#.to_vec()].into_iter()).entities[0].a, Attributes::default());
    }

    // Rows

    #[test]
    fn this_room_comes_first_with_its_scenes_then_lights() {
        let items = home_in(Some("living_room")).items(BASE);
        let here = row(&items, HERE);
        assert_eq!(
            lines(&here.iter().map(|i| (*i).clone()).collect::<Vec<_>>()),
            [
                ("here", "Whole room", "2 lights on, 19.5°"),
                ("e:scene.movie_time", "Movie time", "Scene"),
                ("e:scene.living_room_relax", "Relax", "Scene"),
                ("e:light.living_room", "Lights", "On, 65%"),
                ("e:light.ceiling_lights", "Ceiling Lights", "On, 71%"),
                ("e:light.living_room_lamp", "Lamp", "On, 60%"),
                ("e:cover.living_room_window", "Window", "70% open"),
                ("e:climate.heatpump", "HeatPump", "19.5°, heating to 21°"),
                ("e:fan.living_room_fan", "Fan", "Off"),
                ("e:media_player.living_room", "Speaker", "Playing, Kind of Blue"),
                ("e:switch.decorative_lights", "Decorative Lights", "On"),
            ]
        );
        assert_eq!(here.iter().map(|i| i.glyph.as_deref().unwrap_or("")).collect::<Vec<_>>(), ["home", "scene", "scene", "light", "light", "light", "blind", "thermostat", "fan", "speaker", "plug"]);
        assert_eq!(here.iter().map(|i| i.on).collect::<Vec<_>>(), [Some(true), None, None, Some(true), Some(true), Some(true), Some(true), Some(true), Some(false), Some(true), Some(true)]);
        // The room names its row, its tile opens the room, and the weights keep the order.
        assert_eq!((here[0].row_title.as_deref(), does(here[0]).as_str()), (Some("Living Room"), "opens room:living_room"));
        assert!(here.windows(2).all(|pair| pair[0].weight > pair[1].weight));
        // Every tile sits in a row the manifest declares, and every glyph is one Lounge draws.
        let declared: Vec<String> = <HomeAssistant as sdk::Plugin>::manifest().rows.into_iter().map(|r| r.id).collect();
        assert!(items.iter().all(|i| i.row.as_ref().is_some_and(|r| declared.contains(r))));
        assert!(items.iter().filter_map(|i| i.glyph.as_deref()).all(|g| sdk::GLYPHS.contains(&g)));
        let mut ids: Vec<&str> = items.iter().map(|i| i.id.as_str()).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), items.len());
    }

    #[test]
    fn a_tile_toggles_and_keeps_its_sheet_a_menu_away() {
        let items = home_in(Some("living_room")).items(BASE);
        let offers = |id: &str| items.iter().find(|i| i.id == id).map(|i| (does(i), i.more.clone().unwrap_or_default())).unwrap_or_else(|| panic!("no {id}"));
        let sheet = |entity: &str| format!("dev:{entity}");
        assert_eq!(offers("e:light.ceiling_lights"), ("toggle:light.ceiling_lights".to_string(), sheet("light.ceiling_lights")));
        assert_eq!(offers("e:fan.living_room_fan"), ("toggle:fan.living_room_fan".to_string(), sheet("fan.living_room_fan")));
        assert_eq!(offers("e:cover.living_room_window"), ("cover:cover.living_room_window:toggle".to_string(), sheet("cover.living_room_window")));
        assert_eq!(offers("e:media_player.living_room"), ("media:media_player.living_room:play_pause".to_string(), sheet("media_player.living_room")));
        // A switch has nothing to adjust, and a scene nothing but itself.
        assert_eq!(offers("e:switch.decorative_lights"), ("toggle:switch.decorative_lights".to_string(), String::new()));
        assert_eq!(offers("e:scene.movie_time"), ("scene:scene.movie_time".to_string(), String::new()));
        // A thermostat and an alarm have no one thing a press could mean.
        assert_eq!(offers("e:climate.heatpump"), ("opens dev:climate.heatpump".to_string(), String::new()));
        assert_eq!(offers("e:alarm_control_panel.house_alarm"), ("opens dev:alarm_control_panel.house_alarm".to_string(), String::new()));
        let kitchen = home_in(Some("kitchen")).items(BASE);
        let robo = kitchen.iter().find(|i| i.id == "e:vacuum.robo").unwrap();
        assert_eq!((does(robo), robo.more.as_deref(), robo.subtitle.as_deref()), ("vacuum:vacuum.robo:start".to_string(), Some("dev:vacuum.robo"), Some("Docked")));
        // A light that only switches has no sheet to offer.
        assert_eq!(kitchen.iter().find(|i| i.id == "e:light.kitchen_lights").map(|i| (does(i), i.more.clone())), Some(("toggle:light.kitchen_lights".to_string(), None)));
    }

    #[test]
    fn the_order_holds_when_states_change() {
        let before = home_in(Some("living_room")).items(BASE);
        let flipped = [
            ("light.ceiling_lights", "off"),
            ("light.living_room_lamp", "unavailable"),
            ("light.living_room", "off"),
            ("cover.living_room_window", "closed"),
            ("fan.living_room_fan", "on"),
            ("media_player.living_room", "off"),
            ("switch.decorative_lights", "off"),
            ("lock.front_door", "unlocked"),
            ("binary_sensor.kitchen_window", "on"),
            ("person.ana", "not_home"),
            ("alarm_control_panel.house_alarm", "armed_away"),
        ];
        let after = home_after(Some("living_room"), &flipped).items(BASE);
        let places = |items: &[Item]| items.iter().map(|i| (i.id.clone(), i.row.clone())).collect::<Vec<_>>();
        assert_eq!(places(&before), places(&after));
        // The words did change.
        assert_eq!(lines(&after)[3..6], [("e:light.living_room", "Lights", "Off"), ("e:light.ceiling_lights", "Ceiling Lights", "Off"), ("e:light.living_room_lamp", "Lamp", "Not responding")]);
        assert_eq!(lines(&after)[0], ("here", "Whole room", "Lights off, 19.5°"));
    }

    #[test]
    fn rooms_leave_this_one_out_and_end_with_what_has_no_room() {
        let items = home_in(Some("living_room")).items(BASE);
        let rooms = row(&items, ROOMS);
        assert_eq!(
            lines(&rooms.iter().map(|i| (*i).clone()).collect::<Vec<_>>()),
            [
                ("room:buro_cafe", "Büro & Café", "Lights off, 23°"),
                ("room:garage", "Garage", "2 devices"),
                ("room:hall", "Hall", "3 devices"),
                ("room:kitchen", "Kitchen", "Lights off, 21.4°"),
                ("room:", "Not in a room", "1 device"),
            ]
        );
        assert_eq!(rooms.iter().map(|i| does(i)).collect::<Vec<_>>(), ["opens room:buro_cafe", "opens room:garage", "opens room:hall", "opens room:kitchen", "opens room:"]);
        // A room is lit while a lamp in it is, and one without lamps is neither lit nor dark.
        assert_eq!(rooms.iter().map(|i| i.on).collect::<Vec<_>>(), [Some(false), None, None, Some(false), None]);
        assert_eq!(rooms.iter().map(|i| i.row_title.as_deref()).collect::<Vec<_>>(), [Some("Other rooms"), None, None, None, None]);
        // From the kitchen, the living room is one of the others. The attic holds nothing and stays out.
        let from_kitchen = home_in(Some("kitchen")).items(BASE);
        assert_eq!(row(&from_kitchen, ROOMS).iter().map(|i| i.title.as_str()).collect::<Vec<_>>(), ["Büro & Café", "Garage", "Hall", "Living Room", "Not in a room"]);
        assert_eq!(row(&from_kitchen, ROOMS)[3].subtitle.as_deref(), Some("2 lights on, 19.5°"));
    }

    #[test]
    fn with_no_room_chosen_the_first_tile_asks_for_one() {
        let items = home_in(None).items(BASE);
        let here = row(&items, HERE);
        assert_eq!(lines(&[here[0].clone()]), [("here", "Choose this room", "See its lights here")]);
        assert_eq!((here.len(), does(here[0]).as_str(), here[0].row_title.as_deref()), (1, "opens rooms", None));
        // Every room is one of the rooms, under the row's own title.
        let rooms = row(&items, ROOMS);
        assert_eq!(rooms.iter().map(|i| i.title.as_str()).collect::<Vec<_>>(), ["Büro & Café", "Garage", "Hall", "Kitchen", "Living Room", "Not in a room"]);
        assert!(rooms.iter().all(|i| i.row_title.is_none()));
        // A room that has since gone from Home Assistant is no room to be in.
        assert_eq!(lines(&home_in(Some("conservatory")).items(BASE))[0], ("here", "Choose this room", "See its lights here"));
    }

    #[test]
    fn the_rooms_sheet_chooses_with_one_press() {
        let sheet = home_in(Some("kitchen")).sheet(ROOMS).unwrap();
        assert_eq!((sheet.title.as_str(), sheet.subtitle.as_deref()), ("This TV's room", Some("Choose the room this TV is in")));
        assert_eq!(
            rows(&sheet),
            [
                "Press | Attic |  | ok here:attic",
                "Press | Büro & Café |  | ok here:buro_cafe",
                "Press | Garage |  | ok here:garage",
                "Press | Hall |  | ok here:hall",
                "Press | Kitchen | This TV | ok here:kitchen",
                "Press | Living Room |  | ok here:living_room",
                "Press | None of these |  | ok here:",
            ]
        );
        assert_eq!(sheet.rows.iter().filter(|r| r.on == Some(true)).map(|r| r.title.as_str()).collect::<Vec<_>>(), ["Kitchen"]);
        // Before a room is chosen there is nothing to take back.
        assert_eq!(home_in(None).sheet(ROOMS).unwrap().rows.len(), 6);
        // The press itself is kept by the TV, so it is only checked here that it reads.
        assert_eq!(Press::parse("here:buro_cafe"), Some(Press::Here("buro_cafe")));
        assert_eq!(Press::parse("here:"), Some(Press::Here("")));
    }

    #[test]
    fn a_home_without_areas_is_one_row() {
        let flat = Registry { mode: Mode::Flat, ..Default::default() };
        let home = Home::build(states(), &flat, Some("living_room"), &Options::default(), now());
        let items = home.items(BASE);
        let here = row(&items, HERE);
        // Everything that has a tile, by kind and then by name, and last a tile that says why there are no rooms.
        assert_eq!(here.iter().map(|i| i.title.as_str()).collect::<Vec<_>>()[..6], ["Living Room", "Ceiling Lights", "Kitchen Lights", "Living Room Lamp", "Office Lamp", "Garage Door"]);
        assert_eq!(lines(&[(*here.last().unwrap()).clone()]), [("norooms", "No rooms yet", "Areas show as rooms")]);
        assert_eq!((here.last().unwrap().row_title.as_deref(), does(here.last().unwrap()).as_str()), (Some("Home"), ""));
        assert_eq!(here.len(), 20);
        assert!(row(&items, ROOMS).is_empty());
        // The scenes are all the home's, and nothing is hidden since nothing said what is.
        assert_eq!(row(&items, SCENES).iter().map(|i| (i.title.as_str(), i.subtitle.as_deref().unwrap_or(""))).collect::<Vec<_>>(), [("Good night", "Scene"), ("Movie time", "Scene"), ("Living Room Relax", "Scene")]);
        assert!(here.iter().any(|i| i.id == "e:fan.hidden_fan"));
        // A home of a hundred lights still ends with the tile that explains.
        let lights = (0..100).map(|n| one(&format!(r#"{{"entity_id":"light.l{n:03}","state":"off","attributes":{{"friendly_name":"Light {n:03}"}}}}"#))).collect();
        let many = Home::build(lights, &flat, None, &Options::default(), now()).items(BASE);
        assert_eq!((row(&many, HERE).len(), row(&many, HERE)[59].id.as_str()), (60, "norooms"));
    }

    #[test]
    fn a_token_that_cannot_read_rooms_still_shows_the_home() {
        let no_admin = Registry { mode: Mode::NoAdmin, ..Default::default() };
        let home = Home::build(states(), &no_admin, None, &Options::default(), now());
        let items = home.items(BASE);
        let here = row(&items, HERE);
        assert_eq!(lines(&[(*here.last().unwrap()).clone()]), [("norooms", "No rooms shown", "Needs an admin token")]);
        assert_eq!(here.last().unwrap().row_title.as_deref(), Some("Home"));
        assert!(here.len() > 10 && row(&items, ROOMS).is_empty());
        // The row around the home needs no rooms.
        assert_eq!(row(&items, AROUND).iter().map(|i| i.id.as_str()).collect::<Vec<_>>(), ["e:alarm_control_panel.house_alarm", "locks", "openings", "batteries", "people", "weather", "lights"]);
        assert_eq!(home.log_line(), "No rooms without an administrator's token, 22 devices, 2 lights on");
        assert_eq!(home_in(None).log_line(), "6 rooms, 21 devices, 2 lights on");
    }

    #[test]
    fn scenes_find_their_room_and_scripts_wait_to_be_asked() {
        // Movie time sets two lamps of the living room, so that is its room. Good night sets lamps in two rooms.
        let home = home_in(Some("kitchen"));
        assert_eq!(home.find("scene.movie_time").and_then(|t| home.room_name(t)), Some("Living Room"));
        assert_eq!(home.find("scene.good_night").and_then(|t| t.room), None);
        let items = home.items(BASE);
        let scenes = row(&items, SCENES);
        assert_eq!(
            lines(&scenes.iter().map(|i| (*i).clone()).collect::<Vec<_>>()),
            [("e:scene.good_night", "Good night", "Scene"), ("e:scene.movie_time", "Movie time", "Living Room"), ("e:scene.living_room_relax", "Living Room Relax", "Living Room")]
        );
        assert!(scenes.iter().all(|i| i.on.is_none() && i.glyph.as_deref() == Some("scene") && does(i).starts_with("scene:scene.")));
        // In the living room its own scenes are in its row, and only the home's are left here.
        let items = home_in(Some("living_room")).items(BASE);
        assert_eq!(row(&items, SCENES).iter().map(|i| i.title.as_str()).collect::<Vec<_>>(), ["Good night"]);
        // Scripts join once the setting is on.
        let with_scripts = Home::build(states(), &registry(), Some("living_room"), &Options { label: None, scripts: true }, now()).items(BASE);
        let scenes = row(&with_scripts, SCENES);
        assert_eq!(scenes.iter().map(|i| (i.title.as_str(), i.subtitle.as_deref().unwrap_or(""), does(i))).collect::<Vec<_>>(), [("Good night", "Scene", "scene:scene.good_night".to_string()), ("Welcome home", "Script", "script:script.welcome_home".to_string())]);
        assert_eq!((scenes[1].glyph.as_deref(), scenes[1].on), (Some("script"), Some(false)));
        // A room with a dozen scenes from its lamps puts eight ahead of the lamps themselves.
        let mut states = states();
        states.extend((0..12).map(|n| one(&format!(r#"{{"entity_id":"scene.living_room_mood_{n:02}","state":"unknown","attributes":{{"friendly_name":"Living Room Mood {n:02}"}}}}"#))));
        let mut registry = registry();
        registry.areas[0].e.extend((0..12).map(|n| format!("scene.living_room_mood_{n:02}")));
        let items = Home::build(states, &registry, Some("living_room"), &Options::default(), now()).items(BASE);
        let here = row(&items, HERE);
        assert_eq!(here.iter().map(|i| i.title.as_str()).collect::<Vec<_>>()[..10], ["Whole room", "Movie time", "Mood 00", "Mood 01", "Mood 02", "Mood 03", "Mood 04", "Mood 05", "Mood 06", "Lights"]);
    }

    #[test]
    fn a_camera_tile_carries_a_snapshot_and_never_the_token() {
        let home = home_in(Some("hall"));
        let items = home.items(BASE);
        let cameras = row(&items, CAMERAS);
        // The hall's own camera comes first, and one without a room has no second line.
        assert_eq!(lines(&cameras.iter().map(|i| (*i).clone()).collect::<Vec<_>>()), [("e:camera.porch", "Porch", "Hall"), ("e:camera.drive", "Drive", "")]);
        assert_eq!(cameras[0].thumbnail.as_deref(), Some("http://192.168.0.20:8123/api/camera_proxy/camera.porch?token=0123abcd&width=960&height=540"));
        assert_eq!((does(cameras[0]).as_str(), cameras[0].action.title.as_deref(), cameras[0].glyph.as_deref()), ("camera:camera.porch", Some("Porch"), None));
        // Opened, it is the same picture at its full size, to be fetched again and again.
        let porch = thing("camera.porch");
        assert_eq!(camera_picture(BASE, &porch, false).as_deref(), Some("http://192.168.0.20:8123/api/camera_proxy/camera.porch?token=0123abcd"));
        // A camera that does not answer says so and shows no picture, and one with no token has none to show.
        let dark = home_after(Some("hall"), &[("camera.porch", "unavailable")]).items(BASE);
        assert_eq!(row(&dark, CAMERAS)[0].subtitle.as_deref(), Some("Not responding"));
        assert_eq!(row(&dark, CAMERAS)[0].thumbnail, None);
        assert_eq!(camera_picture(BASE, &one(r#"{"entity_id":"camera.old","state":"idle","attributes":{"friendly_name":"Old"}}"#), true), None);
        assert_eq!(camera_picture(BASE, &one(r#"{"entity_id":"camera.odd","state":"idle","attributes":{"access_token":"a&b=c"}}"#), true), None);
        // No more than twenty are shown.
        let many = (0..30).map(|n| one(&format!(r#"{{"entity_id":"camera.c{n}","state":"idle","attributes":{{"access_token":"t{n}"}}}}"#))).collect();
        assert_eq!(row(&Home::build(many, &Registry::default(), None, &Options::default(), now()).items(BASE), CAMERAS).len(), 20);
        // The access token is in the request's header and in nothing the TV is handed.
        let server = Server::open(BASE, Some(TOKEN), None).unwrap();
        assert_eq!(server.request("GET", "/api/states", None, 0).headers.get("Authorization").map(String::as_str), Some("Bearer eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.long.lived"));
        let written = serde_json::to_string(&home.items(&server.base)).unwrap();
        assert!(!written.contains(TOKEN) && !written.contains("Bearer") && written.contains("token=0123abcd"));
        let sheets = ["rooms", "room:hall", "room:", "lights", "open", "locks", "people", "batteries", "dev:camera.porch"];
        assert!(sheets.iter().all(|id| !serde_json::to_string(&home.sheet(id).unwrap()).unwrap().contains(TOKEN)));
    }

    #[test]
    fn the_glance_row_puts_problems_first_and_keeps_its_order() {
        let items = home_in(Some("living_room")).items(BASE);
        let around = row(&items, AROUND);
        assert_eq!(
            lines(&around.iter().map(|i| (*i).clone()).collect::<Vec<_>>()),
            [
                ("e:alarm_control_panel.house_alarm", "House Alarm", "Disarmed"),
                ("locks", "Kitchen Door unlocked", "For 2 hours"),
                ("openings", "Front door open", "For 12 minutes"),
                ("batteries", "Battery low", "Hall Sensor, 9%"),
                ("people", "Ana and Ben home", "Dee away"),
                ("weather", "12°C outside", "Cloudy"),
                ("lights", "2 lights on", "Living Room"),
            ]
        );
        assert_eq!(around.iter().map(|i| i.glyph.as_deref().unwrap_or("")).collect::<Vec<_>>(), ["shield", "lock", "door", "battery", "person", "sun", "light"]);
        assert_eq!(around.iter().map(|i| does(i)).collect::<Vec<_>>(), ["opens dev:alarm_control_panel.house_alarm", "opens locks", "opens open", "opens batteries", "opens people", "", "opens lights"]);
        assert_eq!(around.iter().map(|i| i.on).collect::<Vec<_>>(), [Some(false), Some(true), Some(true), Some(true), Some(true), None, Some(true)]);
        // Trouble comes in at the front, and nothing that was there moves.
        let trouble = [
            ("binary_sensor.hall_smoke", "on"),
            ("binary_sensor.kitchen_leak", "on"),
            ("lock.front_door", "unlocked"),
            ("binary_sensor.kitchen_window", "on"),
            ("cover.garage_door", "open"),
            ("sensor.outside_temperature_battery", "4"),
            ("person.dee", "home"),
            ("alarm_control_panel.house_alarm", "triggered"),
        ];
        let items = home_after(Some("living_room"), &trouble).items(BASE);
        assert_eq!(
            lines(&row(&items, AROUND).iter().map(|i| (*i).clone()).collect::<Vec<_>>()),
            [
                ("smoke", "Smoke detected", "Hall"),
                ("leak", "Water leak", "Kitchen"),
                ("e:alarm_control_panel.house_alarm", "House Alarm", "Triggered"),
                ("locks", "2 locks unlocked", "Front Door and Kitchen Door"),
                ("openings", "3 left open", "Garage Door, Front door, Kitchen Window"),
                ("batteries", "2 batteries low", "Outside Temperature and Hall Sensor"),
                ("people", "3 people home", "Ana, Ben, Dee"),
                ("weather", "12°C outside", "Cloudy"),
                ("lights", "2 lights on", "Living Room"),
            ]
        );
        // With everything shut, off and gone out, the tiles rest and the alerts are away.
        let calm = [
            ("lock.kitchen_door", "locked"),
            ("binary_sensor.front_door", "off"),
            ("sensor.hall_sensor_battery", "80"),
            ("person.ana", "not_home"),
            ("person.ben", "Work"),
            ("light.ceiling_lights", "off"),
            ("light.living_room_lamp", "off"),
            ("light.living_room", "off"),
        ];
        let items = home_after(Some("living_room"), &calm).items(BASE);
        let around = row(&items, AROUND);
        assert_eq!(
            lines(&around.iter().map(|i| (*i).clone()).collect::<Vec<_>>()),
            [
                ("e:alarm_control_panel.house_alarm", "House Alarm", "Disarmed"),
                ("locks", "All locked", "3 locks"),
                ("openings", "All closed", "3 doors and windows"),
                ("people", "Nobody home", ""),
                ("weather", "12°C outside", "Cloudy"),
                ("lights", "All lights off", ""),
            ]
        );
        assert_eq!(around.iter().map(|i| i.on).collect::<Vec<_>>(), [Some(false), Some(false), Some(false), Some(false), None, Some(false)]);
        // With no light on there is no list of them to open.
        assert_eq!(does(around[5]), "");
        // The sheets behind the tiles say the same.
        let home = home_in(Some("living_room"));
        assert_eq!(rows(&home.sheet("open").unwrap()), ["Text | Front door | Open", "Text | Garage Door | Closed", "Text | Kitchen Window | Closed"]);
        assert_eq!(home.sheet("open").unwrap().rows[0].subtitle.as_deref(), Some("For 12 minutes"));
        assert_eq!(rows(&home.sheet("people").unwrap()), ["Text | Ana | Home", "Text | Ben | Home", "Text | Dee | Away"]);
        assert_eq!(rows(&home.sheet("batteries").unwrap()), ["Text | Hall Sensor | 9%", "Text | Outside Temperature | 62%"]);
    }

    #[test]
    fn a_phones_battery_is_nobodys_problem() {
        let home = home_in(None);
        let phone = home.find("sensor.ana_s_phone_battery_level").unwrap();
        assert!(phone.phone && phone.e.number() == Some(7.0));
        assert_eq!(home.batteries().iter().map(|(t, level)| (t.e.id.as_str(), *level)).collect::<Vec<_>>(), [("sensor.hall_sensor_battery", 9.0), ("sensor.outside_temperature_battery", 62.0)]);
        // With the hall sensor's battery changed, nothing is low, though the phone is at 7%.
        let items = home_after(None, &[("sensor.hall_sensor_battery", "100")]).items(BASE);
        assert!(!items.iter().any(|i| i.id == "batteries"));
        // A battery that reports no number is not a low one.
        let items = home_after(None, &[("sensor.hall_sensor_battery", "unknown")]).items(BASE);
        assert!(!items.iter().any(|i| i.id == "batteries"));
        assert_eq!(battery_name(&thing("sensor.ana_s_phone_battery_level")), "Ana's Phone");
        assert_eq!(battery_name(&one(r#"{"entity_id":"sensor.battery","state":"50","attributes":{"device_class":"battery","friendly_name":"Battery"}}"#)), "Battery");
    }

    // Words

    #[test]
    fn every_second_line_fits_a_compact_tile() {
        let said = |state: &str| line(&one(state), false);
        let cases = [
            (r#"{"entity_id":"light.a","state":"on","attributes":{"supported_color_modes":["brightness"],"brightness":153}}"#, "On, 60%"),
            (r#"{"entity_id":"light.a","state":"on","attributes":{"supported_color_modes":["onoff"]}}"#, "On"),
            (r#"{"entity_id":"light.a","state":"off","attributes":{"supported_color_modes":["brightness"],"brightness":null}}"#, "Off"),
            (r#"{"entity_id":"light.a","state":"unavailable"}"#, "Not responding"),
            (r#"{"entity_id":"switch.a","state":"unknown"}"#, "Unknown"),
            (r#"{"entity_id":"fan.a","state":"on","attributes":{"percentage":50,"percentage_step":25,"preset_mode":null}}"#, "On, 50%"),
            (r#"{"entity_id":"fan.a","state":"on","attributes":{"percentage":null,"preset_mode":"auto"}}"#, "On, auto"),
            (r#"{"entity_id":"cover.a","state":"open","attributes":{"current_position":40}}"#, "40% open"),
            (r#"{"entity_id":"cover.a","state":"open","attributes":{"current_position":100}}"#, "Open"),
            (r#"{"entity_id":"cover.a","state":"closed","attributes":{"current_position":0}}"#, "Closed"),
            (r#"{"entity_id":"cover.a","state":"opening","attributes":{"current_position":40}}"#, "Opening"),
            (r#"{"entity_id":"cover.a","state":"closing"}"#, "Closing"),
            (r#"{"entity_id":"lock.a","state":"locked"}"#, "Locked"),
            (r#"{"entity_id":"lock.a","state":"unlocking"}"#, "Unlocking"),
            (r#"{"entity_id":"lock.a","state":"jammed"}"#, "Jammed"),
            (r#"{"entity_id":"climate.a","state":"heat","attributes":{"current_temperature":19,"temperature":21,"hvac_action":"heating"}}"#, "19°, heating to 21°"),
            (r#"{"entity_id":"climate.a","state":"cool","attributes":{"current_temperature":24,"temperature":22,"hvac_action":"cooling"}}"#, "24°, cooling to 22°"),
            (r#"{"entity_id":"climate.a","state":"heat","attributes":{"current_temperature":19,"temperature":21,"hvac_action":"idle"}}"#, "19°, set to 21°"),
            (r#"{"entity_id":"climate.a","state":"off","attributes":{"current_temperature":19,"temperature":null}}"#, "Off, 19°"),
            (r#"{"entity_id":"climate.a","state":"heat_cool","attributes":{"current_temperature":21,"target_temp_low":19,"target_temp_high":23}}"#, "21°, 19° to 23°"),
            (r#"{"entity_id":"climate.a","state":"heat","attributes":{"temperature":21}}"#, "Set to 21°"),
            (r#"{"entity_id":"media_player.a","state":"idle"}"#, "Idle"),
            (r#"{"entity_id":"media_player.a","state":"paused","attributes":{"media_title":"Blue"}}"#, "Paused, Blue"),
            (r#"{"entity_id":"vacuum.a","state":"returning"}"#, "Going home"),
            (r#"{"entity_id":"vacuum.a","state":"error"}"#, "Needs attention"),
            (r#"{"entity_id":"vacuum.a","state":"cleaning"}"#, "Cleaning"),
            (r#"{"entity_id":"script.a","state":"on"}"#, "Running"),
            (r#"{"entity_id":"humidifier.a","state":"on","attributes":{"current_humidity":45,"humidity":50}}"#, "45%, set to 50%"),
            (r#"{"entity_id":"humidifier.a","state":"off","attributes":{"humidity":50}}"#, "Off"),
            (r#"{"entity_id":"water_heater.a","state":"eco","attributes":{"current_temperature":50,"temperature":55}}"#, "50°, set to 55°"),
            (r#"{"entity_id":"water_heater.a","state":"eco","attributes":{"current_temperature":null,"temperature":48.3}}"#, "Set to 48.3°"),
            (r#"{"entity_id":"alarm_control_panel.a","state":"armed_home"}"#, "Armed, home"),
            (r#"{"entity_id":"alarm_control_panel.a","state":"armed_vacation"}"#, "Armed, vacation"),
            (r#"{"entity_id":"alarm_control_panel.a","state":"pending"}"#, "Entry delay"),
            (r#"{"entity_id":"alarm_control_panel.a","state":"arming"}"#, "Arming"),
            (r#"{"entity_id":"person.a","state":"not_home"}"#, "Away"),
        ];
        for (state, words) in cases {
            assert_eq!(said(state), words, "{state}");
            // A compact tile's second line fits about this much.
            assert!(words.chars().count() <= 20, "{words}");
        }
        for fixed in ["See its lights here", "Areas show as rooms", "Needs an admin token", "Not responding"] {
            assert!(fixed.chars().count() <= 20, "{fixed}");
        }
        // The sky is a word or two, and a condition without one says nothing.
        for condition in ["clear-night", "cloudy", "fog", "hail", "lightning", "lightning-rainy", "partlycloudy", "pouring", "rainy", "snowy", "snowy-rainy", "sunny", "windy", "windy-variant"] {
            assert!(sky(condition).is_some_and(|words| words.chars().count() <= 20), "{condition}");
        }
        assert_eq!((sky("exceptional"), sky("unavailable")), (None, None));
    }

    #[test]
    fn temperatures_follow_the_homes_unit_and_wear_a_true_minus() {
        assert_eq!((degrees(21.0, false), degrees(19.5, false), degrees(19.96, false), degrees(21.44, false)), ("21°".into(), "19.5°".into(), "20°".into(), "21.4°".into()));
        assert_eq!((degrees(-4.0, false), degrees(-0.5, false), degrees(-0.04, false)), ("\u{2212}4°".into(), "\u{2212}0.5°".into(), "0°".into()));
        // Fahrenheit is whole degrees.
        assert_eq!((degrees(68.5, true), degrees(-3.4, true), degrees(-0.4, true)), ("69°".into(), "\u{2212}3°".into(), "0°".into()));
        let fahrenheit = Registry { fahrenheit: true, ..registry() };
        let home = Home::build(states(), &fahrenheit, Some("living_room"), &Options::default(), now());
        let sheet = home.sheet("dev:climate.heatpump").unwrap();
        // A step is one degree there, and half of one in Celsius.
        assert_eq!(rows(&sheet)[1], "Step | Temperature | 21° | left set:climate.heatpump:temperature:20.0, right set:climate.heatpump:temperature:22.0");
        // Only the reading from outside names its scale, by the forecast's own unit.
        let outside = |unit: &str, reading: f64| {
            let weather = one(&format!(r#"{{"entity_id":"weather.home","state":"snowy","attributes":{{"temperature":{reading},"temperature_unit":"{unit}","friendly_name":"Home"}}}}"#));
            let items = Home::build(vec![weather], &Registry::default(), None, &Options::default(), now()).items(BASE);
            items.iter().find(|i| i.id == "weather").map(|i| format!("{}, {}", i.title, i.subtitle.as_deref().unwrap_or(""))).unwrap()
        };
        assert_eq!((outside("°C", -3.6), outside("°F", 25.2)), ("\u{2212}4°C outside, Snow".to_string(), "25°F outside, Snow".to_string()));
        // The forecast named after the home is the one for where it stands.
        let two = vec![one(r#"{"entity_id":"weather.a_far_place","state":"sunny","attributes":{"temperature":30,"friendly_name":"Far"}}"#), one(r#"{"entity_id":"weather.forecast_home","state":"rainy","attributes":{"temperature":8,"friendly_name":"Home"}}"#)];
        let named = Registry { home_name: Some("Home".into()), ..Default::default() };
        assert_eq!(Home::build(two.clone(), &named, None, &Options::default(), now()).weather().map(|t| t.e.id.as_str()), Some("weather.forecast_home"));
        assert_eq!(Home::build(two, &Registry::default(), None, &Options::default(), now()).weather().map(|t| t.e.id.as_str()), Some("weather.a_far_place"));
    }

    #[test]
    fn a_lamp_lends_the_tile_its_colour() {
        let items = home_in(Some("living_room")).items(BASE);
        let tint = |id: &str| items.iter().find(|i| i.id == id).and_then(|i| i.tint.clone());
        // A lamp set to a white has a colour too, which is the warmth of its light.
        assert_eq!(tint("e:light.ceiling_lights").as_deref(), Some("#FFA452"));
        assert_eq!(tint("e:light.living_room_lamp").as_deref(), Some("#FF4070"));
        assert_eq!((tint("e:switch.decorative_lights"), tint("e:media_player.living_room"), tint("here")), (None, None, None));
        // Off, it lends nothing, whatever colour it last had.
        let off = one(r#"{"entity_id":"light.a","state":"off","attributes":{"rgb_color":[255,0,0]}}"#);
        assert_eq!(super::tint(&off), None);
        assert_eq!(super::tint(&one(r#"{"entity_id":"light.a","state":"on","attributes":{"rgb_color":[0,8,255]}}"#)).as_deref(), Some("#0008FF"));
        assert_eq!(super::tint(&one(r#"{"entity_id":"light.a","state":"on","attributes":{"rgb_color":null}}"#)), None);
    }

    #[test]
    fn a_room_is_summed_up_in_a_line() {
        let home = home_in(None);
        let said = |room: &str| home.summary(home.rooms.iter().position(|r| r.id == room));
        // The group stands for the two lamps, so it is not counted with them.
        assert_eq!(said("living_room"), "2 lights on, 19.5°");
        // The plug's own thermometer reads 35°. The room's is the sensor that reads humidity too.
        assert_eq!(said("kitchen"), "Lights off, 21.4°");
        assert_eq!(said("buro_cafe"), "Lights off, 23°");
        assert_eq!((said("garage"), said("hall"), said("attic")), ("2 devices".to_string(), "3 devices".to_string(), "0 devices".to_string()));
        let one_on = home_after(None, &[("light.living_room_lamp", "off")]);
        assert_eq!(one_on.summary(one_on.rooms.iter().position(|r| r.id == "living_room")), "1 light on, 19.5°");
        // A room whose only light is a group counts the group.
        let hue = vec![one(r#"{"entity_id":"light.den","state":"on","attributes":{"is_hue_group":true,"friendly_name":"Den"}}"#), one(r#"{"entity_id":"sensor.den_temperature","state":"20","attributes":{"device_class":"temperature","friendly_name":"Den Temperature"}}"#)];
        let mut den = Registry::default();
        den.take(serde_json::from_str(r#"{"areas": [{"id": "den", "name": "Den", "e": ["light.den", "sensor.den_temperature"], "d": []}]}"#).unwrap());
        let home = Home::build(hue, &den, Some("den"), &Options::default(), now());
        // Its thermometer is on a device that reads nothing else, so it is not taken for the room's.
        assert_eq!(lines(&home.items(BASE))[..2], [("here", "Whole room", "1 light on"), ("e:light.den", "Lights", "On")]);
    }

    #[test]
    fn names_lose_their_room_and_keep_their_sense() {
        let named = |name: &str, room: &str, id: &str| short_name(&tidy_name(name), Some(room), &one(&format!(r#"{{"entity_id":"{id}","state":"on"}}"#)));
        assert_eq!(named("Kitchen Ceiling", "Kitchen", "light.a"), "Ceiling");
        assert_eq!(named("kitchen: ceiling spots", "Kitchen", "light.a"), "Ceiling spots");
        assert_eq!(named("Kitchen - iPad dock", "Kitchen", "switch.a"), "IPad dock");
        assert_eq!(named("Kitchenette Lamp", "Kitchen", "light.a"), "Kitchenette Lamp");
        assert_eq!(named("Lamp in the Kitchen", "Kitchen", "light.a"), "Lamp in the Kitchen");
        assert_eq!(named("Büro & Café Lampe", "Büro & Café", "light.a"), "Lampe");
        // A name that is only the room's becomes the kind's own word.
        for (id, noun) in [("light.a", "Light"), ("cover.a", "Blind"), ("climate.a", "Thermostat"), ("fan.a", "Fan"), ("media_player.a", "Speaker"), ("switch.a", "Switch"), ("lock.a", "Lock"), ("vacuum.a", "Vacuum")] {
            assert_eq!(named("Kitchen", "Kitchen", id), noun);
        }
        let tv = one(r#"{"entity_id":"media_player.den","state":"on","attributes":{"device_class":"tv","friendly_name":"Den"}}"#);
        assert_eq!((short_name("Den", Some("Den"), &tv), glyph(&tv)), ("TV".to_string(), "tv"));
        assert_eq!(short_name("Den", Some("Den"), &thing("light.living_room")), "Lights");
        assert_eq!(short_name("Den Lamp", None, &tv), "Den Lamp");
        // Names are tidied wherever they are shown.
        assert_eq!(tidy_name("  Front  door\tDoor "), "Front door");
        assert_eq!(tidy_name("Hall\u{7}\nlamp"), "Hall lamp");
        assert_eq!(tidy_name("Door"), "Door");
        assert_eq!(one(r#"{"entity_id":"light.bed_light","state":"on","attributes":{"friendly_name":"  "}}"#).name(), "Bed light");
        assert_eq!(one(r#"{"entity_id":"light.bed_light","state":"on"}"#).name(), "Bed light");
        assert_eq!((worded("heat_cool"), worded("fan_only"), worded("auto_low"), worded("eco")), ("Heat and cool".into(), "Fan only".into(), "Auto low".into(), "Eco".into()));
        assert_eq!((counted(1, "light"), counted(0, "device"), counted(3, "lock")), ("1 light".into(), "0 devices".into(), "3 locks".into()));
        assert_eq!((listed(&["Ana".into()]), listed(&["Ana".into(), "Ben".into()]), listed(&["Ana".into(), "Ben".into(), "Dee".into()])), ("Ana".into(), "Ana and Ben".into(), "Ana, Ben, Dee".into()));
        // What lets people or cars through is told from what only shades a window.
        let garage_like = |id: &str, attributes: &str| one(&format!(r#"{{"entity_id":"{id}","state":"closed","attributes":{attributes}}}"#)).garage_like();
        assert!(garage_like("cover.a", r#"{"device_class":"garage"}"#) && garage_like("cover.a", r#"{"device_class":"gate"}"#) && garage_like("cover.a", r#"{"device_class":"door"}"#));
        assert!(garage_like("cover.side_gate", "{}") && garage_like("cover.a", r#"{"friendly_name":"Garage"}"#));
        assert!(!garage_like("cover.a", r#"{"device_class":"blind"}"#) && !garage_like("cover.kitchen_window", "{}") && !garage_like("cover.garage_window", r#"{"device_class":"shade"}"#));
    }

    #[test]
    fn lengths_of_time_are_words() {
        assert_eq!((span(0), span(59_000), span(12 * 60_000), span(60 * 60_000), span(5 * 3_600_000), span(3 * 86_400_000)), ("1 minute".into(), "1 minute".into(), "12 minutes".into(), "1 hour".into(), "5 hours".into(), "3 days".into()));
        let home = home_in(None);
        assert_eq!(home.how_long(home.find("binary_sensor.front_door").unwrap()).as_deref(), Some("For 12 minutes"));
        assert_eq!(home.how_long(home.find("lock.kitchen_door").unwrap()).as_deref(), Some("For 2 hours"));
        // Without the moment it changed, or with a clock that is behind it, nothing is said.
        assert_eq!(home.how_long(home.find("lock.front_door").unwrap()), None);
        let early = Home::build(states(), &registry(), None, &Options::default(), now() - 86_400_000);
        assert_eq!(early.how_long(early.find("binary_sensor.front_door").unwrap()), None);
    }

    // Presses

    #[test]
    fn presses_are_read_strictly() {
        assert_eq!(Press::parse("toggle:light.bed_light"), Some(Press::Toggle("light.bed_light")));
        assert_eq!(Press::parse("step:light.bed_light:brightness:+10"), Some(Press::Step { entity: "light.bed_light", what: "brightness", by: 10.0 }));
        assert_eq!(Press::parse("step:media_player.tv:volume:-5"), Some(Press::Step { entity: "media_player.tv", what: "volume", by: -5.0 }));
        assert_eq!(Press::parse("set:climate.hvac:temperature:22.0"), Some(Press::Set { entity: "climate.hvac", what: "temperature", value: "22.0" }));
        // A value keeps its colons and spaces.
        assert_eq!(Press::parse("set:media_player.tv:source:HDMI 2: Xbox"), Some(Press::Set { entity: "media_player.tv", what: "source", value: "HDMI 2: Xbox" }));
        assert_eq!(Press::parse("cover:cover.garage_door:open"), Some(Press::Cover { entity: "cover.garage_door", verb: "open" }));
        assert_eq!(Press::parse("all_off:"), Some(Press::AllOff("")));
        assert_eq!(Press::parse("all_on:living_room"), Some(Press::AllOn("living_room")));
        assert_eq!(Press::parse("camera:camera.porch"), Some(Press::Camera("camera.porch")));
        let refused = [
            "",
            "toggle",
            "toggle:",
            "toggle:light",
            "toggle:light.",
            "toggle:.bed",
            "toggle:light.bed light",
            "toggle:light.Bed",
            "toggle:light.bed/../../config",
            "toggle:light.bed?x=1",
            "toggle:light.bed#x",
            "toggle:light.bed.light",
            "toggle:climate.hvac",
            "toggle:automation.morning",
            "scene:light.bed_light",
            "script:scene.movie_time",
            "unlock:cover.garage_door",
            "lock:light.bed_light",
            "camera:light.bed_light",
            "step:light.bed_light:brightness",
            "step:light.bed_light:brightness:lots",
            "step:light.bed_light:brightness:inf",
            "step:light.bed_light:brightness:5000",
            "set:climate.hvac:temperature:",
            "set:climate.hvac::22",
            "set:homeassistant.core:restart:now",
            "cover:cover.garage_door:smash",
            "cover:lock.front_door:open",
            "media:media_player.tv:eject",
            "vacuum:vacuum.robo:explode",
            "alarm:alarm_control_panel.home:trigger",
            "all_on:",
            "all_off:Living Room",
            "all_off:../kitchen",
            "here:Living Room",
            "restart:homeassistant.core",
            "movie:12",
        ];
        for action in refused {
            assert_eq!(Press::parse(action), None, "{action}");
        }
        // A press that reads but does not suit the thing makes no call.
        let bed = one(r#"{"entity_id":"light.bed_light","state":"on"}"#);
        for unsuited in ["set:light.bed_light:temperature:22.0", "set:light.bed_light:colour:red", "set:light.bed_light:colour:FF00", "set:light.bed_light:kelvin:99999", "step:light.bed_light:volume:+5", "set:light.bed_light:source:dvd"] {
            assert_eq!(plan(&Press::parse(unsuited).unwrap(), Some(&bed)), None, "{unsuited}");
        }
        assert_eq!(plan(&Press::parse("step:switch.ac:brightness:+10").unwrap(), None), None);
        // Sheets are named as strictly.
        let home = home_in(None);
        for id in ["room:Living Room", "room:../x", "dev:light", "dev:light.nope", "dev:light.bed/..", "src:nothing", "attic", ""] {
            assert!(home.sheet(id).is_none(), "{id}");
        }
        // Anything unknown is said in the words of a tile that has aged.
        assert_eq!(<HomeAssistant as sdk::Plugin>::resolve("restart:homeassistant.core").err().as_deref(), Some("This tile is out of date. Refresh the plugin, then try again."));
    }

    #[test]
    fn each_press_is_one_service_call() {
        let call = |action: &str, entity: &str| {
            let call = plan(&Press::parse(action).unwrap_or_else(|| panic!("{action} does not read")), Some(&thing(entity))).unwrap_or_else(|| panic!("{action} makes no call"));
            (call.service, call.data)
        };
        let made = |service: &str, data: Value| (service.to_string(), data);
        assert_eq!(call("toggle:light.ceiling_lights", "light.ceiling_lights"), made("light/turn_off", json!({"entity_id": "light.ceiling_lights"})));
        assert_eq!(call("toggle:light.kitchen_lights", "light.kitchen_lights"), made("light/turn_on", json!({"entity_id": "light.kitchen_lights"})));
        assert_eq!(call("toggle:switch.decorative_lights", "switch.decorative_lights"), made("switch/turn_off", json!({"entity_id": "switch.decorative_lights"})));
        assert_eq!(call("toggle:fan.living_room_fan", "fan.living_room_fan"), made("fan/turn_on", json!({"entity_id": "fan.living_room_fan"})));
        assert_eq!(call("scene:scene.movie_time", "scene.movie_time"), made("scene/turn_on", json!({"entity_id": "scene.movie_time"})));
        assert_eq!(call("script:script.welcome_home", "script.welcome_home"), made("script/turn_on", json!({"entity_id": "script.welcome_home"})));
        assert_eq!(call("step:media_player.living_room:volume:+5", "media_player.living_room"), made("media_player/volume_set", json!({"entity_id": "media_player.living_room", "volume_level": 0.45})));
        assert_eq!(call("step:cover.living_room_window:position:-10", "cover.living_room_window"), made("cover/set_cover_position", json!({"entity_id": "cover.living_room_window", "position": 60})));
        assert_eq!(call("step:cover.living_room_window:tilt:+10", "cover.living_room_window"), made("cover/set_cover_tilt_position", json!({"entity_id": "cover.living_room_window", "tilt_position": 60})));
        assert_eq!(call("step:fan.living_room_fan:speed:+1", "fan.living_room_fan"), made("fan/set_percentage", json!({"entity_id": "fan.living_room_fan", "percentage": 33})));
        assert_eq!(call("set:climate.heatpump:temperature:21.5", "climate.heatpump"), made("climate/set_temperature", json!({"entity_id": "climate.heatpump", "temperature": 21.5})));
        // One end of a range goes with the other end as it stands.
        assert_eq!(call("set:climate.ecobee:low:20.5", "climate.ecobee"), made("climate/set_temperature", json!({"entity_id": "climate.ecobee", "target_temp_low": 20.5, "target_temp_high": 24.0})));
        assert_eq!(call("set:climate.ecobee:high:25.0", "climate.ecobee"), made("climate/set_temperature", json!({"entity_id": "climate.ecobee", "target_temp_low": 21.0, "target_temp_high": 25.0})));
        assert_eq!(call("set:climate.ecobee:mode:fan_only", "climate.ecobee"), made("climate/set_hvac_mode", json!({"entity_id": "climate.ecobee", "hvac_mode": "fan_only"})));
        assert_eq!(call("set:climate.ecobee:preset:eco", "climate.ecobee"), made("climate/set_preset_mode", json!({"entity_id": "climate.ecobee", "preset_mode": "eco"})));
        assert_eq!(call("set:climate.ecobee:fan:on_low", "climate.ecobee"), made("climate/set_fan_mode", json!({"entity_id": "climate.ecobee", "fan_mode": "on_low"})));
        assert_eq!(call("set:fan.living_room_fan:preset:sleep", "fan.living_room_fan"), made("fan/set_preset_mode", json!({"entity_id": "fan.living_room_fan", "preset_mode": "sleep"})));
        assert_eq!(call("set:fan.living_room_fan:oscillating:on", "fan.living_room_fan"), made("fan/oscillate", json!({"entity_id": "fan.living_room_fan", "oscillating": true})));
        assert_eq!(call("set:light.ceiling_lights:kelvin:2700", "light.ceiling_lights"), made("light/turn_on", json!({"entity_id": "light.ceiling_lights", "color_temp_kelvin": 2700})));
        assert_eq!(call("set:light.ceiling_lights:colour:FF8000", "light.ceiling_lights"), made("light/turn_on", json!({"entity_id": "light.ceiling_lights", "rgb_color": [255, 128, 0]})));
        assert_eq!(call("set:media_player.living_room:source:HDMI 2: Xbox", "media_player.living_room"), made("media_player/select_source", json!({"entity_id": "media_player.living_room", "source": "HDMI 2: Xbox"})));
        assert_eq!(call("set:vacuum.robo:fan:max", "vacuum.robo"), made("vacuum/set_fan_speed", json!({"entity_id": "vacuum.robo", "fan_speed": "max"})));
        assert_eq!(call("cover:cover.living_room_window:toggle", "cover.living_room_window"), made("cover/close_cover", json!({"entity_id": "cover.living_room_window"})));
        assert_eq!(call("cover:cover.garage_door:toggle", "cover.garage_door"), made("cover/open_cover", json!({"entity_id": "cover.garage_door"})));
        assert_eq!(call("cover:cover.living_room_window:stop", "cover.living_room_window"), made("cover/stop_cover", json!({"entity_id": "cover.living_room_window"})));
        assert_eq!(call("lock:lock.kitchen_door", "lock.kitchen_door"), made("lock/lock", json!({"entity_id": "lock.kitchen_door"})));
        assert_eq!(call("unlock:lock.front_door", "lock.front_door"), made("lock/unlock", json!({"entity_id": "lock.front_door"})));
        assert_eq!(call("media:media_player.living_room:play_pause", "media_player.living_room"), made("media_player/media_pause", json!({"entity_id": "media_player.living_room"})));
        assert_eq!(call("media:media_player.living_room:next", "media_player.living_room"), made("media_player/media_next_track", json!({"entity_id": "media_player.living_room"})));
        assert_eq!(call("media:media_player.living_room:previous", "media_player.living_room"), made("media_player/media_previous_track", json!({"entity_id": "media_player.living_room"})));
        assert_eq!(call("media:media_player.living_room:mute", "media_player.living_room"), made("media_player/volume_mute", json!({"entity_id": "media_player.living_room", "is_volume_muted": true})));
        assert_eq!(call("media:media_player.living_room:off", "media_player.living_room"), made("media_player/turn_off", json!({"entity_id": "media_player.living_room"})));
        assert_eq!(call("vacuum:vacuum.robo:start", "vacuum.robo"), made("vacuum/start", json!({"entity_id": "vacuum.robo"})));
        assert_eq!(call("vacuum:vacuum.robo:pause", "vacuum.robo"), made("vacuum/pause", json!({"entity_id": "vacuum.robo"})));
        assert_eq!(call("vacuum:vacuum.robo:dock", "vacuum.robo"), made("vacuum/return_to_base", json!({"entity_id": "vacuum.robo"})));
        assert_eq!(call("vacuum:vacuum.robo:locate", "vacuum.robo"), made("vacuum/locate", json!({"entity_id": "vacuum.robo"})));
        assert_eq!(call("alarm:alarm_control_panel.house_alarm:home", "alarm_control_panel.house_alarm"), made("alarm_control_panel/alarm_arm_home", json!({"entity_id": "alarm_control_panel.house_alarm"})));
        assert_eq!(call("alarm:alarm_control_panel.house_alarm:away", "alarm_control_panel.house_alarm"), made("alarm_control_panel/alarm_arm_away", json!({"entity_id": "alarm_control_panel.house_alarm"})));
        assert_eq!(call("alarm:alarm_control_panel.house_alarm:night", "alarm_control_panel.house_alarm"), made("alarm_control_panel/alarm_arm_night", json!({"entity_id": "alarm_control_panel.house_alarm"})));
        assert_eq!(call("alarm:alarm_control_panel.house_alarm:disarm", "alarm_control_panel.house_alarm"), made("alarm_control_panel/alarm_disarm", json!({"entity_id": "alarm_control_panel.house_alarm"})));
        // Things the fixture's home has none of.
        let other = |action: &str, state: &str| {
            let call = plan(&Press::parse(action).unwrap(), Some(&one(state))).unwrap();
            (call.service, call.data)
        };
        assert_eq!(other("toggle:humidifier.a", r#"{"entity_id":"humidifier.a","state":"on"}"#), made("humidifier/turn_off", json!({"entity_id": "humidifier.a"})));
        assert_eq!(other("toggle:input_boolean.a", r#"{"entity_id":"input_boolean.a","state":"off"}"#), made("input_boolean/turn_on", json!({"entity_id": "input_boolean.a"})));
        assert_eq!(other("set:humidifier.a:humidity:55", r#"{"entity_id":"humidifier.a","state":"on"}"#), made("humidifier/set_humidity", json!({"entity_id": "humidifier.a", "humidity": 55})));
        assert_eq!(other("set:humidifier.a:mode:eco", r#"{"entity_id":"humidifier.a","state":"on"}"#), made("humidifier/set_mode", json!({"entity_id": "humidifier.a", "mode": "eco"})));
        assert_eq!(other("set:water_heater.a:temperature:55.0", r#"{"entity_id":"water_heater.a","state":"eco"}"#), made("water_heater/set_temperature", json!({"entity_id": "water_heater.a", "temperature": 55.0})));
        assert_eq!(other("set:water_heater.a:mode:heat_pump", r#"{"entity_id":"water_heater.a","state":"eco"}"#), made("water_heater/set_operation_mode", json!({"entity_id": "water_heater.a", "operation_mode": "heat_pump"})));
        assert_eq!(other("set:water_heater.a:away:on", r#"{"entity_id":"water_heater.a","state":"eco"}"#), made("water_heater/set_away_mode", json!({"entity_id": "water_heater.a", "away_mode": true})));
        assert_eq!(other("media:media_player.a:play_pause", r#"{"entity_id":"media_player.a","state":"paused"}"#), made("media_player/media_play", json!({"entity_id": "media_player.a"})));
        assert_eq!(other("media:media_player.a:play_pause", r#"{"entity_id":"media_player.a","state":"off"}"#), made("media_player/turn_on", json!({"entity_id": "media_player.a"})));
        assert_eq!(other("cover:cover.a:toggle", r#"{"entity_id":"cover.a","state":"opening"}"#), made("cover/stop_cover", json!({"entity_id": "cover.a"})));
        // A level is kept inside what the thing can take.
        assert_eq!(other("step:media_player.a:volume:+5", r#"{"entity_id":"media_player.a","state":"playing","attributes":{"volume_level":0.98}}"#).1["volume_level"], json!(1.0));
        assert_eq!(other("step:cover.a:position:-10", r#"{"entity_id":"cover.a","state":"open","attributes":{"current_position":4}}"#).1["position"], json!(0));
        let fan = |state: &str, speed: &str, by: &str| other(&format!("step:fan.a:speed:{by}"), &format!(r#"{{"entity_id":"fan.a","state":"{state}","attributes":{{"percentage":{speed},"percentage_step":33.333333333333336}}}}"#)).1["percentage"].clone();
        assert_eq!((fan("on", "33", "+1"), fan("on", "66", "+1"), fan("on", "100", "+1"), fan("on", "33", "-1"), fan("off", "0", "-1")), (json!(67), json!(100), json!(100), json!(0), json!(0)));
        // Dimming asks Home Assistant to step, and needs no reading of the lamp first.
        let dim = |action: &str| plan(&Press::parse(action).unwrap(), None).map(|c| (c.service, c.data, c.say));
        assert_eq!(dim("step:light.bed_light:brightness:+10"), Some(("light/turn_on".to_string(), json!({"entity_id": "light.bed_light", "brightness_step_pct": 10}), None)));
        assert_eq!(dim("step:light.bed_light:brightness:-10").map(|c| c.1["brightness_step_pct"].clone()), Some(json!(-10)));
        assert!(!Press::parse("step:light.bed_light:brightness:-10").unwrap().reads_first() && Press::parse("step:cover.a:position:-10").unwrap().reads_first());
        // Every other press reads the thing first, and a press on nothing in particular has nothing to read.
        assert_eq!(Press::parse("toggle:light.bed_light").map(|p| (p.entity(), p.reads_first())), Some((Some("light.bed_light"), true)));
        assert_eq!(Press::parse("all_off:kitchen").map(|p| p.entity()), Some(None));
        assert_eq!(dim("toggle:light.bed_light"), None);
        // The call goes to the server's own service address, as JSON with the token in its header.
        let server = Server::open(BASE, Some(TOKEN), None).unwrap();
        let request = server.request("POST", "/api/services/light/turn_on", Some(json!({"entity_id": "light.bed_light"}).to_string()), PRESS_TIMEOUT_MS);
        assert_eq!((request.method.as_str(), request.url.as_str(), request.timeout_ms), ("POST", "http://192.168.0.20:8123/api/services/light/turn_on", 9_000));
        assert_eq!(request.headers.get("Content-Type").map(String::as_str), Some("application/json"));
        assert_eq!(request.body.as_deref(), Some(r#"{"entity_id":"light.bed_light"}"#));
    }

    #[test]
    fn all_the_lights_of_a_room_go_in_one_call_that_names_them() {
        let home = home_in(Some("living_room"));
        let off = home.all_lights("living_room", false).unwrap();
        assert_eq!((off.service.as_str(), off.data, off.say.as_deref()), ("light/turn_off", json!({"entity_id": ["light.living_room", "light.ceiling_lights", "light.living_room_lamp"]}), Some("Living Room lights off")));
        let on = home.all_lights("kitchen", true).unwrap();
        assert_eq!((on.service.as_str(), on.data, on.say.as_deref()), ("light/turn_on", json!({"entity_id": ["light.kitchen_lights"]}), Some("Kitchen lights on")));
        // The whole home is every light the plugin shows, less the one that does not answer.
        let all = home.all_lights("", false).unwrap();
        assert_eq!((all.service.as_str(), all.data, all.say.as_deref()), ("light/turn_off", json!({"entity_id": ["light.living_room", "light.kitchen_lights", "light.ceiling_lights", "light.living_room_lamp"]}), Some("2 lights off")));
        // A room with no lights, or no such room, makes no call.
        assert_eq!((home.all_lights("garage", false), home.all_lights("cellar", false)), (None, None));
        // A label keeps the call to the lights that carry it.
        let labelled = Home::build(states(), &registry(), None, &Options { label: Some("TV".into()), scripts: false }, now());
        assert_eq!(labelled.all_lights("", false).unwrap().data, json!({"entity_id": ["light.ceiling_lights"]}));
    }

    #[test]
    fn a_press_says_what_it_did() {
        let said = |action: &str, entity: &str| plan(&Press::parse(action).unwrap(), Some(&thing(entity))).unwrap().say.unwrap_or_default();
        assert_eq!(said("toggle:light.ceiling_lights", "light.ceiling_lights"), "Ceiling Lights off");
        assert_eq!(said("toggle:light.kitchen_lights", "light.kitchen_lights"), "Kitchen Lights on");
        assert_eq!(said("scene:scene.movie_time", "scene.movie_time"), "Movie time set");
        assert_eq!(said("script:script.welcome_home", "script.welcome_home"), "Welcome home started");
        assert_eq!(said("cover:cover.living_room_window:toggle", "cover.living_room_window"), "Living Room Window closing");
        assert_eq!(said("cover:cover.garage_door:open", "cover.garage_door"), "Garage Door opening");
        assert_eq!(said("cover:cover.garage_door:stop", "cover.garage_door"), "Garage Door stopped");
        assert_eq!(said("lock:lock.kitchen_door", "lock.kitchen_door"), "Kitchen Door locked");
        assert_eq!(said("unlock:lock.front_door", "lock.front_door"), "Front Door unlocked");
        assert_eq!(said("media:media_player.living_room:play_pause", "media_player.living_room"), "Living Room paused");
        assert_eq!(said("media:media_player.living_room:off", "media_player.living_room"), "Living Room off");
        assert_eq!(said("vacuum:vacuum.robo:start", "vacuum.robo"), "Robo cleaning");
        assert_eq!(said("vacuum:vacuum.robo:dock", "vacuum.robo"), "Robo going home");
        assert_eq!(said("alarm:alarm_control_panel.house_alarm:away", "alarm_control_panel.house_alarm"), "House Alarm armed");
        assert_eq!(said("alarm:alarm_control_panel.house_alarm:disarm", "alarm_control_panel.house_alarm"), "House Alarm disarmed");
        // A step or a setting shows on its row, and a held key would say it over and over.
        for (quiet, entity) in [
            ("step:media_player.living_room:volume:+5", "media_player.living_room"),
            ("step:cover.living_room_window:position:+10", "cover.living_room_window"),
            ("set:climate.heatpump:temperature:21.5", "climate.heatpump"),
            ("set:climate.ecobee:mode:auto", "climate.ecobee"),
            ("set:light.ceiling_lights:kelvin:2700", "light.ceiling_lights"),
            ("media:media_player.living_room:next", "media_player.living_room"),
            ("media:media_player.living_room:mute", "media_player.living_room"),
            ("vacuum:vacuum.robo:locate", "vacuum.robo"),
        ] {
            assert_eq!(said(quiet, entity), "", "{quiet}");
        }
        // Choosing a room is said by the room's name, which the TV keeps and needs no server for.
        let registry = registry();
        assert_eq!(choose_room(&registry, "buro_cafe").and_then(|a| a.say), Some("This TV is in Büro & Café".to_string()));
        assert_eq!(choose_room(&registry, "").and_then(|a| a.say), Some("This TV has no room".to_string()));
        assert_eq!(choose_room(&registry, "cellar"), None);
        assert!(choose_room(&registry, "attic").is_some_and(|a| a.did_the_work()));
    }

    #[test]
    fn a_lock_never_unlocks_on_one_press() {
        let home = home_in(Some("hall"));
        let items = home.items(BASE);
        let tile = |id: &str| items.iter().find(|i| i.id == id).map(|i| (does(i), i.on, i.subtitle.clone().unwrap_or_default())).unwrap();
        // Locked, its tile opens the sheet, where unlocking asks first.
        assert_eq!(tile("e:lock.front_door"), ("opens dev:lock.front_door".to_string(), Some(false), "Locked".to_string()));
        assert_eq!(rows(&home.sheet("dev:lock.front_door").unwrap()), ["Confirm | Unlock |  | ok unlock:lock.front_door, asks Yes, unlock Front Door"]);
        assert_eq!(home.sheet("dev:lock.front_door").unwrap().rows[0].kind, SheetRowKind::Confirm);
        // Unlocked, one press locks it.
        let kitchen = home_in(Some("kitchen"));
        let unlocked = kitchen.items(BASE).into_iter().find(|i| i.id == "e:lock.kitchen_door").unwrap();
        assert_eq!((does(&unlocked), unlocked.on, unlocked.more.as_deref()), ("lock:lock.kitchen_door".to_string(), Some(true), Some("dev:lock.kitchen_door")));
        assert_eq!(rows(&kitchen.sheet("dev:lock.kitchen_door").unwrap()), ["Press | Lock |  | ok lock:lock.kitchen_door"]);
        // A press that could go either way only ever locks, whatever the lock says it is.
        for state in ["locked", "unlocked", "jammed", "unlocking", "unknown"] {
            let lock = one(&format!(r#"{{"entity_id":"lock.front_door","state":"{state}","attributes":{{"friendly_name":"Front Door"}}}}"#));
            assert_eq!(plan(&Press::Toggle("lock.front_door"), Some(&lock)).map(|c| c.service).as_deref(), Some("lock/lock"), "{state}");
            let (action, _) = press_of(&lock);
            assert!(!action.resolve.as_deref().unwrap_or("").starts_with("unlock"), "{state}");
            // Only the row that asks first carries the unlocking.
            let lists = [home.row(&Thing { e: lock.clone(), room: None, name: "Front Door".into(), extra: false, phone: false }, "Front Door")];
            assert!(lists.iter().all(|r| r.resolve.as_deref() != Some("unlock:lock.front_door") || r.kind == SheetRowKind::Confirm), "{state}");
        }
        // One that jammed can be tried either way.
        let jammed = home_after(Some("hall"), &[("lock.front_door", "jammed")]);
        assert_eq!(rows(&jammed.sheet("dev:lock.front_door").unwrap()), ["Press | Lock |  | ok lock:lock.front_door", "Confirm | Unlock |  | ok unlock:lock.front_door, asks Yes, unlock Front Door"]);
        // A lock that wants its code cannot be worked from a remote at all.
        assert_eq!(tile("e:lock.front_door").0, "opens dev:lock.front_door");
        let garage = home_in(Some("garage"));
        let cellar = garage.items(BASE).into_iter().find(|i| i.id == "e:lock.cellar_door").unwrap();
        assert_eq!(does(&cellar), "opens dev:lock.cellar_door");
        assert_eq!(rows(&garage.sheet("dev:lock.cellar_door").unwrap()), ["Text | Needs its code, so unlock it in Home Assistant"]);
        let open = home_after(Some("garage"), &[("lock.cellar_door", "unlocked")]);
        assert_eq!(does(&open.items(BASE).into_iter().find(|i| i.id == "e:lock.cellar_door").unwrap()), "opens dev:lock.cellar_door");
        assert_eq!(rows(&open.sheet("dev:lock.cellar_door").unwrap()), ["Text | Needs its code, so lock it in Home Assistant"]);
        // In the list of locks, the locked ones ask and the unlocked one locks.
        assert_eq!(
            rows(&home.sheet("locks").unwrap()),
            [
                "Text | Cellar Door | Locked",
                "Confirm | Front Door | Locked | ok unlock:lock.front_door, asks Yes, unlock Front Door",
                "Press | Kitchen Door | Unlocked | ok lock:lock.kitchen_door",
            ]
        );
        // An alarm that needs no code disarms after asking, and arms in one press.
        assert_eq!(rows(&home.sheet("dev:alarm_control_panel.house_alarm").unwrap()), ["Press | Arm home |  | ok alarm:alarm_control_panel.house_alarm:home", "Press | Arm away |  | ok alarm:alarm_control_panel.house_alarm:away", "Press | Arm night |  | ok alarm:alarm_control_panel.house_alarm:night"]);
        let armed = home_after(Some("hall"), &[("alarm_control_panel.house_alarm", "armed_night")]);
        assert_eq!(rows(&armed.sheet("dev:alarm_control_panel.house_alarm").unwrap()), ["Confirm | Disarm |  | ok alarm:alarm_control_panel.house_alarm:disarm, asks Yes, disarm House Alarm"]);
        // One that wants a code is read and not worked.
        let coded = |state: &str| {
            let alarm = one(&format!(r#"{{"entity_id":"alarm_control_panel.security","state":"{state}","attributes":{{"code_format":"number","code_arm_required":true,"friendly_name":"Security","supported_features":63}}}}"#));
            rows(&Home::build(vec![alarm], &Registry::default(), None, &Options::default(), now()).sheet("dev:alarm_control_panel.security").unwrap())
        };
        assert_eq!(coded("disarmed"), ["Text | Needs its code, so arm it in Home Assistant"]);
        assert_eq!(coded("armed_away"), ["Text | Needs its code, so disarm it in Home Assistant"]);
    }

    #[test]
    fn a_garage_door_asks_before_it_moves() {
        let home = home_in(Some("garage"));
        let items = home.items(BASE);
        let door = items.iter().find(|i| i.id == "e:cover.garage_door").unwrap();
        assert_eq!((does(door).as_str(), door.glyph.as_deref(), door.on, door.subtitle.as_deref(), door.more.as_deref()), ("opens dev:cover.garage_door", Some("garage"), Some(false), Some("Closed"), None));
        let sheet = home.sheet("dev:cover.garage_door").unwrap();
        assert_eq!((sheet.title.as_str(), sheet.subtitle.as_deref()), ("Garage Door", Some("Garage, closed")));
        assert_eq!(rows(&sheet), ["Confirm | Open |  | ok cover:cover.garage_door:open, asks Yes, open Garage Door"]);
        let open = home_after(Some("garage"), &[("cover.garage_door", "open")]);
        assert_eq!(rows(&open.sheet("dev:cover.garage_door").unwrap()), ["Confirm | Close |  | ok cover:cover.garage_door:close, asks Yes, close Garage Door"]);
        // While it moves the one thing to do is stop it, which needs no asking.
        let moving = home_after(Some("garage"), &[("cover.garage_door", "closing")]);
        assert_eq!(rows(&moving.sheet("dev:cover.garage_door").unwrap()), ["Press | Stop | Closing | ok cover:cover.garage_door:stop"]);
        // In its room's sheet it is a row that opens that sheet, never one that moves it.
        assert_eq!(rows(&home.sheet("room:garage").unwrap())[..4], ["Group | Blinds and doors", "Open | Door | Closed | opens dev:cover.garage_door", "Group | Locks", "Text | Cellar Door | Locked"]);
        // A blind is not a door: its tile moves it, and its sheet offers each way.
        let window = home_in(Some("living_room")).sheet("dev:cover.living_room_window").unwrap();
        assert_eq!(
            rows(&window),
            [
                "Step | Position | 70% | ok cover:cover.living_room_window:toggle, left step:cover.living_room_window:position:-10, right step:cover.living_room_window:position:+10",
                "Press | Open |  | ok cover:cover.living_room_window:open",
                "Press | Close |  | ok cover:cover.living_room_window:close",
                "Press | Stop |  | ok cover:cover.living_room_window:stop",
                "Step | Tilt | 50% | left step:cover.living_room_window:tilt:-10, right step:cover.living_room_window:tilt:+10",
            ]
        );
    }

    #[test]
    fn changed_states_from_a_press_update_what_is_kept() {
        let mut snap = Snap::default().renewed(states(), now());
        let at = |snap: &Snap, id: &str| snap.entities.iter().find(|e| e.id == id).map(|e| (e.state.clone(), e.a.brightness, e.updated_ms));
        let three = sdk::epoch_ms(2026, 11, 20, 15, 0, 0, 0);
        assert_eq!(at(&snap, "light.ceiling_lights"), Some(("on".to_string(), Some(180.0), Some(three))));
        // What a call answers with is the state it left behind.
        let changed = read_states(r#"[{"entity_id":"light.ceiling_lights","state":"off","attributes":{"supported_color_modes":["color_temp","hs"],"brightness":null,"friendly_name":"Ceiling Lights"},"last_changed":"2026-11-20T16:03:01+00:00","last_updated":"2026-11-20T16:03:01+00:00"}]"#).unwrap();
        snap.patch(changed);
        assert_eq!(at(&snap, "light.ceiling_lights"), Some(("off".to_string(), None, Some(now() + 1000))));
        assert_eq!(snap.entities.len(), 41);
        // An older reading does not undo it, as when a refresh read the states just before the press.
        let stale = snap.clone().renewed(states(), now() + 2000);
        assert_eq!(at(&stale, "light.ceiling_lights"), Some(("off".to_string(), None, Some(now() + 1000))));
        assert_eq!(stale.at_ms, now() + 2000);
        snap.patch(vec![thing("light.ceiling_lights")]);
        assert_eq!(at(&snap, "light.ceiling_lights").map(|s| s.0), Some("off".to_string()));
        // A newer one does, and a thing Home Assistant no longer lists goes.
        let later = one(r#"{"entity_id":"light.ceiling_lights","state":"on","attributes":{"brightness":26},"last_updated":"2026-11-20T16:05:00+00:00"}"#);
        let fewer: Vec<Entity> = vec![later, thing("light.kitchen_lights")];
        let renewed = snap.clone().renewed(fewer, now() + 120_000);
        assert_eq!(renewed.entities.iter().map(|e| (e.id.as_str(), e.state.as_str())).collect::<Vec<_>>(), [("light.ceiling_lights", "on"), ("light.kitchen_lights", "off")]);
        // Something read for the first time is taken in.
        snap.patch(vec![one(r#"{"entity_id":"light.new_lamp","state":"on"}"#)]);
        assert_eq!((snap.entities.len(), at(&snap, "light.new_lamp").map(|s| s.0)), (42, Some("on".to_string())));
        // A call's answer may be empty, or not a list of states at all.
        assert!(read_states("[]").unwrap().is_empty() && read_states(r#"{"message":"ok"}"#).is_err());
        // A press writes down only what it found, and the kept home takes that in when it is read.
        let kept = Snap::default().renewed(states(), now());
        let off = one(r#"{"entity_id":"light.ceiling_lights","state":"off","last_updated":"2026-11-20T16:03:05+00:00"}"#);
        let found = Recent::default().with(vec![off.clone()], false, now() + 5000);
        assert_eq!((found.at_ms, found.unsure, found.entities.len()), (now() + 5000, false, 1));
        let read = kept.clone().with(found.clone());
        assert_eq!((at(&read, "light.ceiling_lights").map(|s| s.0), read.at_ms, read.entities.len()), (Some("off".to_string()), now(), 41));
        // A device that has not said what became of it makes the kept home count as old.
        let unsure = found.clone().with(vec![thing("light.kitchen_lights")], true, now() + 6000);
        assert_eq!((unsure.entities.len(), unsure.unsure, kept.clone().with(unsure.clone()).at_ms), (2, true, 0));
        // Once the home has been read again, what was found before that reading says nothing new.
        let later = kept.clone().renewed(states(), now() + 30_000);
        assert_eq!((later.clone().with(unsure.clone()).at_ms, at(&later.clone().with(unsure.clone()), "light.ceiling_lights").map(|s| s.0)), (now() + 30_000, Some("on".to_string())));
        // What was found long ago is dropped when the next press writes, and it comes back as it went.
        let again = unsure.clone().with(vec![off], false, now() + 10 * 60_000);
        assert_eq!((again.entities.len(), again.unsure), (1, false));
        assert_eq!(serde_json::from_slice::<Recent>(&unsure.kept()).unwrap(), unsure);
        // More than one stored value holds is not kept, and the next sheet reads the home again.
        let flood = Recent::default().with((0..3000).map(|n| one(&format!(r#"{{"entity_id":"light.l{n}","state":"off","attributes":{{"friendly_name":"A light with quite a long name, number {n}"}}}}"#))).collect(), false, 9);
        assert_eq!(serde_json::from_slice::<Recent>(&flood.kept()).unwrap(), Recent { at_ms: 9, unsure: true, entities: Vec::new() });
    }

    // Sheets

    #[test]
    fn the_room_sheet_lists_what_can_be_done_there() {
        let home = home_in(Some("living_room"));
        let sheet = home.sheet("room:living_room").unwrap();
        assert_eq!((sheet.title.as_str(), sheet.subtitle.as_deref(), sheet.footnote.as_deref()), ("Living Room", Some("2 lights on, 19.5°"), None));
        assert_eq!(
            rows(&sheet),
            [
                "Press | All lights | 2 on | ok all_off:living_room",
                "Group | Scenes",
                "Press | Movie time |  | ok scene:scene.movie_time",
                "Press | Relax |  | ok scene:scene.living_room_relax",
                "Group | Lights",
                "Step | Lights | 65% | ok toggle:light.living_room, left step:light.living_room:brightness:-10, right step:light.living_room:brightness:+10",
                "Step | Ceiling Lights | 71% | ok toggle:light.ceiling_lights, left step:light.ceiling_lights:brightness:-10, right step:light.ceiling_lights:brightness:+10",
                "Step | Lamp | 60% | ok toggle:light.living_room_lamp, left step:light.living_room_lamp:brightness:-10, right step:light.living_room_lamp:brightness:+10",
                "Group | Blinds and doors",
                "Step | Window | 70% open | ok cover:cover.living_room_window:toggle, left step:cover.living_room_window:position:-10, right step:cover.living_room_window:position:+10",
                "Group | Climate",
                "Open | HeatPump | 19.5°, heating to 21° | opens dev:climate.heatpump",
                "Group | Fans",
                "Step | Fan | Off | ok toggle:fan.living_room_fan, left step:fan.living_room_fan:speed:-1, right step:fan.living_room_fan:speed:+1",
                "Group | Speakers and TVs",
                "Open | Speaker | Playing | opens dev:media_player.living_room",
                "Group | Switches",
                "Press | Decorative Lights | On | ok toggle:switch.decorative_lights",
                "Group | More",
                "Press | Child lock | Off | ok toggle:switch.living_room_child_lock",
                "Group | This TV",
                "Press | This TV is in this room | On | ok here:",
            ]
        );
        assert_eq!(sheet.rows[15].subtitle.as_deref(), Some("Kind of Blue"));
        // Row ids stay put from one reading to the next, so the TV keeps its place.
        let mut ids: Vec<&str> = sheet.rows.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids[..6], ["all", "g:scenes", "scene.movie_time", "scene.living_room_relax", "g:lights", "light.living_room"]);
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), sheet.rows.len());
        assert!(sheet.rows.iter().filter_map(|r| r.glyph.as_deref()).all(|g| sdk::GLYPHS.contains(&g)));
        // Another room offers to become this TV's, and with its lights off offers to turn them on.
        let kitchen = home.sheet("room:kitchen").unwrap();
        assert_eq!(
            rows(&kitchen),
            [
                "Press | All lights | Off | ok all_on:kitchen",
                "Group | Lights",
                "Press | Lights | Off | ok toggle:light.kitchen_lights",
                "Group | Switches",
                "Press | Plug | On | ok toggle:switch.kitchen_plug",
                "Group | Locks",
                "Press | Door | Unlocked | ok lock:lock.kitchen_door",
                "Group | Vacuums",
                "Open | Robo | Docked | opens dev:vacuum.robo",
                "Group | This TV",
                "Press | This TV is in this room | Off | ok here:kitchen",
            ]
        );
        // The hall has its alarm and its camera, and a lamp that does not answer still has its row.
        assert_eq!(rows(&home.sheet("room:hall").unwrap())[..6], ["Group | Locks", "Confirm | Front Door | Locked | ok unlock:lock.front_door, asks Yes, unlock Front Door", "Group | Alarm", "Open | House Alarm | Disarmed | opens dev:alarm_control_panel.house_alarm", "Group | Cameras", "Press | Porch |  | ok camera:camera.porch"]);
        assert_eq!(rows(&home.sheet("room:buro_cafe").unwrap())[..3], ["Press | All lights | Off | ok all_on:buro_cafe", "Group | Lights", "Press | Office Lamp | Not responding | ok toggle:light.office_lamp"]);
        // What has no room is listed with how to give it one, and is no room for the TV to be in.
        let loose = home.sheet("room:").unwrap();
        assert_eq!((loose.title.as_str(), loose.subtitle.as_deref()), ("Not in a room", Some("1 device")));
        assert_eq!(rows(&loose), ["Group | Cameras", "Press | Drive |  | ok camera:camera.drive"]);
        assert!(loose.footnote.is_some_and(|note| reads_as_sentence(&note)));
        // A room with more than the TV keeps of a sheet still ends with the row for this TV.
        let mut states = states();
        states.extend((0..80).map(|n| one(&format!(r#"{{"entity_id":"switch.attic_{n:02}","state":"off","attributes":{{"friendly_name":"Attic Socket {n:02}"}}}}"#))));
        let mut registry = registry();
        registry.areas[5].e.extend((0..80).map(|n| format!("switch.attic_{n:02}")));
        let attic = Home::build(states, &registry, None, &Options::default(), now()).sheet("room:attic").unwrap();
        assert_eq!((attic.rows.len(), attic.rows[59].resolve.as_deref()), (60, Some("here:attic")));
        // The lights that are on, room by room, with one row that asks before it turns them all off.
        assert_eq!(
            rows(&home.sheet("lights").unwrap()),
            [
                "Confirm | All lights off |  | ok all_off:, asks Yes, turn off 2 lights",
                "Group | Living Room",
                "Step | Ceiling Lights | 71% | ok toggle:light.ceiling_lights, left step:light.ceiling_lights:brightness:-10, right step:light.ceiling_lights:brightness:+10",
                "Step | Lamp | 60% | ok toggle:light.living_room_lamp, left step:light.living_room_lamp:brightness:-10, right step:light.living_room_lamp:brightness:+10",
            ]
        );
        assert_eq!(home.sheet("lights").unwrap().subtitle.as_deref(), Some("2 lights in 1 room"));
    }

    #[test]
    fn a_thermostat_steps_by_its_own_step() {
        let home = home_in(Some("living_room"));
        let sheet = home.sheet("dev:climate.heatpump").unwrap();
        assert_eq!((sheet.title.as_str(), sheet.subtitle.as_deref()), ("HeatPump", Some("Living Room, 19.5°, heating to 21°")));
        // It names no step of its own, so it moves by half a degree, to a temperature and not by an amount.
        assert_eq!(
            rows(&sheet),
            [
                "Text | Now | 19.5°",
                "Step | Temperature | 21° | left set:climate.heatpump:temperature:20.5, right set:climate.heatpump:temperature:21.5",
                "Step | Mode | Heat | left set:climate.heatpump:mode:off, right set:climate.heatpump:mode:off",
            ]
        );
        let stepped = |attributes: &str| {
            let thermostat = one(&format!(r#"{{"entity_id":"climate.a","state":"heat","attributes":{attributes}}}"#));
            rows(&Home::build(vec![thermostat], &Registry::default(), None, &Options::default(), now()).sheet("dev:climate.a").unwrap())
        };
        assert_eq!(stepped(r#"{"temperature":21,"target_temp_step":1}"#), ["Step | Temperature | 21° | left set:climate.a:temperature:20.0, right set:climate.a:temperature:22.0"]);
        assert_eq!(stepped(r#"{"temperature":21.2,"target_temp_step":0.1}"#), ["Step | Temperature | 21.2° | left set:climate.a:temperature:21.1, right set:climate.a:temperature:21.3"]);
        // It stops at the ends of what the thermostat takes.
        assert_eq!(stepped(r#"{"temperature":35,"min_temp":7,"max_temp":35}"#), ["Step | Temperature | 35° | left set:climate.a:temperature:34.5, right set:climate.a:temperature:35.0"]);
        assert_eq!(stepped(r#"{"temperature":7,"min_temp":7,"max_temp":35,"target_temp_step":2}"#), ["Step | Temperature | 7° | left set:climate.a:temperature:7.0, right set:climate.a:temperature:9.0"]);
        // One that holds a range has an end to heat to and an end to cool to, and its modes come round.
        let ecobee = home.sheet("dev:climate.ecobee").unwrap();
        assert_eq!(ecobee.subtitle.as_deref(), Some("Büro & Café, 23°, 21° to 24°"));
        assert_eq!(
            rows(&ecobee),
            [
                "Text | Now | 23°",
                "Step | Heat to | 21° | left set:climate.ecobee:low:20.5, right set:climate.ecobee:low:21.5",
                "Step | Cool to | 24° | left set:climate.ecobee:high:23.5, right set:climate.ecobee:high:24.5",
                "Step | Mode | Heat and cool | left set:climate.ecobee:mode:cool, right set:climate.ecobee:mode:auto",
                "Step | Preset | Home | left set:climate.ecobee:preset:away, right set:climate.ecobee:preset:eco",
                "Step | Fan | Auto low | left set:climate.ecobee:fan:on_high, right set:climate.ecobee:fan:auto_high",
            ]
        );
        // A water heater steps the same way and has its own modes.
        let heater = one(r#"{"entity_id":"water_heater.tank","state":"eco","attributes":{"min_temp":43.3,"max_temp":60.0,"target_temp_step":1,"operation_list":["eco","electric","off"],"current_temperature":null,"temperature":48.3,"operation_mode":"eco","away_mode":"off","friendly_name":"Tank"}}"#);
        assert_eq!(
            rows(&Home::build(vec![heater], &Registry::default(), None, &Options::default(), now()).sheet("dev:water_heater.tank").unwrap()),
            [
                "Step | Temperature | 48.3° | left set:water_heater.tank:temperature:47.3, right set:water_heater.tank:temperature:49.3",
                "Step | Mode | Eco | left set:water_heater.tank:mode:off, right set:water_heater.tank:mode:electric",
                "Press | Away mode | Off | ok set:water_heater.tank:away:on",
            ]
        );
        // A humidifier steps its target by five in a hundred, and OK switches it.
        let humidifier = one(r#"{"entity_id":"humidifier.bedroom","state":"on","attributes":{"min_humidity":30,"max_humidity":70,"available_modes":["home","eco"],"current_humidity":45,"humidity":68,"mode":"home","friendly_name":"Bedroom Humidifier"}}"#);
        assert_eq!(
            rows(&Home::build(vec![humidifier], &Registry::default(), None, &Options::default(), now()).sheet("dev:humidifier.bedroom").unwrap()),
            [
                "Text | Now | 45%",
                "Step | Humidity | 68% | ok toggle:humidifier.bedroom, left set:humidifier.bedroom:humidity:63, right set:humidifier.bedroom:humidity:70",
                "Step | Mode | Home | left set:humidifier.bedroom:mode:eco, right set:humidifier.bedroom:mode:eco",
            ]
        );
    }

    #[test]
    fn a_light_sheet_offers_only_what_the_light_can_do() {
        let home = home_in(Some("living_room"));
        // Warm white now: its warmth is named, and its colour is not one of the wheel's.
        let ceiling = home.sheet("dev:light.ceiling_lights").unwrap();
        assert_eq!((ceiling.title.as_str(), ceiling.subtitle.as_deref()), ("Ceiling Lights", Some("Living Room, on, 71%")));
        assert_eq!(
            rows(&ceiling),
            [
                "Step | Brightness | 71% | ok toggle:light.ceiling_lights, left step:light.ceiling_lights:brightness:-10, right step:light.ceiling_lights:brightness:+10",
                "Step | Warmth | Warm | left set:light.ceiling_lights:kelvin:2200, right set:light.ceiling_lights:kelvin:4000",
                "Step | Colour |  | left set:light.ceiling_lights:colour:FF00AA, right set:light.ceiling_lights:colour:FF0000",
            ]
        );
        // A lamp of colours alone has no warmth to set, and says the colour it is nearest.
        assert_eq!(
            rows(&home.sheet("dev:light.living_room_lamp").unwrap()),
            [
                "Step | Brightness | 60% | ok toggle:light.living_room_lamp, left step:light.living_room_lamp:brightness:-10, right step:light.living_room_lamp:brightness:+10",
                "Step | Colour | Red | left set:light.living_room_lamp:colour:FF00AA, right set:light.living_room_lamp:colour:FF8000",
            ]
        );
        // One that only switches has one row.
        assert_eq!(rows(&home.sheet("dev:light.kitchen_lights").unwrap()), ["Press | Power | Off | ok toggle:light.kitchen_lights"]);
        // One that does not answer says so and offers nothing.
        assert_eq!(rows(&home.sheet("dev:light.office_lamp").unwrap()), ["Text | Not responding"]);
        assert_eq!(home.sheet("dev:light.office_lamp").unwrap().subtitle.as_deref(), Some("Büro & Café, not responding"));
        // A group is followed by the lamps in it.
        assert_eq!(
            rows(&home.sheet("dev:light.living_room").unwrap()),
            [
                "Step | Brightness | 65% | ok toggle:light.living_room, left step:light.living_room:brightness:-10, right step:light.living_room:brightness:+10",
                "Step | Warmth |  | left set:light.living_room:kelvin:2700, right set:light.living_room:kelvin:5000",
                "Step | Colour | Red | left set:light.living_room:colour:FF00AA, right set:light.living_room:colour:FF8000",
                "Group | In this group",
                "Step | Ceiling Lights | 71% | ok toggle:light.ceiling_lights, left step:light.ceiling_lights:brightness:-10, right step:light.ceiling_lights:brightness:+10",
                "Step | Living Room Lamp | 60% | ok toggle:light.living_room_lamp, left step:light.living_room_lamp:brightness:-10, right step:light.living_room_lamp:brightness:+10",
            ]
        );
        // Warmth stays inside what the lamp can show, and stops at its ends.
        let narrow = one(r#"{"entity_id":"light.a","state":"on","attributes":{"supported_color_modes":["color_temp"],"brightness":255,"color_temp_kelvin":2700,"min_color_temp_kelvin":2700,"max_color_temp_kelvin":4000}}"#);
        assert_eq!(rows(&Home::build(vec![narrow], &Registry::default(), None, &Options::default(), now()).sheet("dev:light.a").unwrap())[1], "Step | Warmth | Warm | left set:light.a:kelvin:2700, right set:light.a:kelvin:4000");
        assert_eq!((hue([255, 0, 0]), hue([0, 255, 0]), hue([0, 0, 255]), hue([255, 255, 255]), hue([128, 128, 130])), (Some(0.0), Some(120.0), Some(240.0), None, None));
        // The other kinds, each with what it can do and no more.
        assert_eq!(
            rows(&home.sheet("dev:fan.living_room_fan").unwrap()),
            [
                "Step | Speed | Off | ok toggle:fan.living_room_fan, left step:fan.living_room_fan:speed:-1, right step:fan.living_room_fan:speed:+1",
                "Press | Oscillate | Off | ok set:fan.living_room_fan:oscillating:on",
                "Step | Mode |  | left set:fan.living_room_fan:preset:on, right set:fan.living_room_fan:preset:auto",
            ]
        );
        assert_eq!(
            rows(&home.sheet("dev:media_player.living_room").unwrap()),
            [
                "Press | Pause | Playing | ok media:media_player.living_room:play_pause",
                "Step | Volume | 40% | ok media:media_player.living_room:mute, left step:media_player.living_room:volume:-5, right step:media_player.living_room:volume:+5",
                "Press | Next |  | ok media:media_player.living_room:next",
                "Press | Previous |  | ok media:media_player.living_room:previous",
                "Step | Source | dvd | left set:media_player.living_room:source:youtube, right set:media_player.living_room:source:youtube",
                "Press | Turn off |  | ok media:media_player.living_room:off",
            ]
        );
        assert_eq!(
            rows(&home.sheet("dev:vacuum.robo").unwrap()),
            [
                "Press | Start | Docked | ok vacuum:vacuum.robo:start",
                "Press | Send home |  | ok vacuum:vacuum.robo:dock",
                "Press | Find it |  | ok vacuum:vacuum.robo:locate",
                "Step | Power | Medium | left set:vacuum.robo:fan:min, right set:vacuum.robo:fan:high",
            ]
        );
        // A speaker with many sources lists them on a sheet of their own.
        let sources: Vec<String> = (1..=10).map(|n| format!("\"HDMI {n}\"")).collect();
        let tv = one(&format!(r#"{{"entity_id":"media_player.tv","state":"on","attributes":{{"device_class":"tv","source_list":[{}],"source":"HDMI 2","friendly_name":"TV","supported_features":2441}}}}"#, sources.join(",")));
        let den = Home::build(vec![tv], &Registry::default(), None, &Options::default(), now());
        assert_eq!(rows(&den.sheet("dev:media_player.tv").unwrap()), ["Press | Play | On | ok media:media_player.tv:play_pause", "Open | Source | HDMI 2 | opens src:media_player.tv", "Press | Turn off |  | ok media:media_player.tv:off"]);
        let list = den.sheet("src:media_player.tv").unwrap();
        assert_eq!((list.title.as_str(), list.subtitle.as_deref(), list.rows.len()), ("Source", Some("TV"), 10));
        assert_eq!(rows(&list)[1], "Press | HDMI 2 |  | ok set:media_player.tv:source:HDMI 2");
        assert_eq!(list.rows.iter().filter(|r| r.on == Some(true)).count(), 1);
    }

    // Errors and search

    #[test]
    fn errors_say_the_cause_and_the_fix() {
        let odd_names = ["Kitchen Lights", "iPad dock", "3D printer", "客厅灯", "\"Quoted\"", &"A very long name that someone typed without ever stopping to think ".repeat(8)];
        let mut sentences: Vec<String> = [REFUSED, SIGNED_OUT, SIGN_IN, NO_COMPANION, BLOCKED, NOT_ALLOWED, TOO_LARGE, NO_LABELS, LABELS_NEED_ADMIN, OUT_OF_DATE].iter().map(|s| s.to_string()).collect();
        sentences.push(still_starting("http://192.168.0.20:8123"));
        for name in odd_names {
            sentences.extend([no_label(name), gone(name), not_responding(name), turned_down(name), could_not(name), still_working(name)]);
        }
        for sentence in &sentences {
            assert!(reads_as_sentence(sentence), "{sentence}");
            assert!(!sentence.contains('!') && !sentence.contains("Oops") && !sentence.contains(" - "), "{sentence}");
        }
        assert_eq!(not_responding("Bed Light"), "Home Assistant can't reach Bed Light. Check its power and connection, then try again.");
        assert_eq!(gone("Bed Light"), "Home Assistant no longer has Bed Light. If it was renamed there, its tile catches up in a few minutes.");
        assert_eq!(turned_down("Hvac"), "Home Assistant turned that down for Hvac. Check the device in Home Assistant.");
        assert_eq!(could_not("Front Door"), "Home Assistant couldn't get Front Door to do that. Check the device there, then try again.");
        assert_eq!(still_working("Slow fade"), "Home Assistant is still working on Slow fade. Look at its tile in a moment.");
        assert_eq!(no_label("TV"), "Home Assistant has no label called TV. Check the label in the plugin's settings.");
        assert_eq!(still_starting("http://192.168.0.20:8123"), "Home Assistant at 192.168.0.20:8123 is still starting. The tiles come back by themselves in a minute.");
        // With no token yet, the settings are where to add one.
        assert_eq!(Server::open(BASE, None, None).err().as_deref(), Some("Add the access token in the plugin's settings."));
        assert_eq!(Server::open(BASE, Some("  "), None).err().as_deref(), Some("Add the access token in the plugin's settings."));
        // Off the TV nothing answers, which reads as a server that is away.
        let server = Server::open(BASE, Some(TOKEN), None).unwrap();
        assert_eq!(server.states(0).err().as_deref(), Some("Home Assistant didn't answer at 192.168.0.20:8123. Check the server, then try again."));
        assert_eq!(server.explain(Failure::Slow), "Home Assistant didn't answer at 192.168.0.20:8123. Check the server, then try again.");
        assert_eq!((server.explain(Failure::Refused).as_str(), server.explain(Failure::Blocked).as_str()), (REFUSED, BLOCKED));
        assert_eq!(server.explain(Failure::Limit(host::BUDGET_SPENT.into())), host::BUDGET_SPENT);
        assert_eq!(server.wrong_status(404, "/api/states"), "The server at 192.168.0.20:8123 didn't answer like a Home Assistant server. Check the server address in the plugin's settings.");
        assert_eq!(server.wrong_status(502, "/api/states"), "Home Assistant at 192.168.0.20:8123 answered with error 502. Check the server, then try again.");
        // A press on a server that is away says so too, and never the token.
        let call = Call::new("light/turn_on", json!({"entity_id": "light.bed_light"}));
        assert_eq!(server.call(&call, "Bed Light", false).err().as_deref(), Some("Home Assistant didn't answer at 192.168.0.20:8123. Check the server, then try again."));
        // What Home Assistant says of a refusal goes to the log in its own words, cut short.
        assert_eq!(said(r#"{"message":"Entity cover.kitchen_window does not support action cover.set_cover_position"}"#), "Entity cover.kitchen_window does not support action cover.set_cover_position");
        assert_eq!(said("400: Bad Request"), "400: Bad Request");
        assert_eq!(said(&"x".repeat(900)).len(), 200);
    }

    #[test]
    fn a_refused_token_is_not_tried_again() {
        let server = Server::open(BASE, Some(TOKEN), None).unwrap();
        let refused = server.mark();
        // The same server and token are turned away here, before any request is made.
        assert_eq!(Server::open(BASE, Some(TOKEN), Some(&refused)).err().as_deref(), Some("Home Assistant didn't accept the access token. Check it in the plugin's settings, or make a new one."));
        assert_eq!(Server::open("192.168.0.20:8123/", Some(&format!("Bearer {TOKEN}\n")), Some(&refused)).err().as_deref(), Some(REFUSED));
        // A new token, or the same one for another server, is given its chance.
        assert!(Server::open(BASE, Some("a.new.token"), Some(&refused)).is_ok());
        assert!(Server::open("http://192.168.0.21:8123", Some(TOKEN), Some(&refused)).is_ok());
        assert!(Server::open(BASE, Some(TOKEN), Some("")).is_ok());
        // The address is cut to the server, and the token to itself.
        assert_eq!(origin("http://192.168.0.20:8123/lovelace/0?edit=1"), "http://192.168.0.20:8123");
        assert_eq!(origin("https://ha.example.org"), "https://ha.example.org");
        assert_eq!(origin("http://[fd00::5]:8123/"), "http://[fd00::5]:8123");
        assert_eq!((tidy_token(" Bearer abc.def \n"), tidy_token("bearer abc"), tidy_token("abc.\ndef"), tidy_token("bearer")), ("abc.def".into(), "abc".into(), "abc.def".into(), "bearer".into()));
        assert_eq!(server.auth.header(), format!("Bearer {TOKEN}"));
        // A token that reads but may not control is marked the same way, under a key of its own,
        // so its reads go on and its presses are turned away here.
        assert!(server.barred(Some(&refused)) && !server.barred(None) && !Server::open(BASE, Some("a.new.token"), None).unwrap().barred(Some(&refused)));
        assert_ne!((NOT_ALLOWED, NO_CALLS_KEY), (REFUSED, REFUSED_KEY));
    }

    #[test]
    fn search_finds_rooms_scenes_and_devices_by_name() {
        let home = home_in(Some("kitchen"));
        let found = |query: &str| home.search(query, 12, true);
        // The room first, then the scene, then what is in the room, each as its tile would be.
        assert_eq!(
            lines(&found("liv")),
            [
                ("room:living_room", "Living Room", "2 lights on, 19.5°"),
                ("e:scene.living_room_relax", "Living Room Relax", "Living Room"),
                ("e:light.living_room", "Living Room", "Living Room, on, 65%"),
                ("e:light.living_room_lamp", "Living Room Lamp", "Living Room, on, 60%"),
                ("e:cover.living_room_window", "Living Room Window", "Living Room, 70% open"),
                ("e:fan.living_room_fan", "Living Room Fan", "Living Room, off"),
                ("e:media_player.living_room", "Living Room", "Living Room, playing, Kind of Blue"),
            ]
        );
        assert_eq!(found("liv").iter().map(does).collect::<Vec<_>>()[..3], ["opens room:living_room", "scene:scene.living_room_relax", "toggle:light.living_room"]);
        // A word is matched from its start, in any case, and several words as they stand.
        assert_eq!(found("LAMP").iter().map(|i| i.title.as_str()).collect::<Vec<_>>(), ["Office Lamp", "Living Room Lamp"]);
        assert_eq!(found("room la").iter().map(|i| i.title.as_str()).collect::<Vec<_>>(), ["Living Room Lamp"]);
        assert!(found("amp").is_empty() && found("  ").is_empty() && found("xyzzy").is_empty());
        assert_eq!(found("café").iter().map(|i| i.id.as_str()).collect::<Vec<_>>(), ["room:buro_cafe"]);
        // Nothing that lets someone in moves on one press from a search either.
        assert_eq!(
            found("door").iter().map(|i| (i.title.as_str(), i.subtitle.as_deref().unwrap_or(""), does(i))).collect::<Vec<_>>(),
            [
                ("Garage Door", "Garage, closed", "opens dev:cover.garage_door".to_string()),
                ("Cellar Door", "Garage, locked", "opens dev:lock.cellar_door".to_string()),
                ("Front Door", "Hall, locked", "opens dev:lock.front_door".to_string()),
                ("Kitchen Door", "Kitchen, unlocked", "lock:lock.kitchen_door".to_string()),
            ]
        );
        // A scene of the whole home, a switch kept under More, a sensor and a hidden fan.
        assert_eq!(lines(&found("good")), [("e:scene.good_night", "Good night", "Scene")]);
        assert!(found("child").is_empty() && found("battery").is_empty() && found("hidden").is_empty());
        assert_eq!(lines(&found("drive")), [("e:camera.drive", "Drive", "Streaming")]);
        // No more than were asked for.
        assert_eq!(home.search("l", 3, true).len(), 3);
        // Once the states are old the room is still said, and the state and its light are not.
        let stale = home.search("lamp", 12, false);
        assert_eq!(lines(&stale), [("e:light.office_lamp", "Office Lamp", "Büro & Café"), ("e:light.living_room_lamp", "Living Room Lamp", "Living Room")]);
        assert!(stale.iter().all(|i| i.on.is_none() && i.tint.is_none() && i.glyph.as_deref() == Some("light")));
        assert_eq!(lines(&home.search("liv", 1, false)), [("room:living_room", "Living Room", "")]);
        assert_eq!(home.search("door", 12, false).iter().map(does).collect::<Vec<_>>(), ["opens dev:cover.garage_door", "opens dev:lock.cellar_door", "opens dev:lock.front_door", "lock:lock.kitchen_door"]);
        assert!(matches_words("Living Room Lamp", "room") && !matches_words("Bedroom Lamp", "room") && matches_words("Büro & Café", "caf"));
    }
}
