//! Turns the graph into a `HomeSnapshot` and resolves actions back into platform calls.

use crate::{
    now_ms, Badge, CoreConfig, CoreError, HomeSnapshot, Inner, Row, RowSize, Stage, StageAction, StageSpotlight, State,
    Tile, TileKind,
};
use lounge_graph::{DeepLink, Graph, Item, ItemKind};
use std::collections::HashMap;

/// Lookups a build needs more than once, computed once per build.
pub(crate) struct BuildIndex<'a> {
    airing: HashMap<i64, &'a Item>,
    previews: Vec<(i64, String, Vec<&'a Item>)>,
    plugin_names: HashMap<&'a str, &'a str>,
}

impl<'a> BuildIndex<'a> {
    fn new(state: &'a State, now: i64) -> BuildIndex<'a> {
        BuildIndex {
            airing: state.graph.airing_index(now),
            previews: state.graph.preview_channels(),
            plugin_names: state.plugins.iter().map(|p| (p.id.as_str(), p.manifest.name.as_str())).collect(),
        }
    }

    fn airing(&self, ch: &Item) -> Option<&'a Item> {
        ch.channel_id.and_then(|id| self.airing.get(&id).copied())
    }

    /// The plugin's name as its manifest gives it, or its id when it is not installed.
    fn plugin_name(&self, id: &str) -> String {
        self.plugin_names.get(id).map(|n| n.to_string()).unwrap_or_else(|| id.to_string())
    }
}

const STAGE_CAP: usize = 6;
const NOTICE_TV_LISTINGS: &str = "notice:tv_listings";

/// The core rows and their titles, in default order.
pub(crate) const CORE_ROWS: [(&str, &str); 6] = [
    ("continue", "Continue watching"),
    ("apps", "Your apps"),
    ("games", "Games"),
    ("live", "Live now"),
    ("tvcontrols", "TV controls"),
    ("allapps", "All apps"),
];

/// The row order as it will be built: the user's order (or the default), with plugin and
/// channel rows the user has not placed inserted before All apps.
pub(crate) fn effective_row_order(state: &State) -> Vec<String> {
    let graph = &state.graph;
    let mut order = graph.layout.row_order_or_default();
    let mut extra: Vec<String> = Vec::new();
    for p in state.plugins.iter().filter(|p| p.enabled) {
        for r in &p.manifest.rows {
            extra.push(format!("plugin:{}:{}", p.id, r.id));
        }
    }
    for (id, _, _) in graph.preview_channels() {
        if graph.layout.enabled_channels.contains(&id) {
            extra.push(format!("channel:{id}"));
        }
    }
    let missing: Vec<String> = extra.into_iter().filter(|id| !order.contains(id)).collect();
    let at = order.iter().position(|r| r == "allapps").unwrap_or(order.len());
    for (i, id) in missing.into_iter().enumerate() {
        order.insert(at + i, id);
    }
    order
}

pub(crate) fn build(state: &State, config: &CoreConfig, utc_offset_minutes: i32) -> HomeSnapshot {
    let graph = &state.graph;
    let now = now_ms();
    let hour_local = local_hour(now, utc_offset_minutes);
    let index = BuildIndex::new(state, now);
    let mut rows = Vec::new();
    for row_id in effective_row_order(state) {
        let built = if let Some(rest) = row_id.strip_prefix("plugin:") {
            plugin_row(state, rest)
        } else if let Some(id) = row_id.strip_prefix("channel:") {
            id.parse::<i64>().ok().and_then(|cid| channel_row(graph, &index, cid))
        } else {
            build_row(state, &index, &row_id, now, config)
        };
        if let Some(row) = built {
            rows.push(row);
        }
    }
    HomeSnapshot {
        revision: state.revision,
        generated_at_ms: now,
        stage: build_stage(graph, &index, now, hour_local, config),
        rows,
        now_playing: state.now_playing.clone(),
    }
}

