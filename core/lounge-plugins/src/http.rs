//! The one way out of the sandbox: HTTP(S) to hosts the manifest declared. Bodies are capped,
//! redirects are followed only to allowed hosts, and artwork downloads land in a cache directory.

use crate::api::{base64_encode, is_home_name, is_local_network, HttpRequest, HttpResponse, Manifest};
pub use crate::api::host_of;
use crate::{PluginError, Result};
use std::io::Read;
use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use ureq::config::Config;
use ureq::http::Uri;
use ureq::unversioned::resolver::{DefaultResolver, ResolvedSocketAddrs, Resolver};
use ureq::unversioned::transport::{DefaultConnector, NextTimeout};

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(20);

pub struct Fetcher {
    agent: ureq::Agent,
    pub max_body: usize,
}

impl Default for Fetcher {
    fn default() -> Self {
        Self::new()
    }
}

/// A finished artwork download.
pub struct Download {
    pub path: PathBuf,
    /// Bytes fetched from the network; 0 when the file was already cached.
    pub fetched: u64,
}

impl Fetcher {
    /// A fetcher that may connect anywhere, for addresses the person typed themselves.
    pub fn new() -> Fetcher {
        Fetcher::with_resolver(DefaultResolver::default())
    }

    /// A fetcher for a plugin: loopback, unspecified and this device's own addresses are refused
    /// whatever name leads to them, and home-network addresses too, except as `reach` allows.
    pub fn guarded(reach: Reach) -> Fetcher {
        Fetcher::with_resolver(GuardedResolver { inner: DefaultResolver::default(), reach })
    }

    fn with_resolver(resolver: impl Resolver) -> Fetcher {
        let config = ureq::Agent::config_builder()
            .timeout_global(Some(DEFAULT_TIMEOUT))
            .max_redirects(0)
            .http_status_as_error(false)
            .user_agent("Lounge/0.1 (Android TV; plugin host)")
            .build();
        Fetcher { agent: ureq::Agent::with_parts(config, DefaultConnector::default(), resolver), max_body: 8 * 1024 * 1024 }
    }

