//! Deterministic, explainable ordering.

use crate::{DeepLink, Item, ItemKind};
use std::collections::HashMap;

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

/// One item for each normalised title (continue-watching from several sources), in the place of
/// the first. Where titles collide, the one whose link reaches the title most directly wins: a
/// Watch Next entry beside a playing session that only opens its app, or a server's resume point
/// beside a Trakt entry that only searches for the title.
pub fn dedupe_by_title(items: &mut Vec<&Item>) {
    let mut place: HashMap<String, usize> = HashMap::new();
    let mut kept: Vec<&Item> = Vec::with_capacity(items.len());
    for it in items.drain(..) {
        match place.get(&normalise(&it.title)) {
            Some(&i) => {
                if reach(it) > reach(kept[i]) {
                    kept[i] = it;
                }
            }
            None => {
                place.insert(normalise(&it.title), kept.len());
                kept.push(it);
            }
        }
    }
    *items = kept;
}

/// How directly an item's link reaches its title: only the app, a search for the title, or the
/// title itself.
fn reach(it: &Item) -> u8 {
    match &it.deep_link {
        DeepLink::LaunchApp { .. } => 0,
        DeepLink::Control(c) if c.starts_with("search:") => 1,
        DeepLink::ViewUri(u) if u.contains("action=android.search.action.GLOBAL_SEARCH") => 1,
        _ => 2,
    }
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

/// Why an item is in the spotlight, in the product's voice. `source_name` is what to call the
/// item's source, such as Jellyfin; `habit` counts the app's sittings in each local hour.
pub fn explanation(item: &Item, now_ms: i64, hour_local: u32, source_name: &str, habit: Option<&[u32; 24]>) -> Option<String> {
    if item.kind == ItemKind::MediaSession {
        return item.last_used_ms.map(|used| format!("You played this {}", when_word(now_ms - used)));
    }
    if item.kind == ItemKind::Resume {
        return Some("You were watching this".to_string());
    }
    if let Some(added) = item.added_ms {
        let age = now_ms - added;
        if (0..86_400_000).contains(&age) && !matches!(item.source.as_str(), "apps" | "tvprovider" | "sessions" | "inputs") {
            return Some(format!("Added to {source_name} today"));
        }
    }
    if item.progress.is_some() {
        return Some("You were watching this".to_string());
    }
    if let Some(hour) = habit.and_then(usual_hour) {
        let apart = (hour as i32 - hour_local as i32).rem_euclid(24);
        if apart <= 1 || apart >= 23 {
            let verb = if item.flags.is_game || item.kind == ItemKind::Game { "play" } else { "watch" };
            return Some(format!("Because you {verb} at {}", clock_word(hour)));
        }
    }
    None
}

/// The hour of day an app is usually opened, when there is a real pattern: at least three
/// sittings in that hour and two in five of all sittings.
pub fn usual_hour(habit: &[u32; 24]) -> Option<u32> {
    let total: u32 = habit.iter().sum();
    let (hour, &count) = habit.iter().enumerate().max_by_key(|(h, c)| (**c, std::cmp::Reverse(*h)))?;
    (count >= 3 && count * 5 >= total * 2).then_some(hour as u32)
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
    fn of_two_with_one_title_the_one_that_resumes_stays() {
        let mut session = it("The Bear", Some(9));
        session.id = "session".into();
        session.deep_link = DeepLink::LaunchApp { package: "tv.app".into(), activity: String::new() };
        let mut watch_next = it("The Bear", Some(5));
        watch_next.id = "wn".into();
        watch_next.deep_link = DeepLink::ViewUri("app://bear/s2e4".into());
        let other = it("Alien", Some(7));
        let mut v = vec![&session, &other, &watch_next];
        dedupe_by_title(&mut v);
        let ids: Vec<&str> = v.iter().map(|i| i.id.as_str()).collect();
        assert_eq!(ids, vec!["wn", "app:Alien"]);
    }

    #[test]
    fn a_resume_point_beats_a_search_for_the_same_title() {
        let mut trakt = it("Severance", Some(9));
        trakt.id = "trakt".into();
        trakt.deep_link = DeepLink::Control("search:Severance".into());
        let mut server = it("Severance", Some(5));
        server.id = "jellyfin".into();
        server.deep_link = DeepLink::ViewOrLaunch { uri: "intent:#Intent;end".into(), package: "org.jellyfin.androidtv".into() };
        let mut session = it("Severance", Some(3));
        session.id = "session".into();
        session.deep_link = DeepLink::LaunchApp { package: "tv.app".into(), activity: String::new() };
        let mut v = vec![&trakt, &server, &session];
        dedupe_by_title(&mut v);
        assert_eq!(v.iter().map(|i| i.id.as_str()).collect::<Vec<_>>(), vec!["jellyfin"]);

        let mut v = vec![&session, &trakt];
        dedupe_by_title(&mut v);
        assert_eq!(v.iter().map(|i| i.id.as_str()).collect::<Vec<_>>(), vec!["trakt"]);
    }

    #[test]
    fn habits_need_a_real_pattern() {
        let mut h = [0u32; 24];
        h[21] = 2;
        assert_eq!(usual_hour(&h), None);
        h[21] = 4;
        h[9] = 2;
        assert_eq!(usual_hour(&h), Some(21));
        h[13] = 9;
        assert_eq!(usual_hour(&h), Some(13));
        let even = [1u32; 24];
        assert_eq!(usual_hour(&even), None);
    }

    #[test]
    fn explanations_say_only_what_is_known() {
        let mut app = it("Netflix", Some(1_000));
        let mut habit = [0u32; 24];
        habit[21] = 5;
        assert_eq!(explanation(&app, 2_000, 21, "Apps", Some(&habit)).as_deref(), Some("Because you watch at 9 pm"));
        assert_eq!(explanation(&app, 2_000, 3, "Apps", Some(&habit)), None);
        assert_eq!(explanation(&app, 2_000, 21, "Apps", None), None);
        app.source = "plugin:tv.lounge.jellyfin".into();
        app.added_ms = Some(1_500);
        assert_eq!(explanation(&app, 2_000, 21, "Jellyfin", None).as_deref(), Some("Added to Jellyfin today"));
    }

    #[test]
    fn clock_words() {
        assert_eq!(clock_word(21), "9 pm");
        assert_eq!(clock_word(9), "9 am");
        assert_eq!(clock_word(12), "noon");
    }
}
