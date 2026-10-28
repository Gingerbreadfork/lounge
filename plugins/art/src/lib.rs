//! Art mode: public-domain paintings from the Met and the Cleveland Museum of Art become the
//! screensaver, each hung whole with the painter, the year and the museum under the clock. Every
//! refresh brings a few more into the gallery, and the wall shows a different selection each time.
use lounge_plugin_sdk::{self as sdk, host, HttpRequest, Item, ItemKind, Kind, Manifest, RefreshContext, SettingKind, SettingSpec};
use serde::{Deserialize, Serialize};

pub struct Art;

const GALLERY_KEY: &str = "gallery";
/// Paintings remembered between refreshes; the oldest go when the gallery is full.
const GALLERY_CAP: usize = 300;
/// Met objects looked up per refresh: two fetches each, inside the host's budget with room to spare.
const MET_PER_REFRESH: usize = 28;
/// The TV prepares a picture of up to 12 MB and 13 million pixels.
const PICTURE_MAX_BYTES: u64 = 12 * 1024 * 1024;
const PICTURE_MAX_PIXELS: u64 = 13_000_000;
const MET_PAGE: usize = 40;
const CMA_PAGE: usize = 60;
const MET: &str = "The Met";
const CMA: &str = "Cleveland Museum of Art";

impl sdk::Plugin for Art {
    fn manifest() -> Manifest {
        Manifest {
            id: "tv.lounge.art".into(),
            name: "Art mode".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            description: "Paintings from the Met and the Cleveland Museum of Art, hung on the screensaver with the painter and the year.".into(),
            publisher_name: "Lounge".into(),
            kinds: vec![Kind::Wallpaper],
            fetch_hosts: vec!["collectionapi.metmuseum.org".into(), "images.metmuseum.org".into(), "openaccess-api.clevelandart.org".into(), "openaccess-cdn.clevelandart.org".into()],
            refresh_minutes: 6 * 60,
            rows: vec![],
            settings: vec![
                SettingSpec { key: "met".into(), label: "The Met, New York".into(), kind: SettingKind::Toggle, required: false, help: None, default: Some("true".into()) },
                SettingSpec { key: "cleveland".into(), label: "Cleveland Museum of Art".into(), kind: SettingKind::Toggle, required: false, help: None, default: Some("true".into()) },
                SettingSpec {
                    key: "max".into(),
                    label: "Paintings on the wall".into(),
                    kind: SettingKind::Number,
                    required: false,
                    help: Some("The screensaver cycles through these; each refresh hangs some new ones.".into()),
                    default: Some("60".into()),
                },
                SettingSpec {
                    key: "query".into(),
                    label: "Only paintings matching".into(),
                    kind: SettingKind::Text,
                    required: false,
                    help: Some("A painter or a subject, such as Monet or landscape. Empty shows the museums' highlights.".into()),
                    default: None,
                },
            ],
            homepage: None,
        }
    }

    fn refresh(_ctx: &RefreshContext) -> Result<Vec<Item>, String> {
        let met_on = host::setting("met").map(|v| v != "false").unwrap_or(true);
        let cma_on = host::setting("cleveland").map(|v| v != "false").unwrap_or(true);
        let max = host::setting("max").and_then(|v| v.trim().parse::<usize>().ok()).unwrap_or(60).clamp(4, 200);
        let query = host::setting("query").map(|q| q.trim().to_string()).unwrap_or_default();
        if !met_on && !cma_on {
            return Err("Turn on at least one museum in the plugin's settings".into());
        }
        // An equal share each, so the museum that sends more per refresh cannot fill the walls.
        let share = GALLERY_CAP / (usize::from(met_on) + usize::from(cma_on));
        let mut gallery = Gallery::load().filter(|g| g.query == query).unwrap_or_else(|| Gallery { query: query.clone(), ..Default::default() });
        let mut problems = Vec::new();
        if met_on {
            match met_slice(&query, gallery.met_offset, &gallery) {
                Ok((found, next)) => {
                    gallery.met_offset = next;
                    gallery.hang(found, share);
                }
                Err(e) => problems.push(e),
            }
        }
        if cma_on {
            match cma_slice(&query, gallery.cma_skip) {
                Ok((found, next)) => {
                    gallery.cma_skip = next;
                    gallery.hang(found, share);
                }
                Err(e) => problems.push(e),
            }
        }
        if !met_on {
            gallery.paintings.retain(|p| p.museum != MET);
        }
        if !cma_on {
            gallery.paintings.retain(|p| p.museum != CMA);
        }
        for p in &problems {
            sdk::warn!("{p}");
        }
        if gallery.paintings.is_empty() {
            return Err(problems.into_iter().next().unwrap_or_else(|| format!("No paintings match \"{query}\". Try a painter's surname or a subject.")));
        }
        let shown = gallery.show(max);
        gallery.save();
        sdk::info!("{} paintings on the wall from a gallery of {}", shown.len(), gallery.paintings.len());
        Ok(shown.iter().enumerate().map(|(i, p)| p.to_item((shown.len() - i) as i32)).collect())
    }
}

