//! The core's single worker thread: ingests records, mutates the graph and store,
//! rebuilds the snapshot and notifies listeners.

use crate::plugins::{channel_key, ReadyItem};
use crate::{
    engine, now_ms, AppRecord, AppUsage, Inner, InputRecord, InputState, LayoutCommand, NowPlaying, Permissions,
    PlaybackStatus, ProgramKind, ProgramRecord, SessionRecord, State, WatchNextType,
};
use lounge_plugin_api::ItemKind as PluginItemKind;
use lounge_artwork::{prepare_many, Prepared, Style};
use lounge_graph::{Artwork, DeepLink, Item, ItemKind};
use parking_lot::Mutex;
use rayon::prelude::*;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};

const SESSION_HISTORY: usize = 20;
/// The longest the snapshot goes without a rebuild while the screen is on, so relative times
/// such as "23 minutes left" and what is airing stay true.
const TICK: Duration = Duration::from_secs(60);
const HOUSEKEEPING_EVERY: Duration = Duration::from_secs(6 * 3600);
const FIRST_HOUSEKEEPING: Duration = Duration::from_secs(180);
/// Prepared pictures nothing has used for this long may be deleted.
const ART_KEEP: Duration = Duration::from_secs(24 * 3600);
/// Schedule entries this far ahead get their pictures now; later ones on a later ingest.
const SCHEDULE_ART_AHEAD_MS: i64 = 12 * 3_600_000;

pub(crate) enum Job {
    /// First job after open: read the stored items and usage into the graph.
    LoadGraph,
    IngestApps(Vec<AppRecord>),
    IngestPrograms(Vec<ProgramRecord>),
    IngestInputs(Vec<InputRecord>),
    IngestSessions(Vec<SessionRecord>),
    /// A plugin's refresh finished; its artwork is already on disk.
    PluginItems { plugin: String, items: Vec<ReadyItem> },
    /// Prepare the next few pictures of ingests that had too many to do at once.
    MoreArt,
    Usage(Vec<AppUsage>),
    Command(LayoutCommand),
    Permissions(Permissions),
    UpdateNotice(Option<String>),
    UtcOffset(i32),
    Rebuild,
    /// Delete prepared pictures and downloads nothing uses any more.
    Housekeeping,
}

/// Ingests whose pictures did not all fit in one pass. Each pass prepares a few, publishes what
/// it has, and queues the next, so layout commands and other ingests never wait for long.
#[derive(Default)]
pub(crate) struct PendingArt {
    apps: Option<Vec<AppRecord>>,
    /// The latest programme records, kept in `State::last_programs`, still want pictures.
    programs: bool,
    plugins: HashMap<String, Vec<ReadyItem>>,
    queued: bool,
}

pub(crate) fn spawn(inner: Arc<Inner>, rx: crossbeam_channel::Receiver<Job>) {
    std::thread::Builder::new()
        .name("lounge-core".into())
        .spawn(move || {
            let mut utc_offset = inner.config.utc_offset_minutes;
            let mut next_change: Option<i64> = None;
            let mut housekeeping_at = Instant::now() + FIRST_HOUSEKEEPING;
            loop {
                let screen_on = inner.screen_on.load(Ordering::Relaxed);
                let now = Instant::now();
                let mut wake = housekeeping_at;
                if screen_on {
                    let until_change = next_change
                        .map(|at| Duration::from_millis((at - now_ms()).max(0) as u64 + 250))
                        .unwrap_or(TICK)
                        .min(TICK);
                    wake = wake.min(now + until_change);
                }
                let first = match rx.recv_timeout(wake.saturating_duration_since(now)) {
                    Ok(job) => {
                        inner.queue_depth.fetch_sub(1, Ordering::Relaxed);
                        Some(job)
                    }
                    Err(crossbeam_channel::RecvTimeoutError::Timeout) => None,
                    Err(crossbeam_channel::RecvTimeoutError::Disconnected) => break,
                };
                let mut pending: Vec<Job> = first.into_iter().collect();
                while let Ok(next) = rx.try_recv() {
                    inner.queue_depth.fetch_sub(1, Ordering::Relaxed);
                    pending.push(next);
                }
                if Instant::now() >= housekeeping_at {
                    housekeeping_at = Instant::now() + HOUSEKEEPING_EVERY;
                    pending.push(Job::Housekeeping);
                }
                // Nothing arrived: the clock moved on, so what the screen says may have too.
                let mut dirty = pending.is_empty() && screen_on;
                for job in pending {
                    match job {
                        Job::LoadGraph => {
                            if let Err(e) = load_graph(&inner) {
                                tracing::error!("load graph failed: {e}");
                            }
                        }
                        Job::IngestApps(apps) => {
                            if let Err(e) = ingest_apps(&inner, apps) {
                                tracing::error!("ingest apps failed: {e}");
                            }
                            inner.last_ingest_ms.store(now_ms() as u64, Ordering::Relaxed);
                            dirty = true;
                        }
                        Job::IngestPrograms(programs) => {
                            if let Err(e) = ingest_programs(&inner, programs) {
                                tracing::error!("ingest programs failed: {e}");
                            }
                            dirty = true;
                        }
                        Job::IngestInputs(inputs) => {
                            if let Err(e) = ingest_inputs(&inner, inputs) {
                                tracing::error!("ingest inputs failed: {e}");
                            }
                            dirty = true;
                        }
                        Job::PluginItems { plugin, items } => {
                            if let Err(e) = ingest_plugin_items(&inner, &plugin, items) {
                                tracing::error!("ingest plugin {plugin} failed: {e}");
                            }
                            dirty = true;
                        }
                        Job::MoreArt => {
                            more_art(&inner);
                            dirty = true;
                        }
                        Job::IngestSessions(sessions) => {
                            if let Err(e) = ingest_sessions(&inner, sessions) {
                                tracing::error!("ingest sessions failed: {e}");
                            }
                            dirty = true;
                        }
                        Job::Usage(usage) => {
                            if let Err(e) = apply_usage(&inner, usage) {
                                tracing::error!("usage failed: {e}");
                            }
                            dirty = true;
                        }
                        Job::Command(cmd) => {
                            if let Err(e) = apply_command(&inner, cmd) {
                                tracing::error!("command failed: {e}");
                            }
                            dirty = true;
                        }
                        Job::Permissions(p) => {
                            inner.state.lock().permissions = p;
                            dirty = true;
                        }
                        Job::UpdateNotice(v) => {
                            inner.state.lock().update = v;
                            dirty = true;
                        }
                        Job::UtcOffset(m) => {
                            utc_offset = m;
                            dirty = true;
                        }
                        Job::Rebuild => dirty = true,
                        Job::Housekeeping => housekeeping(&inner),
                    }
                }
                if dirty {
                    next_change = rebuild(&inner, utc_offset);
                }
            }
            tracing::info!("worker thread exiting");
        })
        .expect("spawn core worker");
}

