//! Abstract screensavers that plugins publish: shader programs checked here and kept per plugin
//! in the store, so the screensaver has them in a new process before any plugin refreshes.
use crate::{ShaderScreensaver, ShaderUniform};
use lounge_plugin_api::{Item, ItemKind, Uniform};
use lounge_store::Store;

const MAX_PER_PLUGIN: usize = 8;
const MAX_SOURCE_BYTES: usize = 32 * 1024;
const MAX_UNIFORMS: usize = 24;
const MAX_FLOATS: usize = 16;

/// Set by Lounge on every program that declares them; a plugin's value for one is ignored.
const PROVIDED: [&str; 10] = ["resolution", "time", "hour", "velvet", "horizon", "curtain", "houselight", "ember", "signal", "dust"];

fn meta_key(plugin: &str) -> String {
    format!("screensavers:{plugin}")
}

/// The screensavers among a plugin's items, and a sentence for each one left out.
pub(crate) fn from_items<'a>(plugin: &str, plugin_name: &str, items: impl IntoIterator<Item = &'a Item>) -> (Vec<ShaderScreensaver>, Vec<String>) {
    let mut kept = Vec::new();
    let mut problems = Vec::new();
    for it in items.into_iter().filter(|i| i.kind == ItemKind::Screensaver) {
        match check(plugin, plugin_name, it) {
            Ok(_) if kept.len() >= MAX_PER_PLUGIN => problems.push(format!("screensaver {}: only {MAX_PER_PLUGIN} are kept", it.id)),
            Ok(s) => kept.push(s),
            Err(e) => problems.push(format!("screensaver {}: {e}", it.id)),
        }
    }
    (kept, problems)
}

fn check(plugin: &str, plugin_name: &str, it: &Item) -> Result<ShaderScreensaver, String> {
    let shader = it.shader.as_ref().ok_or("it has no shader")?;
    if it.id.is_empty() || it.id.len() > 64 || !it.id.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.')) {
        return Err("its id must be letters, digits, '-', '_' or '.'".into());
    }
    let name = it.title.trim();
    if name.is_empty() {
        return Err("it has no title".into());
    }
    if shader.source.trim().is_empty() {
        return Err("its program is empty".into());
    }
    if shader.source.len() > MAX_SOURCE_BYTES {
        return Err(format!("its program is larger than {} KB", MAX_SOURCE_BYTES / 1024));
    }
    if shader.uniforms.len() > MAX_UNIFORMS {
        return Err(format!("it sets more than {MAX_UNIFORMS} uniforms"));
    }
    let mut uniforms = Vec::new();
    for (name, value) in &shader.uniforms {
        if !is_identifier(name) {
            return Err(format!("{name} is not a uniform name"));
        }
        if PROVIDED.contains(&name.as_str()) {
            continue;
        }
        let (floats, color_argb) = match value {
            Uniform::Number(n) => (vec![*n], None),
            Uniform::Numbers(v) => (v.clone(), None),
            Uniform::Color(c) => (Vec::new(), Some(parse_color(c).ok_or_else(|| format!("{c} is not a colour like #FFA86B"))?)),
        };
        if color_argb.is_none() && (floats.is_empty() || floats.len() > MAX_FLOATS || floats.iter().any(|f| !f.is_finite())) {
            return Err(format!("{name} needs one to {MAX_FLOATS} finite numbers"));
        }
        uniforms.push(ShaderUniform { name: name.clone(), floats, color_argb });
    }
    Ok(ShaderScreensaver {
        id: format!("plugin:{plugin}:{}", it.id),
        name: clip(name, 40),
        description: it.subtitle.as_deref().map(str::trim).filter(|s| !s.is_empty()).map(|s| clip(s, 120)),
        source: shader.source.clone(),
        plugin_name: plugin_name.to_string(),
        uniforms,
    })
}

