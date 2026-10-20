//! Internet Archive: three rows of free, legally hosted video from archive.org collections,
//! and search across its films. A row lists a collection's best-rated items; selecting one
//! looks up its files and plays the MP4 in the TV's own player.
use lounge_plugin_sdk::{self as sdk, host, Action, Item, Kind, Manifest, RefreshContext, RowSpec, RowStyle, SettingKind, SettingSpec};
use serde::Deserialize;
use std::collections::HashSet;

pub struct Archive;

const ROWS: [(&str, &str, &str); 3] = [
    ("films", "Feature films", "feature_films"),
    ("cartoons", "Classic cartoons", "classic_cartoons"),
    ("tv", "Classic television", "classic_tv"),
];

impl sdk::Plugin for Archive {
    fn manifest() -> Manifest {
        Manifest {
            id: "tv.lounge.archive".into(),
            name: "Internet Archive".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            description: "Free films, cartoons and television from archive.org, played on the TV.".into(),
            publisher_name: "Lounge".into(),
            kinds: vec![Kind::Content, Kind::Search],
            fetch_hosts: vec!["archive.org".into(), "*.archive.org".into()],
            refresh_minutes: 24 * 60,
            rows: ROWS.iter().map(|(id, title, _)| RowSpec { id: (*id).into(), title: (*title).into(), style: RowStyle::Landscape }).collect(),
            settings: ROWS
                .iter()
                .map(|(id, title, collection)| SettingSpec {
                    key: format!("{id}_collection"),
                    label: format!("{title} collection"),
                    kind: SettingKind::Text,
                    required: false,
                    help: Some("An archive.org collection id, the last part of its address, such as feature_films.".into()),
                    default: Some((*collection).into()),
                })
                .chain(std::iter::once(SettingSpec { key: "per_row".into(), label: "Items per row".into(), kind: SettingKind::Number, required: false, help: None, default: Some("24".into()) }))
                .collect(),
            homepage: Some("https://archive.org".into()),
        }
    }

    fn refresh(_ctx: &RefreshContext) -> Result<Vec<Item>, String> {
        let per_row = host::setting("per_row").and_then(|v| v.trim().parse::<usize>().ok()).unwrap_or(24).clamp(4, 60);
        let mut items = Vec::new();
        let mut failures = Vec::new();
        let mut shown = HashSet::new();
        for (row, _, default) in ROWS {
            let collection = host::setting(&format!("{row}_collection")).map(|c| c.trim().to_string()).filter(|c| !c.is_empty()).unwrap_or_else(|| default.to_string());
            // A few extra, as an item already in an earlier row is left out of this one.
            let wanted = per_row + 8;
            let docs = match search(&collection_query(&collection, true), wanted) {
                Ok(d) => d,
                Err(e) => {
                    failures.push(format!("{collection}: {e}"));
                    continue;
                }
            };
            let docs = row_docs(docs, || search(&collection_query(&collection, false), wanted).unwrap_or_default(), &mut shown, per_row);
            let count = docs.len();
            items.extend(docs.into_iter().enumerate().map(|(i, d)| item_for(&d, Some(row), (count - i) as i32)));
        }
        if items.is_empty() {
            return Err(format!("archive.org gave nothing back. {}", failures.join("; ")));
        }
        for f in &failures {
            sdk::warn!("{f}");
        }
        sdk::info!("{} items", items.len());
        Ok(items)
    }

    fn search(query: &str, limit: u32) -> Result<Vec<Item>, String> {
        let q = format!("title:({}) AND mediatype:movies", quote(query));
        Ok(search(&q, limit.clamp(1, 40) as usize)?.iter().map(|d| item_for(d, None, 0)).collect())
    }

