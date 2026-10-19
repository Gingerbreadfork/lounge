//! The content graph: every source normalises into `Item`s, rows are queries
//! over the graph, and ranking is deterministic and explainable.

pub mod color;
pub mod rank;

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

pub use color::{light_from_palette, Palette};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ItemKind {
    App,
    Program,
    Channel,
    Input,
    Device,
    MediaSession,
    Resume,
    Game,
    /// A picture for the screensaver; never a tile.
    Wallpaper,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum DeepLink {
    LaunchApp { package: String, activity: String },
    ViewUri(String),
    /// Open a URI; if nothing handles it, launch the package instead.
    ViewOrLaunch { uri: String, package: String },
    /// A stream URL for whatever video player the device has.
    Play { url: String, title: String },
    /// Ask the plugin what to do when the item is selected.
    PluginResolve { plugin: String, action: String },
    SwitchInput(String),
    Tune(String),
    Control(String),
    /// Nothing happens on select: wallpapers.
    None,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Artwork {
    pub poster: Option<String>,
    pub landscape: Option<String>,
    /// Larger landscape variant for the Stage (640×360 for app banners, up to 1280×720 for key art).
    pub hero: Option<String>,
    /// Tiny thumbnail used as a soft backdrop behind framed artwork.
    pub backdrop: Option<String>,
    pub logo: Option<String>,
    pub palette: Option<Palette>,
    /// The lowered Stage's slice of the key art (1728×200), cut around `focal_y`.
    #[serde(default)]
    pub band: Option<String>,
    /// The full-screen picture of a wallpaper item.
    #[serde(default)]
    pub wallpaper: Option<String>,
    /// Where the picture's interest sits vertically within its 16:9 frame (0 top, 1 bottom).
    #[serde(default = "half")]
    pub focal_y: f32,
}

fn half() -> f32 {
    0.5
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Flags {
    pub is_game: bool,
    pub is_live: bool,
    pub is_new: bool,
    pub has_banner: bool,
    /// Published to Watch Next (any type): belongs in Continue watching even without progress.
    #[serde(default)]
    pub watch_next: bool,
    /// An input or device with a live connection (as opposed to cabled but in standby).
    #[serde(default)]
    pub connected: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Item {
    pub id: String,
    pub kind: ItemKind,
    pub source: String,
    pub title: String,
    pub subtitle: Option<String>,
    pub artwork: Artwork,
    pub progress: Option<f32>,
    pub last_used_ms: Option<i64>,
    pub added_ms: Option<i64>,
    pub airing_start_ms: Option<i64>,
    pub airing_end_ms: Option<i64>,
    pub deep_link: DeepLink,
    pub flags: Flags,
    /// Package of the app this came from, when the deep link does not say.
    #[serde(default)]
    pub owner: Option<String>,
    /// Season and episode as the provider labels them ("2", "4").
    #[serde(default)]
    pub season: Option<String>,
    #[serde(default)]
    pub episode: Option<String>,
    /// Running time of the content, for "minutes left".
    #[serde(default)]
    pub duration_ms: Option<i64>,
    /// TvProvider channel this programme or schedule entry belongs to.
    #[serde(default)]
    pub channel_id: Option<i64>,
    #[serde(default)]
    pub channel_name: Option<String>,
    /// Provider-supplied ordering weight (preview programmes).
    #[serde(default)]
    pub weight: i32,
    /// Source artwork aspect (width / height); 0 when unknown.
    #[serde(default)]
    pub art_aspect: f32,
    /// Row a plugin put this in (its manifest's row id).
    #[serde(default)]
    pub row: Option<String>,
}

impl Item {
    /// A content item (programme, channel, observed playback) with nothing attached yet.
    pub fn content(id: &str, kind: ItemKind, source: &str, title: &str, deep_link: DeepLink) -> Item {
        Item {
            id: id.to_string(),
            kind,
            source: source.to_string(),
            title: title.to_string(),
            subtitle: None,
            artwork: Artwork::default(),
            progress: None,
            last_used_ms: None,
            added_ms: None,
            airing_start_ms: None,
            airing_end_ms: None,
            deep_link,
            flags: Flags::default(),
            owner: None,
            season: None,
            episode: None,
            duration_ms: None,
            channel_id: None,
            channel_name: None,
            weight: 0,
            art_aspect: 0.0,
            row: None,
        }
    }

    pub fn app(package: &str, activity: &str, label: &str) -> Item {
        Item {
            id: format!("app:{package}"),
            kind: ItemKind::App,
            source: "apps".into(),
            title: label.to_string(),
            subtitle: None,
            artwork: Artwork::default(),
            progress: None,
            last_used_ms: None,
            added_ms: None,
            airing_start_ms: None,
            airing_end_ms: None,
            deep_link: DeepLink::LaunchApp {
                package: package.to_string(),
                activity: activity.to_string(),
            },
            flags: Flags::default(),
            owner: Some(package.to_string()),
            season: None,
            episode: None,
            duration_ms: None,
            channel_id: None,
            channel_name: None,
            weight: 0,
            art_aspect: 0.0,
            row: None,
        }
    }

    pub fn package(&self) -> Option<&str> {
        match &self.deep_link {
            DeepLink::LaunchApp { package, .. } => Some(package),
            DeepLink::ViewOrLaunch { package, .. } => Some(package),
            _ => self.owner.as_deref(),
        }
    }

    /// Portrait or square artwork is framed in the Stage; wide artwork goes full-bleed.
    pub fn framed_art(&self) -> bool {
        self.art_aspect > 0.0 && self.art_aspect < 1.2
    }

    pub fn light(&self) -> u32 {
        self.artwork
            .palette
            .as_ref()
            .map(light_from_palette)
            .unwrap_or(color::HOUSELIGHT)
    }
}

/// User-authored layout state that survives re-indexing.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Layout {
    pub hidden: HashSet<String>,
    pub pinned_apps: Vec<String>,
    pub row_order: Vec<String>,
    pub stage_pins: Vec<String>,
    /// Notice tiles ("turn on TV listings") the user dismissed; they stay dismissed.
    #[serde(default)]
    pub dismissed_notices: HashSet<String>,
    /// Preview channels (other apps' rows) the user chose to show.
    #[serde(default)]
    pub enabled_channels: HashSet<i64>,
}

impl Layout {
    pub const DEFAULT_ROWS: [&'static str; 6] = [
        "continue",
        "apps",
        "games",
        "live",
        "tvcontrols",
        "allapps",
    ];

    pub fn row_order_or_default(&self) -> Vec<String> {
        if self.row_order.is_empty() {
            Self::DEFAULT_ROWS.iter().map(|s| s.to_string()).collect()
        } else {
            self.row_order.clone()
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Graph {
    items: HashMap<String, Item>,
    pub layout: Layout,
}

impl Graph {
    pub fn new(layout: Layout) -> Graph {
        Graph {
            items: HashMap::new(),
            layout,
        }
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn get(&self, id: &str) -> Option<&Item> {
        self.items.get(id)
    }

    pub fn items(&self) -> impl Iterator<Item = &Item> {
        self.items.values()
    }

    /// Replace every item from `source` with `items`, keeping items from other sources.
    pub fn replace_source(&mut self, source: &str, items: Vec<Item>) {
        self.items.retain(|_, it| it.source != source);
        for it in items {
            self.items.insert(it.id.clone(), it);
        }
    }

    pub fn upsert(&mut self, item: Item) {
        self.items.insert(item.id.clone(), item);
    }

    pub fn remove(&mut self, id: &str) {
        self.items.remove(id);
    }

    pub fn set_usage(&mut self, package: &str, last_used_ms: i64) {
        let id = format!("app:{package}");
        if let Some(it) = self.items.get_mut(&id) {
            it.last_used_ms = Some(match it.last_used_ms {
                Some(prev) => prev.max(last_used_ms),
                None => last_used_ms,
            });
        }
    }

    pub fn hide(&mut self, id: &str) {
        self.layout.hidden.insert(id.to_string());
    }

    pub fn unhide(&mut self, id: &str) {
        self.layout.hidden.remove(id);
    }

    pub fn is_hidden(&self, id: &str) -> bool {
        self.layout.hidden.contains(id)
    }

    pub fn pin_app(&mut self, package: &str) {
        let p = package.to_string();
        if !self.layout.pinned_apps.contains(&p) {
            self.layout.pinned_apps.push(p);
        }
    }

    pub fn unpin_app(&mut self, package: &str) {
        self.layout.pinned_apps.retain(|p| p != package);
    }

    pub fn move_app(&mut self, package: &str, to_index: usize) {
        let mut pins = self.layout.pinned_apps.clone();
        if !pins.iter().any(|p| p == package) {
            pins.push(package.to_string());
        }
        pins.retain(|p| p != package);
        let idx = to_index.min(pins.len());
        pins.insert(idx, package.to_string());
        self.layout.pinned_apps = pins;
    }

    pub fn set_row_order(&mut self, rows: Vec<String>) {
        self.layout.row_order = rows;
    }

    pub fn reset_layout(&mut self) {
        self.layout = Layout::default();
    }

    /// Apps the user pinned, in their order, followed by the rest by recency then name.
    pub fn your_apps(&self) -> Vec<&Item> {
        let mut out: Vec<&Item> = Vec::new();
        for p in &self.layout.pinned_apps {
            if let Some(it) = self.items.get(&format!("app:{p}")) {
                if !self.is_hidden(&it.id) {
                    out.push(it);
                }
            }
        }
        let mut rest: Vec<&Item> = self
            .items
            .values()
            .filter(|it| it.kind == ItemKind::App && it.flags.has_banner)
            .filter(|it| !self.is_hidden(&it.id))
            .filter(|it| !out.iter().any(|o| o.id == it.id))
            .collect();
        rank::by_recency_then_title(&mut rest);
        out.extend(rest);
        out
    }

    pub fn all_apps(&self) -> Vec<&Item> {
        let mut v: Vec<&Item> = self
            .items
            .values()
            .filter(|it| it.kind == ItemKind::App)
            .filter(|it| !self.is_hidden(&it.id))
            .collect();
        v.sort_by(|a, b| a.title.to_lowercase().cmp(&b.title.to_lowercase()));
        v
    }

    pub fn games(&self) -> Vec<&Item> {
        let mut v: Vec<&Item> = self
            .items
            .values()
            .filter(|it| (it.kind == ItemKind::Game || it.flags.is_game) && !self.is_hidden(&it.id))
            .collect();
        rank::by_recency_then_title(&mut v);
        v
    }

    /// Resume points and Watch Next entries, newest first, one per title.
    pub fn continue_watching(&self) -> Vec<&Item> {
        let mut v: Vec<&Item> = self
            .items
            .values()
            .filter(|it| matches!(it.kind, ItemKind::Resume | ItemKind::MediaSession) || (it.kind == ItemKind::Program && it.flags.watch_next))
            .filter(|it| !self.is_hidden(&it.id))
            .collect();
        rank::by_recency_then_title(&mut v);
        rank::dedupe_by_title(&mut v);
        v
    }

    /// Tuner channels (anything that is not a preview channel), by number then name.
    pub fn channels(&self) -> Vec<&Item> {
        let mut v: Vec<&Item> = self
            .items
            .values()
            .filter(|it| it.kind == ItemKind::Channel && !self.is_hidden(&it.id))
            .collect();
        v.sort_by(|a, b| {
            let na = a.subtitle.as_deref().and_then(channel_number_key);
            let nb = b.subtitle.as_deref().and_then(channel_number_key);
            match (na, nb) {
                (Some(x), Some(y)) => x.total_cmp(&y),
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => std::cmp::Ordering::Equal,
            }
            .then_with(|| a.title.cmp(&b.title))
        });
        v
    }

    /// Channels with something airing now (or with no schedule at all), most recently watched first.
    pub fn live_now(&self, now_ms: i64) -> Vec<&Item> {
        let mut v: Vec<&Item> = self
            .channels()
            .into_iter()
            .filter(|it| match (it.airing_start_ms, it.airing_end_ms) {
                (Some(s), Some(e)) => s <= now_ms && now_ms < e,
                _ => true,
            })
            .collect();
        v.sort_by(|a, b| b.last_used_ms.cmp(&a.last_used_ms));
        v
    }

    /// Schedule entries for a channel in `[from_ms, to_ms)`, in airing order.
    pub fn schedule(&self, channel_id: i64, from_ms: i64, to_ms: i64) -> Vec<&Item> {
        let mut v: Vec<&Item> = self
            .items
            .values()
            .filter(|it| it.kind == ItemKind::Program && it.channel_id == Some(channel_id) && !it.flags.watch_next)
            .filter(|it| match (it.airing_start_ms, it.airing_end_ms) {
                (Some(s), Some(e)) => e > from_ms && s < to_ms,
                _ => false,
            })
            .collect();
        v.sort_by_key(|it| it.airing_start_ms);
        v
    }

    /// What is airing on every channel at `now_ms`, in one pass over the items.
    pub fn airing_index(&self, now_ms: i64) -> HashMap<i64, &Item> {
        let mut out = HashMap::new();
        for it in self.items.values() {
            if it.kind != ItemKind::Program {
                continue;
            }
            let Some(ch) = it.channel_id else { continue };
            if matches!((it.airing_start_ms, it.airing_end_ms), (Some(s), Some(e)) if s <= now_ms && now_ms < e) {
                out.entry(ch).or_insert(it);
            }
        }
        out
    }

    /// Schedule entries for every channel in `[from_ms, to_ms)`, in airing order, in one pass.
    pub fn schedule_index(&self, from_ms: i64, to_ms: i64) -> HashMap<i64, Vec<&Item>> {
        let mut out: HashMap<i64, Vec<&Item>> = HashMap::new();
        for it in self.items.values() {
            if it.kind != ItemKind::Program || it.flags.watch_next {
                continue;
            }
            let Some(ch) = it.channel_id else { continue };
            if matches!((it.airing_start_ms, it.airing_end_ms), (Some(s), Some(e)) if e > from_ms && s < to_ms) {
                out.entry(ch).or_default().push(it);
            }
        }
        for v in out.values_mut() {
            v.sort_by_key(|it| it.airing_start_ms);
        }
        out
    }

    /// What is airing on `channel_id` at `now_ms`, if the schedule says.
    pub fn airing_now(&self, channel_id: i64, now_ms: i64) -> Option<&Item> {
        self.items
            .values()
            .filter(|it| it.kind == ItemKind::Program && it.channel_id == Some(channel_id))
            .find(|it| matches!((it.airing_start_ms, it.airing_end_ms), (Some(s), Some(e)) if s <= now_ms && now_ms < e))
    }

    /// Items a plugin published into one of its rows, by weight then recency.
    pub fn plugin_row(&self, plugin: &str, row: &str) -> Vec<&Item> {
        let source = format!("plugin:{plugin}");
        let mut v: Vec<&Item> = self
            .items
            .values()
            .filter(|it| it.source == source && it.row.as_deref() == Some(row) && !self.is_hidden(&it.id))
            .collect();
        v.sort_by(|a, b| b.weight.cmp(&a.weight).then_with(|| b.added_ms.cmp(&a.added_ms)).then_with(|| a.title.cmp(&b.title)));
        v
    }

    /// Pictures plugins published for the screensaver, by weight then recency.
    pub fn wallpapers(&self) -> Vec<&Item> {
        let mut v: Vec<&Item> = self.items.values().filter(|it| it.kind == ItemKind::Wallpaper && !self.is_hidden(&it.id)).collect();
        v.sort_by(|a, b| b.weight.cmp(&a.weight).then_with(|| b.added_ms.cmp(&a.added_ms)).then_with(|| a.id.cmp(&b.id)));
        v
    }

    pub fn count_source(&self, source: &str) -> usize {
        self.items.values().filter(|it| it.source == source).count()
    }

    /// Preview programmes grouped by channel: (channel id, channel name, programmes by weight).
    pub fn preview_channels(&self) -> Vec<(i64, String, Vec<&Item>)> {
        let mut by_channel: HashMap<i64, (String, Vec<&Item>)> = HashMap::new();
        for it in self.items.values() {
            if it.kind != ItemKind::Program || it.flags.watch_next || it.airing_start_ms.is_some() {
                continue;
            }
            let Some(ch) = it.channel_id else { continue };
            if self.is_hidden(&it.id) {
                continue;
            }
            let entry = by_channel.entry(ch).or_insert_with(|| (it.channel_name.clone().unwrap_or_default(), Vec::new()));
            entry.1.push(it);
        }
        let mut out: Vec<(i64, String, Vec<&Item>)> = by_channel
            .into_iter()
            .map(|(id, (name, mut items))| {
                items.sort_by(|a, b| b.weight.cmp(&a.weight).then_with(|| b.added_ms.cmp(&a.added_ms)).then_with(|| a.title.cmp(&b.title)));
                (id, name, items)
            })
            .collect();
        out.sort_by(|a, b| a.1.cmp(&b.1).then(a.0.cmp(&b.0)));
        out
    }

    /// Inputs and devices: connected first (weight), then as the collector ordered them.
    pub fn devices(&self) -> Vec<&Item> {
        let mut v: Vec<&Item> = self
            .items
            .values()
            .filter(|it| matches!(it.kind, ItemKind::Input | ItemKind::Device) && !self.is_hidden(&it.id))
            .collect();
        v.sort_by(|a, b| b.weight.cmp(&a.weight).then_with(|| a.title.cmp(&b.title)));
        v
    }

    /// Stage candidates: pinned first, then continue-watching, then recent apps, capped at `cap`.
    pub fn stage_candidates(&self, cap: usize) -> Vec<&Item> {
        let mut out: Vec<&Item> = Vec::new();
        for id in &self.layout.stage_pins {
            if let Some(it) = self.items.get(id) {
                if !self.is_hidden(&it.id) {
                    out.push(it);
                }
            }
        }
        for it in self.continue_watching() {
            if out.len() >= cap {
                break;
            }
            if !out.iter().any(|o| o.id == it.id) {
                out.push(it);
            }
        }
        if out.len() < cap {
            let mut recent: Vec<&Item> = self
                .items
                .values()
                .filter(|it| it.kind == ItemKind::App && it.last_used_ms.is_some() && !self.is_hidden(&it.id))
                .collect();
            rank::by_recency_then_title(&mut recent);
            for it in recent {
                if out.len() >= cap {
                    break;
                }
                if !out.iter().any(|o| o.id == it.id) {
                    out.push(it);
                }
            }
        }
        // Without usage data (first run, or usage access not granted) the Stage still has to
        // open on something: the user's apps in their own order.
        if out.len() < cap {
            for it in self.your_apps() {
                if out.len() >= cap {
                    break;
                }
                if !out.iter().any(|o| o.id == it.id) {
                    out.push(it);
                }
            }
        }
        out
    }
}

fn channel_number_key(s: &str) -> Option<f64> {
    let mut parts = s.split(['-', '.']);
    let major: f64 = parts.next()?.trim().parse().ok().filter(|f: &f64| f.is_finite())?;
    let minor: f64 = parts.next().and_then(|m| m.trim().parse().ok()).filter(|f: &f64| f.is_finite()).unwrap_or(0.0);
    Some(major + minor / 1000.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(pkg: &str, label: &str, used: Option<i64>) -> Item {
        let mut it = Item::app(pkg, "Main", label);
        it.last_used_ms = used;
        it.flags.has_banner = true;
        it
    }

    #[test]
    fn wallpapers_are_listed_by_weight_and_hidden_ones_skipped() {
        let mut g = Graph::new(Layout::default());
        for (id, weight) in [("plugin:x:a", 1), ("plugin:x:b", 5), ("plugin:x:c", 3)] {
            let mut it = Item::content(id, ItemKind::Wallpaper, "plugin:x", "Summer", DeepLink::None);
            it.weight = weight;
            g.upsert(it);
        }
        g.hide("plugin:x:c");
        let ids: Vec<&str> = g.wallpapers().iter().map(|it| it.id.as_str()).collect();
        assert_eq!(ids, vec!["plugin:x:b", "plugin:x:a"]);
    }

    #[test]
    fn pinned_apps_come_first_then_recency() {
        let mut g = Graph::default();
        g.replace_source(
            "apps",
            vec![
                app("a", "Alpha", Some(10)),
                app("b", "Bravo", Some(30)),
                app("c", "Charlie", None),
            ],
        );
        g.pin_app("c");
        let ids: Vec<&str> = g.your_apps().iter().map(|i| i.id.as_str()).collect();
        assert_eq!(ids, vec!["app:c", "app:b", "app:a"]);
    }

    #[test]
    fn hidden_items_never_appear() {
        let mut g = Graph::default();
        g.replace_source("apps", vec![app("a", "Alpha", None)]);
        g.hide("app:a");
        assert!(g.your_apps().is_empty());
        assert!(g.all_apps().is_empty());
        g.unhide("app:a");
        assert_eq!(g.all_apps().len(), 1);
    }

    #[test]
    fn move_app_reorders_pins() {
        let mut g = Graph::default();
        g.replace_source("apps", vec![app("a", "A", None), app("b", "B", None), app("c", "C", None)]);
        g.pin_app("a");
        g.pin_app("b");
        g.move_app("c", 0);
        assert_eq!(g.layout.pinned_apps, vec!["c", "a", "b"]);
        g.move_app("a", 99);
        assert_eq!(g.layout.pinned_apps, vec!["c", "b", "a"]);
    }

    #[test]
    fn continue_watching_includes_watch_next_and_dedupes() {
        let mut g = Graph::default();
        let mut a = Item::content("wn:1", ItemKind::Resume, "tvprovider", "The Bear", DeepLink::ViewUri("x".into()));
        a.last_used_ms = Some(20);
        let mut b = Item::content("session:1", ItemKind::MediaSession, "sessions", "The Bear", DeepLink::ViewUri("y".into()));
        b.last_used_ms = Some(10);
        let mut c = Item::content("wn:2", ItemKind::Program, "tvprovider", "Up Next Show", DeepLink::ViewUri("z".into()));
        c.flags.watch_next = true;
        let d = Item::content("prog:9", ItemKind::Program, "tvprovider", "Schedule only", DeepLink::ViewUri("w".into()));
        g.replace_source("tvprovider", vec![a, c, d]);
        g.replace_source("sessions", vec![b]);
        let ids: Vec<&str> = g.continue_watching().iter().map(|i| i.id.as_str()).collect();
        assert_eq!(ids, vec!["wn:1", "wn:2"]);
    }

    #[test]
    fn channels_sort_by_number_and_schedule_finds_now() {
        let mut g = Graph::default();
        let mut c10 = Item::content("chan:10", ItemKind::Channel, "tvprovider", "Ten", DeepLink::Tune("t".into()));
        c10.subtitle = Some("10".into());
        c10.channel_id = Some(10);
        let mut c2 = Item::content("chan:2", ItemKind::Channel, "tvprovider", "ABC", DeepLink::Tune("a".into()));
        c2.subtitle = Some("2".into());
        c2.channel_id = Some(2);
        let mut p = Item::content("prog:1", ItemKind::Program, "tvprovider", "News", DeepLink::Tune("a".into()));
        p.channel_id = Some(2);
        p.airing_start_ms = Some(100);
        p.airing_end_ms = Some(200);
        g.replace_source("tvprovider", vec![c10, c2, p]);
        let ids: Vec<&str> = g.channels().iter().map(|i| i.id.as_str()).collect();
        assert_eq!(ids, vec!["chan:2", "chan:10"]);
        assert_eq!(g.airing_now(2, 150).map(|i| i.title.as_str()), Some("News"));
        assert!(g.airing_now(2, 250).is_none());
        assert_eq!(g.schedule(2, 0, 1000).len(), 1);
    }

    #[test]
    fn replace_source_keeps_other_sources() {
        let mut g = Graph::default();
        g.replace_source("apps", vec![app("a", "A", None)]);
        let mut ch = Item::app("x", "y", "Channel");
        ch.id = "chan:1".into();
        ch.kind = ItemKind::Channel;
        ch.source = "tif".into();
        g.replace_source("tif", vec![ch]);
        g.replace_source("apps", vec![]);
        assert_eq!(g.len(), 1);
        assert!(g.get("chan:1").is_some());
    }
}
