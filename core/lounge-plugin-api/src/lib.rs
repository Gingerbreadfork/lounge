//! The `lounge:plugin@1.0` interface: the manifest, items, refresh context, HTTP messages and the
//! core-wasm ABI, shared by the host (`lounge-plugins`) and the SDK (`plugins/sdk`).
//!
//! A plugin exports `memory`, `lg_alloc`, `lg_free`, `lg_manifest`, `lg_refresh` and optionally
//! `lg_search` and `lg_resolve`. Strings cross as UTF-8 JSON in linear memory, a `(ptr, len)` pair
//! packed into one `i64` as `ptr << 32 | len`. The host writes into memory from the plugin's
//! `lg_alloc`; memory the plugin returns is freed with `lg_free`. Host functions live in the
//! `lounge` import module.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const ABI_VERSION: u32 = 1;
pub const HOST_MODULE: &str = "lounge";

pub mod exports {
    pub const MEMORY: &str = "memory";
    pub const ALLOC: &str = "lg_alloc";
    pub const FREE: &str = "lg_free";
    pub const MANIFEST: &str = "lg_manifest";
    pub const REFRESH: &str = "lg_refresh";
    pub const SEARCH: &str = "lg_search";
    pub const RESOLVE: &str = "lg_resolve";
}

pub mod imports {
    pub const FETCH: &str = "fetch";
    pub const KV_GET: &str = "kv_get";
    pub const KV_SET: &str = "kv_set";
    pub const KV_DELETE: &str = "kv_delete";
    pub const NOW: &str = "now";
    pub const LOG: &str = "log";
    pub const SECRET: &str = "secret";
    pub const SETTING: &str = "setting";
}

pub const LOG_DEBUG: i32 = 0;
pub const LOG_INFO: i32 = 1;
pub const LOG_WARN: i32 = 2;
pub const LOG_ERROR: i32 = 3;

pub fn pack(ptr: u32, len: u32) -> i64 {
    (((ptr as u64) << 32) | len as u64) as i64
}

pub fn unpack(v: i64) -> (u32, u32) {
    let u = v as u64;
    ((u >> 32) as u32, (u & 0xffff_ffff) as u32)
}

/// What a plugin is and what it needs. Shipped as `manifest.json` in the bundle and also
/// returned by `lg_manifest`; the two must agree.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Manifest {
    /// Stable identifier, lowercase, dots and dashes allowed ("tv.lounge.iptv").
    pub id: String,
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub publisher_name: String,
    #[serde(default)]
    pub kinds: Vec<Kind>,
    /// Hosts the plugin may fetch from; `*` means any host and is shown to the user as such.
    #[serde(default)]
    pub fetch_hosts: Vec<String>,
    /// How often `refresh` runs while the screen is on; 0 means the host default.
    #[serde(default)]
    pub refresh_minutes: u32,
    /// Rows the plugin publishes items into, in order.
    #[serde(default)]
    pub rows: Vec<RowSpec>,
    #[serde(default)]
    pub settings: Vec<SettingSpec>,
    #[serde(default)]
    pub homepage: Option<String>,
}

/// A plugin id is a reverse-domain name: labels of `[a-z0-9]` joined by `.` or `-`, at least
/// two of them, no label empty. It is also a directory name, so `.` and `..` can never pass.
pub fn valid_plugin_id(id: &str) -> bool {
    let labels: Vec<&str> = id.split(['.', '-']).collect();
    labels.len() >= 2 && labels.iter().all(|l| !l.is_empty() && l.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit()))
}

