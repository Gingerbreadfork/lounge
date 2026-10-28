//! Immich photos: the pictures in an album shared from Immich become the screensaver. A shared
//! link needs no account on the TV, and its key lets the launcher fetch the pictures without
//! headers, which is what the artwork pipeline needs.
use lounge_plugin_sdk::{self as sdk, host, HttpRequest, Item, ItemKind, Kind, Manifest, RefreshContext, SettingKind, SettingSpec};
use serde::Deserialize;
use std::collections::HashSet;

/// Most months of an album read through the timeline in one refresh.
const MAX_MONTHS: usize = 24;
/// A photo narrower than this, width over height, would lose too much filling the screen, so it
/// is hung whole instead.
const FRAME_BELOW: f64 = 1.25;

pub struct Immich;

impl sdk::Plugin for Immich {
    fn manifest() -> Manifest {
        Manifest {
            id: "tv.lounge.immich".into(),
            name: "Immich photos".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            description: "An album shared from Immich becomes the screensaver.".into(),
            publisher_name: "Lounge".into(),
            kinds: vec![Kind::Wallpaper],
            fetch_hosts: vec!["$link".into(), "*".into()],
            refresh_minutes: 6 * 60,
            rows: vec![],
            settings: vec![
                SettingSpec {
                    key: "link".into(),
                    label: "Shared album link".into(),
                    kind: SettingKind::Url,
                    required: true,
                    help: Some("In Immich open the album, Share, Create link, and paste the whole address, like https://photos.example.com/share/AbC123… or its custom /s/ address.".into()),
                    default: None,
                },
                SettingSpec { key: "password".into(), label: "Link password".into(), kind: SettingKind::Password, required: false, help: Some("Only if the shared link has one.".into()), default: None },
                SettingSpec { key: "max".into(), label: "Pictures to show".into(), kind: SettingKind::Number, required: false, help: Some("The newest ones are kept. Pictures come at Immich's preview size; for a 4K panel set that to 2160 in Immich's image settings.".into()), default: Some("60".into()) },
            ],
            homepage: Some("https://immich.app".into()),
        }
    }

    fn refresh(_ctx: &RefreshContext) -> Result<Vec<Item>, String> {
        let link = host::required_setting("link", "Shared album link")?;
        let (base, name) = parse_link(&link)?;
        let password = host::secret("password").filter(|p| !p.is_empty());
        let max = host::setting("max").and_then(|v| v.trim().parse::<usize>().ok()).unwrap_or(60).clamp(1, 300);
        let auth = query(&name, password.as_deref());
        let shared = shared_link(&base, &name, password.as_deref())?;
        let album = shared.album.as_ref().and_then(|a| a.album_name.clone()).unwrap_or_else(|| "Photos".into());
        // Older servers list an album's pictures with the link; newer ones serve them a month at a time.
        let (mut pictures, album_id) = match shared.album {
            Some(a) => ([shared.assets.0, a.assets.0].concat(), Some(a.id).filter(|id| !id.is_empty())),
            None => (shared.assets.0, None),
        };
        if let (true, Some(id)) = (pictures.is_empty(), album_id) {
            pictures = timeline(&base, &id, &auth, max)?;
        }
        let pictures = newest(pictures, max);
        let count = pictures.len();
        sdk::info!("{count} pictures from \"{album}\"");
        Ok(pictures
            .into_iter()
            .enumerate()
            .map(|(i, p)| Item {
                id: format!("asset:{}", p.id),
                kind: ItemKind::Wallpaper,
                title: album.clone(),
                subtitle: p.taken_ms.map(|t| month_year(t + p.local_offset_ms)),
                added_ms: p.taken_ms,
                thumbnail: Some(format!("{base}/api/assets/{}/thumbnail?size=preview&{auth}", p.id)),
                framed: p.framed,
                weight: (count - i) as i32,
                ..Default::default()
            })
            .collect())
    }
}

const TOO_BIG: &str = "The album is too big to read: Immich answered with more than the 8 MB a plugin may read at once. Turn off Show metadata on the shared link, or share a smaller album.";

/// GET from Immich, turning its refusals into advice.
fn get(url: &str) -> Result<String, String> {
    let (status, body) = send(&HttpRequest::get(url).header("Accept", "application/json"))?;
    answer(status, body)
}