fn is_identifier(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 32
        && s.chars().next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// "#RRGGBB" or "#AARRGGBB" as ARGB.
pub(crate) fn parse_color(text: &str) -> Option<u32> {
    let hex = text.trim().strip_prefix('#')?;
    let v = u32::from_str_radix(hex, 16).ok()?;
    match hex.len() {
        6 => Some(0xFF00_0000 | v),
        8 => Some(v),
        _ => None,
    }
}

fn clip(s: &str, max: usize) -> String {
    s.chars().take(max).collect()
}

/// Keeps a plugin's screensavers; an empty list replaces any it published before.
pub(crate) fn save(store: &Store, plugin: &str, list: &[ShaderScreensaver]) {
    let key = meta_key(plugin);
    if list.is_empty() && store.meta(&key).ok().flatten().is_none_or(|v| v == "[]") {
        return;
    }
    let json = serde_json::to_string(list).unwrap_or_else(|_| "[]".to_string());
    if let Err(e) = store.set_meta(&key, &json) {
        tracing::warn!("screensavers for {plugin}: {e}");
    }
}

pub(crate) fn load(store: &Store, plugin: &str) -> Vec<ShaderScreensaver> {
    store.meta(&meta_key(plugin)).ok().flatten().and_then(|json| serde_json::from_str(&json).ok()).unwrap_or_default()
}

pub(crate) fn clear(store: &Store, plugin: &str) {
    save(store, plugin, &[]);
}

#[cfg(test)]
mod tests {
    use super::*;
    use lounge_plugin_api::Shader;

    fn item(id: &str, shader: Option<Shader>) -> Item {
        Item { id: id.into(), kind: ItemKind::Screensaver, title: "Smoke".into(), subtitle: Some("In the projector's beam".into()), shader, ..Default::default() }
    }

    #[test]
    fn screensavers_are_checked_and_kept() {
        let good = Shader::new("half4 main(float2 p) { return half4(1); }").float("speed", 0.5).floats("drift", &[0.1, 0.2]).color("smoke", "#C8B8A0").float("time", 9.0);
        let items = vec![
            item("smoke", Some(good)),
            item("bare", None),
            item("bad colour", Some(Shader::new("x").color("ink", "blue"))),
            Item { id: "film".into(), title: "A film".into(), ..Default::default() },
        ];
        let (kept, problems) = from_items("tv.lounge.sample", "Sample library", &items);
        assert_eq!(kept.len(), 1);
        let s = &kept[0];
        assert_eq!(s.id, "plugin:tv.lounge.sample:smoke");
        assert_eq!(s.plugin_name, "Sample library");
        assert_eq!(s.description.as_deref(), Some("In the projector's beam"));
        let names: Vec<&str> = s.uniforms.iter().map(|u| u.name.as_str()).collect();
        assert_eq!(names, vec!["drift", "smoke", "speed"]);
        assert_eq!(s.uniforms[1].color_argb, Some(0xFFC8_B8A0));
        assert_eq!(problems.len(), 2);
        assert!(problems[0].contains("no shader"));
        assert!(problems[1].contains("id must be"));
    }

    #[test]
    fn values_must_be_sound() {
        let (_, problems) = from_items("p", "P", &[item("a", Some(Shader::new("x").floats("v", &[])))]);
        assert!(problems[0].contains("finite numbers"));
        let (_, problems) = from_items("p", "P", &[item("a", Some(Shader::new("x").float("1st", 1.0)))]);
        assert!(problems[0].contains("not a uniform name"));
        let (_, problems) = from_items("p", "P", &[item("a", Some(Shader::new("x").color("c", "#12345")))]);
        assert!(problems[0].contains("not a colour"));
    }

    #[test]
    fn colours_parse() {
        assert_eq!(parse_color("#FFA86B"), Some(0xFFFF_A86B));
        assert_eq!(parse_color("#80FFA86B"), Some(0x80FF_A86B));
        assert_eq!(parse_color("FFA86B"), None);
    }

    #[test]
    fn they_survive_in_the_store() {
        let store = Store::open_in_memory().unwrap();
        assert!(load(&store, "p").is_empty());
        let (kept, _) = from_items("p", "P", &[item("a", Some(Shader::new("half4 main(float2 p) { return half4(1); }")))]);
        save(&store, "p", &kept);
        assert_eq!(load(&store, "p"), kept);
        clear(&store, "p");
        assert!(load(&store, "p").is_empty());
    }
}
