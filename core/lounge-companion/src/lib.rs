//! The companion server: serves the Svelte app and a small JSON API on the LAN, pushes snapshots
//! over a WebSocket, and pairs phones with a code or a QR link. The core implements
//! [`CompanionActions`]; this crate knows nothing about Android or the graph.

mod auth;
mod server;

pub use auth::{PairingInfo, TokenStore};
pub use server::Companion;

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DeviceInfo {
    pub name: String,
    pub model: String,
    pub version: String,
    pub tier: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RowRef {
    pub id: String,
    pub title: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LayoutDoc {
    pub row_order: Vec<String>,
    pub available_rows: Vec<RowRef>,
    pub hidden: Vec<String>,
    pub pinned_apps: Vec<String>,
    pub stage_pins: Vec<String>,
    pub enabled_channels: Vec<i64>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LayoutPatch {
    pub row_order: Option<Vec<String>>,
    pub hidden: Option<Vec<String>>,
    pub pinned_apps: Option<Vec<String>>,
    pub stage_pins: Option<Vec<String>>,
    pub enabled_channels: Option<Vec<i64>>,
}

/// Everything the API can do, provided by the core.
pub trait CompanionActions: Send + Sync {
    fn device_info(&self) -> DeviceInfo;
    /// The current `HomeSnapshot` as JSON.
    fn snapshot_json(&self) -> String;
    fn perform(&self, action: &str) -> Result<(), String>;
    fn launch(&self, package: &str) -> Result<(), String>;
    /// up, down, left, right, center, back, home, menu, play_pause, rewind, fast_forward, volume_up, volume_down, mute.
    fn send_key(&self, key: &str) -> Result<(), String>;
    fn type_text(&self, text: &str) -> Result<(), String>;
    fn search(&self, query: &str) -> Result<(), String>;
    fn layout(&self) -> LayoutDoc;
    fn set_layout(&self, patch: LayoutPatch) -> Result<LayoutDoc, String>;
    fn plugins_json(&self) -> String;
    /// Install a bundle from a file; returns the plugin's info as JSON.
    fn install_plugin(&self, path: &Path) -> Result<String, String>;
    /// enable, disable, refresh, remove.
    fn plugin_command(&self, id: &str, command: &str) -> Result<(), String>;
    fn set_plugin_setting(&self, id: &str, key: &str, value: Option<String>) -> Result<(), String>;
    fn plugin_log(&self, id: &str) -> Vec<String>;
    fn diagnostics_json(&self) -> String;
    fn export_json(&self) -> String;
    fn import_json(&self, json: &str) -> Result<(), String>;
    /// Directories artwork may be served from; nothing outside them is readable.
    fn art_roots(&self) -> Vec<PathBuf>;
    fn temp_dir(&self) -> PathBuf;
    /// Fetch a URL to a file (used for install-from-URL); the core already has an HTTP client.
    fn download(&self, url: &str, into: &Path) -> Result<(), String>;
}

#[derive(Debug, thiserror::Error)]
pub enum CompanionError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Other(String),
}

/// The device's LAN address, found by asking the OS which interface would route outwards.
pub fn local_ip() -> Option<std::net::IpAddr> {
    let socket = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("8.8.8.8:80").ok()?;
    socket.local_addr().ok().map(|a| a.ip()).filter(|ip| !ip.is_loopback() && !ip.is_unspecified())
}

/// Render a QR code of `text` to a PNG file; black modules on the house's light.
pub fn qr_png(text: &str, out: &Path, module_px: u32) -> Result<(), CompanionError> {
    let code = qrcode::QrCode::new(text.as_bytes()).map_err(|e| CompanionError::Other(e.to_string()))?;
    let w = code.width() as u32;
    let quiet = 4u32;
    let size = (w + quiet * 2) * module_px;
    let mut img = image::RgbaImage::from_pixel(size, size, image::Rgba([0xF6, 0xEF, 0xE7, 0xFF]));
    for (i, color) in code.to_colors().iter().enumerate() {
        if *color == qrcode::Color::Dark {
            let x = (i as u32 % w + quiet) * module_px;
            let y = (i as u32 / w + quiet) * module_px;
            for dy in 0..module_px {
                for dx in 0..module_px {
                    img.put_pixel(x + dx, y + dy, image::Rgba([0x1A, 0x10, 0x16, 0xFF]));
                }
            }
        }
    }
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent)?;
    }
    img.save_with_format(out, image::ImageFormat::Png).map_err(|e| CompanionError::Other(e.to_string()))?;
    Ok(())
}