sdk::export_plugin!(Art);

/// One painting the gallery knows: enough for the screensaver, nothing more.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Painting {
    pub id: String,
    pub title: String,
    /// "Vincent van Gogh, 1889, The Met".
    pub line: String,
    pub url: String,
    pub museum: String,
}

impl Painting {
    fn to_item(&self, weight: i32) -> Item {
        Item {
            id: self.id.clone(),
            kind: ItemKind::Wallpaper,
            title: self.title.clone(),
            subtitle: Some(self.line.clone()),
            thumbnail: Some(self.url.clone()),
            framed: true,
            weight,
            ..Default::default()
        }
    }
}

/// What has been collected so far, where to look next, and which paintings were shown last.
#[derive(Default, Serialize, Deserialize)]
pub struct Gallery {
    pub query: String,
    pub paintings: Vec<Painting>,
    pub met_offset: usize,
    pub cma_skip: usize,
    pub cursor: usize,
}

impl Gallery {
    fn load() -> Option<Gallery> {
        host::kv_get(GALLERY_KEY).and_then(|v| serde_json::from_slice(&v).ok())
    }

    fn save(&self) {
        if let Ok(bytes) = serde_json::to_vec(self) {
            host::kv_set(GALLERY_KEY, &bytes);
        }
    }

    pub fn knows(&self, id: &str) -> bool {
        self.paintings.iter().any(|p| p.id == id)
    }

    /// Adds what a museum sent, newest in front, and lets each museum's oldest go past `share`
    /// and the gallery's past the cap.
    pub fn hang(&mut self, found: Vec<Painting>, share: usize) {
        let mut fresh: Vec<Painting> = found.into_iter().filter(|p| !self.knows(&p.id)).collect();
        fresh.append(&mut self.paintings);
        let mut held: std::collections::HashMap<String, usize> = Default::default();
        fresh.retain(|p| {
            let n = held.entry(p.museum.clone()).or_default();
            *n += 1;
            *n <= share
        });
        fresh.truncate(GALLERY_CAP);
        self.paintings = fresh;
    }

    /// `max` paintings from the cursor on, wrapping round, the museums taking turns so the wall
    /// mixes them; the cursor moves a third of the way each time so the wall is never the same
    /// twice, nor entirely new.
    pub fn show(&mut self, max: usize) -> Vec<Painting> {
        let mixed = self.mixed();
        let n = mixed.len();
        if n == 0 {
            return Vec::new();
        }
        let start = self.cursor % n;
        let shown: Vec<Painting> = mixed.iter().cycle().skip(start).take(max.min(n)).map(|p| (*p).clone()).collect();
        self.cursor = (start + (max / 3).max(1)) % n;
        shown
    }

    /// The gallery with the museums interleaved, each in its own order.
    fn mixed(&self) -> Vec<&Painting> {
        let mut lists: Vec<Vec<&Painting>> = Vec::new();
        for p in &self.paintings {
            match lists.iter_mut().find(|l| l[0].museum == p.museum) {
                Some(l) => l.push(p),
                None => lists.push(vec![p]),
            }
        }
        let longest = lists.iter().map(Vec::len).max().unwrap_or(0);
        (0..longest).flat_map(|i| lists.iter().filter_map(move |l| l.get(i).copied())).collect()
    }
}

// ---------------------------------------------------------------------------
// The Met: a paginated search for ids, then one object each
// ---------------------------------------------------------------------------

#[derive(Deserialize, Default)]
struct MetSearch {
    #[serde(default)]
    total: usize,
    #[serde(rename = "objectIDs", default)]
    object_ids: Option<Vec<u64>>,
}

