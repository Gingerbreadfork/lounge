//! Art mode: public-domain paintings from the Met and the Cleveland Museum of Art become the
//! screensaver, each hung whole with the painter, the year and the museum under the clock. Every
//! refresh brings a few more into the gallery, and the wall shows a different selection each time.
use lounge_plugin_sdk::{self as sdk, host, Item, ItemKind, Kind, Manifest, RefreshContext, SettingKind, SettingSpec};
use serde::{Deserialize, Serialize};

pub struct Art;

const GALLERY_KEY: &str = "gallery";
/// Paintings remembered between refreshes; the oldest go when the gallery is full.
const GALLERY_CAP: usize = 300;
/// Met objects looked up per refresh: one fetch each, inside the host's budget with room to spare.
const MET_PER_REFRESH: usize = 36;
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
        let mut gallery = Gallery::load().filter(|g| g.query == query).unwrap_or_else(|| Gallery { query: query.clone(), ..Default::default() });
        let mut problems = Vec::new();
        if met_on {
            match met_slice(&query, gallery.met_offset, &gallery) {
                Ok((found, next)) => {
                    gallery.met_offset = next;
                    gallery.hang(found);
                }
                Err(e) => problems.push(format!("{MET}: {e}")),
            }
        }
        if cma_on {
            match cma_slice(&query, gallery.cma_skip) {
                Ok((found, next)) => {
                    gallery.cma_skip = next;
                    gallery.hang(found);
                }
                Err(e) => problems.push(format!("{CMA}: {e}")),
            }
        }
        if !met_on {
            gallery.paintings.retain(|p| p.museum != MET);
        }
        if !cma_on {
            gallery.paintings.retain(|p| p.museum != CMA);
        }
        if gallery.paintings.is_empty() {
            return Err(if problems.is_empty() {
                format!("No paintings match \"{query}\". Try a painter's surname or a subject.")
            } else {
                problems.join("; ")
            });
        }
        for p in &problems {
            sdk::warn!("{p}");
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

    /// Adds what a museum sent, newest in front, and lets the oldest go past the cap.
    pub fn hang(&mut self, found: Vec<Painting>) {
        let mut fresh: Vec<Painting> = found.into_iter().filter(|p| !self.knows(&p.id)).collect();
        fresh.append(&mut self.paintings);
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
    #[serde(rename = "isPublicDomain", default)]
    pub public_domain: bool,
    #[serde(default)]
    pub classification: String,
}

/// The next page of the Met's paintings as ids, each looked up, and the offset to use next time.
fn met_slice(query: &str, offset: usize, gallery: &Gallery) -> Result<(Vec<Painting>, usize), String> {
    let q = if query.is_empty() { "painting".to_string() } else { url_encode(query) };
    let highlights = if query.is_empty() { "&isHighlight=true" } else { "" };
    let url = format!("https://collectionapi.metmuseum.org/public/collection/v1.1/search?q={q}&hasImages=true{highlights}&limit={MET_PAGE}&offset={offset}");
    let search: MetSearch = serde_json::from_str(&host::get_text(&url)?).map_err(|e| format!("unexpected search answer: {e}"))?;
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
        if let Some(p) = serde_json::from_str::<MetObject>(&body).ok().and_then(met_painting) {
            found.push(p);
        }
    }
    Ok((found, next))
}

pub fn met_painting(o: MetObject) -> Option<Painting> {
    if !o.public_domain || o.image.is_empty() || !o.classification.starts_with("Paintings") || o.title.trim().is_empty() {
        return None;
    }
    Some(Painting { id: format!("met:{}", o.object_id), title: clip(o.title.trim(), 80), line: line(&o.artist, &o.date, MET), url: o.image, museum: MET.into() })
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
}

#[derive(Deserialize, Default)]
pub struct CmaImage {
    #[serde(default)]
    pub url: String,
}

fn cma_slice(query: &str, skip: usize) -> Result<(Vec<Painting>, usize), String> {
    let q = if query.is_empty() { String::new() } else { format!("&q={}", url_encode(query)) };
    let url = format!("https://openaccess-api.clevelandart.org/api/artworks/?type=Painting&has_image=1&cc0=1&limit={CMA_PAGE}&skip={skip}&fields=id,title,creators,creation_date,images{q}");
    let page: CmaPage = serde_json::from_str(&host::get_text(&url)?).map_err(|e| format!("unexpected answer: {e}"))?;
    let next = if page.info.total == 0 || skip + CMA_PAGE >= page.info.total { 0 } else { skip + CMA_PAGE };
    Ok((page.data.into_iter().filter_map(cma_painting).collect(), next))
}

