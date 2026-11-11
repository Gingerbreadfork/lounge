//! Turns the graph into a `HomeSnapshot` and resolves actions back into platform calls.

use crate::{
    now_ms, Badge, CoreConfig, CoreError, HomeSnapshot, Inner, ItemRef, Row, RowInfo, RowSize, SourceInfo, Stage, StageAction,
    StageSpotlight, State, Tile, TileKind,
};
use lounge_graph::{DeepLink, Graph, Item, ItemKind};
use std::collections::{HashMap, HashSet};

/// Lookups a build needs more than once, computed once per build.
pub(crate) struct BuildIndex<'a> {
    airing: HashMap<i64, &'a Item>,
    previews: Vec<(i64, String, Vec<&'a Item>)>,
    plugin_names: HashMap<&'a str, &'a str>,
    habits: &'a HashMap<String, [u32; 24]>,
    /// What the Stage strip shows, in order, so Continue watching can start after it.
    stage: Vec<&'a Item>,
    /// Plugins whose last refresh failed, so their rows say they show what was kept.
    offline: HashSet<String>,
    clock_24h: bool,
}

impl<'a> BuildIndex<'a> {
    fn new(state: &'a State, now: i64) -> BuildIndex<'a> {
        BuildIndex {
            airing: state.graph.airing_index(now),
            previews: state.graph.preview_channels(),
            plugin_names: state.plugins.iter().map(|p| (p.id.as_str(), p.manifest.name.as_str())).collect(),
            habits: &state.habits,
            stage: state.graph.stage_candidates(STAGE_CAP),
            offline: offline_plugins(state),
            clock_24h: state.clock_24h,
        }
    }

    fn airing(&self, ch: &Item) -> Option<&'a Item> {
        ch.channel_id.and_then(|id| self.airing.get(&id).copied())
    }

    fn on_stage(&self, id: &str) -> bool {
        self.stage.iter().any(|it| it.id == id)
    }

    /// The plugin's name as its manifest gives it, or its id when it is not installed.
    fn plugin_name(&self, id: &str) -> String {
        self.plugin_names.get(id).map(|n| n.to_string()).unwrap_or_else(|| id.to_string())
    }
}

/// The plugins that are on and whose last refresh ended in an error or a question.
fn offline_plugins(state: &State) -> HashSet<String> {
    if !state.plugins.iter().any(|p| p.enabled) {
        return HashSet::new();
    }
    state.store.plugins().unwrap_or_default().into_iter().filter(|r| r.enabled && r.last_error.is_some()).map(|r| r.id).collect()
}

pub(crate) const STAGE_CAP: usize = 6;
const NOTICE_TV_LISTINGS: &str = "notice:tv_listings";
/// The row of plugins suggested for the apps on the TV.
const NOTICE_PLUGINS: &str = "notice:plugins";
/// Beside the header of a plugin's row while its last refresh failed (DESIGN.md §8).
const OFFLINE_NOTE: &str = "Offline, showing what we have";

/// The notice that suggests setting a plugin up.
pub(crate) fn plugin_notice(plugin: &str) -> String {
    format!("notice:plugin:{plugin}")
}

/// The core rows and their titles, in default order. Every app is reachable from the All apps
/// screen, which Your apps ends with, so there is no row of them all.
pub(crate) const CORE_ROWS: [(&str, &str); 5] = [
    ("continue", "Continue watching"),
    ("apps", "Your apps"),
    ("games", "Games"),
    ("live", "Live now"),
    ("tvcontrols", "TV controls"),
];

/// The row order as it will be built: the user's order (or the default), with plugin and
/// channel rows the user has not placed appended. Rows that cannot be built now, such as a
/// switched-off plugin's, are left out; their saved places are kept for later.
pub(crate) fn effective_row_order(state: &State) -> Vec<String> {
    let graph = &state.graph;
    let mut order = graph.layout.row_order_or_default();
    let mut extra: Vec<String> = CORE_ROWS.iter().map(|(id, _)| id.to_string()).collect();
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
    order.retain(|id| extra.contains(id));
    let missing: Vec<String> = extra.into_iter().filter(|id| !order.contains(id)).collect();
    order.extend(missing);
    order
}

/// `order` with `id` moved to `to` among the rows that can be built now; rows saved but not
/// buildable keep a place after them.
pub(crate) fn moved_row_order(state: &State, id: &str, to: usize) -> Vec<String> {
    let mut order = moved(effective_row_order(state), id, to);
    for saved in state.graph.layout.row_order_or_default() {
        if !order.contains(&saved) {
            order.push(saved);
        }
    }
    order
}

/// Whether a row belongs on the home screen: the user's choice, or shown.
pub(crate) fn row_shown(state: &State, row_id: &str) -> bool {
    state.graph.row_choice(row_id).unwrap_or(true)
}

/// `order` with `id` moved to `to`, clamped to the ends; an id not in the order is placed there.
pub(crate) fn moved(mut order: Vec<String>, id: &str, to: usize) -> Vec<String> {
    order.retain(|r| r != id);
    order.insert(to.min(order.len()), id.to_string());
    order
}

/// "3 apps", "1 title", or `none` when there are none.
fn counted(n: usize, one: &str, many: &str, none: &str) -> String {
    match n {
        0 => none.to_string(),
        1 => format!("1 {one}"),
        n => format!("{n} {many}"),
    }
}