fn build_row(state: &State, index: &BuildIndex, row_id: &str, now: i64, config: &CoreConfig) -> Option<Row> {
    let graph = &state.graph;
    let (title, kind, size, badge, items): (&str, TileKind, RowSize, Option<Badge>, Vec<&Item>) = match row_id {
        "continue" => {
            let items = graph.continue_watching();
            if items.is_empty() {
                // Without TV listings there is nothing to show here; say so once, in the row's
                // place, until the user grants access or dismisses it (DESIGN.md §8).
                return if !state.permissions.tv_listings && !graph.layout.dismissed_notices.contains(NOTICE_TV_LISTINGS) {
                    Some(notice_row(
                        NOTICE_TV_LISTINGS,
                        "Continue watching",
                        "Turn on TV listings",
                        "See what you were watching",
                        "permission:tv_listings",
                    ))
                } else {
                    None
                };
            }
            ("Continue watching", TileKind::Landscape, RowSize::M, None, items)
        }
        "apps" => ("Your apps", TileKind::AppBanner, RowSize::M, None, graph.your_apps()),
        "games" => ("Games", TileKind::Landscape, RowSize::M, Some(Badge::Controller), graph.games()),
        "live" => {
            let channels = graph.live_now(now);
            if channels.is_empty() {
                return None;
            }
            return Some(Row {
                id: row_id.to_string(),
                title: "Live now".to_string(),
                kind: TileKind::Landscape,
                size: RowSize::M,
                badge: Some(Badge::Live),
                tiles: channels.iter().map(|ch| live_tile(graph, index.airing(ch), ch, now)).collect(),
            });
        }
        "tvcontrols" => {
            // The TV controls card already sits in the Stage strip and the top bar; this row
            // exists for inputs, so a TV with none gets no third copy of the card.
            let devices = graph.devices();
            if devices.is_empty() {
                return None;
            }
            let mut tiles: Vec<Tile> = devices.iter().map(|it| tile_for(graph, it, TileKind::Compact)).collect();
            if config.has_tv_controls {
                tiles.insert(0, tv_controls_tile(config));
            }
            return Some(Row {
                id: row_id.to_string(),
                title: if config.has_tv_controls { "TV controls".to_string() } else { "Inputs".to_string() },
                kind: TileKind::Compact,
                size: RowSize::M,
                badge: None,
                tiles,
            });
        }
        "allapps" => ("All apps", TileKind::Compact, RowSize::M, None, graph.all_apps()),
        _ => return None,
    };
    if items.is_empty() {
        return None;
    }
    Some(Row {
        id: row_id.to_string(),
        title: title.to_string(),
        kind,
        size,
        badge,
        tiles: items.iter().map(|it| tile_for(graph, it, kind)).collect(),
    })
}

/// One plugin row ("<plugin id>:<row id>") with the items the plugin put there, if the plugin is on.
fn plugin_row(state: &State, key: &str) -> Option<Row> {
    let graph = &state.graph;
    let (pid, rid) = key.rsplit_once(':')?;
    let p = state.plugins.iter().find(|p| p.id == pid && p.enabled)?;
    let spec = p.manifest.rows.iter().find(|r| r.id == rid)?;
    let items = graph.plugin_row(&p.id, &spec.id);
    if items.is_empty() {
        return None;
    }
    let kind = match spec.style {
        lounge_plugin_api::RowStyle::Poster => TileKind::Poster,
        lounge_plugin_api::RowStyle::Compact => TileKind::Compact,
        lounge_plugin_api::RowStyle::Landscape => TileKind::Landscape,
    };
    Some(Row {
        id: format!("plugin:{key}"),
        title: spec.title.clone(),
        kind,
        size: RowSize::M,
        badge: None,
        tiles: items.iter().map(|it| tile_for(graph, it, kind)).collect(),
    })
}

/// A preview channel the user switched on, poster or landscape by what it publishes.
fn channel_row(graph: &Graph, index: &BuildIndex, channel_id: i64) -> Option<Row> {
    if !graph.layout.enabled_channels.contains(&channel_id) {
        return None;
    }
    let (id, name, items) = index.previews.iter().find(|(id, _, _)| *id == channel_id)?;
    let (id, name, items) = (*id, name.clone(), items.clone());
    if items.is_empty() {
        return None;
    }
    let portrait = items.iter().filter(|it| it.art_aspect > 0.0 && it.art_aspect < 0.9).count() * 2 > items.len();
    let kind = if portrait { TileKind::Poster } else { TileKind::Landscape };
    Some(Row {
        id: format!("channel:{id}"),
        title: if name.is_empty() { format!("Channel {id}") } else { name },
        kind,
        size: RowSize::M,
        badge: None,
        tiles: items.iter().map(|it| tile_for(graph, it, kind)).collect(),
    })
}

