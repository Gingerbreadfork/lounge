//! Plugins inside the core: bundles live under `<data_dir>/plugins/<id>/`, their key-values and
//! settings in the store. One thread runs refreshes on a schedule or on demand and hands the items
//! to the worker; another answers tile presses and searches, so neither waits on the other.

use crate::{now_ms, screensavers, worker, Inner, PluginInfo, PluginSettingInfo};
use lounge_plugin_api::{valid_plugin_id, Action, HttpRequest, HttpResponse, Item as ApiItem, ItemKind as ApiItemKind, Kind, Manifest, RefreshContext};
use lounge_plugins::bundle::Bundle;
use lounge_plugins::host::{kept, kept_results, Host, Limits, LoadedPlugin, Services, MAX_ARTWORK_BYTES, SEARCH_LIMIT, WINDOW_HOURS};
use lounge_plugins::http::{host_permitted, reach, Fetcher, Reach};
use lounge_store::PluginRow;
use parking_lot::Mutex;
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock, Weak};
use std::time::{Duration, Instant};

const LOG_LINES: usize = 200;
const LOG_LINE_BYTES: usize = 4096;
/// A wallpaper fills the panel, so its picture may be as large as the decoder will take.
const MAX_WALLPAPER_BYTES: usize = lounge_artwork::MAX_SOURCE_BYTES as usize;
/// What one refresh may download: new pictures, their bytes, and the time it may spend.
const ART_DOWNLOADS: usize = 600;
const ART_BYTES: u64 = 192 * 1024 * 1024;
const ART_TIME: Duration = Duration::from_secs(120);
const ART_THREADS: usize = 4;
/// The time a search's pictures may take.
const SEARCH_ART_TIME: Duration = Duration::from_secs(4);

