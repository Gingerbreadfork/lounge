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
pub use serde_json;

pub trait Plugin {
    fn manifest() -> Manifest;
    fn refresh(ctx: &RefreshContext) -> Result<Vec<Item>, String>;
    fn search(_query: &str, _limit: u32) -> Result<Vec<Item>, String> {
        Ok(Vec::new())
    }
    fn resolve(_action: &str) -> Result<Action, String> {
        Err("this plugin does not resolve actions".to_string())
    }
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
    let outcome: Outcome<O> = match serde_json::from_slice::<I>(&input) {
        Ok(i) => match f(i) {
            Ok(o) => Outcome::Ok(o),
            Err(e) => Outcome::Err(e),
        },
        Err(e) => Outcome::Err(format!("bad input: {e}")),
    };
    __return_json(&outcome)
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
            $crate::__return_json(&$crate::Outcome::Ok(<$plugin as $crate::Plugin>::manifest()))
        }
        #[unsafe(no_mangle)]
        pub extern "C" fn lg_refresh(ptr: i32, len: i32) -> i64 {
            $crate::__call(ptr, len, |ctx: $crate::RefreshContext| <$plugin as $crate::Plugin>::refresh(&ctx))
        }
        #[unsafe(no_mangle)]
        pub extern "C" fn lg_search(ptr: i32, len: i32) -> i64 {
            $crate::__call(ptr, len, |q: $crate::SearchQuery| <$plugin as $crate::Plugin>::search(&q.query, q.limit))
        }
        #[unsafe(no_mangle)]
        pub extern "C" fn lg_resolve(ptr: i32, len: i32) -> i64 {
            $crate::__call(ptr, len, |a: String| <$plugin as $crate::Plugin>::resolve(&a))
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

    /// Perform an HTTP request. The host refuses hosts outside the manifest's allow list.
    pub fn fetch(request: &HttpRequest) -> Result<HttpResponse, String> {
        let req = serde_json::to_vec(request).map_err(|e| e.to_string())?;
        let (p, l) = pass(&req);
        let out = take(unsafe { raw::fetch(p, l) }).ok_or_else(|| "no response from host".to_string())?;
        match serde_json::from_slice::<Outcome<HttpResponse>>(&out).map_err(|e| e.to_string())? {
            Outcome::Ok(r) => Ok(r),
            Outcome::Err(e) => Err(e),
        }
    }

    /// GET a URL and return the body, failing on non-2xx.
    pub fn get_text(url: &str) -> Result<String, String> {
        let r = fetch(&HttpRequest::get(url))?;
        if !r.ok() {
            return Err(format!("HTTP {} for {url}", r.status));
        }
        Ok(if r.body_base64.is_some() { String::from_utf8_lossy(&r.bytes()).into_owned() } else { r.body })
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

    pub fn required_setting(key: &str, label: &str) -> Result<String, String> {
        setting(key).ok_or_else(|| format!("{label} is not set"))
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
/// milliseconds. Fractions are kept to the millisecond; a missing zone means UTC.
pub fn parse_iso8601(s: &str) -> Option<i64> {
    let s = s.trim();
    if s.len() < 19 {
        return None;
    }
    let (date, rest) = s.split_at(10);
    let rest = rest.strip_prefix(['T', 't', ' '])?;
    let year: i64 = date.get(0..4)?.parse().ok()?;
    let month: u32 = date.get(5..7)?.parse().ok()?;
    let day: u32 = date.get(8..10)?.parse().ok()?;
    let hour: u32 = rest.get(0..2)?.parse().ok()?;
    let minute: u32 = rest.get(3..5)?.parse().ok()?;
    let second: u32 = rest.get(6..8)?.parse().ok()?;
    let mut tail = rest.get(8..).unwrap_or("");
    let mut millis: i64 = 0;
    if let Some(frac) = tail.strip_prefix('.') {
        let digits: String = frac.chars().take_while(|c| c.is_ascii_digit()).collect();
        millis = format!("{digits:0<3}").get(..3).and_then(|d| d.parse().ok()).unwrap_or(0);
        tail = &frac[digits.len()..];
    }
    let offset_minutes = match tail.chars().next() {
        None | Some('Z') | Some('z') => 0,
        Some(sign @ ('+' | '-')) => {
            let hh: i32 = tail.get(1..3)?.parse().ok()?;
            let mm: i32 = tail.get(4..6).or_else(|| tail.get(3..5)).and_then(|m| m.parse().ok()).unwrap_or(0);
            let v = hh * 60 + mm;
            if sign == '-' { -v } else { v }
        }
        _ => return None,
    };
    Some(epoch_ms(year, month, day, hour, minute, second, offset_minutes) + millis)
}

#[cfg(test)]
mod iso_tests {
    use super::parse_iso8601;

    #[test]
    fn every_common_shape() {
        assert_eq!(parse_iso8601("2026-10-17T02:09:23+00:00"), Some(1792202963000));
        assert_eq!(parse_iso8601("2026-10-17T02:09:23.250Z"), Some(1792202963250));
        assert_eq!(parse_iso8601("2026-10-17T12:09:23+10:00"), Some(1792202963000));
        assert_eq!(parse_iso8601("2026-10-17 02:09:23"), Some(1792202963000));
        assert_eq!(parse_iso8601("2026-10-18T22:11:00.0000000Z"), Some(1792361460000));
        assert_eq!(parse_iso8601("yesterday"), None);
    }
}
