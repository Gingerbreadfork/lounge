//! Plugins inside the core: bundles live under `<data_dir>/plugins/<id>/`, their key-values and
//! settings in the store, and a dedicated thread runs refreshes on a schedule or on demand and
//! hands the items to the worker.

use crate::{now_ms, screensavers, worker, Inner, PluginInfo, PluginSettingInfo};
use lounge_plugin_api::{valid_plugin_id, Action, HttpRequest, HttpResponse, Item as ApiItem, ItemKind as ApiItemKind, Manifest, RefreshContext};
use lounge_plugins::bundle::Bundle;
use lounge_plugins::host::{Host, Limits, LoadedPlugin, Services};
use lounge_plugins::http::Fetcher;
use lounge_store::PluginRow;
use parking_lot::Mutex;
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock, Weak};
use std::time::Duration;

const LOG_LINES: usize = 200;
const MAX_ITEMS: usize = 2000;
const MAX_ARTWORK_BYTES: usize = 4 * 1024 * 1024;

pub(crate) enum PluginJob {
    Refresh(String),
    Resolve {
        plugin: String,
        action: String,
        reply: crossbeam_channel::Sender<Result<Action, String>>,
    },
}

/// A plugin and its state as the engine sees it.
#[derive(Clone, Debug)]
pub(crate) struct PluginEntry {
    pub id: String,
    pub manifest: Manifest,
    pub enabled: bool,
}

/// An item with its artwork already on disk, ready for the worker.
pub(crate) struct ReadyItem {
    pub item: ApiItem,
    pub poster: Option<String>,
    pub thumbnail: Option<String>,
    pub logo: Option<String>,
}

pub(crate) struct PluginManager {
    host: Host,
    dir: PathBuf,
    art_dir: PathBuf,
    loaded: Mutex<HashMap<String, Arc<LoadedPlugin>>>,
    logs: Arc<Mutex<HashMap<String, VecDeque<String>>>>,
    pub tx: crossbeam_channel::Sender<PluginJob>,
    fetcher: Fetcher,
}

/// The host's view of storage, backed by the core's store once it exists.
pub(crate) struct CoreServices {
    inner: OnceLock<Weak<Inner>>,
    fetcher: Fetcher,
    logs: Arc<Mutex<HashMap<String, VecDeque<String>>>>,
}

impl CoreServices {
    fn with_store<T>(&self, f: impl FnOnce(&mut lounge_store::Store) -> T) -> Option<T> {
        let inner = self.inner.get()?.upgrade()?;
        let mut state = inner.state.lock();
        Some(f(&mut state.store))
    }
}

impl Services for CoreServices {
    fn kv_get(&self, plugin: &str, key: &str) -> Option<Vec<u8>> {
        self.with_store(|s| s.plugin_kv_get(plugin, key).ok().flatten()).flatten()
    }
    fn kv_set(&self, plugin: &str, key: &str, value: &[u8]) {
        self.with_store(|s| s.plugin_kv_set(plugin, key, value, Limits::default().kv_keys).ok());
    }
    fn kv_delete(&self, plugin: &str, key: &str) {
        self.with_store(|s| s.plugin_kv_delete(plugin, key).ok());
    }
    fn secret(&self, plugin: &str, key: &str) -> Option<String> {
        self.setting(plugin, key)
    }
    fn setting(&self, plugin: &str, key: &str) -> Option<String> {
        self.with_store(|s| s.plugin_settings(plugin).ok())
            .flatten()
            .and_then(|all| all.into_iter().find(|(k, _)| k == key).map(|(_, v)| v))
    }
    fn log(&self, plugin: &str, level: i32, message: &str) {
        push_log(&self.logs, plugin, level, message);
        match level {
            3 => tracing::warn!("plugin {plugin}: {message}"),
            _ => tracing::info!("plugin {plugin}: {message}"),
        }
    }
    fn fetch(&self, plugin: &str, request: &HttpRequest) -> Result<HttpResponse, String> {
        let manifest = self
            .inner
            .get()
            .and_then(|w| w.upgrade())
            .and_then(|inner| inner.state.lock().plugins.iter().find(|p| p.id == plugin).map(|p| p.manifest.clone()))
            .ok_or_else(|| "plugin is not installed".to_string())?;
        self.fetcher.fetch(request, &|host| host_permitted(&manifest, host)).map_err(|e| e.to_string())
    }
}

