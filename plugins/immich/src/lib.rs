//! Immich photos: the pictures in an album shared from Immich become the screensaver. A shared
//! link needs no account on the TV, and its key lets the launcher fetch the pictures without
//! headers, which is what the artwork pipeline needs.
use lounge_plugin_sdk::{self as sdk, host, Item, ItemKind, Kind, Manifest, RefreshContext, SettingKind, SettingSpec};
use serde::Deserialize;

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
            fetch_hosts: vec!["*".into()],
            refresh_minutes: 6 * 60,
            rows: vec![],
            settings: vec![
                SettingSpec {
                    key: "link".into(),
                    label: "Shared album link".into(),
                    kind: SettingKind::Url,
                    required: true,
                    help: Some("In Immich open the album, Share, Create link, and paste the whole address, like https://photos.example.com/share/AbC123…".into()),
                    default: None,
                },
                SettingSpec { key: "password".into(), label: "Link password".into(), kind: SettingKind::Password, required: false, help: Some("Only if the shared link has one.".into()), default: None },
                SettingSpec { key: "max".into(), label: "Pictures to show".into(), kind: SettingKind::Number, required: false, help: Some("The newest ones are kept.".into()), default: Some("60".into()) },
            ],
            homepage: Some("https://immich.app".into()),
        }
    }

    fn refresh(_ctx: &RefreshContext) -> Result<Vec<Item>, String> {
        let link = host::required_setting("link", "Shared album link")?;
        let (base, key) = parse_link(&link)?;
        let password = host::secret("password").filter(|p| !p.is_empty());
        let max = host::setting("max").and_then(|v| v.trim().parse::<usize>().ok()).unwrap_or(60).clamp(1, 300);
        let auth = query(&key, password.as_deref());
        let r = host::fetch(&sdk::HttpRequest::get(format!("{base}/api/shared-links/me?{auth}")).header("Accept", "application/json"))?;
        match r.status {
            401 => return Err("Immich refused the link. If it has a password, enter it in the plugin's settings.".into()),
            404 => return Err("Immich does not know this link any more. Create a new shared link for the album.".into()),
            s if !(200..300).contains(&s) => return Err(format!("Immich answered HTTP {s}")),
            _ => {}
        }
        let shared: SharedLink = serde_json::from_str(&r.body).map_err(|e| format!("unexpected answer from Immich: {e}"))?;
        let album = shared.album.as_ref().and_then(|a| a.album_name.clone()).unwrap_or_else(|| "Photos".into());
        let mut assets: Vec<Asset> = shared.assets.into_iter().chain(shared.album.map(|a| a.assets).unwrap_or_default()).filter(|a| a.kind == "IMAGE").collect();
        assets.sort_by_key(|a| std::cmp::Reverse(a.taken_ms()));
        assets.dedup_by(|a, b| a.id == b.id);
        assets.truncate(max);
        let count = assets.len();
        sdk::info!("{count} pictures from \"{album}\"");
        Ok(assets
            .into_iter()
            .enumerate()
            .map(|(i, a)| Item {
                id: format!("asset:{}", a.id),
                kind: ItemKind::Wallpaper,
                title: album.clone(),
                subtitle: a.taken_ms().map(month_year),
                added_ms: a.taken_ms(),
                thumbnail: Some(format!("{base}/api/assets/{}/thumbnail?size=preview&{auth}", a.id)),
                weight: (count - i) as i32,
                ..Default::default()
            })
            .collect())
    }
}

sdk::export_plugin!(Immich);

#[derive(Deserialize, Default)]
struct SharedLink {
    #[serde(default)]
    album: Option<Album>,
    #[serde(default)]
    assets: Vec<Asset>,
}

#[derive(Deserialize, Default)]
struct Album {
    #[serde(rename = "albumName")]
    album_name: Option<String>,
    #[serde(default)]
    assets: Vec<Asset>,
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
}

impl Asset {
    fn taken_ms(&self) -> Option<i64> {
        self.exif.as_ref().and_then(|e| e.date_time_original.as_deref()).or(self.file_created_at.as_deref()).and_then(sdk::parse_iso8601)
    }
}

/// "https://photos.example.com/share/AbC123" → the server and the link's key.
pub fn parse_link(link: &str) -> Result<(String, String), String> {
    let link = link.trim().trim_end_matches('/');
    let (base, key) = link.split_once("/share/").ok_or_else(|| "That is not a shared link: it should contain /share/ followed by the key".to_string())?;
    let key: String = key.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_').collect();
    if key.is_empty() || !(base.starts_with("http://") || base.starts_with("https://")) {
        return Err("That is not a shared link: it should look like https://photos.example.com/share/AbC123".into());
    }
    Ok((base.to_string(), key))
}

fn query(key: &str, password: Option<&str>) -> String {
    match password {
        Some(p) => format!("key={key}&password={}", url_encode(p)),
        None => format!("key={key}"),
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
        assert_eq!(parse_link("https://photos.example.com/share/AbC-123_x/").unwrap(), ("https://photos.example.com".into(), "AbC-123_x".into()));
        assert!(parse_link("https://photos.example.com/albums/1").is_err());
        assert!(parse_link("photos.example.com/share/k").is_err());
    }

    #[test]
    fn a_shared_album_reads_its_pictures() {
        let s: SharedLink = serde_json::from_str(r#"{"type": "ALBUM", "album": {"albumName": "Summer", "assets": [
            {"id": "a1", "type": "IMAGE", "fileCreatedAt": "2025-10-03T10:00:00.000Z", "exifInfo": {"dateTimeOriginal": "2025-10-01T09:00:00.000Z"}},
            {"id": "v1", "type": "VIDEO", "fileCreatedAt": "2025-10-04T10:00:00.000Z"},
            {"id": "a2", "type": "IMAGE", "fileCreatedAt": "2025-12-24T10:00:00.000Z"}
        ]}, "assets": []}"#).unwrap();
        let album = s.album.as_ref().unwrap();
        assert_eq!(album.album_name.as_deref(), Some("Summer"));
        let images: Vec<&Asset> = album.assets.iter().filter(|a| a.kind == "IMAGE").collect();
        assert_eq!(images.len(), 2);
        assert_eq!(images[0].taken_ms(), sdk::parse_iso8601("2025-10-01T09:00:00.000Z"));
        assert_eq!(month_year(images[1].taken_ms().unwrap()), "December 2025");
    }

    #[test]
    fn months_come_out_right_across_years() {
        assert_eq!(month_year(0), "January 1970");
        assert_eq!(month_year(sdk::parse_iso8601("2024-02-29T00:00:00Z").unwrap()), "February 2024");
        assert_eq!(month_year(sdk::parse_iso8601("2026-10-19T23:59:59Z").unwrap()), "October 2026");
    }

    #[test]
    fn the_key_and_password_travel_as_query() {
        assert_eq!(query("k", None), "key=k");
        assert_eq!(query("k", Some("p w")), "key=k&password=p%20w");
    }
}