#[derive(Deserialize, Default)]
pub struct MetObject {
    #[serde(rename = "objectID", default)]
    pub object_id: u64,
    #[serde(default)]
    pub title: String,
    #[serde(rename = "artistDisplayName", default)]
    pub artist: String,
    #[serde(rename = "objectDate", default)]
    pub date: String,
    #[serde(rename = "primaryImageSmall", default)]
    pub image: String,
    #[serde(rename = "primaryImage", default)]
    pub original: String,
    #[serde(rename = "isPublicDomain", default)]
    pub public_domain: bool,
    #[serde(default)]
    pub classification: String,
    #[serde(rename = "objectName", default)]
    pub object_name: String,
}

/// A museum's answer at `url`. A failure is a sentence for the TV, and its cause goes to the log.
fn get_json<T: serde::de::DeserializeOwned>(museum: &str, url: &str) -> Result<T, String> {
    let r = sdk::fetch_online(museum, &HttpRequest::get(url))?;
    if !r.ok() {
        sdk::warn!("HTTP {} for {}", r.status, sdk::redact_url(url));
        return Err(sdk::online_error(museum, r.status));
    }
    serde_json::from_str(&r.body).map_err(|e| {
        sdk::warn!("unexpected answer from {museum}: {e}");
        sdk::unreadable(museum, &sdk::host_in_setting(url).unwrap_or_default())
    })
}

/// The next page of the Met's paintings as ids, each looked up, and the offset to use next time.
fn met_slice(query: &str, offset: usize, gallery: &Gallery) -> Result<(Vec<Painting>, usize), String> {
    let q = if query.is_empty() { "painting".to_string() } else { url_encode(query) };
    let highlights = if query.is_empty() { "&isHighlight=true" } else { "" };
    let url = format!("https://collectionapi.metmuseum.org/public/collection/v1.1/search?q={q}&hasImages=true{highlights}&limit={MET_PAGE}&offset={offset}");
    let search: MetSearch = get_json(MET, &url)?;
    let ids = search.object_ids.unwrap_or_default();
    let next = if search.total == 0 || offset + MET_PAGE >= search.total { 0 } else { offset + MET_PAGE };
    let mut found = Vec::new();
    let mut looked = 0;
    for id in ids {
        if looked >= MET_PER_REFRESH {
            break;
        }
        if gallery.knows(&format!("met:{id}")) {
            continue;
        }
        looked += 1;
        let body = match host::get_text(&format!("https://collectionapi.metmuseum.org/public/collection/v1/objects/{id}")) {
            Ok(b) => b,
            Err(e) if e.contains("budget") => break,
            Err(e) => {
                sdk::warn!("{MET} object {id}: {e}");
                continue;
            }
        };
        if let Some(p) = serde_json::from_str::<MetObject>(&body).ok().and_then(|o| met_painting(o, &mut met_original_fits)) {
            found.push(p);
        }
    }
    Ok((found, next))
}

/// Whether the original is small enough for the TV to prepare, from its first bytes: the whole
/// size from the range the server answers with, the pixels from the JPEG's frame header. The
/// small picture the Met offers otherwise is a few hundred pixels across.
fn met_original_fits(url: &str) -> bool {
    let Ok(r) = host::fetch(&HttpRequest::get(url).header("Range", "bytes=0-65535")) else { return false };
    if !(200..300).contains(&r.status) {
        return false;
    }
    let header = |name: &str| r.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.clone());
    let total = header("content-range").and_then(|v| v.rsplit('/').next()?.trim().parse::<u64>().ok()).or_else(|| header("content-length")?.trim().parse().ok());
    let size = jpeg_size(&sdk::body_bytes(r));
    match (total, size) {
        (Some(bytes), Some((w, h))) => bytes <= PICTURE_MAX_BYTES && u64::from(w) * u64::from(h) <= PICTURE_MAX_PIXELS,
        _ => false,
    }
}

/// A JPEG's width and height, from its frame header, if `bytes` reach that far.
pub fn jpeg_size(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.get(..2)? != [0xFF, 0xD8] {
        return None;
    }
    let mut i = 2;
    while i + 4 <= bytes.len() {
        if bytes[i] != 0xFF {
            return None;
        }
        let marker = bytes[i + 1];
        if marker == 0xFF {
            i += 1;
            continue;
        }
        // Frame headers are C0 to CF, apart from the tables and arithmetic coding markers.
        if (0xC0..=0xCF).contains(&marker) && !matches!(marker, 0xC4 | 0xC8 | 0xCC) {
            let at = |k: usize| bytes.get(i + k).map(|&b| u32::from(b));
            return Some((at(7)? << 8 | at(8)?, at(5)? << 8 | at(6)?));
        }
        i += 2 + (usize::from(bytes[i + 2]) << 8 | usize::from(bytes[i + 3]));
    }
    None
}