    fn resolve(action: &str) -> Result<Action, String> {
        let identifier = action.strip_prefix("play:").ok_or_else(|| format!("unknown action {action}"))?;
        let meta: Metadata = get_json(&format!("https://archive.org/metadata/{identifier}"))?;
        let file = best_video(&meta.files).ok_or_else(|| "This item has no playable video file".to_string())?;
        let title = meta.metadata.and_then(|m| m.title.map(|t| t.into_string())).unwrap_or_else(|| identifier.to_string());
        Ok(Action::play(format!("https://archive.org/download/{identifier}/{}", encode_path(&file.name)), title))
    }
}

sdk::export_plugin!(Archive);

#[derive(Deserialize, Default)]
struct SearchResponse {
    response: SearchBody,
}

#[derive(Deserialize, Default)]
struct SearchBody {
    #[serde(default)]
    docs: Vec<Doc>,
}

/// One search hit. archive.org returns most fields as a string or a list of strings.
#[derive(Deserialize, Default, Clone, Debug)]
pub struct Doc {
    #[serde(default)]
    pub identifier: String,
    #[serde(default)]
    pub title: Option<Text>,
    #[serde(default)]
    pub year: Option<Text>,
    #[serde(default)]
    pub runtime: Option<Text>,
    #[serde(default)]
    pub description: Option<Text>,
    #[serde(default)]
    pub creator: Option<Text>,
}

#[derive(Deserialize, Clone, Debug)]
#[serde(untagged)]
pub enum Text {
    One(String),
    Many(Vec<String>),
    Number(f64),
}

impl Text {
    pub fn into_string(self) -> String {
        match self {
            Text::One(s) => s,
            Text::Many(v) => v.into_iter().next().unwrap_or_default(),
            Text::Number(n) => format!("{}", n as i64),
        }
    }
}

#[derive(Deserialize, Default)]
struct Metadata {
    #[serde(default)]
    files: Vec<ArchiveFile>,
    #[serde(default)]
    metadata: Option<ItemMeta>,
}

#[derive(Deserialize, Default)]
struct ItemMeta {
    #[serde(default)]
    title: Option<Text>,
}

#[derive(Deserialize, Default, Clone, Debug)]
pub struct ArchiveFile {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub format: String,
    #[serde(default)]
    pub size: Option<Text>,
}

/// A collection's films, leaving out what a living room row should not surprise anyone with.
fn collection_query(collection: &str, well_rated: bool) -> String {
    let base = format!("collection:{} AND mediatype:movies AND NOT (subject:({EXCLUDED}) OR title:({EXCLUDED}))", quote(collection));
    if well_rated { format!("{base} AND avg_rating:[4 TO 5] AND num_reviews:[3 TO 100000]") } else { base }
}

const EXCLUDED: &str = "sex OR adult OR erotic OR erotica OR nudist OR nudism OR nudity OR sexploitation";

fn search(query: &str, rows: usize) -> Result<Vec<Doc>, String> {
    let url = format!(
        "https://archive.org/advancedsearch.php?q={}&fl[]=identifier&fl[]=title&fl[]=year&fl[]=runtime&fl[]=description&fl[]=creator&sort[]=downloads+desc&rows={rows}&output=json",
        encode_query(query)
    );
    let r: SearchResponse = get_json(&url)?;
    Ok(unique_docs(r.response.docs))
}

/// A row's docs: the well-rated ones, topped up from `more` when there are fewer than eight,
/// leaving out anything an earlier row already shows.
fn row_docs(mut docs: Vec<Doc>, more: impl FnOnce() -> Vec<Doc>, shown: &mut HashSet<String>, per_row: usize) -> Vec<Doc> {
    docs.retain(|d| !shown.contains(&d.identifier));
    if docs.len() < 8 {
        for d in more() {
            if !shown.contains(&d.identifier) && !docs.iter().any(|x| x.identifier == d.identifier) {
                docs.push(d);
            }
        }
    }
    docs.truncate(per_row);
    shown.extend(docs.iter().map(|d| d.identifier.clone()));
    docs
}

