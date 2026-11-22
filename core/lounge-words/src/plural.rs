//! Which of a language's plural forms a number takes, in the order gettext catalogs list them.

/// The index of the form `tag`'s language uses for `n` things. A language not listed here has
/// one form for exactly one and another for everything else.
pub fn forms(tag: &str, n: i64) -> usize {
    let n = n.unsigned_abs();
    let (tens, hundreds) = (n % 10, n % 100);
    let lower = tag.to_ascii_lowercase();
    let language = lower.split(['-', '_']).next().unwrap_or("");
    match language {
        "ja" | "ko" | "zh" | "vi" | "th" | "id" | "ms" | "ka" | "km" | "lo" | "my" => 0,
        "fr" | "oc" | "fil" | "tl" | "hy" | "ln" | "mg" | "wa" => (n > 1) as usize,
        "pt" if matches!(lower.as_str(), "pt" | "pt-br" | "pt_br") => (n > 1) as usize,
        "ru" | "uk" | "be" | "sr" | "hr" | "bs" => match (tens, hundreds) {
            (1, h) if h != 11 => 0,
            (2..=4, h) if !(12..=14).contains(&h) => 1,
            _ => 2,
        },
        "pl" => match (n, tens, hundreds) {
            (1, _, _) => 0,
            (_, 2..=4, h) if !(12..=14).contains(&h) => 1,
            _ => 2,
        },
        "cs" | "sk" => match n {
            1 => 0,
            2..=4 => 1,
            _ => 2,
        },
        "ro" => match (n, hundreds) {
            (1, _) => 0,
            (0, _) | (_, 1..=19) => 1,
            _ => 2,
        },
        "lt" => match (tens, hundreds) {
            (1, h) if h != 11 => 0,
            (2..=9, h) if !(11..=19).contains(&h) => 1,
            _ => 2,
        },
        "lv" => match (n, tens, hundreds) {
            (0, _, _) => 2,
            (_, 1, h) if h != 11 => 0,
            _ => 1,
        },
        "sl" => match hundreds {
            1 => 0,
            2 => 1,
            3 | 4 => 2,
            _ => 3,
        },
        "ar" => match (n, hundreds) {
            (0, _) => 0,
            (1, _) => 1,
            (2, _) => 2,
            (_, 3..=10) => 3,
            (_, 11..=99) => 4,
            _ => 5,
        },
        _ => (n != 1) as usize,
    }
}

#[cfg(test)]
mod tests {
    use super::forms;

    #[test]
    fn each_language_counts_its_own_way() {
        let across = |tag: &str| [0, 1, 2, 5, 11, 21, 22, 101].map(|n| forms(tag, n));
        assert_eq!(across("de"), [1, 0, 1, 1, 1, 1, 1, 1]);
        assert_eq!(across("en"), across("nl"));
        assert_eq!(across("fr"), [0, 0, 1, 1, 1, 1, 1, 1]);
        assert_eq!(across("pt-BR"), across("fr"));
        assert_eq!(across("pt-PT"), across("de"));
        assert_eq!(across("ja"), [0; 8]);
        assert_eq!(across("ru"), [2, 0, 1, 2, 2, 0, 1, 0]);
        assert_eq!(across("pl"), [2, 0, 1, 2, 2, 2, 1, 2]);
        assert_eq!(across("cs"), [2, 0, 1, 2, 2, 2, 2, 2]);
        assert_eq!(across("ar"), [0, 1, 2, 3, 4, 4, 4, 5]);
        assert_eq!(forms("de", -1), 0);
    }
}
