//! The plugin SDK. Implement [`Plugin`], call [`export_plugin!`], build for
//! `wasm32-unknown-unknown`, bundle with `lgp-tool`. Everything the host offers is in [`host`].
//!
//! ```ignore
//! struct Mine;
//! impl lounge_plugin_sdk::Plugin for Mine {
//!     fn manifest() -> Manifest { ... }
//!     fn refresh(ctx: &RefreshContext) -> Result<Vec<Item>, String> { ... }
//! }
//! lounge_plugin_sdk::export_plugin!(Mine);
//! ```

pub use lounge_plugin_api::*;
pub use lounge_words as words;
pub use lounge_words::{mark, t, tc, tn, tr};
pub use serde_json;

use std::collections::{HashMap, HashSet};

/// Most items the host keeps from one refresh; anything after them is dropped.
pub const MAX_ITEMS: usize = 2_000;

/// The SDK's own sentences in the languages Lounge ships.
const LOCALES: words::Catalogs = &[
    ("de", include_str!("../locales/de.po")),
    ("es", include_str!("../locales/es.po")),
    ("fr", include_str!("../locales/fr.po")),
    ("it", include_str!("../locales/it.po")),
    ("nl", include_str!("../locales/nl.po")),
    ("pt-BR", include_str!("../locales/pt-BR.po")),
    ("pt-PT", include_str!("../locales/pt-PT.po")),
];

pub trait Plugin {
    /// The plugin's translations: a language tag and its catalog. [`locales!`] writes this from
    /// the `locales` directory beside `Cargo.toml`.
    fn locales() -> words::Catalogs {
        &[]
    }
    fn manifest() -> Manifest;
    fn refresh(ctx: &RefreshContext) -> Result<Vec<Item>, String>;
    fn search(_query: &str, _limit: u32) -> Result<Vec<Item>, String> {
        Ok(Vec::new())
    }
    fn resolve(_action: &str) -> Result<Action, String> {
        Err(t!("This plugin can't open that."))
    }
    /// Describes the sheet an action or a sheet row named by its id.
    fn sheet(_id: &str) -> Result<Sheet, String> {
        Err(t!("This plugin has nothing more to show here."))
    }
}

thread_local! {
    /// What the call in progress asks of the person, if anything.
    static ASKED: std::cell::RefCell<Option<Ask>> = const { std::cell::RefCell::new(None) };
}

/// The error for `refresh` to return while it waits on the person: `Err(sdk::ask(...))`. The TV
/// shows the ask as a step to take; its `say` is what a plain-text surface shows.
pub fn ask(ask: Ask) -> String {
    let say = ask.say.clone();
    ASKED.with(|a| *a.borrow_mut() = Some(ask));
    say
}

/// [`ask`] for a sign-in with a code: the person opens `link` on their phone and enters `code`.
/// The TV shows both with a QR code and refreshes by itself until the code is taken. The
/// sentence names the page without its scheme or query, as people read it out.
pub fn sign_in(link: &str, code: &str) -> String {
    sign_in_ask(link, code, None)
}

/// [`sign_in`] for a code kept under `key` in the plugin's store. The plugin's page then offers
/// a new code, which deletes the key and refreshes, so the plugin asks for another.
pub fn sign_in_kept(link: &str, code: &str, key: &str) -> String {
    sign_in_ask(link, code, Some(key))
}

fn sign_in_ask(link: &str, code: &str, key: Option<&str>) -> String {
    let shown = link.trim_start_matches("https://").split(['?', '#']).next().unwrap_or(link).trim_end_matches('/');
    // Translators: {shown} is a web address and {code} a short code. "Refresh now" is the button on the plugin's page on the TV.
    let say = t!("Sign in: on your phone open {shown} and enter {code}, then press Refresh now here.", shown = shown, code = code);
    ask(Ask { say, link: Some(link.to_string()), code: Some(code.to_string()), after: None, watch: true, code_key: key.map(str::to_string) })
}

/// Memory handed across the boundary: exact-length boxed slices, allocated and freed on this side.
#[doc(hidden)]
pub fn __alloc(len: i32) -> i32 {
    if len <= 0 {
        return 0;
    }
    let boxed = vec![0u8; len as usize].into_boxed_slice();
    Box::leak(boxed).as_mut_ptr() as i32
}

#[doc(hidden)]
pub fn __free(ptr: i32, len: i32) {
    if ptr == 0 || len <= 0 {
        return;
    }
    // SAFETY: the pointer came from `__alloc` or `__return_json` with exactly this length.
    unsafe { drop(Box::from_raw(std::ptr::slice_from_raw_parts_mut(ptr as *mut u8, len as usize))) };
}

#[doc(hidden)]
pub fn __take(ptr: i32, len: i32) -> Vec<u8> {
    if ptr == 0 || len <= 0 {
        return Vec::new();
    }
    // SAFETY: host-written input lives in memory from `__alloc`; we own it from here.
    unsafe { Box::from_raw(std::ptr::slice_from_raw_parts_mut(ptr as *mut u8, len as usize)).into_vec() }
}

#[doc(hidden)]
pub fn __return_bytes(bytes: Vec<u8>) -> i64 {
    let len = bytes.len() as u32;
    let ptr = Box::leak(bytes.into_boxed_slice()).as_mut_ptr() as u32;
    pack(ptr, len)
}

#[doc(hidden)]
pub fn __return_json<T: serde::Serialize>(value: &T) -> i64 {
    let bytes = serde_json::to_vec(value).unwrap_or_else(|e| format!("{{\"err\":\"serialise: {e}\"}}").into_bytes());
    __return_bytes(bytes)
}

#[doc(hidden)]
pub fn __call<I: serde::de::DeserializeOwned, O: serde::Serialize>(ptr: i32, len: i32, f: impl FnOnce(I) -> Result<O, String>) -> i64 {
    let input = __take(ptr, len);
    ASKED.with(|a| a.borrow_mut().take());
    let outcome: Outcome<O> = match serde_json::from_slice::<I>(&input) {
        Ok(i) => match f(i) {
            Ok(o) => Outcome::Ok(o),
            Err(e) => asked_or_failed(spoken(e), ASKED.with(|a| a.borrow_mut().take())),
        },
        Err(e) => Outcome::Err(format!("bad input: {e}")),
    };
    __return_json(&outcome)
}

/// The SDK's errors that are constants, for plugins to compare with, in the language in use.
fn spoken(error: String) -> String {
    if [host::TOO_LARGE, host::BUDGET_SPENT, GUIDE_KEPT].contains(&error.as_str()) { words::tr(&error) } else { error }
}

/// An error that carries what [`ask`] made is the ask, with whatever the plugin said around it;
/// any other error is a failure.
fn asked_or_failed<O>(error: String, asked: Option<Ask>) -> Outcome<O> {
    match asked {
        Some(ask) if error.contains(&ask.say) => Outcome::Ask(Ask { say: error, ..ask }),
        _ => Outcome::Err(error),
    }
}