/// The manifest's allow list, applied to every hop. Loopback and the TV's own addresses are
/// never implied by a wildcard: a plugin reaches the companion server only by naming it.
pub(crate) fn host_permitted(manifest: &Manifest, host: &str) -> bool {
    if !manifest.allows_host(host) {
        return false;
    }
    let h = host.to_ascii_lowercase();
    let local = h == "localhost" || h == "0.0.0.0" || h == "::1" || h == "::" || h.starts_with("127.") || h.ends_with(".localhost");
    !local || manifest.fetch_hosts.iter().any(|allowed| allowed.eq_ignore_ascii_case(&h))
}

fn push_log(logs: &Mutex<HashMap<String, VecDeque<String>>>, plugin: &str, level: i32, message: &str) {
    let tag = ["debug", "info", "warn", "error"].get(level as usize).copied().unwrap_or("log");
    let mut logs = logs.lock();
    let q = logs.entry(plugin.to_string()).or_default();
    if q.len() >= LOG_LINES {
        q.pop_front();
    }
    q.push_back(format!("{} {tag}: {message}", now_ms()));
}

impl PluginManager {
    pub fn new(data_dir: &Path, cache_dir: &Path) -> (Arc<PluginManager>, crossbeam_channel::Receiver<PluginJob>, Arc<CoreServices>) {
        let logs = Arc::new(Mutex::new(HashMap::new()));
        let services = Arc::new(CoreServices { inner: OnceLock::new(), fetcher: Fetcher::new(), logs: Arc::clone(&logs) });
        let (tx, rx) = crossbeam_channel::unbounded();
        let manager = Arc::new(PluginManager {
            host: Host::new(services.clone(), Limits::default()),
            dir: data_dir.join("plugins"),
            art_dir: cache_dir.join("plugin-art"),
            loaded: Mutex::new(HashMap::new()),
            logs,
            tx,
            fetcher: Fetcher::new(),
        });
        (manager, rx, services)
    }

    pub fn logs(&self, plugin: &str) -> Vec<String> {
        self.logs.lock().get(plugin).map(|q| q.iter().cloned().collect()).unwrap_or_default()
    }

    /// `<plugins>/<id>`; an id that is not a single plain path component never becomes a path.
    fn plugin_dir(&self, id: &str) -> Option<PathBuf> {
        if !valid_plugin_id(id) {
            return None;
        }
        let mut parts = Path::new(id).components();
        match (parts.next(), parts.next()) {
            (Some(std::path::Component::Normal(_)), None) => Some(self.dir.join(id)),
            _ => None,
        }
    }

    /// Verify, pin the publisher and copy a bundle into place. Returns the stored row.
    pub fn install(&self, path: &Path, built_in: bool, existing: Option<&PluginRow>) -> Result<(PluginRow, Manifest), String> {
        let bundle = Bundle::read(path).map_err(|e| e.to_string())?;
        bundle.verify().map_err(|e| e.to_string())?;
        if let Some(prev) = existing {
            if prev.publisher != bundle.publisher_hex() {
                return Err(format!("{} is signed by a different publisher than the installed version", bundle.manifest.name));
            }
        }
        // The module must describe itself the same way the bundle does.
        let probe = self.host.load(&bundle.manifest.id, bundle.manifest.clone(), &bundle.wasm).map_err(|e| e.to_string())?;
        let reported = self.host.manifest_of(&probe).map_err(|e| e.to_string())?;
        if reported.id != bundle.manifest.id || reported.version != bundle.manifest.version {
            return Err("the plugin module does not match its manifest".into());
        }
        let dir = self.plugin_dir(&bundle.manifest.id).ok_or_else(|| format!("invalid plugin id {:?}", bundle.manifest.id))?;
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        std::fs::write(dir.join("plugin.wasm"), &bundle.wasm).map_err(|e| e.to_string())?;
        std::fs::write(dir.join("manifest.json"), &bundle.manifest_json).map_err(|e| e.to_string())?;
        self.loaded.lock().insert(bundle.manifest.id.clone(), Arc::new(probe));
        let row = PluginRow {
            id: bundle.manifest.id.clone(),
            manifest_json: String::from_utf8_lossy(&bundle.manifest_json).into_owned(),
            publisher: bundle.publisher_hex(),
            enabled: existing.map(|p| p.enabled).unwrap_or(false),
            built_in,
            installed_ms: now_ms(),
            last_refresh_ms: None,
            last_error: None,
        };
        Ok((row, bundle.manifest))
    }

    pub fn remove_files(&self, id: &str) {
        self.loaded.lock().remove(id);
        if let Some(dir) = self.plugin_dir(id) {
            if dir.parent() == Some(self.dir.as_path()) {
                let _ = std::fs::remove_dir_all(dir);
            }
        }
    }

