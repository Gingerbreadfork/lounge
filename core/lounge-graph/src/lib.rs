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
    /// A stream URL for whatever video player the device has, from `start_ms` for a resume point.
    Play {
        url: String,
        title: String,
        #[serde(default)]
        start_ms: Option<i64>,
    },
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
    /// The lowered Stage's light: six colours read across the key art's focal line, left to right.
    #[serde(default)]
    pub glow: Vec<u32>,
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
    /// A wallpaper shown whole with the room around it, not filling the screen.
    #[serde(default)]
    pub framed: bool,
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

    /// Whether this is something to continue: a resume point, something playing, or a Watch
    /// Next entry.
    pub fn continues(&self) -> bool {
        matches!(self.kind, ItemKind::Resume | ItemKind::MediaSession) || (self.kind == ItemKind::Program && self.flags.watch_next)
    }

    /// Where something to continue comes from: the app it opens in, or its plugin when it opens
    /// in no app. Continue watching can leave a provider out.
    pub fn provider(&self) -> Option<&str> {
        self.package().filter(|p| !p.is_empty()).or_else(|| self.source.starts_with("plugin:").then_some(self.source.as_str()))
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
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
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
    /// Rows the user showed or hid by hand; a row not listed follows its default.
    #[serde(default)]
    pub row_visibility: HashMap<String, bool>,
    /// How many recently opened apps follow the pins in Your apps; zero keeps the row to pins.
    #[serde(default = "default_recent_apps")]
    pub recent_apps: u32,
    /// Apps and plugins left out of Continue watching, each by [Item::provider].
    #[serde(default)]
    pub muted_providers: HashSet<String>,
}

pub const DEFAULT_RECENT_APPS: u32 = 5;

fn default_recent_apps() -> u32 {
    DEFAULT_RECENT_APPS
}

impl Default for Layout {
    fn default() -> Self {
        Layout {
            hidden: HashSet::new(),
            pinned_apps: Vec::new(),
            row_order: Vec::new(),
            stage_pins: Vec::new(),
            dismissed_notices: HashSet::new(),
            enabled_channels: HashSet::new(),
            row_visibility: HashMap::new(),
            recent_apps: DEFAULT_RECENT_APPS,
            muted_providers: HashSet::new(),
        }
    }
}

impl Layout {
    pub const DEFAULT_ROWS: [&'static str; 5] = [
        "continue",
        "apps",
        "games",
        "live",
        "tvcontrols",
    ];

