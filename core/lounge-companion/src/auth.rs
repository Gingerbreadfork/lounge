//! Bearer tokens for paired phones, kept hashed in a JSON file in the data directory, and
//! short-lived pairing codes that exchange for a token.

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::net::IpAddr;
use std::path::PathBuf;

const CODE_TTL_MS: i64 = 5 * 60_000;
/// Wrong codes in a row before pairing locks for that address; the lockout doubles each time,
/// up to ten minutes.
const WRONG_CODES_BEFORE_LOCK: u32 = 5;
const FIRST_LOCKOUT_MS: i64 = 30_000;
const MAX_LOCKOUT_MS: i64 = 10 * 60_000;
/// Wrong codes from everyone together within a code's lifetime before typing codes pauses; with
/// a million codes that keeps guessing hopeless. The QR link keeps working meanwhile.
const WRONG_CODES_WINDOW: usize = 30;
/// Addresses remembered for lockouts; the oldest are forgotten first.
const TRACKED_ADDRESSES: usize = 256;

#[derive(Debug, PartialEq, Eq)]
pub enum RedeemError {
    Wrong,
    LockedOut,
}

#[derive(Default)]
struct Guesses {
    wrong: u32,
    lockouts: u32,
    locked_until_ms: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Device {
    pub id: String,
    pub name: String,
    pub created_ms: i64,
    pub last_seen_ms: i64,
    /// sha256 of the bearer token, hex.
    token_hash: String,
    /// sha256 of the session cookie, hex; empty until the phone asks for one.
    #[serde(default)]
    session_hash: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PairingInfo {
    pub code: String,
    /// The token the link carries; the code hands out a different one.
    pub token: String,
    pub url: String,
    pub expires_ms: i64,
}

/// A live code and link, each with its own token, so whoever photographs the code on screen can
/// never end up with the token of the phone that typed it. Using either retires both.
struct Pending {
    link_token: String,
    code_token: String,
    expires_ms: i64,
}

#[derive(Default)]
struct Lockouts {
    by_address: HashMap<IpAddr, Guesses>,
    /// When each recent wrong code arrived, from anyone.
    recent_wrong: VecDeque<i64>,
}

pub struct TokenStore {
    path: PathBuf,
    devices: Mutex<Vec<Device>>,
    pending: Mutex<HashMap<String, Pending>>,
    lockouts: Mutex<Lockouts>,
}

impl TokenStore {
    pub fn open(data_dir: &std::path::Path) -> TokenStore {
        let path = data_dir.join("companion-devices.json");
        let devices = match std::fs::read_to_string(&path) {
            Ok(text) => match serde_json::from_str(&text) {
                Ok(devices) => devices,
                Err(e) => {
                    tracing::warn!("companion: paired devices file is unreadable ({e}); starting with none");
                    Vec::new()
                }
            },
            Err(_) => Vec::new(),
        };
        TokenStore { path, devices: Mutex::new(devices), pending: Mutex::new(HashMap::new()), lockouts: Mutex::new(Lockouts::default()) }
    }

    fn save(&self, devices: &[Device]) {
        let Ok(json) = serde_json::to_string_pretty(devices) else { return };
        let tmp = self.path.with_extension("json.tmp");
        if std::fs::write(&tmp, json).and_then(|_| std::fs::rename(&tmp, &self.path)).is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
    }

    /// The paired devices, marking the one whose id is `current`.
    pub fn devices(&self, current: Option<&str>) -> Vec<serde_json::Value> {
        self.devices
            .lock()
            .iter()
            .map(|d| {
                serde_json::json!({
                    "id": d.id, "name": d.name, "createdMs": d.created_ms, "lastSeenMs": d.last_seen_ms,
                    "current": current == Some(d.id.as_str()),
                })
            })
            .collect()
    }

    pub fn count(&self) -> usize {
        self.devices.lock().len()
    }

    /// A new six-digit code and the token it will hand out; the URL pairs by itself when opened.
    /// Any earlier code stops working: one live code at a time.
    pub fn new_pairing(&self, base_url: &str, now_ms: i64) -> Result<PairingInfo, String> {
        let failed = || "the TV's random number source failed, so no code was made".to_string();
        let (link_token, code_token) = (random_token().ok_or_else(failed)?, random_token().ok_or_else(failed)?);
        let code = format!("{:06}", random_u32().ok_or_else(failed)? % 1_000_000);
        let expires_ms = now_ms + CODE_TTL_MS;
        let mut pending = self.pending.lock();
        pending.clear();
        pending.insert(code.clone(), Pending { link_token: link_token.clone(), code_token, expires_ms });
        Ok(PairingInfo { url: format!("{base_url}/#token={link_token}"), code, token: link_token, expires_ms })
    }

    /// Exchange a code for its token and register the device. Wrong codes lock out the address
    /// that sent them for a while, and too many from anyone pause typed codes, so a million
    /// guesses never get anywhere and no one address can keep everyone else out.
    pub fn redeem(&self, code: &str, device_name: &str, from: IpAddr, now_ms: i64) -> Result<String, RedeemError> {
        {
            let mut l = self.lockouts.lock();
            while l.recent_wrong.front().is_some_and(|&t| t <= now_ms - CODE_TTL_MS) {
                l.recent_wrong.pop_front();
            }
            if l.recent_wrong.len() >= WRONG_CODES_WINDOW || l.by_address.get(&from).is_some_and(|g| g.locked_until_ms > now_ms) {
                return Err(RedeemError::LockedOut);
            }
        }
        let pending = self.pending.lock().remove(code).filter(|p| p.expires_ms >= now_ms);
        match pending {
            Some(pending) => {
                self.lockouts.lock().by_address.remove(&from);
                self.register(&pending.code_token, device_name, now_ms);
                Ok(pending.code_token)
            }
            None => {
                let mut l = self.lockouts.lock();
                l.recent_wrong.push_back(now_ms);
                if l.by_address.len() >= TRACKED_ADDRESSES && !l.by_address.contains_key(&from) {
                    let unlocked: Vec<IpAddr> = l.by_address.iter().filter(|(_, g)| g.locked_until_ms <= now_ms).map(|(ip, _)| *ip).collect();
                    for ip in unlocked {
                        l.by_address.remove(&ip);
                    }
                }
                let g = l.by_address.entry(from).or_default();
                g.wrong += 1;
                if g.wrong >= WRONG_CODES_BEFORE_LOCK {
                    g.wrong = 0;
                    g.lockouts += 1;
                    g.locked_until_ms = now_ms + (FIRST_LOCKOUT_MS << (g.lockouts - 1).min(5)).min(MAX_LOCKOUT_MS);
                    return Err(RedeemError::LockedOut);
                }
                Err(RedeemError::Wrong)
            }
        }
    }

    /// A token from a pairing URL registers itself on first use. Returns the device's id.
    pub fn register(&self, token: &str, device_name: &str, now_ms: i64) -> String {
        let h = hash(token);
        let mut devices = self.devices.lock();
        if let Some(d) = devices.iter().find(|d| d.token_hash == h) {
            return d.id.clone();
        }
        // The id only names the device in the list; without randomness the hash serves.
        let id = random_token().map(|t| t[..12].to_string()).unwrap_or_else(|| h[..12].to_string());
        devices.push(Device { id: id.clone(), name: device_name.to_string(), created_ms: now_ms, last_seen_ms: now_ms, token_hash: h, session_hash: String::new() });
        self.save(&devices);
        id
    }

    /// The id of the device a token was handed out to, whether registered or still pending from
    /// a QR link. A token from a QR link registers its phone on first use, under `device_name`.
    pub fn authenticate(&self, token: &str, now_ms: i64, device_name: &str) -> Option<String> {
        let h = hash(token);
        if let Some(id) = self.seen(|d| d.token_hash == h, now_ms) {
            return Some(id);
        }
        let mut pending = self.pending.lock();
        let code = pending.iter().find(|(_, p)| p.link_token == token && p.expires_ms >= now_ms).map(|(c, _)| c.clone())?;
        pending.remove(&code);
        drop(pending);
        Some(self.register(token, device_name, now_ms))
    }

    /// The id of the device a session cookie was handed to. The cookie is its own secret, so it
    /// never works where a token is asked for.
    pub fn authenticate_session(&self, session: &str, now_ms: i64) -> Option<String> {
        let h = hash(session);
        self.seen(|d| d.session_hash == h, now_ms)
    }

    /// A fresh session cookie value for a paired device, replacing its last one. None when the
    /// device is gone or the system cannot give randomness.
    pub fn new_session(&self, device_id: &str) -> Option<String> {
        let session = random_token()?;
        let mut devices = self.devices.lock();
        let d = devices.iter_mut().find(|d| d.id == device_id)?;
        d.session_hash = hash(&session);
        self.save(&devices);
        Some(session)
    }

    /// The id of the first device that matches, noting that it was just seen.
    fn seen(&self, matches: impl Fn(&Device) -> bool, now_ms: i64) -> Option<String> {
        let mut devices = self.devices.lock();
        let d = devices.iter_mut().find(|d| matches(d))?;
        let id = d.id.clone();
        if now_ms - d.last_seen_ms > 60_000 {
            d.last_seen_ms = now_ms;
            let snapshot = devices.clone();
            drop(devices);
            self.save(&snapshot);
        }
        Some(id)
    }

    pub fn revoke(&self, id: &str) -> bool {
        let mut devices = self.devices.lock();
        let before = devices.len();
        devices.retain(|d| d.id != id);
        let changed = devices.len() != before;
        if changed {
            self.save(&devices);
        }
        changed
    }

    pub fn revoke_all(&self) {
        let mut devices = self.devices.lock();
        devices.clear();
        self.save(&devices);
        self.pending.lock().clear();
    }
}

fn hash(token: &str) -> String {
    use sha2::Digest;
    hex::encode(sha2::Sha256::digest(token.as_bytes()))
}

/// None when the system cannot give randomness, which must never become a guessable code.
fn random_u32() -> Option<u32> {
    let mut b = [0u8; 4];
    getrandom::fill(&mut b).ok()?;
    Some(u32::from_le_bytes(b))
}

fn random_token() -> Option<String> {
    let mut b = [0u8; 24];
    getrandom::fill(&mut b).ok()?;
    // URL-safe without padding, so it survives a QR code and a fragment.
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    Some(b.iter().map(|x| ALPHABET[(*x & 63) as usize] as char).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    const PHONE: IpAddr = IpAddr::V4(std::net::Ipv4Addr::new(192, 168, 0, 20));
    const PEST: IpAddr = IpAddr::V4(std::net::Ipv4Addr::new(192, 168, 0, 66));

    #[test]
    fn pairing_code_exchanges_once() {
        let dir = tempfile::tempdir().unwrap();
        let store = TokenStore::open(dir.path());
        let p = store.new_pairing("http://tv:8787", 1_000).unwrap();
        assert_eq!(p.code.len(), 6);
        assert!(store.redeem("000000", "Phone", PHONE, 1_000).is_err() || p.code == "000000");
        let token = store.redeem(&p.code, "Pixel", PHONE, 2_000).unwrap();
        // The code's token is the phone's own; the link's, seen on screen, is retired with it.
        assert_ne!(token, p.token);
        assert!(store.authenticate(&p.token, 2_500, "Camera").is_none());
        assert!(store.redeem(&p.code, "Pixel", PHONE, 2_000).is_err());
        assert!(store.authenticate(&token, 3_000, "Phone").is_some());
        assert!(store.authenticate("nope", 3_000, "Phone").is_none());
        assert_eq!(store.count(), 1);
        let reopened = TokenStore::open(dir.path());
        assert!(reopened.authenticate(&token, 4_000, "Phone").is_some());
    }

    fn wrong_code(p: &PairingInfo) -> &'static str {
        if p.code == "000001" { "000002" } else { "000001" }
    }

    #[test]
    fn wrong_codes_lock_out_only_the_guesser() {
        let dir = tempfile::tempdir().unwrap();
        let store = TokenStore::open(dir.path());
        let p = store.new_pairing("http://tv:8787", 0).unwrap();
        let wrong = wrong_code(&p);
        for _ in 0..4 {
            assert_eq!(store.redeem(wrong, "x", PEST, 10), Err(RedeemError::Wrong));
        }
        assert_eq!(store.redeem(wrong, "x", PEST, 10), Err(RedeemError::LockedOut));
        assert_eq!(store.redeem(&p.code, "x", PEST, 20), Err(RedeemError::LockedOut));
        // Someone else on the network can still pair with the live code.
        assert!(store.redeem(&p.code, "Pixel", PHONE, 30).is_ok());
    }

    #[test]
    fn too_many_wrong_codes_from_anyone_pause_typed_codes() {
        let dir = tempfile::tempdir().unwrap();
        let store = TokenStore::open(dir.path());
        let p = store.new_pairing("http://tv:8787", 0).unwrap();
        let wrong = wrong_code(&p);
        for i in 0..WRONG_CODES_WINDOW {
            let ip = IpAddr::V4(std::net::Ipv4Addr::new(10, 0, (i / 200) as u8, (i % 200) as u8 + 1));
            let _ = store.redeem(wrong, "x", ip, 10);
        }
        assert_eq!(store.redeem(&p.code, "x", PHONE, 20), Err(RedeemError::LockedOut));
        // The QR link still pairs, and typed codes work again once the window has passed.
        assert!(store.authenticate(&p.token, 30, "Phone").is_some());
        let p2 = store.new_pairing("http://tv:8787", CODE_TTL_MS + 100).unwrap();
        assert!(store.redeem(&p2.code, "x", PHONE, CODE_TTL_MS + 200).is_ok());
    }

    #[test]
    fn a_new_code_retires_the_old_one() {
        let dir = tempfile::tempdir().unwrap();
        let store = TokenStore::open(dir.path());
        let old = store.new_pairing("http://tv:8787", 0).unwrap();
        let new = store.new_pairing("http://tv:8787", 10).unwrap();
        if old.code != new.code {
            assert_eq!(store.redeem(&old.code, "x", PHONE, 20), Err(RedeemError::Wrong));
        }
        assert!(store.authenticate(&old.token, 20, "Phone").is_none());
        assert!(store.redeem(&new.code, "x", PHONE, 30).is_ok());
    }

    #[test]
    fn qr_token_registers_on_first_use_and_expires() {
        let dir = tempfile::tempdir().unwrap();
        let store = TokenStore::open(dir.path());
        let p = store.new_pairing("http://tv:8787", 0).unwrap();
        assert!(store.authenticate(&p.token, CODE_TTL_MS + 1, "Phone").is_none());
        let p2 = store.new_pairing("http://tv:8787", CODE_TTL_MS + 1).unwrap();
        assert!(store.authenticate(&p2.token, CODE_TTL_MS + 10, "Pixel 8 · Chrome").is_some());
        assert!(store.authenticate(&p2.token, 2 * CODE_TTL_MS + 10, "Phone").is_some());
        assert_eq!(store.devices(None)[0]["name"], "Pixel 8 · Chrome");
    }
}
