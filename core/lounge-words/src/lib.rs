//! How Lounge's Rust code speaks the person's language. Text is written in English where it is
//! used, inside [`t!`], [`tc!`] or [`tn!`], and looked up in the catalog of the language in use.
//! Text with no translation stays English. docs/TRANSLATING.md has the rules for writing it.

mod plural;
mod po;

use std::borrow::Cow;
use std::cell::RefCell;
use std::fmt::Display;
use std::sync::{Arc, RwLock};

pub use plural::forms;
pub use po::{Entry, parse};

/// The language text is written in, and the one used when nothing else fits.
pub const SOURCE: &str = "en";

/// Catalogs a crate ships: a language tag and the text of its PO file.
pub type Catalogs = &'static [(&'static str, &'static str)];

/// Separates a context from its text in a catalog key, as gettext does.
const CONTEXT_END: char = '\u{4}';

/// The translations of one language, from one or more catalogs, earlier catalogs first.
#[derive(Debug, Default)]
pub struct Words {
    tag: String,
    entries: Vec<(Cow<'static, str>, Vec<Cow<'static, str>>)>,
}

impl Words {
    /// Reads `sources` as the catalogs of `tag`; where two translate the same text, the first wins.
    pub fn read(tag: &str, sources: &[&'static str]) -> Words {
        let mut entries = Vec::new();
        for source in sources {
            for entry in parse(source) {
                if entry.fuzzy || entry.id.is_empty() || entry.forms.iter().any(|form| form.is_empty()) {
                    continue;
                }
                let key = match entry.context {
                    Some(context) => Cow::Owned(key(Some(&context), &entry.id).into_owned()),
                    None => entry.id,
                };
                entries.push((key, entry.forms));
            }
        }
        Words { tag: tag.to_string(), entries }
    }

    pub fn tag(&self) -> &str {
        &self.tag
    }

    fn form(&self, context: Option<&str>, text: &str, index: usize) -> Option<&str> {
        // A catalog holds a few hundred texts and a call asks for a few of them, so looking
        // along the list costs less than putting it in order first. The first match wins.
        let key = key(context, text);
        let forms = &self.entries.iter().find(|(have, _)| have.len() == key.len() && have.as_ref() == key.as_ref())?.1;
        forms.get(index).or(forms.last()).map(|form| form.as_ref())
    }
}

fn key<'a>(context: Option<&str>, text: &'a str) -> Cow<'a, str> {
    match context {
        Some(context) => Cow::Owned(format!("{context}{CONTEXT_END}{text}")),
        None => Cow::Borrowed(text),
    }
}

static IN_USE: RwLock<Option<Arc<Words>>> = RwLock::new(None);

thread_local! {
    /// The language of [`in_language`] on this thread, which comes before the one in use.
    static HERE: RefCell<Option<Option<Arc<Words>>>> = const { RefCell::new(None) };
}

fn in_use() -> Option<Arc<Words>> {
    if let Some(here) = HERE.with(|h| h.borrow().clone()) {
        return here;
    }
    IN_USE.read().unwrap_or_else(|e| e.into_inner()).clone()
}

/// The tag among `have` that serves someone who asked for `wanted`: the tag itself, then its
/// language without a region, then another region of the same language. Portuguese outside the
/// Americas reads as Portugal's.
pub fn best<'a>(wanted: &str, have: impl IntoIterator<Item = &'a str>) -> Option<&'a str> {
    let wanted = wanted.replace('_', "-").to_ascii_lowercase();
    let (language, region) = match wanted.split_once('-') {
        Some((language, rest)) => (language, rest.rsplit('-').next().unwrap_or("")),
        None => (wanted.as_str(), ""),
    };
    let have: Vec<&str> = have.into_iter().collect();
    let find = |tag: &str| have.iter().copied().find(|h| h.eq_ignore_ascii_case(tag));
    if let Some(exact) = find(&wanted) {
        return Some(exact);
    }
    if language == "pt" {
        let european = !region.is_empty() && !matches!(region, "br" | "us" | "419");
        if let Some(found) = find(if european { "pt-PT" } else { "pt-BR" }) {
            return Some(found);
        }
    }
    find(language).or_else(|| have.iter().copied().find(|h| h.split('-').next().is_some_and(|l| l.eq_ignore_ascii_case(language))))
}

/// Puts `wanted` in use for the whole process, from the catalogs in `shipped`, earlier ones
/// first. Returns the tag now in use, which is [`SOURCE`] when no catalog fits.
pub fn speak(wanted: &str, shipped: &[Catalogs]) -> String {
    let tag = shipped_tag(wanted, shipped).unwrap_or(SOURCE);
    let now = IN_USE.read().unwrap_or_else(|e| e.into_inner()).as_ref().map_or(SOURCE.to_string(), |w| w.tag.clone());
    if now != tag {
        *IN_USE.write().unwrap_or_else(|e| e.into_inner()) = words_for(wanted, shipped).map(Arc::new);
    }
    tag.to_string()
}