    /// Perform `req` on behalf of a plugin; `allowed` decides per host, including for redirects.
    /// The plugin's headers go only to the origin it addressed: a redirect elsewhere drops them.
    pub fn fetch(&self, req: &HttpRequest, allowed: &dyn Fn(&str) -> bool) -> Result<HttpResponse> {
        let timeout = if req.timeout_ms == 0 { DEFAULT_TIMEOUT } else { Duration::from_millis(req.timeout_ms as u64).min(DEFAULT_TIMEOUT) };
        let mut url = req.url.clone();
        let mut method = req.method.to_ascii_uppercase();
        let mut body = req.body.clone();
        let mut headers = req.headers.clone();
        for _hop in 0..5 {
            let host = host_of(&url).ok_or_else(|| PluginError::Fetch(format!("bad url {}", redact(&url))))?;
            if !allowed(&host) {
                return Err(PluginError::Denied(format!("host {host} is not in the plugin's allow list")));
            }
            let (address, basic) = credentials_of(&url);
            // A header the plugin set itself wins over a name and password in the URL.
            let basic = basic.filter(|_| !headers.keys().any(|k| k.eq_ignore_ascii_case("authorization")));
            let mut response = match method.as_str() {
                "POST" | "PUT" | "PATCH" => {
                    let mut b = match method.as_str() {
                        "POST" => self.agent.post(&address),
                        "PUT" => self.agent.put(&address),
                        _ => self.agent.patch(&address),
                    };
                    for (k, v) in &headers {
                        b = b.header(k, v);
                    }
                    if let Some(basic) = &basic {
                        b = b.header("Authorization", basic);
                    }
                    b.config().timeout_global(Some(timeout)).build().send(body.as_deref().unwrap_or("")).map_err(|e| fetch_error(e, &url))?
                }
                _ => {
                    let mut b = match method.as_str() {
                        "DELETE" => self.agent.delete(&address),
                        "HEAD" => self.agent.head(&address),
                        _ => self.agent.get(&address),
                    };
                    for (k, v) in &headers {
                        b = b.header(k, v);
                    }
                    if let Some(basic) = &basic {
                        b = b.header("Authorization", basic);
                    }
                    b.config().timeout_global(Some(timeout)).build().call().map_err(|e| fetch_error(e, &url))?
                }
            };
            let status = response.status().as_u16();
            if matches!(status, 301 | 302 | 303 | 307 | 308)
                && let Some(loc) = response.headers().get("location").and_then(|v| v.to_str().ok())
            {
                let next = resolve_relative(&url, loc);
                if origin_of(&next) != origin_of(&url) {
                    headers.clear();
                }
                if status == 303 || (matches!(status, 301 | 302) && method == "POST") {
                    method = "GET".into();
                    body = None;
                }
                url = next;
                continue;
            }
            let mut out_headers = std::collections::BTreeMap::new();
            for (k, v) in response.headers() {
                if let Ok(v) = v.to_str() {
                    out_headers.insert(k.as_str().to_ascii_lowercase(), v.to_string());
                }
            }
            // ureq's limit counts bytes before they are unpacked, so the unpacked body is capped too.
            let mut bytes = Vec::new();
            response
                .body_mut()
                .with_config()
                .limit(self.max_body as u64)
                .reader()
                .take(self.max_body as u64 + 1)
                .read_to_end(&mut bytes)
                .map_err(|e| fetch_error(e.into(), &url))?;
            if bytes.len() > self.max_body {
                return Err(fetch_error(ureq::Error::BodyExceedsLimit(self.max_body as u64), &url));
            }
            return Ok(match String::from_utf8(bytes) {
                Ok(text) => HttpResponse { status, headers: out_headers, body: text, body_base64: None },
                Err(e) => HttpResponse { status, headers: out_headers, body: String::new(), body_base64: Some(base64_encode(e.as_bytes())) },
            });
        }
        Err(PluginError::Fetch("too many redirects".into()))
    }

    /// The cached download of `url` in `dir`, if there is one.
    pub fn cached(url: &str, dir: &Path) -> Option<PathBuf> {
        let path = cache_path(url, dir);
        std::fs::metadata(&path).ok().filter(|m| m.len() > 0).map(|_| path)
    }

