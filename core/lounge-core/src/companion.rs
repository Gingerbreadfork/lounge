//! The companion server's view of the core: snapshots, actions, layout, plugins, backup. Every
//! failure it hands the phone is a sentence with the cause and the fix.

use crate::{engine, now_ms, plugins, worker, CompanionStatus, CoreError, Inner, PairingInfo};
use lounge_companion::{sentence_or, AppRef, CompanionActions, DeviceInfo, LayoutDoc, LayoutPatch, NamedRef, RowRef, SourceRef};
use lounge_graph::Layout;
use lounge_plugins::http::{Fetcher, Reach};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Weak};

struct CoreActions {
    inner: Weak<Inner>,
}

impl CoreActions {
    fn inner(&self) -> Result<Arc<Inner>, String> {
        self.inner.upgrade().ok_or_else(|| "Lounge on the TV is restarting. Try again in a moment.".to_string())
    }
}

const NOT_SAVED: &str = "The TV couldn't save that. Check it has free space, then try again.";
/// How long a search from the phone waits for Lounge to come to the front.
const FRONT_WAIT: std::time::Duration = std::time::Duration::from_secs(3);
const NOT_A_BACKUP: &str = "That file isn't a Lounge backup. Choose a file that Download backup saved.";

/// What the phone reads when a tile, link or control fails on the TV.
fn failed_to(inner: &Inner, action: &str, e: CoreError) -> String {
    let (verb, target) = action.split_once(':').unwrap_or((action, ""));
    match e {
        CoreError::UnknownAction { .. } => "That isn't on the TV's home screen any more. Choose it again from what the TV shows now.".into(),
        CoreError::Platform { detail } => match verb {
            "open" => {
                let title = inner.state.lock().graph.get(target).map(|it| it.title.clone());
                let fallback = match title {
                    Some(title) => format!("Couldn't open {title}. Is its app still installed?"),
                    None => "Couldn't open that. Is its app still installed?".to_string(),
                };
                sentence_or(&detail, &fallback)
            }
            "url" => sentence_or(&detail, "Nothing on the TV can open that link. Install an app for it, then try again."),
            "media" => sentence_or(&detail, "The TV couldn't control what is playing. Try it from the TV."),
            _ => sentence_or(&detail, "The TV couldn't do that. Try it from the TV."),
        },
        // A plugin's error is its own sentence, which may name a server by its address.
        CoreError::Plugin { detail } if plugin_sentence(&detail) => detail,
        other => sentence_or(&other.to_string(), "The TV couldn't do that. Try it from the TV."),
    }
}

fn plugin_sentence(detail: &str) -> bool {
    let d = detail.trim();
    d.chars().next().is_some_and(char::is_uppercase) && d.ends_with(['.', '?']) && d.len() <= 400
}

/// A plugin that would not install, in words.
fn not_installed(e: String) -> String {
    if e.contains("signature") {
        tracing::warn!("companion: plugin install refused ({e})");
        return "That plugin's signature doesn't check out, so the TV won't install it. Get it again from its publisher.".into();
    }
    sentence_or(&e, "That file isn't a Lounge plugin, or it was damaged on the way. Get it again, then install it.")
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
            pairing: c.live_pairing(now_ms()).and_then(|info| match pictured(inner, info) {
                Ok(p) => Some(p),
                Err(e) => {
                    tracing::warn!("companion: the pairing QR code could not be drawn ({e})");
                    None
                }
            }),
        },
        None => CompanionStatus { running: false, url: None, port: 0, paired: 0, pairing: None },
    }
}

pub(crate) fn pairing(inner: &Arc<Inner>) -> Result<PairingInfo, String> {
    let slot = inner.companion.lock();
    let c = slot.as_ref().ok_or_else(|| "The phone companion is off. Turn it on, then try again.".to_string())?;
    let ip = lounge_companion::local_ip().ok_or_else(|| "The TV isn't on a network. Connect it to your Wi-Fi, then try again.".to_string())?;
    let base = format!("http://{ip}:{}", c.port);
    let info = c.new_pairing(&base, now_ms())?;
    pictured(inner, info)
}

