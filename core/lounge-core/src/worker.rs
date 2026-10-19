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
use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::Ordering;
use std::sync::Arc;

const SESSION_HISTORY: usize = 20;

pub(crate) enum Job {
    /// First job after open: read the stored items and usage into the graph.
    LoadGraph,
    IngestApps(Vec<AppRecord>),
    IngestPrograms(Vec<ProgramRecord>),
    IngestInputs(Vec<InputRecord>),
    IngestSessions(Vec<SessionRecord>),
    /// A plugin's refresh finished; its artwork is already on disk.
    PluginItems { plugin: String, items: Vec<ReadyItem> },
    Usage(Vec<AppUsage>),
    Command(LayoutCommand),
    Permissions(Permissions),
    UtcOffset(i32),
    Rebuild,
}

pub(crate) fn spawn(inner: Arc<Inner>, rx: crossbeam_channel::Receiver<Job>) {
    std::thread::Builder::new()
        .name("lounge-core".into())
        .spawn(move || {
            let mut utc_offset = inner.config.utc_offset_minutes;
            while let Ok(job) = rx.recv() {
                inner.queue_depth.fetch_sub(1, Ordering::Relaxed);
                let mut pending = vec![job];
                while let Ok(next) = rx.try_recv() {
                    inner.queue_depth.fetch_sub(1, Ordering::Relaxed);
                    pending.push(next);
                }
                let mut dirty = false;
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
                        Job::UtcOffset(m) => {
                            utc_offset = m;
                            dirty = true;
                        }
                        Job::Rebuild => dirty = true,
                    }
                }
                if dirty {
                    rebuild(&inner, utc_offset);
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

struct ArtPlan {
    wants: Vec<(String, Style)>,
    index: HashMap<(String, Style), usize>,
}

impl ArtPlan {
    fn new() -> ArtPlan {
        ArtPlan {
            wants: Vec::new(),
            index: HashMap::new(),
        }
    }

    fn want(&mut self, src: Option<&str>, style: Style) -> Option<usize> {
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

    fn run(self, state: &Mutex<State>) -> Vec<Option<Prepared>> {
        let (art_dir, cached, threads) = {
            let st = state.lock();
            let cached: Vec<Option<Prepared>> = self
                .wants
                .iter()
                .map(|(src, style)| cached(&st.store, src, *style))
                .collect();
            (st.art_dir.clone(), cached, st.art_threads)
        };
        let misses = cached.iter().filter(|c| c.is_none()).count();
        if misses == 0 {
            return cached;
        }
        // Every source that missed is decoded once and all its styles are cut from that decode.
        let mut by_src: HashMap<&str, Vec<(usize, Style)>> = HashMap::new();
        for (i, ((src, style), hit)) in self.wants.iter().zip(cached.iter()).enumerate() {
            if hit.is_none() {
                by_src.entry(src.as_str()).or_default().push((i, *style));
            }
        }
        let groups: Vec<(&str, Vec<(usize, Style)>)> = by_src.into_iter().collect();
        let work = |groups: &[(&str, Vec<(usize, Style)>)]| -> Vec<Vec<(usize, Option<Prepared>)>> {
            groups
                .par_iter()
                .map(|(src, styles)| {
                    let wanted: Vec<Style> = styles.iter().map(|(_, s)| *s).collect();
                    let results = prepare_many(Path::new(src), &art_dir, &wanted);
                    styles
                        .iter()
                        .zip(results)
                        .map(|((i, style), r)| (*i, r.map_err(|e| tracing::warn!("{} for {src}: {e}", style.slug())).ok()))
                        .collect()
                })
                .collect()
        };
        let done = match rayon::ThreadPoolBuilder::new().num_threads(threads.max(1)).build() {
            Ok(pool) => pool.install(|| work(&groups)),
            Err(_) => work(&groups),
        };
        release_freed_memory();
        let mut prepared = cached;
        let mut rows = Vec::new();
        for (i, p) in done.into_iter().flatten() {
            if let Some(p) = p {
                let (src, style) = &self.wants[i];
                if let Ok((len, mtime)) = lounge_artwork::file_signature(Path::new(src)) {
                    if let Ok(json) = serde_json::to_string(&p) {
                        rows.push((format!("{src}#{}", style.slug()), len, mtime, json));
                    }
                }
                prepared[i] = Some(p);
            }
        }
        if let Err(e) = state.lock().store.artwork_put_many(&rows) {
            tracing::warn!("remember artwork: {e}");
        }
        prepared
    }
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

fn cached(store: &lounge_store::Store, src: &str, style: Style) -> Option<Prepared> {
    let (len, mtime) = lounge_artwork::file_signature(Path::new(src)).ok()?;
    let prepared: Option<Prepared> = store
        .artwork_cached(&format!("{src}#{}", style.slug()), len, mtime)
        .ok()
        .flatten();
    prepared.filter(|p| Path::new(&p.path).exists())
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
    let art = plan.run(&inner.state);

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
    let mut state = inner.state.lock();
    let now = now_ms();
    state.store.replace_items("apps", &items, now)?;
    let usage = state.store.load_usage()?;
    state.graph.replace_source("apps", items);
    for (pkg, ms) in usage {
        state.graph.set_usage(&pkg, ms);
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
    let mut plan = ArtPlan::new();
    let wants: Vec<ProgramArt> = programs
        .iter()
        .map(|p| {
            // A 16:9 thumbnail is the landscape source when there is one; the poster is used
            // in its own orientation and as the Stage's framed art when it is portrait.
            let wide = p.thumbnail_path.as_deref().or(if p.poster_is_portrait { None } else { p.poster_path.as_deref() });
            let portrait = if p.poster_is_portrait { p.poster_path.as_deref() } else { None };
            ProgramArt {
                poster: plan.want(portrait, Style::Poster),
                landscape: plan.want(wide.or(p.poster_path.as_deref()), Style::Landscape),
                hero: plan.want(wide, Style::Hero),
                hero_fit: plan.want(if wide.is_none() { portrait } else { None }, Style::HeroFit),
                backdrop: plan.want(wide.or(portrait), Style::Backdrop),
                band: plan.want(wide, Style::Band),
                logo: plan.want(p.logo_path.as_deref(), Style::Logo),
                wallpaper: None,
            }
        })
        .collect();
    let art = plan.run(&inner.state);
    let now = now_ms();

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
                it.last_used_ms = p.engaged_ms.or(Some(now));
                it.added_ms = p.engaged_ms;
            }
            ProgramKind::Preview => {
                it.subtitle = p.subtitle.clone();
                it.progress = progress(p.position_ms, p.duration_ms);
                it.added_ms = Some(now);
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
    let mut state = inner.state.lock();
    state.store.replace_items_encoded("tvprovider", &rows, now)?;
    state.graph.replace_source("tvprovider", items);
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
    let art = plan.run(&inner.state);
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
        let a = &p.action;
        let deep_link = if let Some(url) = &a.play {
            DeepLink::Play { url: url.clone(), title: a.title.clone().unwrap_or_else(|| p.title.clone()) }
        } else if let (Some(uri), Some(pkg)) = (&a.view, &a.fallback_launch) {
            DeepLink::ViewOrLaunch { uri: uri.clone(), package: pkg.clone() }
        } else if let Some(uri) = &a.view {
            DeepLink::ViewUri(uri.clone())
        } else if let Some(pkg) = &a.launch {
            DeepLink::LaunchApp { package: pkg.clone(), activity: String::new() }
        } else if let Some(action) = &a.resolve {
            DeepLink::PluginResolve { plugin: plugin.to_string(), action: action.clone() }
        } else if p.kind == PluginItemKind::Wallpaper {
            DeepLink::None
        } else {
            continue;
        };
        let kind = match p.kind {
            PluginItemKind::Resume => ItemKind::Resume,
            PluginItemKind::Channel => ItemKind::Channel,
            PluginItemKind::Wallpaper => ItemKind::Wallpaper,
            PluginItemKind::Program | PluginItemKind::Schedule => ItemKind::Program,
        };
        let mut it = Item::content(&format!("plugin:{plugin}:{}", p.id), kind, &source, &p.title, deep_link);
        it.owner = a.fallback_launch.clone().or_else(|| a.launch.clone());
        it.subtitle = p.subtitle.clone();
        it.season = p.season.clone();
        it.episode = p.episode.clone();
        it.duration_ms = p.duration_ms;
        it.progress = p.progress.or_else(|| progress(p.position_ms, p.duration_ms));
        it.last_used_ms = p.last_used_ms;
        it.added_ms = p.added_ms.or(Some(now));
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
            PluginItemKind::Program | PluginItemKind::Wallpaper => {}
        }
        items.push(it);
    }
    let mut state = inner.state.lock();
    state.store.replace_items(&source, &items, now)?;
    state.graph.replace_source(&source, items);
    Ok(())
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
                (Some(c), InputState::Standby) if c != &i.label => format!("{} · Standby", i.label),
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
    let art = plan.run(&inner.state);
    let now = now_ms();

    let mut items = Vec::new();
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

    let mut state = inner.state.lock();
    if !items.is_empty() {
        state.store.upsert_items(&items, now)?;
        let usage: Vec<(String, i64)> = items.iter().filter_map(|it| Some((it.owner.clone()?, it.last_used_ms?))).collect();
        state.store.set_usage(&usage)?;
        for (pkg, ms) in &usage {
            state.graph.set_usage(pkg, *ms);
        }
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
    let mut state = inner.state.lock();
    state.store.set_usage(&pairs)?;
    for (pkg, ms) in &pairs {
        state.graph.set_usage(pkg, *ms);
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
        }
        LayoutCommand::DisableChannel { channel_id } => {
            state.graph.layout.enabled_channels.remove(&channel_id);
        }
    }
    let layout = state.graph.layout.clone();
    state.store.save_layout(&layout)?;
    Ok(())
}

fn rebuild(inner: &Inner, utc_offset_minutes: i32) {
    let started = std::time::Instant::now();
    let snapshot = {
        let mut state = inner.state.lock();
        state.revision += 1;
        engine::build(&state, &inner.config, utc_offset_minutes)
    };
    inner
        .last_build_micros
        .store(started.elapsed().as_micros() as u64, Ordering::Relaxed);
    // A rebuild that changed nothing the screen shows costs nothing further: no persist, no
    // listener, no recomposition. Revision and time stamp are not part of "what the screen shows".
    let unchanged = {
        let prev = inner.snapshot.read();
        prev.revision > 0 && prev.stage == snapshot.stage && prev.rows == snapshot.rows && prev.now_playing == snapshot.now_playing
    };
    if unchanged {
        return;
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
