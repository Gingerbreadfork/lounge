//! The public core: the only crate the Kotlin shell sees.
//!
//! Every entry point returns immediately; work runs on the core's worker thread and
//! results arrive through `SnapshotListener`.

mod companion;
mod engine;
mod plugins;
mod worker;

use lounge_graph::Graph;
use lounge_store::Store;
use parking_lot::{Mutex, RwLock};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU64, Ordering};
use std::sync::Arc;

uniffi::setup_scaffolding!();

// ---------------------------------------------------------------------------
// Records and enums that cross the boundary
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum, Serialize, Deserialize)]
pub enum DeviceTier {
    A,
    B,
    C,
}

#[derive(Clone, Debug, uniffi::Record)]
pub struct CoreConfig {
    pub data_dir: String,
    pub cache_dir: String,
    pub tier: DeviceTier,
    pub utc_offset_minutes: i32,
    pub has_tv_controls: bool,
    pub device_name: String,
}

#[derive(Clone, Debug, uniffi::Record)]
pub struct AppRecord {
    pub package: String,
    pub activity: String,
    pub label: String,
    pub banner_path: Option<String>,
    pub icon_path: Option<String>,
    pub last_used_ms: Option<i64>,
    pub installed_ms: i64,
    pub is_game: bool,
    pub is_leanback: bool,
}

#[derive(Clone, Debug, uniffi::Record)]
pub struct AppUsage {
    pub package: String,
    pub last_used_ms: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum, Serialize, Deserialize)]
pub enum ProgramKind {
    /// A Watch Next entry (resume point, next episode, new content, watchlist).
    WatchNext,
    /// A programme on another app's preview channel.
    Preview,
    /// A tuner / TV-input channel.
    Channel,
    /// A schedule entry (EPG) for a channel.
    Schedule,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum, Serialize, Deserialize)]
pub enum WatchNextType {
    Continue,
    Next,
    New,
    Watchlist,
    Unknown,
}

/// One TvProvider row, with artwork already copied to local files by the shell.
#[derive(Clone, Debug, uniffi::Record)]
pub struct ProgramRecord {
    pub id: String,
    pub kind: ProgramKind,
    pub package: String,
    pub title: String,
    pub subtitle: Option<String>,
    pub season: Option<String>,
    pub episode: Option<String>,
    pub watch_next_type: WatchNextType,
    pub position_ms: Option<i64>,
    pub duration_ms: Option<i64>,
    pub engaged_ms: Option<i64>,
    pub poster_path: Option<String>,
    pub poster_is_portrait: bool,
    pub thumbnail_path: Option<String>,
    pub logo_path: Option<String>,
    pub intent_uri: Option<String>,
    pub channel_id: Option<i64>,
    pub channel_name: Option<String>,
    pub channel_number: Option<String>,
    pub input_id: Option<String>,
    pub start_ms: Option<i64>,
    pub end_ms: Option<i64>,
    pub weight: i32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum, Serialize, Deserialize)]
pub enum PlaybackStatus {
    Playing,
    Paused,
    Stopped,
}

/// One active media session as the shell sees it.
#[derive(Clone, Debug, uniffi::Record)]
pub struct SessionRecord {
    pub package: String,
    pub app_label: String,
    pub title: Option<String>,
    pub subtitle: Option<String>,
    pub art_path: Option<String>,
    pub duration_ms: Option<i64>,
    pub position_ms: Option<i64>,
    pub status: PlaybackStatus,
    pub can_pause: bool,
    pub can_play: bool,
    pub can_skip_next: bool,
    pub can_skip_previous: bool,
    pub updated_ms: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum, Serialize, Deserialize)]
pub enum InputState {
    Connected,
    Standby,
    Disconnected,
}

/// One TV input from `TvInputManager`: HDMI/AV ports (pass-through) and tuners.
#[derive(Clone, Debug, uniffi::Record)]
pub struct InputRecord {
    pub id: String,
    pub label: String,
    /// The name the user or CEC gave it ("PlayStation 5"), if any.
    pub custom_label: Option<String>,
    /// hdmi, av, tuner or other.
    pub kind: String,
    pub hdmi_port: Option<i32>,
    pub state: InputState,
    pub package: String,
    pub passthrough: bool,
}

