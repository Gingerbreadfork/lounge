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
    fn play_stream(&self, url: String, title: String, start_ms: Option<i64>, headers: std::collections::HashMap<String, String>, audio: bool) -> Result<(), PlatformError> {
        let mut headers: Vec<String> = headers.iter().map(|(name, value)| format!(" with {name}: {value}")).collect();
        headers.sort();
        self.note(format!("play {url} {title}{}{}{}", start_ms.map(|s| format!(" from {s}")).unwrap_or_default(), if audio { " as audio" } else { "" }, headers.concat()))
    }
    fn send_key(&self, k: String) -> Result<(), PlatformError> {
        self.note(format!("key {k}"))
    }
    fn type_text(&self, t: String) -> Result<(), PlatformError> {
        self.note(format!("text {t}"))
    }
    fn preferences(&self) -> Result<std::collections::HashMap<String, String>, PlatformError> {
        Ok(Default::default())
    }
    fn restore_preferences(&self, _: std::collections::HashMap<String, String>) -> Result<(), PlatformError> {
        Ok(())
    }
    fn tv_controls(&self) -> Result<lounge_core::TvControls, PlatformError> { Ok(lounge_core::TvControls::default()) }
    fn run_tv_control(&self, _: String) -> Result<(), PlatformError> { Ok(()) }
    fn phone_view(&self, _: String) -> Result<String, PlatformError> { Ok("{}".into()) }
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
    let found = r#"[{"id":"s1","kind":"schedule","title":"News","channel_id":"1"},{"id":"m1","title":"A film","action":{"play":"http://dvr/m1","start_ms":600000}}]"#;
    signed(dir, &manifest, &module(&manifest, &items, found))
}

fn signed(dir: &std::path::Path, manifest: &str, wasm: &[u8]) -> std::path::PathBuf {
    signed_by(7, dir, manifest, wasm)
}