    /// Drop repeats from the ordered lists, keeping each entry's first place; the home keys rows
    /// and tiles by id, so a repeat must never reach it.
    pub fn normalise(&mut self) {
        fn dedupe(v: &mut Vec<String>) {
            let mut seen = HashSet::new();
            v.retain(|x| seen.insert(x.clone()));
        }
        dedupe(&mut self.row_order);
        dedupe(&mut self.pinned_apps);
        dedupe(&mut self.stage_pins);
    }

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
    pub fn new(mut layout: Layout) -> Graph {
        layout.normalise();
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

    pub fn mute_provider(&mut self, provider: &str) {
        self.layout.muted_providers.insert(provider.to_string());
    }

    pub fn unmute_provider(&mut self, provider: &str) {
        self.layout.muted_providers.remove(provider);
    }

    fn is_muted(&self, it: &Item) -> bool {
        it.provider().is_some_and(|p| self.layout.muted_providers.contains(p))
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

    /// Moves an app to `to_index` in Your apps as it is shown. The apps up to the last pinned or
    /// moved one keep their places as pins; pins not shown now, such as an uninstalled app's, stay
    /// at the end so they come back where they were left.
    pub fn move_app(&mut self, package: &str, to_index: usize) {
        let pins = &self.layout.pinned_apps;
        let mut order: Vec<String> = self.your_apps().iter().filter_map(|it| it.id.strip_prefix("app:")).map(str::to_string).collect();
        let stale: Vec<String> = pins.iter().filter(|p| *p != package && !order.contains(p)).cloned().collect();
        order.retain(|p| p != package);
        order.insert(to_index.min(order.len()), package.to_string());
        let last = order.iter().rposition(|p| p == package || pins.contains(p)).unwrap_or(0);
        order.truncate(last + 1);
        order.extend(stale);
        self.layout.pinned_apps = order;
    }

    pub fn set_row_order(&mut self, rows: Vec<String>) {
        self.layout.row_order = rows;
        self.layout.normalise();
    }

    pub fn set_row_shown(&mut self, row_id: &str, shown: bool) {
        self.layout.row_visibility.insert(row_id.to_string(), shown);
    }

    /// The user's own choice for a row, if they made one.
    pub fn row_choice(&self, row_id: &str) -> Option<bool> {
        self.layout.row_visibility.get(row_id).copied()
    }

    pub fn reset_layout(&mut self) {
        self.layout = Layout::default();
    }

    /// Apps the user pinned, in their order, then the most recently opened of the rest, as many
    /// as the layout allows. Games keep to their own row. Without usage data the recent ones
    /// are the first by name, so a new TV's row has something in it.
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
            .filter(|it| it.kind == ItemKind::App && !it.flags.is_game)
            .filter(|it| !self.is_hidden(&it.id))
            .filter(|it| !out.iter().any(|o| o.id == it.id))
            .collect();
        // Apps with a banner first among those never opened, so a bare system app does not fill a
        // new TV's row ahead of the streaming apps.
        rest.sort_by_cached_key(|it| (std::cmp::Reverse(it.last_used_ms), !it.flags.has_banner, it.title.to_lowercase()));
        out.extend(rest.into_iter().take(self.layout.recent_apps as usize));
        out
    }

    /// The apps Your apps holds right now: pinned, and recent behind them.
    pub fn your_apps_counts(&self) -> (usize, usize) {
        let all = self.your_apps();
        let pinned = all.iter().filter(|it| it.package().is_some_and(|p| self.layout.pinned_apps.iter().any(|x| x == p))).count();
        (pinned, all.len() - pinned)
    }

    pub fn all_apps(&self) -> Vec<&Item> {
        let mut v: Vec<&Item> = self
            .items
            .values()
            .filter(|it| it.kind == ItemKind::App)
            .filter(|it| !self.is_hidden(&it.id))
            .collect();
        v.sort_by_cached_key(|it| it.title.to_lowercase());
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

    /// Resume points and Watch Next entries, newest first, one per title, without those from
    /// apps and plugins the user left out.
    pub fn continue_watching(&self) -> Vec<&Item> {
        let mut v: Vec<&Item> = self
            .items
            .values()
            .filter(|it| it.continues() && !self.is_hidden(&it.id) && !self.is_muted(it))
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

    /// Tuner channels, most recently watched first; what each is airing comes from the airing index.
    pub fn live_now(&self) -> Vec<&Item> {
        let mut v = self.channels();
        v.sort_by(|a, b| b.last_used_ms.cmp(&a.last_used_ms));
        v
    }

    /// What is airing on every channel at `now_ms`, in one pass over the items. Where a guide's
    /// entries overlap, the one that started last wins, the same one every rebuild.
    pub fn airing_index(&self, now_ms: i64) -> HashMap<i64, &Item> {
        let mut out: HashMap<i64, &Item> = HashMap::new();
        for it in self.items.values() {
            if it.kind != ItemKind::Program {
                continue;
            }
            let Some(ch) = it.channel_id else { continue };
            if airing_at(it, now_ms) {
                let slot = out.entry(ch).or_insert(it);
                if (it.airing_start_ms, &it.id) > (slot.airing_start_ms, &slot.id) {
                    *slot = it;
                }
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
            if airing_within(it, from_ms, to_ms) {
                out.entry(ch).or_default().push(it);
            }
        }
        for v in out.values_mut() {
            v.sort_by_key(|it| it.airing_start_ms);
        }
        out
    }

    /// The next moment after `now_ms` when a programme starts or ends.
    pub fn next_airing_change(&self, now_ms: i64) -> Option<i64> {
        self.items
            .values()
            .filter(|it| it.kind == ItemKind::Program)
            .flat_map(|it| [it.airing_start_ms, it.airing_end_ms])
            .flatten()
            .filter(|&t| t > now_ms)
            .min()
    }

    /// The channel a schedule entry belongs to.
    pub fn channel_for(&self, channel_id: i64) -> Option<&Item> {
        self.items.values().find(|it| it.kind == ItemKind::Channel && it.channel_id == Some(channel_id))
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

    /// Pictures plugins published for the screensaver, each source's by weight then recency, the
    /// sources taking turns so one with many pictures cannot crowd out the rest.
    pub fn wallpapers(&self) -> Vec<&Item> {
        let mut v: Vec<&Item> = self.items.values().filter(|it| it.kind == ItemKind::Wallpaper && !self.is_hidden(&it.id)).collect();
        v.sort_by(|a, b| b.weight.cmp(&a.weight).then_with(|| b.added_ms.cmp(&a.added_ms)).then_with(|| a.id.cmp(&b.id)));
        let mut by_source: std::collections::BTreeMap<&str, std::collections::VecDeque<&Item>> = Default::default();
        for it in v {
            by_source.entry(it.source.as_str()).or_default().push_back(it);
        }
        let mut out = Vec::new();
        while by_source.values().any(|q| !q.is_empty()) {
            out.extend(by_source.values_mut().filter_map(|q| q.pop_front()));
        }
        out
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
            if out.len() >= cap {
                break;
            }
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
        // open on something: the pinned apps, then banner apps by name, whatever Your apps shows.
        if out.len() < cap {
            let mut fallback: Vec<&Item> = self.your_apps();
            let mut named: Vec<&Item> = self.items.values().filter(|it| it.kind == ItemKind::App && it.flags.has_banner && !it.flags.is_game && !self.is_hidden(&it.id)).collect();
            named.sort_by_cached_key(|it| it.title.to_lowercase());
            fallback.extend(named);
            for it in fallback {
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

/// Whether a schedule entry is on at `now_ms`. One with a start and no end, a live stream, is
/// on from its start until something else is known.
fn airing_at(it: &Item, now_ms: i64) -> bool {
    match (it.airing_start_ms, it.airing_end_ms) {
        (Some(s), Some(e)) => s <= now_ms && now_ms < e,
        (Some(s), None) => s <= now_ms,
        _ => false,
    }
}

/// Whether a schedule entry overlaps `[from_ms, to_ms)`.
fn airing_within(it: &Item, from_ms: i64, to_ms: i64) -> bool {
    match (it.airing_start_ms, it.airing_end_ms) {
        (Some(s), Some(e)) => e > from_ms && s < to_ms,
        (Some(s), None) => s < to_ms,
        _ => false,
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
    fn wallpaper_sources_take_turns() {
        let mut g = Graph::new(Layout::default());
        for (source, n) in [("plugin:immich", 5), ("plugin:art", 2)] {
            for i in 0..n {
                let mut it = Item::content(&format!("{source}:{i}"), ItemKind::Wallpaper, source, "Picture", DeepLink::None);
                it.weight = 300 - i;
                g.upsert(it);
            }
        }
        let ids: Vec<&str> = g.wallpapers().iter().map(|it| it.id.as_str()).collect();
        assert_eq!(ids, ["plugin:art:0", "plugin:immich:0", "plugin:art:1", "plugin:immich:1", "plugin:immich:2", "plugin:immich:3", "plugin:immich:4"]);
    }

    #[test]
    fn a_layout_saved_before_row_visibility_still_loads() {
        let json = r#"{"hidden":[],"pinned_apps":["a"],"row_order":["apps"],"stage_pins":[]}"#;
        let l: Layout = serde_json::from_str(json).unwrap();
        assert!(l.row_visibility.is_empty());
        assert_eq!(l.pinned_apps, vec!["a"]);
    }

    #[test]
    fn a_row_choice_is_remembered() {
        let mut g = Graph::default();
        assert_eq!(g.row_choice("games"), None);
        g.set_row_shown("games", false);
        assert_eq!(g.row_choice("games"), Some(false));
        g.set_row_shown("games", true);
        assert_eq!(g.row_choice("games"), Some(true));
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
    fn your_apps_holds_the_pins_and_then_the_recent_few() {
        let mut g = Graph::default();
        let mut apps: Vec<Item> = (0..6).map(|i| app(&format!("p{i}"), &format!("App {i}"), Some(100 - i as i64))).collect();
        let mut game = app("game", "A Game", Some(1000));
        game.flags.is_game = true;
        apps.push(game);
        let mut plain = Item::app("plain", "Main", "Aardvark");
        plain.last_used_ms = None;
        apps.push(plain);
        g.replace_source("apps", apps);
        g.layout.recent_apps = 3;
        g.pin_app("p5");
        let ids: Vec<&str> = g.your_apps().iter().map(|i| i.id.as_str()).collect();
        // The pin, then the three opened most recently; the game and the never-opened app wait.
        assert_eq!(ids, vec!["app:p5", "app:p0", "app:p1", "app:p2"]);
        assert_eq!(g.your_apps_counts(), (1, 3));
        g.layout.recent_apps = 0;
        assert_eq!(g.your_apps().len(), 1);
        // Never opened: a banner app comes before a bare one with an earlier name.
        g.layout.recent_apps = 10;
        for it in g.items.values_mut() {
            it.last_used_ms = None;
        }
        let ids: Vec<&str> = g.your_apps().iter().map(|i| i.id.as_str()).collect();
        assert_eq!(ids.last(), Some(&"app:plain"));
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
    fn a_move_follows_the_row_as_shown_past_pins_of_missing_apps() {
        let mut g = Graph::default();
        g.replace_source("apps", vec![app("a", "A", None), app("b", "B", None), app("c", "C", None)]);
        g.layout.pinned_apps = vec!["gone".into(), "a".into(), "b".into()];
        g.move_app("a", 1);
        let shown: Vec<&str> = g.your_apps().iter().map(|it| it.id.as_str()).collect();
        assert_eq!(shown, vec!["app:b", "app:a", "app:c"]);
        assert_eq!(g.layout.pinned_apps, vec!["b", "a", "gone"]);
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

    /// A resume point `title` from `provider`, last watched at `used`.
    fn watching(id: &str, provider: &str, title: &str, used: i64) -> Item {
        let mut it = Item::content(id, ItemKind::Resume, "tvprovider", title, DeepLink::ViewUri(id.to_string()));
        it.owner = Some(provider.to_string());
        it.last_used_ms = Some(used);
        it
    }

    #[test]
    fn a_provider_is_an_app_or_a_plugin_that_opens_in_none() {
        assert_eq!(watching("wn:1", "tv.canal", "A", 1).provider(), Some("tv.canal"));
        let mut server = Item::content("plugin:plex:1", ItemKind::Resume, "plugin:plex", "B", DeepLink::ViewOrLaunch { uri: "plex://1".into(), package: "com.plexapp.android".into() });
        assert_eq!(server.provider(), Some("com.plexapp.android"));
        server.deep_link = DeepLink::Control("search:B".into());
        assert_eq!(server.provider(), Some("plugin:plex"));
        // A Watch Next entry that names no app has no provider.
        assert_eq!(watching("wn:2", "", "C", 1).provider(), None);
    }

    #[test]
    fn a_muted_provider_stays_out_of_continue_watching_and_the_stage() {
        let mut g = Graph::default();
        g.replace_source("tvprovider", vec![watching("wn:c1", "tv.canal", "News", 30), watching("wn:c2", "tv.canal", "Severance", 20), watching("wn:p1", "tv.plex", "Severance", 10)]);
        let ids = |g: &Graph| g.continue_watching().iter().map(|i| i.id.clone()).collect::<Vec<_>>();
        assert_eq!(ids(&g), ["wn:c1", "wn:c2"]);
        g.mute_provider("tv.canal");
        // The same title from another app takes the place the muted one held.
        assert_eq!(ids(&g), ["wn:p1"]);
        assert_eq!(g.stage_candidates(6).iter().map(|i| i.id.as_str()).collect::<Vec<_>>(), ["wn:p1"]);
        // A pin is the user's own choice, muted or not.
        g.layout.stage_pins = vec!["wn:c1".into()];
        assert_eq!(g.stage_candidates(6).iter().map(|i| i.id.as_str()).collect::<Vec<_>>(), ["wn:c1", "wn:p1"]);
        g.unmute_provider("tv.canal");
        assert_eq!(ids(&g), ["wn:c1", "wn:c2"]);
    }

    #[test]
    fn a_layout_saved_before_muting_still_loads() {
        let l: Layout = serde_json::from_str(r#"{"hidden":[],"pinned_apps":[],"row_order":[],"stage_pins":[]}"#).unwrap();
        assert!(l.muted_providers.is_empty());
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
        assert_eq!(g.airing_index(150).get(&2).map(|i| i.title.as_str()), Some("News"));
        assert!(!g.airing_index(250).contains_key(&2));
        assert_eq!(g.schedule_index(0, 1000).get(&2).map(Vec::len), Some(1));
    }

    #[test]
    fn a_stream_with_no_end_is_on_from_its_start() {
        let mut g = Graph::default();
        let mut ch = Item::content("chan:tw", ItemKind::Channel, "plugin:tw", "Someone", DeepLink::None);
        ch.channel_id = Some(7);
        let mut live = Item::content("prog:tw", ItemKind::Program, "plugin:tw", "Speedruns", DeepLink::None);
        live.channel_id = Some(7);
        live.airing_start_ms = Some(100);
        g.replace_source("plugin:tw", vec![ch, live]);
        assert!(!g.airing_index(50).contains_key(&7));
        assert_eq!(g.airing_index(5_000_000).get(&7).map(|i| i.title.as_str()), Some("Speedruns"));
        assert_eq!(g.live_now().len(), 1);
        assert_eq!(g.schedule_index(1_000, 2_000).get(&7).map(Vec::len), Some(1));
        assert_eq!(g.next_airing_change(50), Some(100));
        assert_eq!(g.next_airing_change(200), None);
    }

    #[test]
    fn layouts_never_repeat_an_entry() {
        let mut l = Layout { row_order: vec!["apps".into(), "live".into(), "apps".into()], pinned_apps: vec!["a".into(), "a".into()], stage_pins: vec!["x".into(), "y".into(), "x".into()], ..Default::default() };
        l.normalise();
        assert_eq!(l.row_order, vec!["apps", "live"]);
        assert_eq!(l.pinned_apps, vec!["a"]);
        assert_eq!(l.stage_pins, vec!["x", "y"]);
        let mut g = Graph::new(Layout::default());
        g.set_row_order(vec!["a".into(), "a".into()]);
        assert_eq!(g.layout.row_order, vec!["a"]);
    }

    #[test]
    fn stage_pins_respect_the_cap() {
        let mut g = Graph::default();
        let apps: Vec<Item> = (0..5).map(|i| app(&format!("p{i}"), "A", None)).collect();
        g.replace_source("apps", apps);
        g.layout.stage_pins = (0..5).map(|i| format!("app:p{i}")).collect();
        assert_eq!(g.stage_candidates(3).len(), 3);
    }

    #[test]
    fn overlapping_entries_give_the_same_programme_every_time() {
        let mut g = Graph::default();
        let programme = |id: &str, start: i64| {
            let mut p = Item::content(id, ItemKind::Program, "t", id, DeepLink::None);
            p.channel_id = Some(1);
            p.airing_start_ms = Some(start);
            p.airing_end_ms = Some(1000);
            p
        };
        g.replace_source("t", vec![programme("p:a", 0), programme("p:b", 100), programme("p:c", 100)]);
        assert_eq!(g.airing_index(500).get(&1).map(|i| i.id.as_str()), Some("p:c"));
    }

    #[test]
    fn the_next_change_is_the_nearest_boundary() {
        let mut g = Graph::default();
        let mut a = Item::content("p:1", ItemKind::Program, "t", "A", DeepLink::None);
        a.airing_start_ms = Some(100);
        a.airing_end_ms = Some(200);
        let mut b = Item::content("p:2", ItemKind::Program, "t", "B", DeepLink::None);
        b.airing_start_ms = Some(200);
        b.airing_end_ms = Some(400);
        g.replace_source("t", vec![a, b]);
        assert_eq!(g.next_airing_change(150), Some(200));
        assert_eq!(g.next_airing_change(250), Some(400));
        assert_eq!(g.next_airing_change(400), None);
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