/// Which optional accesses the user has granted; drives the notice tiles.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, uniffi::Record, Serialize, Deserialize)]
pub struct Permissions {
    pub tv_listings: bool,
    pub notifications: bool,
    pub usage: bool,
}

/// What is playing right now, for the top bar chip and the Now Playing card.
#[derive(Clone, Debug, PartialEq, uniffi::Record, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NowPlaying {
    pub package: String,
    pub app_label: String,
    pub title: String,
    pub subtitle: Option<String>,
    pub art_path: Option<String>,
    pub light_argb: u32,
    pub duration_ms: Option<i64>,
    pub position_ms: Option<i64>,
    pub status: PlaybackStatus,
    pub can_pause: bool,
    pub can_play: bool,
    pub can_skip_next: bool,
    pub can_skip_previous: bool,
    pub updated_ms: i64,
}

/// One programme in the guide grid.
#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct GuideProgramme {
    pub id: String,
    pub title: String,
    pub subtitle: Option<String>,
    pub start_ms: i64,
    pub end_ms: i64,
    pub is_now: bool,
}

/// One channel row in the guide grid, programmes in airing order within the requested window.
#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct GuideChannel {
    pub id: String,
    pub name: String,
    pub number: Option<String>,
    pub logo_path: Option<String>,
    pub light_argb: u32,
    pub action: String,
    pub programmes: Vec<GuideProgramme>,
}

#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct Guide {
    pub from_ms: i64,
    pub to_ms: i64,
    pub now_ms: i64,
    pub channels: Vec<GuideChannel>,
}

/// One setting a plugin asks for, with its current value (secrets only say whether one is set).
#[derive(Clone, Debug, PartialEq, uniffi::Record, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginSettingInfo {
    pub key: String,
    pub label: String,
    /// text, url, password, number or toggle.
    pub kind: String,
    pub required: bool,
    pub help: Option<String>,
    pub value: Option<String>,
    pub has_value: bool,
}

#[derive(Clone, Debug, PartialEq, uniffi::Record, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginInfo {
    pub id: String,
    pub name: String,
    pub version: String,
    pub description: String,
    /// Hex Ed25519 public key the bundle was signed with; pinned on first install.
    pub publisher: String,
    pub publisher_name: String,
    pub kinds: Vec<String>,
    pub fetch_hosts: Vec<String>,
    pub enabled: bool,
    pub built_in: bool,
    /// Every required setting has a value.
    pub configured: bool,
    pub refresh_minutes: u32,
    pub last_refresh_ms: Option<i64>,
    pub last_error: Option<String>,
    pub items: u32,
    pub settings: Vec<PluginSettingInfo>,
}

#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct CompanionStatus {
    pub running: bool,
    /// `http://<lan ip>:<port>` while running.
    pub url: Option<String>,
    pub port: u16,
    pub paired: u32,
}

/// What the TV shows to pair a phone: a code to type and a link (as a QR PNG) that pairs by itself.
#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct PairingInfo {
    pub code: String,
    pub url: String,
    pub qr_png_path: String,
    pub expires_ms: i64,
}

/// A preview channel another app publishes, for the channels picker.
#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct ChannelInfo {
    pub id: i64,
    pub name: String,
    pub package: String,
    pub enabled: bool,
    pub programmes: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum, Serialize, Deserialize)]
pub enum TileKind {
    Poster,
    Landscape,
    AppBanner,
    Compact,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum, Serialize, Deserialize)]
pub enum Badge {
    Live,
    New,
    Connected,
    Controller,
}

#[derive(Clone, Debug, PartialEq, uniffi::Record, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Tile {
    pub id: String,
    pub kind: TileKind,
    pub title: String,
    pub subtitle: Option<String>,
    pub artwork_path: Option<String>,
    pub icon_path: Option<String>,
    pub light_argb: u32,
    pub progress: Option<f32>,
    pub badge: Option<Badge>,
    pub action: String,
    pub pinned: bool,
    pub stage_pinned: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum, Serialize, Deserialize)]