/// Every row the home could show, in order, with what each holds, for the row editors on the
/// TV and the phone. Rows with nothing in them are listed too: the editor is where you learn why
/// a row is not on the screen.
pub(crate) fn row_infos(state: &State, config: &CoreConfig) -> Vec<RowInfo> {
    let graph = &state.graph;
    let previews = graph.preview_channels();
    effective_row_order(state)
        .into_iter()
        .map(|id| {
            let shown = row_shown(state, &id);
            let chosen = graph.row_choice(&id).is_some();
            let (title, count, detail): (String, usize, String) = if let Some(rest) = id.strip_prefix("plugin:") {
                let (pid, rid) = rest.split_once(':').unwrap_or((rest, ""));
                let plugin = state.plugins.iter().find(|p| p.id == pid);
                let name = plugin.map(|p| p.manifest.name.clone()).unwrap_or_else(|| pid.to_string());
                let title = plugin
                    .and_then(|p| p.manifest.rows.iter().find(|r| r.id == rid))
                    .map(|r| r.title.clone())
                    .unwrap_or_else(|| rid.to_string());
                let n = graph.plugin_row(pid, rid).len();
                (title, n, format!("{name}, {}", counted(n, "title", "titles", "nothing yet")))
            } else if let Some(cid) = id.strip_prefix("channel:") {
                let cid: i64 = cid.parse().unwrap_or(0);
                let found = previews.iter().find(|(id, _, _)| *id == cid);
                let (name, n) = found.map(|(_, name, items)| (name.clone(), items.len())).unwrap_or_default();
                let title = if name.is_empty() { format!("Channel {cid}") } else { name };
                (title, n, counted(n, "programme", "programmes", "Nothing yet"))
            } else {
                let title = match id.as_str() {
                    "tvcontrols" if !config.has_tv_controls => "Inputs".to_string(),
                    other => CORE_ROWS.iter().find(|(i, _)| *i == other).map(|(_, t)| t.to_string()).unwrap_or_else(|| other.to_string()),
                };
                let (n, detail) = match id.as_str() {
                    "continue" => {
                        // The row holds what the Stage strip does not already show.
                        let all = graph.continue_watching();
                        let stage = graph.stage_candidates(STAGE_CAP);
                        let n = all.iter().filter(|it| !stage.iter().any(|s| s.id == it.id)).count();
                        let detail = if n == 0 && !all.is_empty() {
                            format!("{}, all on the Stage", counted(all.len(), "title", "titles", ""))
                        } else {
                            counted(n, "title", "titles", "Nothing to continue yet")
                        };
                        (n, detail)
                    }
                    "apps" => {
                        let (pinned, recent) = graph.your_apps_counts();
                        let detail = match (pinned, recent) {
                            (0, 0) => "Only the way to All apps, until you pin one".to_string(),
                            _ if !state.permissions.usage && recent > 0 => "Allow usage access in Settings to follow what you open".to_string(),
                            (0, r) => format!("{} recent", counted(r, "app", "apps", "")),
                            (p, 0) => format!("{}, pinned only", counted(p, "app", "apps", "")),
                            (p, r) => format!("{p} pinned, {r} recent"),
                        };
                        (pinned + recent, detail)
                    }
                    "games" => {
                        let n = graph.games().len();
                        (n, counted(n, "game", "games", "No games installed"))
                    }
                    "live" => {
                        let n = graph.live_now().len();
                        (n, counted(n, "channel", "channels", "No channels yet"))
                    }
                    "tvcontrols" => {
                        let n = graph.devices().len();
                        (n, counted(n, "input", "inputs", "No inputs found"))
                    }
                    _ => (0, String::new()),
                };
                (title, n, detail)
            };
            RowInfo { id, title, shown, chosen, count: count as u32, detail }
        })
        .collect()
}

/// A hidden or pinned item named for an editor; an item that has since gone keeps a readable name.
pub(crate) fn item_ref(graph: &Graph, id: &str) -> ItemRef {
    match graph.get(id) {
        Some(it) => ItemRef { id: id.to_string(), title: it.title.clone(), icon_path: it.artwork.logo.clone() },
        None => gone_ref(graph, id),
    }
}

/// A name for an item that has gone. An app is known by its package; a title only by the app
/// its id names, since what follows is the app's own number for it.
fn gone_ref(graph: &Graph, id: &str) -> ItemRef {
    let named = |title: String, icon_path: Option<String>| ItemRef { id: id.to_string(), title, icon_path };
    if let Some(package) = id.strip_prefix("app:") {
        return named(package.to_string(), None);
    }
    match id.split(':').find_map(|part| graph.get(&format!("app:{part}"))) {
        Some(app) => named(format!("A title {} no longer lists", app.title), app.artwork.logo.clone()),
        None => named("A title that is no longer listed".to_string(), None),
    }
}

/// The items the user hid from every row, by title.
pub(crate) fn hidden_items(state: &State) -> Vec<ItemRef> {
    let mut v: Vec<ItemRef> = state.graph.layout.hidden.iter().map(|id| item_ref(&state.graph, id)).collect();
    v.sort_by_key(|r| r.title.to_lowercase());
    v
}

/// An app or plugin Continue watching draws on, named for a menu or an editor: the plugin for a
/// plugin's own key, the app when it is installed, else the plugin whose titles open in it.
/// `from` is one of its titles, when one is at hand.
fn provider_ref(graph: &Graph, plugin_name: &dyn Fn(&str) -> String, provider: &str, from: Option<&Item>) -> ItemRef {
    let named = |title: String, icon_path: Option<String>| ItemRef { id: provider.to_string(), title, icon_path };
    if let Some(id) = provider.strip_prefix("plugin:") {
        return named(plugin_name(id), None);
    }
    if let Some(app) = graph.get(&format!("app:{provider}")) {
        return named(app.title.clone(), app.artwork.logo.clone());
    }
    let from = from.or_else(|| graph.items().find(|it| it.provider() == Some(provider)));
    match from.and_then(|it| it.source.strip_prefix("plugin:")) {
        Some(id) => named(plugin_name(id), None),
        None => named(provider.rsplit('.').next().unwrap_or(provider).to_string(), None),
    }
}

/// Every app and plugin Continue watching draws on, by name, with what each brings and whether
/// it is shown. One left out stays listed while it brings nothing, so it can be shown again.
pub(crate) fn continue_sources(state: &State) -> Vec<SourceInfo> {
    let graph = &state.graph;
    let muted = &graph.layout.muted_providers;
    let plugin_name = |id: &str| state.plugins.iter().find(|p| p.id == id).map(|p| p.manifest.name.clone()).unwrap_or_else(|| id.to_string());
    let source = |provider: &str, count: usize, from: Option<&Item>| {
        let named = provider_ref(graph, &plugin_name, provider, from);
        SourceInfo { id: named.id, title: named.title, icon_path: named.icon_path, count: count as u32, shown: !muted.contains(provider) }
    };
    let mut sources: Vec<SourceInfo> = graph.continue_providers().into_iter().map(|(provider, count, from)| source(provider, count, Some(from))).collect();
    let idle: Vec<SourceInfo> = muted.iter().filter(|p| !sources.iter().any(|s| &s.id == *p)).map(|p| source(p, 0, None)).collect();
    sources.extend(idle);
    sources.sort_by_key(|s| (s.title.to_lowercase(), s.id.clone()));
    sources
}

/// The tile for something to continue, saying which app or plugin it comes from.
fn provided(mut tile: Tile, graph: &Graph, index: &BuildIndex, it: &Item) -> Tile {
    if let Some(provider) = it.provider().filter(|_| it.continues()) {
        let named = provider_ref(graph, &|id| index.plugin_name(id), provider, Some(it));
        tile.provider = Some(named.id);
        tile.provider_name = Some(named.title);
    }
    tile
}

/// The most tiles a content row carries, so a big source cannot swell every rebuild. The guide
/// and search reach the rest.
const ROW_TILES: usize = 60;
/// Stops across the lowered Stage's light: close enough that a straight gradient between them
/// keeps the eased pools.
const GLOW_STOPS: usize = 25;