pub fn met_painting(o: MetObject, original_fits: &mut dyn FnMut(&str) -> bool) -> Option<Painting> {
    // The American Wing leaves the classification empty and names the object instead.
    let painting = o.classification.starts_with("Paintings") || (o.classification.is_empty() && o.object_name.eq_ignore_ascii_case("Painting"));
    if !o.public_domain || o.image.is_empty() || !painting || o.title.trim().is_empty() {
        return None;
    }
    let url = if !o.original.is_empty() && original_fits(&o.original) { o.original } else { o.image };
    Some(Painting { id: format!("met:{}", o.object_id), title: clip(o.title.trim(), 80), line: line(&o.artist, &o.date, MET), url, museum: MET.into() })
}

// ---------------------------------------------------------------------------
// Cleveland: one page of paintings with their pictures
// ---------------------------------------------------------------------------

#[derive(Deserialize, Default)]
struct CmaPage {
    #[serde(default)]
    data: Vec<CmaWork>,
    #[serde(default)]
    info: CmaInfo,
}

#[derive(Deserialize, Default)]
struct CmaInfo {
    #[serde(default)]
    total: usize,
}

#[derive(Deserialize, Default)]
pub struct CmaWork {
    #[serde(default)]
    pub id: u64,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub creation_date: Option<String>,
    #[serde(default)]
    pub creators: Vec<CmaCreator>,
    #[serde(default)]
    pub images: Option<CmaImages>,
}

#[derive(Deserialize, Default)]
pub struct CmaCreator {
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Deserialize, Default)]
pub struct CmaImages {
    #[serde(default)]
    pub web: Option<CmaImage>,
    #[serde(default)]
    pub print: Option<CmaImage>,
}

/// Cleveland gives sizes as strings.
#[derive(Deserialize, Default)]
pub struct CmaImage {
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub width: String,
    #[serde(default)]
    pub height: String,
    #[serde(default)]
    pub filesize: String,
}

impl CmaImage {
    /// A print-size picture the TV can prepare: inside its byte and pixel limits.
    fn fits(&self) -> bool {
        let n = |s: &str| s.trim().parse::<u64>().ok();
        match (n(&self.width), n(&self.height), n(&self.filesize)) {
            (Some(w), Some(h), Some(bytes)) => w * h <= PICTURE_MAX_PIXELS && bytes <= PICTURE_MAX_BYTES,
            _ => false,
        }
    }
}

fn cma_slice(query: &str, skip: usize) -> Result<(Vec<Painting>, usize), String> {
    let q = if query.is_empty() { String::new() } else { format!("&q={}", url_encode(query)) };
    let url = format!("https://openaccess-api.clevelandart.org/api/artworks/?type=Painting&has_image=1&cc0=1&limit={CMA_PAGE}&skip={skip}&fields=id,title,creators,creation_date,images{q}");
    let page: CmaPage = get_json(CMA, &url)?;
    let next = if page.info.total == 0 || skip + CMA_PAGE >= page.info.total { 0 } else { skip + CMA_PAGE };
    Ok((page.data.into_iter().filter_map(cma_painting).collect(), next))
}

pub fn cma_painting(w: CmaWork) -> Option<Painting> {
    let images = w.images?;
    let print = images.print.filter(|p| p.fits() && p.url.starts_with("https://"));
    let url = print.or(images.web).map(|i| i.url).filter(|u| u.starts_with("https://"))?;
    if w.title.trim().is_empty() {
        return None;
    }
    // "John Singleton Copley (American, 1738–1815)": the name is what goes under the clock.
    let artist = w.creators.first().and_then(|c| c.description.as_deref()).map(|d| d.split(" (").next().unwrap_or(d).trim().to_string()).unwrap_or_default();
    Some(Painting { id: format!("cma:{}", w.id), title: clip(w.title.trim(), 80), line: line(&artist, w.creation_date.as_deref().unwrap_or(""), CMA), url, museum: CMA.into() })
}