pub enum RowSize {
    S,
    M,
    L,
}

#[derive(Clone, Debug, PartialEq, uniffi::Record, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Row {
    pub id: String,
    pub title: String,
    pub kind: TileKind,
    pub size: RowSize,
    pub badge: Option<Badge>,
    pub tiles: Vec<Tile>,
}

#[derive(Clone, Debug, PartialEq, uniffi::Record, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StageAction {
    pub label: String,
    pub action: String,
}

/// What the spotlight is, so the Stage can compose its artwork appropriately: an app banner
/// is framed at native size, key art goes full-bleed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum, Serialize, Deserialize)]
pub enum SpotlightKind {
    App,
    Game,
    Content,
    Channel,
    Device,
}

#[derive(Clone, Debug, PartialEq, uniffi::Record, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StageSpotlight {
    pub id: String,
    pub kind: SpotlightKind,
    pub category: String,
    pub title: String,
    pub lines: Vec<String>,
    pub progress: Option<f32>,
    pub artwork_path: Option<String>,
    pub backdrop_path: Option<String>,
    pub icon_path: Option<String>,
    pub light_argb: u32,
    pub primary: StageAction,
    pub secondary: Option<StageAction>,
    pub explanation: Option<String>,
    /// Framed artwork sits at its own size over a soft backdrop; otherwise it fills the Stage.
    #[serde(default)]
    pub framed: bool,
    /// Source artwork aspect (width / height); 0 when unknown.
    #[serde(default)]
    pub art_aspect: f32,
    /// The lowered Stage's slice of the key art, cut at index time around `focal_y`.
    #[serde(default)]
    pub band_path: Option<String>,
    /// Where the picture's interest sits vertically in its 16:9 frame (0 top, 1 bottom).
    #[serde(default = "half")]
    pub focal_y: f32,
    /// Where OK will take you: the app or plugin that has this, or the channel.
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub source_icon_path: Option<String>,
}

fn half() -> f32 {
    0.5
}

#[derive(Clone, Debug, PartialEq, uniffi::Record, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Stage {
    pub spotlights: Vec<StageSpotlight>,
    pub strip: Vec<Tile>,
}

#[derive(Clone, Debug, PartialEq, uniffi::Record, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HomeSnapshot {
    pub revision: u64,
    pub generated_at_ms: i64,
    pub stage: Stage,
    pub rows: Vec<Row>,
    #[serde(default)]
    pub now_playing: Option<NowPlaying>,
    /// Pictures plugins published for the screensaver, prepared like any artwork.
    #[serde(default)]
    pub wallpapers: Vec<Wallpaper>,
}

/// One screensaver picture from a plugin: the prepared file, what it shows and who sent it.
#[derive(Clone, Debug, PartialEq, uniffi::Record, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Wallpaper {
    pub artwork_path: String,
    pub title: String,
    pub source: String,
}

impl HomeSnapshot {
    pub fn empty() -> HomeSnapshot {
        HomeSnapshot {
            revision: 0,
            generated_at_ms: 0,
            stage: Stage {
                spotlights: Vec::new(),
                strip: Vec::new(),
            },
            rows: Vec::new(),
            now_playing: None,
            wallpapers: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, uniffi::Enum)]
pub enum LayoutCommand {
    HideItem { id: String },
    UnhideItem { id: String },
    PinApp { package: String },
    UnpinApp { package: String },
    MoveApp { package: String, to_index: u32 },
    PinToStage { id: String },
    UnpinFromStage { id: String },
    SetRowOrder { row_ids: Vec<String> },
    ResetLayout,
    DismissNotice { id: String },
    EnableChannel { channel_id: i64 },
    DisableChannel { channel_id: i64 },
}

#[derive(Clone, Debug, uniffi::Record)]
pub struct Diagnostics {
    pub items: u32,
    pub revision: u64,
    pub ffi_calls: u64,
    pub last_ingest_ms: i64,
    pub last_build_micros: u64,
    pub db_bytes: u64,
    pub queue_depth: u32,
}

#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum CoreError {
    #[error("storage: {detail}")]
    Storage { detail: String },
    #[error("unknown action: {action}")]
    UnknownAction { action: String },
    #[error("platform: {detail}")]
    Platform { detail: String },
    #[error("plugin: {detail}")]
    Plugin { detail: String },
    #[error("companion: {detail}")]
    Companion { detail: String },
}