/// Puts the language Lounge is shown in into use for the call that follows.
#[doc(hidden)]
pub fn __speak<P: Plugin>() {
    let wanted = host::setting(LOCALE_SETTING).unwrap_or_else(|| words::SOURCE.to_string());
    words::speak(&wanted, &[P::locales(), LOCALES]);
}

/// The manifest in English, with what each language the plugin ships says for its text.
#[doc(hidden)]
pub fn __manifest<P: Plugin>() -> Manifest {
    let shipped = [P::locales(), LOCALES];
    let mut manifest = words::in_language(words::SOURCE, &[], P::manifest);
    for (tag, _) in P::locales() {
        let theirs = words::in_language(tag, &shipped, P::manifest);
        let (english, said) = (manifest.shown_texts(), theirs.shown_texts());
        if english.len() != said.len() {
            continue;
        }
        let said: std::collections::BTreeMap<String, String> =
            english.iter().zip(&said).filter(|(english, said)| english != said).map(|(english, said)| (english.to_string(), said.to_string())).collect();
        if !said.is_empty() {
            manifest.locales.insert(tag.to_string(), said);
        }
    }
    manifest
}

/// Writes [`Plugin::locales`] inside a plugin's `impl`, from `locales/<tag>.po` beside its
/// `Cargo.toml`. With no tags it is the languages Lounge itself ships.
#[macro_export]
macro_rules! locales {
    () => {
        $crate::locales!("de", "es", "fr", "it", "nl", "pt-BR", "pt-PT");
    };
    ($($tag:literal),+ $(,)?) => {
        fn locales() -> $crate::words::Catalogs {
            &[$(($tag, include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/locales/", $tag, ".po")))),+]
        }
    };
}

/// Exports the ABI entry points for a [`Plugin`] implementation.
#[macro_export]
macro_rules! export_plugin {
    ($plugin:ty) => {
        #[unsafe(no_mangle)]
        pub extern "C" fn lg_alloc(len: i32) -> i32 {
            $crate::__alloc(len)
        }
        #[unsafe(no_mangle)]
        pub extern "C" fn lg_free(ptr: i32, len: i32) {
            $crate::__free(ptr, len)
        }
        #[unsafe(no_mangle)]
        pub extern "C" fn lg_manifest() -> i64 {
            $crate::__return_json(&$crate::Outcome::Ok($crate::__manifest::<$plugin>()))
        }
        #[unsafe(no_mangle)]
        pub extern "C" fn lg_refresh(ptr: i32, len: i32) -> i64 {
            $crate::__speak::<$plugin>();
            $crate::__call(ptr, len, |ctx: $crate::RefreshContext| <$plugin as $crate::Plugin>::refresh(&ctx))
        }
        #[unsafe(no_mangle)]
        pub extern "C" fn lg_search(ptr: i32, len: i32) -> i64 {
            $crate::__speak::<$plugin>();
            $crate::__call(ptr, len, |q: $crate::SearchQuery| <$plugin as $crate::Plugin>::search(&q.query, q.limit))
        }
        #[unsafe(no_mangle)]
        pub extern "C" fn lg_resolve(ptr: i32, len: i32) -> i64 {
            $crate::__speak::<$plugin>();
            $crate::__call(ptr, len, |a: String| <$plugin as $crate::Plugin>::resolve(&a))
        }
        #[unsafe(no_mangle)]
        pub extern "C" fn lg_sheet(ptr: i32, len: i32) -> i64 {
            $crate::__speak::<$plugin>();
            $crate::__call(ptr, len, |id: String| <$plugin as $crate::Plugin>::sheet(&id))
        }
    };
}

/// What the host lets a plugin do.
pub mod host {
    use super::*;

    mod raw {
        #[cfg(target_arch = "wasm32")]
        #[link(wasm_import_module = "lounge")]
        unsafe extern "C" {
            pub fn fetch(ptr: i32, len: i32) -> i64;
            pub fn kv_get(ptr: i32, len: i32) -> i64;
            pub fn kv_set(kp: i32, kl: i32, vp: i32, vl: i32);
            pub fn kv_delete(ptr: i32, len: i32);
            pub fn now() -> i64;
            pub fn log(level: i32, ptr: i32, len: i32);
            pub fn secret(ptr: i32, len: i32) -> i64;
            pub fn setting(ptr: i32, len: i32) -> i64;
        }

        // Off wasm (unit tests of a plugin's parsing code) the host is absent.
        #[cfg(not(target_arch = "wasm32"))]
        pub unsafe fn fetch(_: i32, _: i32) -> i64 { 0 }
        #[cfg(not(target_arch = "wasm32"))]
        pub unsafe fn kv_get(_: i32, _: i32) -> i64 { 0 }
        #[cfg(not(target_arch = "wasm32"))]
        pub unsafe fn kv_set(_: i32, _: i32, _: i32, _: i32) {}
        #[cfg(not(target_arch = "wasm32"))]
        pub unsafe fn kv_delete(_: i32, _: i32) {}
        #[cfg(not(target_arch = "wasm32"))]
        pub unsafe fn now() -> i64 { 0 }
        #[cfg(not(target_arch = "wasm32"))]
        pub unsafe fn log(_: i32, _: i32, _: i32) {}
        #[cfg(not(target_arch = "wasm32"))]
        pub unsafe fn secret(_: i32, _: i32) -> i64 { 0 }
        #[cfg(not(target_arch = "wasm32"))]
        pub unsafe fn setting(_: i32, _: i32) -> i64 { 0 }
    }

    fn pass(bytes: &[u8]) -> (i32, i32) {
        (bytes.as_ptr() as i32, bytes.len() as i32)
    }

    fn take(packed: i64) -> Option<Vec<u8>> {
        let (ptr, len) = unpack(packed);
        if len == 0 { None } else { Some(__take(ptr as i32, len as i32)) }
    }

    /// Perform an HTTP request. The host refuses hosts outside the manifest's allow list. Any
    /// address in an error has its query string and password taken out.
    pub fn fetch(request: &HttpRequest) -> Result<HttpResponse, String> {
        let req = serde_json::to_vec(request).map_err(|e| e.to_string())?;
        let (p, l) = pass(&req);
        let out = take(unsafe { raw::fetch(p, l) }).ok_or_else(|| "no response from host".to_string())?;
        drop(req);
        crate::wire::read_response(out).map_err(refusal)
    }

    /// The host's refusal of a fetch as the plugin reads it: the two limits as their constants,
    /// anything else with its addresses redacted.
    pub(crate) fn refusal(e: String) -> String {
        if e.contains("body is larger than") {
            TOO_LARGE.to_string()
        } else if e.starts_with("fetch budget ") {
            BUDGET_SPENT.to_string()
        } else {
            crate::redact(&e)
        }
    }

    /// The error `fetch` returns when an answer is bigger than the host lets a plugin read.
    pub const TOO_LARGE: &str = mark!("The answer is larger than the 8 MB a plugin may read.");

    /// The error `fetch` returns once a call has made all the requests, or waited all the time,
    /// the host allows it.
    pub const BUDGET_SPENT: &str = mark!("The plugin reached the most it may fetch in one refresh, so it stopped early. It tries again in a few minutes.");

    /// GET a URL and return the body, failing on non-2xx.
    pub fn get_text(url: &str) -> Result<String, String> {
        let r = fetch(&HttpRequest::get(url))?;
        if !r.ok() {
            return Err(t!("The server at {address} answered with error {status}. Check the address, then try again.", address = crate::address(url), status = r.status));
        }
        Ok(if r.body_base64.is_some() { String::from_utf8_lossy(&crate::body_bytes(r)).into_owned() } else { r.body })
    }

    pub fn kv_get(key: &str) -> Option<Vec<u8>> {
        let (p, l) = pass(key.as_bytes());
        take(unsafe { raw::kv_get(p, l) })
    }

    pub fn kv_get_string(key: &str) -> Option<String> {
        kv_get(key).and_then(|v| String::from_utf8(v).ok())
    }

    pub fn kv_set(key: &str, value: &[u8]) {
        let (kp, kl) = pass(key.as_bytes());
        let (vp, vl) = pass(value);
        unsafe { raw::kv_set(kp, kl, vp, vl) }
    }

    pub fn kv_delete(key: &str) {
        let (p, l) = pass(key.as_bytes());
        unsafe { raw::kv_delete(p, l) }
    }

    pub fn now_ms() -> i64 {
        unsafe { raw::now() }
    }

    pub fn log(level: i32, message: &str) {
        let (p, l) = pass(message.as_bytes());
        unsafe { raw::log(level, p, l) }
    }

    pub fn secret(key: &str) -> Option<String> {
        let (p, l) = pass(key.as_bytes());
        take(unsafe { raw::secret(p, l) }).and_then(|v| String::from_utf8(v).ok())
    }

    pub fn setting(key: &str) -> Option<String> {
        let (p, l) = pass(key.as_bytes());
        take(unsafe { raw::setting(p, l) }).and_then(|v| String::from_utf8(v).ok()).filter(|s| !s.trim().is_empty())
    }

    /// A setting's value, or [crate::not_set] for `label`, the field's name as it reads inside a
    /// sentence, such as "server address".
    /// The address of this plugin's sign-in landing page on the TV, for a service to send the
    /// person's phone back to. None while the phone companion is off.
    pub fn callback() -> Option<String> {
        setting(lounge_plugin_api::CALLBACK_SETTING)
    }

    /// The query the landing page was last opened with, without its `?`. It is read once and
    /// then forgotten.
    pub fn take_callback() -> Option<String> {
        let query = kv_get_string(lounge_plugin_api::CALLBACK_KEY)?;
        kv_delete(lounge_plugin_api::CALLBACK_KEY);
        Some(query)
    }

    pub fn required_setting(key: &str, label: &str) -> Result<String, String> {
        setting(key).ok_or_else(|| crate::not_set(label))
    }

    /// A server address from setting `key`, as [crate::server_address] tidies it.
    pub fn server_setting(key: &str, label: &str) -> Result<String, String> {
        Ok(crate::server_address(&required_setting(key, label)?))
    }
}

#[macro_export]
macro_rules! info {
    ($($arg:tt)*) => { $crate::host::log($crate::LOG_INFO, &format!($($arg)*)) };
}

#[macro_export]
macro_rules! warn {
    ($($arg:tt)*) => { $crate::host::log($crate::LOG_WARN, &format!($($arg)*)) };
}

#[macro_export]
macro_rules! error {
    ($($arg:tt)*) => { $crate::host::log($crate::LOG_ERROR, &format!($($arg)*)) };
}

/// Days-from-civil (Howard Hinnant) so plugins can turn dates into epoch milliseconds without a
/// calendar crate.
pub fn epoch_ms(year: i64, month: u32, day: u32, hour: u32, minute: u32, second: u32, utc_offset_minutes: i32) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let m = month as i64;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + day as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    (days * 86_400 + hour as i64 * 3600 + minute as i64 * 60 + second as i64 - utc_offset_minutes as i64 * 60) * 1000
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_ask_travels_as_itself_and_any_other_error_as_a_failure() {
        let said = sign_in("https://plex.tv/link", "25Q4");
        assert_eq!(said, "Sign in: on your phone open plex.tv/link and enter 25Q4, then press Refresh now here.");
        let asked = ASKED.with(|a| a.borrow_mut().take());
        // The plugin may say more around the sentence.
        let Outcome::Ask(ask) = asked_or_failed::<()>(format!("That code ran out. {said}"), asked.clone()) else { panic!("not an ask") };
        assert_eq!(ask.say, format!("That code ran out. {said}"));
        assert_eq!((ask.link.as_deref(), ask.code.as_deref(), ask.after), (Some("https://plex.tv/link"), Some("25Q4"), None));
        assert_eq!(asked_or_failed::<()>("Something else broke".into(), asked), Outcome::Err("Something else broke".into()));
        assert_eq!(asked_or_failed::<()>("plain".into(), None), Outcome::Err("plain".into()));
        assert_eq!(serde_json::to_string(&Outcome::<()>::Ask(Ask { say: "Wait".into(), ..Default::default() })).unwrap(), r#"{"ask":{"say":"Wait"}}"#);
        // The sentence names the page as it is read out; the link keeps what it carries.
        assert_eq!(sign_in("https://www.twitch.tv/activate?device-code=ABCD", "ABCD"), "Sign in: on your phone open www.twitch.tv/activate and enter ABCD, then press Refresh now here.");
        assert_eq!(sign_in("https://simkl.com/pin/", "B-1"), "Sign in: on your phone open simkl.com/pin and enter B-1, then press Refresh now here.");
        let twitch = ASKED.with(|a| a.borrow_mut().take()).unwrap();
        assert!(twitch.watch && twitch.link.as_deref() == Some("https://simkl.com/pin/") && twitch.code_key.is_none());
        // A code the plugin keeps names its key, so the TV can have it dropped.
        assert_eq!(sign_in_kept("https://plex.tv/link", "25Q4", "pin"), said);
        let kept = ASKED.with(|a| a.borrow_mut().take()).unwrap();
        assert_eq!(serde_json::to_string(&Outcome::<()>::Ask(Ask { say: "Wait".into(), ..kept })).unwrap(), r#"{"ask":{"say":"Wait","link":"https://plex.tv/link","code":"25Q4","watch":true,"code_key":"pin"}}"#);
    }

    #[test]
    fn epoch_matches_known_dates() {
        assert_eq!(epoch_ms(1970, 1, 1, 0, 0, 0, 0), 0);
        assert_eq!(epoch_ms(2000, 3, 1, 0, 0, 0, 0), 951_868_800_000);
        assert_eq!(epoch_ms(2026, 10, 19, 0, 15, 0, 0), 1_792_368_900_000);
        assert_eq!(epoch_ms(2026, 10, 19, 10, 15, 0, 600), 1_792_368_900_000);
    }

    // Pointers are 32-bit only on wasm; natively the i32 ABI would truncate them.
    #[cfg(target_arch = "wasm32")]
    #[test]
    fn alloc_and_free_roundtrip() {
        let p = __alloc(16);
        assert_ne!(p, 0);
        __free(p, 16);
        let packed = __return_json(&Outcome::Ok(1u8));
        let (ptr, len) = unpack(packed);
        let bytes = __take(ptr as i32, len as i32);
        assert_eq!(bytes, br#"{"ok":1}"#);
    }
}

/// "2026-10-17T02:09:23+00:00", "2026-10-18T22:11:00.0000000Z" or a date with a space → epoch
/// milliseconds. Fractions are kept to the millisecond; a missing zone means UTC. Anything else,
/// including text that is not ASCII where digits belong, gives `None`.
pub fn parse_iso8601(s: &str) -> Option<i64> {
    let b = s.trim().as_bytes();
    let num = |at: usize, len: usize| -> Option<u32> {
        let digits = b.get(at..at + len)?;
        digits.iter().all(u8::is_ascii_digit).then(|| digits.iter().fold(0, |n, d| n * 10 + u32::from(d - b'0')))
    };
    if b.len() < 19 || b[4] != b'-' || b[7] != b'-' || !matches!(b[10], b'T' | b't' | b' ') || b[13] != b':' || b[16] != b':' {
        return None;
    }
    let (year, month, day) = (num(0, 4)?, num(5, 2)?, num(8, 2)?);
    let (hour, minute, second) = (num(11, 2)?, num(14, 2)?, num(17, 2)?);
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) || hour > 24 || minute > 59 || second > 60 {
        return None;
    }
    let mut at = 19;
    let mut millis = 0;
    if matches!(b.get(at), Some(b'.' | b',')) {
        let start = at + 1;
        at = start;
        while b.get(at).is_some_and(u8::is_ascii_digit) {
            at += 1;
        }
        if at == start {
            return None;
        }
        millis = b[start..at].iter().chain(b"00").take(3).fold(0, |n, d| n * 10 + i64::from(d - b'0'));
    }
    let offset_minutes = match b.get(at) {
        None | Some(b'Z' | b'z') => 0,
        Some(sign @ (b'+' | b'-')) => {
            let hh = num(at + 1, 2)?;
            let mm = if b.get(at + 3) == Some(&b':') { num(at + 4, 2)? } else { num(at + 3, 2).unwrap_or(0) };
            let v = (hh * 60 + mm) as i32;
            if *sign == b'-' { -v } else { v }
        }
        _ => return None,
    };
    Some(epoch_ms(i64::from(year), month, day, hour, minute, second, offset_minutes) + millis)
}

#[cfg(test)]
mod iso_tests {
    use super::parse_iso8601;

    #[test]
    fn every_common_shape() {
        assert_eq!(parse_iso8601("2026-10-17T02:09:23+00:00"), Some(1792202963000));
        assert_eq!(parse_iso8601("2026-10-17T02:09:23.250Z"), Some(1792202963250));
        assert_eq!(parse_iso8601("2026-10-17T02:09:23.5Z"), Some(1792202963500));
        assert_eq!(parse_iso8601("2026-10-17T12:09:23+10:00"), Some(1792202963000));
        assert_eq!(parse_iso8601("2026-10-17T12:09:23+1000"), Some(1792202963000));
        assert_eq!(parse_iso8601("2026-10-16T22:09:23-04:00"), Some(1792202963000));
        assert_eq!(parse_iso8601("2026-10-17 02:09:23"), Some(1792202963000));
        assert_eq!(parse_iso8601("2026-10-18T22:11:00.0000000Z"), Some(1792361460000));
        assert_eq!(parse_iso8601("yesterday"), None);
    }

    #[test]
    fn malformed_input_gives_none_instead_of_panicking() {
        for bad in [
            "2026-10-1ÄT02:09:23Z",
            "２０２６-10-17T02:09:23Z",
            "2026-10-17T02:09:2é",
            "2026-10-17T02:09:23+1é:00",
            "2026-10-17T02:09:23.Z",
            "2026-13-17T02:09:23Z",
            "2026-+1-17T02:09:23Z",
            "2026-10-17X02:09:23Z",
            "2026-10-17T02:09:23 garbage",
            "ééééééééééééééééééééé",
            "",
        ] {
            assert_eq!(parse_iso8601(bad), None, "{bad:?}");
        }
    }
}

/// A server address as people type it, ready to put paths after: trimmed, with `http://` when no
/// scheme was given and without a trailing slash, so "192.168.0.10:8096" works.
pub fn server_address(value: &str) -> String {
    let address = value.trim().trim_end_matches('/');
    if address.contains("://") { address.to_string() } else { format!("http://{address}") }
}

const GUIDE_SHOWN_KEY: &str = "sdk:guide_shown";

/// Notes that this refresh published a guide, which [`last_guide_still_showing`] looks for.
pub fn guide_shown(ctx: &RefreshContext) {
    host::kv_set(GUIDE_SHOWN_KEY, ctx.now_ms.to_string().as_bytes());
}

/// Whether a guide an earlier refresh published still reaches into the window. A refresh whose
/// guide failed should then fail as well, so the TV keeps that guide instead of showing none; a
/// guide that never came through leaves the channels to show without one.
pub fn last_guide_still_showing(ctx: &RefreshContext) -> bool {
    guide_reaches(host::kv_get_string(GUIDE_SHOWN_KEY).and_then(|v| v.parse().ok()), ctx)
}

fn guide_reaches(shown_ms: Option<i64>, ctx: &RefreshContext) -> bool {
    shown_ms.is_some_and(|t| ctx.now_ms - t < i64::from(ctx.window_hours.max(1)) * 3_600_000)
}

/// Whether something added at `added_ms` arrived within the past week, the span a New badge
/// stands for.
pub fn new_this_week(added_ms: Option<i64>, now_ms: i64) -> bool {
    added_ms.is_some_and(|t| (0..7 * 86_400_000).contains(&(now_ms - t)))
}

/// The error for a refresh that failed only because its guide did.
pub const GUIDE_KEPT: &str = mark!("The guide didn't come through, so the TV is showing the last one it had. It tries again in a few minutes.");

/// `url` without its query string, fragment, user name or password, for messages people read.
pub fn redact_url(url: &str) -> String {
    let (scheme, rest) = match url.find("://") {
        Some(i) => url.split_at(i + 3),
        None => ("", url),
    };
    let cut = rest.find(['?', '#']);
    let kept = &rest[..cut.unwrap_or(rest.len())];
    let (authority, path) = kept.split_at(kept.find('/').unwrap_or(kept.len()));
    let host = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    format!("{scheme}{host}{path}{}", if cut.is_some() { "?…" } else { "" })
}

/// `text` with every http or https address in it passed through [`redact_url`].
pub fn redact(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(i) = [rest.find("http://"), rest.find("https://")].into_iter().flatten().min() {
        out.push_str(&rest[..i]);
        let tail = &rest[i..];
        let end = tail.find(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | '<' | '>' | ')')).unwrap_or(tail.len());
        out.push_str(&redact_url(&tail[..end]));
        rest = &tail[end..];
    }
    out.push_str(rest);
    out
}

/// The server in `url` as an error on the TV names it: host, port and path, without the scheme,
/// password or query.
pub fn address(url: &str) -> String {
    let url = redact_url(url);
    url.split_once("://").map_or(url.as_str(), |(_, rest)| rest).trim_end_matches('/').to_string()
}

/// The error for a server the user named that could not be reached.
pub fn didnt_answer(service: &str, url: &str) -> String {
    t!("{service} didn't answer at {address}. Check the server, then try again.", service = service, address = address(url))
}

/// The error for a server the user named that answered with HTTP `status`.
pub fn answered_error(service: &str, url: &str, status: u16) -> String {
    t!("{service} at {address} answered with error {status}. Check the server, then try again.", service = service, address = address(url), status = status)
}

/// The error for an address where something other than `service` answered.
pub fn wrong_server(service: &str, url: &str) -> String {
    // English says "an Emby server" and "a Plex server"; a translation reads the same for both.
    if service.starts_with(['A', 'E', 'I', 'O', 'U']) {
        t!("The server at {address} didn't answer like an {service} server. Check the server address in the plugin's settings.", address = address(url), service = service)
    } else {
        t!("The server at {address} didn't answer like a {service} server. Check the server address in the plugin's settings.", address = address(url), service = service)
    }
}

/// The error for an answer from `service` at `url` that the plugin cannot read.
pub fn unreadable(service: &str, url: &str) -> String {
    t!("{service} at {address} sent a reply this plugin can't read.", service = service, address = address(url))
}

/// The error for an online service that could not be reached.
pub fn offline(service: &str) -> String {
    t!("{service} didn't answer. Check the TV's internet connection, then try again.", service = service)
}

/// The error for an online service that answered with HTTP `status`.
pub fn online_error(service: &str, status: u16) -> String {
    t!("{service} answered with error {status}. Try again later.", service = service, status = status)
}

/// The error for a setting still to fill in. `label` is the field's name as it reads inside a
/// sentence, such as "server address".
pub fn not_set(label: &str) -> String {
    // Translators: {label} names a setting, already translated, such as "server address". Word the sentence so the name can stand as it is, for example after a colon.
    t!("Add the {label} in the plugin's settings.", label = label)
}

/// [`host::fetch`] from the `service` at `server`. A server that cannot be reached gives
/// [`didnt_answer`] and the cause goes to the log; [`host::TOO_LARGE`] and
/// [`host::BUDGET_SPENT`] come back unchanged, so callers can still tell them apart.
pub fn fetch_from(service: &str, server: &str, request: &HttpRequest) -> Result<HttpResponse, String> {
    fetch_or(request, || didnt_answer(service, server))
}

/// [`fetch_from`] for an online service, which gives [`offline`] when it cannot be reached.
pub fn fetch_online(service: &str, request: &HttpRequest) -> Result<HttpResponse, String> {
    fetch_or(request, || offline(service))
}

fn fetch_or(request: &HttpRequest, unreachable: impl FnOnce() -> String) -> Result<HttpResponse, String> {
    host::fetch(request).map_err(|e| {
        if e == host::TOO_LARGE || e == host::BUDGET_SPENT {
            return e;
        }
        warn!("{e}");
        unreachable()
    })
}

/// A response's body as bytes. A base64 body is decoded where it lies, so a large one is held once.
pub fn body_bytes(response: HttpResponse) -> Vec<u8> {
    match response.body_base64 {
        Some(b64) => {
            let mut bytes = b64.into_bytes();
            let n = base64_decode_in_place(&mut bytes);
            bytes.truncate(n);
            bytes
        }
        None => response.body.into_bytes(),
    }
}

/// Decodes base64 into the front of `buf` and returns the decoded length; the output never
/// overtakes the input, so one buffer serves both.
fn base64_decode_in_place(buf: &mut [u8]) -> usize {
    let (mut w, mut acc, mut bits) = (0, 0u32, 0);
    for r in 0..buf.len() {
        let v = match buf[r] {
            c @ b'A'..=b'Z' => c - b'A',
            c @ b'a'..=b'z' => c - b'a' + 26,
            c @ b'0'..=b'9' => c - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            _ => continue,
        };
        acc = (acc << 6) | u32::from(v);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            buf[w] = (acc >> bits) as u8;
            w += 1;
            acc &= (1 << bits) - 1;
        }
    }
    w
}

/// Where the first byte of `hay` that is one of `bytes` sits. The search takes eight bytes at a
/// time, which costs a plugin far less than looking at each byte, so it suits scanning a large
/// answer for the next quote or tag.
pub fn find_first_of(hay: &[u8], bytes: &[u8]) -> Option<usize> {
    const ONES: u64 = 0x0101_0101_0101_0101;
    const HIGHS: u64 = 0x8080_8080_8080_8080;
    let mut words = hay.chunks_exact(8);
    let mut at = 0;
    for word in &mut words {
        let word = u64::from_le_bytes([word[0], word[1], word[2], word[3], word[4], word[5], word[6], word[7]]);
        let mut found = 0;
        for &byte in bytes {
            // A high bit is set at each byte equal to `byte`. The lowest is exact; one above it can be set by a borrow.
            let diff = word ^ (ONES * u64::from(byte));
            found |= diff.wrapping_sub(ONES) & !diff & HIGHS;
        }
        if found != 0 {
            return Some(at + (found.trailing_zeros() / 8) as usize);
        }
        at += 8;
    }
    words.remainder().iter().position(|b| bytes.contains(b)).map(|i| at + i)
}

/// Reading the host's answer to `fetch` without copying the body: the body's JSON string is
/// unescaped inside the buffer the answer arrived in, which then becomes the body.
mod wire {
    use super::HttpResponse;
    use serde::Deserialize;
    use serde_json::value::RawValue;
    use std::collections::BTreeMap;

    #[derive(Deserialize)]
    enum Envelope<'a> {
        #[serde(rename = "ok", borrow)]
        Ok(Head<'a>),
        #[serde(rename = "err")]
        Err(String),
    }

    #[derive(Deserialize)]
    struct Head<'a> {
        status: u16,
        #[serde(default)]
        headers: BTreeMap<String, String>,
        #[serde(borrow, default)]
        body: Option<&'a RawValue>,
        #[serde(borrow, default)]
        body_base64: Option<&'a RawValue>,
    }

    type Span = (usize, usize);

    pub fn read_response(json: Vec<u8>) -> Result<HttpResponse, String> {
        let origin = json.as_ptr() as usize;
        let (status, headers, body, body_base64) = {
            let head = match serde_json::from_slice::<Envelope>(&json).map_err(|e| format!("unreadable answer from the host: {e}"))? {
                Envelope::Ok(head) => head,
                Envelope::Err(e) => return Err(e),
            };
            let span = |raw: Option<&RawValue>| raw.map(|r| (r.get().as_ptr() as usize - origin, r.get().len()));
            (head.status, head.headers, span(head.body), span(head.body_base64))
        };
        Ok(match body_base64 {
            Some(b64) => {
                let body = match body {
                    Some((at, len)) => serde_json::from_slice(&json[at..at + len]).map_err(|e| e.to_string())?,
                    None => String::new(),
                };
                HttpResponse { status, headers, body, body_base64: Some(string_in_place(json, b64)?) }
            }
            None => HttpResponse { status, headers, body: body.map(|b| string_in_place(json, b)).transpose()?.unwrap_or_default(), body_base64: None },
        })
    }

    /// The JSON string at `span` of `json`, unescaped into the same allocation.
    fn string_in_place(mut json: Vec<u8>, (at, len): Span) -> Result<String, String> {
        if len < 2 || json[at] != b'"' {
            return Err("unreadable answer from the host: the body is not a string".into());
        }
        let start = at + 1;
        let n = unescape_in_place(&mut json[start..at + len - 1])?;
        json.copy_within(start..start + n, 0);
        json.truncate(n);
        String::from_utf8(json).map_err(|_| "unreadable answer from the host: the body is not UTF-8".to_string())
    }

    /// Unescapes JSON string content into the front of `buf` and returns its new length. An
    /// escape is never shorter than what it stands for, so writing trails reading.
    fn unescape_in_place(buf: &mut [u8]) -> Result<usize, String> {
        let bad = || "unreadable answer from the host: bad escape".to_string();
        let hex = |buf: &[u8], at: usize| -> Option<u32> { u32::from_str_radix(std::str::from_utf8(buf.get(at..at + 4)?).ok()?, 16).ok() };
        let (mut r, mut w) = (0, 0);
        while r < buf.len() {
            let next = crate::find_first_of(&buf[r..], b"\\").map_or(buf.len(), |i| r + i);
            if w != r {
                buf.copy_within(r..next, w);
            }
            w += next - r;
            r = next;
            if r == buf.len() {
                break;
            }
            let escape = *buf.get(r + 1).ok_or_else(bad)?;
            r += 2;
            let c = match escape {
                b'"' => '"',
                b'\\' => '\\',
                b'/' => '/',
                b'b' => '\u{8}',
                b'f' => '\u{c}',
                b'n' => '\n',
                b'r' => '\r',
                b't' => '\t',
                b'u' => {
                    let high = hex(buf, r).ok_or_else(bad)?;
                    r += 4;
                    let low = (buf.get(r..r + 2) == Some(b"\\u")).then(|| hex(buf, r + 2)).flatten().filter(|l| (0xDC00..0xE000).contains(l));
                    match low {
                        Some(low) if (0xD800..0xDC00).contains(&high) => {
                            r += 6;
                            char::from_u32(0x10000 + ((high - 0xD800) << 10) + (low - 0xDC00)).unwrap_or('\u{FFFD}')
                        }
                        _ => char::from_u32(high).unwrap_or('\u{FFFD}'),
                    }
                }
                _ => return Err(bad()),
            };
            w += c.encode_utf8(&mut buf[w..w + c.len_utf8()]).len();
        }
        Ok(w)
    }
}

/// Calls `each` with every element of the JSON array in `json` in turn, so a long array is
/// never held whole.
pub fn for_each_in_array<'de, T: serde::Deserialize<'de>>(json: &'de str, each: impl FnMut(T)) -> Result<(), String> {
    struct Each<T, F>(F, std::marker::PhantomData<T>);
    impl<'de, T: serde::Deserialize<'de>, F: FnMut(T)> serde::de::Visitor<'de> for Each<T, F> {
        type Value = ();
        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            f.write_str("a list")
        }
        fn visit_seq<A: serde::de::SeqAccess<'de>>(mut self, mut seq: A) -> Result<(), A::Error> {
            while let Some(item) = seq.next_element()? {
                (self.0)(item);
            }
            Ok(())
        }
    }
    let mut de = serde_json::Deserializer::from_str(json);
    serde::Deserializer::deserialize_seq(&mut de, Each(each, std::marker::PhantomData)).map_err(|e| e.to_string())?;
    de.end().map_err(|e| e.to_string())
}

