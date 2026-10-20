//! The server over a real socket: what an unpaired client gets, and what the cookie opens.

use lounge_companion::{Companion, CompanionActions, DeviceInfo, LayoutDoc, LayoutPatch};
use std::io::{Read, Write};
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
        Vec::new()
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
    let token = companion.new_pairing(&format!("http://{host}"), now()).token;
    let (status, head) = send(port, &format!("POST /api/session HTTP/1.1\r\nHost: {host}\r\nAuthorization: Bearer {token}\r\nContent-Length: 0\r\n\r\n"));
    assert_eq!(status, 204);
    assert!(head.contains("set-cookie: lounge_session=") && head.contains("httponly") && head.contains("samesite=strict"), "{head}");
    let cookie = format!("Cookie: lounge_session={token}");
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

fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as i64
}