impl From<lounge_store::StoreError> for CoreError {
    fn from(e: lounge_store::StoreError) -> Self {
        CoreError::Storage {
            detail: e.to_string(),
        }
    }
}

#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum PlatformError {
    #[error("{detail}")]
    Failed { detail: String },
}

impl From<uniffi::UnexpectedUniFFICallbackError> for PlatformError {
    fn from(e: uniffi::UnexpectedUniFFICallbackError) -> Self {
        PlatformError::Failed {
            detail: e.to_string(),
        }
    }
}

// ---------------------------------------------------------------------------
// Foreign traits implemented by the Kotlin shell
// ---------------------------------------------------------------------------

#[uniffi::export(with_foreign)]
pub trait Platform: Send + Sync {
    fn launch_app(&self, package: String, activity: String) -> Result<(), PlatformError>;
    fn view_uri(&self, uri: String) -> Result<(), PlatformError>;
    fn switch_input(&self, input_id: String) -> Result<(), PlatformError>;
    fn tune(&self, channel_uri: String) -> Result<(), PlatformError>;
    fn control(&self, command: String) -> Result<(), PlatformError>;
    fn open_app_info(&self, package: String) -> Result<(), PlatformError>;
    /// Transport command for `package`'s media session: play-pause, play, pause, next, previous, stop.
    fn media_command(&self, package: String, command: String) -> Result<(), PlatformError>;
    /// Ask the user for an optional access: tv_listings, notifications, usage.
    fn request_permission(&self, name: String) -> Result<(), PlatformError>;
    /// Play a stream URL in whatever video player the device has.
    fn play_stream(&self, url: String, title: String) -> Result<(), PlatformError>;
    /// A remote key from the companion: up, down, left, right, center, back, home, menu,
    /// play_pause, rewind, fast_forward, volume_up, volume_down, mute.
    fn send_key(&self, key: String) -> Result<(), PlatformError>;
    /// Text from the companion into the TV's focused field.
    fn type_text(&self, text: String) -> Result<(), PlatformError>;
}

#[uniffi::export(with_foreign)]
pub trait SnapshotListener: Send + Sync {
    fn on_snapshot(&self, snapshot: HomeSnapshot);
}

// ---------------------------------------------------------------------------
// The core object
// ---------------------------------------------------------------------------

pub(crate) struct Inner {
    pub config: CoreConfig,
    pub platform: Arc<dyn Platform>,
    pub state: Mutex<State>,
    pub snapshot: RwLock<HomeSnapshot>,
    /// The snapshot as persisted, so the companion never serialises it a second time.
    pub snapshot_json: Mutex<String>,
    pub listeners: Mutex<Vec<Arc<dyn SnapshotListener>>>,
    pub ffi_calls: AtomicU64,
    pub screen_on: AtomicBool,
    pub last_ingest_ms: AtomicU64,
    pub last_build_micros: AtomicU64,
    pub tx: crossbeam_channel::Sender<worker::Job>,
    pub queue_depth: AtomicU64,
    pub loaded: parking_lot::Condvar,
    pub utc_offset_minutes: AtomicI32,
    pub plugins: Arc<plugins::PluginManager>,
    pub companion: Mutex<Option<lounge_companion::Companion>>,
    pub started_ms: i64,
}

impl Inner {
    pub(crate) fn enqueue_job(&self, job: worker::Job) {
        self.queue_depth.fetch_add(1, Ordering::Relaxed);
        if self.tx.send(job).is_err() {
            tracing::error!("worker thread is gone; job dropped");
        }
    }
}