/// Docs with an identifier, each identifier once.
fn unique_docs(docs: Vec<Doc>) -> Vec<Doc> {
    let mut seen = HashSet::new();
    docs.into_iter().filter(|d| !d.identifier.is_empty() && seen.insert(d.identifier.clone())).collect()
}

fn get_json<T: serde::de::DeserializeOwned>(url: &str) -> Result<T, String> {
    let body = host::get_text(url)?;
    serde_json::from_str(&body).map_err(|e| format!("unexpected answer from archive.org: {e}"))
}

fn item_for(d: &Doc, row: Option<&str>, weight: i32) -> Item {
    let title = d.title.clone().map(Text::into_string).unwrap_or_else(|| d.identifier.clone());
    let year = d.year.clone().map(Text::into_string).filter(|y| !y.is_empty());
    let creator = d.creator.clone().map(Text::into_string).filter(|c| !c.is_empty() && !c.eq_ignore_ascii_case(&title));
    let blurb = || d.description.clone().map(|t| first_sentence(&t.into_string())).filter(|s| !s.is_empty() && !s.eq_ignore_ascii_case(&title));
    let subtitle = match (year, creator) {
        (Some(y), Some(c)) => Some(format!("{y}, {c}")),
        (Some(y), None) => Some(y),
        (None, Some(c)) => Some(c),
        (None, None) => blurb(),
    }
    .filter(|s| !s.eq_ignore_ascii_case(&title))
    .or_else(blurb);
    Item {
        id: format!("item:{}", d.identifier),
        title,
        subtitle,
        duration_ms: d.runtime.clone().and_then(|r| parse_runtime(&r.into_string())),
        thumbnail: Some(format!("https://archive.org/services/img/{}", d.identifier)),
        row: row.map(Into::into),
        weight,
        action: Action { resolve: Some(format!("play:{}", d.identifier)), ..Default::default() },
        ..Default::default()
    }
}

/// The best file to play: an MP4 first, then whatever the player stands a chance with.
pub fn best_video(files: &[ArchiveFile]) -> Option<&ArchiveFile> {
    const ORDER: [&str; 6] = ["h.264", "h.264 HD", "MPEG4", "HiRes MPEG4", "512Kb MPEG4", "Ogg Video"];
    for format in ORDER {
        if let Some(f) = files.iter().filter(|f| f.format == format).max_by_key(|f| f.size.clone().map(|s| s.into_string().parse::<i64>().unwrap_or(0)).unwrap_or(0)) {
            return Some(f);
        }
    }
    files.iter().find(|f| f.name.to_ascii_lowercase().ends_with(".mp4"))
}

/// "1:31:44", "72 min." or "20:33" → milliseconds.
pub fn parse_runtime(s: &str) -> Option<i64> {
    let s = s.trim();
    if let Some(min) = s.strip_suffix("min.").or_else(|| s.strip_suffix("min")).or_else(|| s.strip_suffix("minutes")) {
        return min.trim().parse::<i64>().ok().map(|m| m * 60_000);
    }
    let parts: Vec<i64> = s.split(':').map(|p| p.trim().parse::<i64>().ok()).collect::<Option<Vec<_>>>()?;
    let seconds = match parts.as_slice() {
        [h, m, sec] => h * 3600 + m * 60 + sec,
        [m, sec] => m * 60 + sec,
        _ => return None,
    };
    Some(seconds * 1000)
}

fn first_sentence(s: &str) -> String {
    let s = s.replace('\n', " ");
    let end = s.find(". ").map(|i| i + 1).unwrap_or(s.len()).min(80);
    let cut = s.char_indices().take_while(|(i, _)| *i < end).map(|(i, c)| i + c.len_utf8()).last().unwrap_or(0);
    s[..cut].trim().to_string()
}

fn quote(s: &str) -> String {
    let cleaned: String = s.chars().filter(|c| !matches!(c, '"' | '(' | ')' | ':' | '[' | ']')).collect();
    if cleaned.contains(' ') { format!("\"{}\"", cleaned.trim()) } else { cleaned }
}