/// Drops items whose id came earlier, keeping the first, and returns how many went.
pub fn dedup_ids(items: &mut Vec<Item>) -> usize {
    let before = items.len();
    let mut seen = HashSet::new();
    items.retain(|i| seen.insert(i.id.clone()));
    before - items.len()
}

/// The largest share `q` such that giving every channel `min(count, q)` entries stays within
/// `budget`.
pub fn fair_share(counts: &[usize], budget: usize) -> usize {
    let fits = |q: usize| counts.iter().map(|&c| c.min(q)).sum::<usize>() <= budget;
    let (mut lo, mut hi) = (0, counts.iter().copied().max().unwrap_or(0));
    while lo < hi {
        let mid = lo + (hi - lo).div_ceil(2);
        if fits(mid) { lo = mid } else { hi = mid - 1 }
    }
    lo
}

/// Keeps at most `budget` schedule items, shared evenly between channels. Each channel keeps
/// its earliest programmes, and what one channel cannot use goes to the others, earliest first.
/// Repeated ids are dropped.
pub fn fit_schedule(items: Vec<Item>, budget: usize) -> Vec<Item> {
    let mut order: Vec<String> = Vec::new();
    let mut groups: HashMap<String, Vec<Item>> = HashMap::new();
    let mut seen = HashSet::new();
    for item in items {
        if !seen.insert(item.id.clone()) {
            continue;
        }
        let channel = item.channel_id.clone().unwrap_or_default();
        groups
            .entry(channel)
            .or_insert_with_key(|c| {
                order.push(c.clone());
                Vec::new()
            })
            .push(item);
    }
    let start = |i: &Item| i.start_ms.unwrap_or(i64::MAX);
    for g in groups.values_mut() {
        g.sort_by_key(start);
    }
    let counts: Vec<usize> = order.iter().map(|c| groups[c].len()).collect();
    let share = fair_share(&counts, budget);
    let used: usize = counts.iter().map(|&c| c.min(share)).sum();
    let mut extra: Vec<(i64, usize)> = order.iter().enumerate().filter_map(|(n, c)| groups[c].get(share).map(|i| (start(i), n))).collect();
    extra.sort_unstable();
    extra.truncate(budget - used);
    let extra: HashSet<usize> = extra.into_iter().map(|(_, n)| n).collect();
    let mut out = Vec::with_capacity(budget.min(seen.len()));
    for (n, c) in order.iter().enumerate() {
        let mut g = groups.remove(c).unwrap_or_default();
        g.truncate(share + usize::from(extra.contains(&n)));
        out.extend(g);
    }
    out
}

