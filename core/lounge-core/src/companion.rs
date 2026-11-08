//! The companion server's view of the core: snapshots, actions, layout, plugins, backup.

use crate::{engine, now_ms, plugins, worker, CompanionStatus, Inner, PairingInfo};
use lounge_companion::{AppRef, CompanionActions, DeviceInfo, LayoutDoc, LayoutPatch, NamedRef, RowRef};
use lounge_graph::Layout;
use lounge_plugins::http::{Fetcher, Reach};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Weak};

struct CoreActions {
    inner: Weak<Inner>,
}

impl CoreActions {
    fn inner(&self) -> Result<Arc<Inner>, String> {
        self.inner.upgrade().ok_or_else(|| "the launcher is shutting down".to_string())
    }
}

pub(crate) fn start(inner: &Arc<Inner>, port: u16) -> Result<(), String> {
    let mut slot = inner.companion.lock();
    if slot.as_ref().is_some_and(|c| port == 0 || c.port == port) {
        return Ok(());
    }
    *slot = None;
    let actions: Arc<dyn CompanionActions> = Arc::new(CoreActions { inner: Arc::downgrade(inner) });
    let companion = lounge_companion::Companion::start(port, Path::new(&inner.config.data_dir), actions).map_err(|e| e.to_string())?;
    tracing::info!("companion listening on port {}", companion.port);
    *slot = Some(companion);
    Ok(())
}

pub(crate) fn status(inner: &Arc<Inner>) -> CompanionStatus {
    let slot = inner.companion.lock();
    match slot.as_ref() {
        Some(c) => CompanionStatus {
            running: true,
            url: lounge_companion::local_ip().map(|ip| format!("http://{ip}:{}", c.port)),
            port: c.port,
            paired: c.paired_devices() as u32,
        },
        None => CompanionStatus { running: false, url: None, port: 0, paired: 0 },
    }
}

