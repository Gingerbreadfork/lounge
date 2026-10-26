//! The worker outlives a job that panics: a listener that panics on one snapshot gets the next.

use lounge_core::{AppRecord, Core, CoreConfig, DeviceTier, HomeSnapshot, LayoutCommand, Platform, PlatformError, SnapshotListener};
use parking_lot::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

struct Quiet;

impl Platform for Quiet {
    fn launch_app(&self, _: String, _: String) -> Result<(), PlatformError> { Ok(()) }
    fn view_uri(&self, _: String) -> Result<(), PlatformError> { Ok(()) }
    fn switch_input(&self, _: String) -> Result<(), PlatformError> { Ok(()) }
    fn tune(&self, _: String) -> Result<(), PlatformError> { Ok(()) }
    fn control(&self, _: String) -> Result<(), PlatformError> { Ok(()) }
    fn open_app_info(&self, _: String) -> Result<(), PlatformError> { Ok(()) }
    fn media_command(&self, _: String, _: String) -> Result<(), PlatformError> { Ok(()) }
    fn request_permission(&self, _: String) -> Result<(), PlatformError> { Ok(()) }
    fn play_stream(&self, _: String, _: String) -> Result<(), PlatformError> { Ok(()) }
    fn send_key(&self, _: String) -> Result<(), PlatformError> { Ok(()) }
    fn type_text(&self, _: String) -> Result<(), PlatformError> { Ok(()) }
}

/// Panics on the first snapshot the worker delivers and keeps the ones after it.
#[derive(Default)]
struct Flaky {
    calls: AtomicUsize,
    last: Mutex<Option<HomeSnapshot>>,
}

impl SnapshotListener for Flaky {
    fn on_snapshot(&self, s: HomeSnapshot) {
        // Call 0 is add_listener's own, on the test thread; call 1 is the worker's first.
        if self.calls.fetch_add(1, Ordering::SeqCst) == 1 {
            panic!("a listener that fails");
        }
        *self.last.lock() = Some(s);
    }
}

fn app(package: &str) -> AppRecord {
    AppRecord {
        package: package.into(),
        activity: format!("{package}.Main"),
        label: package.into(),
        banner_path: None,
        icon_path: None,
        last_used_ms: None,
        installed_ms: 1,
        is_game: false,
        is_leanback: true,
    }
}

fn wait_for(what: &str, mut ok: impl FnMut() -> bool) {
    let started = Instant::now();
    while !ok() {
        assert!(started.elapsed() < Duration::from_secs(15), "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn the_worker_carries_on_after_a_panic() {
    let dir = tempfile::tempdir().unwrap();
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
    let core = Core::open(config, Arc::new(Quiet)).unwrap();
    let listener = Arc::new(Flaky::default());
    core.add_listener(listener.clone());

    core.ingest_apps(vec![app("one.app")]);
    wait_for("the snapshot that panics", || listener.calls.load(Ordering::SeqCst) >= 2);

    core.ingest_apps(vec![app("one.app"), app("two.app")]);
    wait_for("a snapshot after the panic", || {
        listener.last.lock().as_ref().is_some_and(|s| s.rows.iter().any(|r| r.id == "allapps" && r.tiles.len() == 2))
    });
}

/// Keeps the latest snapshot.
#[derive(Default)]
struct Latest(Mutex<Option<HomeSnapshot>>);

impl SnapshotListener for Latest {
    fn on_snapshot(&self, s: HomeSnapshot) {
        *self.0.lock() = Some(s);
    }
}

#[test]
fn a_pin_past_the_stage_s_places_replaces_the_oldest() {
    let dir = tempfile::tempdir().unwrap();
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
    let core = Core::open(config, Arc::new(Quiet)).unwrap();
    let latest = Arc::new(Latest::default());
    core.add_listener(latest.clone());
    let packages: Vec<String> = (1..=7).map(|n| format!("app{n}.tv")).collect();
    core.ingest_apps(packages.iter().map(|p| app(p)).collect());
    for p in &packages {
        core.command(LayoutCommand::PinToStage { id: format!("app:{p}") });
    }
    let pinned = |s: &HomeSnapshot| -> Vec<String> {
        let all = s.rows.iter().find(|r| r.id == "allapps").map(|r| r.tiles.clone()).unwrap_or_default();
        all.iter().filter(|t| t.stage_pinned).map(|t| t.id.clone()).collect()
    };
    wait_for("the last pin", || latest.0.lock().as_ref().is_some_and(|s| pinned(s).contains(&"app:app7.tv".to_string())));
    let now = pinned(latest.0.lock().as_ref().unwrap());
    assert_eq!(now.len(), 6);
    assert!(!now.contains(&"app:app1.tv".to_string()));
}