fn notice_row(id: &str, row_title: &str, title: &str, subtitle: &str, action: &str) -> Row {
    Row {
        id: id.to_string(),
        title: row_title.to_string(),
        kind: TileKind::Compact,
        size: RowSize::L,
        badge: None,
        tiles: vec![Tile {
            id: id.to_string(),
            kind: TileKind::Compact,
            title: title.to_string(),
            subtitle: Some(subtitle.to_string()),
            artwork_path: None,
            icon_path: None,
            light_argb: lounge_graph::color::HOUSELIGHT,
            progress: None,
            badge: None,
            action: action.to_string(),
            pinned: false,
            stage_pinned: false,
        }],
    }
}

fn tv_controls_tile(config: &CoreConfig) -> Tile {
    Tile {
        id: "device:tv".to_string(),
        kind: TileKind::Compact,
        title: "TV controls".to_string(),
        subtitle: Some(config.device_name.clone()),
        artwork_path: None,
        icon_path: None,
        light_argb: lounge_graph::color::HOUSELIGHT,
        progress: None,
        badge: None,
        action: "control:tv:open".to_string(),
        pinned: false,
        stage_pinned: false,
    }
}

/// A channel tile shows what is on it now: programme title, time left, elapsed progress.
fn live_tile(graph: &Graph, airing: Option<&Item>, ch: &Item, now: i64) -> Tile {
    let mut tile = tile_for(graph, ch, TileKind::Landscape);
    tile.badge = Some(Badge::Live);
    match airing {
        Some(prog) => {
            tile.title = prog.title.clone();
            tile.subtitle = Some(match prog.airing_end_ms {
                Some(end) => format!("Now, {}", left_text(end - now)),
                None => channel_label(ch),
            });
            tile.progress = elapsed(prog, now);
            if prog.artwork.landscape.is_some() {
                tile.artwork_path = prog.artwork.landscape.clone();
            }
        }
        None => tile.subtitle = channel_number(ch),
    }
    tile
}

/// "2 ABC": number and name together, under a programme title.
fn channel_label(ch: &Item) -> String {
    match &ch.subtitle {
        Some(number) if !number.is_empty() => format!("{number} {}", ch.title),
        _ => ch.title.clone(),
    }
}

/// "Channel 2" under the channel's own name, when nothing is known about what is on.
fn channel_number(ch: &Item) -> Option<String> {
    match &ch.subtitle {
        Some(number) if !number.is_empty() => Some(format!("Channel {number}")),
        _ => None,
    }
}

fn elapsed(prog: &Item, now: i64) -> Option<f32> {
    let (s, e) = (prog.airing_start_ms?, prog.airing_end_ms?);
    if e <= s {
        return None;
    }
    Some(((now - s) as f32 / (e - s) as f32).clamp(0.0, 1.0))
}

pub(crate) fn tile_for(graph: &Graph, it: &Item, kind: TileKind) -> Tile {
    let artwork = match kind {
        TileKind::Poster => it.artwork.poster.clone().or_else(|| it.artwork.landscape.clone()),
        TileKind::Landscape | TileKind::AppBanner => it.artwork.landscape.clone().or_else(|| it.artwork.poster.clone()),
        TileKind::Compact => None,
    };
    let badge = if it.flags.is_live {
        Some(Badge::Live)
    } else if it.flags.is_new {
        Some(Badge::New)
    } else if matches!(it.kind, ItemKind::Input | ItemKind::Device) && it.flags.connected {
        Some(Badge::Connected)
    } else {
        None
    };
    let subtitle = match it.kind {
        ItemKind::Resume | ItemKind::MediaSession => Some(content_meta(graph, it)),
        ItemKind::Program if it.flags.watch_next => Some(content_meta(graph, it)),
        _ => it.subtitle.clone(),
    };
    Tile {
        id: it.id.clone(),
        kind,
        title: it.title.clone(),
        subtitle,
        artwork_path: artwork,
        icon_path: it.artwork.logo.clone(),
        light_argb: it.light(),
        progress: it.progress,
        badge,
        action: format!("open:{}", it.id),
        pinned: it
            .package()
            .map(|p| graph.layout.pinned_apps.iter().any(|x| x == p))
            .unwrap_or(false),
        stage_pinned: graph.layout.stage_pins.iter().any(|x| x == &it.id),
    }
}