    fn loaded(&self, id: &str, manifest: &Manifest) -> Result<Arc<LoadedPlugin>, String> {
        if let Some(p) = self.loaded.lock().get(id) {
            return Ok(Arc::clone(p));
        }
        let dir = self.plugin_dir(id).ok_or_else(|| format!("invalid plugin id {id:?}"))?;
        let wasm = std::fs::read(dir.join("plugin.wasm")).map_err(|e| format!("plugin files missing: {e}"))?;
        let plugin = Arc::new(self.host.load(id, manifest.clone(), &wasm).map_err(|e| e.to_string())?);
        self.loaded.lock().insert(id.to_string(), Arc::clone(&plugin));
        Ok(plugin)
    }

    /// Run a refresh now; downloads artwork; returns items ready for the worker.
    fn refresh(&self, entry: &PluginEntry, ctx: &RefreshContext) -> Result<Vec<ReadyItem>, String> {
        let plugin = self.loaded(&entry.id, &entry.manifest)?;
        let mut items = self.host.refresh(&plugin, ctx).map_err(|e| e.to_string())?;
        items.truncate(MAX_ITEMS);
        let dir = self.art_dir.join(&entry.id);
        let fetcher = &self.fetcher;
        // Artwork is fetched by the host, four at a time; a failed image is just a missing image.
        let mut ready: Vec<ReadyItem> = items.into_iter().map(|item| ReadyItem { item, poster: None, thumbnail: None, logo: None }).collect();
        let chunk_len = ready_chunk(ready.len());
        let dir = &dir;
        std::thread::scope(|scope| {
            for chunk in ready.chunks_mut(chunk_len) {
                let manifest = &entry.manifest;
                scope.spawn(move || {
                    let local = |url: Option<&str>| {
                        let u = url?;
                        let host = lounge_plugins::http::host_of(u)?;
                        if !host_permitted(manifest, &host) {
                            return None;
                        }
                        fetcher.download(u, dir, MAX_ARTWORK_BYTES, &|h| host_permitted(manifest, h)).ok().map(|p| p.to_string_lossy().into_owned())
                    };
                    for r in chunk.iter_mut() {
                        r.poster = local(r.item.poster.as_deref());
                        r.thumbnail = local(r.item.thumbnail.as_deref());
                        r.logo = local(r.item.logo.as_deref());
                    }
                });
            }
        });
        Ok(ready)
    }

    fn resolve(&self, entry: &PluginEntry, action: &str) -> Result<Action, String> {
        let plugin = self.loaded(&entry.id, &entry.manifest)?;
        self.host.resolve(&plugin, action).map_err(|e| e.to_string())
    }
}

fn ready_chunk(len: usize) -> usize {
    (len / 4).max(1)
}

/// Install-time data for the engine: which plugins are enabled and what rows they declare.
pub(crate) fn entries_from_rows(rows: &[PluginRow]) -> Vec<PluginEntry> {
    rows.iter()
        .filter_map(|r| {
            let manifest: Manifest = serde_json::from_str(&r.manifest_json).ok()?;
            Some(PluginEntry { id: r.id.clone(), manifest, enabled: r.enabled })
        })
        .collect()
}

pub(crate) fn info_for(row: &PluginRow, manifest: &Manifest, settings: &[(String, String)], items: u32) -> PluginInfo {
    let setting_infos: Vec<PluginSettingInfo> = manifest
        .settings
        .iter()
        .map(|s| {
            let value = settings.iter().find(|(k, _)| k == &s.key).map(|(_, v)| v.clone());
            PluginSettingInfo {
                key: s.key.clone(),
                label: s.label.clone(),
                kind: format!("{:?}", s.kind).to_lowercase(),
                required: s.required,
                help: s.help.clone(),
                has_value: value.as_ref().map(|v| !v.is_empty()).unwrap_or(false),
                value: if s.kind.is_secret() { None } else { value },
            }
        })
        .collect();
    let configured = manifest.settings.iter().filter(|s| s.required).all(|s| settings.iter().any(|(k, v)| k == &s.key && !v.is_empty()));
    PluginInfo {
        id: row.id.clone(),
        name: manifest.name.clone(),
        version: manifest.version.clone(),
        description: manifest.description.clone(),
        publisher: row.publisher.clone(),
        publisher_name: manifest.publisher_name.clone(),
        kinds: manifest.kinds.iter().map(|k| format!("{k:?}").to_lowercase()).collect(),
        fetch_hosts: manifest.fetch_hosts.clone(),
        enabled: row.enabled,
        built_in: row.built_in,
        configured,
        refresh_minutes: manifest.refresh_minutes_or_default(),
        last_refresh_ms: row.last_refresh_ms,
        last_error: row.last_error.clone(),
        items,
        settings: setting_infos,
    }
}