fn load_graph(inner: &Inner) -> Result<(), lounge_store::StoreError> {
    let started = std::time::Instant::now();
    let mut state = inner.state.lock();
    let items = state.store.load_items()?;
    let usage = state.store.load_usage()?;
    let count = items.len();
    for it in items {
        state.graph.upsert(it);
    }
    for (pkg, ms) in usage {
        state.graph.set_usage(&pkg, ms);
    }
    state.loaded = true;
    inner.loaded.notify_all();
    tracing::info!("graph loaded: {count} items in {} ms", started.elapsed().as_millis());
    Ok(())
}

// ---------------------------------------------------------------------------
// Artwork: every (source file, style) pair is prepared once, in parallel, and remembered
// in the store keyed by the file's size and mtime so unchanged files cost one lookup.
// ---------------------------------------------------------------------------

pub(crate) struct ArtPlan {
    wants: Vec<(String, Style)>,
    index: HashMap<(String, Style), usize>,
}

/// The pictures of one ingest: what is ready, and whether some sources had to wait.
pub(crate) struct ArtDone {
    pub results: Vec<Option<Prepared>>,
    pub deferred: bool,
}

impl ArtPlan {
    pub(crate) fn new() -> ArtPlan {
        ArtPlan {
            wants: Vec::new(),
            index: HashMap::new(),
        }
    }

    pub(crate) fn want(&mut self, src: Option<&str>, style: Style) -> Option<usize> {
        let src = src?;
        let key = (src.to_string(), style);
        if let Some(&i) = self.index.get(&key) {
            return Some(i);
        }
        let i = self.wants.len();
        self.wants.push(key.clone());
        self.index.insert(key, i);
        Some(i)
    }

    /// Look everything up, then decode at most `sources` of the files that missed, in the order
    /// they were asked for. The state lock is taken per lookup, never across the whole plan.
    pub(crate) fn run(self, state: &Mutex<State>, sources: usize) -> ArtDone {
        let (art_dir, threads) = {
            let st = state.lock();
            (st.art_dir.clone(), st.art_threads)
        };
        let mut signatures: HashMap<&str, Option<(u64, i64)>> = HashMap::new();
        let mut prepared: Vec<Option<Prepared>> = Vec::with_capacity(self.wants.len());
        let mut missed: Vec<bool> = Vec::with_capacity(self.wants.len());
        for (src, style) in &self.wants {
            let sig = *signatures.entry(src.as_str()).or_insert_with(|| lounge_artwork::file_signature(Path::new(src)).ok());
            let Some((len, mtime)) = sig else {
                prepared.push(None);
                missed.push(false);
                continue;
            };
            let json = state.lock().store.artwork_cached_json(&art_key(src, *style), len, mtime).ok().flatten();
            match json.as_deref() {
                // Known not to decode; it is tried again only when the file changes.
                Some("null") => {
                    prepared.push(None);
                    missed.push(false);
                }
                Some(j) => match serde_json::from_str::<Prepared>(j) {
                    Ok(p) if Path::new(&p.path).exists() => {
                        prepared.push(Some(p));
                        missed.push(false);
                    }
                    _ => {
                        prepared.push(None);
                        missed.push(true);
                    }
                },
                None => {
                    prepared.push(None);
                    missed.push(true);
                }
            }
        }
        // Every source that missed is decoded once and all its styles are cut from that decode.
        let mut order: Vec<&str> = Vec::new();
        let mut by_src: HashMap<&str, Vec<(usize, Style)>> = HashMap::new();
        for (i, (src, style)) in self.wants.iter().enumerate() {
            if missed[i] {
                let group = by_src.entry(src.as_str()).or_default();
                if group.is_empty() {
                    order.push(src.as_str());
                }
                group.push((i, *style));
            }
        }
        if order.is_empty() {
            return ArtDone { results: prepared, deferred: false };
        }
        let deferred = order.len() > sources;
        let groups: Vec<(&str, Vec<(usize, Style)>)> = order.into_iter().take(sources).map(|src| (src, by_src.remove(src).unwrap_or_default())).collect();
        let work = |groups: &[(&str, Vec<(usize, Style)>)]| -> Vec<Vec<(usize, Result<Prepared, String>)>> {
            groups
                .par_iter()
                .map(|(src, styles)| {
                    let wanted: Vec<Style> = styles.iter().map(|(_, s)| *s).collect();
                    let results = prepare_many(Path::new(src), &art_dir, &wanted);
                    styles.iter().zip(results).map(|((i, _), r)| (*i, r.map_err(|e| e.to_string()))).collect()
                })
                .collect()
        };
        let done = match rayon::ThreadPoolBuilder::new().num_threads(threads.max(1)).build() {
            Ok(pool) => pool.install(|| work(&groups)),
            Err(_) => work(&groups),
        };
        release_freed_memory();
        let mut rows = Vec::new();
        for (i, r) in done.into_iter().flatten() {
            let (src, style) = &self.wants[i];
            let Some(Some((len, mtime))) = signatures.get(src.as_str()).copied() else { continue };
            match r {
                Ok(p) => {
                    if let Ok(json) = serde_json::to_string(&p) {
                        rows.push((art_key(src, *style), len, mtime, json));
                    }
                    prepared[i] = Some(p);
                }
                Err(e) => {
                    tracing::warn!("{} for {src}: {e}", style.slug());
                    rows.push((art_key(src, *style), len, mtime, "null".to_string()));
                }
            }
        }
        if let Err(e) = state.lock().store.artwork_put_many(&rows) {
            tracing::warn!("remember artwork: {e}");
        }
        ArtDone { results: prepared, deferred }
    }
}

