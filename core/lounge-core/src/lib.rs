//! The public core: the only crate the Kotlin shell sees.
//!
//! Most entry points return at once; their work runs on the core's worker thread and the
//! results arrive through `SnapshotListener`. Those that answer a question wait for it: `guide`,
//! `perform`, `available_channels`, `search_plugins`, `install_plugin` and `companion_pairing`,
//! so they are called off the main thread.

mod companion;
mod engine;
mod plugins;
mod screensavers;
mod search;
mod worker;

use lounge_graph::{Graph, Layout};
use lounge_store::{PluginRow, Store, StoreError};
use parking_lot::{Mutex, RwLock};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
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
    /// The panel's own pixels, which can be more than the TV draws its apps at: screensaver
    /// pictures are prepared for the panel. Zero when unknown.
    pub screen_width: u32,
    pub screen_height: u32,
}

impl CoreConfig {
    /// A panel taller than 1080p gets screensaver pictures prepared at 2160p.
    pub fn uhd_wallpapers(&self) -> bool {
        self.screen_height >= 2000
    }
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
    /// What the plugin uses when nothing is set, as the manifest gives it.
    pub default_value: Option<String>,
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
    /// The lowered Stage's light: colours evenly across the band, left to right, read from the key
    /// art at index time; empty for framed art and anything without key art.
    #[serde(default)]
    pub glow_argb: Vec<u32>,
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

/// An abstract screensaver a plugin publishes, checked and ready for the renderer.
#[derive(Clone, Debug, PartialEq, uniffi::Record, Serialize, Deserialize)]
pub struct ShaderScreensaver {
    /// "plugin:<plugin id>:<item id>".
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub source: String,
    pub plugin_name: String,
    pub uniforms: Vec<ShaderUniform>,
}

/// A value for one of a program's own uniforms: numbers, or a colour as ARGB.
#[derive(Clone, Debug, PartialEq, uniffi::Record, Serialize, Deserialize)]
pub struct ShaderUniform {
    pub name: String,
    pub floats: Vec<f32>,
    pub color_argb: Option<u32>,
}

/// One screensaver picture from a plugin: the prepared file, what it shows and who sent it.
#[derive(Clone, Debug, PartialEq, uniffi::Record, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Wallpaper {
    pub artwork_path: String,
    pub title: String,
    /// The line under the title, when the plugin gave one; otherwise the screensaver names the source.
    #[serde(default)]
    pub subtitle: Option<String>,
    pub source: String,
    /// Shown whole with the room around it, in its own shape, rather than filling the screen.
    #[serde(default)]
    pub framed: bool,
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
    /// The disk is full or would not read or write. The database may be sound, so it is kept.
    #[error("storage: the disk is full or would not read or write")]
    DiskFull,
}