pub(crate) fn pairing(inner: &Arc<Inner>) -> Result<PairingInfo, String> {
    let slot = inner.companion.lock();
    let c = slot.as_ref().ok_or_else(|| "the companion is not running".to_string())?;
    let ip = lounge_companion::local_ip().ok_or_else(|| "the TV is not on a network".to_string())?;
    let base = format!("http://{ip}:{}", c.port);
    let info = c.new_pairing(&base, now_ms())?;
    let dir = PathBuf::from(&inner.config.cache_dir).join("companion");
    // One live code at a time: earlier codes' QR pictures go, so nothing can read a token off them.
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for e in entries.flatten() {
            if e.file_name().to_string_lossy().starts_with("pair-") {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
    let png = dir.join(format!("pair-{}.png", info.code));
    lounge_companion::qr_png(&info.url, &png, 12).map_err(|e| e.to_string())?;
    Ok(PairingInfo { code: info.code, url: info.url, qr_png_path: png.to_string_lossy().into_owned(), expires_ms: info.expires_ms })
}

fn named(r: crate::ItemRef) -> NamedRef {
    NamedRef { id: r.id, title: r.title, icon_path: r.icon_path }
}

fn layout_doc(inner: &Inner) -> LayoutDoc {
    let state = inner.state.lock();
    let graph = &state.graph;
    let l = &graph.layout;
    // The same rows the TV's own editor lists, in the same order and words.
    let available_rows = engine::row_infos(&state, &inner.config)
        .into_iter()
        .map(|r| RowRef { id: r.id, title: r.title, shown: r.shown, chosen: r.chosen, count: r.count, detail: r.detail })
        .collect();
    let apps = graph
        .all_apps()
        .into_iter()
        .filter_map(|it| {
            let package = it.package()?.to_string();
            let pinned = l.pinned_apps.contains(&package);
            Some(AppRef { package, title: it.title.clone(), icon_path: it.artwork.logo.clone(), pinned })
        })
        .collect();
    LayoutDoc {
        row_order: engine::effective_row_order(&state),
        available_rows,
        hidden: l.hidden.iter().cloned().collect(),
        pinned_apps: l.pinned_apps.clone(),
        stage_pins: l.stage_pins.clone(),
        enabled_channels: l.enabled_channels.iter().copied().collect(),
        row_visibility: l.row_visibility.clone(),
        apps,
        hidden_items: engine::hidden_items(&state).into_iter().map(named).collect(),
        stage_items: l.stage_pins.iter().map(|id| named(engine::item_ref(graph, id))).collect(),
        recent_apps: l.recent_apps,
    }
}

#[derive(Serialize, Deserialize)]
struct Backup {
    version: u32,
    layout: Layout,
    plugins: Vec<BackupPlugin>,
}

#[derive(Serialize, Deserialize)]
struct BackupPlugin {
    id: String,
    enabled: bool,
    settings: Vec<(String, String)>,
}

impl CompanionActions for CoreActions {
    fn device_info(&self) -> DeviceInfo {
        let (name, model, tier) = match self.inner() {
            Ok(i) => (i.config.device_name.clone(), i.config.device_name.clone(), format!("{:?}", i.config.tier).to_lowercase()),
            Err(_) => ("TV".into(), String::new(), String::new()),
        };
        DeviceInfo { name, model, version: env!("CARGO_PKG_VERSION").to_string(), tier }
    }

    fn snapshot_json(&self) -> String {
        let Ok(inner) = self.inner() else { return "null".to_string() };
        let kept = inner.snapshot_json.lock().clone();
        if !kept.is_empty() {
            return kept;
        }
        serde_json::to_string(&*inner.snapshot.read()).unwrap_or_else(|_| "null".to_string())
    }

    fn perform(&self, action: &str) -> Result<(), String> {
        let inner = self.inner()?;
        engine::perform(&inner, action).map_err(|e| e.to_string())
    }

    fn launch(&self, package: &str) -> Result<(), String> {
        self.inner()?.platform.launch_app(package.to_string(), String::new()).map_err(|e| e.to_string())
    }

    fn send_key(&self, key: &str) -> Result<(), String> {
        self.inner()?.platform.send_key(key.to_string()).map_err(|e| e.to_string())
    }

    fn type_text(&self, text: &str) -> Result<(), String> {
        self.inner()?.platform.type_text(text.to_string()).map_err(|e| e.to_string())
    }

    fn search(&self, query: &str) -> Result<(), String> {
        self.inner()?.platform.control(format!("search:{query}")).map_err(|e| e.to_string())
    }

    fn layout(&self) -> LayoutDoc {
        self.inner().map(|i| layout_doc(&i)).unwrap_or_default()
    }

    fn set_layout(&self, patch: LayoutPatch) -> Result<LayoutDoc, String> {
        let inner = self.inner()?;
        {
            let mut state = inner.state.lock();
            // Changed on a copy, so a layout that fails to save never takes effect.
            let mut layout = state.graph.layout.clone();
            if let Some(order) = patch.row_order {
                layout.row_order = order;
            }
            if let Some(hidden) = patch.hidden {
                layout.hidden = hidden.into_iter().collect();
            }
            if let Some(pins) = patch.pinned_apps {
                layout.pinned_apps = pins;
            }
            if let Some(pins) = patch.stage_pins {
                layout.stage_pins = pins;
            }
            if let Some(channels) = patch.enabled_channels {
                layout.enabled_channels = channels.into_iter().collect();
            }
            if let Some(visibility) = patch.row_visibility {
                layout.row_visibility = visibility;
            }
            if let Some(count) = patch.recent_apps {
                layout.recent_apps = count.min(60);
            }
            layout.normalise();
            state.store.save_layout(&layout).map_err(|e| e.to_string())?;
            let new_channels = layout.enabled_channels.difference(&state.graph.layout.enabled_channels).next().is_some();
            state.graph.layout = layout;
            if new_channels {
                worker::want_program_art(&inner, &mut state);
            }
        }
        inner.enqueue_job(worker::Job::Rebuild);
        Ok(layout_doc(&inner))
    }

    fn plugins_json(&self) -> String {
        self.inner().ok().and_then(|i| serde_json::to_string(&i.plugins_info()).ok()).unwrap_or_else(|| "[]".into())
    }

    fn install_plugin(&self, path: &Path) -> Result<String, String> {
        let info = self.inner()?.plugin_install(path, false)?;
        serde_json::to_string(&info).map_err(|e| e.to_string())
    }

    fn plugin_command(&self, id: &str, command: &str) -> Result<(), String> {
        let inner = self.inner()?;
        match command {
            "enable" => inner.plugin_set_enabled(id, true),
            "disable" => inner.plugin_set_enabled(id, false),
            "refresh" => {
                if !inner.state.lock().plugins.iter().any(|p| p.id == id) {
                    return Err("plugin is not installed".into());
                }
                inner.plugins.refresh_soon(id);
            }
            "remove" => inner.plugin_remove(id)?,
            other => return Err(format!("unknown command {other}")),
        }
        Ok(())
    }

    fn set_plugin_setting(&self, id: &str, key: &str, value: Option<String>) -> Result<(), String> {
        self.inner()?.plugin_set_setting(id, key, value);
        Ok(())
    }

    fn plugin_log(&self, id: &str) -> Vec<String> {
        self.inner().map(|i| i.plugins.logs(id)).unwrap_or_default()
    }

    fn diagnostics_json(&self) -> String {
        let Ok(inner) = self.inner() else { return "{}".into() };
        let state = inner.state.lock();
        serde_json::json!({
            "items": state.graph.len(),
            "revision": state.revision,
            "ffiCalls": inner.ffi_calls.load(std::sync::atomic::Ordering::Relaxed),
            "lastBuildMs": inner.last_build_micros.load(std::sync::atomic::Ordering::Relaxed) / 1000,
            "lastIngestMs": inner.last_ingest_ms.load(std::sync::atomic::Ordering::Relaxed),
            "databaseBytes": state.store.db_bytes(),
            "queueDepth": inner.queue_depth.load(std::sync::atomic::Ordering::Relaxed),
            "plugins": state.plugins.len(),
            "pluginsEnabled": state.plugins.iter().filter(|p| p.enabled).count(),
            "uptimeMs": now_ms() - inner.started_ms,
            "tier": format!("{:?}", inner.config.tier),
        })
        .to_string()
    }

    fn export_json(&self) -> String {
        let Ok(inner) = self.inner() else { return "{}".into() };
        let state = inner.state.lock();
        let plugins = state
            .plugins
            .iter()
            .map(|p| BackupPlugin {
                id: p.id.clone(),
                enabled: p.enabled,
                settings: state
                    .store
                    .plugin_settings(&p.id)
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|(k, _)| !p.manifest.settings.iter().any(|s| &s.key == k && s.kind.is_secret()))
                    .collect(),
            })
            .collect();
        serde_json::to_string_pretty(&Backup { version: 1, layout: state.graph.layout.clone(), plugins }).unwrap_or_else(|_| "{}".into())
    }

    fn import_json(&self, json: &str) -> Result<(), String> {
        let backup: Backup = serde_json::from_str(json).map_err(|e| format!("not a Lounge backup: {e}"))?;
        if backup.version != 1 {
            return Err(format!("backup version {} is not supported", backup.version));
        }
        let inner = self.inner()?;
        let mut layout = backup.layout.clone();
        layout.normalise();
        {
            let mut state = inner.state.lock();
            state.store.save_layout(&layout).map_err(|e| e.to_string())?;
            state.graph.layout = layout;
            // Only settings an installed plugin declares, and never a secret, which no backup
            // carries; each goes through the rule that keeps passwords with their server.
            for p in &backup.plugins {
                let Some(manifest) = state.plugins.iter().find(|e| e.id == p.id).map(|e| e.manifest.clone()) else { continue };
                for (k, v) in &p.settings {
                    if manifest.settings.iter().any(|s| &s.key == k && !s.kind.is_secret()) {
                        plugins::store_setting(&state.store, &manifest, &p.id, k, Some(v));
                    }
                }
            }
        }
        for p in &backup.plugins {
            let installed = inner.state.lock().plugins.iter().any(|e| e.id == p.id);
            if installed {
                inner.plugin_set_enabled(&p.id, p.enabled);
            }
        }
        inner.enqueue_job(worker::Job::Rebuild);
        Ok(())
    }

    fn art_roots(&self) -> Vec<PathBuf> {
        let Ok(inner) = self.inner() else { return Vec::new() };
        let cache = PathBuf::from(&inner.config.cache_dir);
        ["art", "plugin-art", "tvart", "banners", "sessions"].iter().map(|d| cache.join(d)).collect()
    }

    fn temp_dir(&self) -> PathBuf {
        self.inner().map(|i| PathBuf::from(&i.config.cache_dir).join("companion-tmp")).unwrap_or_default()
    }

    fn download(&self, url: &str, into: &Path) -> Result<(), String> {
        let dir = into.parent().ok_or("bad path")?;
        // A bundle the person linked from the phone may come from the internet or the home
        // network, but the link must not reach the TV's own services.
        let reach = Reach { whole_home: true, ..Reach::default() };
        let fetched = Fetcher::guarded(reach)
            .download(url, dir, 16 * 1024 * 1024, std::time::Duration::from_secs(20), &|_| true)
            .map_err(|e| e.to_string())?;
        std::fs::rename(&fetched.path, into).map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_link_to_install_from_never_reaches_the_tv_itself() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let port = listener.local_addr().unwrap().port();
        let dir = tempfile::tempdir().unwrap();
        let actions = CoreActions { inner: Weak::new() };
        for url in [format!("http://127.0.0.1:{port}/a.lgp"), format!("http://localhost:{port}/a.lgp")] {
            let err = actions.download(&url, &dir.path().join("a.lgp")).unwrap_err();
            assert!(err.contains("this device"), "{url}: {err}");
        }
        assert!(listener.accept().is_err(), "a request reached the TV");
    }

    struct Quiet;

    impl crate::Platform for Quiet {
        fn launch_app(&self, _: String, _: String) -> Result<(), crate::PlatformError> { Ok(()) }
        fn view_uri(&self, _: String) -> Result<(), crate::PlatformError> { Ok(()) }
        fn switch_input(&self, _: String) -> Result<(), crate::PlatformError> { Ok(()) }
        fn tune(&self, _: String) -> Result<(), crate::PlatformError> { Ok(()) }
        fn control(&self, _: String) -> Result<(), crate::PlatformError> { Ok(()) }
        fn open_app_info(&self, _: String) -> Result<(), crate::PlatformError> { Ok(()) }
        fn media_command(&self, _: String, _: String) -> Result<(), crate::PlatformError> { Ok(()) }
        fn request_permission(&self, _: String) -> Result<(), crate::PlatformError> { Ok(()) }
        fn play_stream(&self, _: String, _: String, _: Option<i64>) -> Result<(), crate::PlatformError> { Ok(()) }
        fn send_key(&self, _: String) -> Result<(), crate::PlatformError> { Ok(()) }
        fn type_text(&self, _: String) -> Result<(), crate::PlatformError> { Ok(()) }
    }

    fn test_config(dir: &std::path::Path) -> crate::CoreConfig {
        crate::CoreConfig {
            data_dir: dir.join("data").to_string_lossy().into_owned(),
            cache_dir: dir.join("cache").to_string_lossy().into_owned(),
            tier: crate::DeviceTier::B,
            utc_offset_minutes: 0,
            has_tv_controls: false,
            device_name: "TV".into(),
            screen_width: 1920,
            screen_height: 1080,
        }
    }

    /// Apps straight into the graph, with or without banners, then a rebuild.
    fn put_apps(core: &crate::Core, apps: &[(&str, bool)]) {
        let items = apps
            .iter()
            .map(|(pkg, banner)| {
                let mut it = lounge_graph::Item::app(pkg, "Main", &pkg.to_uppercase());
                it.flags.has_banner = *banner;
                it
            })
            .collect();
        core.inner.state.lock().graph.replace_source("apps", items);
        core.inner.enqueue_job(worker::Job::Rebuild);
    }

    /// The row ids on the home once `pred` holds for them, or the last seen if it never does.
    fn rows_when(core: &crate::Core, pred: impl Fn(&[String]) -> bool) -> Vec<String> {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            let ids: Vec<String> = core.snapshot().rows.iter().map(|r| r.id.clone()).collect();
            if pred(&ids) || std::time::Instant::now() > deadline {
                return ids;
            }
            std::thread::sleep(std::time::Duration::from_millis(15));
        }
    }

    #[test]
    fn a_hidden_row_leaves_the_home_and_comes_back() {
        let dir = tempfile::tempdir().unwrap();
        let core = crate::Core::open(test_config(dir.path()), Arc::new(Quiet)).unwrap();
        put_apps(&core, &[("tv.a", true), ("tv.b", false)]);
        let rows = rows_when(&core, |ids| ids.iter().any(|r| r == "apps"));
        assert!(rows.contains(&"apps".to_string()), "{rows:?}");

        core.command(crate::LayoutCommand::HideRow { row_id: "apps".into() });
        let rows = rows_when(&core, |ids| !ids.iter().any(|r| r == "apps"));
        assert!(!rows.contains(&"apps".to_string()), "{rows:?}");
        // Built all the same, so search still reaches its tiles.
        let aside: Vec<String> = core.snapshot().hidden_rows.iter().map(|r| r.id.clone()).collect();
        assert_eq!(aside, ["apps"]);
        let info = core.home_rows().into_iter().find(|r| r.id == "apps").unwrap();
        assert!(!info.shown && info.chosen);
        // Without usage access the row cannot follow what is opened, and says so.
        assert_eq!((info.count, info.detail.as_str()), (2, "Allow usage access so it follows what you open"));
        let doc = CoreActions { inner: Arc::downgrade(&core.inner) }.layout();
        assert_eq!(doc.row_visibility.get("apps"), Some(&false));
        assert_eq!(doc.row_order.first().map(String::as_str), Some("continue"));

        core.command(crate::LayoutCommand::ShowRow { row_id: "apps".into() });
        let rows = rows_when(&core, |ids| ids.iter().any(|r| r == "apps"));
        assert!(rows.contains(&"apps".to_string()), "{rows:?}");
    }

    #[test]
    fn every_app_reaches_the_all_apps_screen() {
        let dir = tempfile::tempdir().unwrap();
        let core = crate::Core::open(test_config(dir.path()), Arc::new(Quiet)).unwrap();
        put_apps(&core, &[("tv.b", false), ("tv.a", true)]);
        rows_when(&core, |ids| ids.iter().any(|r| r == "apps"));
        let snap = core.snapshot();
        // A to Z, with and without banners, and no row of them on the home.
        let all: Vec<&str> = snap.all_apps.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(all, ["app:tv.a", "app:tv.b"]);
        assert!(snap.rows.iter().all(|r| r.id != "allapps"));
        // Your apps holds the recent few, banner apps first while nothing has been opened, and
        // ends with the way to the rest.
        let apps = snap.rows.iter().find(|r| r.id == "apps").unwrap();
        let ids: Vec<&str> = apps.tiles.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(ids, ["app:tv.a", "app:tv.b", "control:allapps"]);
        let last = apps.tiles.last().unwrap();
        assert_eq!((last.title.as_str(), last.subtitle.as_deref(), last.action.as_str()), ("All apps", Some("2 apps"), "control:allapps"));

        // A hidden app leaves the screen's list too.
        core.command(crate::LayoutCommand::HideItem { id: "app:tv.b".into() });
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while core.snapshot().all_apps.len() > 1 && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(15));
        }
        assert_eq!(core.snapshot().all_apps.len(), 1);
        assert!(core.home_rows().iter().all(|r| r.id != "allapps"));
    }

    #[test]
    fn a_moved_row_gets_an_order_of_its_own() {
        let dir = tempfile::tempdir().unwrap();
        let core = crate::Core::open(test_config(dir.path()), Arc::new(Quiet)).unwrap();
        put_apps(&core, &[("tv.a", true), ("tv.b", false)]);
        rows_when(&core, |ids| ids.iter().any(|r| r == "apps"));
        core.command(crate::LayoutCommand::MoveRow { row_id: "apps".into(), to_index: 0 });
        let rows = rows_when(&core, |ids| ids.first().is_some_and(|r| r == "apps"));
        assert_eq!(rows.first().map(String::as_str), Some("apps"), "{rows:?}");
        let order = core.home_rows().into_iter().map(|r| r.id).collect::<Vec<_>>();
        assert_eq!(order[..3], ["apps", "continue", "games"]);

        // A row that cannot be built now is neither listed nor forgotten.
        core.command(crate::LayoutCommand::SetRowOrder { row_ids: vec!["plugin:gone:recent".into(), "games".into(), "continue".into()] });
        // Continue watching stands in as its notice tile while TV listings are off.
        let rows = rows_when(&core, |ids| ids.first().is_some_and(|r| r == "notice:tv_listings"));
        assert_eq!(rows.first().map(String::as_str), Some("notice:tv_listings"), "{rows:?}");
        assert!(core.home_rows().iter().all(|r| r.id != "plugin:gone:recent"));
        core.command(crate::LayoutCommand::MoveRow { row_id: "apps".into(), to_index: 0 });
        rows_when(&core, |ids| ids.first().is_some_and(|r| r == "apps"));
        let saved = core.inner.state.lock().graph.layout.row_order.clone();
        assert_eq!(saved.first().map(String::as_str), Some("apps"));
        assert!(saved.contains(&"plugin:gone:recent".to_string()), "{saved:?}");
    }

    #[test]
    fn the_phone_can_hide_a_row_and_sees_every_app() {
        let dir = tempfile::tempdir().unwrap();
        let core = crate::Core::open(test_config(dir.path()), Arc::new(Quiet)).unwrap();
        put_apps(&core, &[("tv.a", true), ("tv.b", false)]);
        rows_when(&core, |ids| ids.iter().any(|r| r == "apps"));
        let actions = CoreActions { inner: Arc::downgrade(&core.inner) };
        let mut visibility = std::collections::HashMap::new();
        visibility.insert("games".to_string(), false);
        let doc = actions.set_layout(LayoutPatch { row_visibility: Some(visibility), ..LayoutPatch::default() }).unwrap();
        assert_eq!(doc.row_visibility.get("games"), Some(&false));
        assert!(!doc.available_rows.iter().find(|r| r.id == "games").unwrap().shown);
        let mut apps: Vec<&str> = doc.apps.iter().map(|a| a.package.as_str()).collect();
        apps.sort();
        assert_eq!(apps, ["tv.a", "tv.b"]);
        assert!(doc.apps.iter().all(|a| !a.pinned));
    }

    #[test]
    fn a_layout_that_cannot_be_saved_changes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let data_dir = dir.path().join("data");
        let core = crate::Core::open(test_config(dir.path()), Arc::new(Quiet)).unwrap();
        let actions = CoreActions { inner: Arc::downgrade(&core.inner) };
        let backup = actions.export_json();

        let db = rusqlite::Connection::open(data_dir.join(crate::DB_FILE)).unwrap();
        db.busy_timeout(std::time::Duration::from_secs(5)).unwrap();
        db.execute_batch(
            "CREATE TRIGGER refuse_insert BEFORE INSERT ON layout BEGIN SELECT RAISE(FAIL, 'refused'); END;
             CREATE TRIGGER refuse_update BEFORE UPDATE ON layout BEGIN SELECT RAISE(FAIL, 'refused'); END;",
        )
        .unwrap();

        let patch = LayoutPatch { hidden: Some(vec!["apps".into()]), pinned_apps: Some(vec!["tv.example".into()]), ..LayoutPatch::default() };
        assert!(actions.set_layout(patch).is_err());
        let after = actions.layout();
        assert!(after.hidden.is_empty() && after.pinned_apps.is_empty(), "{after:?}");

        let mut restored: serde_json::Value = serde_json::from_str(&backup).unwrap();
        restored["layout"]["pinned_apps"] = serde_json::json!(["tv.example"]);
        assert!(actions.import_json(&restored.to_string()).is_err());
        assert!(actions.layout().pinned_apps.is_empty());
    }
}