pub(crate) enum PluginJob {
    Refresh(String),
    Resolve {
        plugin: String,
        action: String,
        reply: crossbeam_channel::Sender<Result<Action, String>>,
    },
    Search {
        query: String,
        generation: u64,
        reply: crossbeam_channel::Sender<Vec<SearchGroup>>,
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
#[derive(Clone)]
pub(crate) struct ReadyItem {
    pub item: ApiItem,
    pub poster: Option<String>,
    pub thumbnail: Option<String>,
    pub logo: Option<String>,
}

/// One plugin's answer to a search, pictures downloaded.
pub(crate) struct SearchGroup {
    pub plugin: String,
    pub plugin_name: String,
    pub items: Vec<ReadyItem>,
}

/// A plugin's fetcher and where it was built to reach.
type Guarded = (Reach, Arc<Fetcher>);

/// One guarded fetcher per plugin, rebuilt when where it may reach changes.
#[derive(Default)]
pub(crate) struct Fetchers {
    map: Mutex<HashMap<String, Guarded>>,
}

impl Fetchers {
    fn for_plugin(&self, id: &str, manifest: &Manifest, setting: &dyn Fn(&str) -> Option<String>) -> Arc<Fetcher> {
        let reach = reach(manifest, setting);
        let mut map = self.map.lock();
        if let Some((built_for, f)) = map.get(id)
            && *built_for == reach
        {
            return Arc::clone(f);
        }
        let f = Arc::new(Fetcher::guarded(reach.clone()));
        map.insert(id.to_string(), (reach, Arc::clone(&f)));
        f
    }

    fn forget(&self, id: &str) {
        self.map.lock().remove(id);
    }
}

pub(crate) struct PluginManager {
    host: Host,
    dir: PathBuf,
    art_dir: PathBuf,
    loaded: Mutex<HashMap<String, Arc<LoadedPlugin>>>,
    logs: Arc<Mutex<HashMap<String, VecDeque<String>>>>,
    refresh_tx: crossbeam_channel::Sender<PluginJob>,
    calls_tx: crossbeam_channel::Sender<PluginJob>,
    fetchers: Arc<Fetchers>,
    services: Arc<CoreServices>,
    /// The newest search; older ones still queued answer with nothing.
    pub search_generation: AtomicU64,
    /// Held through an install, so two of one id cannot both pass the publisher check.
    pub installing: Mutex<()>,
    /// Plugins with a refresh waiting to run; asking again meanwhile adds nothing.
    refresh_queued: Mutex<HashSet<String>>,
}

/// The host's view of storage, backed by the core's store once it exists.
pub(crate) struct CoreServices {
    inner: OnceLock<Weak<Inner>>,
    fetchers: Arc<Fetchers>,
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
        self.with_store(|s| s.plugin_setting(plugin, key).ok().flatten()).flatten()
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
        let setting = |key: &str| self.setting(plugin, key);
        let fetcher = self.fetchers.for_plugin(plugin, &manifest, &setting);
        fetcher.fetch(request, &|host| host_permitted(&manifest, host, &setting)).map_err(|e| e.to_string())
    }
}

/// Whether manifest version `a` is older than `b`, by dot-separated numbers. A version that is
/// not made of numbers is never older, so it can always be installed.
fn older(a: &str, b: &str) -> bool {
    let parse = |v: &str| v.trim().trim_start_matches('v').split('.').map(|p| p.parse::<u64>().ok()).collect::<Option<Vec<u64>>>();
    let (Some(mut x), Some(mut y)) = (parse(a), parse(b)) else { return false };
    let len = x.len().max(y.len());
    x.resize(len, 0);
    y.resize(len, 0);
    x < y
}

/// Stores one of a plugin's settings. Moving an address setting to another host forgets the
/// plugin's passwords, so a password only ever goes to the server it was entered for, whoever
/// changes the address. A secret that is itself an address is kept, as it goes only to its own.
pub(crate) fn store_setting(store: &lounge_store::Store, manifest: &Manifest, id: &str, key: &str, value: Option<&str>) {
    // A password is kept exactly as typed, since spaces at its ends can be part of it.
    let password = manifest.settings.iter().any(|s| s.key == key && s.kind == lounge_plugin_api::SettingKind::Password);
    let value = value.map(|v| if password { v } else { v.trim() }).filter(|v| !v.is_empty());
    if manifest.decides_origin(key) {
        let old = store.plugin_setting(id, key).ok().flatten();
        let host = |v: Option<&str>| v.and_then(lounge_plugin_api::host_in_setting);
        if old.is_some() && host(old.as_deref()) != host(value) {
            for secret in manifest.settings.iter().filter(|s| s.kind.is_secret() && s.key != key && !manifest.decides_origin(&s.key)) {
                let _ = store.plugin_setting_set(id, &secret.key, None);
            }
        }
    }
    let _ = store.plugin_setting_set(id, key, value);
}

/// `text` cut to at most `max` bytes on a character boundary.
pub(crate) fn clip_bytes(text: &str, max: usize) -> &str {
    if text.len() <= max {
        return text;
    }
    let mut end = max;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

fn push_log(logs: &Mutex<HashMap<String, VecDeque<String>>>, plugin: &str, level: i32, message: &str) {
    let tag = ["debug", "info", "warn", "error"].get(level as usize).copied().unwrap_or("log");
    let mut logs = logs.lock();
    let q = logs.entry(plugin.to_string()).or_default();
    if q.len() >= LOG_LINES {
        q.pop_front();
    }
    q.push_back(format!("{} {tag}: {}", now_ms(), clip_bytes(message, LOG_LINE_BYTES)));
}

impl PluginManager {
    pub fn new(
        data_dir: &Path,
        cache_dir: &Path,
    ) -> (Arc<PluginManager>, crossbeam_channel::Receiver<PluginJob>, crossbeam_channel::Receiver<PluginJob>, Arc<CoreServices>) {
        let logs = Arc::new(Mutex::new(HashMap::new()));
        let fetchers = Arc::new(Fetchers::default());
        let services = Arc::new(CoreServices { inner: OnceLock::new(), fetchers: Arc::clone(&fetchers), logs: Arc::clone(&logs) });
        let (refresh_tx, refresh_rx) = crossbeam_channel::unbounded();
        let (calls_tx, calls_rx) = crossbeam_channel::unbounded();
        let manager = Arc::new(PluginManager {
            host: Host::new(services.clone(), Limits::default()),
            dir: data_dir.join("plugins"),
            art_dir: cache_dir.join("plugin-art"),
            loaded: Mutex::new(HashMap::new()),
            logs,
            refresh_tx,
            calls_tx,
            fetchers,
            services: Arc::clone(&services),
            search_generation: AtomicU64::new(0),
            installing: Mutex::new(()),
            refresh_queued: Mutex::new(HashSet::new()),
        });
        (manager, refresh_rx, calls_rx, services)
    }

    pub fn logs(&self, plugin: &str) -> Vec<String> {
        self.logs.lock().get(plugin).map(|q| q.iter().cloned().collect()).unwrap_or_default()
    }

    /// Queue a refresh on the refresh thread.
    pub fn refresh_soon(&self, id: &str) {
        if self.refresh_queued.lock().insert(id.to_string()) {
            let _ = self.refresh_tx.send(PluginJob::Refresh(id.to_string()));
        }
    }

    /// Queue a resolve or search on the thread that answers presses.
    pub fn call(&self, job: PluginJob) -> Result<(), String> {
        self.calls_tx.send(job).map_err(|_| "plugin thread is gone".to_string())
    }

    /// `<plugins>/<id>`; an id that is not a single plain path component never becomes a path.
    fn plugin_dir(&self, id: &str) -> Option<PathBuf> {
        single_component(&self.dir, id)
    }

    fn plugin_art_dir(&self, id: &str) -> Option<PathBuf> {
        single_component(&self.art_dir, id)
    }

    /// Verify, pin the publisher and copy a bundle into place. Returns the stored row.
    pub fn install(&self, path: &Path, built_in: bool, existing: Option<&PluginRow>) -> Result<(PluginRow, Manifest), String> {
        let bundle = Bundle::read(path).map_err(|e| e.to_string())?;
        bundle.verify().map_err(|e| e.to_string())?;
        let installed: Option<Manifest> = existing.and_then(|prev| serde_json::from_str(&prev.manifest_json).ok());
        if let Some(prev) = existing {
            if prev.publisher != bundle.publisher_hex() {
                // The APK vouches for the plugins it carries, so its copy replaces one an earlier
                // APK installed even when another machine signed it.
                if !(built_in && prev.built_in) {
                    return Err(format!("{} is signed by a different publisher than the installed version", bundle.manifest.name));
                }
                tracing::info!("built-in plugin {} changed publisher from {} to {}", bundle.manifest.id, prev.publisher, bundle.publisher_hex());
            }
            // A copy from the phone may be newer than the one bundled with Lounge; neither rolls back.
            if let Some(old) = &installed
                && older(&bundle.manifest.version, &old.version)
            {
                return Err(format!("{} {} is installed already, newer than this {}", bundle.manifest.name, old.version, bundle.manifest.version));
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
        self.fetchers.forget(&bundle.manifest.id);
        // An update that asks to reach more is turned off until the person allows it again. The
        // ones bundled with Lounge come with the APK the person installed.
        let widened: Vec<String> = match &installed {
            Some(old) if !built_in => bundle.manifest.fetch_hosts.iter().filter(|h| !old.fetch_hosts.contains(h)).cloned().collect(),
            _ => Vec::new(),
        };
        let was_on = existing.is_some_and(|p| p.enabled);
        if was_on && !widened.is_empty() {
            push_log(&self.logs, &bundle.manifest.id, 2, &format!("turned off by the update, which also asks to reach {}; turn it on to allow that", widened.join(", ")));
        }
        let row = PluginRow {
            id: bundle.manifest.id.clone(),
            manifest_json: String::from_utf8_lossy(&bundle.manifest_json).into_owned(),
            publisher: bundle.publisher_hex(),
            enabled: was_on && widened.is_empty(),
            built_in,
            installed_ms: now_ms(),
            last_refresh_ms: None,
            last_error: None,
            last_success_ms: None,
        };
        Ok((row, bundle.manifest))
    }

    pub fn remove_files(&self, id: &str) {
        self.loaded.lock().remove(id);
        self.fetchers.forget(id);
        for dir in [self.plugin_dir(id), self.plugin_art_dir(id)].into_iter().flatten() {
            let _ = std::fs::remove_dir_all(dir);
        }
    }

    /// Art folders of plugins that are no longer installed.
    pub fn prune_orphan_art(&self, installed: &[String]) {
        let Ok(entries) = std::fs::read_dir(&self.art_dir) else { return };
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if !installed.contains(&name) && valid_plugin_id(&name) {
                let _ = std::fs::remove_dir_all(e.path());
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

    /// Run a refresh now and download its artwork; returns items ready for the worker and
    /// sentences worth putting in the plugin's log.
    fn refresh(&self, entry: &PluginEntry, ctx: &RefreshContext) -> Result<(Vec<ReadyItem>, Vec<String>), String> {
        let plugin = self.loaded(&entry.id, &entry.manifest)?;
        let (items, sent) = self.host.refresh_counted(&plugin, ctx).map_err(|e| e.to_string())?;
        let (items, mut notes) = kept(items, sent);
        let dir = self.plugin_art_dir(&entry.id).ok_or_else(|| format!("invalid plugin id {:?}", entry.id))?;
        let settings = self.settings_of(&entry.id);
        let setting = |key: &str| settings.get(key).cloned();
        let fetcher = self.fetchers.for_plugin(&entry.id, &entry.manifest, &setting);
        let permitted = |host: &str| host_permitted(&entry.manifest, host, &setting);
        let budget = ArtBudget { downloads: ART_DOWNLOADS, bytes: ART_BYTES, deadline: Instant::now() + ART_TIME };
        let (ready, skipped) = with_artwork(&fetcher, &permitted, &dir, items, &budget);
        if skipped > 0 {
            notes.push(format!("{skipped} pictures were not downloaded this time; they come with later refreshes"));
        }
        prune_art_dir(&dir, &ready);
        Ok((ready, notes))
    }

    /// A plugin's settings as they are now, for what its allow list's `$key` entries stand for.
    fn settings_of(&self, id: &str) -> HashMap<String, String> {
        self.services.with_store(|s| s.plugin_settings(id).unwrap_or_default()).unwrap_or_default().into_iter().collect()
    }

    fn resolve(&self, entry: &PluginEntry, action: &str) -> Result<Action, String> {
        let plugin = self.loaded(&entry.id, &entry.manifest)?;
        self.host.resolve(&plugin, action).map_err(|e| e.to_string())
    }

    /// Ask every searchable plugin in `entries` at once; each answer gets one picture per item.
    fn search(&self, entries: &[PluginEntry], query: &str) -> Vec<SearchGroup> {
        let deadline = Instant::now() + SEARCH_ART_TIME;
        let found: Vec<Option<SearchGroup>> = std::thread::scope(|scope| {
            let handles: Vec<_> = entries
                .iter()
                .map(|entry| {
                    scope.spawn(move || {
                        let plugin = self.loaded(&entry.id, &entry.manifest).ok()?;
                        if !self.host.can_search(&plugin) {
                            return None;
                        }
                        let items = match self.host.search(&plugin, query, SEARCH_LIMIT) {
                            Ok(items) => items,
                            Err(e) => {
                                push_log(&self.logs, &entry.id, 2, &format!("search failed: {e}"));
                                return None;
                            }
                        };
                        let mut items = kept_results(items);
                        if items.is_empty() {
                            return None;
                        }
                        let dir = self.plugin_art_dir(&entry.id)?;
                        let settings = self.settings_of(&entry.id);
                        let setting = |key: &str| settings.get(key).cloned();
                        let fetcher = self.fetchers.for_plugin(&entry.id, &entry.manifest, &setting);
                        let permitted = |host: &str| host_permitted(&entry.manifest, host, &setting);
                        // One picture per result: the poster or the thumbnail, whichever it has.
                        for it in items.iter_mut() {
                            if it.poster.is_some() {
                                it.thumbnail = None;
                            }
                            it.logo = None;
                        }
                        let budget = ArtBudget { downloads: SEARCH_LIMIT as usize, bytes: 24 * 1024 * 1024, deadline };
                        let (ready, _) = with_artwork(&fetcher, &permitted, &dir, items, &budget);
                        Some(SearchGroup { plugin: entry.id.clone(), plugin_name: entry.manifest.name.clone(), items: ready })
                    })
                })
                .collect();
            handles.into_iter().map(|h| h.join().ok().flatten()).collect()
        });
        found.into_iter().flatten().collect()
    }
}

fn single_component(root: &Path, id: &str) -> Option<PathBuf> {
    if !valid_plugin_id(id) {
        return None;
    }
    let mut parts = Path::new(id).components();
    match (parts.next(), parts.next()) {
        (Some(std::path::Component::Normal(_)), None) => Some(root.join(id)),
        _ => None,
    }
}

/// The first item for each id, and how many repeats were dropped.
struct ArtBudget {
    downloads: usize,
    bytes: u64,
    deadline: Instant,
}

/// Download the pictures `items` name, each URL once, earliest items first, within `budget`.
/// Pictures already cached cost nothing; the rest are what the budget limits. Returns the items
/// with local paths and how many pictures the budget left out.
fn with_artwork(fetcher: &Fetcher, permitted: &(dyn Fn(&str) -> bool + Sync), dir: &Path, items: Vec<ApiItem>, budget: &ArtBudget) -> (Vec<ReadyItem>, usize) {
    let mut urls: Vec<String> = Vec::new();
    let mut seen = HashSet::new();
    let mut wallpapers = HashSet::new();
    for it in &items {
        for u in [&it.poster, &it.thumbnail, &it.logo].into_iter().flatten() {
            if !lounge_plugins::http::host_of(u).is_some_and(|h| permitted(&h)) {
                continue;
            }
            if seen.insert(u.clone()) {
                urls.push(u.clone());
            }
            if it.kind == ApiItemKind::Wallpaper {
                wallpapers.insert(u.clone());
            }
        }
    }
    let mut local: HashMap<String, String> = HashMap::new();
    let mut wanted = Vec::new();
    for u in urls {
        match Fetcher::cached(&u, dir) {
            Some(p) => {
                local.insert(u, p.to_string_lossy().into_owned());
            }
            None => wanted.push(u),
        }
    }
    let skipped_by_count = wanted.len().saturating_sub(budget.downloads);
    wanted.truncate(budget.downloads);
    let next = AtomicUsize::new(0);
    let bytes = AtomicU64::new(0);
    let fetched: Mutex<Vec<(String, String)>> = Mutex::new(Vec::new());
    let left_out = AtomicUsize::new(0);
    std::thread::scope(|scope| {
        for _ in 0..ART_THREADS.min(wanted.len()) {
            scope.spawn(|| loop {
                let i = next.fetch_add(1, Ordering::Relaxed);
                let Some(u) = wanted.get(i) else { break };
                let now = Instant::now();
                if now >= budget.deadline || bytes.load(Ordering::Relaxed) >= budget.bytes {
                    left_out.fetch_add(1, Ordering::Relaxed);
                    continue;
                }
                let timeout = budget.deadline - now;
                let max = if wallpapers.contains(u) { MAX_WALLPAPER_BYTES } else { MAX_ARTWORK_BYTES };
                if let Ok(d) = fetcher.download(u, dir, max, timeout, &|h| permitted(h)) {
                    bytes.fetch_add(d.fetched, Ordering::Relaxed);
                    fetched.lock().push((u.clone(), d.path.to_string_lossy().into_owned()));
                }
            });
        }
    });
    local.extend(fetched.into_inner());
    let path = |u: &Option<String>| u.as_ref().and_then(|u| local.get(u).cloned());
    let ready = items
        .into_iter()
        .map(|item| ReadyItem { poster: path(&item.poster), thumbnail: path(&item.thumbnail), logo: path(&item.logo), item })
        .collect();
    (ready, skipped_by_count + left_out.into_inner())
}

/// Remove downloads the latest refresh no longer names, and temporary files left by a crash.
fn prune_art_dir(dir: &Path, ready: &[ReadyItem]) {
    let keep: HashSet<PathBuf> = ready
        .iter()
        .flat_map(|r| [&r.poster, &r.thumbnail, &r.logo])
        .flatten()
        .map(PathBuf::from)
        .collect();
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let hour_ago = std::time::SystemTime::now() - Duration::from_secs(3600);
    for e in entries.flatten() {
        let path = e.path();
        let is_img = path.extension().is_some_and(|x| x == "img");
        let stale_tmp = path.extension().is_some_and(|x| x == "tmp")
            && e.metadata().and_then(|m| m.modified()).is_ok_and(|t| t < hour_ago);
        if (is_img && !keep.contains(&path)) || stale_tmp {
            let _ = std::fs::remove_file(path);
        }
    }
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
                kind: serde_json::to_value(s.kind).ok().and_then(|v| v.as_str().map(str::to_string)).unwrap_or_default(),
                required: s.required,
                help: s.help.clone(),
                default_value: if s.kind.is_secret() { None } else { s.default.clone() },
                has_value: value.as_ref().map(|v| !v.is_empty()).unwrap_or(false),
                value: if s.kind.is_secret() { None } else { value },
            }
        })
        .collect();
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
        configured: configured(manifest, settings),
        refresh_minutes: manifest.refresh_minutes_or_default(),
        last_refresh_ms: row.last_refresh_ms,
        last_error: row.last_error.clone(),
        items,
        settings: setting_infos,
    }
}

fn configured(manifest: &Manifest, settings: &[(String, String)]) -> bool {
    manifest.settings.iter().filter(|s| s.required).all(|s| settings.iter().any(|(k, v)| k == &s.key && !v.is_empty()))
}

/// The refresh thread: runs refreshes, and once a minute starts any that are due. A second
/// thread answers resolves and searches.
pub(crate) fn spawn(
    inner: Arc<Inner>,
    refresh_rx: crossbeam_channel::Receiver<PluginJob>,
    calls_rx: crossbeam_channel::Receiver<PluginJob>,
    services: Arc<CoreServices>,
) {
    let _ = services.inner.set(Arc::downgrade(&inner));
    let calls_inner = Arc::clone(&inner);
    std::thread::Builder::new()
        .name("lounge-plugin-calls".into())
        .spawn(move || {
            while let Ok(job) = calls_rx.recv() {
                crate::guarded("a plugin call", || answer(&calls_inner, job));
            }
        })
        .expect("spawn plugin calls thread");
    std::thread::Builder::new()
        .name("lounge-plugins".into())
        .spawn(move || loop {
            match refresh_rx.recv_timeout(Duration::from_secs(60)) {
                Ok(PluginJob::Refresh(id)) => {
                    // Taken off first, so a request while this one runs queues the next.
                    inner.plugins.refresh_queued.lock().remove(&id);
                    crate::guarded("a plugin refresh", || run_refresh(&inner, &id));
                }
                Ok(other) => {
                    crate::guarded("a plugin call", || answer(&inner, other));
                }
                Err(crossbeam_channel::RecvTimeoutError::Timeout) => {
                    if !inner.screen_on.load(Ordering::Relaxed) {
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
                                let row = rows.iter().find(|r| r.id == p.id);
                                let last = row.and_then(|r| r.last_refresh_ms);
                                refresh_due(last, row.is_some_and(|r| r.last_error.is_some()), p.manifest.refresh_minutes_or_default(), now)
                            })
                            .map(|p| p.id.clone())
                            .collect()
                    };
                    for id in due {
                        crate::guarded("a plugin refresh", || run_refresh(&inner, &id));
                    }
                }
                Err(crossbeam_channel::RecvTimeoutError::Disconnected) => break,
            }
        })
        .expect("spawn plugin thread");
}