pub(crate) struct State {
    pub graph: Graph,
    pub store: Store,
    pub revision: u64,
    pub art_dir: PathBuf,
    /// How many pictures prepare at once: each one holds its full-size decode while it works.
    pub art_threads: usize,
    pub permissions: Permissions,
    pub now_playing: Option<NowPlaying>,
    /// Set once the worker has loaded the stored items into the graph.
    pub loaded: bool,
    /// Installed plugins with their manifests, for rows and refresh scheduling.
    pub plugins: Vec<plugins::PluginEntry>,
}

#[derive(uniffi::Object)]
pub struct Core {
    inner: Arc<Inner>,
}

#[uniffi::export]
impl Core {
    /// Open (or create) the database under `config.data_dir`, load the persisted snapshot so
    /// the first frame has content, and start the worker thread.
    #[uniffi::constructor]
    pub fn open(config: CoreConfig, platform: Arc<dyn Platform>) -> Result<Arc<Self>, CoreError> {
        init_logging();
        let db_path = PathBuf::from(&config.data_dir).join("lounge.db");
        let store = Store::open(&db_path)?;
        let layout = store.load_layout()?;
        // Only the persisted snapshot is needed for the first frame; the graph itself is loaded
        // by the worker as its first job, so opening the core costs one small read.
        let graph = Graph::new(layout);
        let (revision, snapshot) = match store.load_snapshot()? {
            Some((rev, json)) => match serde_json::from_str::<HomeSnapshot>(&json) {
                Ok(s) => (rev, s),
                Err(e) => {
                    tracing::warn!("persisted snapshot unreadable, rebuilding: {e}");
                    (rev, HomeSnapshot::empty())
                }
            },
            None => (0, HomeSnapshot::empty()),
        };
        let art_dir = lounge_artwork::out_dir_for(&PathBuf::from(&config.cache_dir));
        let art_threads = match config.tier { DeviceTier::A => 2, _ => 1 };
        let plugin_entries = plugins::entries_from_rows(&store.plugins()?);
        let (plugin_manager, plugin_rx, plugin_services) =
            plugins::PluginManager::new(&PathBuf::from(&config.data_dir), &PathBuf::from(&config.cache_dir));
        let utc_offset = config.utc_offset_minutes;

        let (tx, rx) = crossbeam_channel::unbounded();
        let inner = Arc::new(Inner {
            config,
            platform,
            state: Mutex::new(State {
                graph,
                store,
                revision,
                art_dir,
                art_threads,
                permissions: Permissions::default(),
                now_playing: None,
                loaded: false,
                plugins: plugin_entries,
            }),
            snapshot: RwLock::new(snapshot),
            snapshot_json: Mutex::new(String::new()),
            listeners: Mutex::new(Vec::new()),
            ffi_calls: AtomicU64::new(0),
            screen_on: AtomicBool::new(true),
            last_ingest_ms: AtomicU64::new(0),
            last_build_micros: AtomicU64::new(0),
            tx,
            queue_depth: AtomicU64::new(0),
            loaded: parking_lot::Condvar::new(),
            utc_offset_minutes: AtomicI32::new(utc_offset),
            plugins: plugin_manager,
            companion: Mutex::new(None),
            started_ms: now_ms(),
        });
        inner.queue_depth.fetch_add(1, Ordering::Relaxed);
        let _ = inner.tx.send(worker::Job::LoadGraph);
        worker::spawn(Arc::clone(&inner), rx);
        plugins::spawn(Arc::clone(&inner), plugin_rx, plugin_services);
        tracing::info!("core opened, revision {}", inner.state.lock().revision);
        Ok(Arc::new(Core { inner }))
    }

    /// Register for snapshots. The current snapshot is delivered immediately.
    pub fn add_listener(&self, listener: Arc<dyn SnapshotListener>) {
        self.count();
        let current = self.inner.snapshot.read().clone();
        listener.on_snapshot(current);
        self.inner.listeners.lock().push(listener);
    }

    pub fn snapshot(&self) -> HomeSnapshot {
        self.count();
        self.inner.snapshot.read().clone()
    }

    /// Replace the complete set of installed apps.
    pub fn ingest_apps(&self, apps: Vec<AppRecord>) {
        self.count();
        self.enqueue(worker::Job::IngestApps(apps));
    }