pub(crate) fn build(state: &State, config: &CoreConfig, utc_offset_minutes: i32) -> HomeSnapshot {
    let graph = &state.graph;
    let now = now_ms();
    let hour_local = local_hour(now, utc_offset_minutes);
    let index = BuildIndex::new(state, now);
    let every_app = graph.every_app();
    let mut rows = Vec::new();
    let mut hidden_rows = Vec::new();
    for row_id in effective_row_order(state) {
        let built = if let Some(rest) = row_id.strip_prefix("plugin:") {
            plugin_row(state, &index, rest)
        } else if let Some(id) = row_id.strip_prefix("channel:") {
            id.parse::<i64>().ok().and_then(|cid| channel_row(graph, &index, cid))
        } else {
            build_row(state, &index, &row_id, now, config)
        };
        if let Some(row) = built {
            if row_shown(state, &row_id) {
                rows.push(row);
            } else {
                hidden_rows.push(row);
            }
        }
    }
    let suggested = plugin_suggestions(state);
    if !suggested.is_empty() {
        // Under Continue watching, which is where what they bring would show.
        let at = rows.iter().position(|r| r.id == "continue" || r.id == NOTICE_TV_LISTINGS).map_or(0, |i| i + 1);
        rows.insert(at, Row { id: NOTICE_PLUGINS.to_string(), title: "Plugins for your apps".to_string(), kind: TileKind::Compact, size: RowSize::L, badge: None, tiles: suggested, note: None });
    }
    if let Some(version) = &state.update {
        let id = format!("notice:update:{version}");
        if !graph.layout.dismissed_notices.contains(&id) {
            rows.insert(0, notice_row(&id, "Lounge", &format!("Lounge {version} is ready"), "Press to install the update", "control:update"));
        }
    }
    HomeSnapshot {
        revision: state.revision,
        generated_at_ms: now,
        stage: build_stage(state, &index, now, hour_local, config),
        rows,
        hidden_rows,
        all_apps: ranked_app_tiles(graph, &every_app),
        now_playing: state.now_playing.clone(),
        wallpapers: graph
            .wallpapers()
            .into_iter()
            .filter_map(|it| {
                let path = it.artwork.wallpaper.clone().or_else(|| it.artwork.hero.clone()).or_else(|| it.artwork.landscape.clone())?;
                let source = it.source.strip_prefix("plugin:").map(|id| index.plugin_name(id)).unwrap_or_else(|| it.source.clone());
                Some(crate::Wallpaper { artwork_path: path, title: it.title.clone(), subtitle: it.subtitle.clone(), source, framed: it.flags.framed })
            })
            .take(80)
            .collect(),
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
            // The Stage strip already shows the first of them, so the row starts after it.
            let rest = items.into_iter().filter(|it| !index.on_stage(&it.id)).take(ROW_TILES).collect();
            ("Continue watching", TileKind::Landscape, RowSize::M, None, rest)
        }
        "apps" => {
            // The row ends with the way to every app, and stands even with no pins and no recent
            // apps, so that way is never lost.
            let total = graph.every_app().len();
            if total == 0 {
                return None;
            }
            let mut tiles: Vec<Tile> = graph.your_apps().iter().map(|it| tile_for(graph, it, TileKind::AppBanner)).collect();
            tiles.push(all_apps_tile(total));
            return Some(Row { id: row_id.to_string(), title: "Your apps".to_string(), kind: TileKind::AppBanner, size: RowSize::M, badge: None, tiles, note: None });
        }
        "games" => ("Games", TileKind::Landscape, RowSize::M, Some(Badge::Controller), graph.games()),
        "live" => {
            let channels = graph.live_now();
            if channels.is_empty() {
                return None;
            }
            return Some(Row {
                id: row_id.to_string(),
                title: "Live now".to_string(),
                kind: TileKind::Landscape,
                size: RowSize::M,
                badge: Some(Badge::Live),
                tiles: channels.iter().take(ROW_TILES).map(|ch| live_tile(graph, index.airing(ch), ch, now)).collect(),
                note: None,
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
                note: None,
            });
        }
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
        tiles: items.iter().map(|it| provided(tile_for(graph, it, kind), graph, index, it)).collect(),
        note: None,
    })
}

/// One plugin row ("<plugin id>:<row id>") with the items the plugin put there, if the plugin is on.
fn plugin_row(state: &State, index: &BuildIndex, key: &str) -> Option<Row> {
    let graph = &state.graph;
    // Plugin ids never hold a colon; row ids may.
    let (pid, rid) = key.split_once(':')?;
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
        tiles: items.iter().take(ROW_TILES).map(|it| tile_for(graph, it, kind)).collect(),
        note: index.offline.contains(&p.id).then(|| OFFLINE_NOTE.to_string()),
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
        tiles: items.iter().take(ROW_TILES).map(|it| tile_for(graph, it, kind)).collect(),
        note: None,
    })
}

/// A notice tile for each plugin that is off while an app it is for is on the TV, until the
/// plugin is turned on or the notice dismissed. Pressing one opens the plugin's page. An app that
/// already fills Continue watching is offered its plugin only for the rows the plugin adds.
fn plugin_suggestions(state: &State) -> Vec<Tile> {
    let graph = &state.graph;
    let continuing = graph.continue_watching();
    state
        .plugins
        .iter()
        .filter(|p| !p.enabled)
        .filter_map(|p| {
            let id = plugin_notice(&p.id);
            if graph.layout.dismissed_notices.contains(&id) {
                return None;
            }
            // An app the person hid is one they do not want to hear about.
            let (package, app) = p
                .manifest
                .for_apps
                .iter()
                .find_map(|package| graph.get(&format!("app:{package}")).map(|app| (package.as_str(), app)))
                .filter(|(_, app)| !graph.is_hidden(&app.id))?;
            let fills = continuing.iter().any(|it| it.provider() == Some(package));
            let subtitle = if fills { what_it_adds(&p.manifest)? } else { "See what you were watching".to_string() };
            Some(Tile {
                id,
                kind: TileKind::Compact,
                title: format!("Set up {}", p.manifest.name),
                subtitle: Some(subtitle),
                artwork_path: None,
                icon_path: app.artwork.logo.clone(),
                light_argb: lounge_graph::color::HOUSELIGHT,
                progress: None,
                badge: None,
                action: format!("control:plugin:{}", p.id),
                pinned: false,
                stage_pinned: false,
                borrowed_art: false,
                art_title: None,
                opened_rank: None,
                installed_rank: None,
                provider: None,
                provider_name: None,
                hidden: false,
                time_left: None,
            })
        })
        .collect()
}

/// What a plugin's rows add, as "Adds recently added and favourites"; none when it has no rows.
fn what_it_adds(manifest: &lounge_plugin_api::Manifest) -> Option<String> {
    let rows: Vec<String> = manifest.rows.iter().map(|r| row_words(&r.title, &manifest.name)).filter(|w| !w.is_empty()).collect();
    match rows.as_slice() {
        [] => None,
        [one] => Some(format!("Adds {one}")),
        [rest @ .., last] => Some(format!("Adds {} and {last}", rest.join(", "))),
    }
}

/// A row's title as words inside a sentence: "Recently added on Jellyfin" from Jellyfin becomes
/// "recently added". A first word with capitals of its own, such as YouTube, keeps them.
fn row_words(title: &str, plugin: &str) -> String {
    let mut words = title.trim();
    for joiner in [" on ", " from ", " in "] {
        let suffix = format!("{joiner}{plugin}");
        let cut = words.len().saturating_sub(suffix.len());
        if cut > 0 && words.get(cut..).is_some_and(|tail| tail.eq_ignore_ascii_case(&suffix)) {
            words = &words[..cut];
            break;
        }
    }
    let first = words.split_whitespace().next().unwrap_or("");
    if first.chars().skip(1).any(char::is_uppercase) {
        return words.to_string();
    }
    let mut chars = words.chars();
    chars.next().map(|c| c.to_lowercase().chain(chars).collect()).unwrap_or_default()
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
            borrowed_art: false,
            art_title: None,
            opened_rank: None,
            installed_rank: None,
            provider: None,
            provider_name: None,
            hidden: false,
            time_left: None,
        }],
        note: None,
    }
}