fn answer(inner: &Arc<Inner>, job: PluginJob) {
    match job {
        PluginJob::Refresh(id) => run_refresh(inner, &id),
        PluginJob::Resolve { plugin, action, reply } => {
            let entry = inner.state.lock().plugins.iter().find(|p| p.id == plugin && p.enabled).cloned();
            let result = match entry {
                Some(e) => inner.plugins.resolve(&e, &action),
                None => Err("the plugin is turned off or not installed".into()),
            };
            let _ = reply.send(result);
        }
        PluginJob::Search { query, generation, reply } => {
            if generation < inner.plugins.search_generation.load(Ordering::Relaxed) {
                let _ = reply.send(Vec::new());
                return;
            }
            let entries: Vec<PluginEntry> = {
                let state = inner.state.lock();
                state
                    .plugins
                    .iter()
                    .filter(|p| p.enabled && p.manifest.kinds.contains(&Kind::Search))
                    .filter(|p| configured(&p.manifest, &state.store.plugin_settings(&p.id).unwrap_or_default()))
                    .cloned()
                    .collect()
            };
            let _ = reply.send(inner.plugins.search(&entries, &query));
        }
    }
}

/// How soon a failed refresh is tried again, when the plugin's own interval is longer. A sign-in
/// that waits on the user fails until it is done, and its code must not expire in between.
const RETRY_AFTER_FAILURE_MS: i64 = 5 * 60_000;

