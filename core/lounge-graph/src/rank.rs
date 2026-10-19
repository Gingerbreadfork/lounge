//! Deterministic, explainable ordering.

use crate::{Item, ItemKind};
use std::collections::HashSet;

/// Most recently used first; items never used sort after used ones, alphabetically.
pub fn by_recency_then_title(items: &mut [&Item]) {
    items.sort_by(|a, b| {
        match (b.last_used_ms, a.last_used_ms) {
            (Some(x), Some(y)) if x != y => x.cmp(&y),
            (Some(_), None) => std::cmp::Ordering::Greater,
            (None, Some(_)) => std::cmp::Ordering::Less,
            _ => a.title.to_lowercase().cmp(&b.title.to_lowercase()),
        }
    });
}

/// Keep the first item for each normalised title (continue-watching from several sources).
pub fn dedupe_by_title(items: &mut Vec<&Item>) {
    let mut seen: HashSet<String> = HashSet::new();
    items.retain(|it| seen.insert(normalise(&it.title)));
}

fn normalise(title: &str) -> String {
    title
        .chars()
        .filter(|c| c.is_alphanumeric() || c.is_whitespace())
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// Why an item is in the spotlight, in the product's voice.
pub fn explanation(item: &Item, now_ms: i64, hour_local: u32) -> Option<String> {
    if item.kind == ItemKind::MediaSession {
        return item.last_used_ms.map(|used| format!("You played this {}", when_word(now_ms - used)));
    }
    if item.kind == ItemKind::Resume {
        return Some("You were watching this".to_string());
    }
    if let Some(added) = item.added_ms {
        let age = now_ms - added;
        if (0..86_400_000).contains(&age) && !matches!(item.source.as_str(), "apps" | "tvprovider" | "sessions" | "inputs") {
            return Some(format!("Added to {} today", source_label(&item.source)));
        }
    }
    if item.progress.is_some() {
        return Some("You were watching this".to_string());
    }
    if let Some(used) = item.last_used_ms {
        let age = now_ms - used;
        if age < 3 * 86_400_000 {
            return Some(format!("You usually open this around {}", clock_word(hour_local)));
        }
    }
    None
}

fn source_label(source: &str) -> &str {
    match source {
        "jellyfin" => "Jellyfin",
        "plex" => "Plex",
        "emby" => "Emby",
        other => other,
    }
}

fn when_word(age_ms: i64) -> &'static str {
    match age_ms / 60_000 {
        m if m < 60 => "just now",
        m if m < 24 * 60 => "today",
        m if m < 48 * 60 => "yesterday",
        _ => "recently",
    }
}

fn clock_word(hour: u32) -> String {
    match hour {
        0 => "midnight".to_string(),
        12 => "noon".to_string(),
        h if h < 12 => format!("{h} am"),
        h => format!("{} pm", h - 12),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Item;

    fn it(title: &str, used: Option<i64>) -> Item {
        let mut i = Item::app(title, "M", title);
        i.last_used_ms = used;
        i
    }

    #[test]
    fn recency_then_alpha() {
        let a = it("Zeta", None);
        let b = it("Alpha", None);
        let c = it("Mid", Some(5));
        let d = it("Top", Some(9));
        let mut v = vec![&a, &b, &c, &d];
        by_recency_then_title(&mut v);
        let titles: Vec<&str> = v.iter().map(|i| i.title.as_str()).collect();
        assert_eq!(titles, vec!["Top", "Mid", "Alpha", "Zeta"]);
    }

    #[test]
    fn dedupe_ignores_punctuation_and_case() {
        let a = it("The Bear", None);
        let mut b = it("the bear!", None);
        b.id = "other".into();
        let mut v = vec![&a, &b];
        dedupe_by_title(&mut v);
        assert_eq!(v.len(), 1);
    }

    #[test]
    fn clock_words() {
        assert_eq!(clock_word(21), "9 pm");
        assert_eq!(clock_word(9), "9 am");
        assert_eq!(clock_word(12), "noon");
    }
}