/// As `signed`, by the publisher whose key is `seed` repeated.
fn signed_by(seed: u8, dir: &std::path::Path, manifest: &str, wasm: &[u8]) -> std::path::PathBuf {
    use ed25519_dalek::{Signer, SigningKey};
    let key = SigningKey::from_bytes(&[seed; 32]);
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
    assert_eq!(platform.calls.lock().last().map(String::as_str), Some("play http://dvr/m1 A film from 600000"));
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

#[test]
fn a_plugin_that_fell_due_while_closed_refreshes_on_opening() {
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
    let core = Core::open(config.clone(), Arc::new(Recorder::default())).unwrap();
    let manifest = format!(r#"{{"id":"{ID}","name":"Test DVR","version":"1","kinds":["epg"],"fetch_hosts":[]}}"#);
    core.install_plugin(signed(dir.path(), &manifest, &module(&manifest, "[]", "[]")).to_string_lossy().into_owned(), false).unwrap();
    core.set_plugin_enabled(ID.into(), true);
    wait_for("the first refresh", || core.plugins()[0].last_refresh_ms.is_some());
    drop(core);

    let db = rusqlite::Connection::open(dir.path().join("data").join("lounge.db")).unwrap();
    db.execute("UPDATE plugins SET last_refresh_ms = 1", []).unwrap();
    drop(db);
    let core = Core::open(config, Arc::new(Recorder::default())).unwrap();
    wait_for("a refresh on opening", || core.plugins()[0].last_refresh_ms.is_some_and(|ms| ms > 1));
}

#[test]
fn an_update_never_rolls_back_and_is_turned_off_when_it_asks_for_more() {
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
    let version = |v: &str, hosts: &str| {
        let manifest = format!(r#"{{"id":"{ID}","name":"Test DVR","version":"{v}","kinds":["epg"],"fetch_hosts":[{hosts}]}}"#);
        let wasm = module(&manifest, "[]", "[]");
        signed(dir.path(), &manifest, &wasm).to_string_lossy().into_owned()
    };
    core.install_plugin(version("2", r#""a.example""#), false).unwrap();
    core.set_plugin_enabled(ID.into(), true);
    assert!(core.install_plugin(version("1.9", r#""a.example""#), false).is_err());
    assert!(core.install_plugin(version("2.1", r#""a.example""#), false).unwrap().enabled);
    let wider = core.install_plugin(version("3", r#""a.example","b.example""#), false).unwrap();
    assert!(!wider.enabled);
    assert!(core.plugin_log(ID.into()).iter().any(|l| l.contains("b.example")));
    // The screens are told why, through further updates, until the person turns it on.
    assert_eq!(wider.update_asks_to_reach, vec!["b.example"]);
    let further = core.install_plugin(version("4", r#""a.example","b.example","c.example""#), false).unwrap();
    assert_eq!(further.update_asks_to_reach, vec!["b.example", "c.example"]);
    assert_eq!(core.plugins()[0].update_asks_to_reach, vec!["b.example", "c.example"]);
    core.set_plugin_enabled(ID.into(), true);
    assert!(core.plugins()[0].enabled && core.plugins()[0].update_asks_to_reach.is_empty());
    // One the person turned off takes a wider update without being held back.
    core.set_plugin_enabled(ID.into(), false);
    assert!(core.install_plugin(version("5", r#""a.example","b.example","c.example","d.example""#), false).unwrap().update_asks_to_reach.is_empty());
    // One turned off by an update goes back on when a later update asks for nothing more.
    core.set_plugin_enabled(ID.into(), true);
    let held = core.install_plugin(version("6", r#""a.example","b.example","c.example","d.example","e.example""#), false).unwrap();
    assert!(!held.enabled);
    assert_eq!(held.update_asks_to_reach, vec!["e.example"]);
    let back = core.install_plugin(version("7", r#""a.example","b.example","c.example","d.example""#), false).unwrap();
    assert!(back.enabled && back.update_asks_to_reach.is_empty());
    assert!(core.plugins()[0].enabled && core.plugins()[0].update_asks_to_reach.is_empty());
    assert!(core.plugin_log(ID.into()).iter().any(|l| l.contains("turned back on")));
}

#[test]
fn only_a_built_in_plugin_follows_a_new_apk_to_another_key() {
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
    let bundle = |id: &str, version: &str, seed: u8| {
        let manifest = format!(r#"{{"id":"{id}","name":"Test DVR","version":"{version}","kinds":["epg"],"fetch_hosts":[]}}"#);
        signed_by(seed, dir.path(), &manifest, &module(&manifest, "[]", "[]")).to_string_lossy().into_owned()
    };

    // One that came with Lounge takes the next APK's copy, signed elsewhere, and stays on.
    let first = core.install_plugin(bundle(ID, "1", 7), true).unwrap();
    core.set_plugin_enabled(ID.into(), true);
    let moved = core.install_plugin(bundle(ID, "1.1", 8), true).unwrap();
    assert_ne!(moved.publisher, first.publisher);
    assert_eq!(moved.version, "1.1");
    assert!(moved.enabled && moved.built_in);
    // Nobody else's copy replaces it.
    assert!(core.install_plugin(bundle(ID, "2", 9), false).is_err());

    // One the person installed is never replaced by a copy signed by someone else.
    let theirs = "tv.lounge.theirs";
    core.install_plugin(bundle(theirs, "1", 7), false).unwrap();
    assert!(core.install_plugin(bundle(theirs, "2", 8), true).is_err());
    assert!(core.install_plugin(bundle(theirs, "2", 8), false).is_err());
    assert!(core.install_plugin(bundle(theirs, "2", 7), false).is_ok());
}

const SLOW_ID: &str = "tv.lounge.testslow";

/// A plugin with one tile that resolves at the press, and a search that spins for a long while
/// when the query is a long one and answers at once when it is short.
fn slow_search_bundle(dir: &std::path::Path) -> std::path::PathBuf {
    let manifest = format!(r#"{{"id":"{SLOW_ID}","name":"Slow","version":"1","kinds":["content","search"],"fetch_hosts":[],"rows":[{{"id":"r","title":"Films on Slow"}}]}}"#);
    let items = r#"[{"id":"p1","title":"A film","row":"r","action":{"resolve":"p1"}}]"#;
    let found = r#"[{"id":"f1","title":"Found","action":{"play":"http://slow/f1"}}]"#;
    let opened = r#"{"ok":{"play":"http://slow/p1","title":"A film"}}"#;
    let esc = |s: &str| s.replace('\\', "\\\\").replace('"', "\\\"");
    let m = format!(r#"{{"ok":{manifest}}}"#);
    let i = format!(r#"{{"ok":{items}}}"#);
    let f = format!(r#"{{"ok":{found}}}"#);
    let (mo, io, fo, oo) = (65536, 70000, 75000, 80000);
    let wasm = wat::parse_str(format!(
        r#"(module
          (memory (export "memory") 4)
          (global $heap (mut i32) (i32.const 200000))
          (data (i32.const {mo}) "{me}")
          (data (i32.const {io}) "{ie}")
          (data (i32.const {fo}) "{fe}")
          (data (i32.const {oo}) "{oe}")
          (func (export "lg_alloc") (param $len i32) (result i32)
            (local $p i32)
            (local.set $p (global.get $heap))
            (global.set $heap (i32.add (global.get $heap) (i32.add (local.get $len) (i32.const 8))))
            (local.get $p))
          (func (export "lg_free") (param i32 i32))
          (func (export "lg_manifest") (result i64) (i64.or (i64.shl (i64.const {mo}) (i64.const 32)) (i64.const {ml})))
          (func (export "lg_refresh") (param i32 i32) (result i64) (i64.or (i64.shl (i64.const {io}) (i64.const 32)) (i64.const {il})))
          (func (export "lg_resolve") (param i32 i32) (result i64) (i64.or (i64.shl (i64.const {oo}) (i64.const 32)) (i64.const {ol})))
          (func (export "lg_search") (param $ptr i32) (param $len i32) (result i64)
            (local $n i64)
            (if (i32.gt_u (local.get $len) (i32.const 50))
              (then (loop $spin
                (local.set $n (i64.add (local.get $n) (i64.const 1)))
                (br_if $spin (i64.lt_u (local.get $n) (i64.const 4000000000))))))
            (i64.or (i64.shl (i64.const {fo}) (i64.const 32)) (i64.const {fl}))))"#,
        me = esc(&m),
        ie = esc(&i),
        fe = esc(&f),
        oe = esc(opened),
        ml = m.len(),
        il = i.len(),
        fl = f.len(),
        ol = opened.len(),
    ))
    .unwrap();
    signed(dir, &manifest, &wasm)
}

/// A query long enough for the slow plugin's search to spin on.
const LONG_QUERY: &str = "a query long enough to keep the slow plugin busy";

fn slow_core(dir: &std::path::Path) -> (Arc<Core>, Arc<Recorder>) {
    let config = CoreConfig {
        data_dir: dir.join("data").to_string_lossy().into_owned(),
        cache_dir: dir.join("cache").to_string_lossy().into_owned(),
        tier: DeviceTier::B,
        utc_offset_minutes: 0,
        has_tv_controls: false,
        device_name: "TV".into(),
        screen_width: 1920,
        screen_height: 1080,
    };
    let platform = Arc::new(Recorder::default());
    let core = Core::open(config, platform.clone()).unwrap();
    core.install_plugin(slow_search_bundle(dir).to_string_lossy().into_owned(), false).unwrap();
    core.set_plugin_enabled(SLOW_ID.into(), true);
    let tile = format!("plugin:{SLOW_ID}:p1");
    wait_for("the slow plugin's tile", || core.snapshot().rows.iter().any(|r| r.tiles.iter().any(|t| t.id == tile)));
    (core, platform)
}

#[test]
fn a_press_does_not_wait_for_a_search() {
    let dir = tempfile::tempdir().unwrap();
    let (core, platform) = slow_core(dir.path());
    let searched = std::sync::atomic::AtomicBool::new(false);
    std::thread::scope(|scope| {
        scope.spawn(|| {
            core.search_plugins(LONG_QUERY.into());
            searched.store(true, std::sync::atomic::Ordering::SeqCst);
        });
        std::thread::sleep(Duration::from_millis(300));
        let pressed = Instant::now();
        core.perform(format!("open:plugin:{SLOW_ID}:p1")).unwrap();
        assert!(pressed.elapsed() < Duration::from_secs(3), "the press took {:?}", pressed.elapsed());
        assert!(!searched.load(std::sync::atomic::Ordering::SeqCst), "the search was meant to be still running");
        assert_eq!(platform.calls.lock().as_slice(), ["play http://slow/p1 A film"]);
        // A short search replaces the long one, which lets the thread above finish.
        core.search_plugins("go".into());
    });
}

#[test]
fn a_newer_search_stops_the_one_before_it() {
    let dir = tempfile::tempdir().unwrap();
    let (core, _platform) = slow_core(dir.path());
    std::thread::scope(|scope| {
        let first = scope.spawn(|| core.search_plugins(LONG_QUERY.into()));
        std::thread::sleep(Duration::from_millis(300));
        let asked = Instant::now();
        let rows = core.search_plugins("go".into());
        assert!(asked.elapsed() < Duration::from_secs(3), "the newer search waited {:?} for the older one", asked.elapsed());
        assert_eq!(rows.iter().flat_map(|r| &r.tiles).map(|t| t.title.as_str()).collect::<Vec<_>>(), ["Found"]);
        assert!(first.join().unwrap().is_empty(), "a search that was replaced has nothing to show");
    });
}

#[test]
fn a_stream_reaches_the_player_with_its_headers_and_its_kind() {
    let dir = tempfile::tempdir().unwrap();
    let id = "tv.lounge.teststreams";
    let manifest = format!(r#"{{"id":"{id}","name":"Streams","version":"1","kinds":["content"],"fetch_hosts":[],"rows":[{{"id":"r","title":"Stations on Streams"}}]}}"#);
    let items = r#"[{"id":"radio","title":"A station","row":"r","action":{"play":"http://radio/live","media":"audio","headers":{"User-Agent":"Lounge","Bad name":"x"}}},
        {"id":"film","title":"A film","row":"r","action":{"play":"http://films/1"}}]"#;
    let config = CoreConfig {
        data_dir: dir.path().join("data").to_string_lossy().into_owned(),
        cache_dir: dir.path().join("cache").to_string_lossy().into_owned(),
        tier: DeviceTier::B,
        utc_offset_minutes: 0,
        has_tv_controls: false,
        device_name: "TV".into(),
        screen_width: 1920,
        screen_height: 1080,
    };
    let platform = Arc::new(Recorder::default());
    let core = Core::open(config, platform.clone()).unwrap();
    core.install_plugin(signed(dir.path(), &manifest, &module(&manifest, items, "[]")).to_string_lossy().into_owned(), false).unwrap();
    core.set_plugin_enabled(id.into(), true);
    wait_for("the streams", || core.snapshot().rows.iter().any(|r| r.tiles.len() == 2));

    core.perform(format!("open:plugin:{id}:radio")).unwrap();
    core.perform(format!("open:plugin:{id}:film")).unwrap();
    // A header that is not well formed is left behind, and a stream with nothing said is video.
    assert_eq!(platform.calls.lock().as_slice(), ["play http://radio/live A station as audio with User-Agent: Lounge", "play http://films/1 A film"]);
    assert!(core.plugin_log(id.into()).iter().any(|line| line.contains("1 headers on play actions were left out")), "{:?}", core.plugin_log(id.into()));
}


const LAMP_ID: &str = "tv.lounge.testlamp";

/// A plugin for one lamp. Its state lives in the plugin's own storage: a press flips it and says
/// so, a refresh reads it, and a sheet describes the room.
fn lamp_bundle(dir: &std::path::Path) -> std::path::PathBuf {
    lamp_bundle_answering(dir, r#"{"ok":{"say":"Lamp switched, as asked. See http://den.local/lamp?token=secret for more"}}"#, "")
}

/// The lamp plugin with another answer to every press, and more items after its own.
fn lamp_bundle_answering(dir: &std::path::Path, said: &str, more_items: &str) -> std::path::PathBuf {
    let manifest = format!(r#"{{"id":"{LAMP_ID}","name":"Lamps","version":"1","kinds":["content"],"fetch_hosts":[],"rows":[{{"id":"here","title":"This room","style":"compact"}}]}}"#);
    let items = |on: bool| {
        format!(
            r##"[{{"id":"lamp","title":"Lamp","subtitle":"{}","row":"here","row_title":"Den","glyph":"light","on":{on},"tint":"#3366ff","more":"lamp","weight":3,"action":{{"resolve":"toggle"}}}},
                {{"id":"room","title":"Den","row":"here","glyph":"home","weight":2,"action":{{"sheet":"room"}}}},
                {{"id":"door","title":"Door","subtitle":"Closed","row":"here","glyph":"door","weight":1,"on":false}}{more_items}]"##,
            if on { "On" } else { "Off" }
        )
    };
    let sheet = r#"{"ok":{"title":"Den","subtitle":"1 light","rows":[
        {"id":"lamp","kind":"press","title":"Lamp","glyph":"light","value":"Off","on":false,"resolve":"toggle"},
        {"id":"level","kind":"step","title":"Brightness","value":"40%","down":"dim","up":"brighten"},
        {"id":"more","kind":"open","title":"More","sheet":"more"},
        {"id":"unlock","kind":"confirm","title":"Unlock the door","resolve":"unlock","ask":"Unlock the door?"},
        {"id":"lamp","kind":"press","title":"Lamp again","resolve":"toggle"},
        {"id":"dial","kind":"dial","title":"Something newer"}]}}"#;
    let esc = |s: &str| s.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', " ");
    let m = format!(r#"{{"ok":{manifest}}}"#);
    let off = format!(r#"{{"ok":{}}}"#, items(false));
    let on = format!(r#"{{"ok":{}}}"#, items(true));
    let (mo, fo, no, so, sho) = (65536, 70000, 75000, 80000, 85000);
    let wasm = wat::parse_str(format!(
        r#"(module
          (import "lounge" "kv_get" (func $kv_get (param i32 i32) (result i64)))
          (import "lounge" "kv_set" (func $kv_set (param i32 i32 i32 i32)))
          (import "lounge" "kv_delete" (func $kv_delete (param i32 i32)))
          (memory (export "memory") 4)
          (global $heap (mut i32) (i32.const 200000))
          (data (i32.const 60000) "on")
          (data (i32.const {mo}) "{me}")
          (data (i32.const {fo}) "{fe}")
          (data (i32.const {no}) "{ne}")
          (data (i32.const {so}) "{se}")
          (data (i32.const {sho}) "{she}")
          (func (export "lg_alloc") (param $len i32) (result i32)
            (local $p i32)
            (local.set $p (global.get $heap))
            (global.set $heap (i32.add (global.get $heap) (i32.add (local.get $len) (i32.const 8))))
            (local.get $p))
          (func (export "lg_free") (param i32 i32))
          (func (export "lg_manifest") (result i64) (i64.or (i64.shl (i64.const {mo}) (i64.const 32)) (i64.const {ml})))
          (func (export "lg_refresh") (param i32 i32) (result i64)
            (if (result i64) (i64.eqz (call $kv_get (i32.const 60000) (i32.const 2)))
              (then (i64.or (i64.shl (i64.const {fo}) (i64.const 32)) (i64.const {fl})))
              (else (i64.or (i64.shl (i64.const {no}) (i64.const 32)) (i64.const {nl})))))
          (func (export "lg_resolve") (param i32 i32) (result i64)
            (if (i64.eqz (call $kv_get (i32.const 60000) (i32.const 2)))
              (then (call $kv_set (i32.const 60000) (i32.const 2) (i32.const 60000) (i32.const 2)))
              (else (call $kv_delete (i32.const 60000) (i32.const 2))))
            (i64.or (i64.shl (i64.const {so}) (i64.const 32)) (i64.const {sl})))
          (func (export "lg_sheet") (param i32 i32) (result i64) (i64.or (i64.shl (i64.const {sho}) (i64.const 32)) (i64.const {shl}))))"#,
        me = esc(&m),
        fe = esc(&off),
        ne = esc(&on),
        se = esc(said),
        she = esc(sheet),
        ml = m.len(),
        fl = off.len(),
        nl = on.len(),
        sl = said.len(),
        shl = sheet.replace('\n', " ").len(),
    ))
    .unwrap();
    signed(dir, &manifest, &wasm)
}

fn lamp_core(dir: &std::path::Path) -> (Arc<Core>, Arc<Recorder>) {
    let bundle = lamp_bundle(dir);
    lamp_core_from(dir, &bundle)
}

fn lamp_core_from(dir: &std::path::Path, bundle: &std::path::Path) -> (Arc<Core>, Arc<Recorder>) {
    let config = CoreConfig {
        data_dir: dir.join("data").to_string_lossy().into_owned(),
        cache_dir: dir.join("cache").to_string_lossy().into_owned(),
        tier: DeviceTier::B,
        utc_offset_minutes: 0,
        has_tv_controls: false,
        device_name: "TV".into(),
        screen_width: 1920,
        screen_height: 1080,
    };
    let platform = Arc::new(Recorder::default());
    let core = Core::open(config, platform.clone()).unwrap();
    core.install_plugin(bundle.to_string_lossy().into_owned(), false).unwrap();
    core.set_plugin_enabled(LAMP_ID.into(), true);
    wait_for("the lamp", || lamp(&core).is_some());
    (core, platform)
}

fn lamp_row(core: &Core) -> Option<lounge_core::Row> {
    core.snapshot().rows.into_iter().find(|r| r.tiles.iter().any(|t| t.id == format!("plugin:{LAMP_ID}:lamp")))
}

fn lamp(core: &Core) -> Option<lounge_core::Tile> {
    lamp_row(core)?.tiles.into_iter().find(|t| t.id == format!("plugin:{LAMP_ID}:lamp"))
}

fn refreshes(core: &Core) -> usize {
    core.plugin_log(LAMP_ID.into()).iter().filter(|line| line.contains("refreshed:")).count()
}

#[test]
fn a_control_shows_its_state_and_a_press_brings_the_new_one() {
    let dir = tempfile::tempdir().unwrap();
    let (core, platform) = lamp_core(dir.path());

    // The row takes the name the refresh gave it.
    let row = lamp_row(&core).unwrap();
    assert_eq!(row.title, "Den");
    assert_eq!(row.kind, lounge_core::TileKind::Compact);
    assert_eq!(row.tiles.iter().map(|t| (t.title.as_str(), t.glyph.as_deref(), t.on)).collect::<Vec<_>>(), [("Lamp", Some("light"), Some(false)), ("Den", Some("home"), None), ("Door", Some("door"), Some(false))]);
    let off = lamp(&core).unwrap();
    assert_eq!(off.more.as_deref(), Some(format!("{LAMP_ID}:lamp").as_str()));
    // A tile to read opens nothing, and one that opens a sheet says so in its action.
    assert_eq!(row.tiles[2].action, "");
    assert_eq!(row.tiles[1].action, format!("control:sheet:{LAMP_ID}:room"));
    // None of them is put forward for the Stage.
    assert!(core.snapshot().stage.strip.iter().all(|t| !t.id.starts_with(&format!("plugin:{LAMP_ID}:"))));

    // A press says what the plugin said, less anything secret in an address, and the tile follows by itself.
    let pressed = Instant::now();
    let done = core.perform(off.action.clone()).unwrap();
    assert_eq!(done.say.as_deref(), Some("Lamp switched, as asked. See http://den.local/lamp for more"));
    wait_for("the lamp to come on", || lamp(&core).is_some_and(|t| t.on == Some(true)));
    assert!(pressed.elapsed() < Duration::from_millis(1500), "the tile took {:?} to follow the press", pressed.elapsed());
    let on = lamp(&core).unwrap();
    assert_eq!(on.subtitle.as_deref(), Some("On"));
    // Lit, it takes its own colour; resting, it has none of its own.
    assert_ne!(on.light_argb, off.light_argb);
    assert!(platform.calls.lock().is_empty(), "{:?}", platform.calls.lock());

    // Opening the room's sheet is handed to the screen.
    core.perform(row.tiles[1].action.clone()).unwrap();
    assert_eq!(platform.calls.lock().as_slice(), [format!("control sheet:{LAMP_ID}:room")]);
}

#[test]
fn presses_in_quick_succession_bring_one_refresh_and_a_second_look() {
    let dir = tempfile::tempdir().unwrap();
    let (core, _platform) = lamp_core(dir.path());
    let before = refreshes(&core);
    let action = lamp(&core).unwrap().action;
    for _ in 0..5 {
        core.perform(action.clone()).unwrap();
    }
    wait_for("the lamp to come on", || lamp(&core).is_some_and(|t| t.on == Some(true)));
    std::thread::sleep(Duration::from_millis(3200));
    assert_eq!(refreshes(&core) - before, 2, "{:?}", core.plugin_log(LAMP_ID.into()));
}

#[test]
fn a_sheet_is_read_row_by_row_and_its_presses_reach_the_plugin() {
    use lounge_core::PluginSheetRowKind as Kind;
    let dir = tempfile::tempdir().unwrap();
    let (core, _platform) = lamp_core(dir.path());

    let sheet = core.plugin_sheet(LAMP_ID.into(), "room".into()).unwrap();
    assert_eq!((sheet.title.as_str(), sheet.subtitle.as_deref()), ("Den", Some("1 light")));
    // A second row under one id is left out, and a kind this TV does not know is a row to read.
    assert_eq!(
        sheet.rows.iter().map(|r| (r.id.as_str(), r.kind.clone())).collect::<Vec<_>>(),
        [
            ("lamp", Kind::Press { action: "toggle".into() }),
            ("level", Kind::Step { down: Some("dim".into()), up: Some("brighten".into()), press: None }),
            ("more", Kind::Open { sheet: "more".into() }),
            ("unlock", Kind::Confirm { action: "unlock".into(), ask: "Unlock the door?".into() }),
            ("dial", Kind::Text),
        ]
    );
    assert_eq!((sheet.rows[0].glyph.as_deref(), sheet.rows[0].value.as_deref(), sheet.rows[0].on), (Some("light"), Some("Off"), Some(false)));

    // A row's press does what a tile's does, and the home follows it.
    let done = core.plugin_sheet_press(LAMP_ID.into(), "toggle".into()).unwrap();
    assert!(done.say.is_some_and(|say| say.starts_with("Lamp switched")));
    wait_for("the lamp to come on", || lamp(&core).is_some_and(|t| t.on == Some(true)));
}

#[test]
fn a_phone_shows_a_sheet_itself_and_presses_its_rows_by_name() {
    let dir = tempfile::tempdir().unwrap();
    let (core, platform) = lamp_core(dir.path());
    let phone = core.as_phone();
    let room = lamp_row(&core).unwrap().tiles[1].action.clone();

    // The room's sheet is handed to the phone, and the TV's screen is left as it is.
    let opened = phone.perform_here(&room).unwrap();
    assert_eq!(opened.sheet.map(|s| (s.plugin, s.id)), Some((LAMP_ID.to_string(), "room".to_string())));
    assert!(platform.calls.lock().is_empty(), "{:?}", platform.calls.lock());

    // The phone is told what each row can do, and none of the actions behind them.
    let sheet: serde_json::Value = serde_json::from_str(&phone.sheet_json(LAMP_ID, "room").unwrap()).unwrap();
    assert_eq!((sheet["title"].as_str(), sheet["subtitle"].as_str()), (Some("Den"), Some("1 light")));
    assert_eq!(sheet["rows"][0], serde_json::json!({ "id": "lamp", "kind": "press", "title": "Lamp", "glyph": "light", "subtitle": null, "value": "Off", "on": false }));
    assert_eq!(sheet["rows"][1], serde_json::json!({ "id": "level", "kind": "step", "title": "Brightness", "glyph": null, "subtitle": null, "value": "40%", "on": null, "down": true, "up": true, "press": false }));
    assert_eq!((sheet["rows"][2]["kind"].as_str(), sheet["rows"][2]["sheet"].as_str()), (Some("open"), Some("more")));
    assert_eq!((sheet["rows"][3]["kind"].as_str(), sheet["rows"][3]["ask"].as_str()), (Some("confirm"), Some("Unlock the door?")));
    assert_eq!(sheet["rows"][4]["kind"].as_str(), Some("text"));
    for action in ["toggle", "dim", "brighten", "resolve", "action"] {
        assert!(!sheet.to_string().contains(action), "{action} reached the phone in {sheet}");
    }

    // A row is pressed by its name, and the home follows.
    let done = phone.sheet_press(LAMP_ID, "room", "lamp", "press").unwrap();
    assert!(done.say.is_some_and(|say| say.starts_with("Lamp switched")));
    wait_for("the lamp to come on", || lamp(&core).is_some_and(|t| t.on == Some(true)));
    assert!(phone.sheet_press(LAMP_ID, "room", "level", "up").is_ok());

    // A row that has no such press, or an action in place of a row, reaches no plugin.
    let changed = "That has changed since it was shown. Close this and open it again.";
    assert_eq!(phone.sheet_press(LAMP_ID, "room", "level", "press").unwrap_err(), changed);
    assert_eq!(phone.sheet_press(LAMP_ID, "room", "toggle", "press").unwrap_err(), changed);
    assert_eq!(phone.sheet_press(LAMP_ID, "room", "dial", "press").unwrap_err(), changed);

    // A picture no press handed out is not fetched, shown on the TV or asked about.
    assert!(phone.live_picture(LAMP_ID, "http://camera.local/1.jpg").is_err());
    assert!(phone.live_on_tv(LAMP_ID, "http://camera.local/1.jpg", "Door").is_err());
    assert!(phone.live_again(LAMP_ID, "http://camera.local/1.jpg").is_err());
    assert!(platform.calls.lock().is_empty(), "{:?}", platform.calls.lock());
}

#[test]
fn a_live_picture_reaches_the_phone_with_what_opened_it_behind_it() {
    let dir = tempfile::tempdir().unwrap();
    // Every press answers with a camera's picture, and one more tile names a picture outright.
    let door = r#"{"ok":{"live":"http://camera.local/door.jpg","title":"Door camera"}}"#;
    let yard = r#",{"id":"yard","title":"Yard","row":"here","weight":0,"action":{"live":"http://camera.local/yard.jpg"}}"#;
    let (core, platform) = lamp_core_from(dir.path(), &lamp_bundle_answering(dir.path(), door, yard));
    let phone = core.as_phone();
    let not_shown = "not a picture that is being shown";

    // The picture a press answered with is the phone's to show, and the TV's screen is left alone.
    let shown = phone.perform_here(&lamp(&core).unwrap().action).unwrap().live.unwrap();
    assert_eq!((shown.plugin.as_str(), shown.title.as_str(), shown.url.as_str()), (LAMP_ID, "Door camera", "http://camera.local/door.jpg"));
    assert!(platform.calls.lock().is_empty(), "{:?}", platform.calls.lock());
    // It may be fetched: what stops it here is the plugin's own list of hosts, not the phone's leave.
    assert_ne!(phone.live_picture(LAMP_ID, &shown.url).unwrap_err(), not_shown);
    // A new address is asked of the action that opened it, and the TV opens it by that action too.
    assert_eq!(phone.live_again(LAMP_ID, &shown.url).unwrap(), shown.url);
    phone.live_on_tv(LAMP_ID, &shown.url, "Anything").unwrap();
    assert_eq!(platform.calls.lock().as_slice(), [format!("control live:{LAMP_ID}\nDoor camera\nhttp://camera.local/door.jpg")]);
    platform.calls.lock().clear();

    // A picture a tile names outright is shown the same, though no action can renew it.
    let tile = lamp_row(&core).unwrap().tiles.into_iter().find(|t| t.title == "Yard").unwrap();
    let named = phone.perform_here(&tile.action).unwrap().live.unwrap();
    assert_eq!((named.title.as_str(), named.url.as_str()), ("Yard", "http://camera.local/yard.jpg"));
    assert!(platform.calls.lock().is_empty(), "{:?}", platform.calls.lock());
    assert_ne!(phone.live_picture(LAMP_ID, &named.url).unwrap_err(), not_shown);
    assert!(phone.live_again(LAMP_ID, &named.url).is_err());
    phone.live_on_tv(LAMP_ID, &named.url, "Yard").unwrap();
    assert_eq!(platform.calls.lock().as_slice(), [format!("control live:{LAMP_ID}\nYard\nhttp://camera.local/yard.jpg")]);

    // An address neither of them gave stays out of reach.
    assert_eq!(phone.live_picture(LAMP_ID, "http://camera.local/other.jpg").unwrap_err(), not_shown);
}

#[test]
fn a_plugin_without_sheets_says_it_has_none() {
    let dir = tempfile::tempdir().unwrap();
    let (core, _platform) = slow_core(dir.path());
    let refused = core.plugin_sheet(SLOW_ID.into(), "anything".into()).unwrap_err();
    assert_eq!(refused.to_string(), "plugin: Slow has nothing more to show here.");
}

#[test]
fn a_live_picture_can_only_be_asked_about_by_what_opened_it() {
    let dir = tempfile::tempdir().unwrap();
    let (core, platform) = lamp_core(dir.path());
    // A control from outside cannot open one, so it cannot name an action to run either.
    let crafted = format!("control:live:{LAMP_ID}\nDoor\nhttp://camera.local/1.jpg\ntoggle");
    assert!(matches!(core.perform(crafted), Err(lounge_core::CoreError::UnknownAction { .. })));
    assert!(platform.calls.lock().is_empty(), "{:?}", platform.calls.lock());
    // A picture the core never opened has no action behind it, and the plugin is not asked.
    let refused = core.plugin_live(LAMP_ID.into(), "http://camera.local/1.jpg".into()).unwrap_err();
    assert_eq!(refused.to_string(), "plugin: The picture has no address now.");
    std::thread::sleep(Duration::from_millis(600));
    assert_eq!(lamp(&core).unwrap().on, Some(false));
}