/// Gathers schedule items for at most `budget` places. Items are held per channel and trimmed
/// as they arrive to what could still be shown, so a long guide never piles up; `finish`
/// then shares the places out as [`fit_schedule`] does.
pub struct ScheduleBudget {
    budget: usize,
    order: Vec<String>,
    channels: HashMap<String, Vec<Item>>,
    held: usize,
}

impl ScheduleBudget {
    pub fn new(budget: usize) -> ScheduleBudget {
        ScheduleBudget { budget, order: Vec::new(), channels: HashMap::new(), held: 0 }
    }

    pub fn push(&mut self, item: Item) {
        let channel = item.channel_id.clone().unwrap_or_default();
        let order = &mut self.order;
        self.channels
            .entry(channel)
            .or_insert_with_key(|c| {
                order.push(c.clone());
                Vec::new()
            })
            .push(item);
        self.held += 1;
        if self.held > 2 * self.budget + self.order.len() + 256 {
            self.trim();
        }
    }

    /// Keeps each channel's earliest programmes up to one more than its fair share. The share
    /// only shrinks as items arrive, and no channel ends up with more than one past it.
    fn trim(&mut self) {
        let start = |i: &Item| i.start_ms.unwrap_or(i64::MAX);
        for list in self.channels.values_mut() {
            list.sort_by_key(start);
            list.dedup_by(|a, b| a.id == b.id);
        }
        let counts: Vec<usize> = self.order.iter().map(|c| self.channels[c].len()).collect();
        let keep = fair_share(&counts, self.budget) + 1;
        for list in self.channels.values_mut() {
            list.truncate(keep);
        }
        self.held = self.channels.values().map(Vec::len).sum();
    }

