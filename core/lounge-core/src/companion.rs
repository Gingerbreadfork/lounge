//! The companion server's view of the core: snapshots, actions, layout, plugins, backup.

use crate::{engine, now_ms, plugins, worker, CompanionStatus, Inner, PairingInfo};
use lounge_companion::{CompanionActions, DeviceInfo, LayoutDoc, LayoutPatch, RowRef};
use lounge_graph::Layout;
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
    let info = c.new_pairing(&base, now_ms());
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

/// Row ids and their titles for the layout editor: the core rows, plus plugin and channel rows.
fn available_rows(inner: &Inner) -> Vec<RowRef> {
    let state = inner.state.lock();
    let mut rows: Vec<RowRef> = engine::CORE_ROWS
        .iter()
        .map(|(id, title)| RowRef { id: id.to_string(), title: if *id == "tvcontrols" && !inner.config.has_tv_controls { "Inputs".into() } else { title.to_string() } })
        .collect();
    for p in state.plugins.iter().filter(|p| p.enabled) {
        for r in &p.manifest.rows {
            rows.push(RowRef { id: format!("plugin:{}:{}", p.id, r.id), title: format!("{} · {}", p.manifest.name, r.title) });
        }
    }
    for (id, name, _) in state.graph.preview_channels() {
        rows.push(RowRef { id: format!("channel:{id}"), title: if name.is_empty() { format!("Channel {id}") } else { name } });
    }
    rows
}

fn layout_doc(inner: &Inner) -> LayoutDoc {
    let available = available_rows(inner);
    let state = inner.state.lock();
    let l = &state.graph.layout;
    LayoutDoc {
        row_order: engine::effective_row_order(&state),
        available_rows: available,
        hidden: l.hidden.iter().cloned().collect(),
        pinned_apps: l.pinned_apps.clone(),
        stage_pins: l.stage_pins.clone(),
        enabled_channels: l.enabled_channels.iter().copied().collect(),
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
            if let Some(order) = patch.row_order {
                state.graph.layout.row_order = order;
            }
            if let Some(hidden) = patch.hidden {
                state.graph.layout.hidden = hidden.into_iter().collect();
            }
            if let Some(pins) = patch.pinned_apps {
                state.graph.layout.pinned_apps = pins;
            }
            if let Some(pins) = patch.stage_pins {
                state.graph.layout.stage_pins = pins;
            }
            if let Some(channels) = patch.enabled_channels {
                let before = std::mem::replace(&mut state.graph.layout.enabled_channels, channels.into_iter().collect());
                if state.graph.layout.enabled_channels.difference(&before).next().is_some() {
                    worker::want_program_art(&inner, &mut state);
                }
            }
            state.graph.layout.normalise();
            let layout = state.graph.layout.clone();
            state.store.save_layout(&layout).map_err(|e| e.to_string())?;
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
            state.graph.layout = layout.clone();
            state.store.save_layout(&layout).map_err(|e| e.to_string())?;
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
        // A bundle the user asked for by address, from the phone: any host they named is fine.
        let fetched = lounge_plugins::http::Fetcher::new()
            .download(url, dir, 16 * 1024 * 1024, std::time::Duration::from_secs(20), &|_| true)
            .map_err(|e| e.to_string())?;
        std::fs::rename(&fetched.path, into).map_err(|e| e.to_string())
    }
}
