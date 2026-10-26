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
use std::net::IpAddr;

pub const ABI_VERSION: u32 = 1;
pub const HOST_MODULE: &str = "lounge";

/// The allow-list entry for a plugin that finds devices on the home network itself, such as a
/// tuner: it may reach home-network IP literals and `.local` names it was not given.
pub const LOCAL_NETWORK: &str = "local-network";

/// The host of a URL, lowercased, without user name, port or brackets.
pub fn host_of(url: &str) -> Option<String> {
    let rest = url.split("://").nth(1)?;
    let authority = rest.split(['/', '?', '#']).next()?;
    let host = authority.rsplit('@').next()?;
    // An IPv6 literal keeps its brackets' contents whole; anything else stops at the port.
    let host = if let Some(inner) = host.strip_prefix('[') {
        inner.split(']').next()?
    } else {
        host.split(':').next()?
    };
    if host.is_empty() { None } else { Some(host.to_ascii_lowercase()) }
}

/// Private, shared, link-local and unique local addresses: a home network's, not the internet's.
pub fn is_local_network(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let o = v4.octets();
            v4.is_private() || v4.is_link_local() || (o[0] == 100 && (o[1] & 0xc0) == 64)
        }
        IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(v4) => is_local_network(IpAddr::V4(v4)),
            None => {
                let first = v6.segments()[0];
                (first & 0xfe00) == 0xfc00 || (first & 0xffc0) == 0xfe80
            }
        },
    }
}

/// A name only the home network answers for: one label, or a suffix the internet does not delegate.
fn local_style(host: &str) -> bool {
    !host.contains('.') || [".local", ".lan", ".home.arpa", ".internal"].iter().any(|suffix| host.ends_with(suffix))
}

/// A home-network IP literal or a local-style name, which [LOCAL_NETWORK] covers.
pub fn is_home_name(host: &str) -> bool {
    let host = host.trim_start_matches('[').trim_end_matches(']');
    match host.parse::<IpAddr>() {
        Ok(ip) => is_local_network(ip),
        Err(_) => local_style(host),
    }
}

/// The host a setting's value names, with or without its scheme.
pub fn host_in_setting(value: &str) -> Option<String> {
    let value = value.trim();
    host_of(value).or_else(|| host_of(&format!("http://{value}")))
}

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

/// What is wrong with an allow-list entry that is neither `*`, [LOCAL_NETWORK] nor `$key`: it
/// must be a host name or an IP literal, with no scheme, port or path, and a wildcard must sit
/// above at least two labels of a name, so `*.com` and `*.1` cannot stand in for `*`.
fn fetch_host_problem(entry: &str) -> Option<&'static str> {
    let (wild, name) = match entry.strip_prefix("*.") {
        Some(rest) => (true, rest),
        None => (false, entry),
    };
    let literal = name.trim_start_matches('[').trim_end_matches(']');
    if !wild && literal.parse::<IpAddr>().is_ok() {
        return None;
    }
    let labels: Vec<&str> = name.split('.').collect();
    let label_ok = |l: &&str| !l.is_empty() && l.len() <= 63 && !l.starts_with('-') && !l.ends_with('-') && l.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-');
    if name.len() > 253 || !labels.iter().all(label_ok) {
        return Some("is not a host name or an IP address; leave out the scheme, port and path");
    }
    if labels.last().is_some_and(|l| l.bytes().all(|b| b.is_ascii_digit())) {
        return Some("is not a host name or an IP address");
    }
    if wild && labels.len() < 2 {
        return Some("is too wide; a wildcard needs a name of two labels or more under it, such as *.example.com");
    }
    None
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
        for h in &self.fetch_hosts {
            match h.strip_prefix('$') {
                Some(key) if !self.settings.iter().any(|s| s.key == key) => return Err(format!("fetch host {h:?} names no setting")),
                Some(_) => {}
                None if h == "*" || h.eq_ignore_ascii_case(LOCAL_NETWORK) => {}
                None => {
                    if let Some(problem) = fetch_host_problem(h) {
                        return Err(format!("fetch host {h:?} {problem}"));
                    }
                }
            }
        }
        Ok(())
    }

    /// Whether the allow list names `host`. `$key` stands for the host in the plugin's setting
    /// `key`, which `setting` looks up, and [LOCAL_NETWORK] for home-network literals and names.
    pub fn allows_host_with(&self, host: &str, setting: &dyn Fn(&str) -> Option<String>) -> bool {
        let host = host.to_ascii_lowercase();
        self.fetch_hosts.iter().any(|h| {
            let h = h.to_ascii_lowercase();
            match h.strip_prefix('$') {
                Some(key) => setting(key).and_then(|v| host_in_setting(&v)).is_some_and(|given| given == host),
                None if h == LOCAL_NETWORK => is_home_name(&host),
                None => h == "*" || h == host || (h.starts_with("*.") && host.ends_with(&h[1..])),
            }
        })
    }

    /// As [Manifest::allows_host_with] for a plugin with no settings filled in.
    pub fn allows_host(&self, host: &str) -> bool {
        self.allows_host_with(host, &|_| None)
    }

    /// The hosts this plugin may follow into the home network: those its `$key` settings hold, and
    /// the IP literals and local-style names its allow list names exactly. A public name gets public
    /// addresses alone, so its owner cannot point it at a router.
    pub fn home_hosts(&self, setting: &dyn Fn(&str) -> Option<String>) -> Vec<String> {
        let mut hosts: Vec<String> = self
            .fetch_hosts
            .iter()
            .filter_map(|h| match h.strip_prefix('$') {
                Some(key) => setting(key).and_then(|v| host_in_setting(&v)),
                None if h == "*" || h.starts_with("*.") || h.eq_ignore_ascii_case(LOCAL_NETWORK) => None,
                None => {
                    let h = h.trim().trim_start_matches('[').trim_end_matches(']').to_ascii_lowercase();
                    (h.parse::<IpAddr>().is_ok() || local_style(&h)).then_some(h)
                }
            })
            .collect();
        hosts.sort();
        hosts.dedup();
        hosts
    }

    /// Whether changing setting `key` can move where the plugin connects: an address setting, or
    /// one the allow list names with `$key`.
    pub fn decides_origin(&self, key: &str) -> bool {
        self.settings.iter().any(|s| s.key == key && s.kind == SettingKind::Url) || self.fetch_hosts.iter().any(|h| h.strip_prefix('$') == Some(key))
    }

    /// Whether the allow list has [LOCAL_NETWORK].
    pub fn reaches_local_network(&self) -> bool {
        self.fetch_hosts.iter().any(|h| h.eq_ignore_ascii_case(LOCAL_NETWORK))
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
    /// An address that carries credentials, such as an IPTV provider's playlist link.
    #[serde(rename = "secret_url")]
    SecretUrl,
}

