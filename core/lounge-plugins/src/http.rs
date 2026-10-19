//! The one way out of the sandbox: HTTP(S) to hosts the manifest declared. Bodies are capped,
//! redirects are followed only to allowed hosts, and artwork downloads land in a cache directory.

use crate::api::{base64_encode, HttpRequest, HttpResponse};
use crate::{PluginError, Result};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

pub struct Fetcher {
    agent: ureq::Agent,
    pub max_body: usize,
}

impl Default for Fetcher {
    fn default() -> Self {
        Self::new()
    }
}

impl Fetcher {
    pub fn new() -> Fetcher {
        let config = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(20)))
            .max_redirects(0)
            .http_status_as_error(false)
            .user_agent("Lounge/0.1 (Android TV; plugin host)")
            .build();
        Fetcher { agent: config.into(), max_body: 8 * 1024 * 1024 }
    }

    /// Perform `req` on behalf of a plugin; `allowed` decides per host, including for redirects.
    pub fn fetch(&self, req: &HttpRequest, allowed: &dyn Fn(&str) -> bool) -> Result<HttpResponse> {
        let mut url = req.url.clone();
        for _hop in 0..5 {
            let host = host_of(&url).ok_or_else(|| PluginError::Fetch(format!("bad url {url}")))?;
            if !allowed(&host) {
                return Err(PluginError::Denied(format!("host {host} is not in the plugin's allow list")));
            }
            let method = req.method.to_ascii_uppercase();
            let mut builder = match method.as_str() {
                "GET" => self.agent.get(&url),
                "DELETE" => self.agent.delete(&url),
                "HEAD" => self.agent.head(&url),
                _ => self.agent.get(&url),
            };
            let mut response = if matches!(method.as_str(), "POST" | "PUT" | "PATCH") {
                let mut b = match method.as_str() {
                    "POST" => self.agent.post(&url),
                    "PUT" => self.agent.put(&url),
                    _ => self.agent.patch(&url),
                };
                for (k, v) in &req.headers {
                    b = b.header(k, v);
                }
                b.send(req.body.as_deref().unwrap_or("")).map_err(|e| PluginError::Fetch(e.to_string()))?
            } else {
                for (k, v) in &req.headers {
                    builder = builder.header(k, v);
                }
                builder.call().map_err(|e| PluginError::Fetch(e.to_string()))?
            };
            let status = response.status().as_u16();
            if (301..=308).contains(&status) {
                if let Some(loc) = response.headers().get("location").and_then(|v| v.to_str().ok()) {
                    url = resolve_relative(&url, loc);
                    continue;
                }
            }
            let mut headers = std::collections::BTreeMap::new();
            for (k, v) in response.headers() {
                if let Ok(v) = v.to_str() {
                    headers.insert(k.as_str().to_ascii_lowercase(), v.to_string());
                }
            }
            let bytes = response
                .body_mut()
                .with_config()
                .limit(self.max_body as u64)
                .read_to_vec()
                .map_err(|e| PluginError::Fetch(e.to_string()))?;
            return Ok(match String::from_utf8(bytes) {
                Ok(text) => HttpResponse { status, headers, body: text, body_base64: None },
                Err(e) => HttpResponse { status, headers, body: String::new(), body_base64: Some(base64_encode(e.as_bytes())) },
            });
        }
        Err(PluginError::Fetch("too many redirects".into()))
    }

    /// Download artwork to `<dir>/<sha256(url)>.img`, reusing a file already there; `allowed`
    /// decides per host, including for redirects.
    pub fn download(&self, url: &str, dir: &Path, max: usize, allowed: &dyn Fn(&str) -> bool) -> Result<PathBuf> {
        std::fs::create_dir_all(dir)?;
        let name = {
            use sha2::Digest;
            hex::encode(sha2::Sha256::digest(url.as_bytes()))
        };
        let path = dir.join(format!("{}.img", &name[..32]));
        if path.exists() && std::fs::metadata(&path).map(|m| m.len() > 0).unwrap_or(false) {
            return Ok(path);
        }
        let mut url = url.to_string();
        for _hop in 0..5 {
            let host = host_of(&url).ok_or_else(|| PluginError::Fetch(format!("bad url {url}")))?;
            if !allowed(&host) {
                return Err(PluginError::Denied(format!("host {host} is not in the plugin's allow list")));
            }
            let mut response = self.agent.get(&url).call().map_err(|e| PluginError::Fetch(e.to_string()))?;
            let status = response.status().as_u16();
            if (301..=308).contains(&status) {
                if let Some(loc) = response.headers().get("location").and_then(|v| v.to_str().ok()) {
                    url = resolve_relative(&url, loc);
                    continue;
                }
            }
            if !(200..300).contains(&status) {
                return Err(PluginError::Fetch(format!("HTTP {status} for {url}")));
            }
            let tmp = path.with_extension("tmp");
            {
                let mut reader = response.body_mut().with_config().limit(max as u64).reader();
                let mut file = std::fs::File::create(&tmp)?;
                let mut buf = [0u8; 16 * 1024];
                loop {
                    let n = reader.read(&mut buf).map_err(|e| PluginError::Fetch(e.to_string()))?;
                    if n == 0 {
                        break;
                    }
                    std::io::Write::write_all(&mut file, &buf[..n])?;
                }
            }
            if std::fs::metadata(&tmp)?.len() == 0 {
                let _ = std::fs::remove_file(&tmp);
                return Err(PluginError::Fetch("empty body".into()));
            }
            std::fs::rename(&tmp, &path)?;
            return Ok(path);
        }
        Err(PluginError::Fetch("too many redirects".into()))
    }
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

fn resolve_relative(base: &str, location: &str) -> String {
    if location.contains("://") {
        return location.to_string();
    }
    let scheme_end = base.find("://").map(|i| i + 3).unwrap_or(0);
    let path_start = base[scheme_end..].find('/').map(|i| i + scheme_end).unwrap_or(base.len());
    if location.starts_with('/') {
        format!("{}{}", &base[..path_start], location)
    } else {
        let dir_end = base.rfind('/').map(|i| i + 1).unwrap_or(base.len());
        format!("{}{}", &base[..dir_end.max(path_start + 1).min(base.len())], location)
    }
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
}