/// "S2 E4, Netflix": where you are, and which app has it (commas, never dots: DESIGN.md §4.2).
fn content_meta(graph: &Graph, it: &Item) -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Some(se) = season_episode_short(it) {
        parts.push(se);
    } else if it.kind == ItemKind::MediaSession {
        if let Some(s) = &it.subtitle {
            parts.push(s.clone());
        }
    }
    if let Some(app) = app_label(graph, it) {
        if parts.last() != Some(&app) {
            parts.push(app);
        }
    }
    parts.join(", ")
}

fn app_label(graph: &Graph, it: &Item) -> Option<String> {
    let pkg = it.package()?;
    Some(
        graph
            .get(&format!("app:{pkg}"))
            .map(|app| app.title.clone())
            .unwrap_or_else(|| pkg.rsplit('.').next().unwrap_or(pkg).to_string()),
    )
}

fn season_episode_short(it: &Item) -> Option<String> {
    match (&it.season, &it.episode) {
        (Some(s), Some(e)) => Some(format!("S{s} E{e}")),
        (None, Some(e)) => Some(format!("Episode {e}")),
        (Some(s), None) => Some(format!("Season {s}")),
        (None, None) => None,
    }
}

fn season_episode_long(it: &Item) -> Option<String> {
    match (&it.season, &it.episode) {
        (Some(s), Some(e)) => Some(format!("Season {s}, Episode {e}")),
        (None, Some(e)) => Some(format!("Episode {e}")),
        (Some(s), None) => Some(format!("Season {s}")),
        (None, None) => None,
    }
}

fn build_stage(graph: &Graph, index: &BuildIndex, now: i64, hour_local: u32, config: &CoreConfig) -> Stage {
    let candidates = graph.stage_candidates(STAGE_CAP);
    let mut spotlights = Vec::with_capacity(candidates.len() + 1);
    let mut strip = Vec::with_capacity(candidates.len() + 1);
    for it in candidates {
        spotlights.push(spotlight_for(graph, &index, it, now, hour_local));
        let kind = if it.kind == ItemKind::App { TileKind::AppBanner } else { TileKind::Landscape };
        let tile = if it.kind == ItemKind::Channel { live_tile(graph, index.airing(it), it, now) } else { tile_for(graph, it, kind) };
        strip.push(tile);
    }
    if config.has_tv_controls {
        spotlights.push(StageSpotlight {
            id: "device:tv".to_string(),
            kind: crate::SpotlightKind::Device,
            category: "Your TV".to_string(),
            title: config.device_name.clone(),
            lines: vec!["Power, input and sound, over HDMI-CEC".to_string()],
            progress: None,
            artwork_path: None,
            backdrop_path: None,
            icon_path: None,
            light_argb: lounge_graph::color::HOUSELIGHT,
            primary: StageAction {
                label: "Open TV controls".to_string(),
                action: "control:tv:open".to_string(),
            },
            secondary: None,
            explanation: None,
            framed: true,
            art_aspect: 0.0,
            band_path: None,
            focal_y: 0.5,
            source: None,
            source_icon_path: None,
        });
        strip.push(tv_controls_tile(config));
    }
    Stage { spotlights, strip }
}