fn art_key(src: &str, style: Style) -> String {
    format!("{src}#{}", style.slug())
}

/// How many source pictures one pass may decode: a couple of seconds' work on any TV.
fn art_sources(state: &Mutex<State>) -> usize {
    6 * state.lock().art_threads.max(1)
}

/// Returns pages freed after a batch of decodes to the system; otherwise they stay resident for the
/// rest of the session.
fn release_freed_memory() {
    #[cfg(target_os = "android")]
    unsafe {
        unsafe extern "C" {
            fn mallopt(param: libc::c_int, value: libc::c_int) -> libc::c_int;
        }
        const M_PURGE: libc::c_int = -101;
        mallopt(M_PURGE, 0);
    }
}

/// Remember an ingest whose pictures are not all ready, and make sure a pass is queued.
fn defer(inner: &Inner, f: impl FnOnce(&mut PendingArt)) {
    let queue = {
        let mut state = inner.state.lock();
        f(&mut state.pending_art);
        !std::mem::replace(&mut state.pending_art.queued, true)
    };
    if queue {
        inner.enqueue_job(Job::MoreArt);
    }
}

/// Ask for the pictures of the latest programme records again: a preview channel the person just
/// switched on has had none prepared.
pub(crate) fn want_program_art(inner: &Inner, state: &mut State) {
    state.pending_art.programs = true;
    if !std::mem::replace(&mut state.pending_art.queued, true) {
        inner.enqueue_job(Job::MoreArt);
    }
}

/// The next pass over every ingest still waiting for pictures.
fn more_art(inner: &Inner) {
    let pending = {
        let mut state = inner.state.lock();
        std::mem::take(&mut state.pending_art)
    };
    if let Some(apps) = pending.apps
        && let Err(e) = ingest_apps(inner, apps)
    {
        tracing::error!("ingest apps failed: {e}");
    }
    if pending.programs {
        // Nothing has been read from the TV provider yet this time: there is nothing to prepare,
        // and an empty ingest would throw away what is stored.
        let programs = inner.state.lock().last_programs.clone();
        if !programs.is_empty()
            && let Err(e) = ingest_programs(inner, programs)
        {
            tracing::error!("ingest programs failed: {e}");
        }
    }
    for (plugin, items) in pending.plugins {
        if let Err(e) = ingest_plugin_items(inner, &plugin, items) {
            tracing::error!("ingest plugin {plugin} failed: {e}");
        }
    }
}

/// When each item of `source` was first seen, so a refresh never makes old things look new.
fn first_seen(state: &State, source: &str) -> HashMap<String, i64> {
    state.graph.items().filter(|it| it.source == source).filter_map(|it| Some((it.id.clone(), it.added_ms?))).collect()
}

fn pick<'a>(results: &'a [Option<Prepared>], idx: Option<usize>) -> Option<&'a Prepared> {
    idx.and_then(|i| results[i].as_ref())
}

fn aspect(p: Option<&Prepared>) -> f32 {
    match p {
        Some(p) if p.src_width > 0 && p.src_height > 0 => p.src_width as f32 / p.src_height as f32,
        _ => 0.0,
    }
}

// ---------------------------------------------------------------------------
// Apps
// ---------------------------------------------------------------------------