/// Every app as a tile, A to Z, each carrying its place by last opened and by arrival so the
/// screen can reorder without the times themselves, which move while something plays.
fn ranked_app_tiles(graph: &Graph, apps: &[&Item]) -> Vec<Tile> {
    let mut by_opened: Vec<usize> = (0..apps.len()).collect();
    by_opened.sort_by(|&a, &b| apps[b].last_used_ms.cmp(&apps[a].last_used_ms).then(a.cmp(&b)));
    let mut by_added: Vec<usize> = (0..apps.len()).collect();
    by_added.sort_by(|&a, &b| apps[b].added_ms.cmp(&apps[a].added_ms).then(a.cmp(&b)));
    let mut opened_rank = vec![0u32; apps.len()];
    let mut installed_rank = vec![0u32; apps.len()];
    for (rank, &i) in by_opened.iter().enumerate() {
        opened_rank[i] = rank as u32;
    }
    for (rank, &i) in by_added.iter().enumerate() {
        installed_rank[i] = rank as u32;
    }
    apps.iter()
        .enumerate()
        .map(|(i, it)| {
            let mut tile = tile_for(graph, it, TileKind::AppBanner);
            tile.opened_rank = Some(opened_rank[i]);
            tile.installed_rank = Some(installed_rank[i]);
            tile
        })
        .collect()
}

/// The last tile of Your apps: opens the All apps screen.
fn all_apps_tile(count: usize) -> Tile {
    Tile {
        id: "control:allapps".to_string(),
        kind: TileKind::AppBanner,
        title: "All apps".to_string(),
        subtitle: Some(counted(count, "app", "apps", "Nothing yet")),
        artwork_path: None,
        icon_path: None,
        light_argb: lounge_graph::color::HOUSELIGHT,
        progress: None,
        badge: None,
        action: "control:allapps".to_string(),
        pinned: false,
        stage_pinned: false,
        borrowed_art: false,
        art_title: None,
        opened_rank: None,
        installed_rank: None,
        provider: None,
        provider_name: None,
        hidden: false,
        time_left: None,
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
        borrowed_art: false,
        art_title: None,
        opened_rank: None,
        installed_rank: None,
        provider: None,
        provider_name: None,
        hidden: false,
        time_left: None,
    }
}

/// A channel tile shows what is on it now: programme title, time left, elapsed progress.
fn live_tile(graph: &Graph, airing: Option<&Item>, ch: &Item, now: i64) -> Tile {
    let mut tile = tile_for(graph, ch, TileKind::Landscape);
    tile.badge = Some(Badge::Live);
    match airing {
        Some(prog) => {
            tile.title = prog.title.clone();
            tile.time_left = prog.airing_end_ms.map(|end| left_text(end - now));
            tile.subtitle = Some(tile.time_left.clone().unwrap_or_else(|| channel_label(ch)));
            tile.progress = elapsed(prog, now);
            if prog.artwork.landscape.is_some() {
                tile.artwork_path = prog.artwork.landscape.clone();
            }
        }
        None => tile.subtitle = channel_number(ch),
    }
    // A channel whose logo fills the tile as its picture does not wear it in the corner too. The
    // picture is the channel's, lent to the programme, and already says the channel's name, so
    // the tile does not say it again.
    if tile.artwork_path.is_some() && tile.artwork_path == ch.artwork.landscape {
        tile.icon_path = None;
        tile.borrowed_art = true;
        tile.art_title = past_the_name(&tile.title, &ch.title).map(str::to_string);
    }
    tile
}

/// What a title says beyond its channel's name, when it starts with that name: "Wicked Tuna:
/// Big Fish" on Wicked Tuna says "Big Fish", and "COPS" on COPS says nothing more. After a
/// name of one word a space is not enough unless an episode's number follows, since "Wild
/// Kratts" on a channel called Wild is a title of its own.
fn past_the_name<'a>(title: &'a str, name: &str) -> Option<&'a str> {
    let name = name.trim();
    let (head, rest) = title.split_at_checked(name.len())?;
    if name.is_empty() || !head.eq_ignore_ascii_case(name) {
        return None;
    }
    let spaced = rest.trim_start();
    if let Some(after) = spaced.strip_prefix([':', '-', '–', '—', '|']) {
        return Some(after.trim_start());
    }
    let after_a_space = spaced.len() < rest.len() && (name.contains(char::is_whitespace) || numbers_an_episode(spaced));
    (spaced.is_empty() || after_a_space).then_some(spaced)
}

/// Whether text opens with a season or episode number: "S5 E3", "Season 2", "Ep 4", "12".
fn numbers_an_episode(text: &str) -> bool {
    let word = text.split_whitespace().next().unwrap_or("").trim_end_matches([':', ',', '.']).to_ascii_lowercase();
    let numbered = |prefix: &str| word.strip_prefix(prefix).is_some_and(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()));
    matches!(word.as_str(), "season" | "series" | "episode" | "ep" | "part") || ["", "s", "e", "ep"].into_iter().any(numbered)
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

/// The app whose pictures stand in for a session that brought none of its own: a live stream
/// from 7plus has no key art, so its tile wears the 7plus banner.
pub(crate) fn lender<'a>(graph: &'a Graph, it: &Item) -> Option<&'a Item> {
    if it.kind != ItemKind::MediaSession || it.artwork.landscape.is_some() {
        return None;
    }
    graph.get(&format!("app:{}", it.owner.as_deref()?)).filter(|app| app.artwork.landscape.is_some())
}

