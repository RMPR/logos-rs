//! Recognising a book of the Bible from a heading or a running header.
//!
//! Publishers name books in their own language and style, so the matcher
//! knows the canonical English names and abbreviations, and is handed the
//! localized names already in the database (French, Norwegian, Hebrew, Greek…)
//! on top. Matching ignores case, accents, spaces and punctuation, and reads a
//! leading ordinal however it is written: "1 Samuel", "I Samuel",
//! "First Samuel", "1st Samuel".

use std::collections::HashMap;

use unicode_normalization::UnicodeNormalization;

use crate::canon::BOOKS;

pub struct BookMatcher {
    names: HashMap<String, i64>,
}

/// Lower-case letters and digits only, accents removed.
pub fn normalize(s: &str) -> String {
    s.nfkd()
        .filter(|c| !unicode_normalization::char::is_combining_mark(*c))
        .flat_map(char::to_lowercase)
        .filter(|c| c.is_alphanumeric())
        .collect()
}

/// A leading ordinal word as a digit: "first" / "i" / "1st" → "1".
fn ordinal(word: &str) -> Option<&'static str> {
    match normalize(word).as_str() {
        "1" | "i" | "1st" | "first" | "premier" | "premiere" | "forste" => Some("1"),
        "2" | "ii" | "2nd" | "second" | "deuxieme" | "andre" => Some("2"),
        "3" | "iii" | "3rd" | "third" | "troisieme" | "tredje" => Some("3"),
        _ => None,
    }
}

/// Wordings that only frame a book's name in a title.
const FRAMES: &[&str] = &[
    "thebookof", "bookof", "thegospelaccordingto", "gospelaccordingto", "thegospelof",
    "thefirstbookof", "thesecondbookof", "levangileselon", "evangelietetter", "livrede",
    "theepistleof", "theepistleofpaulthe", "theletterof", "theletterto", "theepistleto",
];

impl BookMatcher {
    /// The canonical names and abbreviations, plus `localized` (book number,
    /// name) pairs, typically every book name in the database.
    pub fn new(localized: impl IntoIterator<Item = (i64, String)>) -> BookMatcher {
        let mut names = HashMap::new();
        for b in BOOKS {
            names.insert(normalize(b.name), b.nr);
            // Short abbreviations ("ex", "he", "re") are ordinary words in
            // running text; only the longer ones are safe in a heading.
            for alias in b.aliases.iter().filter(|a| a.len() >= 4) {
                names.insert(normalize(alias), b.nr);
            }
        }
        for (nr, name) in localized {
            let key = normalize(&name);
            if (1..=66).contains(&nr) && key.chars().count() >= 2 {
                names.entry(key).or_insert(nr);
            }
        }
        BookMatcher { names }
    }

    fn lookup(&self, words: &[&str]) -> Option<i64> {
        if words.is_empty() {
            return None;
        }
        let mut key = String::new();
        let rest = match ordinal(words[0]) {
            Some(digit) if words.len() > 1 => {
                key.push_str(digit);
                &words[1..]
            }
            _ => words,
        };
        for w in rest {
            key.push_str(&normalize(w));
        }
        if let Some(&nr) = self.names.get(&key) {
            return Some(nr);
        }
        let unframed = FRAMES.iter().find_map(|f| key.strip_prefix(f))?;
        self.names.get(unframed).copied()
    }

    /// The book a whole heading names, if it is just a book's name.
    pub fn book(&self, heading: &str) -> Option<i64> {
        let words: Vec<&str> = heading.split_whitespace().collect();
        if words.len() > 8 {
            return None;
        }
        self.lookup(&words)
    }

    /// A book name followed by a chapter number at the start of `line` — a
    /// chapter heading ("Genesis 3") or a running page header ("Genesis 21
    /// New American Standard Bible") — as (book, chapter).
    pub fn book_and_chapter(&self, line: &str) -> Option<(i64, i64)> {
        let words: Vec<&str> = line.split_whitespace().collect();
        // the longest name wins: "Song of Solomon 2" over "Song"
        for k in (1..words.len().min(6)).rev() {
            let Ok(chapter) = words[k].trim_end_matches([':', '.']).parse::<i64>() else {
                continue;
            };
            if !(1..=150).contains(&chapter) {
                continue;
            }
            if let Some(nr) = self.lookup(&words[..k]) {
                return Some((nr, chapter));
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_in_their_usual_spellings() {
        let m = BookMatcher::new([(1, "Genèse".to_string()), (62, "1 Jean".to_string())]);
        assert_eq!(m.book("Genesis"), Some(1));
        assert_eq!(m.book("GENÈSE"), Some(1));
        assert_eq!(m.book("Genese"), Some(1));
        assert_eq!(m.book("I Samuel"), Some(9));
        assert_eq!(m.book("First Samuel"), Some(9));
        assert_eq!(m.book("1st John"), Some(62));
        assert_eq!(m.book("1 Jean"), Some(62));
        assert_eq!(m.book("The Gospel According to John"), Some(43));
        assert_eq!(m.book("Song of Solomon"), Some(22));
        assert_eq!(m.book("In the beginning"), None);
        // a two-letter abbreviation is not a heading
        assert_eq!(m.book("He"), None);
    }

    #[test]
    fn headers_with_a_chapter() {
        let m = BookMatcher::new([]);
        assert_eq!(m.book_and_chapter("Genesis 21        New American Standard Bible"), Some((1, 21)));
        assert_eq!(m.book_and_chapter("Song of Solomon 2"), Some((22, 2)));
        assert_eq!(m.book_and_chapter("2 Kings 4"), Some((12, 4)));
        assert_eq!(m.book_and_chapter("Psalm 119"), Some((19, 119)));
        assert_eq!(m.book_and_chapter("12 The gold of that land is good"), None);
        assert_eq!(m.book_and_chapter("Genesis"), None);
    }
}