impl From<lounge_store::StoreError> for CoreError {
    fn from(e: lounge_store::StoreError) -> Self {
        if e.is_disk_full() {
            return CoreError::DiskFull;
        }
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
    /// Play a stream URL in whatever video player the device has, from `start_ms` when given.
    fn play_stream(&self, url: String, title: String, start_ms: Option<i64>) -> Result<(), PlatformError>;
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
    /// What the tiles of the latest plugin search open, by tile id.
    pub search_links: Mutex<std::collections::HashMap<String, lounge_graph::DeepLink>>,
    /// The plugin whose item was opened last, refreshed once Lounge is back in front.
    pub opened_plugin: Mutex<Option<String>>,
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
    /// The `art_threads` threads pictures are prepared on.
    pub art_pool: Option<Arc<rayon::ThreadPool>>,
    pub permissions: Permissions,
    pub now_playing: Option<NowPlaying>,
    /// A downloaded update waiting for the person; the version it would install.
    pub update: Option<String>,
    /// Set once the worker has loaded the stored items into the graph.
    pub loaded: bool,
    /// Installed plugins with their manifests, for rows and refresh scheduling.
    pub plugins: Vec<plugins::PluginEntry>,
    /// How often each app was opened in each local hour, for honest "usually" explanations.
    pub habits: std::collections::HashMap<String, [u32; 24]>,
    /// Packages whose media session was playing at the last session ingest.
    pub playing: std::collections::HashSet<String>,
    /// Ingests whose pictures are still being prepared, a few at a time.
    pub pending_art: worker::PendingArt,
    /// The latest TvProvider records, for preparing pictures later.
    pub last_programs: Vec<ProgramRecord>,
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
        let db_path = PathBuf::from(&config.data_dir).join(DB_FILE);
        let Opened { store, layout, persisted, plugins: plugin_rows, habits } =
            open_store(&db_path).inspect_err(|e| tracing::error!("could not open the database: {e}"))?;
        // Only the persisted snapshot is needed for the first frame; the graph itself is loaded
        // by the worker as its first job, so opening the core costs one small read.
        let graph = Graph::new(layout);
        let (revision, snapshot) = match persisted {
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
        let art_pool = rayon::ThreadPoolBuilder::new()
            .num_threads(art_threads)
            .thread_name(|i| format!("lounge-art-{i}"))
            .build()
            .ok()
            .map(Arc::new);
        let plugin_entries = plugins::entries_from_rows(&plugin_rows);
        let (plugin_manager, refresh_rx, calls_rx, plugin_services) =
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
                art_pool,
                permissions: Permissions::default(),
                update: None,
                now_playing: None,
                loaded: false,
                plugins: plugin_entries,
                habits,
                playing: Default::default(),
                pending_art: Default::default(),
                last_programs: Vec::new(),
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
            search_links: Mutex::new(std::collections::HashMap::new()),
            opened_plugin: Mutex::new(None),
        });
        inner.queue_depth.fetch_add(1, Ordering::Relaxed);
        let _ = inner.tx.send(worker::Job::LoadGraph);
        worker::spawn(Arc::clone(&inner), rx);
        plugins::spawn(Arc::clone(&inner), refresh_rx, calls_rx, plugin_services);
        tracing::info!("core opened, revision {}", inner.state.lock().revision);
        Ok(Arc::new(Core { inner }))
    }

    /// Register for snapshots. The current snapshot is delivered immediately.
    pub fn add_listener(&self, listener: Arc<dyn SnapshotListener>) {
        self.count();
        // Delivered with no lock held, so the listener may call back into the core. It is listed
        // once nothing newer came out meanwhile, so it neither misses one nor ends on an older one.
        let mut snapshot = self.inner.snapshot.read().clone();
        loop {
            let delivered = snapshot.revision;
            listener.on_snapshot(snapshot);
            let mut listeners = self.inner.listeners.lock();
            let current = self.inner.snapshot.read();
            if current.revision == delivered {
                listeners.push(listener);
                return;
            }
            snapshot = current.clone();
        }
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

    /// A downloaded update that waits for the person to install it; `None` takes the notice down.
    pub fn set_update_notice(&self, version: Option<String>) {
        self.count();
        self.enqueue(worker::Job::UpdateNotice(version));
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
            // Back from watching something a plugin offered, so its resume points have moved.
            if let Some(plugin) = self.inner.opened_plugin.lock().take() {
                self.inner.plugins.refresh_soon(&plugin);
            }
            self.enqueue(worker::Job::Rebuild);
        }
    }

    pub fn set_utc_offset_minutes(&self, minutes: i32) {
        self.count();
        self.inner.utc_offset_minutes.store(minutes, Ordering::Relaxed);
        self.enqueue(worker::Job::UtcOffset(minutes));
    }

    // ----- plugins -----

    /// Abstract screensavers from enabled plugins, in install order.
    pub fn screensavers(&self) -> Vec<ShaderScreensaver> {
        self.count();
        let state = self.inner.state.lock();
        state.plugins.iter().filter(|p| p.enabled).flat_map(|p| screensavers::load(&state.store, &p.id)).collect()
    }

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
        if let Err(e) = self.inner.plugin_remove(&id) {
            tracing::warn!("plugin {id}: {e}");
        }
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
        self.inner.plugins.refresh_soon(&id);
    }

    /// Ask the enabled search plugins about `query`: one row per plugin that found something.
    /// It can take a few seconds, so call it off the main thread; when a newer search starts,
    /// one still waiting returns nothing.
    pub fn search_plugins(&self, query: String) -> Vec<Row> {
        self.count();
        search::run(&self.inner, query.trim())
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
        // Taken out first, so the server stops without the lock held.
        let stopped = self.inner.companion.lock().take();
        drop(stopped);
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
        self.inner.enqueue_job(job);
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
            // Android discards stderr, so a panic's message would otherwise be lost.
            let default_hook = std::panic::take_hook();
            std::panic::set_hook(Box::new(move |info| {
                tracing::error!("panic on {}: {info}", std::thread::current().name().unwrap_or("an unnamed thread"));
                default_hook(info);
            }));
        }
        #[cfg(not(target_os = "android"))]
        {
            let _ = tracing_subscriber::fmt().with_test_writer().try_init();
        }
    });
}

