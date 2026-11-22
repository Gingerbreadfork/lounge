//! The core writes in the language it is told, keeps to it across a restart, and goes back to
//! English when asked. These change the language of the whole process, so they sit in a test
//! binary of their own and in one test.

use lounge_core::{AppRecord, Core, CoreConfig, DeviceTier, InputRecord, InputState, Platform, PlatformError};
use std::path::Path;
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
    fn play_stream(&self, _: String, _: String, _: Option<i64>, _: std::collections::HashMap<String, String>, _: bool) -> Result<(), PlatformError> { Ok(()) }
    fn send_key(&self, _: String) -> Result<(), PlatformError> { Ok(()) }
    fn type_text(&self, _: String) -> Result<(), PlatformError> { Ok(()) }
    fn preferences(&self) -> Result<std::collections::HashMap<String, String>, PlatformError> { Ok(Default::default()) }
    fn restore_preferences(&self, _: std::collections::HashMap<String, String>) -> Result<(), PlatformError> { Ok(()) }
    fn tv_controls(&self) -> Result<lounge_core::TvControls, PlatformError> { Ok(lounge_core::TvControls::default()) }
    fn run_tv_control(&self, _: String) -> Result<(), PlatformError> { Ok(()) }
}

fn config(dir: &Path) -> CoreConfig {
    CoreConfig {
        data_dir: dir.to_string_lossy().into_owned(),
        cache_dir: dir.join("cache").to_string_lossy().into_owned(),
        tier: DeviceTier::B,
        utc_offset_minutes: 0,
        has_tv_controls: false,
        device_name: "Test TV".into(),
        screen_width: 1920,
        screen_height: 1080,
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

fn tuner() -> InputRecord {
    InputRecord { id: "tuner".into(), label: String::new(), custom_label: None, kind: "tuner".into(), hdmi_port: None, state: InputState::Connected, package: "tv.input".into(), passthrough: false }
}

fn wait_for(what: &str, mut ok: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while !ok() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// The header of the row of apps and the name of a tuner the TV gave none, as the home shows them.
fn shown(core: &Core) -> (Option<String>, Option<String>) {
    let home = core.snapshot();
    let apps = home.rows.iter().find(|row| row.id == "apps").map(|row| row.title.clone());
    let tuner = home.rows.iter().flat_map(|row| &row.tiles).find(|tile| tile.id == "input:tuner").map(|tile| tile.title.clone());
    (apps, tuner)
}

#[test]
fn the_home_is_written_in_the_language_it_is_told() {
    let dir = tempfile::tempdir().unwrap();
    let core = Core::open(config(dir.path()), Arc::new(Quiet)).unwrap();
    core.ingest_apps(vec![app("tv.example.one")]);
    core.ingest_inputs(vec![tuner()]);
    let english = (Some("Your apps".to_string()), Some("Live TV".to_string()));
    wait_for("the English home", || shown(&core) == english);
    assert_eq!(core.language(), "en");

    // A region Lounge has no translation of its own for reads as the language.
    assert_eq!(core.set_locale("de-AT".into()), "de");
    wait_for("the German home", || {
        let (apps, tuner) = shown(&core);
        apps.is_some() && tuner.is_some() && apps != english.0 && tuner != english.1
    });
    let german = shown(&core);

    // What was stored was written in German, so a restart starts in German before it is told anything.
    drop(core);
    assert_eq!(lounge_words::speak("en", &[]), "en");
    let core = Core::open(config(dir.path()), Arc::new(Quiet)).unwrap();
    assert_eq!(core.language(), "de");
    wait_for("the German home after a restart", || shown(&core).0 == german.0);

    // A language Lounge does not ship is English.
    assert_eq!(core.set_locale("ja".into()), "en");
    wait_for("the English row of apps again", || shown(&core).0 == english.0);
    // The inputs the core was given before the restart went with it, so the TV sends them again.
    core.ingest_inputs(vec![tuner()]);
    wait_for("the English home again", || shown(&core) == english);
    // From here a change of language rewords them with nothing sent again.
    assert_eq!(core.set_locale("de".into()), "de");
    wait_for("the German tuner", || shown(&core) == german);
    assert_eq!(core.set_locale("en".into()), "en");
    wait_for("the English tuner", || shown(&core) == english);
}