fn shipped_tag(wanted: &str, shipped: &[Catalogs]) -> Option<&'static str> {
    best(wanted, shipped.iter().flat_map(|catalogs| catalogs.iter().map(|(tag, _)| *tag)))
}

fn words_for(wanted: &str, shipped: &[Catalogs]) -> Option<Words> {
    let tag = shipped_tag(wanted, shipped)?;
    let sources: Vec<&str> = shipped.iter().flat_map(|catalogs| catalogs.iter().filter(|(t, _)| *t == tag).map(|(_, text)| *text)).collect();
    Some(Words::read(tag, &sources))
}

/// The tag of the language in use.
pub fn language() -> String {
    in_use().map_or(SOURCE.to_string(), |w| w.tag.clone())
}

/// Runs `f` with `wanted` in use on this thread only, leaving every other thread as it was.
pub fn in_language<R>(wanted: &str, shipped: &[Catalogs], f: impl FnOnce() -> R) -> R {
    let before = HERE.with(|h| h.replace(Some(words_for(wanted, shipped).map(Arc::new))));
    let result = f();
    HERE.with(|h| *h.borrow_mut() = before);
    result
}

/// `text` in the language in use, or as written when it has no translation. `context` tells
/// apart two uses of the same English words that other languages say differently.
pub fn translate(context: Option<&str>, text: &str) -> String {
    match in_use() {
        Some(words) => words.form(context, text, 0).unwrap_or(text).to_string(),
        None => text.to_string(),
    }
}

/// [`translate`] for text held in a table or a constant, which [`mark!`] names for translators.
pub fn tr(text: &str) -> String {
    translate(None, text)
}

/// The wording for `n` things: `one` or `other` in English, or the form the language in use has
/// for that number.
pub fn translate_count(context: Option<&str>, n: i64, one: &str, other: &str) -> String {
    let english = if n == 1 { one } else { other };
    match in_use() {
        Some(words) => words.form(context, one, forms(&words.tag, n)).unwrap_or(english).to_string(),
        None => english.to_string(),
    }
}