fn ingest_apps(inner: &Inner, apps: Vec<AppRecord>) -> Result<(), lounge_store::StoreError> {
    let mut plan = ArtPlan::new();
    let wants: Vec<_> = apps
        .iter()
        .map(|a| {
            (
                plan.want(a.banner_path.as_deref(), Style::AppBanner),
                plan.want(a.banner_path.as_deref(), Style::HeroBanner),
                plan.want(a.banner_path.as_deref(), Style::Backdrop),
                plan.want(a.icon_path.as_deref(), Style::Icon),
            )
        })
        .collect();
    let done = plan.run(&inner.state, art_sources(&inner.state));
    let art = done.results;

    let mut items = Vec::with_capacity(apps.len());
    for (app, (banner, hero, backdrop, icon)) in apps.iter().zip(wants) {
        let banner = pick(&art, banner);
        let icon = pick(&art, icon);
        let mut it = Item::app(&app.package, &app.activity, &app.label);
        it.last_used_ms = app.last_used_ms;
        it.added_ms = Some(app.installed_ms);
        it.flags.is_game = app.is_game;
        it.flags.has_banner = banner.is_some();
        it.art_aspect = aspect(banner);
        it.artwork = Artwork {
            landscape: banner.map(|p| p.path.clone()),
            hero: pick(&art, hero).map(|p| p.path.clone()),
            backdrop: pick(&art, backdrop).map(|p| p.path.clone()),
            logo: icon.map(|p| p.path.clone()),
            poster: None,
            palette: banner.or(icon).map(|p| p.palette),
            band: None,
            wallpaper: None,
            focal_y: 0.5,
        };
        items.push(it);
    }
    let rows = lounge_store::encode_items(&items)?;
    {
        let mut state = inner.state.lock();
        let now = now_ms();
        state.store.replace_items_encoded("apps", &rows, now)?;
        let usage = state.store.load_usage()?;
        state.graph.replace_source("apps", items);
        for (pkg, ms) in usage {
            state.graph.set_usage(&pkg, ms);
        }
    }
    if done.deferred {
        defer(inner, |p| p.apps = Some(apps));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// TV provider: Watch Next, preview programmes, channels, schedule
// ---------------------------------------------------------------------------

#[derive(Default)]
struct ProgramArt {
    poster: Option<usize>,
    landscape: Option<usize>,
    hero: Option<usize>,
    hero_fit: Option<usize>,
    backdrop: Option<usize>,
    band: Option<usize>,
    logo: Option<usize>,
    wallpaper: Option<usize>,
}

fn ingest_programs(inner: &Inner, programs: Vec<ProgramRecord>) -> Result<(), lounge_store::StoreError> {
    let now = now_ms();
    let (enabled_channels, seen) = {
        let state = inner.state.lock();
        (state.graph.layout.enabled_channels.clone(), first_seen(&state, "tvprovider"))
    };
    let mut plan = ArtPlan::new();
    let wants: Vec<ProgramArt> = programs
        .iter()
        .map(|p| {
            // Pictures only for what can be on screen: preview rows the person switched on, and
            // schedule entries airing in the next few hours.
            let shown = match p.kind {
                ProgramKind::Preview => p.channel_id.is_some_and(|c| enabled_channels.contains(&c)),
                ProgramKind::Schedule => p.end_ms.is_none_or(|e| e > now) && p.start_ms.is_none_or(|s| s < now + SCHEDULE_ART_AHEAD_MS),
                ProgramKind::WatchNext | ProgramKind::Channel => true,
            };
            if !shown {
                return ProgramArt::default();
            }
            // A 16:9 thumbnail is the landscape source when there is one; the poster is used
            // in its own orientation and as the Stage's framed art when it is portrait.
            let wide = p.thumbnail_path.as_deref().or(if p.poster_is_portrait { None } else { p.poster_path.as_deref() });
            let portrait = if p.poster_is_portrait { p.poster_path.as_deref() } else { None };
            let schedule = p.kind == ProgramKind::Schedule;
            ProgramArt {
                poster: if schedule { None } else { plan.want(portrait, Style::Poster) },
                landscape: plan.want(wide.or(p.poster_path.as_deref()), Style::Landscape),
                hero: plan.want(wide, Style::Hero),
                hero_fit: plan.want(if wide.is_none() { portrait } else { None }, Style::HeroFit),
                backdrop: plan.want(wide.or(portrait), Style::Backdrop),
                band: plan.want(wide, Style::Band),
                logo: if schedule { None } else { plan.want(p.logo_path.as_deref(), Style::Logo) },
                wallpaper: None,
            }
        })
        .collect();
    let done = plan.run(&inner.state, art_sources(&inner.state));
    let art = done.results;

    let mut items = Vec::with_capacity(programs.len());
    for (p, w) in programs.iter().zip(wants) {
        let poster = pick(&art, w.poster);
        let landscape = pick(&art, w.landscape);
        let hero = pick(&art, w.hero).or(pick(&art, w.hero_fit));
        let logo = pick(&art, w.logo);
        let band = pick(&art, w.band);
        let artwork = Artwork {
            poster: poster.map(|x| x.path.clone()),
            landscape: landscape.map(|x| x.path.clone()),
            hero: hero.map(|x| x.path.clone()),
            backdrop: pick(&art, w.backdrop).map(|x| x.path.clone()),
            logo: logo.map(|x| x.path.clone()),
            palette: landscape.or(poster).or(logo).map(|x| x.palette),
            band: band.map(|x| x.path.clone()),
            wallpaper: None,
            focal_y: band.or(hero).map(|x| x.focal_y).unwrap_or(0.5),
        };
        let art_aspect = aspect(hero.or(landscape).or(poster));
        let deep_link = match p.kind {
            ProgramKind::Channel | ProgramKind::Schedule => DeepLink::Tune(p.intent_uri.clone().unwrap_or_default()),
            _ => match &p.intent_uri {
                Some(uri) if !uri.is_empty() && !p.package.is_empty() => DeepLink::ViewOrLaunch { uri: uri.clone(), package: p.package.clone() },
                Some(uri) if !uri.is_empty() => DeepLink::ViewUri(uri.clone()),
                _ => DeepLink::LaunchApp {
                    package: p.package.clone(),
                    activity: String::new(),
                },
            },
        };
        let kind = match p.kind {
            ProgramKind::WatchNext => {
                if p.watch_next_type == WatchNextType::Continue || p.position_ms.is_some() {
                    ItemKind::Resume
                } else {
                    ItemKind::Program
                }
            }
            ProgramKind::Preview | ProgramKind::Schedule => ItemKind::Program,
            ProgramKind::Channel => ItemKind::Channel,
        };
        let mut it = Item::content(&p.id, kind, "tvprovider", &p.title, deep_link);
        it.owner = Some(p.package.clone());
        it.artwork = artwork;
        it.art_aspect = art_aspect;
        it.season = p.season.clone();
        it.episode = p.episode.clone();
        it.duration_ms = p.duration_ms;
        it.channel_id = p.channel_id;
        it.channel_name = p.channel_name.clone();
        it.weight = p.weight;
        it.flags.is_new = p.watch_next_type == WatchNextType::New;
        match p.kind {
            ProgramKind::WatchNext => {
                it.subtitle = p.subtitle.clone();
                it.flags.watch_next = true;
                it.progress = progress(p.position_ms, p.duration_ms);
                // Without an engagement time there is no telling when it was watched, so it
                // sorts after everything that says.
                it.last_used_ms = p.engaged_ms.filter(|&ms| ms > 0);
                it.added_ms = it.last_used_ms;
            }
            ProgramKind::Preview => {
                it.subtitle = p.subtitle.clone();
                it.progress = progress(p.position_ms, p.duration_ms);
                it.added_ms = Some(seen.get(&p.id).copied().unwrap_or(now));
            }
            ProgramKind::Channel => {
                it.subtitle = p.channel_number.clone();
                it.flags.is_live = true;
            }
            ProgramKind::Schedule => {
                it.subtitle = p.subtitle.clone();
                it.airing_start_ms = p.start_ms;
                it.airing_end_ms = p.end_ms;
            }
        }
        items.push(it);
    }
    // Schedule rows are regenerated seconds after launch and go stale within hours, so the graph
    // holds them and the database does not. Persisted items are encoded here, before the lock.
    let persisted: Vec<Item> = items.iter().filter(|it| !(it.kind == ItemKind::Program && it.airing_start_ms.is_some())).cloned().collect();
    let rows = lounge_store::encode_items(&persisted)?;
    {
        let mut state = inner.state.lock();
        state.store.replace_items_encoded("tvprovider", &rows, now)?;
        state.graph.replace_source("tvprovider", items);
        state.last_programs = programs;
    }
    if done.deferred {
        defer(inner, |pending| pending.programs = true);
    }
    Ok(())
}

fn progress(position_ms: Option<i64>, duration_ms: Option<i64>) -> Option<f32> {
    match (position_ms, duration_ms) {
        (Some(p), Some(d)) if d > 0 && p > 0 => Some((p as f32 / d as f32).clamp(0.0, 1.0)),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Plugins: items a plugin published, artwork already downloaded by the plugin thread
// ---------------------------------------------------------------------------

fn ingest_plugin_items(inner: &Inner, plugin: &str, ready: Vec<ReadyItem>) -> Result<(), lounge_store::StoreError> {
    let source = format!("plugin:{plugin}");
    let enabled = |state: &State| state.plugins.iter().any(|p| p.id == plugin && p.enabled);
    // A refresh that finishes after the plugin was turned off or removed is dropped.
    let seen = {
        let state = inner.state.lock();
        if !enabled(&state) {
            return Ok(());
        }
        first_seen(&state, &source)
    };
    let mut ids = HashSet::new();
    let ready: Vec<ReadyItem> = ready.into_iter().filter(|r| ids.insert(r.item.id.clone())).collect();
    let mut plan = ArtPlan::new();
    let wants: Vec<ProgramArt> = ready
        .iter()
        .map(|r| {
            // `poster` is portrait art, `thumbnail` is 16:9; whichever exists feeds the other size.
            let wide = r.thumbnail.as_deref();
            let portrait = r.poster.as_deref();
            if r.item.kind == PluginItemKind::Wallpaper {
                return ProgramArt { wallpaper: plan.want(wide.or(portrait), Style::Wallpaper), ..Default::default() };
            }
            ProgramArt {
                poster: plan.want(portrait, Style::Poster),
                landscape: plan.want(wide.or(portrait), Style::Landscape),
                hero: plan.want(wide, Style::Hero),
                hero_fit: plan.want(if wide.is_none() { portrait } else { None }, Style::HeroFit),
                backdrop: plan.want(wide.or(portrait), Style::Backdrop),
                band: plan.want(wide, Style::Band),
                logo: plan.want(r.logo.as_deref(), Style::Logo),
                wallpaper: None,
            }
        })
        .collect();
    let done = plan.run(&inner.state, art_sources(&inner.state));
    let art = done.results;
    let now = now_ms();

    let mut items = Vec::with_capacity(ready.len());
    for (r, w) in ready.iter().zip(wants) {
        let p = &r.item;
        if p.title.trim().is_empty() || p.id.trim().is_empty() {
            continue;
        }
        let poster = pick(&art, w.poster);
        let landscape = pick(&art, w.landscape);
        let hero = pick(&art, w.hero).or(pick(&art, w.hero_fit));
        let logo = pick(&art, w.logo);
        let Some(deep_link) = plugin_link(plugin, p) else { continue };
        let kind = match p.kind {
            PluginItemKind::Resume => ItemKind::Resume,
            PluginItemKind::Channel => ItemKind::Channel,
            PluginItemKind::Wallpaper => ItemKind::Wallpaper,
            PluginItemKind::Program | PluginItemKind::Schedule => ItemKind::Program,
            PluginItemKind::Screensaver => continue,
        };
        let id = format!("plugin:{plugin}:{}", p.id);
        let a = &p.action;
        let mut it = Item::content(&id, kind, &source, &p.title, deep_link);
        it.owner = a.fallback_launch.clone().or_else(|| a.launch.clone());
        it.subtitle = p.subtitle.clone();
        it.season = p.season.clone();
        it.episode = p.episode.clone();
        it.duration_ms = p.duration_ms;
        it.progress = p.progress.or_else(|| progress(p.position_ms, p.duration_ms));
        it.last_used_ms = p.last_used_ms;
        it.added_ms = p.added_ms.or_else(|| seen.get(&id).copied()).or(Some(now));
        it.weight = p.weight;
        it.row = p.row.clone();
        it.flags.is_new = p.is_new;
        it.channel_id = p.channel_id.as_deref().map(|c| channel_key(plugin, c));
        it.channel_name = p.channel_name.clone();
        it.art_aspect = aspect(hero.or(landscape).or(poster));
        let band = pick(&art, w.band);
        it.artwork = Artwork {
            poster: poster.map(|x| x.path.clone()),
            landscape: landscape.map(|x| x.path.clone()),
            hero: hero.map(|x| x.path.clone()),
            backdrop: pick(&art, w.backdrop).map(|x| x.path.clone()),
            logo: logo.map(|x| x.path.clone()),
            palette: landscape.or(poster).or(logo).map(|x| x.palette),
            band: band.map(|x| x.path.clone()),
            wallpaper: pick(&art, w.wallpaper).map(|x| x.path.clone()),
            focal_y: band.or(hero).map(|x| x.focal_y).unwrap_or(0.5),
        };
        match p.kind {
            PluginItemKind::Resume => {
                it.flags.watch_next = true;
                if it.last_used_ms.is_none() {
                    it.last_used_ms = Some(now);
                }
            }
            PluginItemKind::Channel => {
                it.subtitle = p.channel_number.clone();
                it.flags.is_live = true;
            }
            PluginItemKind::Schedule => {
                it.airing_start_ms = p.start_ms;
                it.airing_end_ms = p.end_ms;
            }
            PluginItemKind::Program | PluginItemKind::Wallpaper | PluginItemKind::Screensaver => {}
        }
        items.push(it);
    }
    let rows = lounge_store::encode_items(&items)?;
    {
        let mut state = inner.state.lock();
        if !enabled(&state) {
            return Ok(());
        }
        state.store.replace_items_encoded(&source, &rows, now)?;
        state.graph.replace_source(&source, items);
    }
    if done.deferred {
        let plugin = plugin.to_string();
        defer(inner, move |pending| {
            pending.plugins.insert(plugin, ready);
        });
    }
    Ok(())
}

/// What selecting a plugin item does. Wallpapers do nothing; a schedule entry without an action
/// plays through its channel; anything else needs an action of its own.
pub(crate) fn plugin_link(plugin: &str, p: &lounge_plugin_api::Item) -> Option<DeepLink> {
    let a = &p.action;
    Some(if let Some(url) = &a.play {
        DeepLink::Play { url: url.clone(), title: a.title.clone().unwrap_or_else(|| p.title.clone()) }
    } else if let (Some(uri), Some(pkg)) = (&a.view, &a.fallback_launch) {
        DeepLink::ViewOrLaunch { uri: uri.clone(), package: pkg.clone() }
    } else if let Some(uri) = &a.view {
        DeepLink::ViewUri(uri.clone())
    } else if let Some(pkg) = &a.launch {
        DeepLink::LaunchApp { package: pkg.clone(), activity: String::new() }
    } else if let Some(action) = &a.resolve {
        DeepLink::PluginResolve { plugin: plugin.to_string(), action: action.clone() }
    } else if p.kind == PluginItemKind::Wallpaper || (p.kind == PluginItemKind::Schedule && p.channel_id.is_some()) {
        DeepLink::None
    } else {
        return None;
    })
}

// ---------------------------------------------------------------------------
// TV inputs: connected and standby ports become Input items; unplugged ports are not shown
// ---------------------------------------------------------------------------

fn ingest_inputs(inner: &Inner, inputs: Vec<InputRecord>) -> Result<(), lounge_store::StoreError> {
    let now = now_ms();
    let mut items: Vec<Item> = inputs
        .iter()
        .filter(|i| i.state != InputState::Disconnected)
        .map(|i| {
            let name = i.custom_label.clone().filter(|c| !c.trim().is_empty()).unwrap_or_else(|| i.label.clone());
            let mut it = Item::content(&format!("input:{}", i.id), ItemKind::Input, "inputs", &name, DeepLink::SwitchInput(i.id.clone()));
            it.owner = Some(i.package.clone());
            let kind_label = match i.kind.as_str() {
                "hdmi" => "HDMI",
                "tuner" => "Live TV",
                "av" => "AV",
                _ => "Input",
            };
            let subtitle = match (&i.custom_label, i.state) {
                (Some(c), InputState::Standby) if c != &i.label => format!("{}, standby", i.label),
                (Some(c), _) if c != &i.label => i.label.clone(),
                (_, InputState::Standby) => "Standby".to_string(),
                _ => kind_label.to_string(),
            };
            // "Live TV · Live TV" says nothing twice.
            it.subtitle = Some(subtitle).filter(|s| s != &name);
            it.flags.connected = i.state == InputState::Connected;
            // Connected ports first, then by port number, so the row reads like the back of the TV.
            it.weight = (if i.state == InputState::Connected { 1000 } else { 0 }) - i.hdmi_port.unwrap_or(99);
            it
        })
        .collect();
    items.sort_by(|a, b| b.weight.cmp(&a.weight).then_with(|| a.title.cmp(&b.title)));
    let mut state = inner.state.lock();
    state.store.replace_items("inputs", &items, now)?;
    state.graph.replace_source("inputs", items);
    Ok(())
}

// ---------------------------------------------------------------------------
// Media sessions: now playing, and observed playback as resume items
// ---------------------------------------------------------------------------

fn ingest_sessions(inner: &Inner, sessions: Vec<SessionRecord>) -> Result<(), lounge_store::StoreError> {
    let titled: Vec<&SessionRecord> = sessions.iter().filter(|s| s.title.as_deref().is_some_and(|t| !t.trim().is_empty())).collect();
    let mut plan = ArtPlan::new();
    let wants: Vec<_> = titled
        .iter()
        .map(|s| {
            (
                plan.want(s.art_path.as_deref(), Style::Landscape),
                plan.want(s.art_path.as_deref(), Style::Hero),
                plan.want(s.art_path.as_deref(), Style::HeroFit),
                plan.want(s.art_path.as_deref(), Style::Backdrop),
                plan.want(s.art_path.as_deref(), Style::Band),
            )
        })
        .collect();
    let art = plan.run(&inner.state, art_sources(&inner.state)).results;
    let now = now_ms();

    let mut items = Vec::new();
    // Only something playing counts as using its app; a session paused yesterday does not.
    let mut playing: Vec<(String, i64)> = Vec::new();
    let mut now_playing: Option<(NowPlaying, (bool, i64))> = None;
    for (s, (landscape, hero, hero_fit, backdrop, band)) in titled.iter().zip(wants) {
        let title = s.title.clone().unwrap_or_default();
        let landscape = pick(&art, landscape);
        let art_aspect = aspect(landscape);
        // Wide art fills the Stage; square or portrait art (album covers) is framed.
        let hero = if art_aspect >= 1.2 { pick(&art, hero) } else { pick(&art, hero_fit) };
        let light = landscape
            .map(|p| lounge_graph::light_from_palette(&p.palette))
            .unwrap_or(lounge_graph::color::HOUSELIGHT);

        let mut it = Item::content(
            &format!("session:{}:{}", s.package, slug(&title)),
            ItemKind::MediaSession,
            "sessions",
            &title,
            DeepLink::LaunchApp {
                package: s.package.clone(),
                activity: String::new(),
            },
        );
        it.owner = Some(s.package.clone());
        it.subtitle = s.subtitle.clone().or_else(|| Some(s.app_label.clone()));
        it.progress = progress(s.position_ms, s.duration_ms);
        it.duration_ms = s.duration_ms;
        it.last_used_ms = Some(s.updated_ms);
        it.added_ms = Some(s.updated_ms);
        if s.status == PlaybackStatus::Playing {
            playing.push((s.package.clone(), s.updated_ms.min(now)));
        }
        it.art_aspect = art_aspect;
        let band = if art_aspect >= 1.2 { pick(&art, band) } else { None };
        it.artwork = Artwork {
            landscape: landscape.map(|p| p.path.clone()),
            hero: hero.map(|p| p.path.clone()),
            backdrop: pick(&art, backdrop).map(|p| p.path.clone()),
            logo: None,
            poster: None,
            palette: landscape.map(|p| p.palette),
            band: band.map(|p| p.path.clone()),
            wallpaper: None,
            focal_y: band.or(hero).map(|p| p.focal_y).unwrap_or(0.5),
        };
        items.push(it);

        // The chip shows what is playing; failing that, the most recently touched session.
        let rank = (s.status == PlaybackStatus::Playing, s.updated_ms);
        if s.status != PlaybackStatus::Stopped && now_playing.as_ref().is_none_or(|(_, r)| rank > *r) {
            now_playing = Some((
                NowPlaying {
                    package: s.package.clone(),
                    app_label: s.app_label.clone(),
                    title,
                    subtitle: s.subtitle.clone(),
                    art_path: landscape.map(|p| p.path.clone()),
                    light_argb: light,
                    duration_ms: s.duration_ms,
                    position_ms: s.position_ms,
                    status: s.status,
                    can_pause: s.can_pause,
                    can_play: s.can_play,
                    can_skip_next: s.can_skip_next,
                    can_skip_previous: s.can_skip_previous,
                    updated_ms: s.updated_ms,
                },
                rank,
            ));
        }
    }

    let offset = inner.utc_offset_minutes.load(Ordering::Relaxed);
    let mut state = inner.state.lock();
    if !items.is_empty() {
        state.store.upsert_items(&items, now)?;
        record_usage(&mut state, &playing, offset)?;
        for it in items {
            state.graph.upsert(it);
        }
        for id in state.store.prune_source("sessions", SESSION_HISTORY)? {
            state.graph.remove(&id);
        }
    }
    state.now_playing = now_playing.map(|(np, _)| np);
    Ok(())
}

fn slug(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars().flat_map(char::to_lowercase) {
        if c.is_alphanumeric() {
            out.push(c);
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    out.trim_matches('-').chars().take(64).collect()
}

// ---------------------------------------------------------------------------

fn apply_usage(inner: &Inner, usage: Vec<AppUsage>) -> Result<(), lounge_store::StoreError> {
    let pairs: Vec<(String, i64)> = usage.into_iter().map(|u| (u.package, u.last_used_ms)).collect();
    let offset = inner.utc_offset_minutes.load(Ordering::Relaxed);
    let mut state = inner.state.lock();
    record_usage(&mut state, &pairs, offset)
}

/// Remember when apps were used, in the store, the graph and the habits behind explanations.
fn record_usage(state: &mut State, usage: &[(String, i64)], utc_offset_minutes: i32) -> Result<(), lounge_store::StoreError> {
    if usage.is_empty() {
        return Ok(());
    }
    let counted = state.store.set_usage(usage, |ms| engine::local_hour(ms, utc_offset_minutes))?;
    for (pkg, ms) in usage {
        state.graph.set_usage(pkg, *ms);
    }
    for (pkg, hour) in counted {
        state.habits.entry(pkg).or_insert([0; 24])[hour as usize % 24] += 1;
    }
    Ok(())
}

fn apply_command(inner: &Inner, cmd: LayoutCommand) -> Result<(), lounge_store::StoreError> {
    let mut state = inner.state.lock();
    match cmd {
        LayoutCommand::HideItem { id } => state.graph.hide(&id),
        LayoutCommand::UnhideItem { id } => state.graph.unhide(&id),
        LayoutCommand::PinApp { package } => state.graph.pin_app(&package),
        LayoutCommand::UnpinApp { package } => state.graph.unpin_app(&package),
        LayoutCommand::MoveApp { package, to_index } => state.graph.move_app(&package, to_index as usize),
        LayoutCommand::PinToStage { id } => {
            if !state.graph.layout.stage_pins.contains(&id) {
                state.graph.layout.stage_pins.push(id);
            }
        }
        LayoutCommand::UnpinFromStage { id } => state.graph.layout.stage_pins.retain(|p| p != &id),
        LayoutCommand::SetRowOrder { row_ids } => state.graph.set_row_order(row_ids),
        LayoutCommand::ResetLayout => state.graph.reset_layout(),
        LayoutCommand::DismissNotice { id } => {
            state.graph.layout.dismissed_notices.insert(id);
        }
        LayoutCommand::EnableChannel { channel_id } => {
            state.graph.layout.enabled_channels.insert(channel_id);
            // Its programmes had no pictures while the row was off; a pass prepares them now.
            want_program_art(inner, &mut state);
        }
        LayoutCommand::DisableChannel { channel_id } => {
            state.graph.layout.enabled_channels.remove(&channel_id);
        }
    }
    state.graph.layout.normalise();
    let layout = state.graph.layout.clone();
    state.store.save_layout(&layout)?;
    Ok(())
}

/// Build and publish the snapshot. Returns when what is airing next changes, if anything does.
fn rebuild(inner: &Inner, utc_offset_minutes: i32) -> Option<i64> {
    let started = std::time::Instant::now();
    let (snapshot, next_change) = {
        let mut state = inner.state.lock();
        state.revision += 1;
        let snapshot = engine::build(&state, &inner.config, utc_offset_minutes);
        (snapshot, state.graph.next_airing_change(now_ms()))
    };
    inner
        .last_build_micros
        .store(started.elapsed().as_micros() as u64, Ordering::Relaxed);
    // A rebuild that changed nothing the screen shows costs nothing further: no persist, no
    // listener, no recomposition. Revision and time stamp are not part of "what the screen shows".
    let unchanged = {
        let prev = inner.snapshot.read();
        prev.revision > 0
            && prev.stage == snapshot.stage
            && prev.rows == snapshot.rows
            && prev.now_playing == snapshot.now_playing
            && prev.wallpapers == snapshot.wallpapers
    };
    if unchanged {
        return next_change;
    }
    match serde_json::to_string(&snapshot) {
        Ok(json) => {
            if let Err(e) = inner.state.lock().store.save_snapshot(snapshot.revision, &json) {
                tracing::warn!("persist snapshot: {e}");
            }
            *inner.snapshot_json.lock() = json;
        }
        Err(e) => tracing::warn!("serialise snapshot: {e}"),
    }
    *inner.snapshot.write() = snapshot.clone();
    let listeners = inner.listeners.lock().clone();
    for l in listeners {
        l.on_snapshot(snapshot.clone());
    }
    if let Some(c) = inner.companion.lock().as_ref() {
        c.notify();
    }
    next_change
}

/// Delete prepared pictures no item uses and that have not been written for a day, forget the
/// store's memory of them, and drop art folders of plugins that are gone.
fn housekeeping(inner: &Inner) {
    let started = Instant::now();
    let (art_dir, used, installed) = {
        let state = inner.state.lock();
        let used: HashSet<PathBuf> = state.graph.items().flat_map(|it| artwork_paths(&it.artwork)).map(PathBuf::from).collect();
        let installed: Vec<String> = state.plugins.iter().map(|p| p.id.clone()).collect();
        (state.art_dir.clone(), used, installed)
    };
    let cutoff = std::time::SystemTime::now() - ART_KEEP;
    let mut removed = 0usize;
    if let Ok(entries) = std::fs::read_dir(&art_dir) {
        for e in entries.flatten() {
            let path = e.path();
            if used.contains(&path) {
                continue;
            }
            let old = e.metadata().and_then(|m| m.modified()).is_ok_and(|t| t < cutoff);
            if old && std::fs::remove_file(&path).is_ok() {
                removed += 1;
            }
        }
    }
    let rows = inner.state.lock().store.artwork_rows().unwrap_or_default();
    let forget: Vec<String> = rows
        .into_iter()
        .filter(|(key, json)| {
            let source_gone = key.rsplit_once('#').is_none_or(|(src, _)| !Path::new(src).exists());
            let output_gone = serde_json::from_str::<Prepared>(json).is_ok_and(|p| !Path::new(&p.path).exists());
            source_gone || output_gone
        })
        .map(|(key, _)| key)
        .collect();
    if !forget.is_empty()
        && let Err(e) = inner.state.lock().store.artwork_forget(&forget)
    {
        tracing::warn!("forget artwork: {e}");
    }
    inner.plugins.prune_orphan_art(&installed);
    tracing::info!("housekeeping: {removed} pictures and {} records removed in {} ms", forget.len(), started.elapsed().as_millis());
}

fn artwork_paths(a: &Artwork) -> impl Iterator<Item = &String> {
    [&a.poster, &a.landscape, &a.hero, &a.backdrop, &a.logo, &a.band, &a.wallpaper].into_iter().flatten()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slug_is_stable_and_safe() {
        assert_eq!(slug("The Bear: S2 E4 — \"Honeydew\""), "the-bear-s2-e4-honeydew");
        assert_eq!(slug("   "), "");
    }

    #[test]
    fn progress_needs_both_numbers() {
        assert_eq!(progress(Some(500), Some(1000)), Some(0.5));
        assert_eq!(progress(Some(500), None), None);
        assert_eq!(progress(Some(5000), Some(1000)), Some(1.0));
    }
}