    pub fn report_usage(&self, usage: Vec<AppUsage>) {
        self.count();
        self.enqueue(worker::Job::Usage(usage));
    }

    /// Replace everything read from the TV provider.
    pub fn ingest_programs(&self, programs: Vec<ProgramRecord>) {
        self.count();
        self.enqueue(worker::Job::IngestPrograms(programs));
    }

    /// Replace the set of TV inputs.
    pub fn ingest_inputs(&self, inputs: Vec<InputRecord>) {
        self.count();
        self.enqueue(worker::Job::IngestInputs(inputs));
    }

    /// The current media sessions; observed playback becomes resume items.
    pub fn ingest_sessions(&self, sessions: Vec<SessionRecord>) {
        self.count();
        self.enqueue(worker::Job::IngestSessions(sessions));
    }

    pub fn set_permissions(&self, permissions: Permissions) {
        self.count();
        self.enqueue(worker::Job::Permissions(permissions));
    }

    /// The guide grid: every tuner channel with its schedule for `hours` from `from_ms`.
    pub fn guide(&self, from_ms: i64, hours: u32) -> Guide {
        self.count();
        engine::wait_for_graph(&self.inner);
        let now = now_ms();
        let to_ms = from_ms + hours as i64 * 3_600_000;
        let state = self.inner.state.lock();
        let schedules = state.graph.schedule_index(from_ms, to_ms);
        let channels = state
            .graph
            .channels()
            .into_iter()
            .map(|ch| GuideChannel {
                id: ch.id.clone(),
                name: ch.title.clone(),
                number: ch.subtitle.clone(),
                logo_path: ch.artwork.logo.clone(),
                light_argb: ch.light(),
                action: format!("open:{}", ch.id),
                programmes: ch
                    .channel_id
                    .and_then(|cid| schedules.get(&cid))
                    .map(|progs| {
                        progs
                            .iter()
                            .map(|p| GuideProgramme {
                                id: p.id.clone(),
                                title: p.title.clone(),
                                subtitle: p.subtitle.clone(),
                                start_ms: p.airing_start_ms.unwrap_or(from_ms),
                                end_ms: p.airing_end_ms.unwrap_or(to_ms),
                                is_now: matches!((p.airing_start_ms, p.airing_end_ms), (Some(s), Some(e)) if s <= now && now < e),
                            })
                            .collect()
                    })
                    .unwrap_or_default(),
            })
            .collect();
        Guide {
            from_ms,
            to_ms,
            now_ms: now,
            channels,
        }
    }

    /// Preview channels other apps publish, with whether each is shown as a row.
    pub fn available_channels(&self) -> Vec<ChannelInfo> {
        self.count();
        engine::wait_for_graph(&self.inner);
        let state = self.inner.state.lock();
        state
            .graph
            .preview_channels()
            .into_iter()
            .map(|(id, name, items)| ChannelInfo {
                id,
                name: if name.is_empty() { format!("Channel {id}") } else { name },
                package: items.first().and_then(|it| it.owner.clone()).unwrap_or_default(),
                enabled: state.graph.layout.enabled_channels.contains(&id),
                programmes: items.len() as u32,
            })
            .collect()
    }

    pub fn command(&self, command: LayoutCommand) {
        self.count();
        self.enqueue(worker::Job::Command(command));
    }

    /// Dispatch a tile or stage action. Returns an error only for malformed actions;
    /// platform failures are reported by the platform itself.
    pub fn perform(&self, action: String) -> Result<(), CoreError> {
        self.count();
        engine::perform(&self.inner, &action)
    }

    pub fn set_screen_state(&self, on: bool) {
        self.count();
        self.inner.screen_on.store(on, Ordering::Relaxed);
        if on {
            self.enqueue(worker::Job::Rebuild);
        }
    }

    pub fn set_utc_offset_minutes(&self, minutes: i32) {
        self.count();
        self.inner.utc_offset_minutes.store(minutes, Ordering::Relaxed);
        self.enqueue(worker::Job::UtcOffset(minutes));
    }

