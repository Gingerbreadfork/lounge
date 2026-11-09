//! The companion server: serves the Svelte app and a small JSON API on the LAN, pushes snapshots
//! over a WebSocket, and pairs phones with a code or a QR link. The core implements
//! [`CompanionActions`]; this crate knows nothing about Android or the graph.

mod auth;
mod server;

pub use auth::{PairingInfo, TokenStore};
pub use server::Companion;

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr};
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
    /// Whether the row is on the home screen now, by the user's choice or its default.
    #[serde(default = "shown_by_default")]
    pub shown: bool,
    /// Whether the user chose that, rather than the row following its default.
    #[serde(default)]
    pub chosen: bool,
    /// Tiles the row would carry now.
    #[serde(default)]
    pub count: u32,
    /// One short line under the title: "14 apps", "Jellyfin · 30 titles".
    #[serde(default)]
    pub detail: String,
}

fn shown_by_default() -> bool {
    true
}

/// An item named for an editor list: something hidden or pinned to the Stage.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NamedRef {
    pub id: String,
    pub title: String,
    pub icon_path: Option<String>,
}

/// An installed app, for pinning from the phone.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppRef {
    pub package: String,
    pub title: String,
    pub icon_path: Option<String>,
    pub pinned: bool,
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
    /// Rows the user showed or hid by hand; a row not listed follows its default.
    #[serde(default)]
    pub row_visibility: HashMap<String, bool>,
    /// Every app on the TV, A to Z, so any of them can be pinned from the phone.
    #[serde(default)]
    pub apps: Vec<AppRef>,
    /// The hidden items with their names, by title.
    #[serde(default)]
    pub hidden_items: Vec<NamedRef>,
    /// The Stage pins with their names, in pin order.
    #[serde(default)]
    pub stage_items: Vec<NamedRef>,
    /// How many recently opened apps follow the pins in Your apps; zero keeps the row to pins.
    #[serde(default)]
    pub recent_apps: u32,
    /// Apps and plugins left out of Continue watching, each as a tile's `provider`.
    #[serde(default)]
    pub muted_providers: Vec<String>,
    /// The same with their names, by title; each id is the provider.
    #[serde(default)]
    pub muted_provider_items: Vec<NamedRef>,
    /// Notice tiles the person chose not to see again, by id.
    #[serde(default)]
    pub dismissed_notices: Vec<String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LayoutPatch {
    pub row_order: Option<Vec<String>>,
    pub hidden: Option<Vec<String>>,
    pub pinned_apps: Option<Vec<String>>,
    pub stage_pins: Option<Vec<String>>,
    pub enabled_channels: Option<Vec<i64>>,
    pub row_visibility: Option<HashMap<String, bool>>,
    pub recent_apps: Option<u32>,
    pub muted_providers: Option<Vec<String>>,
    pub dismissed_notices: Option<Vec<String>>,
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

/// The device's LAN address: a private IPv4 address on an interface that is up, so a network
/// with no way to the internet still has one. When several qualify, or none does, the address
/// the OS would route outwards from decides.
pub fn local_ip() -> Option<IpAddr> {
    pick_lan_ip(&private_ipv4s(), routed_ip)
}

fn pick_lan_ip(private: &[Ipv4Addr], routed: impl FnOnce() -> Option<IpAddr>) -> Option<IpAddr> {
    match private {
        [] => routed(),
        [only] => Some(IpAddr::V4(*only)),
        several => match routed() {
            Some(IpAddr::V4(ip)) if several.contains(&ip) => Some(IpAddr::V4(ip)),
            _ => Some(IpAddr::V4(several[0])),
        },
    }
}

/// The address the OS would send from towards the internet; none when there is no route there.
fn routed_ip() -> Option<IpAddr> {
    let socket = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("8.8.8.8:80").ok()?;
    socket.local_addr().ok().map(|a| a.ip()).filter(|ip| !ip.is_loopback() && !ip.is_unspecified())
}

/// Private IPv4 addresses on interfaces that are up and running, loopback aside.
#[cfg(unix)]
fn private_ipv4s() -> Vec<Ipv4Addr> {
    let mut found = Vec::new();
    let mut list: *mut libc::ifaddrs = std::ptr::null_mut();
    // The list stays valid until freeifaddrs, and each pointer is checked before it is read.
    unsafe {
        if libc::getifaddrs(&mut list) != 0 {
            return found;
        }
        let mut node = list;
        while let Some(ifa) = node.as_ref() {
            let flags = ifa.ifa_flags as libc::c_int;
            let live = flags & libc::IFF_UP != 0 && flags & libc::IFF_RUNNING != 0 && flags & libc::IFF_LOOPBACK == 0;
            if live && !ifa.ifa_addr.is_null() && (*ifa.ifa_addr).sa_family as libc::c_int == libc::AF_INET {
                let addr = &*(ifa.ifa_addr as *const libc::sockaddr_in);
                let ip = Ipv4Addr::from(u32::from_be(addr.sin_addr.s_addr));
                if ip.is_private() {
                    found.push(ip);
                }
            }
            node = ifa.ifa_next;
        }
        libc::freeifaddrs(list);
    }
    found
}

#[cfg(not(unix))]
fn private_ipv4s() -> Vec<Ipv4Addr> {
    Vec::new()
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_private_address_needs_no_route_out() {
        let lan = Ipv4Addr::new(192, 168, 1, 20);
        let vpn = Ipv4Addr::new(10, 8, 0, 2);
        let public = IpAddr::V4(Ipv4Addr::new(203, 0, 113, 7));
        assert_eq!(pick_lan_ip(&[lan], || None), Some(IpAddr::V4(lan)));
        assert_eq!(pick_lan_ip(&[lan], || Some(public)), Some(IpAddr::V4(lan)));
        assert_eq!(pick_lan_ip(&[lan, vpn], || Some(IpAddr::V4(vpn))), Some(IpAddr::V4(vpn)));
        assert_eq!(pick_lan_ip(&[lan, vpn], || None), Some(IpAddr::V4(lan)));
        assert_eq!(pick_lan_ip(&[], || Some(public)), Some(public));
        assert_eq!(pick_lan_ip(&[], || None), None);
    }

    #[test]
    fn this_machine_has_a_lan_address_when_it_has_a_private_one() {
        let private = private_ipv4s();
        if let Some(IpAddr::V4(routed)) = routed_ip()
            && routed.is_private()
        {
            assert!(private.contains(&routed), "{routed} is missing from {private:?}");
        }
        if private.is_empty() {
            eprintln!("skipped: no private IPv4 address on this machine");
            return;
        }
        let ip = local_ip();
        assert!(matches!(ip, Some(IpAddr::V4(v4)) if private.contains(&v4)), "{ip:?} is not one of {private:?}");
    }
}