    pub fn finish(mut self) -> Vec<Item> {
        let items = self.order.iter().flat_map(|c| self.channels.remove(c).unwrap_or_default()).collect();
        fit_schedule(items, self.budget)
    }
}

#[cfg(test)]
mod helper_tests {
    use super::*;

    #[test]
    fn new_means_this_week() {
        let now = 30 * 86_400_000;
        assert!(new_this_week(Some(now - 86_400_000), now));
        assert!(!new_this_week(Some(now - 8 * 86_400_000), now));
        assert!(!new_this_week(None, now));
    }

    #[test]
    fn the_last_guide_counts_while_it_reaches_into_the_window() {
        let ctx = RefreshContext { now_ms: 100 * 3_600_000, window_hours: 12, ..Default::default() };
        assert!(guide_reaches(Some(95 * 3_600_000), &ctx));
        assert!(!guide_reaches(Some(88 * 3_600_000), &ctx));
        assert!(!guide_reaches(None, &ctx));
    }

    #[test]
    fn server_addresses_are_tidied() {
        assert_eq!(server_address(" 192.168.0.10:8096/ "), "http://192.168.0.10:8096");
        assert_eq!(server_address("https://jf.example.org/"), "https://jf.example.org");
        assert_eq!(server_address("http://kodi.lan:8080"), "http://kodi.lan:8080");
    }

