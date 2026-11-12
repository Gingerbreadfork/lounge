//! Plugin search for the TV's search screen: the search plugins are asked on the thread that
//! answers presses, their answers become rows of tiles, and each tile remembers what it opens.

use crate::plugins::{channel_key, PluginJob, ReadyItem, SearchGroup};
use crate::worker::{plugin_link, ArtPlan};
use crate::{Badge, Inner, Row, RowSize, Tile, TileKind};
use lounge_artwork::Style;
use lounge_graph::DeepLink;
use lounge_plugin_api::ItemKind as PluginItemKind;
use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::time::Duration;

/// Longest the screen waits for every plugin to answer.
const WAIT: Duration = Duration::from_secs(20);

pub(crate) fn run(inner: &Inner, query: &str) -> Vec<Row> {
    if query.chars().count() < 2 {
        return Vec::new();
    }
    let generation = inner.plugins.search_generation.fetch_add(1, Ordering::SeqCst) + 1;
    let (reply, answers) = crossbeam_channel::bounded(1);
    let job = PluginJob::Search { query: query.to_string(), generation, reply };
    if inner.plugins.call(job).is_err() {
        return Vec::new();
    }
    let groups = answers.recv_timeout(WAIT).unwrap_or_default();
    if inner.plugins.search_generation.load(Ordering::SeqCst) != generation {
        return Vec::new();
    }
    rows_for(inner, groups)
}

fn rows_for(inner: &Inner, groups: Vec<SearchGroup>) -> Vec<Row> {
    let mut plan = ArtPlan::new();
    let mut wants = Vec::new();
    for g in &groups {
        let kind = row_kind(&g.items);
        let picks: Vec<Option<usize>> = g
            .items
            .iter()
            .map(|r| match kind {
                TileKind::Poster => plan.want(r.poster.as_deref().or(r.thumbnail.as_deref()), Style::Poster),
                TileKind::Compact => plan.want(r.logo.as_deref(), Style::Logo),
                _ => plan.want(r.thumbnail.as_deref().or(r.poster.as_deref()), Style::Landscape),
            })
            .collect();
        wants.push((kind, picks));
    }
    let art = plan.run(&inner.state, 48).results;

    let mut links: HashMap<String, DeepLink> = HashMap::new();
    let mut rows = Vec::new();
    {
        let state = inner.state.lock();
        for (g, (kind, picks)) in groups.iter().zip(wants) {
            let mut tiles = Vec::new();
            for (r, pick) in g.items.iter().zip(picks) {
                let it = &r.item;
                let link = match plugin_link(&g.plugin, it) {
                    // A result that only opens search again is no result.
                    Some(DeepLink::Control(c)) if c.starts_with("search:") => None,
                    // A programme with no action of its own plays through its channel.
                    Some(DeepLink::None) => it
                        .channel_id
                        .as_deref()
                        .and_then(|c| state.graph.channel_for(channel_key(&g.plugin, c)))
                        .map(|ch| ch.deep_link.clone()),
                    other => other,
                };
                let Some(link) = link else { continue };
                let id = format!("search:{}:{}", g.plugin, it.id);
                let prepared = pick.and_then(|i| art[i].as_ref());
                tiles.push(Tile {
                    id: id.clone(),
                    kind,
                    title: it.title.clone(),
                    subtitle: it.subtitle.clone().or_else(|| it.channel_name.clone()),
                    artwork_path: prepared.filter(|_| kind != TileKind::Compact).map(|p| p.path.clone()),
                    icon_path: prepared.filter(|_| kind == TileKind::Compact).map(|p| p.path.clone()),
                    light_argb: prepared.map(|p| lounge_graph::light_from_palette(&p.palette)).unwrap_or(lounge_graph::color::HOUSELIGHT),
                    progress: it.progress,
                    badge: (it.kind == PluginItemKind::Channel).then_some(Badge::Live),
                    action: format!("open:{id}"),
                    pinned: false,
                    stage_pinned: false,
                    borrowed_art: false,
                    art_title: None,
                    opened_rank: None,
                    installed_rank: None,
                    provider: None,
                    provider_name: None,
                    hidden: false,
                    time_left: None,
                });
                links.insert(id, link);
            }
            if !tiles.is_empty() {
                rows.push(Row { id: format!("search:plugin:{}", g.plugin), title: g.plugin_name.clone(), kind, size: RowSize::M, badge: None, tiles, note: None });
            }
        }
    }
    *inner.search_links.lock() = links;
    rows
}

/// Posters when most results have one, compact tiles when most have only a logo, else landscape.
fn row_kind(items: &[ReadyItem]) -> TileKind {
    let most = |n: usize| n * 2 > items.len();
    if most(items.iter().filter(|r| r.poster.is_some()).count()) {
        TileKind::Poster
    } else if most(items.iter().filter(|r| r.poster.is_none() && r.thumbnail.is_none() && r.logo.is_some()).count()) {
        TileKind::Compact
    } else {
        TileKind::Landscape
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn result(poster: bool, thumbnail: bool, logo: bool) -> ReadyItem {
        let pic = |has: bool, name: &str| has.then(|| format!("/art/{name}.png"));
        ReadyItem { item: Default::default(), poster: pic(poster, "p"), thumbnail: pic(thumbnail, "t"), logo: pic(logo, "l") }
    }

    #[test]
    fn a_row_takes_the_shape_most_of_its_results_have_pictures_for() {
        assert_eq!(row_kind(&[result(true, false, false), result(true, true, false), result(false, true, false)]), TileKind::Poster);
        assert_eq!(row_kind(&[result(false, false, true), result(false, false, true), result(false, true, false)]), TileKind::Compact, "stations with only their logo");
        assert_eq!(row_kind(&[result(false, true, true), result(false, true, true)]), TileKind::Landscape, "a logo beside a thumbnail is not the picture");
        assert_eq!(row_kind(&[result(false, false, false)]), TileKind::Landscape);
    }
}