pub(crate) fn tile_for(graph: &Graph, it: &Item, kind: TileKind) -> Tile {
    let lent = lender(graph, it);
    let art = lent.map(|app| &app.artwork).unwrap_or(&it.artwork);
    let artwork = match kind {
        TileKind::Poster => art.poster.clone().or_else(|| art.landscape.clone()),
        TileKind::Landscape | TileKind::AppBanner => art.landscape.clone().or_else(|| art.poster.clone()),
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
        // An episode in a row leads with where it falls: "S2 E3, Honeydew".
        ItemKind::Program => match (&it.season, &it.episode, &it.subtitle) {
            (Some(s), Some(e), Some(sub)) => Some(format!("S{s} E{e}, {sub}")),
            (Some(s), Some(e), None) => Some(format!("S{s} E{e}")),
            _ => it.subtitle.clone(),
        },
        _ => it.subtitle.clone(),
    };
    Tile {
        id: it.id.clone(),
        kind,
        title: it.title.clone(),
        subtitle,
        artwork_path: artwork,
        icon_path: art.logo.clone(),
        light_argb: lent.map(|app| app.light()).unwrap_or_else(|| it.light()),
        progress: it.progress,
        badge,
        action: match &it.deep_link {
            // Said outright, so search can leave out a tile that would only open search again.
            DeepLink::Control(c) if c.starts_with("search:") => format!("control:{c}"),
            // A plugin's tile to read has no action, so a screen can tell it opens nothing.
            DeepLink::None if it.kind == ItemKind::Program && it.channel_id.is_none() && it.source.starts_with("plugin:") => String::new(),
            _ => format!("open:{}", it.id),
        },
        // Only an app itself is pinned; a programme from a pinned app is not.
        pinned: it.kind == ItemKind::App && it.package().is_some_and(|p| graph.layout.pinned_apps.iter().any(|x| x == p)),
        stage_pinned: graph.layout.stage_pins.iter().any(|x| x == &it.id),
        borrowed_art: lent.is_some(),
        art_title: None,
        opened_rank: None,
        installed_rank: None,
        provider: None,
        provider_name: None,
        // The screens draw and say this themselves; as a second line it would be searched like a title.
        hidden: graph.is_hidden(&it.id),
        time_left: it.progress.and_then(|p| remaining_text(it, p)),
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

fn build_stage(state: &State, index: &BuildIndex, now: i64, hour_local: u32, config: &CoreConfig) -> Stage {
    let graph = &state.graph;
    let candidates = &index.stage;
    let mut spotlights = Vec::with_capacity(candidates.len() + 1);
    let mut strip = Vec::with_capacity(candidates.len() + 1);
    for &it in candidates {
        spotlights.push(spotlight_for(graph, index, it, now, hour_local));
        let kind = if it.kind == ItemKind::App { TileKind::AppBanner } else { TileKind::Landscape };
        let tile = if it.kind == ItemKind::Channel { live_tile(graph, index.airing(it), it, now) } else { provided(tile_for(graph, it, kind), graph, index, it) };
        strip.push(tile);
    }
    // With Your apps off the home, the strip ends with the way to every app instead.
    if !row_shown(state, "apps") && !graph.every_app().is_empty() {
        let tile = all_apps_tile(graph.every_app().len());
        spotlights.push(StageSpotlight {
            id: tile.id.clone(),
            kind: crate::SpotlightKind::App,
            category: "Your apps".to_string(),
            title: "All apps".to_string(),
            lines: vec![tile.subtitle.clone().unwrap_or_default(), "Hold OK on an app there to pin it".to_string()],
            progress: None,
            artwork_path: None,
            backdrop_path: None,
            icon_path: None,
            light_argb: lounge_graph::color::HOUSELIGHT,
            primary: StageAction { label: "Open".to_string(), action: tile.action.clone() },
            secondary: None,
            explanation: None,
            framed: true,
            art_aspect: 0.0,
            glow_argb: Vec::new(),
            focal_y: 0.5,
            source: None,
            source_icon_path: None,
        });
        strip.push(tile);
    }
    if config.has_tv_controls {
        spotlights.push(StageSpotlight {
            id: "device:tv".to_string(),
            kind: crate::SpotlightKind::Device,
            category: "Your TV".to_string(),
            title: "TV controls".to_string(),
            lines: vec!["Turn the TV on, switch inputs and change the sound".to_string()],
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
            glow_argb: Vec::new(),
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
        ItemKind::Wallpaper => ("Pictures", "Open"),
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
    let lent = lender(graph, it);
    let mut artwork = lent.map(|app| app.artwork.clone()).unwrap_or_else(|| it.artwork.clone());
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
                    artwork.glow = prog.artwork.glow.clone();
                    artwork.focal_y = prog.artwork.focal_y;
                }
                // The programme's own line (an episode title, the game being played), then how long is left.
                if let Some(s) = prog.subtitle.as_deref().filter(|s| !s.trim().is_empty()) {
                    lines.push(s.to_string());
                }
                if let Some(end) = prog.airing_end_ms {
                    lines.push(left_text(end - now));
                }
            }
        }
        ItemKind::Resume | ItemKind::Program | ItemKind::Wallpaper => {
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
        ItemKind::Program | ItemKind::Resume | ItemKind::MediaSession | ItemKind::Wallpaper => crate::SpotlightKind::Content,
    };
    // A borrowed banner sits in its frame like the app's own would; blown up it would be a logo wall.
    let framed = matches!(it.kind, ItemKind::App | ItemKind::Game | ItemKind::Input | ItemKind::Device) || it.framed_art() || lent.is_some();
    let kind = if lent.is_some() { crate::SpotlightKind::App } else { kind };
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
        light_argb: lent.map(|app| app.light()).unwrap_or_else(|| it.light()),
        primary: StageAction {
            label: primary_label.to_string(),
            action: format!("open:{}", it.id),
        },
        secondary,
        explanation: {
            let source_name = it.source.strip_prefix("plugin:").map(|id| index.plugin_name(id)).unwrap_or_else(|| it.source.clone());
            let habit = it.package().and_then(|p| index.habits.get(p));
            lounge_graph::rank::explanation(it, now, hour_local, &source_name, habit, index.clock_24h)
        },
        framed,
        art_aspect: it.art_aspect,
        glow_argb: if framed { Vec::new() } else { lounge_graph::color::glow_stops(&artwork.glow, GLOW_STOPS) },
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

/// "23 minutes left", "1 hour 20 minutes left": durations are spelled out (DESIGN.md §10).
fn left_text(left_ms: i64) -> String {
    let minutes = (left_ms as f64 / 60_000.0).round() as i64;
    if minutes <= 0 {
        return "Almost finished".to_string();
    }
    let (h, m) = (minutes / 60, minutes % 60);
    let hours = counted(h as usize, "hour", "hours", "");
    let mins = counted(m as usize, "minute", "minutes", "");
    match (h, m) {
        (0, _) => format!("{mins} left"),
        (_, 0) => format!("{hours} left"),
        _ => format!("{hours} {mins} left"),
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

pub(crate) fn local_hour(now_ms: i64, utc_offset_minutes: i32) -> u32 {
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
                if let Some(plugin) = plugin_of(target, state.graph.get(target).map(|it| it.source.as_str())) {
                    *inner.opened_plugin.lock() = Some(plugin);
                }
                match state.graph.get(target) {
                    // A schedule entry with nothing of its own to open plays through its channel.
                    Some(it) if it.deep_link == DeepLink::None && it.channel_id.is_some() => {
                        it.channel_id.and_then(|c| state.graph.channel_for(c)).map(|ch| ch.deep_link.clone()).unwrap_or(DeepLink::None)
                    }
                    Some(it) => it.deep_link.clone(),
                    None => inner.search_links.lock().get(target).cloned().ok_or_else(unknown)?,
                }
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

/// The plugin an opened item came from: a home item's source, or a plugin search tile's id.
fn plugin_of(target: &str, source: Option<&str>) -> Option<String> {
    match source {
        Some(source) => source.strip_prefix("plugin:").map(str::to_string),
        None => target.strip_prefix("search:").and_then(|t| t.split_once(':')).map(|(plugin, _)| plugin.to_string()),
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
        DeepLink::LaunchApp { package, activity } => {
            inner.platform.launch_app(package.clone(), activity).map_err(platform_err)?;
            // An app opened from here counts as used even when usage access is off.
            inner.enqueue_job(crate::worker::Job::Usage(vec![crate::AppUsage { package, last_used_ms: now_ms() }]));
            Ok(())
        }
        DeepLink::ViewUri(uri) => inner.platform.view_uri(uri).map_err(platform_err),
        DeepLink::ViewOrLaunch { uri, package } => match inner.platform.view_uri(uri) {
            Ok(()) => Ok(()),
            Err(_) => inner.platform.launch_app(package, String::new()).map_err(platform_err),
        },
        DeepLink::Play { url, title, start_ms } => inner.platform.play_stream(url, title, start_ms).map_err(platform_err),
        DeepLink::PluginResolve { plugin, action } => {
            let (reply_tx, reply_rx) = crossbeam_channel::bounded(1);
            inner
                .plugins
                .call(crate::plugins::PluginJob::Resolve { plugin: plugin.clone(), action, reply: reply_tx })
                .map_err(|detail| CoreError::Plugin { detail })?;
            let resolved = reply_rx
                .recv_timeout(std::time::Duration::from_secs(15))
                .map_err(|_| CoreError::Plugin { detail: format!("{plugin} did not answer in time") })?
                .map_err(|detail| CoreError::Plugin { detail })?;
            let link = if let Some(url) = resolved.play {
                DeepLink::Play { url, title: resolved.title.unwrap_or_default(), start_ms: resolved.start_ms }
            } else if let (Some(uri), Some(pkg)) = (resolved.view.clone(), resolved.fallback_launch.clone()) {
                DeepLink::ViewOrLaunch { uri, package: pkg }
            } else if let Some(uri) = resolved.view {
                DeepLink::ViewUri(uri)
            } else if let Some(pkg) = resolved.launch {
                DeepLink::LaunchApp { package: pkg, activity: String::new() }
            } else if let Some(query) = resolved.search {
                DeepLink::Control(format!("search:{query}"))
            } else {
                // The plugin did the work itself (told a player on the network to start, say).
                return Ok(());
            };
            dispatch(inner, link)
        }
        DeepLink::None => Ok(()),
        DeepLink::SwitchInput(id) => inner.platform.switch_input(id).map_err(platform_err),
        DeepLink::Tune(uri) => inner.platform.tune(uri).map_err(platform_err),
        DeepLink::Control(cmd) => inner.platform.control(cmd).map_err(platform_err),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> CoreConfig {
        CoreConfig {
            data_dir: String::new(),
            cache_dir: String::new(),
            tier: crate::DeviceTier::B,
            utc_offset_minutes: 0,
            has_tv_controls: false,
            device_name: "TV".into(),
            screen_width: 1920,
            screen_height: 1080,
        }
    }

    fn state(graph: Graph, plugins: Vec<crate::plugins::PluginEntry>) -> State {
        State {
            graph,
            store: lounge_store::Store::open_in_memory().unwrap(),
            revision: 1,
            art_dir: std::path::PathBuf::new(),
            art_threads: 1,
            art_pool: None,
            permissions: crate::Permissions { tv_listings: true, notifications: true, usage: true },
            now_playing: None,
            update: None,
            loaded: true,
            plugins,
            habits: Default::default(),
            playing: Default::default(),
            pending_art: Default::default(),
            last_programs: Vec::new(),
            clock_24h: false,
        }
    }

    /// A resume point from `app`, watched `ago` before now.
    fn resume(id: &str, app: &str, ago: i64) -> Item {
        let mut it = Item::content(id, ItemKind::Resume, "tvprovider", id, DeepLink::ViewUri(format!("app://{id}")));
        it.owner = Some(app.into());
        it.last_used_ms = Some(now_ms() - ago);
        it
    }

    #[test]
    fn continue_watching_starts_after_what_the_stage_shows() {
        let mut g = Graph::default();
        let items = (0..9).map(|i| resume(&format!("wn:{i}"), &format!("tv.app{}", i % 3), i * 60_000)).collect();
        g.replace_source("tvprovider", items);
        let snap = build(&state(g, Vec::new()), &config(), 0);
        let strip: Vec<&str> = snap.stage.strip.iter().map(|t| t.id.as_str()).collect();
        let row = snap.rows.iter().find(|r| r.id == "continue").expect("a row for the rest");
        assert_eq!(strip.len() + row.tiles.len(), 9);
        assert!(row.tiles.iter().all(|t| !strip.contains(&t.id.as_str())), "{strip:?}");

        // With everything on the Stage there is no row at all, and the editor says why.
        let mut g = Graph::default();
        g.replace_source("tvprovider", vec![resume("wn:a", "tv.a", 0), resume("wn:b", "tv.b", 60_000)]);
        let s = state(g, Vec::new());
        assert!(build(&s, &config(), 0).rows.iter().all(|r| r.id != "continue"));
        let info = row_infos(&s, &config()).into_iter().find(|r| r.id == "continue").unwrap();
        assert_eq!((info.count, info.detail.as_str()), (0, "2 titles, all on the Stage"));
    }

    #[test]
    fn a_hidden_app_stays_in_all_apps_and_says_so() {
        let mut g = Graph::default();
        g.replace_source("apps", vec![Item::app("tv.a", "Main", "Alpha"), Item::app("tv.b", "Main", "Beta")]);
        g.hide("app:tv.a");
        let snap = build(&state(g, Vec::new()), &config(), 0);
        let alpha = snap.all_apps.iter().find(|t| t.id == "app:tv.a").expect("still in All apps");
        assert!(alpha.hidden);
        assert_eq!(alpha.subtitle, None);
        assert!(!snap.all_apps.iter().find(|t| t.id == "app:tv.b").unwrap().hidden);
        let apps = snap.rows.iter().find(|r| r.id == "apps").unwrap();
        assert!(apps.tiles.iter().all(|t| t.id != "app:tv.a"));
        assert_eq!(apps.tiles.last().unwrap().subtitle.as_deref(), Some("2 apps"));
    }

    #[test]
    fn a_plugin_is_offered_for_what_its_app_does_not_bring() {
        let plugin = |rows: Vec<lounge_plugin_api::RowSpec>| crate::plugins::PluginEntry {
            id: "tv.lounge.jellyfin".into(),
            manifest: lounge_plugin_api::Manifest { id: "tv.lounge.jellyfin".into(), name: "Jellyfin".into(), for_apps: vec!["org.jellyfin".into()], rows, ..Default::default() },
            enabled: false,
        };
        let rows = vec![lounge_plugin_api::RowSpec { id: "recent".into(), title: "Recently added on Jellyfin".into(), style: lounge_plugin_api::RowStyle::Poster }];
        let offered = |g: Graph, rows: Vec<lounge_plugin_api::RowSpec>| {
            build(&state(g, vec![plugin(rows)]), &config(), 0).rows.into_iter().find(|r| r.id == NOTICE_PLUGINS).map(|r| r.tiles[0].clone())
        };
        let mut g = Graph::default();
        g.replace_source("apps", vec![Item::app("org.jellyfin", "Main", "Jellyfin")]);
        let tile = offered(g.clone(), rows.clone()).expect("offered while the app brings nothing");
        assert_eq!((tile.title.as_str(), tile.subtitle.as_deref()), ("Set up Jellyfin", Some("See what you were watching")));

        // Once the app fills Continue watching itself, the plugin is offered for its rows alone.
        g.replace_source("tvprovider", vec![resume("wn:1", "org.jellyfin", 0)]);
        let tile = offered(g.clone(), rows).expect("offered for its rows");
        assert_eq!(tile.subtitle.as_deref(), Some("Adds recently added"));
        assert!(offered(g, Vec::new()).is_none(), "a plugin with nothing more to add is not offered");
    }

    #[test]
    fn a_plugin_row_whose_refresh_failed_says_it_shows_what_was_kept() {
        let id = "tv.lounge.jellyfin";
        let manifest = lounge_plugin_api::Manifest {
            id: id.into(),
            name: "Jellyfin".into(),
            rows: vec![lounge_plugin_api::RowSpec { id: "recent".into(), title: "Recently added".into(), style: lounge_plugin_api::RowStyle::Poster }],
            ..Default::default()
        };
        let mut g = Graph::default();
        let mut film = Item::content("plugin:tv.lounge.jellyfin:recent:1", ItemKind::Program, "plugin:tv.lounge.jellyfin", "Static Bloom", DeepLink::None);
        film.row = Some("recent".into());
        g.upsert(film);
        let s = state(g, vec![crate::plugins::PluginEntry { id: id.into(), manifest: manifest.clone(), enabled: true }]);
        let row = lounge_store::PluginRow {
            id: id.into(),
            manifest_json: serde_json::to_string(&manifest).unwrap(),
            publisher: String::new(),
            enabled: true,
            built_in: true,
            installed_ms: 0,
            last_refresh_ms: None,
            last_error: None,
            last_success_ms: None,
        };
        s.store.plugin_upsert(&row).unwrap();
        let note = |s: &State| build(s, &config(), 0).rows.into_iter().find(|r| r.id.starts_with("plugin:")).unwrap().note;
        assert_eq!(note(&s), None);
        s.store.plugin_set_refresh(id, 1, Some("Jellyfin didn't answer")).unwrap();
        assert_eq!(note(&s).as_deref(), Some(OFFLINE_NOTE));
    }

    #[test]
    fn a_session_without_pictures_wears_its_apps_banner() {
        let mut g = Graph::default();
        let mut app = Item::app("au.com.seven", "Main", "7plus");
        app.artwork.landscape = Some("/art/seven-land.jpg".into());
        app.artwork.logo = Some("/art/seven-icon.png".into());
        g.upsert(app);
        let mut live = Item::content("session:au.com.seven:racing", ItemKind::MediaSession, "sessions", "Racing.com", DeepLink::None);
        live.owner = Some("au.com.seven".into());
        let mut own = Item::content("session:au.com.seven:news", ItemKind::MediaSession, "sessions", "News", DeepLink::None);
        own.owner = Some("au.com.seven".into());
        own.artwork.landscape = Some("/art/news-land.jpg".into());

        let tile = tile_for(&g, &live, TileKind::Landscape);
        assert_eq!(tile.artwork_path.as_deref(), Some("/art/seven-land.jpg"));
        assert_eq!(tile.icon_path.as_deref(), Some("/art/seven-icon.png"));
        assert!(tile.borrowed_art);
        // A session with pictures of its own keeps them, and the app's own tile is not borrowed.
        let tile = tile_for(&g, &own, TileKind::Landscape);
        assert_eq!(tile.artwork_path.as_deref(), Some("/art/news-land.jpg"));
        assert!(!tile.borrowed_art);
        assert!(!tile_for(&g, g.get("app:au.com.seven").unwrap(), TileKind::AppBanner).borrowed_art);
        // Nothing to borrow when the app is gone.
        let mut orphan = live.clone();
        orphan.owner = Some("au.com.gone".into());
        assert!(tile_for(&g, &orphan, TileKind::Landscape).artwork_path.is_none());
    }

    #[test]
    fn an_item_that_has_gone_is_named_by_where_it_was() {
        let mut g = Graph::default();
        let mut app = Item::app("org.jellyfin.androidtv", "Main", "Jellyfin");
        app.artwork.logo = Some("/art/jellyfin.png".into());
        g.upsert(app);
        let gone = item_ref(&g, "wn:org.jellyfin.androidtv:bca47bff-35a3-b7dc-505d-b96791278d6b");
        assert_eq!((gone.title.as_str(), gone.icon_path.as_deref()), ("A title Jellyfin no longer lists", Some("/art/jellyfin.png")));
        assert_eq!(item_ref(&g, "plugin:tv.lounge.plex:deck:41").title, "A title that is no longer listed");
        assert_eq!(item_ref(&g, "app:com.example.removed").title, "com.example.removed");
        assert_eq!(item_ref(&g, "app:org.jellyfin.androidtv").title, "Jellyfin");
    }

    #[test]
    fn a_move_places_the_row_and_clamps_to_the_ends() {
        let order: Vec<String> = ["a", "b", "c"].iter().map(|s| s.to_string()).collect();
        assert_eq!(moved(order.clone(), "c", 0), vec!["c", "a", "b"]);
        assert_eq!(moved(order.clone(), "a", 99), vec!["b", "c", "a"]);
        assert_eq!(moved(order.clone(), "new", 1), vec!["a", "new", "b", "c"]);
        assert_eq!(moved(order, "b", 1), vec!["a", "b", "c"]);
    }

    #[test]
    fn counts_read_as_sentences() {
        assert_eq!(counted(0, "app", "apps", "Nothing yet"), "Nothing yet");
        assert_eq!(counted(1, "app", "apps", "Nothing yet"), "1 app");
        assert_eq!(counted(14, "app", "apps", "Nothing yet"), "14 apps");
    }

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
        assert_eq!(left_text(95 * 60_000), "1 hour 35 minutes left");
        assert_eq!(left_text(120 * 60_000), "2 hours left");
        assert_eq!(left_text(61 * 60_000), "1 hour 1 minute left");
    }

    #[test]
    fn a_plugin_s_rows_read_as_what_it_adds() {
        let row = |title: &str| lounge_plugin_api::RowSpec { id: title.to_lowercase(), title: title.into(), style: lounge_plugin_api::RowStyle::Poster };
        let mut m = lounge_plugin_api::Manifest { name: "Jellyfin".into(), ..Default::default() };
        assert_eq!(what_it_adds(&m), None);
        m.rows = vec![row("Recently added on Jellyfin"), row("Favourites on Jellyfin")];
        assert_eq!(what_it_adds(&m).as_deref(), Some("Adds recently added and favourites"));
        m.rows.push(row("YouTube picks"));
        assert_eq!(what_it_adds(&m).as_deref(), Some("Adds recently added, favourites and YouTube picks"));
        assert_eq!(row_words("Jellyfin", "Jellyfin"), "jellyfin");
    }

    #[test]
    fn an_episode_in_a_row_says_which_one() {
        let g = Graph::default();
        let mut it = Item::content("plugin:trakt:next:1", ItemKind::Program, "plugin:trakt", "The Bear", DeepLink::None);
        it.subtitle = Some("Honeydew".into());
        it.season = Some("2".into());
        it.episode = Some("4".into());
        assert_eq!(tile_for(&g, &it, TileKind::Landscape).subtitle.as_deref(), Some("S2 E4, Honeydew"));
        it.season = None;
        assert_eq!(tile_for(&g, &it, TileKind::Landscape).subtitle.as_deref(), Some("Honeydew"));
    }

    #[test]
    fn an_opened_item_names_its_plugin() {
        assert_eq!(plugin_of("plugin:jellyfin:resume:1", Some("plugin:jellyfin")).as_deref(), Some("jellyfin"));
        assert_eq!(plugin_of("search:plex:latest:9", None).as_deref(), Some("plex"));
        assert_eq!(plugin_of("app:com.netflix.ninja", Some("apps")), None);
    }

    #[test]
    fn a_tile_that_only_searches_says_so() {
        let g = Graph::default();
        let trakt = Item::content("plugin:trakt:next:1", ItemKind::Program, "plugin:trakt", "Severance", DeepLink::Control("search:Severance".into()));
        assert_eq!(tile_for(&g, &trakt, TileKind::Landscape).action, "control:search:Severance");
        let film = Item::content("plugin:jellyfin:latest:1", ItemKind::Program, "plugin:jellyfin", "Static Bloom", DeepLink::ViewUri("intent:#Intent;end".into()));
        assert_eq!(tile_for(&g, &film, TileKind::Poster).action, "open:plugin:jellyfin:latest:1");
    }

    #[test]
    fn a_tile_to_read_has_no_action() {
        let g = Graph::default();
        let today = Item::content("plugin:weather:day:0", ItemKind::Program, "plugin:weather", "17° today", DeepLink::None);
        assert_eq!(tile_for(&g, &today, TileKind::Compact).action, "");
        // A programme on a channel plays through it, and a session is still opened by its id.
        let mut airing = Item::content("plugin:tw:prog", ItemKind::Program, "plugin:tw", "Speedruns", DeepLink::None);
        airing.channel_id = Some(-7);
        assert_eq!(tile_for(&g, &airing, TileKind::Landscape).action, "open:plugin:tw:prog");
        let session = Item::content("session:au.com.seven:news", ItemKind::MediaSession, "sessions", "News", DeepLink::None);
        assert_eq!(tile_for(&g, &session, TileKind::Landscape).action, "open:session:au.com.seven:news");
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
        let tile = live_tile(&g, g.airing_index(30 * 60_000).get(&2).copied(), ch, 30 * 60_000);
        assert_eq!(tile.title, "News");
        assert_eq!(tile.subtitle.as_deref(), Some("30 minutes left"));
        assert_eq!(tile.progress, Some(0.5));
        assert_eq!(tile.badge, Some(Badge::Live));
    }

    #[test]
    fn a_channel_s_card_fills_its_tile_instead_of_a_corner() {
        let channel = |card: Option<&str>| {
            let mut ch = Item::content("chan:2", ItemKind::Channel, "tvprovider", "Nosey", DeepLink::Tune("a".into()));
            ch.flags.is_live = true;
            ch.artwork.logo = Some("/art/nosey-logo.jpg".into());
            ch.artwork.landscape = card.map(str::to_string);
            ch
        };
        let g = Graph::default();
        // A small mark stays in the corner of a tile with no picture.
        let tile = live_tile(&g, None, &channel(None), 0);
        assert_eq!((tile.artwork_path.as_deref(), tile.icon_path.as_deref()), (None, Some("/art/nosey-logo.jpg")));
        // A card is the picture, lent to whatever is on, and is not repeated in the corner. With
        // nothing known to be on, the tile's title is the channel's name, which the card says.
        let tile = live_tile(&g, None, &channel(Some("/art/nosey-land.jpg")), 0);
        assert_eq!((tile.artwork_path.as_deref(), tile.icon_path.as_deref(), tile.borrowed_art), (Some("/art/nosey-land.jpg"), None, true));
        assert_eq!((tile.title.as_str(), tile.art_title.as_deref()), ("Nosey", Some("")));
        // The title stays whole; over the card the tile says only what the card does not.
        let on_now = |title: &str| {
            let prog = Item::content("prog:2", ItemKind::Program, "tvprovider", title, DeepLink::Tune("a".into()));
            let tile = live_tile(&g, Some(&prog), &channel(Some("/art/nosey-land.jpg")), 0);
            assert_eq!(tile.title, title);
            tile.art_title
        };
        assert_eq!(on_now("Nosey: Lie Detector Drama").as_deref(), Some("Lie Detector Drama"));
        assert_eq!(on_now("NOSEY - S2 E5").as_deref(), Some("S2 E5"));
        assert_eq!(on_now("nosey").as_deref(), Some(""));
        assert_eq!(on_now("Nosey:").as_deref(), Some(""));
        // A programme of another name is said in full, and so is one that only begins alike.
        for other in ["Lie Detector Drama", "Nosey Presents", "Nosey's Best", "Noseybonk", "No"] {
            assert_eq!(on_now(other), None, "{other}");
        }
        // A name of several words is its own, so a space is enough to set what follows apart.
        assert_eq!(past_the_name("The Osbournes Season 2", "The Osbournes"), Some("Season 2"));
        assert_eq!(past_the_name("Wild Kratts", "Wild"), None);
        // After a name of one word, a space sets apart only an episode's number.
        assert_eq!(past_the_name("MythBusters S5 E3", "MythBusters"), Some("S5 E3"));
        assert_eq!(past_the_name("Jail Season 2", "Jail"), Some("Season 2"));
        assert_eq!(past_the_name("Arthur 12", "Arthur"), Some("12"));
        assert_eq!(past_the_name("Arthur Ep. 4", "Arthur"), Some("Ep. 4"));
        for own in ["Survivor Stories", "Evil Lives Here", "Storage Wars"] {
            assert_eq!(past_the_name(own, own.split(' ').next().unwrap()), None, "{own}");
        }
        // A programme's own picture takes the tile, with the channel's logo over it.
        let mut prog = Item::content("prog:1", ItemKind::Program, "tvprovider", "Judge Nosey", DeepLink::Tune("a".into()));
        prog.artwork.landscape = Some("/art/judge-land.jpg".into());
        prog.title = "Nosey: Judge Nosey".into();
        let tile = live_tile(&g, Some(&prog), &channel(Some("/art/nosey-land.jpg")), 0);
        assert_eq!((tile.artwork_path.as_deref(), tile.icon_path.as_deref(), tile.borrowed_art), (Some("/art/judge-land.jpg"), Some("/art/nosey-logo.jpg"), false));
        assert_eq!((tile.title.as_str(), tile.art_title), ("Nosey: Judge Nosey", None));
    }
}