/// The shared link and its album. From 2.6 a link with a password is signed in to first; older
/// servers have no sign-in and take the password with the request instead.
fn shared_link(base: &str, name: &str, password: Option<&str>) -> Result<SharedLink, String> {
    let body = match password {
        Some(p) => {
            let mut login = HttpRequest::get(format!("{base}/api/shared-links/login?{}", query(name, None)))
                .header("Content-Type", "application/json")
                .header("Accept", "application/json");
            login.method = "POST".into();
            login.body = Some(serde_json::json!({ "password": p }).to_string());
            match send(&login)? {
                (404, _) => get(&format!("{base}/api/shared-links/me?{}", query(name, Some(p))))?,
                (status, body) => answer(status, body)?,
            }
        }
        None => get(&format!("{base}/api/shared-links/me?{}", query(name, None)))?,
    };
    serde_json::from_str(&body).map_err(|e| format!("unexpected answer from Immich: {e}"))
}

/// Sends `req` and reads the whole answer, whatever its status.
fn send(req: &HttpRequest) -> Result<(u16, String), String> {
    match host::fetch(req) {
        Err(e) if e == host::TOO_LARGE => Err(TOO_BIG.into()),
        Err(e) => Err(e),
        Ok(r) => Ok((r.status, r.body)),
    }
}

fn answer(status: u16, body: String) -> Result<String, String> {
    match status {
        401 => Err(refusal(&body)),
        404 => Err("Immich does not know this link any more. Create a new shared link for the album.".into()),
        s if !(200..300).contains(&s) => Err(format!("Immich answered HTTP {s}")),
        _ => Ok(body),
    }
}

/// Advice for a 401, whose message says whether the key or the password was wrong.
fn refusal(body: &str) -> String {
    let message = serde_json::from_str::<serde_json::Value>(body).ok().and_then(|v| v["message"].as_str().map(str::to_lowercase)).unwrap_or_default();
    if message.contains("invalid password") {
        "Immich turned down the link's password. Check it in the plugin's settings.".into()
    } else if message.contains("password") {
        "This link has a password. Enter it in the plugin's settings.".into()
    } else if message.contains("share key") {
        "Immich does not know this link any more. Create a new shared link for the album.".into()
    } else {
        "Immich refused the link. If it has a password, enter it in the plugin's settings.".into()
    }
}

/// The newest `max` pictures of an album, read a month at a time from Immich's timeline,
/// newest month first.
fn timeline(base: &str, album_id: &str, auth: &str, max: usize) -> Result<Vec<Picture>, String> {
    let mut months: Vec<Month> = serde_json::from_str(&get(&format!("{base}/api/timeline/buckets?albumId={album_id}&order=desc&size=MONTH&{auth}"))?)
        .map_err(|e| format!("unexpected album timeline from Immich: {e}"))?;
    months.sort_by(|a, b| b.time_bucket.cmp(&a.time_bucket));
    let mut out = Vec::new();
    for month in months.iter().take(MAX_MONTHS) {
        if out.len() >= max {
            break;
        }
        let bucket = if month.time_bucket.contains('T') { month.time_bucket.clone() } else { format!("{}T00:00:00.000Z", month.time_bucket) };
        let body = get(&format!("{base}/api/timeline/bucket?albumId={album_id}&timeBucket={}&order=desc&size=MONTH&{auth}", url_encode(&bucket)))?;
        out.extend(month_pictures(&body)?);
    }
    Ok(out)
}

/// The pictures in one month of the timeline: columns of values from newer servers, or a list
/// of assets from older ones.
pub fn month_pictures(json: &str) -> Result<Vec<Picture>, String> {
    if let Ok(c) = serde_json::from_str::<Columns>(json) {
        let offset = |i: usize| c.local_offset_hours.get(i).copied().flatten().map_or(0, |h| (h * 3_600_000.0) as i64);
        return Ok((0..c.id.len())
            .filter(|&i| c.is_image.get(i).copied().unwrap_or(false))
            .map(|i| Picture {
                id: c.id[i].clone(),
                taken_ms: c.file_created_at.get(i).and_then(|t| t.as_deref()).and_then(sdk::parse_iso8601),
                local_offset_ms: offset(i),
                framed: c.ratio.get(i).is_some_and(|&r| r > 0.0 && r < FRAME_BELOW),
            })
            .collect());
    }
    let assets: Images = serde_json::from_str(json).map_err(|e| format!("unexpected album month from Immich: {e}"))?;
    Ok(assets.0)
}