/// `template` with each `{name}` replaced by its value. A name with no value stays as written.
pub fn fill(template: &str, values: &[(&str, &dyn Display)]) -> String {
    let mut out = String::with_capacity(template.len() + 16);
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        let named = after.find('}').and_then(|close| values.iter().find(|(name, _)| *name == &after[..close]).map(|(_, value)| (close, value)));
        match named {
            Some((close, value)) => {
                out.push_str(&value.to_string());
                rest = &after[close + 1..];
            }
            None => {
                out.push('{');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// `items` as a list in a sentence: "A", "A and B", "A, B and C".
pub fn list<S: AsRef<str>>(items: &[S]) -> String {
    match items {
        [] => String::new(),
        [one] => one.as_ref().to_string(),
        [rest @ .., last] => {
            let mut said = rest[0].as_ref().to_string();
            for item in &rest[1..] {
                said = t!("{first}, {next}", first = said, next = item.as_ref());
            }
            t!("{first} and {last}", first = said, last = last.as_ref())
        }
    }
}

/// `value` with `places` decimals, with the decimal mark of the language in use.
pub fn decimal(value: f64, places: usize) -> String {
    let plain = format!("{value:.places$}");
    let mark = tc!("decimal mark", ".");
    if mark == "." { plain } else { plain.replace('.', &mark) }
}

/// Text in the language in use. Values fill the `{name}` places: `t!("Added to {source} today", source = name)`.
#[macro_export]
macro_rules! t {
    ($text:literal) => { $crate::translate(None, $text) };
    ($text:literal, $($name:ident = $value:expr),+ $(,)?) => {
        $crate::fill(&$crate::translate(None, $text), &[$((stringify!($name), &$value as &dyn ::core::fmt::Display)),+])
    };
}

/// [`t!`] with a context first, for English words that mean two things: `tc!("games", "Play")`.
#[macro_export]
macro_rules! tc {
    ($context:literal, $text:literal) => { $crate::translate(Some($context), $text) };
    ($context:literal, $text:literal, $($name:ident = $value:expr),+ $(,)?) => {
        $crate::fill(&$crate::translate(Some($context), $text), &[$((stringify!($name), &$value as &dyn ::core::fmt::Display)),+])
    };
}

/// Text that counts something, with `{n}` for the number: `tn!(count, "{n} app", "{n} apps")`.
#[macro_export]
macro_rules! tn {
    ($n:expr, $one:literal, $other:literal $(, $name:ident = $value:expr)* $(,)?) => {{
        let n = ($n) as i64;
        $crate::fill(&$crate::translate_count(None, n, $one, $other), &[("n", &n as &dyn ::core::fmt::Display) $(, (stringify!($name), &$value as &dyn ::core::fmt::Display))*])
    }};
}

/// Names text for translators and leaves it as written, for a table or a constant whose text
/// is passed to [`tr`] when it is shown.
#[macro_export]
macro_rules! mark {
    ($text:literal) => { $text };
}

#[cfg(test)]
mod tests {
    use super::*;

    const GERMAN: &str = r#"
msgid ""
msgstr "Language: de\n"

msgid "Open"
msgstr "Öffnen"

msgctxt "games"
msgid "Play"
msgstr "Spielen"

msgid "Play"
msgstr "Abspielen"

msgid "Added to {source} today"
msgstr "Heute zu {source} hinzugefügt"

msgid "{n} app"
msgid_plural "{n} apps"
msgstr[0] "{n} App"
msgstr[1] "{n} Apps"

msgid "{first} and {last}"
msgstr "{first} und {last}"

msgctxt "decimal mark"
msgid "."
msgstr ","

#, fuzzy
msgid "Hide"
msgstr "Verstecken"

msgid "Watch"
msgstr ""
"#;
    const SHIPPED: Catalogs = &[("de", GERMAN), ("pt-BR", ""), ("pt-PT", "")];

    #[test]
    fn text_is_said_in_the_language_in_use_and_in_english_otherwise() {
        in_language("en", &[SHIPPED], || {
            assert_eq!((t!("Open"), tn!(1, "{n} app", "{n} apps"), tn!(3, "{n} app", "{n} apps")), ("Open".to_string(), "1 app".to_string(), "3 apps".to_string()));
            assert_eq!((language(), decimal(7.25, 1)), ("en".to_string(), "7.2".to_string()));
        });
        in_language("de-AT", &[SHIPPED], || {
            assert_eq!(language(), "de");
            assert_eq!((t!("Open"), t!("Play"), tc!("games", "Play")), ("Öffnen".to_string(), "Abspielen".to_string(), "Spielen".to_string()));
            assert_eq!(t!("Added to {source} today", source = "Plex"), "Heute zu Plex hinzugefügt");
            assert_eq!((tn!(1, "{n} app", "{n} apps"), tn!(0, "{n} app", "{n} apps")), ("1 App".to_string(), "0 Apps".to_string()));
            assert_eq!((list(&["A"]), list(&["A", "B"]), list(&["A", "B", "C"])), ("A".to_string(), "A und B".to_string(), "A, B und C".to_string()));
            assert_eq!(decimal(7.25, 1), "7,2");
        });
    }

    #[test]
    fn text_with_no_finished_translation_stays_english() {
        in_language("de", &[SHIPPED], || {
            assert_eq!((t!("Hide"), t!("Watch"), t!("Never translated")), ("Hide".to_string(), "Watch".to_string(), "Never translated".to_string()));
            assert_eq!(tn!(2, "{n} row", "{n} rows"), "2 rows");
        });
        in_language("ja", &[SHIPPED], || assert_eq!((language(), t!("Open")), ("en".to_string(), "Open".to_string())));
    }

    #[test]
    fn a_place_with_no_value_stays_as_written() {
        assert_eq!(fill("{a} and {b}, {a} {", &[("a", &1)]), "1 and {b}, 1 {");
        assert_eq!(fill("as in {\"key\": 1}", &[("key", &2)]), "as in {\"key\": 1}");
    }

    #[test]
    fn the_nearest_language_is_chosen() {
        let have = ["de", "es", "pt-BR", "pt-PT"];
        let chosen = |wanted| best(wanted, have);
        assert_eq!((chosen("de"), chosen("de-CH"), chosen("DE_de"), chosen("es-MX")), (Some("de"), Some("de"), Some("de"), Some("es")));
        assert_eq!((chosen("pt"), chosen("pt-BR"), chosen("pt-PT"), chosen("pt-AO"), chosen("pt-US")), (Some("pt-BR"), Some("pt-BR"), Some("pt-PT"), Some("pt-PT"), Some("pt-BR")));
        assert_eq!((chosen("pl"), chosen("en-GB"), chosen("")), (None, None, None));
        assert_eq!(best("es-AR", ["es-ES"]), Some("es-ES"));
    }

    #[test]
    fn speaking_changes_every_thread_until_told_otherwise() {
        assert_eq!(speak("de", &[SHIPPED]), "de");
        assert_eq!(std::thread::spawn(|| t!("Open")).join().unwrap(), "Öffnen");
        // A language set for one thread wins there, and English can be had that way too.
        in_language("en", &[SHIPPED], || assert_eq!(t!("Open"), "Open"));
        assert_eq!(speak("xx", &[SHIPPED]), "en");
    }
}
