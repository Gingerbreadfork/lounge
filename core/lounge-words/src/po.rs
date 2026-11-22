//! Reads gettext PO catalogs: the entries, their contexts, plural forms and the fuzzy flag.

use std::borrow::Cow;

/// One translated text. `forms` holds one translation, or one per plural form. Text is borrowed
/// from the catalog where it stands there whole, which is nearly always.
#[derive(Debug, Default, PartialEq)]
pub struct Entry<'a> {
    pub context: Option<Cow<'a, str>>,
    pub id: Cow<'a, str>,
    pub id_plural: Option<Cow<'a, str>>,
    pub forms: Vec<Cow<'a, str>>,
    /// Marked as needing a translator's eye, so not used.
    pub fuzzy: bool,
}

enum Field {
    Context,
    Id,
    IdPlural,
    Form(usize),
}

/// The entries of the catalog `text`, the header included as the entry with an empty id.
///
/// Each byte is looked at once, since a plugin reads its catalog on every call and the sandbox
/// runs a loop over bytes slowly.
pub fn parse(text: &str) -> Vec<Entry<'_>> {
    let bytes = text.as_bytes();
    let mut entries = Vec::new();
    let mut entry = Entry::default();
    let mut field: Option<Field> = None;
    let mut started = false;
    let mut at = 0;
    while at <= bytes.len() {
        match bytes.get(at).copied() {
            None | Some(b'\n') | Some(b'\r') => {
                if started {
                    entries.push(std::mem::take(&mut entry));
                }
                entry = Entry::default();
                (field, started) = (None, false);
                at += 1;
                continue;
            }
            Some(b'#') => {
                let end = line_end(bytes, at);
                if bytes.get(at + 1) == Some(&b',') && text[at..end].contains("fuzzy") {
                    entry.fuzzy = true;
                }
                at = end + 1;
                continue;
            }
            Some(b'"') => {}
            Some(_) => {
                let keyword_end = at + bytes[at..].iter().position(|&b| b == b' ' || b == b'\n').unwrap_or(bytes.len() - at);
                field = match &text[at..keyword_end] {
                    "msgctxt" => Some(Field::Context),
                    "msgid" => Some(Field::Id),
                    "msgid_plural" => Some(Field::IdPlural),
                    "msgstr" => Some(Field::Form(0)),
                    keyword => keyword.strip_prefix("msgstr[").and_then(|k| k.strip_suffix(']')).and_then(|k| k.parse().ok()).map(Field::Form),
                };
                started |= field.is_some();
                at = keyword_end;
                while bytes.get(at) == Some(&b' ') {
                    at += 1;
                }
                if bytes.get(at) != Some(&b'"') {
                    at = line_end(bytes, at) + 1;
                    continue;
                }
            }
        }
        // A quoted string, read to its closing quote; it is borrowed unless it holds an escape.
        let open = at + 1;
        let mut close = open;
        let mut escaped = false;
        while let Some(&b) = bytes.get(close) {
            match b {
                b'\\' => {
                    escaped = true;
                    close += 2;
                }
                b'"' | b'\n' => break,
                _ => close += 1,
            }
        }
        let close = close.min(bytes.len());
        let inner = &text[open..close];
        add(&mut entry, &field, if escaped { Cow::Owned(unescape(inner)) } else { Cow::Borrowed(inner) });
        at = line_end(bytes, close) + 1;
    }
    // A catalog that ends without a blank line still ends its last entry.
    if started {
        entries.push(entry);
    }
    entries
}

/// Where the line holding `at` ends: its line feed, or the end of the text.
fn line_end(bytes: &[u8], at: usize) -> usize {
    bytes.get(at..).and_then(|rest| rest.iter().position(|&b| b == b'\n')).map_or(bytes.len(), |end| at + end)
}

/// Adds a quoted piece to the field of `entry` being read.
fn add<'a>(entry: &mut Entry<'a>, field: &Option<Field>, piece: Cow<'a, str>) {
    match field {
        Some(Field::Context) => join(entry.context.get_or_insert_default(), piece),
        Some(Field::Id) => join(&mut entry.id, piece),
        Some(Field::IdPlural) => join(entry.id_plural.get_or_insert_default(), piece),
        Some(Field::Form(index)) => {
            if entry.forms.len() <= *index {
                entry.forms.resize(index + 1, Cow::Borrowed(""));
            }
            join(&mut entry.forms[*index], piece);
        }
        None => {}
    }
}

/// Adds `piece` to `text`, which stays borrowed while it is one piece.
fn join<'a>(text: &mut Cow<'a, str>, piece: Cow<'a, str>) {
    if text.is_empty() {
        *text = piece;
    } else if !piece.is_empty() {
        text.to_mut().push_str(&piece);
    }
}

/// `inner`, the text between a PO string's quotes, with its escapes read.
fn unescape(inner: &str) -> String {
    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some(other) => out.push(other),
            None => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_endings_and_spacing_do_not_change_what_is_read() {
        let unix = "msgid \"Open\"\nmsgstr \"Öffnen\"\n\nmsgid \"Hide\"\nmsgstr \"\"\n\"Aus\"\n\"blenden\"\n";
        let windows = unix.replace('\n', "\r\n");
        let spaced = unix.replace("msgstr ", "msgstr   ");
        for text in [unix, &windows, &spaced] {
            let entries = parse(text);
            let read: Vec<(&str, &str)> = entries.iter().map(|e| (e.id.as_ref(), e.forms[0].as_ref())).collect();
            assert_eq!(read, [("Open", "Öffnen"), ("Hide", "Ausblenden")], "{text:?}");
        }
    }

    #[test]
    fn a_catalog_is_read_entry_by_entry() {
        let entries = parse(
            "# A translator's note\nmsgid \"\"\nmsgstr \"\"\n\"Language: de\\n\"\n\n#. Where it shows\n#: src/lib.rs:4\nmsgctxt \"games\"\nmsgid \"Play\"\nmsgstr \"Spielen\"\n\n\n#, fuzzy\nmsgid \"A \\\"long\\\" \"\n\"line\\\\\"\nmsgstr \"\"\n\"Eine lange \"\n\"Zeile\"\n\nmsgid \"{n} app\"\nmsgid_plural \"{n} apps\"\nmsgstr[0] \"{n} App\"\nmsgstr[1] \"{n} Apps\"",
        );
        assert_eq!(entries.len(), 4);
        assert_eq!((entries[0].id.as_ref(), entries[0].forms[0].as_ref()), ("", "Language: de\n"));
        assert_eq!(entries[1], Entry { context: Some("games".into()), id: "Play".into(), id_plural: None, forms: vec!["Spielen".into()], fuzzy: false });
        assert_eq!((entries[2].id.as_ref(), entries[2].forms[0].as_ref(), entries[2].fuzzy), ("A \"long\" line\\", "Eine lange Zeile", true));
        assert_eq!((entries[3].id_plural.as_deref(), entries[3].forms.len()), (Some("{n} apps"), 2));
        // Text that stands whole in the catalog is not copied.
        assert!(matches!(entries[1].forms[0], Cow::Borrowed(_)) && matches!(entries[2].forms[0], Cow::Owned(_)));
    }
}