/// The newest `max`, each picture once.
fn newest(mut pictures: Vec<Picture>, max: usize) -> Vec<Picture> {
    pictures.sort_by_key(|p| std::cmp::Reverse(p.taken_ms));
    let mut seen = HashSet::new();
    pictures.retain(|p| seen.insert(p.id.clone()));
    pictures.truncate(max);
    pictures
}

sdk::export_plugin!(Immich);

#[derive(Deserialize, Default)]
struct SharedLink {
    #[serde(default)]
    album: Option<Album>,
    #[serde(default)]
    assets: Images,
}

#[derive(Deserialize, Default)]
struct Album {
    #[serde(default)]
    id: String,
    #[serde(rename = "albumName")]
    album_name: Option<String>,
    #[serde(default)]
    assets: Images,
}

#[derive(Deserialize)]
struct Month {
    #[serde(rename = "timeBucket")]
    time_bucket: String,
}

/// One month of the timeline as newer servers send it: a column per field.
#[derive(Deserialize)]
struct Columns {
    id: Vec<String>,
    #[serde(rename = "isImage")]
    is_image: Vec<bool>,
    #[serde(rename = "fileCreatedAt", default)]
    file_created_at: Vec<Option<String>>,
    #[serde(rename = "localOffsetHours", default)]
    local_offset_hours: Vec<Option<f64>>,
    /// Width over height, as the picture is shown.
    #[serde(default)]
    ratio: Vec<f64>,
}

/// A picture to show: its id and when it was taken, in UTC, with the offset to local time
/// where Immich knows it, and whether it is hung whole rather than filling the screen.
#[derive(Clone, Debug, PartialEq)]
pub struct Picture {
    pub id: String,
    pub taken_ms: Option<i64>,
    pub local_offset_ms: i64,
    pub framed: bool,
}

/// A list of assets kept as pictures: videos are dropped and each asset is cut down to what a
/// picture needs as it is read, so a long list with metadata stays small.
#[derive(Default)]
struct Images(Vec<Picture>);

impl<'de> Deserialize<'de> for Images {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Images, D::Error> {
        struct Visitor;
        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = Images;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("a list of assets")
            }
            fn visit_seq<A: serde::de::SeqAccess<'de>>(self, mut seq: A) -> Result<Images, A::Error> {
                let mut out = Vec::new();
                while let Some(a) = seq.next_element::<Asset>()? {
                    if a.kind == "IMAGE" {
                        let (taken_ms, framed) = (a.taken_ms(), a.framed());
                        out.push(Picture { id: a.id, taken_ms, local_offset_ms: 0, framed });
                    }
                }
                Ok(Images(out))
            }
        }
        d.deserialize_seq(Visitor)
    }
}

#[derive(Deserialize, Default, Clone)]
pub struct Asset {
    #[serde(default)]
    pub id: String,
    #[serde(rename = "type", default)]
    pub kind: String,
    #[serde(rename = "fileCreatedAt")]
    pub file_created_at: Option<String>,
    #[serde(rename = "exifInfo")]
    pub exif: Option<Exif>,
}

#[derive(Deserialize, Default, Clone)]
pub struct Exif {
    #[serde(rename = "dateTimeOriginal")]
    pub date_time_original: Option<String>,
    #[serde(rename = "exifImageWidth")]
    pub width: Option<f64>,
    #[serde(rename = "exifImageHeight")]
    pub height: Option<f64>,
    /// The EXIF orientation, as text or a number; 5 to 8 turn the picture on its side.
    pub orientation: Option<serde_json::Value>,
}

impl Asset {
    fn taken_ms(&self) -> Option<i64> {
        self.exif.as_ref().and_then(|e| e.date_time_original.as_deref()).or(self.file_created_at.as_deref()).and_then(sdk::parse_iso8601)
    }

