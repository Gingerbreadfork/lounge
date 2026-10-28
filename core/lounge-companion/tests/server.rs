//! The server over a real socket: what an unpaired client gets, and what the cookie opens.

use lounge_companion::{Companion, CompanionActions, DeviceInfo, LayoutDoc, LayoutPatch};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

struct Fake;

impl CompanionActions for Fake {
    fn device_info(&self) -> DeviceInfo {
        DeviceInfo { name: "TV".into(), model: "TV".into(), version: "0".into(), tier: "a".into() }
    }
    fn snapshot_json(&self) -> String {
        "{}".into()
    }
    fn perform(&self, _: &str) -> Result<(), String> {
        Ok(())
    }
    fn launch(&self, _: &str) -> Result<(), String> {
        Ok(())
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
    let mut out = String::new();
    let _ = s.read_to_string(&mut out);
    out
}

fn json_body(response: &str) -> serde_json::Value {
    serde_json::from_str(response.split_once("\r\n\r\n").map(|(_, b)| b).unwrap_or("")).unwrap_or_else(|e| panic!("{e}: {response}"))
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
    let token = companion.new_pairing(&format!("http://{host}"), now()).unwrap().token;
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
    let token = companion.new_pairing(&format!("http://{host}"), now()).unwrap().token;
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
    let token = companion.new_pairing(&format!("http://{host}"), now()).unwrap().token;
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
    let token = companion.new_pairing(&format!("http://{host}"), now()).unwrap().token;
    let mut socket = open_socket(port, &host, &session_cookie(port, &host, &token));
    assert_eq!(frame(&mut socket).0, TEXT);
    forget(port, &host, &token);
    companion.notify();
    let (op, payload) = frame(&mut socket);
    assert_eq!(op, CLOSE);
    assert_eq!(close_code(&payload), 4401);

    // Forgotten from the TV, its socket closes without waiting for a new snapshot.
    let token = companion.new_pairing(&format!("http://{host}"), now()).unwrap().token;
    let mut socket = open_socket(port, &host, &session_cookie(port, &host, &token));
    assert_eq!(frame(&mut socket).0, TEXT);
    companion.forget_all();
    let (op, payload) = frame(&mut socket);
    assert_eq!(op, CLOSE);
    assert_eq!(close_code(&payload), 4401);
}

fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as i64
}