/// Whether a plugin last refreshed at `last_ms` is due again at `now`.
fn refresh_due(last_ms: Option<i64>, failed: bool, interval_minutes: u32, now: i64) -> bool {
    let Some(last) = last_ms else { return true };
    let mut every = interval_minutes as i64 * 60_000;
    if failed {
        every = every.min(RETRY_AFTER_FAILURE_MS);
    }
    now - last >= every
}

fn run_refresh(inner: &Arc<Inner>, id: &str) {
    let (entry, since) = {
        let state = inner.state.lock();
        let Some(entry) = state.plugins.iter().find(|p| p.id == id && p.enabled).cloned() else { return };
        let since = state.store.plugins().ok().and_then(|rows| rows.into_iter().find(|r| r.id == id).and_then(|r| r.last_success_ms));
        (entry, since)
    };
    let ctx = RefreshContext {
        now_ms: now_ms(),
        utc_offset_minutes: inner.utc_offset_minutes.load(Ordering::Relaxed),
        tier: format!("{:?}", inner.config.tier).to_lowercase(),
        since_ms: since,
        window_hours: WINDOW_HOURS,
    };
    let started = Instant::now();
    // A panic while running the plugin or fetching its pictures is that refresh's error, so the
    // plugin waits its interval rather than failing again every minute.
    let outcome = crate::guarded("a plugin refresh", || inner.plugins.refresh(&entry, &ctx))
        .unwrap_or_else(|| Err("the refresh stopped on an error inside Lounge".into()));
    match outcome {
        Ok((items, notes)) => {
            tracing::info!("plugin {id}: {} items in {} ms", items.len(), started.elapsed().as_millis());
            push_log(&inner.plugins.logs, id, 1, &format!("refreshed: {} items in {} ms", items.len(), started.elapsed().as_millis()));
            for n in &notes {
                push_log(&inner.plugins.logs, id, 2, n);
            }
            // Screensavers never become tiles; they are kept apart for the screensaver.
            let (scenes, problems) = screensavers::from_items(id, &entry.manifest.name, items.iter().map(|r| &r.item));
            for p in &problems {
                push_log(&inner.plugins.logs, id, 2, p);
            }
            let items: Vec<ReadyItem> = items.into_iter().filter(|r| r.item.kind != ApiItemKind::Screensaver).collect();
            {
                let state = inner.state.lock();
                if !state.plugins.iter().any(|p| p.id == id && p.enabled) {
                    return;
                }
                let _ = state.store.plugin_set_refresh(id, now_ms(), None);
                screensavers::save(&state.store, id, &scenes);
            }
            inner.enqueue_job(worker::Job::PluginItems { plugin: id.to_string(), items });
        }
        Err(e) => {
            let e = clip_bytes(&e, 1000).to_string();
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
        let _one_at_a_time = self.plugins.installing.lock();
        let id = lounge_plugins::bundle::Bundle::read(path).map_err(|e| e.to_string())?.manifest.id;
        let existing = self.state.lock().store.plugins().map_err(|e| e.to_string())?.into_iter().find(|r| r.id == id);
        let (row, manifest) = self.plugins.install(path, built_in, existing.as_ref())?;
        // The store keeps a plugin's switch through an update, so one turned off by it is turned
        // off here, which also takes away what it published.
        if existing.as_ref().is_some_and(|e| e.enabled) && !row.enabled {
            self.plugin_set_enabled(&row.id, false);
        }
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
            self.plugins.refresh_soon(&info.id);
        }
        self.enqueue_job(worker::Job::Rebuild);
        Ok(info)
    }

    fn installed(&self, id: &str) -> bool {
        valid_plugin_id(id) && self.state.lock().plugins.iter().any(|p| p.id == id)
    }

    /// Removes a plugin and everything it stored. One that comes with Lounge is only turned off,
    /// since removing it would also free its id for another publisher's bundle.
    pub(crate) fn plugin_remove(&self, id: &str) -> Result<(), String> {
        if !self.installed(id) {
            return Ok(());
        }
        let built_in = self.state.lock().store.plugins().unwrap_or_default().iter().any(|r| r.id == id && r.built_in);
        if built_in {
            return Err("A plugin that comes with Lounge can be turned off but not removed".into());
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
        Ok(())
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
            self.plugins.refresh_soon(id);
        }
        self.enqueue_job(worker::Job::Rebuild);
    }

    pub(crate) fn plugin_set_setting(&self, id: &str, key: &str, value: Option<String>) {
        if !self.installed(id) {
            return;
        }
        let enabled = {
            let state = self.state.lock();
            let Some(entry) = state.plugins.iter().find(|p| p.id == id) else { return };
            store_setting(&state.store, &entry.manifest, id, key, value.as_deref());
            entry.enabled
        };
        if enabled {
            self.plugins.refresh_soon(id);
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

    #[test]
    fn versions_compare_by_their_numbers() {
        assert!(older("1.2", "1.10"));
        assert!(older("0.9.9", "1"));
        assert!(!older("1", "1.0"));
        assert!(!older("2", "1.9"));
        assert!(!older("beta", "1"));
    }

    #[test]
    fn a_failed_refresh_is_tried_again_sooner() {
        let minute = 60_000;
        assert!(refresh_due(None, false, 15, 0));
        assert!(!refresh_due(Some(0), false, 15, 14 * minute));
        assert!(refresh_due(Some(0), false, 15, 15 * minute));
        assert!(refresh_due(Some(0), true, 15, 5 * minute));
        assert!(!refresh_due(Some(0), true, 15, 4 * minute));
        assert!(!refresh_due(Some(0), true, 60, 4 * minute));
        assert!(refresh_due(Some(0), true, 5, 5 * minute));
    }

    #[test]
    fn the_server_a_plugin_is_given_may_be_on_the_home_network() {
        let m = Manifest { fetch_hosts: vec!["$server".into(), "*".into(), "localhost".into()], ..Default::default() };
        let r = reach(&m, &|k| (k == "server").then(|| "http://192.168.0.10:8096".to_string()));
        assert_eq!(r.home, vec!["192.168.0.10", "localhost"]);
        assert_eq!(r.this_device, vec!["localhost"]);
        assert!(!r.any_home_name);
        assert!(host_permitted(&m, "192.168.0.10", &|k| (k == "server").then(|| "http://192.168.0.10:8096".to_string())));
    }

    #[test]
    fn a_password_does_not_follow_the_server_to_another_host() {
        let store = lounge_store::Store::open_in_memory().unwrap();
        let mut m = Manifest { fetch_hosts: vec!["$server".into()], ..Default::default() };
        m.settings.push(lounge_plugin_api::SettingSpec { key: "server".into(), label: "Server".into(), kind: lounge_plugin_api::SettingKind::Url, required: true, help: None, default: None });
        m.settings.push(lounge_plugin_api::SettingSpec { key: "password".into(), label: "Password".into(), kind: lounge_plugin_api::SettingKind::Password, required: false, help: None, default: None });
        let password = |store: &lounge_store::Store| store.plugin_setting("p", "password").unwrap();
        store_setting(&store, &m, "p", "server", Some("http://192.168.0.10:8096"));
        store_setting(&store, &m, "p", "password", Some("hunter2"));
        // The same host with a different path or port keeps it; another host does not.
        store_setting(&store, &m, "p", "server", Some("http://192.168.0.10:8920/jellyfin"));
        assert_eq!(password(&store).as_deref(), Some("hunter2"));
        store_setting(&store, &m, "p", "server", Some("http://attacker.example"));
        assert_eq!(password(&store), None);
        // A password keeps its spaces; an address is tidied.
        store_setting(&store, &m, "p", "password", Some(" pass phrase "));
        assert_eq!(password(&store).as_deref(), Some(" pass phrase "));
        store_setting(&store, &m, "p", "server", Some("  http://192.168.0.10:8096 "));
        assert_eq!(store.plugin_setting("p", "server").unwrap().as_deref(), Some("http://192.168.0.10:8096"));
    }

    #[test]
    fn a_secret_address_stays_when_another_address_moves() {
        let store = lounge_store::Store::open_in_memory().unwrap();
        let mut m = Manifest { fetch_hosts: vec!["$playlist_url".into(), "$epg_url".into()], ..Default::default() };
        for key in ["playlist_url", "epg_url"] {
            m.settings.push(lounge_plugin_api::SettingSpec { key: key.into(), label: key.into(), kind: lounge_plugin_api::SettingKind::SecretUrl, required: false, help: None, default: None });
        }
        store_setting(&store, &m, "p", "playlist_url", Some("http://iptv.example/get.php?username=u&password=p"));
        store_setting(&store, &m, "p", "epg_url", Some("http://iptv.example/xmltv.php"));
        store_setting(&store, &m, "p", "epg_url", Some("https://guide.example/epg.xml.gz"));
        assert_eq!(store.plugin_setting("p", "playlist_url").unwrap().as_deref(), Some("http://iptv.example/get.php?username=u&password=p"));
        assert_eq!(store.plugin_setting("p", "epg_url").unwrap().as_deref(), Some("https://guide.example/epg.xml.gz"));
    }

    #[test]
    fn clipping_respects_characters() {
        assert_eq!(clip_bytes("héllo", 2), "h");
        assert_eq!(clip_bytes("héllo", 3), "hé");
        assert_eq!(clip_bytes("abc", 10), "abc");
    }

    #[test]
    fn pruning_keeps_what_the_refresh_names() {
        let dir = tempfile::tempdir().unwrap();
        let keep = dir.path().join("a.img");
        let stale = dir.path().join("b.img");
        std::fs::write(&keep, b"x").unwrap();
        std::fs::write(&stale, b"y").unwrap();
        let ready = vec![ReadyItem { item: ApiItem::default(), poster: Some(keep.to_string_lossy().into_owned()), thumbnail: None, logo: None }];
        prune_art_dir(dir.path(), &ready);
        assert!(keep.exists());
        assert!(!stale.exists());
    }

    #[test]
    fn a_wallpaper_may_bring_a_larger_picture_than_a_tile() {
        // Every request gets the same 5 MB picture.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            let body = vec![0u8; 5 * 1024 * 1024];
            for mut s in listener.incoming().flatten() {
                let mut buf = [0u8; 4096];
                let _ = std::io::Read::read(&mut s, &mut buf);
                let head = format!("HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n", body.len());
                let _ = std::io::Write::write_all(&mut s, head.as_bytes());
                let _ = std::io::Write::write_all(&mut s, &body);
            }
        });
        let manifest = Manifest { fetch_hosts: vec!["127.0.0.1".into()], ..Default::default() };
        let fetcher = Fetcher::guarded(reach(&manifest, &|_| None));
        let dir = tempfile::tempdir().unwrap();
        let item = |id: &str, kind: ApiItemKind| ApiItem { id: id.into(), title: id.into(), kind, thumbnail: Some(format!("http://127.0.0.1:{port}/{id}.jpg")), ..Default::default() };
        let budget = ArtBudget { downloads: 10, bytes: 64 * 1024 * 1024, deadline: Instant::now() + Duration::from_secs(20) };
        let (ready, _) = with_artwork(&fetcher, &|h| host_permitted(&manifest, h, &|_| None), dir.path(), vec![item("painting", ApiItemKind::Wallpaper), item("tile", ApiItemKind::Program)], &budget);
        assert!(ready[0].thumbnail.is_some(), "the wallpaper's picture is kept");
        assert!(ready[1].thumbnail.is_none(), "a tile's picture over 4 MB is not");
    }
}