/// "Artist, date, Museum", leaving out what is not known.
pub fn line(artist: &str, date: &str, museum: &str) -> String {
    [artist.trim(), date.trim(), museum].iter().filter(|s| !s.is_empty()).copied().collect::<Vec<_>>().join(", ")
}

fn clip(s: &str, max: usize) -> String {
    if s.chars().count() <= max { s.to_string() } else { format!("{}…", s.chars().take(max - 1).collect::<String>().trim_end()) }
}

fn url_encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_met_object_becomes_a_painting() {
        let o: MetObject = serde_json::from_str(r#"{"objectID": 436535, "isHighlight": true, "isPublicDomain": true, "primaryImage": "https://images.metmuseum.org/CRDImages/ep/original/DP-42549-001.jpg",
            "primaryImageSmall": "https://images.metmuseum.org/CRDImages/ep/web-large/DP-42549-001.jpg", "department": "European Paintings", "objectName": "Painting",
            "title": "Wheat Field with Cypresses", "artistDisplayName": "Vincent van Gogh", "objectDate": "1889", "classification": "Paintings"}"#).unwrap();
        let mut small = |_: &str| false;
        let p = met_painting(o, &mut small).unwrap();
        assert_eq!(p.id, "met:436535");
        assert_eq!(p.line, "Vincent van Gogh, 1889, The Met");
        assert!(p.url.contains("web-large"));
        let o: MetObject = serde_json::from_str(r#"{"objectID": 436535, "isPublicDomain": true, "primaryImage": "https://images.metmuseum.org/CRDImages/ep/original/DP-42549-001.jpg",
            "primaryImageSmall": "https://images.metmuseum.org/CRDImages/ep/web-large/DP-42549-001.jpg", "title": "Wheat Field with Cypresses", "classification": "Paintings"}"#).unwrap();
        let mut large = |u: &str| u.contains("original");
        assert!(met_painting(o, &mut large).unwrap().url.contains("/original/"));
        let item = p.to_item(3);
        assert_eq!(item.kind, ItemKind::Wallpaper);
        assert!(item.framed);
        assert_eq!(item.subtitle.as_deref(), Some("Vincent van Gogh, 1889, The Met"));

        let drawing: MetObject = serde_json::from_str(r#"{"objectID": 1, "isPublicDomain": true, "primaryImageSmall": "https://images.metmuseum.org/x.jpg", "title": "Study", "classification": "Drawings"}"#).unwrap();
        assert!(met_painting(drawing, &mut small).is_none());
        let copyrighted: MetObject = serde_json::from_str(r#"{"objectID": 2, "isPublicDomain": false, "primaryImageSmall": "https://images.metmuseum.org/x.jpg", "title": "Late", "classification": "Paintings"}"#).unwrap();
        assert!(met_painting(copyrighted, &mut small).is_none());
        let anonymous: MetObject = serde_json::from_str(r#"{"objectID": 3, "isPublicDomain": true, "primaryImageSmall": "https://images.metmuseum.org/x.jpg", "title": "Saint", "objectDate": "ca. 1300", "classification": "Paintings"}"#).unwrap();
        assert_eq!(met_painting(anonymous, &mut small).unwrap().line, "ca. 1300, The Met");
    }

    #[test]
    fn a_cleveland_work_becomes_a_painting() {
        let page: CmaPage = serde_json::from_str(r#"{"info": {"total": 3907}, "data": [
            {"id": 94979, "title": "Nathaniel Hurd", "creation_date": "c. 1765",
             "images": {"web": {"url": "https://openaccess-cdn.clevelandart.org/1915.534/1915.534_web.jpg", "width": "748", "height": "893", "filesize": "402404"},
                        "print": {"url": "https://openaccess-cdn.clevelandart.org/1915.534/1915.534_print.jpg", "width": "2849", "height": "3400", "filesize": "5904910"}},
             "creators": [{"description": "John Singleton Copley (American, born The Thirteen Colonies, 1738–1815)", "role": "artist"}]},
            {"id": 2, "title": "Huge print", "images": {"web": {"url": "https://openaccess-cdn.clevelandart.org/w.jpg"}, "print": {"url": "https://openaccess-cdn.clevelandart.org/p.jpg", "width": "6000", "height": "4000", "filesize": "9000000"}}},
            {"id": 1, "title": "No picture", "creators": []}
        ]}"#).unwrap();
        assert_eq!(page.info.total, 3907);
        let paintings: Vec<Painting> = page.data.into_iter().filter_map(cma_painting).collect();
        assert_eq!(paintings.len(), 2);
        assert_eq!(paintings[0].id, "cma:94979");
        assert!(paintings[0].url.ends_with("_print.jpg"));
        assert_eq!(paintings[0].line, "John Singleton Copley, c. 1765, Cleveland Museum of Art");
        assert!(paintings[1].url.ends_with("/w.jpg"));
    }

    fn painting(id: &str) -> Painting {
        Painting { id: id.into(), title: id.into(), line: String::new(), url: format!("https://images.metmuseum.org/{id}.jpg"), museum: MET.into() }
    }

    #[test]
    fn the_wall_changes_a_little_each_time_and_the_gallery_keeps_its_cap() {
        let mut g = Gallery::default();
        g.hang((0..9).map(|i| painting(&format!("met:{i}"))).collect(), GALLERY_CAP);
        g.hang(vec![painting("met:0"), painting("met:9")], GALLERY_CAP);
        assert_eq!(g.paintings.len(), 10);
        assert_eq!(g.paintings[0].id, "met:9");
        let first: Vec<String> = g.show(6).into_iter().map(|p| p.id).collect();
        let second: Vec<String> = g.show(6).into_iter().map(|p| p.id).collect();
        assert_eq!(first.len(), 6);
        assert_ne!(first, second);
        assert_eq!(first[2..], second[..4]);
        assert_eq!(g.show(50).len(), 10);
        g.hang((0..400).map(|i| painting(&format!("cma:{i}"))).collect(), GALLERY_CAP);
        assert_eq!(g.paintings.len(), GALLERY_CAP);
    }

    #[test]
    fn the_museums_take_turns_on_the_wall() {
        let mut g = Gallery::default();
        g.hang((0..3).map(|i| painting(&format!("met:{i}"))).collect(), GALLERY_CAP / 2);
        g.hang((0..3).map(|i| Painting { museum: CMA.into(), ..painting(&format!("cma:{i}")) }).collect(), GALLERY_CAP / 2);
        let ids: Vec<String> = g.show(6).into_iter().map(|p| p.id).collect();
        assert_eq!(ids, ["cma:0", "met:0", "cma:1", "met:1", "cma:2", "met:2"]);
    }

    #[test]
    fn each_museum_keeps_its_share_of_the_gallery() {
        let mut g = Gallery::default();
        g.hang((0..20).map(|i| painting(&format!("met:{i}"))).collect(), GALLERY_CAP / 2);
        for round in 0..5 {
            g.hang((0..60).map(|i| Painting { museum: CMA.into(), ..painting(&format!("cma:{round}:{i}")) }).collect(), GALLERY_CAP / 2);
        }
        assert_eq!(g.paintings.iter().filter(|p| p.museum == MET).count(), 20);
        assert_eq!(g.paintings.iter().filter(|p| p.museum == CMA).count(), GALLERY_CAP / 2);
    }

    #[test]
    fn a_jpeg_tells_its_size_from_its_frame_header() {
        let mut jpeg = vec![0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x04, 0x4A, 0x46];
        jpeg.extend([0xFF, 0xC0, 0x00, 0x11, 0x08, 0x0D, 0x5D, 0x0F, 0xA0, 0x03]);
        assert_eq!(jpeg_size(&jpeg), Some((4000, 3421)));
        assert_eq!(jpeg_size(&jpeg[..12]), None);
        assert_eq!(jpeg_size(b"GIF89a"), None);
    }

    #[test]
    fn american_paintings_count_as_paintings() {
        let o = |classification: &str, name: &str| MetObject {
            object_id: 1,
            title: "The Gulf Stream".into(),
            image: "https://images.metmuseum.org/web-large/x.jpg".into(),
            public_domain: true,
            classification: classification.into(),
            object_name: name.into(),
            ..Default::default()
        };
        assert!(met_painting(o("", "Painting"), &mut |_| false).is_some());
        assert!(met_painting(o("", "Vase"), &mut |_| false).is_none());
        assert!(met_painting(o("Paintings", ""), &mut |_| false).is_some());
    }

    #[test]
    fn lines_leave_out_what_is_unknown() {
        assert_eq!(line("Monet", "", CMA), "Monet, Cleveland Museum of Art");
        assert_eq!(line("", "", MET), "The Met");
        assert_eq!(clip("a".repeat(100).as_str(), 10).chars().count(), 10);
    }
}
