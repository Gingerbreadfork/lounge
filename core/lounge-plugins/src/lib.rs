//! The plugin host: loads signed `.lgp` bundles, runs their WebAssembly in `wasmi` with fuel and
//! memory limits, and brokers the few capabilities a plugin may use: HTTP to allow-listed hosts, a
//! private key-value store, its own secrets and settings, a log. Plugins never see Android, the
//! file system, sockets or other plugins' data.

pub mod bundle;
pub mod host;
pub mod http;

pub use lounge_plugin_api as api;

#[derive(Debug, thiserror::Error)]
pub enum PluginError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("bundle: {0}")]
    Bundle(String),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("signature invalid")]
    Signature,
    #[error("manifest: {0}")]
    Manifest(String),
    #[error("wasm: {0}")]
    Wasm(String),
    #[error("plugin trapped: {0}")]
    Trap(String),
    #[error("plugin failed: {0}")]
    Failed(String),
    #[error("denied: {0}")]
    Denied(String),
    #[error("fetch: {0}")]
    Fetch(String),
}

pub type Result<T> = std::result::Result<T, PluginError>;