/// The plugin thread: runs refreshes and resolves, and once a minute starts any that are due.
pub(crate) fn spawn(inner: Arc<Inner>, rx: crossbeam_channel::Receiver<PluginJob>, services: Arc<CoreServices>) {
    let _ = services.inner.set(Arc::downgrade(&inner));
    std::thread::Builder::new()
        .name("lounge-plugins".into())
        .spawn(move || loop {
            match rx.recv_timeout(Duration::from_secs(60)) {
                Ok(PluginJob::Refresh(id)) => run_refresh(&inner, &id),
                Ok(PluginJob::Resolve { plugin, action, reply }) => {
                    let entry = inner.state.lock().plugins.iter().find(|p| p.id == plugin).cloned();
                    let result = match entry {
                        Some(e) => inner.plugins.resolve(&e, &action),
                        None => Err("plugin is not installed".into()),
                    };
                    let _ = reply.send(result);
                }
                Err(crossbeam_channel::RecvTimeoutError::Timeout) => {
                    if !inner.screen_on.load(std::sync::atomic::Ordering::Relaxed) {
                        continue;
                    }
                    let due: Vec<String> = {
                        let state = inner.state.lock();
                        let rows = state.store.plugins().unwrap_or_default();
                        let now = now_ms();
                        state
                            .plugins
                            .iter()
                            .filter(|p| p.enabled)
                            .filter(|p| {
                                let last = rows.iter().find(|r| r.id == p.id).and_then(|r| r.last_refresh_ms).unwrap_or(0);
                                now - last >= p.manifest.refresh_minutes_or_default() as i64 * 60_000
                            })
                            .map(|p| p.id.clone())
                            .collect()
                    };
                    for id in due {
                        run_refresh(&inner, &id);
                    }
                }
                Err(crossbeam_channel::RecvTimeoutError::Disconnected) => break,
            }
        })
        .expect("spawn plugin thread");
}

fn run_refresh(inner: &Arc<Inner>, id: &str) {
    let (entry, since) = {
        let state = inner.state.lock();
        let Some(entry) = state.plugins.iter().find(|p| p.id == id && p.enabled).cloned() else { return };
        let since = state.store.plugins().ok().and_then(|rows| rows.into_iter().find(|r| r.id == id).and_then(|r| r.last_refresh_ms));
        (entry, since)
    };
    let ctx = RefreshContext {
        now_ms: now_ms(),
        utc_offset_minutes: inner.utc_offset_minutes.load(std::sync::atomic::Ordering::Relaxed),
        tier: format!("{:?}", inner.config.tier).to_lowercase(),
        since_ms: since,
        window_hours: 12,
    };
    let started = std::time::Instant::now();
    match inner.plugins.refresh(&entry, &ctx) {
        Ok(items) => {
            tracing::info!("plugin {id}: {} items in {} ms", items.len(), started.elapsed().as_millis());
            push_log(&inner.plugins.logs, id, 1, &format!("refreshed: {} items in {} ms", items.len(), started.elapsed().as_millis()));
            // Screensavers never become tiles; they are kept apart for the screensaver.
            let (scenes, problems) = screensavers::from_items(id, &entry.manifest.name, items.iter().map(|r| &r.item));
            for p in &problems {
                push_log(&inner.plugins.logs, id, 2, p);
            }
            let items: Vec<ReadyItem> = items.into_iter().filter(|r| r.item.kind != ApiItemKind::Screensaver).collect();
            {
                let state = inner.state.lock();
                let _ = state.store.plugin_set_refresh(id, now_ms(), None);
                screensavers::save(&state.store, id, &scenes);
            }
            inner.enqueue_job(worker::Job::PluginItems { plugin: id.to_string(), items });
        }
        Err(e) => {
            tracing::warn!("plugin {id}: {e}");
            push_log(&inner.plugins.logs, id, 3, &e);
            let _ = inner.state.lock().store.plugin_set_refresh(id, now_ms(), Some(&e));
            inner.enqueue_job(worker::Job::Rebuild);
        }
    }
}