    #[test]
    fn urls_lose_queries_and_passwords() {
        assert_eq!(redact_url("http://npvr:8866/service?method=session.login&sid=S&md5=abc"), "http://npvr:8866/service?…");
        assert_eq!(redact_url("https://me:secret@iptv.example.com:8080/get.php?username=me&password=secret"), "https://iptv.example.com:8080/get.php?…");
        assert_eq!(redact_url("https://api.hdhomerun.com/api/guide?DeviceAuth=XyZ"), "https://api.hdhomerun.com/api/guide?…");
        assert_eq!(redact_url("http://tvh:9981/api/epg"), "http://tvh:9981/api/epg");
        assert_eq!(redact_url("http://user@host"), "http://host");
        assert_eq!(redact_url("http://host/a#token=1"), "http://host/a?…");
        assert_eq!(
            redact("request to http://x.tv/live?user=u&pass=p failed, then https://a:b@y.tv/z?k=1 timed out"),
            "request to http://x.tv/live?… failed, then https://y.tv/z?… timed out"
        );
        assert_eq!(redact("host not found"), "host not found");
    }

    #[test]
    fn errors_name_the_service_and_what_to_check() {
        assert_eq!(address("https://user:pw@jf.home/jellyfin/"), "jf.home/jellyfin");
        assert_eq!(address("http://192.168.0.20:8096?api_key=k"), "192.168.0.20:8096?…");
        assert_eq!(didnt_answer("Jellyfin", "http://192.168.0.20:8096"), "Jellyfin didn't answer at 192.168.0.20:8096. Check the server, then try again.");
        assert_eq!(answered_error("Jellyfin", "http://192.168.0.20:8096/", 503), "Jellyfin at 192.168.0.20:8096 answered with error 503. Check the server, then try again.");
        assert_eq!(wrong_server("Jellyfin", "http://nas:8096"), "The server at nas:8096 didn't answer like a Jellyfin server. Check the server address in the plugin's settings.");
        assert_eq!(wrong_server("Emby", "http://nas:8096"), "The server at nas:8096 didn't answer like an Emby server. Check the server address in the plugin's settings.");
        assert_eq!(unreadable("Plex", "http://nas:32400"), "Plex at nas:32400 sent a reply this plugin can't read.");
        assert_eq!(offline("Trakt"), "Trakt didn't answer. Check the TV's internet connection, then try again.");
        assert_eq!(online_error("Twitch", 502), "Twitch answered with error 502. Try again later.");
        assert_eq!(not_set("server address"), "Add the server address in the plugin's settings.");
        assert_eq!(host::refusal("fetch budget for this call is used up".into()), host::BUDGET_SPENT);
        assert_eq!(host::refusal("the body is larger than a plugin may read once encoded".into()), host::TOO_LARGE);
        assert_eq!(host::refusal("http://x.tv/live?user=u didn't answer".into()), "http://x.tv/live?… didn't answer");
        // Off the TV there is no host, so every fetch fails as an unreachable server would.
        assert_eq!(
            fetch_from("Emby", "http://192.168.0.20:8096", &HttpRequest::get("http://192.168.0.20:8096/emby/Users/u")).err().as_deref(),
            Some("Emby didn't answer at 192.168.0.20:8096. Check the server, then try again.")
        );
        assert_eq!(fetch_online("Trakt", &HttpRequest::get("https://api.trakt.tv/sync/playback")).err().as_deref(), Some(offline("Trakt").as_str()));
    }

