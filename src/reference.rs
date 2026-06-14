//! A small, forgiving Scripture-reference parser.
//!
//! Handles inputs like `John 3:16`, `Jn 3`, `1 Cor 13:4-7`, `Gen 1`,
//! `Ps 23`, `genesis 1:1`. Returns a [`Reference`] resolved to a canonical
//! book number (1..=66) plus optional chapter and verse range.

use crate::canon;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Reference {
    pub book: i64,
    pub chapter: Option<i64>,
    pub verse_start: Option<i64>,
    pub verse_end: Option<i64>,
}

/// Normalise a book token for alias matching: lower-case, drop spaces and dots.
fn norm(s: &str) -> String {
    s.chars()
        .filter(|c| !c.is_whitespace() && *c != '.')
        .flat_map(|c| c.to_lowercase())
        .collect()
}

/// True if a token is purely a chapter/verse spec, e.g. `3`, `3:16`, `4-7`.
fn is_spec_token(tok: &str) -> bool {
    !tok.is_empty()
        && tok.chars().next().is_some_and(|c| c.is_ascii_digit())
        && tok.chars().all(|c| c.is_ascii_digit() || matches!(c, ':' | '-' | ','))
}

/// Parse a reference string. Returns `None` if the book can't be recognised.
pub fn parse(input: &str) -> Option<Reference> {
    let input = input.trim();
    if input.is_empty() {
        return None;
    }
    let tokens: Vec<&str> = input.split_whitespace().collect();

    // Split into book tokens and an optional trailing chapter/verse spec.
    // The book name may *start* with an ordinal ("1 Corinthians") but only the
    // trailing spec is a pure-number token, so the first such token ends the book.
    let mut split = tokens.len();
    for (i, tok) in tokens.iter().enumerate() {
        if i > 0 && is_spec_token(tok) {
            split = i;
            break;
        }
    }
    let book_key = norm(&tokens[..split].join(""));
    let spec: String = tokens[split..].iter().flat_map(|t| t.chars()).collect();

    let book = canon::BOOKS
        .iter()
        .find(|b| b.aliases.contains(&book_key.as_str()))?
        .nr;

    if spec.is_empty() {
        return Some(Reference { book, chapter: None, verse_start: None, verse_end: None });
    }

    // spec forms: "C", "C:V", "C:V-V"
    let (chapter_str, verse_part) = match spec.split_once(':') {
        Some((c, v)) => (c, Some(v)),
        None => (spec.as_str(), None),
    };
    let chapter = chapter_str.parse::<i64>().ok();

    let (verse_start, verse_end) = match verse_part {
        None => (None, None),
        Some(v) => match v.split_once('-') {
            Some((a, b)) => (a.parse().ok(), b.parse().ok()),
            None => {
                let s = v.parse().ok();
                (s, s)
            }
        },
    };

    Some(Reference { book, chapter, verse_start, verse_end })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(b: i64, c: Option<i64>, vs: Option<i64>, ve: Option<i64>) -> Reference {
        Reference { book: b, chapter: c, verse_start: vs, verse_end: ve }
    }

    #[test]
    fn basic() {
        assert_eq!(parse("John 3:16"), Some(r(43, Some(3), Some(16), Some(16))));
        assert_eq!(parse("jn 3"), Some(r(43, Some(3), None, None)));
        assert_eq!(parse("1 Cor 13:4-7"), Some(r(46, Some(13), Some(4), Some(7))));
        assert_eq!(parse("genesis"), Some(r(1, None, None, None)));
        assert_eq!(parse("Ps 23"), Some(r(19, Some(23), None, None)));
        assert_eq!(parse("song of solomon 2:1"), Some(r(22, Some(2), Some(1), Some(1))));
    }

    #[test]
    fn unknown() {
        assert_eq!(parse("Hogwarts 1:1"), None);
        assert_eq!(parse(""), None);
    }
}