impl Manifest {
    pub fn validate(&self) -> Result<(), String> {
        if !valid_plugin_id(&self.id) {
            return Err(format!("invalid plugin id {:?}", self.id));
        }
        if self.name.trim().is_empty() {
            return Err("name is required".into());
        }
        if self.version.trim().is_empty() {
            return Err("version is required".into());
        }
        let mut keys = std::collections::HashSet::new();
        for s in &self.settings {
            if s.key.is_empty() || !keys.insert(&s.key) {
                return Err(format!("duplicate or empty setting key {:?}", s.key));
            }
        }
        let mut rows = std::collections::HashSet::new();
        for r in &self.rows {
            if r.id.is_empty() || !rows.insert(&r.id) {
                return Err(format!("duplicate or empty row id {:?}", r.id));
            }
        }
        Ok(())
    }

    pub fn allows_host(&self, host: &str) -> bool {
        let host = host.to_ascii_lowercase();
        self.fetch_hosts.iter().any(|h| {
            let h = h.to_ascii_lowercase();
            h == "*" || h == host || (h.starts_with("*.") && host.ends_with(&h[1..]))
        })
    }

    pub fn refresh_minutes_or_default(&self) -> u32 {
        if self.refresh_minutes == 0 { 60 } else { self.refresh_minutes.max(5) }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Content,
    Epg,
    Wallpaper,
    Search,
    Screensaver,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct RowSpec {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub style: RowStyle,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RowStyle {
    #[default]
    Landscape,
    Poster,
    Compact,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SettingSpec {
    pub key: String,
    pub label: String,
    #[serde(default)]
    pub kind: SettingKind,
    #[serde(default)]
    pub required: bool,
    #[serde(default)]
    pub help: Option<String>,
    #[serde(default)]
    pub default: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SettingKind {
    #[default]
    Text,
    Url,
    Password,
    Number,
    Toggle,
}

impl SettingKind {
    pub fn is_secret(self) -> bool {
        self == SettingKind::Password
    }
}

/// What the host tells a plugin when asking it to refresh.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct RefreshContext {
    pub now_ms: i64,
    pub utc_offset_minutes: i32,
    /// a, b or c; plugins may fetch less on smaller tiers.
    pub tier: String,
    /// When the previous successful refresh finished, if any.
    pub since_ms: Option<i64>,
    /// How far ahead schedule data is wanted.
    pub window_hours: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SearchQuery {
    pub query: String,
    pub limit: u32,
}

/// One thing a plugin publishes. The host turns it into a tile, a Stage spotlight, a channel
/// in the guide or a wallpaper, and fetches the artwork URLs itself.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Item {
    pub id: String,
    #[serde(default)]
    pub kind: ItemKind,
    pub title: String,
    #[serde(default)]
    pub subtitle: Option<String>,
    #[serde(default)]
    pub season: Option<String>,
    #[serde(default)]
    pub episode: Option<String>,
    #[serde(default)]
    pub duration_ms: Option<i64>,
    #[serde(default)]
    pub position_ms: Option<i64>,
    #[serde(default)]
    pub progress: Option<f32>,
    #[serde(default)]
    pub last_used_ms: Option<i64>,
    #[serde(default)]
    pub added_ms: Option<i64>,
    /// Portrait key art.
    #[serde(default)]
    pub poster: Option<String>,
    /// 16:9 art.
    #[serde(default)]
    pub thumbnail: Option<String>,
    #[serde(default)]
    pub logo: Option<String>,
    /// Row id from the manifest; items without one go only where the host puts them
    /// (Continue watching for resume points, Live now and the guide for channels).
    #[serde(default)]
    pub row: Option<String>,
    #[serde(default)]
    pub action: Action,
    /// Channel this item is or belongs to (channels and schedule entries).
    #[serde(default)]
    pub channel_id: Option<String>,
    #[serde(default)]
    pub channel_name: Option<String>,
    #[serde(default)]
    pub channel_number: Option<String>,
    #[serde(default)]
    pub start_ms: Option<i64>,
    #[serde(default)]
    pub end_ms: Option<i64>,
    #[serde(default)]
    pub is_new: bool,
    #[serde(default)]
    pub weight: i32,
    /// The program a `screensaver` item draws.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shader: Option<Shader>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ItemKind {
    #[default]
    Program,
    Resume,
    Channel,
    Schedule,
    Wallpaper,
    Screensaver,
}

/// An abstract screensaver: an AGSL program the TV runs on its GPU, and values for the
/// uniforms it declares beyond the ones Lounge provides (PLUGINS.md, Screensavers).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Shader {
    pub source: String,
    #[serde(default)]
    pub uniforms: BTreeMap<String, Uniform>,
}

/// A uniform's value: a number, up to sixteen numbers, or a colour such as "#FFA86B".
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Uniform {
    Number(f32),
    Numbers(Vec<f32>),
    Color(String),
}

impl Shader {
    pub fn new(source: impl Into<String>) -> Shader {
        Shader { source: source.into(), uniforms: BTreeMap::new() }
    }
    pub fn float(mut self, name: &str, value: f32) -> Shader {
        self.uniforms.insert(name.to_string(), Uniform::Number(value));
        self
    }
    pub fn floats(mut self, name: &str, values: &[f32]) -> Shader {
        self.uniforms.insert(name.to_string(), Uniform::Numbers(values.to_vec()));
        self
    }
    pub fn color(mut self, name: &str, hex: &str) -> Shader {
        self.uniforms.insert(name.to_string(), Uniform::Color(hex.to_string()));
        self
    }
}

/// What selecting an item does. The first field that is set wins, in this order: `play`
/// (a stream URL for a video player), `view` (an Android URI), `launch` (a package), `resolve`
/// (ask the plugin at selection time). `fallback_launch` runs if `view` finds no handler.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Action {
    #[serde(default)]
    pub play: Option<String>,
    #[serde(default)]
    pub view: Option<String>,
    #[serde(default)]
    pub launch: Option<String>,
    #[serde(default)]
    pub resolve: Option<String>,
    #[serde(default)]
    pub fallback_launch: Option<String>,
    /// Title handed to the player.
    #[serde(default)]
    pub title: Option<String>,
}

impl Action {
    pub fn play(url: impl Into<String>, title: impl Into<String>) -> Action {
        Action { play: Some(url.into()), title: Some(title.into()), ..Default::default() }
    }
    pub fn view(uri: impl Into<String>) -> Action {
        Action { view: Some(uri.into()), ..Default::default() }
    }
    pub fn launch(package: impl Into<String>) -> Action {
        Action { launch: Some(package.into()), ..Default::default() }
    }
    pub fn is_empty(&self) -> bool {
        self.play.is_none() && self.view.is_none() && self.launch.is_none() && self.resolve.is_none()
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HttpRequest {
    #[serde(default = "default_method")]
    pub method: String,
    pub url: String,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    #[serde(default)]
    pub body: Option<String>,
    /// 0 means the host default.
    #[serde(default)]
    pub timeout_ms: u32,
}

fn default_method() -> String {
    "GET".to_string()
}

impl HttpRequest {
    pub fn get(url: impl Into<String>) -> HttpRequest {
        HttpRequest { method: "GET".into(), url: url.into(), headers: BTreeMap::new(), body: None, timeout_ms: 0 }
    }
    pub fn header(mut self, k: &str, v: &str) -> HttpRequest {
        self.headers.insert(k.to_string(), v.to_string());
        self
    }
}

/// A response body is text when it is valid UTF-8, otherwise base64 in `body_base64`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct HttpResponse {
    pub status: u16,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub body_base64: Option<String>,
}

impl HttpResponse {
    pub fn ok(&self) -> bool {
        (200..300).contains(&self.status)
    }

    pub fn bytes(&self) -> Vec<u8> {
        match &self.body_base64 {
            Some(b64) => base64_decode(b64),
            None => self.body.clone().into_bytes(),
        }
    }
}

/// Result of a plugin call, as JSON: `{"ok": ...}` or `{"err": "..."}`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Outcome<T> {
    #[serde(rename = "ok")]
    Ok(T),
    #[serde(rename = "err")]
    Err(String),
}

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

pub fn base64_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
        out.push(B64[(n >> 18) as usize & 63] as char);
        out.push(B64[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 { B64[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if chunk.len() > 2 { B64[n as usize & 63] as char } else { '=' });
    }
    out
}

pub fn base64_decode(s: &str) -> Vec<u8> {
    fn val(c: u8) -> Option<u32> {
        Some(match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        } as u32)
    }
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    let mut buf = 0u32;
    let mut bits = 0;
    for c in s.bytes() {
        let Some(v) = val(c) else { continue };
        buf = (buf << 6) | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buf >> bits) as u8);
            buf &= (1 << bits) - 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pack_roundtrip() {
        let v = pack(0xdead_beef, 42);
        assert_eq!(unpack(v), (0xdead_beef, 42));
        assert_eq!(unpack(0), (0, 0));
    }

    #[test]
    fn base64_roundtrip() {
        for s in ["", "a", "ab", "abc", "hello, world"] {
            assert_eq!(base64_decode(&base64_encode(s.as_bytes())), s.as_bytes());
        }
        assert_eq!(base64_encode(b"abc"), "YWJj");
    }

    #[test]
    fn manifest_hosts() {
        let m = Manifest { fetch_hosts: vec!["api.example.com".into(), "*.cdn.net".into()], ..Default::default() };
        assert!(m.allows_host("api.example.com"));
        assert!(m.allows_host("img.cdn.net"));
        assert!(!m.allows_host("evil.com"));
        assert!(Manifest { fetch_hosts: vec!["*".into()], ..Default::default() }.allows_host("anything"));
    }

    #[test]
    fn manifest_validation() {
        let mut m = Manifest { id: "tv.lounge.iptv".into(), name: "IPTV".into(), version: "1.0".into(), ..Default::default() };
        assert!(m.validate().is_ok());
        m.id = "Bad Id".into();
        assert!(m.validate().is_err());
    }
}

#[cfg(test)]
mod shader_tests {
    use super::*;

