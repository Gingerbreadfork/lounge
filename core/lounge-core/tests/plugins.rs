//! The core end to end with a real signed plugin: a DVR-style guide, repeated ids, search, and
//! turning the plugin off.

use lounge_core::{Core, CoreConfig, DeviceTier, HomeSnapshot, Platform, PlatformError, SnapshotListener};
use parking_lot::Mutex;
use std::sync::Arc;
use std::time::{Duration, Instant};

#[derive(Default)]
struct Recorder {
    calls: Mutex<Vec<String>>,
}

impl Recorder {
    fn note(&self, s: String) -> Result<(), PlatformError> {
        self.calls.lock().push(s);
        Ok(())
    }
}

impl Platform for Recorder {
    fn launch_app(&self, package: String, _: String) -> Result<(), PlatformError> {
        self.note(format!("launch {package}"))
    }
    fn view_uri(&self, uri: String) -> Result<(), PlatformError> {
        self.note(format!("view {uri}"))
    }
    fn switch_input(&self, id: String) -> Result<(), PlatformError> {
        self.note(format!("input {id}"))
    }
    fn tune(&self, uri: String) -> Result<(), PlatformError> {
        self.note(format!("tune {uri}"))
    }
    fn control(&self, c: String) -> Result<(), PlatformError> {
        self.note(format!("control {c}"))
    }
    fn open_app_info(&self, p: String) -> Result<(), PlatformError> {
        self.note(format!("info {p}"))
    }
    fn media_command(&self, p: String, c: String) -> Result<(), PlatformError> {
        self.note(format!("media {p} {c}"))
    }
    fn request_permission(&self, n: String) -> Result<(), PlatformError> {
        self.note(format!("permission {n}"))
    }
    fn play_stream(&self, url: String, title: String) -> Result<(), PlatformError> {
        self.note(format!("play {url} {title}"))
    }
    fn send_key(&self, k: String) -> Result<(), PlatformError> {
        self.note(format!("key {k}"))
    }
    fn type_text(&self, t: String) -> Result<(), PlatformError> {
        self.note(format!("text {t}"))
    }
}

struct Snapshots(Mutex<Option<HomeSnapshot>>);

impl SnapshotListener for Snapshots {
    fn on_snapshot(&self, s: HomeSnapshot) {
        *self.0.lock() = Some(s);
    }
}

const ID: &str = "tv.lounge.testdvr";

fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as i64
}

