//! The plugin host: loads signed `.lgp` bundles, runs their WebAssembly in `wasmi` with fuel and
//! memory limits, and brokers the few capabilities a plugin may use: HTTP to allow-listed hosts, a
//! private key-value store, its own secrets and settings, a log. Plugins never see Android, the
//! file system, sockets or other plugins' data.

pub mod bundle;
pub mod host;
pub mod http;

pub use lounge_plugin_api as api;

use lounge_words::t;

/// Every error reads as a sentence the TV or the phone can show as it is.
#[derive(Debug, thiserror::Error)]
pub enum PluginError {
    Io(#[from] std::io::Error),
    Bundle(String),
    Json(#[from] serde_json::Error),
    Signature,
    Manifest(String),
    /// The module is not WebAssembly this host can run, or lacks an export. `said` is for the
    /// screen and `cause` for the log.
    Wasm { said: String, cause: String },
    /// A call did not finish: it trapped, ran out of fuel or time, or answered with something
    /// that cannot be read. `said` is for the screen and `cause` for the log.
    Trap { said: String, cause: String },
    /// The plugin's own error, shown as it wrote it.
    Failed(String),
    /// The call was stopped because its answer is no longer wanted; not a failure.
    Stopped,
    /// The plugin waits on the person; not a failure.
    Asks(lounge_plugin_api::Ask),
    Denied(String),
    Fetch(String),
}

impl std::fmt::Display for PluginError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            // Translators: {reason} is the system's own words for what went wrong, in English.
            PluginError::Io(e) => f.write_str(&t!("Lounge couldn't read or write the plugin's files: {reason}. Check that the TV has free space, then try again.", reason = e)),
            // Translators: {reason} says what is wrong with the file, such as "manifest.json is missing".
            PluginError::Bundle(why) => f.write_str(&t!("This file isn't a plugin Lounge can install: {reason}. Check that it is the plugin's .lgp file.", reason = why)),
            // Translators: {reason} is the reader's own words for what went wrong, in English.
            PluginError::Json(e) => f.write_str(&t!("Lounge couldn't read this plugin's manifest: {reason}. Ask its publisher for a corrected one.", reason = e)),
            PluginError::Signature => f.write_str(&t!("This plugin's signature doesn't match its contents, so it may have been changed. Get it again from its publisher.")),
            // Translators: {reason} says what is wrong with the manifest, in English, for its publisher.
            PluginError::Manifest(why) => f.write_str(&t!("This plugin's manifest isn't valid: {reason}. Ask its publisher for a corrected one.", reason = why)),
            PluginError::Wasm { said, .. } | PluginError::Trap { said, .. } => f.write_str(said),
            PluginError::Stopped => f.write_str(&t!("The plugin was stopped because its answer was no longer needed.")),
            PluginError::Asks(ask) => f.write_str(&ask.say),
            PluginError::Failed(said) | PluginError::Denied(said) | PluginError::Fetch(said) => f.write_str(said),
        }
    }
}

pub type Result<T> = std::result::Result<T, PluginError>;