    #[test]
    fn base64_bodies_decode_where_they_lie() {
        let original: Vec<u8> = (0..=255u8).cycle().take(1000).collect();
        let r = HttpResponse { status: 200, body_base64: Some(base64_encode(&original)), ..Default::default() };
        assert_eq!(body_bytes(r), original);
        let text = HttpResponse { status: 200, body: "plain".into(), ..Default::default() };
        assert_eq!(body_bytes(text), b"plain");
    }

    #[test]
    fn host_answers_are_unescaped_in_place() {
        let body = "line one\nquote \" back \\ slash / tab\t é 😀 \u{1}";
        let answer = serde_json::to_vec(&Outcome::Ok(HttpResponse { status: 200, headers: [("x-a".to_string(), "1".to_string())].into(), body: body.into(), body_base64: None })).unwrap();
        let r = wire::read_response(answer).unwrap();
        assert_eq!(r.body, body);
        assert_eq!(r.headers["x-a"], "1");

        let escaped = br#"{"ok":{"status":201,"body":"\u00e9\ud83d\ude00\ud800x\/","body_base64":null}}"#.to_vec();
        let r = wire::read_response(escaped).unwrap();
        assert_eq!((r.status, r.body.as_str()), (201, "é😀\u{FFFD}x/"));

        let b64 = serde_json::to_vec(&Outcome::Ok(HttpResponse { status: 200, body_base64: Some(base64_encode(&[0x1f, 0x8b, 0, 255])), ..Default::default() })).unwrap();
        let r = wire::read_response(b64).unwrap();
        assert_eq!(body_bytes(r), [0x1f, 0x8b, 0, 255]);

        let refusal = "This plugin may not reach evil.com, which isn't on its allow list. Check the addresses in its settings.";
        let err = wire::read_response(format!(r#"{{"err":"{refusal}"}}"#).into_bytes()).unwrap_err();
        assert_eq!(err, refusal);
        assert!(wire::read_response(br#"{"ok":{"status":200,"body":"\x"}}"#.to_vec()).is_err());
    }

    #[test]
    fn arrays_are_read_one_element_at_a_time() {
        let mut seen = Vec::new();
        for_each_in_array::<u32>("[3, 1, 2]", |n| seen.push(n)).unwrap();
        assert_eq!(seen, [3, 1, 2]);
        assert!(for_each_in_array::<u32>("{\"a\": 1}", |_| {}).is_err());
    }

    fn programme(channel: &str, start: i64) -> Item {
        Item { id: format!("{channel}:{start}"), kind: ItemKind::Schedule, channel_id: Some(channel.into()), start_ms: Some(start), ..Default::default() }
    }

    #[test]
    fn the_schedule_budget_is_shared_between_channels() {
        let mut items = Vec::new();
        for start in (0..100).rev() {
            items.push(programme("busy", start));
        }
        for start in 0..3 {
            items.push(programme("quiet", start * 10));
            items.push(programme("quiet", start * 10));
        }
        items.push(programme("other", 50));
        let kept = fit_schedule(items, 10);
        assert_eq!(kept.len(), 10);
        let starts = |c: &str| kept.iter().filter(|i| i.channel_id.as_deref() == Some(c)).map(|i| i.start_ms.unwrap()).collect::<Vec<_>>();
        assert_eq!(starts("quiet"), [0, 10, 20]);
        assert_eq!(starts("other"), [50]);
        assert_eq!(starts("busy"), [0, 1, 2, 3, 4, 5]);
        assert_eq!(fair_share(&[100, 3, 1], 10), 6);
        assert_eq!(fair_share(&[5, 5], 100), 5);
        assert_eq!(fair_share(&[], 10), 0);
        assert!(fit_schedule(vec![programme("a", 1)], 0).is_empty());
    }

    #[test]
    fn leftover_places_go_to_the_earliest_programmes() {
        let items = vec![programme("a", 30), programme("a", 0), programme("b", 5), programme("b", 10), programme("c", 7), programme("c", 20)];
        let kept = fit_schedule(items, 5);
        let mut ids: Vec<String> = kept.into_iter().map(|i| i.id).collect();
        ids.sort();
        assert_eq!(ids, ["a:0", "b:10", "b:5", "c:20", "c:7"]);
    }

    #[test]
    fn a_long_schedule_is_trimmed_as_it_arrives() {
        let mut budget = ScheduleBudget::new(30);
        for start in 0..1000 {
            for channel in ["a", "b", "c"] {
                budget.push(programme(channel, 1000 - start));
            }
            assert!(budget.held <= 2 * 30 + 3 + 256);
        }
        budget.push(programme("d", 5));
        let kept = budget.finish();
        assert_eq!(kept.len(), 30);
        let starts = |c: &str| kept.iter().filter(|i| i.channel_id.as_deref() == Some(c)).map(|i| i.start_ms.unwrap()).collect::<Vec<_>>();
        assert_eq!(starts("d"), [5]);
        assert_eq!(starts("a"), (1..=10).collect::<Vec<_>>()[..starts("a").len()]);
        assert!(starts("a").len() >= 9 && starts("b").len() >= 9);
    }

    #[test]
    fn a_word_at_a_time_finds_what_a_byte_at_a_time_does() {
        let slow = |hay: &[u8], bytes: &[u8]| hay.iter().position(|b| bytes.contains(b));
        // Every length across two words, with the wanted byte at every place and nowhere.
        for len in 0..20 {
            for at in 0..=len {
                let mut hay = vec![b'a'; len];
                if at < len {
                    hay[at] = b'<';
                }
                assert_eq!(find_first_of(&hay, b"<"), slow(&hay, b"<"), "{hay:?}");
                assert_eq!(find_first_of(&hay, b">\"<"), slow(&hay, b">\"<"), "{hay:?}");
            }
        }
        // The byte below a wanted one borrows from it and must not be taken for one.
        assert_eq!(find_first_of(&[0x3D, 0x3C, 0x3C, 0x3D, 0x3D, 0x3D, 0x3D, 0x3D], b"<"), Some(1));
        assert_eq!(find_first_of(&[0x3B, 0x3D, 0x3B, 0x3D, 0x3B, 0x3D, 0x3B, 0x3D, 0x3C], b"<"), Some(8));
        // High bytes, zero bytes and several wanted bytes together.
        let text = "naïve — \u{0} \"quoted\" 'single' <tag attr=\"v\">text</tag>\\".as_bytes();
        for from in 0..text.len() {
            for bytes in [&b"<"[..], b">", b"\\", b"\"", b">\"'", b"\0", &[0xC3], &[0xEF, 0x80]] {
                assert_eq!(find_first_of(&text[from..], bytes), slow(&text[from..], bytes), "from {from} for {bytes:?}");
            }
        }
        assert_eq!(find_first_of(b"anything", b""), None);
    }

    #[test]
    fn repeated_ids_are_dropped() {
        let mut items = vec![programme("a", 1), programme("b", 1), programme("a", 1)];
        assert_eq!(dedup_ids(&mut items), 1);
        assert_eq!(items.len(), 2);
    }
}