pub fn cma_painting(w: CmaWork) -> Option<Painting> {
    let url = w.images.and_then(|i| i.web).map(|i| i.url).filter(|u| u.starts_with("https://"))?;
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
        let p = met_painting(o).unwrap();
        assert_eq!(p.id, "met:436535");
        assert_eq!(p.line, "Vincent van Gogh, 1889, The Met");
        assert!(p.url.contains("web-large"));
        let item = p.to_item(3);
        assert_eq!(item.kind, ItemKind::Wallpaper);
        assert!(item.framed);
        assert_eq!(item.subtitle.as_deref(), Some("Vincent van Gogh, 1889, The Met"));

        let drawing: MetObject = serde_json::from_str(r#"{"objectID": 1, "isPublicDomain": true, "primaryImageSmall": "https://images.metmuseum.org/x.jpg", "title": "Study", "classification": "Drawings"}"#).unwrap();
        assert!(met_painting(drawing).is_none());
        let copyrighted: MetObject = serde_json::from_str(r#"{"objectID": 2, "isPublicDomain": false, "primaryImageSmall": "https://images.metmuseum.org/x.jpg", "title": "Late", "classification": "Paintings"}"#).unwrap();
        assert!(met_painting(copyrighted).is_none());
        let anonymous: MetObject = serde_json::from_str(r#"{"objectID": 3, "isPublicDomain": true, "primaryImageSmall": "https://images.metmuseum.org/x.jpg", "title": "Saint", "objectDate": "ca. 1300", "classification": "Paintings"}"#).unwrap();
        assert_eq!(met_painting(anonymous).unwrap().line, "ca. 1300, The Met");
    }

    #[test]
    fn a_cleveland_work_becomes_a_painting() {
        let page: CmaPage = serde_json::from_str(r#"{"info": {"total": 3907}, "data": [
            {"id": 94979, "title": "Nathaniel Hurd", "creation_date": "c. 1765",
             "images": {"web": {"url": "https://openaccess-cdn.clevelandart.org/1915.534/1915.534_web.jpg", "width": "748", "height": "893"}},
             "creators": [{"description": "John Singleton Copley (American, born The Thirteen Colonies, 1738–1815)", "role": "artist"}]},
            {"id": 1, "title": "No picture", "creators": []}
        ]}"#).unwrap();
        assert_eq!(page.info.total, 3907);
        let paintings: Vec<Painting> = page.data.into_iter().filter_map(cma_painting).collect();
        assert_eq!(paintings.len(), 1);
        assert_eq!(paintings[0].id, "cma:94979");
        assert_eq!(paintings[0].line, "John Singleton Copley, c. 1765, Cleveland Museum of Art");
    }

    fn painting(id: &str) -> Painting {
        Painting { id: id.into(), title: id.into(), line: String::new(), url: format!("https://images.metmuseum.org/{id}.jpg"), museum: MET.into() }
    }

    #[test]
    fn the_wall_changes_a_little_each_time_and_the_gallery_keeps_its_cap() {
        let mut g = Gallery::default();
        g.hang((0..9).map(|i| painting(&format!("met:{i}"))).collect());
        g.hang(vec![painting("met:0"), painting("met:9")]);
        assert_eq!(g.paintings.len(), 10);
        assert_eq!(g.paintings[0].id, "met:9");
        let first: Vec<String> = g.show(6).into_iter().map(|p| p.id).collect();
        let second: Vec<String> = g.show(6).into_iter().map(|p| p.id).collect();
        assert_eq!(first.len(), 6);
        assert_ne!(first, second);
        assert_eq!(first[2..], second[..4]);
        assert_eq!(g.show(50).len(), 10);
        g.hang((0..400).map(|i| painting(&format!("cma:{i}"))).collect());
        assert_eq!(g.paintings.len(), GALLERY_CAP);
    }

    #[test]
    fn the_museums_take_turns_on_the_wall() {
        let mut g = Gallery::default();
        g.hang((0..3).map(|i| painting(&format!("met:{i}"))).collect());
        g.hang((0..3).map(|i| Painting { museum: CMA.into(), ..painting(&format!("cma:{i}")) }).collect());
        let ids: Vec<String> = g.show(6).into_iter().map(|p| p.id).collect();
        assert_eq!(ids, ["cma:0", "met:0", "cma:1", "met:1", "cma:2", "met:2"]);
    }

    #[test]
    fn lines_leave_out_what_is_unknown() {
        assert_eq!(line("Monet", "", CMA), "Monet, Cleveland Museum of Art");
        assert_eq!(line("", "", MET), "The Met");
        assert_eq!(clip("a".repeat(100).as_str(), 10).chars().count(), 10);
    }
}
