//! The plugin host: loads signed `.lgp` bundles, runs their WebAssembly in `wasmi` with fuel and
//! memory limits, and brokers the few capabilities a plugin may use: HTTP to allow-listed hosts, a
//! private key-value store, its own secrets and settings, a log. Plugins never see Android, the
//! file system, sockets or other plugins' data.

pub mod bundle;
pub mod host;
pub mod http;

pub use lounge_plugin_api as api;

/// Every error reads as a sentence the TV or the phone can show as it is.
#[derive(Debug, thiserror::Error)]
pub enum PluginError {
    #[error("Lounge couldn't read or write the plugin's files: {0}. Check that the TV has free space, then try again.")]
    Io(#[from] std::io::Error),
    #[error("This file isn't a plugin Lounge can install: {0}. Check that it is the plugin's .lgp file.")]
    Bundle(String),
    #[error("Lounge couldn't read this plugin's manifest: {0}. Ask its publisher for a corrected one.")]
    Json(#[from] serde_json::Error),
    #[error("This plugin's signature doesn't match its contents, so it may have been changed. Get it again from its publisher.")]
    Signature,
    #[error("This plugin's manifest isn't valid: {0}. Ask its publisher for a corrected one.")]
    Manifest(String),
    /// The module is not WebAssembly this host can run, or lacks an export. `said` is for the
    /// screen and `cause` for the log.
    #[error("{said}")]
    Wasm { said: String, cause: String },
    /// A call did not finish: it trapped, ran out of fuel or time, or answered with something
    /// that cannot be read. `said` is for the screen and `cause` for the log.
    #[error("{said}")]
    Trap { said: String, cause: String },
    /// The plugin's own error, shown as it wrote it.
    #[error("{0}")]
    Failed(String),
    /// The plugin waits on the person; not a failure.
    #[error("{}", .0.say)]
    Asks(lounge_plugin_api::Ask),
    #[error("{0}")]
    Denied(String),
    #[error("{0}")]
    Fetch(String),
}

pub type Result<T> = std::result::Result<T, PluginError>;