fn spotlight_for(graph: &Graph, index: &BuildIndex, it: &Item, now: i64, hour_local: u32) -> StageSpotlight {
    let airing = index.airing(it);
    let (category, primary_label) = match it.kind {
        ItemKind::Resume | ItemKind::MediaSession => ("Continue watching", "Continue"),
        ItemKind::Channel => ("Live now", "Watch"),
        ItemKind::Game => ("Games", "Play"),
        ItemKind::Input | ItemKind::Device => ("Inputs", "Switch"),
        ItemKind::Program => {
            if it.flags.watch_next {
                if it.flags.is_new { ("New for you", "Watch") } else { ("Up next", "Watch") }
            } else {
                ("Tonight", "Watch")
            }
        }
        ItemKind::App => {
            if it.flags.is_game {
                ("Games", "Play")
            } else if it.last_used_ms.is_some() {
                ("Jump back in", "Open")
            } else {
                ("Your apps", "Open")
            }
        }
    };

    let mut title = it.title.clone();
    let mut progress = it.progress;
    let mut artwork = it.artwork.clone();
    let mut lines = Vec::new();
    // Where OK will take you sits in its own chip, not among the lines (DESIGN.md §5.1).
    let mut source = app_label(graph, it);
    let mut source_icon = it.package().and_then(|pkg| graph.get(&format!("app:{pkg}"))).and_then(|app| app.artwork.logo.clone());
    if let Some(rest) = it.source.strip_prefix("plugin:") {
        source = Some(index.plugin_name(rest));
        source_icon = None;
    }
    match it.kind {
        ItemKind::Channel => {
            // The channel's spotlight is what it is showing now; the chip is the channel itself.
            source = Some(channel_label(it));
            source_icon = it.artwork.logo.clone();
            if let Some(prog) = airing {
                title = prog.title.clone();
                progress = elapsed(prog, now);
                if prog.artwork.hero.is_some() || prog.artwork.landscape.is_some() {
                    artwork.hero = prog.artwork.hero.clone();
                    artwork.landscape = prog.artwork.landscape.clone();
                    artwork.backdrop = prog.artwork.backdrop.clone().or(artwork.backdrop);
                    artwork.band = prog.artwork.band.clone();
                    artwork.focal_y = prog.artwork.focal_y;
                }
                if let Some(end) = prog.airing_end_ms {
                    lines.push(left_text(end - now));
                }
            }
        }
        ItemKind::Resume | ItemKind::Program => {
            match (season_episode_long(it), &it.subtitle) {
                (Some(se), Some(sub)) => lines.push(format!("{se}: {sub}")),
                (Some(se), None) => lines.push(se),
                (None, Some(sub)) => lines.push(sub.clone()),
                (None, None) => {}
            }
            if let Some(p) = it.progress {
                if let Some(remaining) = remaining_text(it, p) {
                    lines.push(remaining);
                }
            }
        }
        ItemKind::MediaSession => {
            if let Some(s) = &it.subtitle {
                if Some(s) != source.as_ref() {
                    lines.push(s.clone());
                }
            }
        }
        ItemKind::App | ItemKind::Game => {
            source = None;
            source_icon = None;
            if let Some(s) = &it.subtitle {
                lines.push(s.clone());
            }
            if let Some(used) = it.last_used_ms {
                lines.push(ago_text(now - used));
            }
        }
        ItemKind::Input | ItemKind::Device => {
            source = None;
            source_icon = None;
            if let Some(s) = &it.subtitle {
                lines.push(s.clone());
            }
        }
    }

    let secondary = match it.kind {
        ItemKind::App | ItemKind::Game => Some(StageAction {
            label: "App info".to_string(),
            action: format!("info:{}", it.id),
        }),
        _ => None,
    };
    let kind = match it.kind {
        ItemKind::App => {
            if it.flags.is_game {
                crate::SpotlightKind::Game
            } else {
                crate::SpotlightKind::App
            }
        }
        ItemKind::Game => crate::SpotlightKind::Game,
        ItemKind::Channel => crate::SpotlightKind::Channel,
        ItemKind::Input | ItemKind::Device => crate::SpotlightKind::Device,
        ItemKind::Program | ItemKind::Resume | ItemKind::MediaSession => crate::SpotlightKind::Content,
    };
    let framed = matches!(it.kind, ItemKind::App | ItemKind::Game | ItemKind::Input | ItemKind::Device) || it.framed_art();
    StageSpotlight {
        id: it.id.clone(),
        kind,
        category: category.to_string(),
        title,
        lines,
        progress,
        artwork_path: artwork.hero.clone().or_else(|| artwork.landscape.clone()).or_else(|| artwork.poster.clone()),
        backdrop_path: artwork.backdrop.clone(),
        icon_path: artwork.logo.clone(),
        light_argb: it.light(),
        primary: StageAction {
            label: primary_label.to_string(),
            action: format!("open:{}", it.id),
        },
        secondary,
        explanation: lounge_graph::rank::explanation(it, now, hour_local),
        framed,
        art_aspect: it.art_aspect,
        band_path: if framed { None } else { artwork.band.clone() },
        focal_y: artwork.focal_y,
        source,
        source_icon_path: source_icon,
    }
}

fn remaining_text(it: &Item, progress: f32) -> Option<String> {
    let total = match (it.duration_ms, it.airing_start_ms, it.airing_end_ms) {
        (Some(d), _, _) if d > 0 => d as f32,
        (_, Some(s), Some(e)) if e > s => (e - s) as f32,
        _ => return None,
    };
    let left_ms = ((1.0 - progress.clamp(0.0, 1.0)) * total) as i64;
    Some(left_text(left_ms))
}