    #[test]
    fn uniforms_take_numbers_lists_and_colours() {
        let item: Item = serde_json::from_str(r##"{"id": "smoke", "kind": "screensaver", "title": "Smoke",
            "shader": {"source": "half4 main(float2 p) { return half4(1); }", "uniforms": {"speed": 0.5, "drift": [0.1, 0.2], "smoke": "#C8B8A0"}}}"##).unwrap();
        assert_eq!(item.kind, ItemKind::Screensaver);
        let shader = item.shader.unwrap();
        assert_eq!(shader.uniforms["speed"], Uniform::Number(0.5));
        assert_eq!(shader.uniforms["drift"], Uniform::Numbers(vec![0.1, 0.2]));
        assert_eq!(shader.uniforms["smoke"], Uniform::Color("#C8B8A0".into()));
        let built = Shader::new("x").float("speed", 0.5).floats("drift", &[0.1, 0.2]).color("smoke", "#C8B8A0");
        assert_eq!(serde_json::to_value(&built).unwrap()["uniforms"]["drift"], serde_json::json!([0.1f32, 0.2f32]));
        assert!(serde_json::to_string(&Item::default()).unwrap().find("shader").is_none());
    }
}

#[cfg(test)]
mod id_tests {
    use super::valid_plugin_id;

    #[test]
    fn plugin_ids_are_directory_safe() {
        assert!(valid_plugin_id("tv.lounge.iptv"));
        assert!(valid_plugin_id("acme-media.jellyfin2"));
        for bad in ["", ".", "..", "a", "tv..lounge", ".lounge", "lounge.", "Tv.lounge", "tv/lounge", "tv.lou nge"] {
            assert!(!valid_plugin_id(bad), "{bad:?} accepted");
        }
    }
}