fn encode_query(s: &str) -> String {
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

fn encode_path(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'/' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtimes_parse() {
        assert_eq!(parse_runtime("1:31:44"), Some(5_504_000));
        assert_eq!(parse_runtime("20:33"), Some(1_233_000));
        assert_eq!(parse_runtime("72 min."), Some(4_320_000));
        assert_eq!(parse_runtime("soon"), None);
    }

    #[test]
    fn the_mp4_wins() {
        let files: Vec<ArchiveFile> = serde_json::from_str(r#"[
            {"name": "x.ogv", "format": "Ogg Video", "size": "390213679"},
            {"name": "x.mpeg", "format": "MPEG2", "size": "3292094468"},
            {"name": "x.mp4", "format": "h.264", "size": "575274912"},
            {"name": "__ia_thumb.jpg", "format": "Item Tile", "size": "12290"}
        ]"#).unwrap();
        assert_eq!(best_video(&files).unwrap().name, "x.mp4");
    }

    #[test]
    fn search_docs_read_mixed_field_shapes() {
        let r: SearchResponse = serde_json::from_str(r#"{"response": {"docs": [
            {"identifier": "his_girl_friday", "title": "His Girl Friday", "year": "1940", "runtime": "1:31:44", "description": ["Hilarious. Really."]},
            {"identifier": "two", "title": ["Two"], "year": 1925, "creator": "two"}
        ]}}"#).unwrap();
        let a = item_for(&r.response.docs[0], Some("films"), 2);
        assert_eq!(a.title, "His Girl Friday");
        assert_eq!(a.subtitle.as_deref(), Some("1940"));
        assert_eq!(a.duration_ms, Some(5_504_000));
        assert_eq!(a.action.resolve.as_deref(), Some("play:his_girl_friday"));
        let b = item_for(&r.response.docs[1], None, 0);
        assert_eq!(b.subtitle.as_deref(), Some("1925"));
    }

    #[test]
    fn an_identifier_appears_once() {
        let r: SearchResponse = serde_json::from_str(r#"{"response": {"docs": [
            {"identifier": "a"}, {"identifier": ""}, {"identifier": "b"}, {"identifier": "a"}
        ]}}"#).unwrap();
        let ids: Vec<String> = unique_docs(r.response.docs).into_iter().map(|d| d.identifier).collect();
        assert_eq!(ids, ["a", "b"]);
    }

    #[test]
    fn an_item_in_two_rows_shows_in_the_first() {
        let doc = |id: &str| Doc { identifier: id.into(), ..Default::default() };
        let mut shown = HashSet::new();
        let films = row_docs(vec![doc("a"), doc("b"), doc("c")], Vec::new, &mut shown, 24);
        let tv = row_docs(vec![doc("b"), doc("d")], || vec![doc("a"), doc("e"), doc("d")], &mut shown, 24);
        let ids = |docs: &[Doc]| docs.iter().map(|d| d.identifier.clone()).collect::<Vec<_>>();
        assert_eq!(ids(&films), ["a", "b", "c"]);
        assert_eq!(ids(&tv), ["d", "e"]);
        let items: Vec<Item> = films.iter().chain(&tv).map(|d| item_for(d, Some("films"), 0)).collect();
        assert_eq!(items.iter().map(|i| &i.id).collect::<HashSet<_>>().len(), 5);
    }

    #[test]
    fn queries_are_quoted_and_encoded() {
        let q = collection_query("feature_films", true);
        assert!(q.starts_with("collection:feature_films AND mediatype:movies AND NOT (subject:(sex OR "));
        assert!(q.ends_with(") AND avg_rating:[4 TO 5] AND num_reviews:[3 TO 100000]"));
        assert_eq!(quote("night of the living dead"), "\"night of the living dead\"");
        assert_eq!(encode_query("a b:c"), "a+b%3Ac");
        assert_eq!(encode_path("his girl friday.mp4"), "his%20girl%20friday.mp4");
    }
}