fn left_text(left_ms: i64) -> String {
    let minutes = (left_ms as f64 / 60_000.0).round() as i64;
    match minutes {
        m if m <= 0 => "Almost finished".to_string(),
        1 => "1 minute left".to_string(),
        m if m < 60 => format!("{m} minutes left"),
        m => {
            let h = m / 60;
            let rest = m % 60;
            if rest == 0 { format!("{h} h left") } else { format!("{h} h {rest} min left") }
        }
    }
}

fn ago_text(ms: i64) -> String {
    let minutes = (ms / 60_000).max(0);
    match minutes {
        m if m < 2 => "Opened just now".to_string(),
        m if m < 60 => format!("Opened {m} minutes ago"),
        m if m < 120 => "Opened an hour ago".to_string(),
        m if m < 1440 => format!("Opened {} hours ago", m / 60),
        m if m < 2880 => "Opened yesterday".to_string(),
        m => format!("Opened {} days ago", m / 1440),
    }
}

fn local_hour(now_ms: i64, utc_offset_minutes: i32) -> u32 {
    let local = now_ms + utc_offset_minutes as i64 * 60_000;
    (((local / 3_600_000) % 24 + 24) % 24) as u32
}

/// Actions are `<verb>:<target>`; verbs: `open`, `info`, `control`, `permission`, `media`.
pub(crate) fn perform(inner: &Inner, action: &str) -> Result<(), CoreError> {
    wait_for_graph(inner);
    let (verb, target) = action
        .split_once(':')
        .ok_or_else(|| CoreError::UnknownAction {
            action: action.to_string(),
        })?;
    let unknown = || CoreError::UnknownAction {
        action: action.to_string(),
    };
    let platform_err = |e: crate::PlatformError| CoreError::Platform { detail: e.to_string() };
    match verb {
        "open" => {
            let link = {
                let state = inner.state.lock();
                state.graph.get(target).map(|it| it.deep_link.clone()).ok_or_else(unknown)?
            };
            dispatch(inner, link)
        }
        "info" => {
            let package = {
                let state = inner.state.lock();
                state
                    .graph
                    .get(target)
                    .and_then(|it| it.package().map(|p| p.to_string()))
                    .ok_or_else(unknown)?
            };
            inner.platform.open_app_info(package).map_err(platform_err)
        }
        "control" => inner.platform.control(target.to_string()).map_err(platform_err),
        // A link shared from the phone: Android picks the app that owns it.
        "url" if target.starts_with("http://") || target.starts_with("https://") => {
            inner.platform.view_uri(target.to_string()).map_err(platform_err)
        }
        "permission" => inner.platform.request_permission(target.to_string()).map_err(platform_err),
        "media" => {
            let (package, command) = target.rsplit_once(':').ok_or_else(unknown)?;
            inner
                .platform
                .media_command(package.to_string(), command.to_string())
                .map_err(platform_err)
        }
        _ => Err(unknown()),
    }
}

/// Block briefly until the worker has loaded the stored graph.
pub(crate) fn wait_for_graph(inner: &Inner) {
    let mut state = inner.state.lock();
    if !state.loaded {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while !state.loaded {
            if inner.loaded.wait_until(&mut state, deadline).timed_out() {
                break;
            }
        }
    }
}