    fn framed(&self) -> bool {
        let Some(e) = &self.exif else { return false };
        let (Some(w), Some(h)) = (e.width, e.height) else { return false };
        let turned = e.orientation.as_ref().and_then(|o| o.as_i64().or_else(|| o.as_str()?.trim().parse().ok())).is_some_and(|o| (5..=8).contains(&o));
        let ratio = if turned { h / w } else { w / h };
        ratio > 0.0 && ratio < FRAME_BELOW
    }
}

/// "https://photos.example.com/share/AbC123" → the server and how a request names the link: by
/// its key, or by the custom slug of a /s/ address.
pub fn parse_link(link: &str) -> Result<(String, String), String> {
    let link = link.trim().trim_end_matches('/');
    let token = |rest: &str| rest.chars().take_while(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '~')).collect::<String>();
    let (base, name) = match (link.split_once("/share/"), link.split_once("/s/")) {
        (Some((base, rest)), _) => (base, Some(token(rest)).filter(|k| !k.is_empty()).map(|k| format!("key={k}"))),
        (None, Some((base, rest))) => (base, Some(token(rest)).filter(|k| !k.is_empty()).map(|k| format!("slug={k}"))),
        (None, None) => return Err("That is not a shared link: it should contain /share/ or /s/ followed by the link's name".into()),
    };
    match name {
        Some(name) if base.starts_with("http://") || base.starts_with("https://") => Ok((base.to_string(), name)),
        _ => Err("That is not a shared link: it should look like https://photos.example.com/share/AbC123".into()),
    }
}

fn query(name: &str, password: Option<&str>) -> String {
    match password {
        Some(p) => format!("{name}&password={}", url_encode(p)),
        None => name.to_string(),
    }
}

/// "October 2025" from epoch milliseconds, in UTC.
pub fn month_year(ms: i64) -> String {
    let days = ms.div_euclid(86_400_000);
    let (y, m) = civil_from_days(days);
    const MONTHS: [&str; 12] = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];
    format!("{} {y}", MONTHS[(m - 1) as usize])
}

/// Howard Hinnant's days-to-civil, year and month only.
fn civil_from_days(z: i64) -> (i64, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m)
}

