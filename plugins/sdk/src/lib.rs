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

use std::collections::{HashMap, HashSet};

/// Most items the host keeps from one refresh; anything after them is dropped.
pub const MAX_ITEMS: usize = 2_000;

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

    /// Perform an HTTP request. The host refuses hosts outside the manifest's allow list. Any
    /// address in an error has its query string and password taken out.
    pub fn fetch(request: &HttpRequest) -> Result<HttpResponse, String> {
        let req = serde_json::to_vec(request).map_err(|e| e.to_string())?;
        let (p, l) = pass(&req);
        let out = take(unsafe { raw::fetch(p, l) }).ok_or_else(|| "no response from host".to_string())?;
        drop(req);
        crate::wire::read_response(out).map_err(|e| if e.contains("body is larger than") { TOO_LARGE.to_string() } else { crate::redact(&e) })
    }

    /// The error `fetch` returns when an answer is bigger than the host lets a plugin read.
    pub const TOO_LARGE: &str = "the answer is larger than the 8 MB a plugin may read";

    /// GET a URL and return the body, failing on non-2xx.
    pub fn get_text(url: &str) -> Result<String, String> {
        let r = fetch(&HttpRequest::get(url))?;
        if !r.ok() {
            return Err(format!("HTTP {} for {}", r.status, crate::redact_url(url)));
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

    pub fn required_setting(key: &str, label: &str) -> Result<String, String> {
        setting(key).ok_or_else(|| format!("{label} is not set"))
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
            let next = buf[r..].iter().position(|&b| b == b'\\').map_or(buf.len(), |i| r + i);
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

        let err = wire::read_response(br#"{"err":"host evil.com is not in the plugin's allow list"}"#.to_vec()).unwrap_err();
        assert!(err.contains("allow list"));
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
    fn repeated_ids_are_dropped() {
        let mut items = vec![programme("a", 1), programme("b", 1), programme("a", 1)];
        assert_eq!(dedup_ids(&mut items), 1);
        assert_eq!(items.len(), 2);
    }
}
