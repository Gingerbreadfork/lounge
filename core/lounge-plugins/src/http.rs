//! The one way out of the sandbox: HTTP(S) to hosts the manifest declared. Bodies are capped,
//! redirects are followed only to allowed hosts, and artwork downloads land in a cache directory.

use crate::api::{base64_encode, HttpRequest, HttpResponse};
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
    /// whatever name leads to them, except through a host in `local_hosts`.
    pub fn guarded(local_hosts: Vec<String>) -> Fetcher {
        Fetcher::with_resolver(GuardedResolver { inner: DefaultResolver::default(), local_hosts })
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
            let mut response = match method.as_str() {
                "POST" | "PUT" | "PATCH" => {
                    let mut b = match method.as_str() {
                        "POST" => self.agent.post(&url),
                        "PUT" => self.agent.put(&url),
                        _ => self.agent.patch(&url),
                    };
                    for (k, v) in &headers {
                        b = b.header(k, v);
                    }
                    b.config().timeout_global(Some(timeout)).build().send(body.as_deref().unwrap_or("")).map_err(|e| fetch_error(e, &url))?
                }
                _ => {
                    let mut b = match method.as_str() {
                        "DELETE" => self.agent.delete(&url),
                        "HEAD" => self.agent.head(&url),
                        _ => self.agent.get(&url),
                    };
                    for (k, v) in &headers {
                        b = b.header(k, v);
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
            let bytes = response
                .body_mut()
                .with_config()
                .limit(self.max_body as u64)
                .read_to_vec()
                .map_err(|e| fetch_error(e, &url))?;
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
            let mut response = self.agent.get(&url).config().timeout_global(Some(timeout)).build().call().map_err(|e| fetch_error(e, &url))?;
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
                    std::io::Write::write_all(&mut file, &buf[..n])?;
                    total += n as u64;
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

/// Refuses addresses that lead back into the device unless the plugin named that host itself.
#[derive(Debug)]
struct GuardedResolver {
    inner: DefaultResolver,
    local_hosts: Vec<String>,
}

impl Resolver for GuardedResolver {
    fn resolve(&self, uri: &Uri, config: &Config, timeout: NextTimeout) -> std::result::Result<ResolvedSocketAddrs, ureq::Error> {
        let addrs = self.inner.resolve(uri, config, timeout)?;
        let host = uri.host().unwrap_or("").trim_start_matches('[').trim_end_matches(']').trim_end_matches('.').to_ascii_lowercase();
        if self.local_hosts.contains(&host) {
            return Ok(addrs);
        }
        let mut out = self.empty();
        for addr in addrs.iter() {
            if !is_own_address(addr.ip()) {
                out.push(*addr);
            }
        }
        if out.is_empty() {
            return Err(ureq::Error::Io(std::io::Error::new(std::io::ErrorKind::PermissionDenied, format!("{host} leads to this device itself"))));
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
    fn a_guarded_fetcher_will_not_reach_loopback_by_any_name() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let fetcher = Fetcher::guarded(Vec::new());
        for url in [format!("http://127.0.0.1:{port}/"), format!("http://2130706433:{port}/"), format!("http://localhost:{port}/"), format!("http://[::ffff:127.0.0.1]:{port}/")] {
            let err = fetcher.fetch(&HttpRequest::get(url.clone()), &|_| true).unwrap_err();
            assert!(err.to_string().contains("this device"), "{url}: {err}");
        }
        let named = Fetcher::guarded(vec!["127.0.0.1".into()]);
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
}