fn url_encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn links_split_into_server_and_key() {
        assert_eq!(parse_link("https://photos.example.com/share/AbC-123_x/").unwrap(), ("https://photos.example.com".into(), "key=AbC-123_x".into()));
        assert_eq!(parse_link("https://photos.example.com/s/summer-2025").unwrap(), ("https://photos.example.com".into(), "slug=summer-2025".into()));
        assert!(parse_link("https://photos.example.com/albums/1").is_err());
        assert!(parse_link("photos.example.com/share/k").is_err());
    }

    #[test]
    fn a_shared_album_reads_its_pictures() {
        let s: SharedLink = serde_json::from_str(r#"{"type": "ALBUM", "album": {"id": "al1", "albumName": "Summer", "assets": [
            {"id": "a1", "type": "IMAGE", "fileCreatedAt": "2025-10-03T10:00:00.000Z", "exifInfo": {"dateTimeOriginal": "2025-10-01T09:00:00.000Z", "make": "Canon", "description": "long"}},
            {"id": "v1", "type": "VIDEO", "fileCreatedAt": "2025-10-04T10:00:00.000Z"},
            {"id": "a2", "type": "IMAGE", "fileCreatedAt": "2025-12-24T10:00:00.000Z", "people": [{"name": "Ann"}]}
        ]}, "assets": []}"#).unwrap();
        let album = s.album.as_ref().unwrap();
        assert_eq!((album.id.as_str(), album.album_name.as_deref()), ("al1", Some("Summer")));
        let images = &album.assets.0;
        assert_eq!(images.len(), 2);
        assert_eq!(images[0].taken_ms, sdk::parse_iso8601("2025-10-01T09:00:00.000Z"));
        assert_eq!(month_year(images[1].taken_ms.unwrap()), "December 2025");
    }

    #[test]
    fn newer_servers_send_an_album_without_its_pictures() {
        let s: SharedLink = serde_json::from_str(r#"{"type": "ALBUM", "assets": [], "album": {"id": "al1", "albumName": "Summer", "assetCount": 3000}}"#).unwrap();
        assert!(s.assets.0.is_empty() && s.album.unwrap().assets.0.is_empty());
    }

    #[test]
    fn timeline_months_come_in_columns_or_as_assets() {
        let columns = r#"{"id": ["p1", "v1", "p2"], "ownerId": ["o", "o", "o"], "ratio": [1.5, 1.7, 0.75], "isFavorite": [false, false, true],
            "visibility": ["timeline", "timeline", "timeline"], "isTrashed": [false, false, false], "isImage": [true, false, true],
            "thumbhash": [null, null, null], "createdAt": ["2025-10-02T00:00:00.000Z", "2025-10-02T00:00:00.000Z", "2025-10-02T00:00:00.000Z"],
            "fileCreatedAt": ["2025-10-31T23:30:00.000Z", "2025-10-02T00:00:00.000Z", null], "localOffsetHours": [2, 0, null],
            "duration": [null, "0:00:05.000", null], "projectionType": [null, null, null], "livePhotoVideoId": [null, null, null]}"#;
        let pictures = month_pictures(columns).unwrap();
        assert_eq!(pictures.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(), ["p1", "p2"]);
        assert_eq!(pictures.iter().map(|p| p.framed).collect::<Vec<_>>(), [false, true]);
        assert_eq!(month_year(pictures[0].taken_ms.unwrap() + pictures[0].local_offset_ms), "November 2025");
        assert_eq!(pictures[1].taken_ms, None);
        let assets = r#"[{"id": "a1", "type": "IMAGE", "fileCreatedAt": "2024-02-29T12:00:00.000Z"}, {"id": "v", "type": "VIDEO"}]"#;
        assert_eq!(month_pictures(assets).unwrap(), [Picture { id: "a1".into(), taken_ms: sdk::parse_iso8601("2024-02-29T12:00:00.000Z"), local_offset_ms: 0, framed: false }]);
        let turned = r#"[{"id": "t", "type": "IMAGE", "exifInfo": {"exifImageWidth": 4000, "exifImageHeight": 3000, "orientation": "6"}},
            {"id": "w", "type": "IMAGE", "exifInfo": {"exifImageWidth": 4000, "exifImageHeight": 3000, "orientation": "1"}},
            {"id": "n", "type": "IMAGE", "exifInfo": {"exifImageWidth": 4000, "exifImageHeight": 3000, "orientation": 8}}]"#;
        assert_eq!(month_pictures(turned).unwrap().iter().map(|p| p.framed).collect::<Vec<_>>(), [true, false, true]);
        assert!(month_pictures(r#"{"message": "Forbidden"}"#).is_err());
    }

    #[test]
    fn the_newest_pictures_are_kept_once_each() {
        let p = |id: &str, t: i64| Picture { id: id.into(), taken_ms: Some(t), local_offset_ms: 0, framed: false };
        let kept = newest(vec![p("a", 1), p("b", 3), p("a", 1), p("c", 2), Picture { id: "d".into(), taken_ms: None, local_offset_ms: 0, framed: false }], 3);
        assert_eq!(kept.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(), ["b", "c", "a"]);
    }

    #[test]
    fn months_come_out_right_across_years() {
        assert_eq!(month_year(0), "January 1970");
        assert_eq!(month_year(sdk::parse_iso8601("2024-02-29T00:00:00Z").unwrap()), "February 2024");
        assert_eq!(month_year(sdk::parse_iso8601("2026-10-19T23:59:59Z").unwrap()), "October 2026");
    }

    #[test]
    fn refusals_say_whether_the_link_or_the_password_is_wrong() {
        assert!(refusal(r#"{"message":"Invalid password","error":"Unauthorized","statusCode":401}"#).starts_with("Immich turned down the link's password"));
        assert!(refusal(r#"{"message":"Password required","statusCode":401}"#).starts_with("This link has a password"));
        assert!(refusal(r#"{"message":"Invalid share key","statusCode":401}"#).starts_with("Immich does not know this link"));
        assert!(refusal("<html>").starts_with("Immich refused the link"));
    }

    #[test]
    fn the_key_and_password_travel_as_query() {
        assert_eq!(query("key=k", None), "key=k");
        assert_eq!(query("slug=s", Some("p w")), "slug=s&password=p%20w");
    }
}
