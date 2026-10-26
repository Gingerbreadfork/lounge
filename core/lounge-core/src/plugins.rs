//! Plugins inside the core: bundles live under `<data_dir>/plugins/<id>/`, their key-values and
//! settings in the store. One thread runs refreshes on a schedule or on demand and hands the items
//! to the worker; another answers tile presses and searches, so neither waits on the other.

use crate::{now_ms, screensavers, worker, Inner, PluginInfo, PluginSettingInfo};
use lounge_plugin_api::{valid_plugin_id, Action, HttpRequest, HttpResponse, Item as ApiItem, ItemKind as ApiItemKind, Kind, Manifest, RefreshContext};
use lounge_plugins::bundle::Bundle;
use lounge_plugins::host::{Host, Limits, LoadedPlugin, Services};
use lounge_plugins::http::Fetcher;
use lounge_store::PluginRow;
use parking_lot::Mutex;
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock, Weak};
use std::time::{Duration, Instant};

const LOG_LINES: usize = 200;
const LOG_LINE_BYTES: usize = 4096;
const MAX_ITEMS: usize = 2000;
const MAX_ARTWORK_BYTES: usize = 4 * 1024 * 1024;
/// What one refresh may download: new pictures, their bytes, and the time it may spend.
const ART_DOWNLOADS: usize = 600;
const ART_BYTES: u64 = 192 * 1024 * 1024;
const ART_TIME: Duration = Duration::from_secs(120);
const ART_THREADS: usize = 4;
/// Results asked of each plugin per search, and the time their pictures may take.
const SEARCH_LIMIT: u32 = 12;
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

/// A plugin's fetcher and the local hosts it was built for.
type Guarded = (Vec<String>, Arc<Fetcher>);

/// One guarded fetcher per plugin, rebuilt when the hosts it names change.
#[derive(Default)]
pub(crate) struct Fetchers {
    map: Mutex<HashMap<String, Guarded>>,
}

impl Fetchers {
    fn for_plugin(&self, id: &str, manifest: &Manifest) -> Arc<Fetcher> {
        let local = local_hosts(manifest);
        let mut map = self.map.lock();
        if let Some((hosts, f)) = map.get(id)
            && *hosts == local
        {
            return Arc::clone(f);
        }
        let f = Arc::new(Fetcher::guarded(local.clone()));
        map.insert(id.to_string(), (local, Arc::clone(&f)));
        f
    }

    fn forget(&self, id: &str) {
        self.map.lock().remove(id);
    }
}

/// Hosts that may lead back into the TV: only loopback names and address literals the plugin
/// lists itself, so no DNS name can point a plugin at the TV's own services.
fn local_hosts(manifest: &Manifest) -> Vec<String> {
    let mut hosts: Vec<String> = manifest
        .fetch_hosts
        .iter()
        .map(|h| h.trim().trim_start_matches('[').trim_end_matches(']').to_ascii_lowercase())
        .filter(|h| h == "localhost" || h.ends_with(".localhost") || h.parse::<std::net::IpAddr>().is_ok())
        .collect();
    hosts.sort();
    hosts.dedup();
    hosts
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
    /// The newest search; older ones still queued answer with nothing.
    pub search_generation: AtomicU64,
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
        let fetcher = self.fetchers.for_plugin(plugin, &manifest);
        fetcher.fetch(request, &|host| host_permitted(&manifest, host)).map_err(|e| e.to_string())
    }
}