    // ----- plugins -----

    /// Installed plugins, with settings and status, in install order.
    pub fn plugins(&self) -> Vec<PluginInfo> {
        self.count();
        self.inner.plugins_info()
    }

    /// Verify and install (or update) a `.lgp` bundle. New plugins start disabled.
    pub fn install_plugin(&self, path: String, built_in: bool) -> Result<PluginInfo, CoreError> {
        self.count();
        self.inner.plugin_install(std::path::Path::new(&path), built_in).map_err(|detail| CoreError::Plugin { detail })
    }

    pub fn remove_plugin(&self, id: String) {
        self.count();
        self.inner.plugin_remove(&id);
    }

    pub fn set_plugin_enabled(&self, id: String, enabled: bool) {
        self.count();
        self.inner.plugin_set_enabled(&id, enabled);
    }

    /// Set (or clear with None) one setting; an enabled plugin refreshes with the new value.
    pub fn set_plugin_setting(&self, id: String, key: String, value: Option<String>) {
        self.count();
        self.inner.plugin_set_setting(&id, &key, value);
    }

    pub fn refresh_plugin(&self, id: String) {
        self.count();
        let _ = self.inner.plugins.tx.send(plugins::PluginJob::Refresh(id));
    }

    /// The plugin's recent log lines, oldest first.
    pub fn plugin_log(&self, id: String) -> Vec<String> {
        self.count();
        self.inner.plugins.logs(&id)
    }

    // ----- companion -----

    /// Start the LAN companion server on `port` (0 picks a free one).
    pub fn start_companion(&self, port: u16) -> Result<CompanionStatus, CoreError> {
        self.count();
        companion::start(&self.inner, port).map_err(|detail| CoreError::Companion { detail })?;
        Ok(self.companion_status())
    }

    pub fn stop_companion(&self) {
        self.count();
        *self.inner.companion.lock() = None;
    }

    pub fn companion_status(&self) -> CompanionStatus {
        self.count();
        companion::status(&self.inner)
    }

    /// A fresh pairing code and QR link; valid for five minutes.
    pub fn companion_pairing(&self) -> Result<PairingInfo, CoreError> {
        self.count();
        companion::pairing(&self.inner).map_err(|detail| CoreError::Companion { detail })
    }

    /// Forget every paired phone.
    pub fn companion_forget_all(&self) {
        self.count();
        if let Some(c) = self.inner.companion.lock().as_ref() {
            c.forget_all();
        }
    }

    pub fn diagnostics(&self) -> Diagnostics {
        self.count();
        let state = self.inner.state.lock();
        Diagnostics {
            items: state.graph.len() as u32,
            revision: state.revision,
            ffi_calls: self.inner.ffi_calls.load(Ordering::Relaxed),
            last_ingest_ms: self.inner.last_ingest_ms.load(Ordering::Relaxed) as i64,
            last_build_micros: self.inner.last_build_micros.load(Ordering::Relaxed),
            db_bytes: state.store.db_bytes(),
            queue_depth: self.inner.queue_depth.load(Ordering::Relaxed) as u32,
        }
    }

    pub fn version(&self) -> String {
        env!("CARGO_PKG_VERSION").to_string()
    }
}

impl Core {
    fn count(&self) {
        self.inner.ffi_calls.fetch_add(1, Ordering::Relaxed);
    }

    fn enqueue(&self, job: worker::Job) {
        self.inner.queue_depth.fetch_add(1, Ordering::Relaxed);
        if self.inner.tx.send(job).is_err() {
            tracing::error!("worker thread is gone; job dropped");
        }
    }
}

fn init_logging() {
    use std::sync::Once;
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        #[cfg(target_os = "android")]
        {
            use tracing_subscriber::layer::SubscriberExt;
            let subscriber = tracing_subscriber::registry().with(tracing_android::layer("Lounge").ok());
            let _ = tracing::subscriber::set_global_default(subscriber);
        }
        #[cfg(not(target_os = "android"))]
        {
            let _ = tracing_subscriber::fmt().with_test_writer().try_init();
        }
    });
}

pub(crate) fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}