impl Inner {
    pub(crate) fn plugins_info(&self) -> Vec<PluginInfo> {
        let state = self.state.lock();
        let rows = state.store.plugins().unwrap_or_default();
        rows.iter()
            .filter_map(|row| {
                let manifest: Manifest = serde_json::from_str(&row.manifest_json).ok()?;
                let settings = state.store.plugin_settings(&row.id).unwrap_or_default();
                let items = state.graph.count_source(&format!("plugin:{}", row.id)) as u32;
                Some(info_for(row, &manifest, &settings, items))
            })
            .collect()
    }

    pub(crate) fn plugin_install(&self, path: &Path, built_in: bool) -> Result<PluginInfo, String> {
        let id = lounge_plugins::bundle::Bundle::read(path).map_err(|e| e.to_string())?.manifest.id;
        let existing = self.state.lock().store.plugins().map_err(|e| e.to_string())?.into_iter().find(|r| r.id == id);
        let (row, manifest) = self.plugins.install(path, built_in, existing.as_ref())?;
        let info = {
            let mut state = self.state.lock();
            state.store.plugin_upsert(&row).map_err(|e| e.to_string())?;
            let rows = state.store.plugins().map_err(|e| e.to_string())?;
            state.plugins = entries_from_rows(&rows);
            let settings = state.store.plugin_settings(&row.id).map_err(|e| e.to_string())?;
            let stored = rows.into_iter().find(|r| r.id == row.id).unwrap_or(row.clone());
            info_for(&stored, &manifest, &settings, 0)
        };
        if info.enabled {
            let _ = self.plugins.tx.send(PluginJob::Refresh(info.id.clone()));
        }
        self.enqueue_job(worker::Job::Rebuild);
        Ok(info)
    }

    fn installed(&self, id: &str) -> bool {
        valid_plugin_id(id) && self.state.lock().plugins.iter().any(|p| p.id == id)
    }

    pub(crate) fn plugin_remove(&self, id: &str) {
        if !self.installed(id) {
            return;
        }
        {
            let mut state = self.state.lock();
            let _ = state.store.plugin_remove(id);
            screensavers::clear(&state.store, id);
            state.graph.replace_source(&format!("plugin:{id}"), Vec::new());
            let rows = state.store.plugins().unwrap_or_default();
            state.plugins = entries_from_rows(&rows);
        }
        self.plugins.remove_files(id);
        self.enqueue_job(worker::Job::Rebuild);
    }

    pub(crate) fn plugin_set_enabled(&self, id: &str, enabled: bool) {
        if !self.installed(id) {
            return;
        }
        {
            let mut state = self.state.lock();
            let _ = state.store.plugin_set_enabled(id, enabled);
            if !enabled {
                let _ = state.store.replace_items(&format!("plugin:{id}"), &[], now_ms());
                screensavers::clear(&state.store, id);
                state.graph.replace_source(&format!("plugin:{id}"), Vec::new());
            }
            let rows = state.store.plugins().unwrap_or_default();
            state.plugins = entries_from_rows(&rows);
        }
        if enabled {
            let _ = self.plugins.tx.send(PluginJob::Refresh(id.to_string()));
        }
        self.enqueue_job(worker::Job::Rebuild);
    }

    pub(crate) fn plugin_set_setting(&self, id: &str, key: &str, value: Option<String>) {
        if !self.installed(id) {
            return;
        }
        let enabled = {
            let state = self.state.lock();
            let _ = state.store.plugin_setting_set(id, key, value.as_deref().map(str::trim).filter(|v| !v.is_empty()));
            state.plugins.iter().any(|p| p.id == id && p.enabled)
        };
        if enabled {
            let _ = self.plugins.tx.send(PluginJob::Refresh(id.to_string()));
        }
    }
}

/// Stable negative channel key for a plugin channel, so it never collides with TvProvider ids.
pub(crate) fn channel_key(plugin: &str, channel: &str) -> i64 {
    let hash = blake3::hash(format!("{plugin}\0{channel}").as_bytes());
    let mut bytes = [0u8; 8];
    bytes.copy_from_slice(&hash.as_bytes()[..8]);
    let v = i64::from_le_bytes(bytes) | i64::MIN;
    if v == i64::MIN { i64::MIN + 1 } else { v }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channel_keys_are_negative_and_stable() {
        let a = channel_key("tv.lounge.iptv", "abc.au");
        assert!(a < 0);
        assert_eq!(a, channel_key("tv.lounge.iptv", "abc.au"));
        assert_ne!(a, channel_key("tv.lounge.iptv", "sbs.au"));
    }
}