fn dispatch(inner: &Inner, link: DeepLink) -> Result<(), CoreError> {
    let platform_err = |e: crate::PlatformError| CoreError::Platform { detail: e.to_string() };
    match link {
        DeepLink::LaunchApp { package, activity } => inner.platform.launch_app(package, activity).map_err(platform_err),
        DeepLink::ViewUri(uri) => inner.platform.view_uri(uri).map_err(platform_err),
        DeepLink::ViewOrLaunch { uri, package } => match inner.platform.view_uri(uri) {
            Ok(()) => Ok(()),
            Err(_) => inner.platform.launch_app(package, String::new()).map_err(platform_err),
        },
        DeepLink::Play { url, title } => inner.platform.play_stream(url, title).map_err(platform_err),
        DeepLink::PluginResolve { plugin, action } => {
            let (reply_tx, reply_rx) = crossbeam_channel::bounded(1);
            inner
                .plugins
                .tx
                .send(crate::plugins::PluginJob::Resolve { plugin: plugin.clone(), action, reply: reply_tx })
                .map_err(|_| CoreError::Plugin { detail: "plugin thread is gone".into() })?;
            let resolved = reply_rx
                .recv_timeout(std::time::Duration::from_secs(15))
                .map_err(|_| CoreError::Plugin { detail: format!("{plugin} did not answer in time") })?
                .map_err(|detail| CoreError::Plugin { detail })?;
            let link = if let Some(url) = resolved.play {
                DeepLink::Play { url, title: resolved.title.unwrap_or_default() }
            } else if let (Some(uri), Some(pkg)) = (resolved.view.clone(), resolved.fallback_launch.clone()) {
                DeepLink::ViewOrLaunch { uri, package: pkg }
            } else if let Some(uri) = resolved.view {
                DeepLink::ViewUri(uri)
            } else if let Some(pkg) = resolved.launch {
                DeepLink::LaunchApp { package: pkg, activity: String::new() }
            } else {
                return Err(CoreError::Plugin { detail: format!("{plugin} returned nothing to open") });
            };
            dispatch(inner, link)
        }
        DeepLink::SwitchInput(id) => inner.platform.switch_input(id).map_err(platform_err),
        DeepLink::Tune(uri) => inner.platform.tune(uri).map_err(platform_err),
        DeepLink::Control(cmd) => inner.platform.control(cmd).map_err(platform_err),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_hour_wraps() {
        let midnight_utc = 1_700_000_000_000i64 - (1_700_000_000_000i64 % 86_400_000);
        assert_eq!(local_hour(midnight_utc, 0), 0);
        assert_eq!(local_hour(midnight_utc, 600), 10);
        assert_eq!(local_hour(midnight_utc, -60), 23);
    }

    #[test]
    fn ago_text_voice() {
        assert_eq!(ago_text(30_000), "Opened just now");
        assert_eq!(ago_text(5 * 60_000), "Opened 5 minutes ago");
        assert_eq!(ago_text(90 * 60_000), "Opened an hour ago");
        assert_eq!(ago_text(3 * 3_600_000), "Opened 3 hours ago");
        assert_eq!(ago_text(30 * 3_600_000), "Opened yesterday");
    }

    #[test]
    fn left_text_voice() {
        assert_eq!(left_text(20_000), "Almost finished");
        assert_eq!(left_text(60_000), "1 minute left");
        assert_eq!(left_text(23 * 60_000), "23 minutes left");
        assert_eq!(left_text(95 * 60_000), "1 h 35 min left");
        assert_eq!(left_text(120 * 60_000), "2 h left");
    }

    #[test]
    fn content_meta_reads_like_the_design() {
        let mut g = Graph::default();
        g.replace_source("apps", vec![Item::app("com.netflix.ninja", "Main", "Netflix")]);
        let mut it = Item::content("wn:1", ItemKind::Resume, "tvprovider", "The Bear", DeepLink::ViewUri("x".into()));
        it.owner = Some("com.netflix.ninja".into());
        it.season = Some("2".into());
        it.episode = Some("4".into());
        assert_eq!(content_meta(&g, &it), "S2 E4, Netflix");
        it.season = None;
        it.episode = None;
        assert_eq!(content_meta(&g, &it), "Netflix");
    }

    #[test]
    fn live_tile_shows_the_programme() {
        let mut g = Graph::default();
        let mut ch = Item::content("chan:2", ItemKind::Channel, "tvprovider", "ABC", DeepLink::Tune("a".into()));
        ch.subtitle = Some("2".into());
        ch.channel_id = Some(2);
        ch.flags.is_live = true;
        let mut p = Item::content("prog:1", ItemKind::Program, "tvprovider", "News", DeepLink::Tune("a".into()));
        p.channel_id = Some(2);
        p.airing_start_ms = Some(0);
        p.airing_end_ms = Some(60 * 60_000);
        g.replace_source("tvprovider", vec![ch.clone(), p]);
        let ch = g.get("chan:2").unwrap();
        let tile = live_tile(&g, g.airing_now(2, 30 * 60_000), ch, 30 * 60_000);
        assert_eq!(tile.title, "News");
        assert_eq!(tile.subtitle.as_deref(), Some("Now, 30 minutes left"));
        assert_eq!(tile.progress, Some(0.5));
        assert_eq!(tile.badge, Some(Badge::Live));
    }
}