const DB_FILE: &str = "lounge.db";

/// What the first frame needs from the store, read together so an unreadable file shows up
/// before anything starts.
struct Opened {
    store: Store,
    layout: Layout,
    persisted: Option<(u64, String)>,
    plugins: Vec<PluginRow>,
    habits: std::collections::HashMap<String, [u32; 24]>,
}

fn read_store(path: &Path) -> Result<Opened, StoreError> {
    let store = Store::open(path)?;
    let layout = store.load_layout()?;
    let persisted = store.load_snapshot()?;
    let plugins = store.plugins()?;
    let habits = store.load_habits()?;
    Ok(Opened { store, layout, persisted, plugins, habits })
}

/// Opens the store, setting a database SQLite cannot read aside and starting an empty one.
fn open_store(path: &Path) -> Result<Opened, StoreError> {
    match read_store(path) {
        Err(e) if e.is_corrupt() => {
            let aside = Store::set_aside(path)?;
            tracing::error!("database unreadable ({e}); moved to {} and starting empty", aside.display());
            read_store(path)
        }
        other => other,
    }
}

/// Moves the database under `data_dir` aside so the next `Core::open` starts empty. The recovery
/// screen offers this when the core will not open.
#[uniffi::export]
pub fn set_storage_aside(data_dir: String) -> Result<String, CoreError> {
    let aside = Store::set_aside(&PathBuf::from(data_dir).join(DB_FILE))?;
    Ok(aside.to_string_lossy().into_owned())
}

/// Runs `f`, or returns `None` if it panicked. The core's long-lived threads run each piece of
/// work through this, so one bad picture or plugin answer cannot end the thread.
pub(crate) fn guarded<T>(what: &str, f: impl FnOnce() -> T) -> Option<T> {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)) {
        Ok(v) => Some(v),
        Err(_) => {
            tracing::error!("{what} panicked; carrying on without it");
            None
        }
    }
}

pub(crate) fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unreadable_database_is_set_aside_and_a_new_one_opens() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(DB_FILE);
        std::fs::write(&path, vec![0x5a; 8192]).unwrap();
        let opened = open_store(&path).unwrap();
        assert!(opened.persisted.is_none());
        assert!(opened.plugins.is_empty());
        assert!(dir.path().join("lounge.db.broken").exists());
    }

    #[test]
    fn a_full_disk_asks_for_space_rather_than_a_fresh_start() {
        let db = rusqlite::Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE t(x); PRAGMA max_page_count = 2;").unwrap();
        let full = db.execute("INSERT INTO t VALUES (?1)", ["x".repeat(100_000)]).unwrap_err();
        assert!(matches!(CoreError::from(StoreError::Db(full)), CoreError::DiskFull));

        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(DB_FILE), vec![0x5a; 8192]).unwrap();
        assert!(matches!(CoreError::from(Store::open(&dir.path().join(DB_FILE)).err().unwrap()), CoreError::Storage { .. }));
    }

    #[test]
    fn a_panic_is_contained() {
        assert_eq!(guarded("a sum", || 2 + 5), Some(7));
        assert_eq!(guarded("a test", || -> i32 { panic!("on purpose") }), None);
    }
}