    /// Download artwork to `<dir>/<sha256(url)>.img`, reusing a file already there; `allowed`
    /// decides per host, including for redirects.
    pub fn download(&self, url: &str, dir: &Path, max: usize, timeout: Duration, allowed: &dyn Fn(&str) -> bool) -> Result<Download> {
        if let Some(path) = Self::cached(url, dir) {
            return Ok(Download { path, fetched: 0 });
        }
        std::fs::create_dir_all(dir)?;
        let path = cache_path(url, dir);
        let timeout = timeout.min(DEFAULT_TIMEOUT);
        let mut url = url.to_string();
        for _hop in 0..5 {
            let host = host_of(&url).ok_or_else(|| PluginError::Fetch(format!("bad url {}", redact(&url))))?;
            if !allowed(&host) {
                return Err(PluginError::Denied(format!("host {host} is not in the plugin's allow list")));
            }
            let (address, basic) = credentials_of(&url);
            let mut request = self.agent.get(&address);
            if let Some(basic) = &basic {
                request = request.header("Authorization", basic);
            }
            let mut response = request.config().timeout_global(Some(timeout)).build().call().map_err(|e| fetch_error(e, &url))?;
            let status = response.status().as_u16();
            if matches!(status, 301 | 302 | 303 | 307 | 308)
                && let Some(loc) = response.headers().get("location").and_then(|v| v.to_str().ok())
            {
                url = resolve_relative(&url, loc);
                continue;
            }
            if !(200..300).contains(&status) {
                return Err(PluginError::Fetch(format!("HTTP {status} for {}", redact(&url))));
            }
            // Each download writes its own temporary file, so two fetching one URL never collide.
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let tmp = path.with_extension(format!("{}-{}.tmp", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
            let written = (|| -> Result<u64> {
                let mut reader = response.body_mut().with_config().limit(max as u64).reader();
                let mut file = std::fs::File::create(&tmp)?;
                let mut buf = [0u8; 16 * 1024];
                let mut total = 0u64;
                loop {
                    let n = reader.read(&mut buf).map_err(|e| PluginError::Fetch(e.to_string()))?;
                    if n == 0 {
                        break;
                    }
                    // The reader's limit counts packed bytes; what lands on disk is held to `max`.
                    total += n as u64;
                    if total > max as u64 {
                        return Err(fetch_error(ureq::Error::BodyExceedsLimit(max as u64), &url));
                    }
                    std::io::Write::write_all(&mut file, &buf[..n])?;
                }
                Ok(total)
            })();
            let written = match written {
                Ok(0) => Err(PluginError::Fetch("empty body".into())),
                other => other,
            };
            return match written.and_then(|n| std::fs::rename(&tmp, &path).map(|_| n).map_err(PluginError::from)) {
                Ok(fetched) => Ok(Download { path, fetched }),
                Err(e) => {
                    let _ = std::fs::remove_file(&tmp);
                    Err(e)
                }
            };
        }
        Err(PluginError::Fetch("too many redirects".into()))
    }
}

fn cache_path(url: &str, dir: &Path) -> PathBuf {
    use sha2::Digest;
    let name = hex::encode(sha2::Sha256::digest(url.as_bytes()));
    dir.join(format!("{}.img", &name[..32]))
}

fn fetch_error(e: ureq::Error, url: &str) -> PluginError {
    PluginError::Fetch(format!("{e} ({})", redact(url)))
}

/// A URL fit for a log: no user name, password or query string.
pub fn redact(url: &str) -> String {
    let Some((scheme, rest)) = url.split_once("://") else { return url.split(['?', '#']).next().unwrap_or("").to_string() };
    let authority_end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let authority = rest[..authority_end].rsplit('@').next().unwrap_or("");
    let path = rest[authority_end..].split(['?', '#']).next().unwrap_or("");
    format!("{scheme}://{authority}{path}")
}

/// Where a plugin's fetches may lead besides the public internet.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Reach {
    /// Names that may lead into this device itself: loopback names and literals the plugin lists.
    pub this_device: Vec<String>,
    /// Names that may lead into the home network: those the plugin names exactly or was given.
    pub home: Vec<String>,
    /// Whether any home-network literal or `.local` name may lead there as well.
    pub any_home_name: bool,
}

/// Where a plugin's fetches may lead besides the public internet, with its settings filled in.
pub fn reach(manifest: &Manifest, setting: &dyn Fn(&str) -> Option<String>) -> Reach {
    Reach { this_device: local_hosts(manifest), home: manifest.home_hosts(setting), any_home_name: manifest.reaches_local_network() }
}

/// Hosts that may lead back into the TV: only loopback names and address literals the plugin
/// lists itself, so no DNS name can point a plugin at the TV's own services.
pub fn local_hosts(manifest: &Manifest) -> Vec<String> {
    let mut hosts: Vec<String> = manifest
        .fetch_hosts
        .iter()
        .map(|h| h.trim().trim_start_matches('[').trim_end_matches(']').to_ascii_lowercase())
        .filter(|h| h == "localhost" || h.ends_with(".localhost") || h.parse::<std::net::IpAddr>().is_ok())
        .collect();
    hosts.sort();
    hosts.dedup();
    hosts
}

/// The manifest's allow list, with the plugin's settings for its `$key` entries, applied to every
/// hop by name. The fetcher then checks the address each name resolves to, so loopback, the TV's
/// own addresses and the home network stay out of reach whatever the name; this check keeps the
/// obvious spellings from even being tried.
pub fn host_permitted(manifest: &Manifest, host: &str, setting: &dyn Fn(&str) -> Option<String>) -> bool {
    if !manifest.allows_host_with(host, setting) {
        return false;
    }
    let h = host.to_ascii_lowercase();
    let local = h == "localhost" || h == "0.0.0.0" || h == "::1" || h == "::" || h.starts_with("127.") || h.ends_with(".localhost");
    !local || manifest.fetch_hosts.iter().any(|allowed| allowed.eq_ignore_ascii_case(&h))
}

/// Refuses addresses that lead back into the device, and into the home network for a name the
/// plugin did not name or was not given, so a public name cannot be pointed at a router or a NAS.
#[derive(Debug)]
struct GuardedResolver {
    inner: DefaultResolver,
    reach: Reach,
}

impl Resolver for GuardedResolver {
    fn resolve(&self, uri: &Uri, config: &Config, timeout: NextTimeout) -> std::result::Result<ResolvedSocketAddrs, ureq::Error> {
        let addrs = self.inner.resolve(uri, config, timeout)?;
        let host = uri.host().unwrap_or("").trim_start_matches('[').trim_end_matches(']').trim_end_matches('.').to_ascii_lowercase();
        if self.reach.this_device.contains(&host) {
            return Ok(addrs);
        }
        let home = self.reach.home.contains(&host) || (self.reach.any_home_name && is_home_name(&host));
        let mut out = self.empty();
        let mut kept_out_of_home = false;
        for addr in addrs.iter() {
            if is_own_address(addr.ip()) {
                continue;
            }
            if !home && is_local_network(addr.ip()) {
                kept_out_of_home = true;
                continue;
            }
            out.push(*addr);
        }
        if out.is_empty() {
            let why = if kept_out_of_home { "leads into the home network, where the plugin may reach only the addresses it is given" } else { "leads to this device itself" };
            return Err(ureq::Error::Io(std::io::Error::new(std::io::ErrorKind::PermissionDenied, format!("{host} {why}"))));
        }
        Ok(out)
    }
}

/// Loopback, unspecified, or an address one of this device's interfaces holds.
pub fn is_own_address(ip: IpAddr) -> bool {
    let ip = match ip {
        IpAddr::V6(v6) => v6.to_ipv4_mapped().map(IpAddr::V4).unwrap_or(IpAddr::V6(v6)),
        v4 => v4,
    };
    let this_network = matches!(ip, IpAddr::V4(v4) if v4.octets()[0] == 0);
    if ip.is_loopback() || ip.is_unspecified() || this_network {
        return true;
    }
    // Binding succeeds only to an address the device itself holds.
    std::net::UdpSocket::bind(SocketAddr::new(ip, 0)).is_ok()
}

/// `url` without a user name and password, and those as an `Authorization: Basic` value. ureq
/// would send them still percent-encoded, and a password with a symbol in it would never match.
fn credentials_of(url: &str) -> (String, Option<String>) {
    let Some((scheme, rest)) = url.split_once("://") else { return (url.to_string(), None) };
    let authority_end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let Some((userinfo, hostport)) = rest[..authority_end].rsplit_once('@') else { return (url.to_string(), None) };
    let (user, password) = userinfo.split_once(':').unwrap_or((userinfo, ""));
    let pair = format!("{}:{}", percent_decode(user), percent_decode(password));
    (format!("{scheme}://{hostport}{}", &rest[authority_end..]), Some(format!("Basic {}", base64_encode(pair.as_bytes()))))
}

/// `%XX` escapes turned back into the bytes they stand for; anything else is left as it is.
fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let escaped = (bytes[i] == b'%' && i + 3 <= bytes.len())
            .then(|| std::str::from_utf8(&bytes[i + 1..i + 3]).ok().and_then(|hex| u8::from_str_radix(hex, 16).ok()))
            .flatten();
        match escaped {
            Some(b) => {
                out.push(b);
                i += 3;
            }
            None => {
                out.push(bytes[i]);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Scheme, host and port, for telling whether a redirect leaves the origin.
fn origin_of(url: &str) -> Option<(String, String, Option<u16>)> {
    let (scheme, rest) = url.split_once("://")?;
    let authority = rest.split(['/', '?', '#']).next()?;
    let hostport = authority.rsplit('@').next()?;
    let port = if hostport.starts_with('[') {
        hostport.rsplit_once("]:").and_then(|(_, p)| p.parse().ok())
    } else {
        hostport.rsplit_once(':').and_then(|(_, p)| p.parse().ok())
    };
    let scheme = scheme.to_ascii_lowercase();
    let port = port.or(match scheme.as_str() {
        "http" => Some(80),
        "https" => Some(443),
        _ => None,
    });
    Some((scheme, host_of(url)?, port))
}

fn has_scheme(reference: &str) -> bool {
    let end = reference.find([':', '/', '?', '#']);
    match end {
        Some(i) if i > 0 && reference.as_bytes()[i] == b':' => {
            let scheme = &reference[..i];
            scheme.chars().next().is_some_and(|c| c.is_ascii_alphabetic()) && scheme.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
        }
        _ => false,
    }
}

/// Resolve a `Location` against the URL that sent it, as RFC 3986 section 5.2 does.
pub fn resolve_relative(base: &str, reference: &str) -> String {
    let reference = reference.trim();
    if has_scheme(reference) {
        return reference.to_string();
    }
    let Some((scheme, rest)) = base.split_once("://") else { return reference.to_string() };
    if let Some(r) = reference.strip_prefix("//") {
        return format!("{scheme}://{r}");
    }
    let authority_end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let authority = &rest[..authority_end];
    let after = rest[authority_end..].split('#').next().unwrap_or("");
    let base_path = after.split('?').next().unwrap_or("");
    if reference.is_empty() {
        return format!("{scheme}://{authority}{after}");
    }
    if reference.starts_with('#') {
        return format!("{scheme}://{authority}{after}{reference}");
    }
    if reference.starts_with('?') {
        return format!("{scheme}://{authority}{base_path}{reference}");
    }
    let split = reference.find(['?', '#']).unwrap_or(reference.len());
    let (ref_path, tail) = reference.split_at(split);
    let merged = if ref_path.starts_with('/') {
        ref_path.to_string()
    } else if base_path.is_empty() {
        format!("/{ref_path}")
    } else {
        format!("{}{ref_path}", &base_path[..base_path.rfind('/').map(|i| i + 1).unwrap_or(0)])
    };
    format!("{scheme}://{authority}{}{tail}", remove_dot_segments(&merged))
}

fn remove_dot_segments(path: &str) -> String {
    let mut out: Vec<&str> = Vec::new();
    let segments: Vec<&str> = path.split('/').collect();
    let last = segments.len().saturating_sub(1);
    for (i, seg) in segments.iter().enumerate() {
        match *seg {
            "." => {
                if i == last {
                    out.push("");
                }
            }
            ".." => {
                if out.len() > 1 {
                    out.pop();
                }
                if i == last {
                    out.push("");
                }
            }
            s => out.push(s),
        }
    }
    let joined = out.join("/");
    if joined.starts_with('/') { joined } else { format!("/{joined}") }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hosts_and_redirects() {
        assert_eq!(host_of("https://user:pw@API.Example.com:8096/path?q=1").as_deref(), Some("api.example.com"));
        assert_eq!(host_of("nonsense"), None);
        assert_eq!(host_of("http://[::1]:8787/api").as_deref(), Some("::1"));
        assert_eq!(host_of("http://[fe80::1%25eth0]/x").as_deref(), Some("fe80::1%25eth0"));
        assert_eq!(resolve_relative("https://a.com/x/y", "/z"), "https://a.com/z");
        assert_eq!(resolve_relative("https://a.com/x/y", "z"), "https://a.com/x/z");
        assert_eq!(resolve_relative("https://a.com/x/y", "https://b.com/q"), "https://b.com/q");
    }

    #[test]
    fn locations_resolve_like_a_browser() {
        let base = "http://a.com:8096/web/index.html?x=1#top";
        assert_eq!(resolve_relative(base, "/login?next=https://app/"), "http://a.com:8096/login?next=https://app/");
        assert_eq!(resolve_relative(base, "//cdn.b.com/i.jpg"), "http://cdn.b.com/i.jpg");
        assert_eq!(resolve_relative(base, "?page=2"), "http://a.com:8096/web/index.html?page=2");
        assert_eq!(resolve_relative(base, "../img/a.png"), "http://a.com:8096/img/a.png");
        assert_eq!(resolve_relative(base, "./b/../c"), "http://a.com:8096/web/c");
        assert_eq!(resolve_relative("http://a.com", "x"), "http://a.com/x");
        assert_eq!(resolve_relative(base, "HTTPS://Other.org/"), "HTTPS://Other.org/");
    }

    #[test]
    fn origins_include_the_port() {
        assert_eq!(origin_of("http://a.com/x"), origin_of("HTTP://A.com:80/y"));
        assert_ne!(origin_of("http://a.com/x"), origin_of("https://a.com/x"));
        assert_ne!(origin_of("http://a.com:8096/x"), origin_of("http://a.com:8920/x"));
        assert_ne!(origin_of("http://a.com/x"), origin_of("http://b.com/x"));
        assert_eq!(origin_of("http://[::1]:8787/"), Some(("http".into(), "::1".into(), Some(8787))));
    }

    #[test]
    fn redaction_keeps_only_where() {
        assert_eq!(redact("http://u:p@nextpvr:8866/service?method=x&sid=secret"), "http://nextpvr:8866/service");
        assert_eq!(redact("https://a.com"), "https://a.com");
    }

    #[test]
    fn own_addresses_are_recognised() {
        for ip in ["127.0.0.1", "127.8.9.1", "0.0.0.0", "0.1.2.3", "::1", "::", "::ffff:127.0.0.1"] {
            assert!(is_own_address(ip.parse().unwrap()), "{ip}");
        }
        assert!(!is_own_address("192.0.2.77".parse().unwrap()));
        assert!(!is_own_address("8.8.8.8".parse().unwrap()));
    }

    #[test]
    fn only_literal_local_names_may_reach_the_tv() {
        let m = Manifest { fetch_hosts: vec!["*".into(), "localhost".into(), "[::1]".into(), "192.168.1.5".into(), "evil.example".into(), "*.local".into()], ..Default::default() };
        assert_eq!(local_hosts(&m), vec!["192.168.1.5", "::1", "localhost"]);
    }

    #[test]
    fn credentials_in_a_url_become_a_decoded_basic_header() {
        let (address, basic) = credentials_of("http://kodi:p%40ss%21@tv.lan:8080/image/x?y=1");
        assert_eq!(address, "http://tv.lan:8080/image/x?y=1");
        assert_eq!(basic.as_deref(), Some(format!("Basic {}", base64_encode(b"kodi:p@ss!")).as_str()));
        assert_eq!(credentials_of("https://example.com/a@b"), ("https://example.com/a@b".to_string(), None));
        assert_eq!(percent_decode("100%"), "100%");
    }

    #[test]
    fn only_hosts_a_plugin_was_given_lead_into_the_home_network() {
        let request = |url: &str| HttpRequest { timeout_ms: 300, ..HttpRequest::get(url.to_string()) };
        let err = Fetcher::guarded(Reach::default()).fetch(&request("http://192.168.77.1:9/"), &|_| true).unwrap_err();
        assert!(err.to_string().contains("home network"), "{err}");
        // Named, or covered by the home network entry, the address is tried and simply does not answer.
        for reach in [Reach { home: vec!["192.168.77.1".into()], ..Reach::default() }, Reach { any_home_name: true, ..Reach::default() }] {
            let err = Fetcher::guarded(reach).fetch(&request("http://192.168.77.1:9/"), &|_| true).unwrap_err();
            assert!(!err.to_string().contains("home network"), "{err}");
        }
    }

    #[test]
    fn a_guarded_fetcher_will_not_reach_loopback_by_any_name() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let fetcher = Fetcher::guarded(Reach::default());
        for url in [format!("http://127.0.0.1:{port}/"), format!("http://2130706433:{port}/"), format!("http://localhost:{port}/"), format!("http://[::ffff:127.0.0.1]:{port}/")] {
            let err = fetcher.fetch(&HttpRequest::get(url.clone()), &|_| true).unwrap_err();
            assert!(err.to_string().contains("this device"), "{url}: {err}");
        }
        let named = Fetcher::guarded(Reach { this_device: vec!["127.0.0.1".into()], ..Reach::default() });
        std::thread::spawn(move || {
            if let Ok((mut s, _)) = listener.accept() {
                let mut buf = [0u8; 1024];
                let _ = s.read(&mut buf);
                let _ = std::io::Write::write_all(&mut s, b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\nconnection: close\r\n\r\nok");
            }
        });
        let r = named.fetch(&HttpRequest::get(format!("http://127.0.0.1:{port}/")), &|_| true).unwrap();
        assert_eq!(r.body, "ok");
    }

    #[test]
    fn headers_stay_with_their_origin() {
        // Two servers: the first redirects to the second, which reports what it was sent.
        let second = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let second_port = second.local_addr().unwrap().port();
        let first = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let first_port = first.local_addr().unwrap().port();
        std::thread::spawn(move || {
            if let Ok((mut s, _)) = first.accept() {
                let mut buf = [0u8; 2048];
                let _ = s.read(&mut buf);
                let reply = format!("HTTP/1.1 302 Found\r\nlocation: http://localhost:{second_port}/next\r\ncontent-length: 0\r\nconnection: close\r\n\r\n");
                let _ = std::io::Write::write_all(&mut s, reply.as_bytes());
            }
        });
        std::thread::spawn(move || {
            if let Ok((mut s, _)) = second.accept() {
                let mut buf = [0u8; 2048];
                let n = s.read(&mut buf).unwrap_or(0);
                let seen = String::from_utf8_lossy(&buf[..n]).to_ascii_lowercase();
                let body = if seen.contains("x-plex-token") { "leaked" } else { "clean" };
                let reply = format!("HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}", body.len());
                let _ = std::io::Write::write_all(&mut s, reply.as_bytes());
            }
        });
        let fetcher = Fetcher::new();
        let req = HttpRequest::get(format!("http://127.0.0.1:{first_port}/start")).header("X-Plex-Token", "secret");
        let r = fetcher.fetch(&req, &|_| true).unwrap();
        assert_eq!(r.body, "clean");
    }

    #[test]
    fn a_compressed_body_is_capped_at_its_unpacked_size() {
        // Zeros pack down to far less than the cap, so only the unpacked size can exceed it.
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::best());
        std::io::Write::write_all(&mut gz, &vec![0u8; 1 << 20]).unwrap();
        let packed = gz.finish().unwrap();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            for _ in 0..2 {
                if let Ok((mut s, _)) = listener.accept() {
                    let mut buf = [0u8; 2048];
                    let _ = s.read(&mut buf);
                    let head = format!("HTTP/1.1 200 OK\r\ncontent-encoding: gzip\r\ncontent-length: {}\r\nconnection: close\r\n\r\n", packed.len());
                    let _ = std::io::Write::write_all(&mut s, head.as_bytes());
                    let _ = std::io::Write::write_all(&mut s, &packed);
                }
            }
        });
        let mut fetcher = Fetcher::new();
        fetcher.max_body = 64 * 1024;
        let Err(err) = fetcher.fetch(&HttpRequest::get(format!("http://127.0.0.1:{port}/")), &|_| true) else {
            panic!("an oversized body was read");
        };
        assert!(err.to_string().contains("body is larger than"), "{err}");
        let dir = tempfile::tempdir().unwrap();
        let Err(err) = fetcher.download(&format!("http://127.0.0.1:{port}/art"), dir.path(), 64 * 1024, Duration::from_secs(5), &|_| true) else {
            panic!("an oversized download was kept");
        };
        assert!(err.to_string().contains("body is larger than"), "{err}");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }
}
