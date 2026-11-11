//! Bearer tokens for paired phones, kept hashed in a JSON file in the data directory, and
//! short-lived pairing codes that exchange for a token.

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::net::IpAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

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
    /// The device could not be written to disk, so it was not paired.
    NotSaved(String),
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
    /// The one-time key the link carries, which is never a token itself.
    pub link: String,
    pub url: String,
    pub expires_ms: i64,
}

/// A live code and link. Either one exchanges once for a token that is never shown anywhere, and
/// using either retires both.
struct Pending {
    link: String,
    token: String,
    expires_ms: i64,
}

/// The one live code, keyed by its digits, and the address its links point at.
#[derive(Default)]
struct Live {
    codes: HashMap<String, Pending>,
    base_url: Option<String>,
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
    pending: Mutex<Live>,
    lockouts: Mutex<Lockouts>,
    /// Ticks whenever a device is forgotten, so its open sockets can close at once.
    revoked: tokio::sync::watch::Sender<()>,
    /// Set while the file is behind the list in memory; every authentication retries the save.
    unsaved: AtomicBool,
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
        let revoked = tokio::sync::watch::channel(()).0;
        TokenStore { path, devices: Mutex::new(devices), pending: Mutex::new(Live::default()), lockouts: Mutex::new(Lockouts::default()), revoked, unsaved: AtomicBool::new(false) }
    }

    fn save(&self, devices: &[Device]) -> Result<(), String> {
        let result = serde_json::to_string_pretty(devices).map_err(|e| e.to_string()).and_then(|json| {
            let tmp = self.path.with_extension("json.tmp");
            std::fs::write(&tmp, json).and_then(|_| std::fs::rename(&tmp, &self.path)).map_err(|e| {
                let _ = std::fs::remove_file(&tmp);
                e.to_string()
            })
        });
        match &result {
            Ok(()) => self.unsaved.store(false, Ordering::Relaxed),
            Err(e) => {
                if !self.unsaved.swap(true, Ordering::Relaxed) {
                    tracing::warn!("companion: paired devices could not be saved ({e})");
                }
            }
        }
        result
    }

    /// The paired devices, marking the one whose id is `current`.
    pub fn devices(&self, current: Option<&str>) -> Vec<serde_json::Value> {
        self.devices
            .lock()
            .iter()
            .map(|d| {
                serde_json::json!({
                    "id": d.id, "name": readable_name(&d.name), "createdMs": d.created_ms, "lastSeenMs": d.last_seen_ms,
                    "current": current == Some(d.id.as_str()),
                })
            })
            .collect()
    }

    pub fn count(&self) -> usize {
        self.devices.lock().len()
    }

    /// A new six-digit code, and a URL whose page pairs by itself when opened. Any earlier code
    /// stops working: one live code at a time.
    pub fn new_pairing(&self, base_url: &str, now_ms: i64) -> Result<PairingInfo, String> {
        let mut live = self.pending.lock();
        live.base_url = Some(base_url.to_string());
        replace_code(&mut live, base_url, now_ms)
    }

    /// The code and link that pair a phone now, if one is live.
    pub fn live_pairing(&self, now_ms: i64) -> Option<PairingInfo> {
        let live = self.pending.lock();
        let base_url = live.base_url.as_deref()?;
        live.codes.iter().find(|(_, p)| p.expires_ms >= now_ms).map(|(code, p)| PairingInfo {
            code: code.clone(),
            link: p.link.clone(),
            url: format!("{base_url}/#token={}", p.link),
            expires_ms: p.expires_ms,
        })
    }

    /// A code that was just used is replaced at once, so the TV never shows one that is spent.
    fn renew(&self, live: &mut Live, now_ms: i64) {
        if let Some(base_url) = live.base_url.clone()
            && let Err(e) = replace_code(live, &base_url, now_ms)
        {
            tracing::warn!("companion: no new pairing code ({e})");
        }
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
        let pending = {
            let mut live = self.pending.lock();
            let found = live.codes.remove(code).filter(|p| p.expires_ms >= now_ms);
            if found.is_some() {
                self.renew(&mut live, now_ms);
            }
            found
        };
        match pending {
            Some(pending) => {
                self.lockouts.lock().by_address.remove(&from);
                self.register(&pending.token, device_name, now_ms).map_err(RedeemError::NotSaved)?;
                Ok(pending.token)
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

    /// Exchange the key a QR link carries for a token and register the device. The key sat in a
    /// URL, where a picture or a browser's history can keep it, so it works once and is never a
    /// token itself.
    pub fn redeem_link(&self, link: &str, device_name: &str, now_ms: i64) -> Result<String, RedeemError> {
        let mut live = self.pending.lock();
        let code = live.codes.iter().find(|(_, p)| p.link == link && p.expires_ms >= now_ms).map(|(c, _)| c.clone()).ok_or(RedeemError::Wrong)?;
        let used = live.codes.remove(&code).ok_or(RedeemError::Wrong)?;
        self.renew(&mut live, now_ms);
        drop(live);
        self.register(&used.token, device_name, now_ms).map_err(RedeemError::NotSaved)?;
        Ok(used.token)
    }

    /// Adds the device a token was just handed to, unless it cannot be saved: a phone paired only
    /// in memory would be dropped at the next restart.
    fn register(&self, token: &str, device_name: &str, now_ms: i64) -> Result<(), String> {
        let h = hash(token);
        let mut devices = self.devices.lock();
        if devices.iter().any(|d| d.token_hash == h) {
            return Ok(());
        }
        // The id only names the device in the list; without randomness the hash serves.
        let id = random_token().map(|t| t[..12].to_string()).unwrap_or_else(|| h[..12].to_string());
        devices.push(Device { id, name: device_name.to_string(), created_ms: now_ms, last_seen_ms: now_ms, token_hash: h, session_hash: String::new() });
        if let Err(e) = self.save(&devices) {
            devices.pop();
            return Err(e);
        }
        Ok(())
    }

    /// The id of the device a token was handed to.
    pub fn authenticate(&self, token: &str, now_ms: i64) -> Option<String> {
        let h = hash(token);
        self.seen(|d| d.token_hash == h, now_ms)
    }

    /// The id of the device a session cookie was handed to. The cookie is its own secret, so it
    /// never works where a token is asked for.
    pub fn authenticate_session(&self, session: &str, now_ms: i64) -> Option<String> {
        let h = hash(session);
        self.seen(|d| d.session_hash == h, now_ms)
    }

    /// A fresh session cookie value for a paired device, replacing its last one. None when the
    /// device is gone, the system cannot give randomness, or the cookie cannot be saved.
    pub fn new_session(&self, device_id: &str) -> Option<String> {
        let session = random_token()?;
        let mut devices = self.devices.lock();
        let i = devices.iter().position(|d| d.id == device_id)?;
        let previous = std::mem::replace(&mut devices[i].session_hash, hash(&session));
        if self.save(&devices).is_err() {
            devices[i].session_hash = previous;
            return None;
        }
        Some(session)
    }

    /// The id of the first device that matches, noting that it was just seen.
    fn seen(&self, matches: impl Fn(&Device) -> bool, now_ms: i64) -> Option<String> {
        let mut devices = self.devices.lock();
        let mut save = self.unsaved.load(Ordering::Relaxed);
        let id = devices.iter_mut().find(|d| matches(d)).map(|d| {
            if now_ms - d.last_seen_ms > 60_000 {
                d.last_seen_ms = now_ms;
                save = true;
            }
            d.id.clone()
        });
        if save {
            // Saved under the lock, so an older list never lands after a newer one.
            let _ = self.save(&devices);
        }
        id
    }

    /// Forgets a device at once. An error means it is forgotten only until the launcher
    /// restarts, unless a later save gets through.
    pub fn revoke(&self, id: &str) -> Result<bool, String> {
        let mut devices = self.devices.lock();
        let before = devices.len();
        devices.retain(|d| d.id != id);
        if devices.len() == before {
            return Ok(false);
        }
        let saved = self.save(&devices);
        self.revoked.send_replace(());
        saved.map(|()| true)
    }

    /// Forgets every device at once, with the same caveat as [`TokenStore::revoke`]. The live
    /// code is replaced too, so a phone that saw it cannot pair with it afterwards.
    pub fn revoke_all(&self) -> Result<(), String> {
        let mut devices = self.devices.lock();
        devices.clear();
        let saved = self.save(&devices);
        let mut live = self.pending.lock();
        live.codes.clear();
        self.renew(&mut live, now_ms());
        drop(live);
        self.revoked.send_replace(());
        saved
    }

    pub fn is_paired(&self, device_id: &str) -> bool {
        self.devices.lock().iter().any(|d| d.id == device_id)
    }

    /// Wakes whenever a device is forgotten.
    pub fn revocations(&self) -> tokio::sync::watch::Receiver<()> {
        self.revoked.subscribe()
    }
}

/// Puts a fresh code and link in place of whatever was live.
fn replace_code(live: &mut Live, base_url: &str, now_ms: i64) -> Result<PairingInfo, String> {
    let failed = || "the TV's random number source failed, so no code was made".to_string();
    let (link, token) = (random_token().ok_or_else(failed)?, random_token().ok_or_else(failed)?);
    let code = format!("{:06}", random_u32().ok_or_else(failed)? % 1_000_000);
    let expires_ms = now_ms + CODE_TTL_MS;
    live.codes.clear();
    live.codes.insert(code.clone(), Pending { link: link.clone(), token, expires_ms });
    Ok(PairingInfo { url: format!("{base_url}/#token={link}"), code, link, expires_ms })
}

/// A phone's name as the list shows it, "Safari on iPhone", also for one saved as "iPhone · Safari".
fn readable_name(name: &str) -> String {
    match name.split_once(" · ") {
        Some((device, browser)) => format!("{browser} on {device}"),
        None => name.to_string(),
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
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
        // The link, seen on screen, is retired with the code and never was the token.
        assert_ne!(token, p.link);
        assert!(store.redeem_link(&p.link, "Camera", 2_500).is_err());
        assert!(store.authenticate(&p.link, 2_500).is_none());
        assert!(store.redeem(&p.code, "Pixel", PHONE, 2_000).is_err());
        assert!(store.authenticate(&token, 3_000).is_some());
        assert!(store.authenticate("nope", 3_000).is_none());
        assert_eq!(store.count(), 1);
        let reopened = TokenStore::open(dir.path());
        assert!(reopened.authenticate(&token, 4_000).is_some());
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
        assert!(store.redeem_link(&p.link, "Phone", 30).is_ok());
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
        assert!(store.redeem_link(&old.link, "Phone", 20).is_err());
        assert!(store.redeem(&new.code, "x", PHONE, 30).is_ok());
    }

    #[test]
    fn a_used_code_is_replaced_at_once() {
        let dir = tempfile::tempdir().unwrap();
        let store = TokenStore::open(dir.path());
        assert!(store.live_pairing(0).is_none());
        let first = store.new_pairing("http://tv:8787", 0).unwrap();
        assert_eq!(store.live_pairing(10).map(|p| p.code), Some(first.code.clone()));

        // Scanned by one phone, the TV's next read has a new code and link for the next phone.
        store.redeem_link(&first.link, "Pixel", 20).unwrap();
        let second = store.live_pairing(30).unwrap();
        assert_ne!(second.link, first.link);
        assert_eq!(second.url, format!("http://tv:8787/#token={}", second.link));
        assert!(second.expires_ms >= 20 + CODE_TTL_MS);
        assert!(store.redeem(&second.code, "iPhone", PHONE, 40).is_ok());
        let third = store.live_pairing(50).unwrap();
        assert_ne!(third.link, second.link);
        assert_eq!(store.count(), 2);

        // Forgetting every phone also retires the code they could have seen.
        store.revoke_all().unwrap();
        let fourth = store.live_pairing(now_ms()).unwrap();
        assert_ne!(fourth.link, third.link);
        assert!(store.redeem_link(&third.link, "Camera", now_ms()).is_err());

        // An expired code is not live, and nothing replaces it until the TV asks.
        assert!(store.live_pairing(fourth.expires_ms + 1).is_none());
    }

    #[test]
    fn phones_are_listed_by_browser_and_device() {
        assert_eq!(readable_name("iPhone · Safari"), "Safari on iPhone");
        assert_eq!(readable_name("Safari on iPhone"), "Safari on iPhone");
    }

    #[test]
    fn a_qr_link_pairs_once_and_expires() {
        let dir = tempfile::tempdir().unwrap();
        let store = TokenStore::open(dir.path());
        let p = store.new_pairing("http://tv:8787", 0).unwrap();
        assert!(store.redeem_link(&p.link, "Phone", CODE_TTL_MS + 1).is_err());
        let p2 = store.new_pairing("http://tv:8787", CODE_TTL_MS + 1).unwrap();
        let token = store.redeem_link(&p2.link, "Chrome on Pixel 8", CODE_TTL_MS + 10).unwrap();
        assert_ne!(token, p2.link);
        assert!(store.authenticate(&p2.link, CODE_TTL_MS + 20).is_none());
        assert!(store.redeem_link(&p2.link, "Camera", CODE_TTL_MS + 20).is_err());
        assert!(store.redeem(&p2.code, "Camera", PHONE, CODE_TTL_MS + 20).is_err());
        assert!(store.authenticate(&token, 2 * CODE_TTL_MS + 10).is_some());
        assert_eq!(store.count(), 1);
        assert_eq!(store.devices(None)[0]["name"], "Chrome on Pixel 8");
    }

    #[test]
    fn a_forgotten_phone_that_cannot_be_saved_says_so_and_saves_later() {
        let dir = tempfile::tempdir().unwrap();
        let store = TokenStore::open(dir.path());
        let p = store.new_pairing("http://tv:8787", 0).unwrap();
        let lost = store.redeem_link(&p.link, "Lost", 10).unwrap();
        let p = store.new_pairing("http://tv:8787", 20).unwrap();
        let kept = store.redeem_link(&p.link, "Kept", 30).unwrap();
        let lost_id = store.authenticate(&lost, 40).unwrap();

        // A directory where the temporary file goes makes every save fail.
        let tmp = dir.path().join("companion-devices.json.tmp");
        std::fs::create_dir(&tmp).unwrap();
        assert!(store.revoke(&lost_id).is_err());
        assert!(store.authenticate(&lost, 50).is_none());
        let p = store.new_pairing("http://tv:8787", 60).unwrap();
        assert!(matches!(store.redeem_link(&p.link, "New", 70), Err(RedeemError::NotSaved(_))));
        assert_eq!(store.count(), 1);
        assert!(TokenStore::open(dir.path()).authenticate(&lost, 80).is_some());

        // The next phone the TV hears from saves the list it has.
        std::fs::remove_dir(&tmp).unwrap();
        assert!(store.authenticate(&kept, 90).is_some());
        let reopened = TokenStore::open(dir.path());
        assert!(reopened.authenticate(&lost, 100).is_none());
        assert!(reopened.authenticate(&kept, 100).is_some());
    }
}
