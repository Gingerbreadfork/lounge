//! The server over a real socket: what an unpaired client gets, and what the cookie opens.

use lounge_companion::{Alert, Companion, CompanionActions, DeviceInfo, LayoutDoc, LayoutPatch, LiveRef, Pressed, SheetRef};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

struct Fake;

/// What the fake TV's own app was asked to do for the phone.
static ASKED: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

/// The pop-ups the fake TV was asked to show.
static SHOWN: std::sync::Mutex<Vec<Alert>> = std::sync::Mutex::new(Vec::new());

impl CompanionActions for Fake {
    fn phone_view(&self, what: &str) -> Result<String, String> {
        match what {
            "settings" => Ok(r#"{"groups":[]}"#.into()),
            "notifications" => Ok(r#"{"on":true,"list":[]}"#.into()),
            _ => Err("unknown".into()),
        }
    }
    fn phone_command(&self, command: &str) -> Result<(), String> {
        ASKED.lock().unwrap().push(command.to_string());
        if command == "setting:theme=Neon" { Err("The TV doesn't have that theme.".into()) } else { Ok(()) }
    }
    /// A title of `behind` is refused the way a TV that may not draw over other apps refuses.
    fn alert(&self, alert: Alert) -> Result<(), String> {
        if alert.title == "behind" {
            return Err("Lounge can't show this over another app yet.".into());
        }
        SHOWN.lock().unwrap().push(alert);
        Ok(())
    }
    fn device_info(&self) -> DeviceInfo {
        DeviceInfo { name: "TV".into(), model: "TV".into(), version: "0".into(), tier: "a".into(), lang: "en".into() }
    }
    fn snapshot_json(&self) -> String {
        "{}".into()
    }
    /// An action of `fail:` and some words fails with those words, and one of `say:` says them.
    fn perform(&self, action: &str) -> Result<Option<String>, String> {
        match action.strip_prefix("fail:") {
            Some(said) => Err(said.to_string()),
            None => Ok(action.strip_prefix("say:").map(str::to_string)),
        }
    }
    /// An action of `sheet:` or `live:` hands that back for the phone to show.
    fn perform_here(&self, action: &str) -> Result<Pressed, String> {
        let mut pressed = Pressed::default();
        if let Some(id) = action.strip_prefix("sheet:") {
            pressed.sheet = Some(SheetRef { plugin: "lamps".into(), id: id.into() });
        } else if let Some(url) = action.strip_prefix("live:") {
            pressed.live = Some(LiveRef { plugin: "lamps".into(), title: "Door".into(), url: url.into() });
        } else {
            pressed.say = self.perform(action)?;
        }
        Ok(pressed)
    }
    fn sheet_json(&self, plugin: &str, id: &str) -> Result<String, String> {
        Ok(serde_json::json!({ "title": format!("{plugin} {id}"), "rows": [] }).to_string())
    }
    fn sheet_press(&self, _: &str, sheet: &str, row: &str, which: &str) -> Result<Pressed, String> {
        Ok(Pressed { say: Some(format!("{sheet} {row} {which}")), ..Pressed::default() })
    }
    /// Only the picture at `shown` was handed out.
    fn live_picture(&self, _: &str, url: &str) -> Result<Vec<u8>, String> {
        if url == "http://camera.local/shown.jpg" { Ok(vec![0xFF, 0xD8, 0xFF, 1, 2, 3]) } else { Err("not a picture that is being shown".into()) }
    }
    fn live_again(&self, _: &str, url: &str) -> Result<String, String> {
        Ok(format!("{url}?again"))
    }
    fn live_on_tv(&self, _: &str, _: &str, _: &str) -> Result<(), String> {
        Ok(())
    }
    fn tv_controls_json(&self) -> Result<String, String> {
        Ok(r#"{"note":null,"controls":[{"id":"sleep","title":"Everything off","value":null,"group":null,"glyph":"power"}]}"#.into())
    }
    /// Only `sleep` is a control here; anything else fails in the TV's own words.
    fn tv_control(&self, id: &str) -> Result<(), String> {
        if id == "sleep" { Ok(()) } else { Err("It didn't go to sleep. Try again.".into()) }
    }
    fn launch(&self, _: &str) -> Result<(), String> {
        Ok(())
    }
    /// Only the plugin called `waiting` waits for a sign-in, at the key `k3y`, and its name is the query it was given.
    fn signed_in(&self, plugin: &str, key: &str, query: &str) -> Result<String, String> {
        if plugin != "waiting" || key != "k3y" {
            return Err("Nothing on the TV is waiting for this sign-in.".into());
        }
        Ok(format!("<{query}>"))
    }
    fn send_key(&self, _: &str) -> Result<(), String> {
        Ok(())
    }
    fn type_text(&self, _: &str) -> Result<(), String> {
        Ok(())
    }
    fn search(&self, _: &str) -> Result<(), String> {
        Ok(())
    }
    fn layout(&self) -> LayoutDoc {
        LayoutDoc::default()
    }
    fn set_layout(&self, _: LayoutPatch) -> Result<LayoutDoc, String> {
        Ok(LayoutDoc::default())
    }
    fn plugins_json(&self) -> String {
        "[]".into()
    }
    fn install_plugin(&self, _: &Path) -> Result<String, String> {
        Err("no".into())
    }
    fn plugin_command(&self, _: &str, _: &str) -> Result<(), String> {
        Ok(())
    }
    fn set_plugin_setting(&self, _: &str, _: &str, _: Option<String>) -> Result<(), String> {
        Ok(())
    }
    fn plugin_log(&self, _: &str) -> Vec<String> {
        Vec::new()
    }
    fn diagnostics_json(&self) -> String {
        "{}".into()
    }
    fn export_json(&self) -> String {
        "{\"backup\":true}".into()
    }
    fn import_json(&self, _: &str) -> Result<(), String> {
        Ok(())
    }
    fn art_roots(&self) -> Vec<PathBuf> {
        vec![PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests")]
    }
    fn temp_dir(&self) -> PathBuf {
        std::env::temp_dir()
    }
    fn download(&self, _: &str, _: &Path) -> Result<(), String> {
        Err("no".into())
    }
}

/// Send raw request text, return the status line and the headers, lower-cased.
fn send(port: u16, request: &str) -> (u16, String) {
    let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    s.write_all(request.as_bytes()).unwrap();
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    while let Ok(n) = s.read(&mut chunk) {
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
        if buf.windows(4).any(|w| w == b"\r\n\r\n") {
            break;
        }
    }
    let text = String::from_utf8_lossy(&buf).to_ascii_lowercase();
    let status = text.split_whitespace().nth(1).and_then(|c| c.parse().ok()).unwrap_or(0);
    (status, text)
}

/// Send a request that asks to close the connection, and return the whole response as sent.
fn exchange(port: u16, request: &str) -> String {
    let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    s.write_all(request.as_bytes()).unwrap();
    // A picture's bytes are not text, so the reply is read as bytes first.
    let mut out = Vec::new();
    let _ = s.read_to_end(&mut out);
    String::from_utf8_lossy(&out).into_owned()
}

fn status(response: &str) -> u16 {
    response.split_whitespace().nth(1).and_then(|c| c.parse().ok()).unwrap_or(0)
}

fn json_body(response: &str) -> serde_json::Value {
    serde_json::from_str(response.split_once("\r\n\r\n").map(|(_, b)| b).unwrap_or("")).unwrap_or_else(|e| panic!("{e}: {response}"))
}

/// Exchange a typed `code` or a QR `link`'s key at `POST /api/pair`, as the page does.
fn post_pair(port: u16, host: &str, field: &str, value: &str) -> String {
    let body = format!("{{\"{field}\":\"{value}\"}}");
    exchange(port, &format!("POST /api/pair HTTP/1.1\r\nHost: {host}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()))
}

/// Pair a phone the way scanning the TV's QR code does, and return its token.
fn pair(companion: &Companion, host: &str) -> String {
    let link = companion.new_pairing(&format!("http://{host}"), now()).unwrap().link;
    let response = post_pair(companion.port, host, "link", &link);
    json_body(&response)["token"].as_str().unwrap_or_else(|| panic!("no token: {response}")).to_string()
}

/// The session cookie's value `POST /api/session` hands a paired phone.
fn session_cookie(port: u16, host: &str, token: &str) -> String {
    let response = exchange(port, &format!("POST /api/session HTTP/1.1\r\nHost: {host}\r\nAuthorization: Bearer {token}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"));
    response
        .lines()
        .filter_map(|l| l.split_once(':'))
        .find(|(name, _)| name.eq_ignore_ascii_case("set-cookie"))
        .and_then(|(_, v)| v.trim().strip_prefix("lounge_session=")?.split(';').next().map(str::to_string))
        .unwrap_or_else(|| panic!("no session cookie: {response}"))
}

/// Forget the phone a token belongs to, as another phone or the TV would.
fn forget(port: u16, host: &str, token: &str) {
    let devices = json_body(&exchange(port, &format!("GET /api/devices HTTP/1.1\r\nHost: {host}\r\nAuthorization: Bearer {token}\r\nConnection: close\r\n\r\n")));
    let id = devices.as_array().unwrap().iter().find(|d| d["current"] == true).unwrap()["id"].as_str().unwrap().to_string();
    let (status, _) = send(port, &format!("DELETE /api/devices/{id} HTTP/1.1\r\nHost: {host}\r\nAuthorization: Bearer {token}\r\n\r\n"));
    assert_eq!(status, 204);
}

/// A request for one of the fake's artwork files: this test file itself.
fn art_request(host: &str) -> String {
    let path: String = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/server.rs")
        .bytes()
        .map(|b| if b.is_ascii_alphanumeric() || b"/._-".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") })
        .collect();
    format!("GET /api/art?path={path} HTTP/1.1\r\nHost: {host}\r\n")
}

#[test]
fn the_server_checks_before_it_listens() {
    let dir = tempfile::tempdir().unwrap();
    let companion = Companion::start(0, dir.path(), Arc::new(Fake)).unwrap();
    let port = companion.port;
    let host = format!("127.0.0.1:{port}");

    // An unpaired upload claiming 30 MB is turned away at once, before any of it is sent.
    let started = Instant::now();
    let (status, _) = send(port, &format!("POST /api/restore HTTP/1.1\r\nHost: {host}\r\nContent-Length: 31457280\r\n\r\n"));
    assert_eq!(status, 401);
    assert!(started.elapsed() < Duration::from_secs(2));

    // A name the TV does not go by is not answered.
    let (status, _) = send(port, "GET /api/state HTTP/1.1\r\nHost: rebind.attacker.example\r\n\r\n");
    assert_eq!(status, 421);

    // A paired phone gets a cookie; it opens the backup, not the rest of the API.
    let token = pair(&companion, &host);
    let (status, head) = send(port, &format!("POST /api/session HTTP/1.1\r\nHost: {host}\r\nAuthorization: Bearer {token}\r\nContent-Length: 0\r\n\r\n"));
    assert_eq!(status, 204);
    assert!(head.contains("set-cookie: lounge_session=") && head.contains("httponly") && head.contains("samesite=strict"), "{head}");
    let cookie = format!("Cookie: lounge_session={}", session_cookie(port, &host, &token));
    let (status, _) = send(port, &format!("GET /api/backup HTTP/1.1\r\nHost: {host}\r\n{cookie}\r\n\r\n"));
    assert_eq!(status, 200);
    let (status, _) = send(port, &format!("GET /api/state HTTP/1.1\r\nHost: {host}\r\n{cookie}\r\n\r\n"));
    assert_eq!(status, 401);
    let (status, _) = send(port, &format!("GET /api/backup?token={token} HTTP/1.1\r\nHost: {host}\r\n\r\n"));
    assert_eq!(status, 401);

    // Small JSON bodies only, except on the upload routes.
    let big = "x".repeat(100 * 1024);
    let (status, _) = send(port, &format!("POST /api/text HTTP/1.1\r\nHost: {host}\r\nAuthorization: Bearer {token}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{big}", big.len()));
    assert_eq!(status, 413);
}

#[test]
fn the_cookie_is_a_key_of_its_own() {
    let dir = tempfile::tempdir().unwrap();
    let companion = Companion::start(0, dir.path(), Arc::new(Fake)).unwrap();
    let port = companion.port;
    let host = format!("127.0.0.1:{port}");
    let token = pair(&companion, &host);
    let session = session_cookie(port, &host, &token);
    assert_ne!(session, token);

    // The cookie opens pictures, and the token does not work as one.
    let art = art_request(&host);
    assert_eq!(send(port, &format!("{art}Cookie: lounge_session={session}\r\n\r\n")).0, 200);
    assert_eq!(send(port, &format!("{art}Cookie: lounge_session={token}\r\n\r\n")).0, 401);

    // Nothing the token unlocks takes the cookie's value, as a cookie or as a token.
    assert_eq!(send(port, &format!("GET /api/state HTTP/1.1\r\nHost: {host}\r\nCookie: lounge_session={session}\r\n\r\n")).0, 401);
    assert_eq!(send(port, &format!("GET /api/state HTTP/1.1\r\nHost: {host}\r\nAuthorization: Bearer {session}\r\n\r\n")).0, 401);
    assert_eq!(send(port, &format!("GET /api/state HTTP/1.1\r\nHost: {host}\r\nAuthorization: Bearer {token}\r\n\r\n")).0, 200);

    // Forgetting the phone takes its cookie with it.
    forget(port, &host, &token);
    assert_eq!(send(port, &format!("{art}Cookie: lounge_session={session}\r\n\r\n")).0, 401);
}

#[test]
fn a_qr_link_pairs_once_for_a_token_of_its_own() {
    let dir = tempfile::tempdir().unwrap();
    let companion = Companion::start(0, dir.path(), Arc::new(Fake)).unwrap();
    let port = companion.port;
    let host = format!("127.0.0.1:{port}");
    let link = companion.new_pairing(&format!("http://{host}"), now()).unwrap().link;
    let state = |bearer: &str| send(port, &format!("GET /api/state HTTP/1.1\r\nHost: {host}\r\nAuthorization: Bearer {bearer}\r\n\r\n")).0;
    assert_eq!(state(&link), 401);

    let response = post_pair(port, &host, "link", &link);
    assert_eq!(status(&response), 200, "{response}");
    let token = json_body(&response)["token"].as_str().unwrap().to_string();
    assert_ne!(token, link);
    assert_eq!(state(&token), 200);

    // The key in the link is spent and never opens anything.
    assert_eq!(state(&link), 401);
    assert_eq!(status(&post_pair(port, &host, "link", &link)), 403);

    // A typed code still pairs as it did.
    let code = companion.new_pairing(&format!("http://{host}"), now()).unwrap().code;
    let response = post_pair(port, &host, "code", &code);
    assert_eq!(status(&response), 200, "{response}");
    assert_eq!(state(json_body(&response)["token"].as_str().unwrap()), 200);
}

#[test]
fn a_used_code_is_replaced_and_a_late_phone_is_told_what_to_do() {
    let dir = tempfile::tempdir().unwrap();
    let companion = Companion::start(0, dir.path(), Arc::new(Fake)).unwrap();
    let port = companion.port;
    let host = format!("127.0.0.1:{port}");
    let shown = companion.new_pairing(&format!("http://{host}"), now()).unwrap();
    assert_eq!(status(&post_pair(port, &host, "link", &shown.link)), 200);

    // A second phone that scanned the same picture is told the cause and the fix.
    let response = post_pair(port, &host, "link", &shown.link);
    assert_eq!(status(&response), 403);
    assert_eq!(json_body(&response)["error"], "That code has been used or has expired. Choose New code on the TV and try again.");

    // The TV already has a fresh code to show, and it pairs.
    let fresh = companion.live_pairing(now()).unwrap();
    assert_ne!(fresh.code, shown.code);
    assert_eq!(status(&post_pair(port, &host, "code", &fresh.code)), 200);
}

#[test]
fn a_sentence_the_tv_wrote_reaches_the_phone_as_it_is() {
    let dir = tempfile::tempdir().unwrap();
    let companion = Companion::start(0, dir.path(), Arc::new(Fake)).unwrap();
    let port = companion.port;
    let host = format!("127.0.0.1:{port}");
    let token = pair(&companion, &host);
    let told = |said: &str| {
        let body = serde_json::json!({ "action": format!("fail:{said}") }).to_string();
        let request = format!("POST /api/open HTTP/1.1\r\nHost: {host}\r\nAuthorization: Bearer {token}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
        json_body(&exchange(port, &request))["error"].as_str().unwrap_or_default().to_string()
    };
    // A title with a colon, a server by its name and the plugin host's own words all pass.
    for said in [
        "Couldn't open Alien: Earth. Is its app still installed?",
        "Jellyfin didn't answer at nas.local:8096. Check the server, then try again.",
        "Lounge couldn't read or write the plugin's files: No space left on device (os error 28). Check that the TV has free space, then try again.",
    ] {
        assert_eq!(told(said), said);
    }
    // Text nobody worded stays in the log.
    for raw in ["No space left on device (os error 28)", "platform: busy", "no"] {
        assert_eq!(told(raw), "The TV couldn't open that. Try it from the TV.");
    }
}

#[test]
fn a_press_that_says_something_is_answered_with_its_words() {
    let dir = tempfile::tempdir().unwrap();
    let companion = Companion::start(0, dir.path(), Arc::new(Fake)).unwrap();
    let port = companion.port;
    let host = format!("127.0.0.1:{port}");
    let token = pair(&companion, &host);
    let open = |action: &str| {
        let body = serde_json::json!({ "action": action }).to_string();
        exchange(port, &format!("POST /api/open HTTP/1.1\r\nHost: {host}\r\nAuthorization: Bearer {token}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()))
    };
    let said = open("say:Movie time set");
    assert_eq!(status(&said), 200);
    assert_eq!(json_body(&said)["say"], "Movie time set");
    // A press with nothing to say is answered with nothing, as before.
    assert_eq!(status(&open("app:tv.example")), 204);
}

#[test]
fn a_phone_is_handed_sheets_and_live_pictures_to_show_itself() {
    let dir = tempfile::tempdir().unwrap();
    let companion = Companion::start(0, dir.path(), Arc::new(Fake)).unwrap();
    let port = companion.port;
    let host = format!("127.0.0.1:{port}");
    let token = pair(&companion, &host);
    let post = |path: &str, body: serde_json::Value| {
        let body = body.to_string();
        exchange(port, &format!("POST {path} HTTP/1.1\r\nHost: {host}\r\nAuthorization: Bearer {token}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()))
    };
    let get = |path: &str, token: &str| exchange(port, &format!("GET {path} HTTP/1.1\r\nHost: {host}\r\nAuthorization: Bearer {token}\r\nConnection: close\r\n\r\n"));

    // A press made for the phone's own screen says which sheet or picture to show.
    let sheet = json_body(&post("/api/open", serde_json::json!({ "action": "sheet:room", "here": true })));
    assert_eq!(sheet, serde_json::json!({ "sheet": { "plugin": "lamps", "id": "room" } }));
    let live = json_body(&post("/api/open", serde_json::json!({ "action": "live:http://camera.local/shown.jpg", "here": true })));
    assert_eq!(live, serde_json::json!({ "live": { "plugin": "lamps", "title": "Door", "url": "http://camera.local/shown.jpg" } }));
    assert_eq!(json_body(&post("/api/open", serde_json::json!({ "action": "say:Lamp on", "here": true }))), serde_json::json!({ "say": "Lamp on" }));

    // The sheet is read by name, and a row is pressed by name.
    assert_eq!(json_body(&get("/api/plugins/lamps/sheet?id=room%3Aden", &token))["title"], "lamps room:den");
    let pressed = post("/api/plugins/lamps/sheet", serde_json::json!({ "sheet": "room", "row": "lamp", "which": "press" }));
    assert_eq!(json_body(&pressed), serde_json::json!({ "say": "room lamp press" }));

    // The picture comes as a picture that nothing keeps, and only the one that was handed out.
    let frame = get("/api/plugins/lamps/picture?url=http%3A%2F%2Fcamera.local%2Fshown.jpg", &token);
    assert_eq!(status(&frame), 200);
    let head = frame.to_ascii_lowercase();
    assert!(head.contains("content-type: image/jpeg") && head.contains("cache-control: no-store"), "{frame}");
    assert_eq!(status(&get("/api/plugins/lamps/picture?url=http%3A%2F%2Fcamera.local%2Fother.jpg", &token)), 502);
    assert_eq!(json_body(&post("/api/plugins/lamps/live", serde_json::json!({ "url": "http://camera.local/shown.jpg" })))["url"], "http://camera.local/shown.jpg?again");
    assert_eq!(status(&post("/api/plugins/lamps/live", serde_json::json!({ "url": "http://camera.local/shown.jpg", "tv": true }))), 204);

    // None of it answers a phone the TV does not know.
    for path in ["/api/plugins/lamps/sheet?id=room", "/api/plugins/lamps/picture?url=http%3A%2F%2Fcamera.local%2Fshown.jpg", "/api/tv"] {
        assert_eq!(status(&get(path, "not-a-token")), 401, "{path}");
    }
}

#[test]
fn the_home_shows_a_pop_up_with_its_own_key_and_nothing_else() {
    let dir = tempfile::tempdir().unwrap();
    let companion = Companion::start(0, dir.path(), Arc::new(Fake)).unwrap();
    let port = companion.port;
    let host = format!("127.0.0.1:{port}");
    let phone = pair(&companion, &host);
    let call = |method: &str, path: &str, key: &str, body: &str| {
        exchange(port, &format!("{method} {path} HTTP/1.1\r\nHost: {host}\r\nAuthorization: Bearer {key}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()))
    };
    let ring = r#"{"title":"  Someone is at\n the door ","message":"Front door","image":"http://ha.local:8123/api/camera_proxy/camera.door?token=abc","duration":900}"#;
    // No key has been made, and a phone's token is not one.
    assert_eq!(json_body(&call("GET", "/api/alerts", &phone, ""))["keySet"], false);
    assert_eq!(status(&call("POST", "/api/alert", &phone, ring)), 401);
    let key = json_body(&call("POST", "/api/alerts/key", &phone, ""))["key"].as_str().unwrap().to_string();
    assert_eq!(json_body(&call("GET", "/api/alerts", &phone, ""))["keySet"], true);
    assert_eq!(status(&call("POST", "/api/alert", "guess", ring)), 401);
    assert_eq!(status(&call("POST", "/api/alert", &key, ring)), 204);
    let shown = SHOWN.lock().unwrap().last().cloned().unwrap();
    assert_eq!((shown.title.as_str(), shown.message.as_deref(), shown.seconds), ("Someone is at the door", Some("Front door"), Alert::MAX_SECONDS));
    assert!(shown.image.unwrap().starts_with("http://ha.local:8123/"));
    // The key opens nothing but pop-ups.
    assert_eq!(status(&call("GET", "/api/layout", &key, "")), 401);
    // No title is nothing to show, a picture must be a web address, and the TV's own refusal is passed on.
    assert_eq!(status(&call("POST", "/api/alert", &key, r#"{"title":"  "}"#)), 400);
    assert_eq!(status(&call("POST", "/api/alert", &key, r#"{"title":"Post","image":"file:///sdcard/x.png"}"#)), 204);
    assert_eq!(SHOWN.lock().unwrap().last().unwrap().image, None);
    let refused = call("POST", "/api/alert", &key, r#"{"title":"behind"}"#);
    assert_eq!((status(&refused), json_body(&refused)["error"].as_str()), (400, Some("Lounge can't show this over another app yet.")));
    // The phone can send a test, and taking the key away shuts the door again.
    assert_eq!(status(&call("POST", "/api/alerts/test", &phone, "")), 204);
    assert_eq!(status(&call("DELETE", "/api/alerts/key", &phone, "")), 204);
    assert_eq!(status(&call("POST", "/api/alert", &key, ring)), 401);
}

#[test]
fn the_phone_changes_a_setting_and_dismisses_a_notification() {
    let dir = tempfile::tempdir().unwrap();
    let companion = Companion::start(0, dir.path(), Arc::new(Fake)).unwrap();
    let port = companion.port;
    let host = format!("127.0.0.1:{port}");
    let phone = pair(&companion, &host);
    let call = |method: &str, path: &str, token: &str, body: &str| {
        exchange(port, &format!("{method} {path} HTTP/1.1\r\nHost: {host}\r\nAuthorization: Bearer {token}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()))
    };
    // None of it is open to a caller that is not a paired phone.
    assert_eq!(status(&call("GET", "/api/settings", "guess", "")), 401);
    assert_eq!(json_body(&call("GET", "/api/settings", &phone, ""))["groups"], serde_json::json!([]));
    assert_eq!(json_body(&call("GET", "/api/notifications", &phone, ""))["on"], true);
    assert_eq!(status(&call("PUT", "/api/settings", &phone, r#"{"key":"show_clock","value":"false"}"#)), 204);
    assert_eq!(status(&call("POST", "/api/notifications/dismiss", &phone, r#"{"key":"0|tv.app|7|null|10"}"#)), 204);
    assert_eq!(status(&call("POST", "/api/notifications/dismiss", &phone, "{}")), 204);
    assert_eq!(*ASKED.lock().unwrap(), vec!["setting:show_clock=false", "notification:dismiss:0|tv.app|7|null|10", "notification:dismiss_all"]);
    // A name that is not a setting's is refused here, and the TV's own refusal is passed on.
    assert_eq!(status(&call("PUT", "/api/settings", &phone, r#"{"key":"x=y","value":"1"}"#)), 400);
    let refused = call("PUT", "/api/settings", &phone, r#"{"key":"theme","value":"Neon"}"#);
    assert_eq!((status(&refused), json_body(&refused)["error"].as_str()), (400, Some("The TV doesn't have that theme.")));
}

#[test]
fn the_phone_says_a_few_words_on_the_tv() {
    let dir = tempfile::tempdir().unwrap();
    let companion = Companion::start(0, dir.path(), Arc::new(Fake)).unwrap();
    let port = companion.port;
    let host = format!("127.0.0.1:{port}");
    let phone = pair(&companion, &host);
    let say = |token: &str, body: &str| {
        exchange(port, &format!("POST /api/message HTTP/1.1\r\nHost: {host}\r\nAuthorization: Bearer {token}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()))
    };
    assert_eq!(status(&say("guess", r#"{"text":"Hello"}"#)), 401);
    // A message is a pop-up of its words alone, on one line, and nothing is not a message.
    assert_eq!(status(&say(&phone, r#"{"text":" Dinner\nis ready now "}"#)), 204);
    assert!(SHOWN.lock().unwrap().iter().any(|a| a.title == "Dinner is ready now" && a.message.is_none() && a.image.is_none()));
    assert_eq!(status(&say(&phone, r#"{"text":"  "}"#)), 400);
}

#[test]
fn the_phone_lists_the_tv_s_controls_and_runs_one() {
    let dir = tempfile::tempdir().unwrap();
    let companion = Companion::start(0, dir.path(), Arc::new(Fake)).unwrap();
    let port = companion.port;
    let host = format!("127.0.0.1:{port}");
    let token = pair(&companion, &host);
    let listed = exchange(port, &format!("GET /api/tv HTTP/1.1\r\nHost: {host}\r\nAuthorization: Bearer {token}\r\nConnection: close\r\n\r\n"));
    assert_eq!(json_body(&listed)["controls"][0]["title"], "Everything off");
    let run = |id: &str| {
        let body = serde_json::json!({ "id": id }).to_string();
        exchange(port, &format!("POST /api/tv HTTP/1.1\r\nHost: {host}\r\nAuthorization: Bearer {token}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()))
    };
    assert_eq!(status(&run("sleep")), 204);
    // A control that fails says why in the TV's own sentence.
    let failed = run("cec_tv_power");
    assert_eq!(status(&failed), 400);
    assert_eq!(json_body(&failed)["error"], "It didn't go to sleep. Try again.");
}

#[test]
fn a_sign_in_lands_without_pairing_and_only_where_one_is_waited_for() {
    let dir = tempfile::tempdir().unwrap();
    let companion = Companion::start(0, dir.path(), Arc::new(Fake)).unwrap();
    let port = companion.port;
    let host = format!("127.0.0.1:{port}");
    let get = |path: &str| exchange(port, &format!("GET {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n"));

    // The query reaches the plugin whole, and what the page shows of it is escaped.
    let landed = get("/signin/waiting/k3y?code=abc&state=1");
    assert_eq!(status(&landed), 200);
    assert!(landed.contains("<h1>Signed in</h1>"), "{landed}");
    assert!(landed.contains("&lt;code=abc&amp;state=1&gt; has what it needs. Look at the TV."), "{landed}");
    assert!(landed.to_ascii_lowercase().contains("cache-control: no-store"), "{landed}");

    // Another plugin's page, a guess at the key and an address with no key are all turned away.
    for path in ["/signin/other/k3y?code=abc", "/signin/waiting/guess?code=abc"] {
        let stray = get(path);
        assert_eq!(status(&stray), 409, "{path}");
        assert!(stray.contains("<h1>Not signed in</h1>") && stray.contains("Nothing on the TV is waiting for this sign-in."), "{stray}");
    }
    assert!(!get("/signin/waiting?code=abc").contains("Signed in"));

    // A name the TV does not go by is not answered here either.
    let (code, _) = send(port, "GET /signin/waiting/k3y?code=abc HTTP/1.1\r\nHost: rebind.attacker.example\r\n\r\n");
    assert_eq!(code, 421);
}

#[test]
fn the_live_socket_says_the_tv_is_still_there() {
    let dir = tempfile::tempdir().unwrap();
    let companion = Companion::start(0, dir.path(), Arc::new(Fake)).unwrap();
    let port = companion.port;
    let host = format!("127.0.0.1:{port}");
    let token = pair(&companion, &host);
    let mut socket = open_socket(port, &host, &session_cookie(port, &host, &token));
    socket.get_ref().set_read_timeout(Some(Duration::from_secs(8))).unwrap();
    assert_eq!(frame(&mut socket).0, TEXT);
    // With nothing new to send, a heartbeat that is not a snapshot arrives anyway.
    let (op, payload) = frame(&mut socket);
    assert_eq!((op, payload.as_slice()), (TEXT, b"ping".as_slice()));
}

/// What the server has sent on `s` up to the end of a response head, or nothing within `wait`.
fn head(s: &mut TcpStream, wait: Duration) -> String {
    s.set_read_timeout(Some(wait)).unwrap();
    let mut buf = Vec::new();
    let mut byte = [0u8; 1];
    while !buf.ends_with(b"\r\n\r\n") {
        match s.read(&mut byte) {
            Ok(1) => buf.push(byte[0]),
            _ => break,
        }
    }
    String::from_utf8_lossy(&buf).into_owned()
}

#[test]
fn an_upload_is_not_read_until_its_turn() {
    let dir = tempfile::tempdir().unwrap();
    let companion = Companion::start(0, dir.path(), Arc::new(Fake)).unwrap();
    let port = companion.port;
    let host = format!("127.0.0.1:{port}");
    let token = pair(&companion, &host);
    let backup = "{\"backup\":true}";

    // Each upload waits for the server's "100 Continue", which it sends once it reads the body.
    let start = || {
        let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
        let request = format!(
            "POST /api/restore HTTP/1.1\r\nHost: {host}\r\nAuthorization: Bearer {token}\r\nContent-Length: {}\r\nExpect: 100-continue\r\nConnection: close\r\n\r\n",
            backup.len()
        );
        s.write_all(request.as_bytes()).unwrap();
        s
    };
    let mut first = start();
    assert_eq!(status(&head(&mut first, Duration::from_secs(5))), 100);

    // The first holds the turn while its body arrives, and the second's body is left unread.
    let mut second = start();
    assert_eq!(head(&mut second, Duration::from_millis(300)), "");
    first.write_all(backup.as_bytes()).unwrap();
    assert_eq!(status(&head(&mut first, Duration::from_secs(5))), 204);
    assert_eq!(status(&head(&mut second, Duration::from_secs(5))), 100);
    second.write_all(backup.as_bytes()).unwrap();
    assert_eq!(status(&head(&mut second, Duration::from_secs(5))), 204);

    // The route's limit still holds once an upload has its turn.
    let big = "x".repeat(2 * 1024 * 1024 + 1);
    let (status, _) = send(port, &format!("POST /api/restore HTTP/1.1\r\nHost: {host}\r\nAuthorization: Bearer {token}\r\nContent-Length: {}\r\n\r\n{big}", big.len()));
    assert_eq!(status, 413);
}

#[test]
fn connections_are_capped_and_stopping_does_not_wait_for_them() {
    let dir = tempfile::tempdir().unwrap();
    let companion = Companion::start(0, dir.path(), Arc::new(Fake)).unwrap();
    let port = companion.port;
    // A dozen connections that never finish a request fill one address's share.
    let mut stalled: Vec<TcpStream> = (0..12)
        .map(|_| {
            let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
            s.write_all(b"GET /api/state HTTP/1.1\r\n").unwrap();
            s
        })
        .collect();
    std::thread::sleep(Duration::from_millis(200));
    let mut extra = TcpStream::connect(("127.0.0.1", port)).unwrap();
    extra.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
    let mut buf = [0u8; 16];
    assert_eq!(extra.read(&mut buf).unwrap_or(0), 0, "one more is closed unanswered");

    // Once one goes, the next request is answered again.
    drop(stalled.pop());
    std::thread::sleep(Duration::from_millis(200));
    let (status, _) = send(port, &format!("GET /api/state HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\r\n"));
    assert_eq!(status, 401);

    // Stopping does not wait for the stalled ones, and the port is free straight after.
    let started = Instant::now();
    drop(companion);
    assert!(started.elapsed() < Duration::from_secs(2), "took {:?}", started.elapsed());
    assert!(std::net::TcpListener::bind(("0.0.0.0", port)).is_ok());
}

#[test]
fn the_live_socket_upgrades() {
    let dir = tempfile::tempdir().unwrap();
    let companion = Companion::start(0, dir.path(), Arc::new(Fake)).unwrap();
    let port = companion.port;
    let host = format!("127.0.0.1:{port}");
    let token = pair(&companion, &host);
    let session = session_cookie(port, &host, &token);
    let (status, _) = send(
        port,
        &format!(
            "GET /api/ws HTTP/1.1\r\nHost: {host}\r\nCookie: lounge_session={session}\r\nConnection: Upgrade\r\nUpgrade: websocket\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\r\n"
        ),
    );
    assert_eq!(status, 101);
}

/// Open the live socket with a session cookie and read past the upgrade's response.
fn open_socket(port: u16, host: &str, session: &str) -> BufReader<TcpStream> {
    let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    s.write_all(
        format!(
            "GET /api/ws HTTP/1.1\r\nHost: {host}\r\nCookie: lounge_session={session}\r\nConnection: Upgrade\r\nUpgrade: websocket\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\r\n"
        )
        .as_bytes(),
    )
    .unwrap();
    let mut socket = BufReader::new(s);
    let mut line = String::new();
    socket.read_line(&mut line).unwrap();
    assert!(line.contains(" 101 "), "{line}");
    while line != "\r\n" {
        line.clear();
        socket.read_line(&mut line).unwrap();
    }
    socket
}

/// The next frame the server sends, as its opcode and payload.
fn frame(socket: &mut impl Read) -> (u8, Vec<u8>) {
    let mut head = [0u8; 2];
    socket.read_exact(&mut head).unwrap();
    let len = match head[1] & 0x7f {
        126 => {
            let mut n = [0u8; 2];
            socket.read_exact(&mut n).unwrap();
            u16::from_be_bytes(n) as usize
        }
        127 => {
            let mut n = [0u8; 8];
            socket.read_exact(&mut n).unwrap();
            u64::from_be_bytes(n) as usize
        }
        n => n as usize,
    };
    let mut payload = vec![0u8; len];
    socket.read_exact(&mut payload).unwrap();
    (head[0] & 0x0f, payload)
}

const TEXT: u8 = 1;
const CLOSE: u8 = 8;

fn close_code(payload: &[u8]) -> u16 {
    u16::from_be_bytes([payload[0], payload[1]])
}

#[test]
fn a_forgotten_phone_loses_its_live_socket() {
    let dir = tempfile::tempdir().unwrap();
    let companion = Companion::start(0, dir.path(), Arc::new(Fake)).unwrap();
    let port = companion.port;
    let host = format!("127.0.0.1:{port}");

    // Forgotten from another phone, it gets no more snapshots and is told why.
    let token = pair(&companion, &host);
    let mut socket = open_socket(port, &host, &session_cookie(port, &host, &token));
    assert_eq!(frame(&mut socket).0, TEXT);
    forget(port, &host, &token);
    companion.notify();
    let (op, payload) = frame(&mut socket);
    assert_eq!(op, CLOSE);
    assert_eq!(close_code(&payload), 4401);

    // Forgotten from the TV, its socket closes without waiting for a new snapshot.
    let token = pair(&companion, &host);
    let mut socket = open_socket(port, &host, &session_cookie(port, &host, &token));
    assert_eq!(frame(&mut socket).0, TEXT);
    companion.forget_all().unwrap();
    let (op, payload) = frame(&mut socket);
    assert_eq!(op, CLOSE);
    assert_eq!(close_code(&payload), 4401);
}

fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as i64
}