/// A module whose exports return constant JSON.
fn module(manifest: &str, items: &str, found: &str) -> Vec<u8> {
    let esc = |s: &str| s.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', " ");
    let m = format!(r#"{{"ok":{manifest}}}"#);
    let i = format!(r#"{{"ok":{items}}}"#);
    let f = format!(r#"{{"ok":{found}}}"#);
    let (mo, io, fo) = (65536, 65536 + m.len() + 16, 65536 + m.len() + i.len() + 32);
    wat::parse_str(format!(
        r#"(module
          (memory (export "memory") 4)
          (global $heap (mut i32) (i32.const 200000))
          (data (i32.const {mo}) "{me}")
          (data (i32.const {io}) "{ie}")
          (data (i32.const {fo}) "{fe}")
          (func (export "lg_alloc") (param $len i32) (result i32)
            (local $p i32)
            (local.set $p (global.get $heap))
            (global.set $heap (i32.add (global.get $heap) (i32.add (local.get $len) (i32.const 8))))
            (local.get $p))
          (func (export "lg_free") (param i32 i32))
          (func (export "lg_manifest") (result i64) (i64.or (i64.shl (i64.const {mo}) (i64.const 32)) (i64.const {ml})))
          (func (export "lg_refresh") (param i32 i32) (result i64) (i64.or (i64.shl (i64.const {io}) (i64.const 32)) (i64.const {il})))
          (func (export "lg_search") (param i32 i32) (result i64) (i64.or (i64.shl (i64.const {fo}) (i64.const 32)) (i64.const {fl}))))"#,
        me = esc(&m),
        ie = esc(&i),
        fe = esc(&f),
        ml = m.len(),
        il = i.len(),
        fl = f.len(),
    ))
    .unwrap()
}

fn bundle(dir: &std::path::Path) -> std::path::PathBuf {
    let manifest = format!(r#"{{"id":"{ID}","name":"Test DVR","version":"1","kinds":["epg","search"],"fetch_hosts":[]}}"#);
    let t = now();
    let items = format!(
        r#"[{{"id":"ch1","kind":"channel","title":"One","channel_id":"1","channel_number":"1","action":{{"play":"http://tuner/1"}}}},
            {{"id":"ch1","kind":"channel","title":"Repeat","channel_id":"1","action":{{"play":"http://tuner/dup"}}}},
            {{"id":"s1","kind":"schedule","title":"News","channel_id":"1","start_ms":{},"end_ms":{}}},
            {{"id":"s2","kind":"schedule","title":"Late film","channel_id":"1","start_ms":{},"end_ms":{}}}]"#,
        t - 600_000,
        t + 3_000_000,
        t + 3_000_000,
        t + 9_000_000
    );
    let found = r#"[{"id":"s1","kind":"schedule","title":"News","channel_id":"1"},{"id":"m1","title":"A film","action":{"play":"http://dvr/m1"}}]"#;
    signed(dir, &manifest, &module(&manifest, &items, found))
}

fn signed(dir: &std::path::Path, manifest: &str, wasm: &[u8]) -> std::path::PathBuf {
    use ed25519_dalek::{Signer, SigningKey};
    let key = SigningKey::from_bytes(&[7u8; 32]);
    let sig = key.sign(&lounge_plugins::bundle::Bundle::signed_bytes(manifest.as_bytes(), wasm));
    let path = dir.join("testdvr.lgp");
    lounge_plugins::bundle::Bundle::write(&path, manifest.as_bytes(), wasm, &key.verifying_key().to_bytes(), &sig.to_bytes()).unwrap();
    path
}

fn wait_for(what: &str, mut ok: impl FnMut() -> bool) {
    let started = Instant::now();
    while !ok() {
        assert!(started.elapsed() < Duration::from_secs(15), "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn a_dvr_plugin_fills_the_guide_and_answers_search() {
    let dir = tempfile::tempdir().unwrap();
    let config = CoreConfig {
        data_dir: dir.path().join("data").to_string_lossy().into_owned(),
        cache_dir: dir.path().join("cache").to_string_lossy().into_owned(),
        tier: DeviceTier::A,
        utc_offset_minutes: 0,
        has_tv_controls: false,
        device_name: "TV".into(),
        screen_width: 1920,
        screen_height: 1080,
    };
    let platform = Arc::new(Recorder::default());
    let core = Core::open(config, platform.clone()).unwrap();
    let snapshots = Arc::new(Snapshots(Mutex::new(None)));
    core.add_listener(snapshots.clone());

    let info = core.install_plugin(bundle(dir.path()).to_string_lossy().into_owned(), false).unwrap();
    assert_eq!(info.id, ID);
    core.set_plugin_enabled(ID.into(), true);

    // The guide gets the channel once, under its first title, with both programmes.
    wait_for("the guide", || core.guide(now() - 3_600_000, 6).channels.iter().any(|c| c.programmes.len() == 2));
    let guide = core.guide(now() - 3_600_000, 6);
    assert_eq!(guide.channels.len(), 1);
    assert_eq!(guide.channels[0].name, "One");
    let titles: Vec<&str> = guide.channels[0].programmes.iter().map(|p| p.title.as_str()).collect();
    assert_eq!(titles, vec!["News", "Late film"]);
    assert!(guide.channels[0].programmes[0].is_now);
    assert!(core.plugin_log(ID.into()).iter().any(|l| l.contains("repeated an id")));

    // Live now shows what is airing; a programme with no action plays its channel.
    wait_for("Live now", || snapshots.0.lock().as_ref().is_some_and(|s| s.rows.iter().any(|r| r.id == "live")));
    core.perform(format!("open:plugin:{ID}:s1")).unwrap();
    assert_eq!(platform.calls.lock().last().map(String::as_str), Some("play http://tuner/1 One"));

    // Search: one row from the plugin, and its tiles open.
    let rows = core.search_plugins("news".into());
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].title, "Test DVR");
    let tiles: Vec<&str> = rows[0].tiles.iter().map(|t| t.title.as_str()).collect();
    assert_eq!(tiles, vec!["News", "A film"]);
    core.perform(rows[0].tiles[1].action.clone()).unwrap();
    assert_eq!(platform.calls.lock().last().map(String::as_str), Some("play http://dvr/m1 A film"));
    core.perform(rows[0].tiles[0].action.clone()).unwrap();
    assert_eq!(platform.calls.lock().last().map(String::as_str), Some("play http://tuner/1 One"));
    assert!(core.search_plugins("x".into()).is_empty());

    // Turned off, everything it published goes.
    core.set_plugin_enabled(ID.into(), false);
    wait_for("the guide to empty", || core.guide(now() - 3_600_000, 6).channels.is_empty());
    assert!(core.search_plugins("news".into()).is_empty());
}

#[test]
fn live_now_carries_a_row_of_tiles_and_the_guide_the_rest() {
    let dir = tempfile::tempdir().unwrap();
    let config = CoreConfig {
        data_dir: dir.path().join("data").to_string_lossy().into_owned(),
        cache_dir: dir.path().join("cache").to_string_lossy().into_owned(),
        tier: DeviceTier::A,
        utc_offset_minutes: 0,
        has_tv_controls: false,
        device_name: "TV".into(),
        screen_width: 1920,
        screen_height: 1080,
    };
    let core = Core::open(config, Arc::new(Recorder::default())).unwrap();
    let snapshots = Arc::new(Snapshots(Mutex::new(None)));
    core.add_listener(snapshots.clone());

    let manifest = format!(r#"{{"id":"{ID}","name":"Test DVR","version":"1","kinds":["epg"],"fetch_hosts":[]}}"#);
    let channels: Vec<String> = (1..=80)
        .map(|n| format!(r#"{{"id":"ch{n}","kind":"channel","title":"Channel {n}","channel_id":"{n}","channel_number":"{n}","action":{{"play":"http://tuner/{n}"}}}}"#))
        .collect();
    let wasm = module(&manifest, &format!("[{}]", channels.join(",")), "[]");
    core.install_plugin(signed(dir.path(), &manifest, &wasm).to_string_lossy().into_owned(), false).unwrap();
    core.set_plugin_enabled(ID.into(), true);

    wait_for("the guide", || core.guide(now(), 6).channels.len() == 80);
    wait_for("Live now", || snapshots.0.lock().as_ref().is_some_and(|s| s.rows.iter().any(|r| r.id == "live")));
    let live = snapshots.0.lock().as_ref().unwrap().rows.iter().find(|r| r.id == "live").unwrap().tiles.len();
    assert_eq!(live, 60);
}