/// The manifest's allow list, applied to every hop by name. The fetcher then checks the address
/// each name resolves to, so loopback and the TV's own addresses stay out of reach whatever the
/// name; this check keeps the obvious spellings from even being tried.
pub(crate) fn host_permitted(manifest: &Manifest, host: &str) -> bool {
    if !manifest.allows_host(host) {
        return false;
    }
    let h = host.to_ascii_lowercase();
    let local = h == "localhost" || h == "0.0.0.0" || h == "::1" || h == "::" || h.starts_with("127.") || h.ends_with(".localhost");
    !local || manifest.fetch_hosts.iter().any(|allowed| allowed.eq_ignore_ascii_case(&h))
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
            search_generation: AtomicU64::new(0),
        });
        (manager, refresh_rx, calls_rx, services)
    }

    pub fn logs(&self, plugin: &str) -> Vec<String> {
        self.logs.lock().get(plugin).map(|q| q.iter().cloned().collect()).unwrap_or_default()
    }

    /// Queue a refresh on the refresh thread.
    pub fn refresh_soon(&self, id: &str) {
        let _ = self.refresh_tx.send(PluginJob::Refresh(id.to_string()));
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
        self.fetchers.forget(&bundle.manifest.id);
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
        let items = self.host.refresh(&plugin, ctx).map_err(|e| e.to_string())?;
        let mut notes = Vec::new();
        let (mut items, repeats) = unique_by_id(items);
        if repeats > 0 {
            notes.push(format!("{repeats} items repeated an id and were left out; ids must be unique"));
        }
        if items.len() > MAX_ITEMS {
            notes.push(format!("{} items sent; only the first {MAX_ITEMS} are kept", items.len()));
            items.truncate(MAX_ITEMS);
        }
        let dir = self.plugin_art_dir(&entry.id).ok_or_else(|| format!("invalid plugin id {:?}", entry.id))?;
        let fetcher = self.fetchers.for_plugin(&entry.id, &entry.manifest);
        let budget = ArtBudget { downloads: ART_DOWNLOADS, bytes: ART_BYTES, deadline: Instant::now() + ART_TIME };
        let (ready, skipped) = with_artwork(&fetcher, &entry.manifest, &dir, items, &budget);
        if skipped > 0 {
            notes.push(format!("{skipped} pictures were not downloaded this time; they come with later refreshes"));
        }
        prune_art_dir(&dir, &ready);
        Ok((ready, notes))
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
                        let (mut items, _) = unique_by_id(items);
                        items.retain(|it| !it.title.trim().is_empty() && it.kind != ApiItemKind::Screensaver && it.kind != ApiItemKind::Wallpaper);
                        items.truncate(SEARCH_LIMIT as usize);
                        if items.is_empty() {
                            return None;
                        }
                        let dir = self.plugin_art_dir(&entry.id)?;
                        let fetcher = self.fetchers.for_plugin(&entry.id, &entry.manifest);
                        // One picture per result: the poster or the thumbnail, whichever it has.
                        for it in items.iter_mut() {
                            if it.poster.is_some() {
                                it.thumbnail = None;
                            }
                            it.logo = None;
                        }
                        let budget = ArtBudget { downloads: SEARCH_LIMIT as usize, bytes: 24 * 1024 * 1024, deadline };
                        let (ready, _) = with_artwork(&fetcher, &entry.manifest, &dir, items, &budget);
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
fn unique_by_id(items: Vec<ApiItem>) -> (Vec<ApiItem>, usize) {
    let before = items.len();
    let mut seen = HashSet::new();
    let kept: Vec<ApiItem> = items.into_iter().filter(|it| seen.insert(it.id.clone())).collect();
    let dropped = before - kept.len();
    (kept, dropped)
}

struct ArtBudget {
    downloads: usize,
    bytes: u64,
    deadline: Instant,
}

/// Download the pictures `items` name, each URL once, earliest items first, within `budget`.
/// Pictures already cached cost nothing; the rest are what the budget limits. Returns the items
/// with local paths and how many pictures the budget left out.
fn with_artwork(fetcher: &Fetcher, manifest: &Manifest, dir: &Path, items: Vec<ApiItem>, budget: &ArtBudget) -> (Vec<ReadyItem>, usize) {
    let mut urls: Vec<String> = Vec::new();
    let mut seen = HashSet::new();
    for it in &items {
        for u in [&it.poster, &it.thumbnail, &it.logo].into_iter().flatten() {
            let permitted = lounge_plugins::http::host_of(u).is_some_and(|h| host_permitted(manifest, &h));
            if permitted && seen.insert(u.clone()) {
                urls.push(u.clone());
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
                if let Ok(d) = fetcher.download(u, dir, MAX_ARTWORK_BYTES, timeout, &|h| host_permitted(manifest, h)) {
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
                kind: format!("{:?}", s.kind).to_lowercase(),
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
                                let last = rows.iter().find(|r| r.id == p.id).and_then(|r| r.last_refresh_ms).unwrap_or(0);
                                now - last >= p.manifest.refresh_minutes_or_default() as i64 * 60_000
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

fn run_refresh(inner: &Arc<Inner>, id: &str) {
    let (entry, since) = {
        let state = inner.state.lock();
        let Some(entry) = state.plugins.iter().find(|p| p.id == id && p.enabled).cloned() else { return };
        let since = state.store.plugins().ok().and_then(|rows| rows.into_iter().find(|r| r.id == id).and_then(|r| r.last_refresh_ms));
        (entry, since)
    };
    let ctx = RefreshContext {
        now_ms: now_ms(),
        utc_offset_minutes: inner.utc_offset_minutes.load(Ordering::Relaxed),
        tier: format!("{:?}", inner.config.tier).to_lowercase(),
        since_ms: since,
        window_hours: 12,
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
            self.plugins.refresh_soon(&info.id);
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
            let _ = state.store.plugin_setting_set(id, key, value.as_deref().map(str::trim).filter(|v| !v.is_empty()));
            state.plugins.iter().any(|p| p.id == id && p.enabled)
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
    fn repeated_ids_keep_the_first() {
        let a = ApiItem { id: "x".into(), title: "First".into(), ..Default::default() };
        let b = ApiItem { id: "x".into(), title: "Second".into(), ..Default::default() };
        let c = ApiItem { id: "y".into(), title: "Other".into(), ..Default::default() };
        let (kept, dropped) = unique_by_id(vec![a, b, c]);
        assert_eq!(dropped, 1);
        assert_eq!(kept.iter().map(|i| i.title.as_str()).collect::<Vec<_>>(), vec!["First", "Other"]);
    }

    #[test]
    fn only_literal_local_names_may_reach_the_tv() {
        let m = Manifest { fetch_hosts: vec!["*".into(), "localhost".into(), "[::1]".into(), "192.168.1.5".into(), "evil.example".into(), "*.local".into()], ..Default::default() };
        assert_eq!(local_hosts(&m), vec!["192.168.1.5", "::1", "localhost"]);
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
}