/// A pairing with its QR picture on disk, drawn once per code. One live code at a time: earlier
/// codes' pictures go, so nothing can read a link off them.
fn pictured(inner: &Inner, info: lounge_companion::PairingInfo) -> Result<PairingInfo, String> {
    let dir = PathBuf::from(&inner.config.cache_dir).join("companion");
    let png = dir.join(format!("pair-{}-{}.png", info.code, info.expires_ms));
    if !png.exists() {
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for e in entries.flatten() {
                if e.file_name().to_string_lossy().starts_with("pair-") {
                    let _ = std::fs::remove_file(e.path());
                }
            }
        }
        lounge_companion::qr_png(&info.url, &png, 12).map_err(|e| e.to_string())?;
    }
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
        muted_providers: l.muted_providers.iter().cloned().collect(),
        continue_sources: engine::continue_sources(&state).into_iter().map(|s| SourceRef { id: s.id, title: s.title, icon_path: s.icon_path, count: s.count, shown: s.shown }).collect(),
        dismissed_notices: l.dismissed_notices.iter().cloned().collect(),
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
        engine::perform(&inner, action).map_err(|e| failed_to(&inner, action, e))
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
        let inner = self.inner()?;
        // Search opens inside Lounge, so with another app in front Lounge comes forward first. The
        // search waits until Lounge is in front, so its arrival home cannot close the search again.
        let mut in_front = inner.screen_on.load(Ordering::Relaxed);
        if !in_front {
            if let Err(e) = inner.platform.send_key("home".to_string()) {
                tracing::warn!("companion: could not bring Lounge forward for a search ({e})");
            }
            let deadline = std::time::Instant::now() + FRONT_WAIT;
            while !in_front && std::time::Instant::now() < deadline {
                std::thread::sleep(std::time::Duration::from_millis(50));
                in_front = inner.screen_on.load(Ordering::Relaxed);
            }
        }
        inner.platform.control(format!("search:{query}")).map_err(|e| e.to_string())?;
        if in_front {
            Ok(())
        } else {
            Err("The search is open in Lounge, but another app is still in front on the TV. Press Home to see it.".into())
        }
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
            if let Some(muted) = patch.muted_providers {
                layout.muted_providers = muted.into_iter().collect();
            }
            if let Some(dismissed) = patch.dismissed_notices {
                layout.dismissed_notices = dismissed.into_iter().collect();
            }
            layout.normalise();
            state.store.save_layout(&layout).map_err(|e| {
                tracing::warn!("companion: layout not saved ({e})");
                NOT_SAVED.to_string()
            })?;
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
        let info = self.inner()?.plugin_install(path, false).map_err(not_installed)?;
        serde_json::to_string(&info).map_err(|e| e.to_string())
    }

    fn plugin_command(&self, id: &str, command: &str) -> Result<(), String> {
        let inner = self.inner()?;
        match command {
            "enable" => inner.plugin_switch(id, true),
            "disable" => inner.plugin_switch(id, false),
            "refresh" => {
                if !inner.state.lock().plugins.iter().any(|p| p.id == id) {
                    return Err("That plugin isn't installed any more.".into());
                }
                inner.plugins.refresh_now(id);
            }
            "remove" => inner.plugin_remove(id).map_err(|e| sentence_or(&e, "The TV couldn't remove that plugin. Try again."))?,
            other => {
                tracing::warn!("companion: unknown plugin command {other}");
                return Err("Lounge on the TV doesn't know that command. Update Lounge, then try again.".into());
            }
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
        let backup: Backup = serde_json::from_str(json).map_err(|e| {
            tracing::warn!("companion: not a backup ({e})");
            NOT_A_BACKUP.to_string()
        })?;
        if backup.version != 1 {
            return Err("That backup comes from a newer Lounge. Update Lounge on the TV, then restore it.".into());
        }
        let inner = self.inner()?;
        let mut layout = backup.layout.clone();
        layout.normalise();
        {
            let mut state = inner.state.lock();
            state.store.save_layout(&layout).map_err(|e| {
                tracing::warn!("companion: restored layout not saved ({e})");
                NOT_SAVED.to_string()
            })?;
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
        let fetched = Fetcher::guarded(reach).download(url, dir, 16 * 1024 * 1024, std::time::Duration::from_secs(20), &|_| true).map_err(|e| {
            let e = e.to_string();
            tracing::warn!("companion: plugin download failed ({e})");
            if e.contains("this device") {
                "That link leads back to the TV itself. Use a link to the plugin on the internet or another device.".to_string()
            } else {
                "The TV couldn't download that link. Check the address, then try again.".to_string()
            }
        })?;
        std::fs::rename(&fetched.path, into).map_err(|e| {
            tracing::warn!("companion: downloaded plugin not kept ({e})");
            NOT_SAVED.to_string()
        })
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
            assert!(err.contains("the TV itself"), "{url}: {err}");
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

    /// Remembers the keys and controls it is sent, in order.
    #[derive(Default)]
    struct Recorder(parking_lot::Mutex<Vec<String>>);

    impl crate::Platform for Recorder {
        fn launch_app(&self, _: String, _: String) -> Result<(), crate::PlatformError> { Ok(()) }
        fn view_uri(&self, _: String) -> Result<(), crate::PlatformError> { Ok(()) }
        fn switch_input(&self, _: String) -> Result<(), crate::PlatformError> { Ok(()) }
        fn tune(&self, _: String) -> Result<(), crate::PlatformError> { Ok(()) }
        fn control(&self, command: String) -> Result<(), crate::PlatformError> {
            self.0.lock().push(format!("control {command}"));
            Ok(())
        }
        fn open_app_info(&self, _: String) -> Result<(), crate::PlatformError> { Ok(()) }
        fn media_command(&self, _: String, _: String) -> Result<(), crate::PlatformError> { Ok(()) }
        fn request_permission(&self, _: String) -> Result<(), crate::PlatformError> { Ok(()) }
        fn play_stream(&self, _: String, _: String, _: Option<i64>) -> Result<(), crate::PlatformError> { Ok(()) }
        fn send_key(&self, key: String) -> Result<(), crate::PlatformError> {
            self.0.lock().push(format!("key {key}"));
            Ok(())
        }
        fn type_text(&self, _: String) -> Result<(), crate::PlatformError> { Ok(()) }
    }

    #[test]
    fn a_search_from_the_phone_brings_lounge_forward_first() {
        let dir = tempfile::tempdir().unwrap();
        let platform = Arc::new(Recorder::default());
        let core = crate::Core::open(test_config(dir.path()), platform.clone()).unwrap();
        let actions = CoreActions { inner: Arc::downgrade(&core.inner) };

        actions.search("bear").unwrap();
        assert_eq!(*platform.0.lock(), ["control search:bear"]);

        // Behind another app: Home first, and the search once Lounge is in front.
        platform.0.lock().clear();
        core.set_screen_state(false);
        std::thread::scope(|s| {
            s.spawn(|| {
                std::thread::sleep(std::time::Duration::from_millis(200));
                assert_eq!(*platform.0.lock(), ["key home"]);
                core.set_screen_state(true);
            });
            actions.search("bear").unwrap();
        });
        assert_eq!(*platform.0.lock(), ["key home", "control search:bear"]);

        // When Lounge does not come forward, the phone is told where the search is.
        platform.0.lock().clear();
        core.set_screen_state(false);
        let err = actions.search("bear").unwrap_err();
        assert!(err.starts_with("The search is open in Lounge"), "{err}");
        assert_eq!(*platform.0.lock(), ["key home", "control search:bear"]);
    }

    #[test]
    fn the_tv_reads_the_code_that_pairs_now_with_its_picture() {
        let dir = tempfile::tempdir().unwrap();
        let core = crate::Core::open(test_config(dir.path()), Arc::new(Quiet)).unwrap();
        start(&core.inner, 0).unwrap();
        assert_eq!(status(&core.inner).pairing, None);

        let made = |core: &crate::Core| core.inner.companion.lock().as_ref().unwrap().new_pairing("http://tv:8787", now_ms()).unwrap();
        let first = made(&core);
        let shown = status(&core.inner).pairing.unwrap();
        assert_eq!((shown.code.as_str(), shown.url.as_str()), (first.code.as_str(), first.url.as_str()));
        assert!(Path::new(&shown.qr_png_path).exists());
        // Read again, the same picture is kept rather than drawn anew.
        assert_eq!(status(&core.inner).pairing.unwrap().qr_png_path, shown.qr_png_path);

        // A new code's picture replaces the old one.
        made(&core);
        let next = status(&core.inner).pairing.unwrap();
        assert_ne!(next.qr_png_path, shown.qr_png_path);
        assert!(Path::new(&next.qr_png_path).exists() && !Path::new(&shown.qr_png_path).exists());
    }

    #[test]
    fn a_failed_tile_is_explained_in_words() {
        let dir = tempfile::tempdir().unwrap();
        let core = crate::Core::open(test_config(dir.path()), Arc::new(Quiet)).unwrap();
        let platform_said = |detail: &str| CoreError::Platform { detail: detail.into() };
        assert_eq!(failed_to(&core.inner, "open:app:com.netflix.ninja", platform_said("No launch intent for com.netflix.ninja")), "Couldn't open that. Is its app still installed?");
        assert_eq!(failed_to(&core.inner, "media:tv.a:next", platform_said("That app is not playing anything")), "That app is not playing anything.");
        assert_eq!(
            failed_to(&core.inner, "open:wn:1", CoreError::UnknownAction { action: "open:wn:1".into() }),
            "That isn't on the TV's home screen any more. Choose it again from what the TV shows now."
        );
        let plugin_said = |detail: &str| CoreError::Plugin { detail: detail.into() };
        let jellyfin = "Jellyfin didn't answer at 192.168.0.20:8096. Check the server, then try again.";
        assert_eq!(failed_to(&core.inner, "open:plugin:jf:1", plugin_said(jellyfin)), jellyfin);
        assert_eq!(failed_to(&core.inner, "open:plugin:jf:1", plugin_said("the plugin is turned off or not installed")), "The TV couldn't do that. Try it from the TV.");
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
        assert_eq!((info.count, info.detail.as_str()), (2, "Allow usage access in Settings to follow what you open"));
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

        // A hidden app leaves Your apps but stays on the screen's list, marked hidden.
        core.command(crate::LayoutCommand::HideItem { id: "app:tv.b".into() });
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !core.snapshot().all_apps.iter().any(|t| t.hidden) && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(15));
        }
        let snap = core.snapshot();
        assert_eq!(snap.all_apps.iter().map(|t| (t.id.as_str(), t.hidden)).collect::<Vec<_>>(), [("app:tv.a", false), ("app:tv.b", true)]);
        assert!(snap.rows.iter().find(|r| r.id == "apps").unwrap().tiles.iter().all(|t| t.id != "app:tv.b"));
        assert!(core.home_rows().iter().all(|r| r.id != "allapps"));
    }

    #[test]
    fn a_muted_app_leaves_continue_watching_until_it_is_shown_again() {
        let dir = tempfile::tempdir().unwrap();
        let core = crate::Core::open(test_config(dir.path()), Arc::new(Quiet)).unwrap();
        put_apps(&core, &[("tv.canal", true)]);
        let watching = |id: &str, package: &str, title: &str, used: i64| {
            let mut it = lounge_graph::Item::content(id, lounge_graph::ItemKind::Resume, "tvprovider", title, lounge_graph::DeepLink::ViewUri(id.into()));
            it.owner = Some(package.into());
            it.last_used_ms = Some(used);
            it
        };
        core.inner.state.lock().graph.replace_source("tvprovider", vec![watching("wn:1", "tv.canal", "News", 20), watching("wn:2", "tv.plex", "Severance", 10)]);
        core.inner.enqueue_job(worker::Job::Rebuild);
        // What there is to continue, on the Stage strip and then in the row after it, once there
        // are `count` of them, or as last seen.
        let tiles_when = |count: usize| {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            loop {
                let snap = core.snapshot();
                let row = snap.rows.iter().find(|r| r.id == "continue").map(|r| r.tiles.clone()).unwrap_or_default();
                let tiles: Vec<crate::Tile> = snap.stage.strip.iter().filter(|t| t.provider.is_some()).cloned().chain(row).collect();
                if tiles.len() == count || std::time::Instant::now() > deadline {
                    return tiles;
                }
                std::thread::sleep(std::time::Duration::from_millis(15));
            }
        };
        // Each tile names where it comes from: the app by its label, or by its package when it is gone.
        let tiles = tiles_when(2);
        let from: Vec<(Option<&str>, Option<&str>)> = tiles.iter().map(|t| (t.provider.as_deref(), t.provider_name.as_deref())).collect();
        assert_eq!(from, [(Some("tv.canal"), Some("TV.CANAL")), (Some("tv.plex"), Some("plex"))]);
        assert!(core.snapshot().stage.strip.iter().any(|t| t.provider.as_deref() == Some("tv.canal")));

        core.command(crate::LayoutCommand::MuteProvider { provider: "tv.canal".into() });
        assert_eq!(tiles_when(1).iter().map(|t| t.id.as_str()).collect::<Vec<_>>(), ["wn:2"]);
        assert!(core.snapshot().stage.strip.iter().all(|t| t.provider.as_deref() != Some("tv.canal")));
        // Both stay listed, with what each brings and which is shown.
        let listed = |core: &crate::Core| core.continue_sources().into_iter().map(|s| (s.title, s.count, s.shown)).collect::<Vec<_>>();
        assert_eq!(listed(&core), [("plex".to_string(), 1, true), ("TV.CANAL".to_string(), 1, false)]);

        // The phone reads the same list and can empty it.
        let actions = CoreActions { inner: Arc::downgrade(&core.inner) };
        let doc = actions.layout();
        assert_eq!(doc.muted_providers, ["tv.canal"]);
        assert_eq!(doc.continue_sources.iter().map(|s| (s.title.as_str(), s.shown)).collect::<Vec<_>>(), [("plex", true), ("TV.CANAL", false)]);
        // By the names the phone's page reads them under.
        let sent = serde_json::to_string(&doc).unwrap();
        assert!(sent.contains(r#""mutedProviders":["tv.canal"]"#) && sent.contains(r#""continueSources":[{"id":"tv.plex","title":"plex","iconPath":null,"count":1,"shown":true}"#), "{sent}");
        let tile = serde_json::to_string(&tiles_when(1)[0]).unwrap();
        assert!(tile.contains(r#""provider":"tv.plex","providerName":"plex""#), "{tile}");
        let doc = actions.set_layout(LayoutPatch { muted_providers: Some(Vec::new()), ..LayoutPatch::default() }).unwrap();
        assert!(doc.muted_providers.is_empty());
        assert_eq!(tiles_when(2).len(), 2);
        assert_eq!(listed(&core), [("plex".to_string(), 1, true), ("TV.CANAL".to_string(), 1, true)]);

        // An app left out stays listed while it brings nothing, so it can be shown again.
        core.command(crate::LayoutCommand::MuteProvider { provider: "tv.gone".into() });
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while core.continue_sources().len() < 3 && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(15));
        }
        assert!(core.continue_sources().contains(&crate::SourceInfo { id: "tv.gone".into(), title: "gone".into(), icon_path: None, count: 0, shown: false }));
    }

    #[test]
    fn a_plugin_waiting_on_the_person_shows_its_ask_not_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let core = crate::Core::open(test_config(dir.path()), Arc::new(Quiet)).unwrap();
        let manifest = lounge_plugin_api::Manifest { id: "tv.example.plex".into(), name: "Plex".into(), version: "1".into(), ..Default::default() };
        let said = "Sign in: on your phone open plex.tv/link and enter 25Q4, then press Refresh now here.";
        // Past the refreshes queued as Lounge opens, which would try this plugin again.
        engine::wait_for_graph(&core.inner);
        {
            let mut state = core.inner.state.lock();
            let row = lounge_store::PluginRow {
                id: manifest.id.clone(),
                manifest_json: serde_json::to_string(&manifest).unwrap(),
                publisher: String::new(),
                enabled: true,
                built_in: true,
                installed_ms: 0,
                last_refresh_ms: Some(1),
                last_error: Some(said.into()),
                last_success_ms: None,
            };
            state.store.plugin_upsert(&row).unwrap();
            // Refreshed just now, so the refresh thread leaves it alone for the length of the test.
            state.store.plugin_set_refresh(&row.id, now_ms(), Some(said)).unwrap();
            let rows = state.store.plugins().unwrap();
            state.plugins = plugins::entries_from_rows(&rows);
        }
        // Until a plugin is refreshed again only the sentence is left.
        let info = core.plugins().remove(0);
        assert_eq!((info.last_error.as_deref(), info.ask), (Some(said), None));

        let ask = lounge_plugin_api::Ask { say: said.into(), link: Some("plex.tv/link".into()), code: Some("25Q4".into()), after: None, watch: true };
        core.inner.plugins.asks.lock().insert(manifest.id.clone(), plugins::PendingAsk { ask, since_ms: 0, watch: None });
        let info = core.plugins().remove(0);
        assert_eq!(info.last_error, None);
        let shown = info.ask.unwrap();
        assert_eq!((shown.link.as_deref(), shown.code.as_deref()), (Some("https://plex.tv/link"), Some("25Q4")));
        assert!(shown.qr_png_path.is_some_and(|p| std::path::Path::new(&p).exists()));
        // The phone is told the same, under the names its page reads.
        let actions = CoreActions { inner: Arc::downgrade(&core.inner) };
        let sent = actions.plugins_json();
        assert!(sent.contains(r#""ask":{"say":"Sign in: on your phone open plex.tv/link"#) && sent.contains(r#""linkText":"plex.tv/link","code":"25Q4""#), "{sent}");

        // Turned off, it waits on no one.
        core.set_plugin_enabled(manifest.id.clone(), false);
        assert_eq!(core.plugins().remove(0).ask, None);
    }

    #[test]
    fn a_plugin_is_suggested_while_its_app_is_on_the_tv_and_it_is_off() {
        let dir = tempfile::tempdir().unwrap();
        let core = crate::Core::open(test_config(dir.path()), Arc::new(Quiet)).unwrap();
        let manifest = lounge_plugin_api::Manifest { id: "tv.example.plex".into(), name: "Plex".into(), version: "1".into(), for_apps: vec!["tv.plex".into()], ..Default::default() };
        {
            let mut state = core.inner.state.lock();
            let row = lounge_store::PluginRow {
                id: manifest.id.clone(),
                manifest_json: serde_json::to_string(&manifest).unwrap(),
                publisher: String::new(),
                enabled: false,
                built_in: true,
                installed_ms: 0,
                last_refresh_ms: None,
                last_error: None,
                last_success_ms: None,
            };
            state.store.plugin_upsert(&row).unwrap();
            let rows = state.store.plugins().unwrap();
            state.plugins = plugins::entries_from_rows(&rows);
        }
        let suggested = |ids: &[String]| ids.iter().any(|r| r == "notice:plugins");
        // Nothing is suggested while the app is not on the TV.
        put_apps(&core, &[("tv.other", true)]);
        assert!(!suggested(&rows_when(&core, |ids| ids.iter().any(|r| r == "apps"))));

        put_apps(&core, &[("tv.other", true), ("tv.plex", true)]);
        assert!(suggested(&rows_when(&core, suggested)));
        let tile = core.snapshot().rows.iter().find(|r| r.id == "notice:plugins").unwrap().tiles[0].clone();
        assert_eq!((tile.id.as_str(), tile.title.as_str(), tile.action.as_str()), ("notice:plugin:tv.example.plex", "Set up Plex", "control:plugin:tv.example.plex"));
        assert_eq!(tile.subtitle.as_deref(), Some("See what you were watching"));

        // Dismissed, it stays away; so does one the person turns off themselves.
        core.command(crate::LayoutCommand::DismissNotice { id: tile.id.clone() });
        assert!(!suggested(&rows_when(&core, |ids| !suggested(ids))));
        core.command(crate::LayoutCommand::ResetLayout);
        assert!(suggested(&rows_when(&core, suggested)));
        core.set_plugin_enabled("tv.example.plex".into(), false);
        assert!(!suggested(&rows_when(&core, |ids| !suggested(ids))));
        assert!(core.inner.state.lock().graph.layout.dismissed_notices.contains("notice:plugin:tv.example.plex"));

        // The phone can dismiss it as well.
        core.command(crate::LayoutCommand::ResetLayout);
        assert!(suggested(&rows_when(&core, suggested)));
        let actions = CoreActions { inner: Arc::downgrade(&core.inner) };
        let doc = actions.set_layout(LayoutPatch { dismissed_notices: Some(vec![tile.id.clone()]), ..LayoutPatch::default() }).unwrap();
        assert_eq!(doc.dismissed_notices, [tile.id.as_str()]);
        assert!(!suggested(&rows_when(&core, |ids| !suggested(ids))));

        // Turned on, it needs no suggesting.
        core.command(crate::LayoutCommand::ResetLayout);
        assert!(suggested(&rows_when(&core, suggested)));
        core.inner.state.lock().plugins[0].enabled = true;
        core.inner.enqueue_job(worker::Job::Rebuild);
        assert!(!suggested(&rows_when(&core, |ids| !suggested(ids))));
    }

    #[test]
    fn the_way_to_all_apps_survives_an_empty_or_hidden_your_apps() {
        let dir = tempfile::tempdir().unwrap();
        let core = crate::Core::open(test_config(dir.path()), Arc::new(Quiet)).unwrap();
        put_apps(&core, &[("tv.a", true), ("tv.b", false)]);
        rows_when(&core, |ids| ids.iter().any(|r| r == "apps"));
        // Pins only, with nothing pinned: the row stands with just the All apps tile.
        core.command(crate::LayoutCommand::SetRecentApps { count: 0 });
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            let snap = core.snapshot();
            let apps = snap.rows.iter().find(|r| r.id == "apps");
            if apps.is_some_and(|r| r.tiles.len() == 1) || std::time::Instant::now() > deadline {
                let row = apps.expect("Your apps row");
                assert_eq!(row.tiles.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(), ["control:allapps"]);
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(15));
        }
        // Hidden altogether: the Stage strip ends with the tile instead.
        core.command(crate::LayoutCommand::HideRow { row_id: "apps".into() });
        rows_when(&core, |ids| !ids.iter().any(|r| r == "apps"));
        let snap = core.snapshot();
        assert_eq!(snap.stage.strip.last().map(|t| t.id.as_str()), Some("control:allapps"));
        assert_eq!(snap.stage.spotlights.last().map(|s| s.title.as_str()), Some("All apps"));
    }

    #[test]
    fn a_playing_app_does_not_republish_the_home_every_tick() {
        let dir = tempfile::tempdir().unwrap();
        let core = crate::Core::open(test_config(dir.path()), Arc::new(Quiet)).unwrap();
        put_apps(&core, &[("tv.a", true), ("tv.b", true)]);
        rows_when(&core, |ids| ids.iter().any(|r| r == "apps"));
        core.report_usage(vec![crate::AppUsage { package: "tv.a".into(), last_used_ms: 1_000 }]);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while core.snapshot().all_apps.iter().find(|t| t.id == "app:tv.a").and_then(|t| t.opened_rank) != Some(0) && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(15));
        }
        let before = core.snapshot();
        assert_eq!(before.all_apps.iter().find(|t| t.id == "app:tv.a").and_then(|t| t.opened_rank), Some(0));
        // The same app, used again a moment later: the order is unchanged, so the tiles are too.
        core.report_usage(vec![crate::AppUsage { package: "tv.a".into(), last_used_ms: 2_000 }]);
        std::thread::sleep(std::time::Duration::from_millis(600));
        let after = core.snapshot();
        assert_eq!(after.all_apps, before.all_apps);
        assert_eq!(after.rows, before.rows);
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