impl SettingKind {
    /// Read only through `secret`, never shown back or written into a backup.
    pub fn is_secret(self) -> bool {
        matches!(self, SettingKind::Password | SettingKind::SecretUrl)
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
    /// A `wallpaper` shown whole, with the room around it, rather than filling the screen: paintings.
    #[serde(default)]
    pub framed: bool,
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
/// (ask the plugin at selection time), `search` (Lounge's own search, with this query).
/// `fallback_launch` runs if `view` finds no handler. An action `resolve` hands back with
/// nothing set means the plugin has done the work itself.
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
    pub search: Option<String>,
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
    /// Open Lounge's search with `query`: for things that live in some app the plugin cannot name.
    pub fn search(query: impl Into<String>) -> Action {
        Action { search: Some(query.into()), ..Default::default() }
    }
    pub fn is_empty(&self) -> bool {
        self.play.is_none() && self.view.is_none() && self.launch.is_none() && self.resolve.is_none() && self.search.is_none()
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
    fn allow_list_entries_must_be_hosts() {
        let check = |h: &str| Manifest { id: "tv.example.p".into(), name: "P".into(), version: "1".into(), fetch_hosts: vec![h.into()], ..Default::default() }.validate();
        for good in ["*", "local-network", "localhost", "127.0.0.1", "[::1]", "api.example.com", "*.example.com", "*.ytimg.com"] {
            assert!(check(good).is_ok(), "{good}");
        }
        for bad in ["*.", "*.com", "*.1", "*.0.1", "http://a.example", "a.example:8080", "a.example/path", "a..example", "example.", "-a.example", "10.0.0"] {
            assert!(check(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn a_secret_url_is_kept_like_a_password() {
        let k: SettingKind = serde_json::from_str("\"secret_url\"").unwrap();
        assert_eq!(k, SettingKind::SecretUrl);
        assert_eq!(serde_json::to_string(&k).unwrap(), "\"secret_url\"");
        assert!(k.is_secret() && SettingKind::Password.is_secret() && !SettingKind::Url.is_secret());
    }

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
    fn settings_and_the_home_network_in_the_allow_list() {
        let m = Manifest { fetch_hosts: vec!["$server".into(), "*".into(), "*.cdn.net".into(), "api.example.com".into(), "nas.local".into(), "10.0.0.9".into()], ..Default::default() };
        let setting = |k: &str| (k == "server").then(|| "http://jf.example.org:8096/".to_string());
        assert!(m.allows_host_with("jf.example.org", &setting));
        // The server the person gave may be anywhere; of the plugin's own names, only local ones.
        assert_eq!(m.home_hosts(&setting), vec!["10.0.0.9", "jf.example.org", "nas.local"]);
        // A server typed without its scheme names its host all the same.
        assert_eq!(m.home_hosts(&|_| Some("192.168.0.10:8096".into())), vec!["10.0.0.9", "192.168.0.10", "nas.local"]);
        assert!(m.home_hosts(&|_| None).iter().all(|h| h != "*"));

        let tuner = Manifest { fetch_hosts: vec![LOCAL_NETWORK.into()], ..Default::default() };
        assert!(tuner.allows_host("192.168.0.20") && tuner.allows_host("hdhr.local") && tuner.allows_host("fd00::5") && tuner.allows_host("tuner"));
        assert!(!tuner.allows_host("8.8.8.8") && !tuner.allows_host("evil.example"));
        assert!(tuner.reaches_local_network() && !m.reaches_local_network());
    }

    #[test]
    fn home_network_addresses() {
        for ip in ["10.1.2.3", "172.16.0.1", "192.168.1.1", "100.64.0.1", "169.254.1.1", "fd12::1", "fe80::1", "::ffff:192.168.1.1"] {
            assert!(is_local_network(ip.parse().unwrap()), "{ip}");
        }
        for ip in ["8.8.8.8", "100.128.0.1", "172.32.0.1", "2001:4860::8888"] {
            assert!(!is_local_network(ip.parse().unwrap()), "{ip}");
        }
    }

    #[test]
    fn manifest_validation() {
        let mut m = Manifest { id: "tv.lounge.iptv".into(), name: "IPTV".into(), version: "1.0".into(), ..Default::default() };
        assert!(m.validate().is_ok());
        m.id = "Bad Id".into();
        assert!(m.validate().is_err());
        let unknown = Manifest { id: "tv.lounge.iptv".into(), name: "IPTV".into(), version: "1.0".into(), fetch_hosts: vec!["$server".into()], ..Default::default() };
        assert!(unknown.validate().is_err());
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
