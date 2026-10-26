//! Naming the lexeme of a Greek word that has no tagged original.
//!
//! The Septuagint's deuterocanonical books (Tobit, Sirach, the Maccabees…)
//! and its additions to Esther and Daniel were written in Greek or survive
//! only in it: there is no Hebrew to align them to. Their words are the
//! original, and what a reader wants of one is its lexeme. Strong's Greek
//! numbers are those of the New Testament's vocabulary, which the
//! Septuagint largely shares, so a word is identified by the SBLGNT's tagged
//! words: by its exact form (accents aside), else by its stem, else by the
//! stem of a lexicon headword. A word whose lexeme the New Testament never
//! uses has no Strong's number, and stays unnamed.

use std::collections::HashMap;

use super::align::{greek_stem, plain_greek};

pub struct GreekLexemes {
    forms: HashMap<String, u32>,
    stems: HashMap<String, u32>,
    headwords: HashMap<String, u32>,
}

/// The most frequent value of each key.
fn majority(counts: HashMap<String, HashMap<u32, usize>>) -> HashMap<String, u32> {
    counts
        .into_iter()
        .filter_map(|(k, by)| by.into_iter().max_by_key(|&(g, n)| (n, std::cmp::Reverse(g))).map(|(g, _)| (k, g)))
        .collect()
}

fn plain(word: &str) -> String {
    plain_greek(&word.trim_matches(|c: char| !c.is_alphabetic()).to_lowercase())
}

impl GreekLexemes {
    /// From tagged words (surface form, Strong's number) and lexicon
    /// headwords (headword, Strong's number).
    pub fn new<'a>(
        tagged: impl IntoIterator<Item = (&'a str, u32)>,
        headwords: impl IntoIterator<Item = (&'a str, u32)>,
    ) -> GreekLexemes {
        let mut forms: HashMap<String, HashMap<u32, usize>> = HashMap::new();
        let mut stems: HashMap<String, HashMap<u32, usize>> = HashMap::new();
        for (word, g) in tagged {
            let p = plain(word);
            if p.is_empty() {
                continue;
            }
            *stems.entry(greek_stem(&p)).or_default().entry(g).or_default() += 1;
            *forms.entry(p).or_default().entry(g).or_default() += 1;
        }
        let mut heads: HashMap<String, HashMap<u32, usize>> = HashMap::new();
        for (word, g) in headwords {
            let p = plain(word);
            if !p.is_empty() {
                *heads.entry(greek_stem(&p)).or_default().entry(g).or_default() += 1;
            }
        }
        GreekLexemes { forms: majority(forms), stems: majority(stems), headwords: majority(heads) }
    }

    /// The Strong's number (bare) of a Greek word, if the New Testament's
    /// vocabulary has its lexeme.
    pub fn lexeme(&self, word: &str) -> Option<u32> {
        let p = plain(word);
        if p.is_empty() {
            return None;
        }
        if let Some(&g) = self.forms.get(&p) {
            return Some(g);
        }
        let stem = greek_stem(&p);
        self.stems.get(&stem).or_else(|| self.headwords.get(&stem)).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn by_form_then_stem_then_headword() {
        let lex = GreekLexemes::new(
            [("λόγος", 3056), ("λόγον", 3056), ("θεοῦ", 2316)],
            [("ἀγαπάω", 25), ("λόγος", 3056)],
        );
        assert_eq!(lex.lexeme("λόγος"), Some(3056));
        assert_eq!(lex.lexeme("Λόγου,"), Some(3056)); // by stem
        assert_eq!(lex.lexeme("θεός"), Some(2316)); // by stem
        assert_eq!(lex.lexeme("ἀγαπάω"), Some(25)); // a headword
        assert_eq!(lex.lexeme("Τωβιθ"), None);
    }
}
