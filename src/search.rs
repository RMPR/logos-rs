//! Advanced search: a small query language over the tagged original-language
//! text, evaluated in memory.
//!
//! A query is a boolean combination of *slots*. A slot constrains one word:
//!
//! ```text
//! love                          surface form (accents and case ignored)
//! "in the beginning"            adjacent surface forms
//! lemma:ἀγαπάω   l:אלהים        dictionary form
//! G25  H430  strongs:G25        Strong's number
//! gloss:love                    any lexeme whose lexicon gloss mentions it
//! [lemma:ἀγαπάω aorist active]  several constraints on the SAME word
//! tense:aorist  mood:imperative  case:genitive  pos:verb  stem:hiphil …
//! aorist  genitive  hiphil …    bare morphology keywords
//! ```
//!
//! Slots combine with `AND` (or just a space), `OR`, `NOT`, parentheses, and
//! word-order proximity: `A BEFORE/2 B` (A at most two words before B),
//! `A AFTER/1 B`, `A NEAR/3 B` (either side). A count may be omitted:
//! `BEFORE` / `AFTER` mean anywhere in the verse in that order, `NEAR` is
//! `NEAR/3`. Values may be alternated with `|`: `tense:aorist|perfect`.
//!
//! Evaluation yields, per verse, the positions of the words that matched, so
//! the caller can mark them — and their Strong's numbers, so the same hits can
//! be marked in a translation that only carries Strong's tags.

use std::collections::{HashMap, HashSet};
use std::fmt;

use unicode_normalization::UnicodeNormalization;

// ---------------------------------------------------------------------------
// Text normalisation
// ---------------------------------------------------------------------------

/// Fold a word for matching: strip accents, breathings, Hebrew points and
/// cantillation; lower-case; final sigma → sigma. Both the corpus and the
/// query go through this, so `αγαπαω` finds `ἀγαπάω`.
pub fn fold(s: &str) -> String {
    s.nfd()
        .filter(|c| !is_mark(*c))
        .flat_map(char::to_lowercase)
        .map(|c| if c == 'ς' { 'σ' } else { c })
        .collect()
}

fn is_mark(c: char) -> bool {
    // combining diacritics, Hebrew points/accents, and the maqaf-adjacent marks
    matches!(c as u32,
        0x0300..=0x036F | 0x0591..=0x05BD | 0x05BF | 0x05C1..=0x05C2 | 0x05C4..=0x05C5 | 0x05C7
        | 0x1AB0..=0x1AFF | 0x1DC0..=0x1DFF | 0x20D0..=0x20FF | 0xFE20..=0xFE2F)
}

// ---------------------------------------------------------------------------
// Morphology as features
// ---------------------------------------------------------------------------

/// The grammatical features a slot can ask about, unified across Hebrew and
/// Greek. Each is a small closed set of values; `0` means "not applicable".
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Field {
    Pos,
    Tense,
    Voice,
    Mood,
    Case,
    Number,
    Gender,
    Person,
    Stem,
    State,
    Degree,
}

impl Field {
    fn parse(s: &str) -> Option<Field> {
        Some(match s {
            "pos" => Field::Pos,
            "tense" | "form" | "conj" | "aspect" => Field::Tense,
            "voice" => Field::Voice,
            "mood" => Field::Mood,
            "case" => Field::Case,
            "number" | "num" => Field::Number,
            "gender" | "gen" => Field::Gender,
            "person" | "pers" => Field::Person,
            "stem" | "binyan" => Field::Stem,
            "state" => Field::State,
            "degree" => Field::Degree,
            _ => return None,
        })
    }
}

/// One morpheme's features. Hebrew words are often several morphemes
/// (`HC/Vqp3ms` = conjunction + verb), and a slot's morphology constraints
/// must all hold on the *same* morpheme, so a word carries a list of these.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Morph {
    pub pos: u8,
    pub tense: u8,
    pub voice: u8,
    pub mood: u8,
    pub case: u8,
    pub number: u8,
    pub gender: u8,
    pub person: u8,
    pub stem: u8,
    pub state: u8,
    pub degree: u8,
}

impl Morph {
    fn get(&self, f: Field) -> u8 {
        match f {
            Field::Pos => self.pos,
            Field::Tense => self.tense,
            Field::Voice => self.voice,
            Field::Mood => self.mood,
            Field::Case => self.case,
            Field::Number => self.number,
            Field::Gender => self.gender,
            Field::Person => self.person,
            Field::Stem => self.stem,
            Field::State => self.state,
            Field::Degree => self.degree,
        }
    }
}

/// Every value keyword the language knows, with the field it belongs to and
/// the code the corpus stores. Bare keywords in a query resolve through this
/// table, so `aorist` alone means `tense:aorist`.
const VALUES: &[(&str, Field, u8)] = &[
    // part of speech
    ("verb", Field::Pos, 1), ("noun", Field::Pos, 2), ("adjective", Field::Pos, 3),
    ("adj", Field::Pos, 3), ("article", Field::Pos, 4), ("pronoun", Field::Pos, 5),
    ("preposition", Field::Pos, 6), ("prep", Field::Pos, 6), ("conjunction", Field::Pos, 7),
    ("conj", Field::Pos, 7), ("adverb", Field::Pos, 8), ("particle", Field::Pos, 9),
    ("interjection", Field::Pos, 10), ("name", Field::Pos, 11), ("proper", Field::Pos, 11),
    ("suffix", Field::Pos, 12), ("numeral", Field::Pos, 13), ("number-word", Field::Pos, 13),
    // tense / aspect / conjugation (Greek tenses, then Hebrew conjugations)
    ("present", Field::Tense, 1), ("imperfect", Field::Tense, 2), ("future", Field::Tense, 3),
    ("aorist", Field::Tense, 4), ("perfect", Field::Tense, 5), ("pluperfect", Field::Tense, 6),
    ("wayyiqtol", Field::Tense, 7), ("sequential-imperfect", Field::Tense, 7),
    ("weqatal", Field::Tense, 8), ("sequential-perfect", Field::Tense, 8),
    ("cohortative", Field::Tense, 9), ("jussive", Field::Tense, 10),
    ("infinitive-absolute", Field::Tense, 11), ("infinitive-construct", Field::Tense, 12),
    // voice
    ("active", Field::Voice, 1), ("middle", Field::Voice, 2), ("passive", Field::Voice, 3),
    // mood (Hebrew imperative / participle / infinitive live here too)
    ("indicative", Field::Mood, 1), ("imperative", Field::Mood, 2), ("subjunctive", Field::Mood, 3),
    ("optative", Field::Mood, 4), ("infinitive", Field::Mood, 5), ("participle", Field::Mood, 6),
    // case
    ("nominative", Field::Case, 1), ("genitive", Field::Case, 2), ("dative", Field::Case, 3),
    ("accusative", Field::Case, 4), ("vocative", Field::Case, 5),
    // number
    ("singular", Field::Number, 1), ("plural", Field::Number, 2), ("dual", Field::Number, 3),
    // gender
    ("masculine", Field::Gender, 1), ("feminine", Field::Gender, 2), ("neuter", Field::Gender, 3),
    ("common", Field::Gender, 4), ("both", Field::Gender, 5),
    // person
    ("1", Field::Person, 1), ("2", Field::Person, 2), ("3", Field::Person, 3),
    ("1st", Field::Person, 1), ("2nd", Field::Person, 2), ("3rd", Field::Person, 3),
    // Hebrew stem (binyan)
    ("qal", Field::Stem, 1), ("niphal", Field::Stem, 2), ("piel", Field::Stem, 3),
    ("pual", Field::Stem, 4), ("hiphil", Field::Stem, 5), ("hophal", Field::Stem, 6),
    ("hithpael", Field::Stem, 7), ("polel", Field::Stem, 8), ("polal", Field::Stem, 9),
    ("hithpolel", Field::Stem, 10), ("tiphil", Field::Stem, 11),
    // state
    ("absolute", Field::State, 1), ("construct", Field::State, 2), ("determined", Field::State, 3),
    // degree
    ("comparative", Field::Degree, 1), ("superlative", Field::Degree, 2),
];

fn value(field: Field, word: &str) -> Option<u8> {
    VALUES.iter().find(|(w, f, _)| *f == field && *w == word).map(|&(_, _, v)| v)
}

fn keyword(word: &str) -> Option<(Field, u8)> {
    VALUES.iter().find(|(w, _, _)| *w == word).map(|&(_, f, v)| (f, v))
}

/// Decode a MorphGNT code (`"V- 3AAI-S--"`: part of speech, then the eight
/// positional parse characters) into features.
pub fn greek_morph(code: &str) -> Vec<Morph> {
    let (pos, parse) = code.split_once(' ').unwrap_or((code, ""));
    let p: Vec<char> = parse.chars().collect();
    let at = |i: usize| p.get(i).copied().filter(|&c| c != '-');
    let mut m = Morph {
        pos: match pos {
            "V-" => 1,
            "N-" => 2,
            "A-" => 3,
            "RA" => 4,
            "RD" | "RI" | "RP" | "RR" => 5,
            "P-" => 6,
            "C-" => 7,
            "D-" => 8,
            "X-" => 9,
            "I-" => 10,
            _ => 0,
        },
        ..Morph::default()
    };
    m.person = match at(0) { Some('1') => 1, Some('2') => 2, Some('3') => 3, _ => 0 };
    m.tense = match at(1) {
        Some('P') => 1, Some('I') => 2, Some('F') => 3, Some('A') => 4, Some('X') => 5, Some('Y') => 6, _ => 0,
    };
    m.voice = match at(2) { Some('A') => 1, Some('M') => 2, Some('P') => 3, _ => 0 };
    m.mood = match at(3) {
        Some('I') => 1, Some('D') => 2, Some('S') => 3, Some('O') => 4, Some('N') => 5, Some('P') => 6, _ => 0,
    };
    m.case = match at(4) { Some('N') => 1, Some('G') => 2, Some('D') => 3, Some('A') => 4, Some('V') => 5, _ => 0 };
    m.number = match at(5) { Some('S') => 1, Some('P') => 2, _ => 0 };
    m.gender = match at(6) { Some('M') => 1, Some('F') => 2, Some('N') => 3, _ => 0 };
    m.degree = match at(7) { Some('C') => 1, Some('S') => 2, _ => 0 };
    vec![m]
}

/// Decode an OSHM code (`HC/Vqp3ms`: language letter, then `/`-separated
/// morphemes) into one feature set per morpheme.
pub fn hebrew_morph(code: &str) -> Vec<Morph> {
    code.split('/')
        .map(|seg| seg.strip_prefix(['H', 'A']).unwrap_or(seg))
        .filter(|seg| !seg.is_empty())
        .map(hebrew_segment)
        .collect()
}

fn hebrew_segment(s: &str) -> Morph {
    let c: Vec<char> = s.chars().collect();
    let at = |i: usize| c.get(i).copied();
    let gender = |ch: Option<char>| match ch { Some('m') => 1, Some('f') => 2, Some('b') => 5, Some('c') => 4, _ => 0 };
    let number = |ch: Option<char>| match ch { Some('s') => 1, Some('p') => 2, Some('d') => 3, _ => 0 };
    let state = |ch: Option<char>| match ch { Some('a') => 1, Some('c') => 2, Some('d') => 3, _ => 0 };
    let person = |ch: Option<char>| match ch { Some('1') => 1, Some('2') => 2, Some('3') => 3, _ => 0 };
    let mut m = Morph::default();
    match at(0) {
        Some('V') => {
            m.pos = 1;
            m.stem = match at(1) {
                Some('q') => 1, Some('N') => 2, Some('p') => 3, Some('P') => 4, Some('h') => 5, Some('H') => 6,
                Some('t') => 7, Some('o') => 8, Some('O') => 9, Some('r') => 10, Some('e') => 11, _ => 0,
            };
            match at(2) {
                Some('p') => m.tense = 5,
                Some('q') => m.tense = 8,
                Some('i') => m.tense = 2,
                Some('w') => m.tense = 7,
                Some('h') => m.tense = 9,
                Some('j') => m.tense = 10,
                Some('v') => m.mood = 2,
                Some('r') => { m.mood = 6; m.voice = 1; }
                Some('s') => { m.mood = 6; m.voice = 3; }
                Some('a') => { m.mood = 5; m.tense = 11; }
                Some('c') => { m.mood = 5; m.tense = 12; }
                _ => {}
            }
            match at(2) {
                Some('r') | Some('s') => {
                    m.gender = gender(at(3));
                    m.number = number(at(4));
                    m.state = state(at(5));
                }
                Some('a') | Some('c') => {}
                _ => {
                    m.person = person(at(3));
                    m.gender = gender(at(4));
                    m.number = number(at(5));
                }
            }
        }
        Some('N') => {
            m.pos = if at(1) == Some('p') { 11 } else { 2 };
            if at(1) != Some('p') {
                m.gender = gender(at(2));
                m.number = number(at(3));
                m.state = state(at(4));
            }
        }
        Some('A') => {
            m.pos = if matches!(at(1), Some('c') | Some('o')) { 13 } else { 3 };
            m.gender = gender(at(2));
            m.number = number(at(3));
            m.state = state(at(4));
        }
        Some('P') => {
            m.pos = 5;
            m.person = person(at(2));
            m.gender = gender(at(3));
            m.number = number(at(4));
        }
        Some('S') => {
            m.pos = 12;
            if at(1) == Some('p') {
                m.person = person(at(2));
                m.gender = gender(at(3));
                m.number = number(at(4));
            }
        }
        Some('R') => m.pos = 6,
        Some('C') => m.pos = 7,
        Some('D') => m.pos = 8,
        Some('T') => m.pos = match at(1) { Some('d') => 4, Some('j') => 10, _ => 9 },
        _ => {}
    }
    m
}

// ---------------------------------------------------------------------------
// Corpus
// ---------------------------------------------------------------------------

/// One word of the corpus, everything pre-folded and interned.
#[derive(Clone, Debug)]
pub struct Word {
    pub surface: u32,
    pub lemma: u32,
    pub strongs: u32,
    pub morphs: Vec<Morph>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct VerseId {
    pub book: i64,
    pub chapter: i64,
    pub verse: i64,
}

/// A verse's words are `words[start..end]`.
#[derive(Clone, Copy, Debug)]
pub struct VerseSpan {
    pub id: VerseId,
    pub start: usize,
    pub end: usize,
}

#[derive(Default)]
pub struct Interner {
    ids: HashMap<String, u32>,
    names: Vec<String>,
}

impl Interner {
    pub fn intern(&mut self, s: &str) -> u32 {
        if let Some(&id) = self.ids.get(s) {
            return id;
        }
        let id = self.names.len() as u32;
        self.names.push(s.to_string());
        self.ids.insert(s.to_string(), id);
        id
    }
    pub fn get(&self, s: &str) -> Option<u32> {
        self.ids.get(s).copied()
    }
    pub fn name(&self, id: u32) -> &str {
        &self.names[id as usize]
    }
}

/// The words of one translation, in canonical order, ready to search.
pub struct Corpus {
    pub words: Vec<Word>,
    pub verses: Vec<VerseSpan>,
    pub surfaces: Interner,
    pub lemmas: Interner,
    pub strongs: Interner,
    /// Original surface forms, for marking hits in this corpus's own text.
    pub display: Vec<String>,
}

/// Raw input for building a corpus: one row per word, in canonical order.
pub struct RawWord {
    pub book: i64,
    pub chapter: i64,
    pub verse: i64,
    pub text: String,
    pub lemma: String,
    pub strongs: String,
    pub morph: String,
}

impl Corpus {
    pub fn build(rows: Vec<RawWord>, hebrew: bool) -> Corpus {
        let mut c = Corpus {
            words: Vec::with_capacity(rows.len()),
            verses: Vec::new(),
            surfaces: Interner::default(),
            lemmas: Interner::default(),
            strongs: Interner::default(),
            display: Vec::with_capacity(rows.len()),
        };
        // id 0 is reserved for "none" in every interner
        c.surfaces.intern("");
        c.lemmas.intern("");
        c.strongs.intern("");
        let mut cur: Option<VerseId> = None;
        for r in rows {
            let id = VerseId { book: r.book, chapter: r.chapter, verse: r.verse };
            if cur != Some(id) {
                let start = c.words.len();
                if let Some(last) = c.verses.last_mut() {
                    last.end = start;
                }
                c.verses.push(VerseSpan { id, start, end: start });
                cur = Some(id);
            }
            let morphs = if r.morph.is_empty() {
                Vec::new()
            } else if hebrew {
                hebrew_morph(&r.morph)
            } else {
                greek_morph(&r.morph)
            };
            c.words.push(Word {
                surface: c.surfaces.intern(&fold(&r.text)),
                lemma: c.lemmas.intern(&fold(&r.lemma)),
                strongs: c.strongs.intern(&r.strongs),
                morphs,
            });
            c.display.push(r.text);
        }
        if let Some(last) = c.verses.last_mut() {
            last.end = c.words.len();
        }
        c
    }
}

impl Corpus {
    /// The span of one verse, by binary search (verses are in canonical order).
    pub fn span(&self, id: VerseId) -> Option<&VerseSpan> {
        self.verses.binary_search_by(|s| s.id.cmp(&id)).ok().map(|i| &self.verses[i])
    }
}

/// Lexicon entries, folded, for resolving `lemma:` and `gloss:` to Strong's.
pub struct Lexicon {
    pub entries: Vec<LexEntry>,
}

pub struct LexEntry {
    pub strongs: String,
    pub lemma: String, // folded
    pub gloss: String, // folded, gloss + definition
}

impl Lexicon {
    pub fn new(rows: impl IntoIterator<Item = (String, String, String)>) -> Lexicon {
        Lexicon {
            entries: rows
                .into_iter()
                .map(|(strongs, lemma, gloss)| LexEntry { strongs, lemma: fold(&lemma), gloss: fold(&gloss) })
                .collect(),
        }
    }

    fn strongs_for_lemma(&self, lemma: &str) -> Vec<String> {
        self.entries.iter().filter(|e| e.lemma == lemma).map(|e| e.strongs.clone()).collect()
    }

    fn strongs_for_gloss(&self, term: &str) -> Vec<String> {
        // whole-word match inside the gloss text
        self.entries
            .iter()
            .filter(|e| e.gloss.split(|c: char| !c.is_alphanumeric()).any(|w| w == term))
            .map(|e| e.strongs.clone())
            .collect()
    }
}

// ---------------------------------------------------------------------------
// Query language
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
pub enum Constraint {
    /// Surface form(s); a trailing `*` matches a prefix.
    Surface(Vec<String>),
    Lemma(Vec<String>),
    Strongs(Vec<String>),
    Gloss(Vec<String>),
    Feature(Field, Vec<u8>),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Slot {
    pub constraints: Vec<Constraint>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Prox {
    Before,
    After,
    Near,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Node {
    Slot(Slot),
    Phrase(Vec<Slot>),
    And(Vec<Node>),
    Or(Vec<Node>),
    Not(Box<Node>),
    Prox { a: Box<Node>, b: Box<Node>, kind: Prox, dist: Option<usize> },
}

#[derive(Debug, Clone, PartialEq)]
pub struct ParseError(pub String);

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Word(String),
    Quoted(String),
    LBracket,
    RBracket,
    LParen,
    RParen,
    And,
    Or,
    Not,
    Prox(Prox, Option<usize>),
}

fn lex(q: &str) -> Result<Vec<Tok>, ParseError> {
    let mut out = Vec::new();
    let chars: Vec<char> = q.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match c {
            c if c.is_whitespace() => i += 1,
            '[' => { out.push(Tok::LBracket); i += 1; }
            ']' => { out.push(Tok::RBracket); i += 1; }
            '(' => { out.push(Tok::LParen); i += 1; }
            ')' => { out.push(Tok::RParen); i += 1; }
            '"' | '“' | '”' | '«' | '»' => {
                let mut j = i + 1;
                let mut s = String::new();
                while j < chars.len() && !matches!(chars[j], '"' | '“' | '”' | '«' | '»') {
                    s.push(chars[j]);
                    j += 1;
                }
                if j >= chars.len() {
                    return Err(ParseError("unclosed quote".into()));
                }
                out.push(Tok::Quoted(s));
                i = j + 1;
            }
            _ => {
                let mut j = i;
                while j < chars.len() && !chars[j].is_whitespace() && !matches!(chars[j], '[' | ']' | '(' | ')' | '"' | '“' | '”' | '«' | '»') {
                    j += 1;
                }
                let mut w: String = chars[i..j].iter().collect();
                i = j;
                // `-word` is NOT word, as in every search box
                if w.len() > 1 && w.starts_with('-') {
                    out.push(Tok::Not);
                    w.remove(0);
                }
                let upper = w.to_ascii_uppercase();
                let tok = match upper.as_str() {
                    "AND" | "&&" => Tok::And,
                    "OR" | "||" => Tok::Or,
                    "NOT" | "-" => Tok::Not,
                    _ => {
                        if let Some(rest) = upper.strip_prefix("BEFORE") {
                            Tok::Prox(Prox::Before, prox_dist(rest)?)
                        } else if let Some(rest) = upper.strip_prefix("AFTER") {
                            Tok::Prox(Prox::After, prox_dist(rest)?)
                        } else if let Some(rest) = upper.strip_prefix("NEAR") {
                            Tok::Prox(Prox::Near, prox_dist(rest)?.or(Some(3)))
                        } else if let Some(rest) = upper.strip_prefix("WITHIN") {
                            Tok::Prox(Prox::Near, prox_dist(rest)?.or(Some(3)))
                        } else {
                            Tok::Word(w)
                        }
                    }
                };
                out.push(tok);
            }
        }
    }
    Ok(out)
}

/// `"/3"` → `Some(3)`; `""` → `None`; anything else is an error.
fn prox_dist(rest: &str) -> Result<Option<usize>, ParseError> {
    if rest.is_empty() {
        return Ok(None);
    }
    rest.strip_prefix('/')
        .and_then(|n| n.parse::<usize>().ok())
        .map(Some)
        .ok_or_else(|| ParseError(format!("bad distance {rest:?}: use e.g. NEAR/3")))
}

struct Parser {
    toks: Vec<Tok>,
    i: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.i)
    }
    fn next(&mut self) -> Option<Tok> {
        let t = self.toks.get(self.i).cloned();
        self.i += 1;
        t
    }

    // or := and ( OR and )*
    fn or(&mut self) -> Result<Node, ParseError> {
        let mut parts = vec![self.and()?];
        while self.peek() == Some(&Tok::Or) {
            self.next();
            parts.push(self.and()?);
        }
        Ok(if parts.len() == 1 { parts.pop().unwrap() } else { Node::Or(parts) })
    }

    // and := prox ( [AND] prox )*     — juxtaposition is AND
    fn and(&mut self) -> Result<Node, ParseError> {
        let mut parts = vec![self.prox()?];
        loop {
            match self.peek() {
                Some(Tok::And) => {
                    self.next();
                    parts.push(self.prox()?);
                }
                Some(Tok::Or) | Some(Tok::RParen) | None => break,
                Some(Tok::RBracket) => return Err(ParseError("unexpected ]".into())),
                _ => parts.push(self.prox()?),
            }
        }
        Ok(if parts.len() == 1 { parts.pop().unwrap() } else { Node::And(parts) })
    }

    // prox := unary ( PROX unary )*   — left-associative
    fn prox(&mut self) -> Result<Node, ParseError> {
        let mut left = self.unary()?;
        while let Some(Tok::Prox(kind, dist)) = self.peek().cloned() {
            self.next();
            let right = self.unary()?;
            left = Node::Prox { a: Box::new(left), b: Box::new(right), kind, dist };
        }
        Ok(left)
    }

    // unary := NOT unary | primary
    fn unary(&mut self) -> Result<Node, ParseError> {
        if self.peek() == Some(&Tok::Not) {
            self.next();
            return Ok(Node::Not(Box::new(self.unary()?)));
        }
        self.primary()
    }

    // primary := '(' or ')' | '[' constraint* ']' | quoted | word
    fn primary(&mut self) -> Result<Node, ParseError> {
        match self.next() {
            Some(Tok::LParen) => {
                let n = self.or()?;
                if self.next() != Some(Tok::RParen) {
                    return Err(ParseError("missing )".into()));
                }
                Ok(n)
            }
            Some(Tok::LBracket) => {
                let mut constraints = Vec::new();
                loop {
                    match self.next() {
                        Some(Tok::RBracket) => break,
                        Some(Tok::Word(w)) => constraints.push(constraint(&w)?),
                        Some(Tok::Quoted(w)) => constraints.push(Constraint::Surface(vec![fold(&w)])),
                        Some(other) => return Err(ParseError(format!("{other:?} not allowed inside [ ]"))),
                        None => return Err(ParseError("missing ]".into())),
                    }
                }
                if constraints.is_empty() {
                    return Err(ParseError("empty [ ]".into()));
                }
                Ok(Node::Slot(Slot { constraints }))
            }
            Some(Tok::Quoted(s)) => {
                let slots: Vec<Slot> = s
                    .split_whitespace()
                    .map(|w| Slot { constraints: vec![Constraint::Surface(vec![fold(w)])] })
                    .collect();
                if slots.is_empty() {
                    return Err(ParseError("empty phrase".into()));
                }
                Ok(if slots.len() == 1 { Node::Slot(slots.into_iter().next().unwrap()) } else { Node::Phrase(slots) })
            }
            Some(Tok::Word(w)) => Ok(Node::Slot(Slot { constraints: vec![constraint(&w)?] })),
            Some(t) => Err(ParseError(format!("unexpected {t:?}"))),
            None => Err(ParseError("unexpected end of query".into())),
        }
    }
}

/// One constraint token: `field:value|value`, a Strong's number, a bare
/// morphology keyword, or a surface form.
fn constraint(w: &str) -> Result<Constraint, ParseError> {
    if let Some((field, vals)) = w.split_once(':') {
        let field_l = field.to_ascii_lowercase();
        let alts: Vec<&str> = vals.split('|').filter(|v| !v.is_empty()).collect();
        if alts.is_empty() {
            return Err(ParseError(format!("{field}: needs a value")));
        }
        return match field_l.as_str() {
            "lemma" | "l" => Ok(Constraint::Lemma(alts.iter().map(|v| fold(v)).collect())),
            "strongs" | "strong" | "s" => Ok(Constraint::Strongs(alts.iter().map(|v| norm_strongs(v)).collect())),
            "gloss" | "g" | "sense" => Ok(Constraint::Gloss(alts.iter().map(|v| fold(v)).collect())),
            "text" | "word" | "t" => Ok(Constraint::Surface(alts.iter().map(|v| fold(v)).collect())),
            _ => {
                let f = Field::parse(&field_l)
                    .ok_or_else(|| ParseError(format!("unknown field {field:?}")))?;
                let mut codes = Vec::new();
                for v in alts {
                    let vl = v.to_ascii_lowercase();
                    codes.push(value(f, &vl).ok_or_else(|| ParseError(format!("{field} has no value {v:?}")))?);
                }
                Ok(Constraint::Feature(f, codes))
            }
        };
    }
    // alternatives without a field: G25|G26, aorist|perfect, love|hate
    let alts: Vec<&str> = w.split('|').filter(|v| !v.is_empty()).collect();
    if alts.iter().all(|a| is_strongs(a)) {
        return Ok(Constraint::Strongs(alts.iter().map(|v| norm_strongs(v)).collect()));
    }
    let kws: Vec<Option<(Field, u8)>> = alts.iter().map(|a| keyword(&a.to_ascii_lowercase())).collect();
    if kws.iter().all(Option::is_some) {
        let field = kws[0].unwrap().0;
        if kws.iter().all(|k| k.unwrap().0 == field) {
            return Ok(Constraint::Feature(field, kws.iter().map(|k| k.unwrap().1).collect()));
        }
        return Err(ParseError(format!("{w}: alternatives must belong to one field")));
    }
    Ok(Constraint::Surface(alts.iter().map(|v| fold(v)).collect()))
}

fn is_strongs(s: &str) -> bool {
    let mut c = s.chars();
    matches!(c.next(), Some('G' | 'H' | 'g' | 'h')) && {
        let rest: String = c.collect();
        !rest.is_empty() && rest.chars().all(|d| d.is_ascii_digit())
    }
}

fn norm_strongs(s: &str) -> String {
    let mut c = s.chars();
    let letter = c.next().map(|l| l.to_ascii_uppercase()).unwrap_or('G');
    let digits: String = c.collect();
    let n: u32 = digits.parse().unwrap_or(0);
    format!("{letter}{n}")
}

pub fn parse(q: &str) -> Result<Node, ParseError> {
    let toks = lex(q)?;
    if toks.is_empty() {
        return Err(ParseError("empty query".into()));
    }
    let mut p = Parser { toks, i: 0 };
    let n = p.or()?;
    if p.i < p.toks.len() {
        return Err(ParseError(format!("unexpected {:?}", p.toks[p.i])));
    }
    Ok(n)
}

/// Does this query use the language, or is it plain words for full-text
/// search? Fields, brackets, Strong's numbers, operators and morphology
/// keywords all mean the former.
pub fn is_advanced(q: &str) -> bool {
    let toks = match lex(q) {
        Ok(t) => t,
        Err(_) => return true, // let the parser produce the message
    };
    toks.iter().any(|t| match t {
        Tok::Word(w) => {
            w.contains(':')
                || w.contains('|')
                || is_strongs(w)
                || keyword(&w.to_ascii_lowercase()).is_some()
        }
        Tok::Quoted(_) => false,
        _ => true,
    })
}

// ---------------------------------------------------------------------------
// Evaluation
// ---------------------------------------------------------------------------

/// A query compiled against one corpus: every string resolved to interned
/// ids or Strong's sets, so matching a word is a few integer comparisons.
pub struct Compiled {
    node: CNode,
}

#[derive(Clone)]
enum CNode {
    Slot(CSlot),
    Phrase(Vec<CSlot>),
    And(Vec<CNode>),
    Or(Vec<CNode>),
    Not(Box<CNode>),
    Prox { a: Box<CNode>, b: Box<CNode>, kind: Prox, dist: Option<usize> },
}

#[derive(Clone)]
struct CSlot {
    surfaces: Option<Vec<SurfaceMatch>>,
    lemmas: Option<HashSet<u32>>,
    strongs: Option<HashSet<u32>>,
    features: Vec<(Field, Vec<u8>)>,
    /// A constraint that resolved to nothing in this corpus (an unknown
    /// lemma, a gloss word no entry mentions): the slot can never match.
    dead: bool,
}

#[derive(Clone)]
enum SurfaceMatch {
    Exact(u32),
    Prefix(String),
}

pub struct Hit {
    pub verse: VerseId,
    /// Indices into `Corpus::words` of the words that matched.
    pub positions: Vec<usize>,
}

impl Compiled {
    pub fn new(node: &Node, corpus: &Corpus, lexicon: &Lexicon) -> Compiled {
        Compiled { node: compile(node, corpus, lexicon) }
    }

    /// Every verse that satisfies the query, in corpus (canonical) order.
    pub fn run(&self, corpus: &Corpus) -> Vec<Hit> {
        let mut hits = Vec::new();
        for span in &corpus.verses {
            if let Some(mut positions) = eval(&self.node, corpus, span) {
                positions.sort_unstable();
                positions.dedup();
                hits.push(Hit { verse: span.id, positions });
            }
        }
        hits
    }
}

fn compile(node: &Node, corpus: &Corpus, lex: &Lexicon) -> CNode {
    match node {
        Node::Slot(s) => CNode::Slot(compile_slot(s, corpus, lex)),
        Node::Phrase(ss) => CNode::Phrase(ss.iter().map(|s| compile_slot(s, corpus, lex)).collect()),
        Node::And(v) => CNode::And(v.iter().map(|n| compile(n, corpus, lex)).collect()),
        Node::Or(v) => CNode::Or(v.iter().map(|n| compile(n, corpus, lex)).collect()),
        Node::Not(n) => CNode::Not(Box::new(compile(n, corpus, lex))),
        Node::Prox { a, b, kind, dist } => CNode::Prox {
            a: Box::new(compile(a, corpus, lex)),
            b: Box::new(compile(b, corpus, lex)),
            kind: *kind,
            dist: *dist,
        },
    }
}

fn compile_slot(slot: &Slot, corpus: &Corpus, lex: &Lexicon) -> CSlot {
    let mut c = CSlot { surfaces: None, lemmas: None, strongs: None, features: Vec::new(), dead: false };
    let mut strongs_acc: Option<HashSet<u32>> = None;
    let add_strongs = |set: HashSet<u32>, acc: &mut Option<HashSet<u32>>, dead: &mut bool| {
        if set.is_empty() {
            *dead = true;
        }
        // several Strong's-bearing constraints on one slot intersect
        *acc = Some(match acc.take() {
            Some(prev) => prev.intersection(&set).copied().collect(),
            None => set,
        });
    };
    for k in &slot.constraints {
        match k {
            Constraint::Surface(alts) => {
                let ms: Vec<SurfaceMatch> = alts
                    .iter()
                    .filter_map(|a| {
                        if let Some(prefix) = a.strip_suffix('*') {
                            Some(SurfaceMatch::Prefix(prefix.to_string()))
                        } else {
                            corpus.surfaces.get(a).map(SurfaceMatch::Exact)
                        }
                    })
                    .collect();
                if ms.is_empty() {
                    c.dead = true;
                }
                c.surfaces = Some(ms);
            }
            Constraint::Lemma(alts) => {
                // the corpus's own lemma column where it has one (Greek),
                // and the lexicon's headwords via Strong's (Hebrew, or any
                // lemma the corpus spells differently)
                let ids: HashSet<u32> = alts.iter().filter_map(|a| corpus.lemmas.get(a)).collect();
                let via_lex: HashSet<u32> = alts
                    .iter()
                    .flat_map(|a| lex.strongs_for_lemma(a))
                    .filter_map(|s| corpus.strongs.get(&s))
                    .collect();
                if ids.is_empty() && via_lex.is_empty() {
                    c.dead = true;
                }
                if !ids.is_empty() {
                    c.lemmas = Some(ids);
                }
                if !via_lex.is_empty() {
                    // lemma-by-strongs is an alternative to lemma-by-id, so
                    // it is kept apart from explicit Strong's constraints
                    c.lemmas.get_or_insert_with(HashSet::new);
                    let mut s = strongs_acc.take();
                    add_strongs(via_lex, &mut s, &mut false);
                    strongs_acc = s;
                    // an explicit strongs constraint later will intersect
                }
            }
            Constraint::Strongs(alts) => {
                let set: HashSet<u32> = alts.iter().filter_map(|s| corpus.strongs.get(s)).collect();
                add_strongs(set, &mut strongs_acc, &mut c.dead);
            }
            Constraint::Gloss(terms) => {
                let set: HashSet<u32> = terms
                    .iter()
                    .flat_map(|t| lex.strongs_for_gloss(t))
                    .filter_map(|s| corpus.strongs.get(&s))
                    .collect();
                add_strongs(set, &mut strongs_acc, &mut c.dead);
            }
            Constraint::Feature(f, vals) => c.features.push((*f, vals.clone())),
        }
    }
    c.strongs = strongs_acc;
    c
}

fn slot_matches(slot: &CSlot, corpus: &Corpus, w: &Word) -> bool {
    if slot.dead {
        return false;
    }
    if let Some(ms) = &slot.surfaces {
        let ok = ms.iter().any(|m| match m {
            SurfaceMatch::Exact(id) => *id == w.surface,
            SurfaceMatch::Prefix(p) => corpus.surfaces.name(w.surface).starts_with(p.as_str()),
        });
        if !ok {
            return false;
        }
    }
    // lemma: by the corpus's lemma id, or by a Strong's the lexicon gave it
    let lemma_ok = match (&slot.lemmas, &slot.strongs) {
        (Some(ids), Some(set)) if !ids.is_empty() => ids.contains(&w.lemma) || set.contains(&w.strongs),
        (Some(ids), None) => ids.contains(&w.lemma),
        (_, Some(set)) => set.contains(&w.strongs),
        (None, None) => true,
    };
    if !lemma_ok {
        return false;
    }
    if slot.features.is_empty() {
        return true;
    }
    // all morphology constraints on one morpheme
    w.morphs.iter().any(|m| slot.features.iter().all(|(f, vals)| vals.contains(&m.get(*f))))
}

/// Positions in the verse matching a slot.
fn slot_positions(slot: &CSlot, corpus: &Corpus, span: &VerseSpan) -> Vec<usize> {
    (span.start..span.end).filter(|&i| slot_matches(slot, corpus, &corpus.words[i])).collect()
}

/// `None` = the verse does not satisfy the node; `Some(positions)` = it does,
/// with the words to mark (possibly none, e.g. under NOT).
fn eval(node: &CNode, corpus: &Corpus, span: &VerseSpan) -> Option<Vec<usize>> {
    match node {
        CNode::Slot(s) => {
            let p = slot_positions(s, corpus, span);
            if p.is_empty() { None } else { Some(p) }
        }
        CNode::Phrase(slots) => {
            let mut out = Vec::new();
            for start in span.start..span.end {
                if start + slots.len() > span.end {
                    break;
                }
                if slots.iter().enumerate().all(|(k, s)| slot_matches(s, corpus, &corpus.words[start + k])) {
                    out.extend(start..start + slots.len());
                }
            }
            if out.is_empty() { None } else { Some(out) }
        }
        CNode::And(parts) => {
            let mut all = Vec::new();
            for p in parts {
                all.extend(eval(p, corpus, span)?);
            }
            Some(all)
        }
        CNode::Or(parts) => {
            let mut any = false;
            let mut all = Vec::new();
            for p in parts {
                if let Some(v) = eval(p, corpus, span) {
                    any = true;
                    all.extend(v);
                }
            }
            if any { Some(all) } else { None }
        }
        CNode::Not(inner) => {
            if eval(inner, corpus, span).is_some() { None } else { Some(Vec::new()) }
        }
        CNode::Prox { a, b, kind, dist } => {
            let pa = eval(a, corpus, span)?;
            let pb = eval(b, corpus, span)?;
            let mut out = Vec::new();
            for &i in &pa {
                for &j in &pb {
                    let ok = match kind {
                        // A before B: 0 < j - i <= dist
                        Prox::Before => j > i && dist.is_none_or(|d| j - i <= d),
                        Prox::After => i > j && dist.is_none_or(|d| i - j <= d),
                        Prox::Near => i != j && dist.is_none_or(|d| i.abs_diff(j) <= d),
                    };
                    if ok {
                        out.push(i);
                        out.push(j);
                    }
                }
            }
            if out.is_empty() { None } else { Some(out) }
        }
    }
}

// ---------------------------------------------------------------------------
// Result rows
// ---------------------------------------------------------------------------

/// A snippet with matched words wrapped in `open`…`close`, turned into lines
/// of `(word, is_hit)` no wider than `budget` characters, for rendering as
/// highlighted chips. Right-to-left text (Hebrew) is detected from the script
/// and its lines reversed into reading order; the flag is returned too.
pub fn hit_lines(snippet: &str, open: char, close: char, budget: usize) -> (Vec<Vec<(String, bool)>>, bool) {
    let rtl = snippet.chars().any(|c| ('\u{0590}'..='\u{05FF}').contains(&c));
    let mut lines: Vec<Vec<(String, bool)>> = Vec::new();
    let mut cur: Vec<(String, bool)> = Vec::new();
    let mut width = 0usize;
    for tok in snippet.split_whitespace() {
        // punctuation outside the markers stays in the same chip as its word
        let hit = tok.contains(open) || tok.contains(close);
        let text: String = tok.chars().filter(|&c| c != open && c != close).collect();
        if text.is_empty() {
            continue;
        }
        let w = text.chars().count() + 1;
        if width + w > budget && !cur.is_empty() {
            lines.push(finish_line(std::mem::take(&mut cur), rtl));
            width = 0;
        }
        cur.push((text, hit));
        width += w;
    }
    if !cur.is_empty() {
        lines.push(finish_line(cur, rtl));
    }
    (lines, rtl)
}

fn finish_line(mut words: Vec<(String, bool)>, rtl: bool) -> Vec<(String, bool)> {
    if rtl {
        words.reverse(); // right-to-left reading order within the line
    }
    words
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn w(book: i64, ch: i64, v: i64, text: &str, lemma: &str, strongs: &str, morph: &str) -> RawWord {
        RawWord {
            book,
            chapter: ch,
            verse: v,
            text: text.into(),
            lemma: lemma.into(),
            strongs: strongs.into(),
            morph: morph.into(),
        }
    }

    /// A toy Greek corpus: two verses.
    fn greek() -> Corpus {
        Corpus::build(
            vec![
                // John 3:16-ish: "οὕτως ἠγάπησεν ὁ θεὸς τὸν κόσμον"
                w(43, 3, 16, "οὕτως", "οὕτως", "G3779", "D- --------"),
                w(43, 3, 16, "ἠγάπησεν", "ἀγαπάω", "G25", "V- 3AAI-S--"),
                w(43, 3, 16, "ὁ", "ὁ", "G3588", "RA ----NSM-"),
                w(43, 3, 16, "θεὸς", "θεός", "G2316", "N- ----NSM-"),
                w(43, 3, 16, "τὸν", "ὁ", "G3588", "RA ----ASM-"),
                w(43, 3, 16, "κόσμον", "κόσμος", "G2889", "N- ----ASM-"),
                // 1 John 4:19-ish: "ἡμεῖς ἀγαπῶμεν" (present)
                w(62, 4, 19, "ἡμεῖς", "ἐγώ", "G1473", "RP 1---NP--"),
                w(62, 4, 19, "ἀγαπῶμεν", "ἀγαπάω", "G25", "V- 1PAI-P--"),
            ],
            false,
        )
    }

    fn lexicon() -> Lexicon {
        Lexicon::new(vec![
            ("G25".to_string(), "ἀγαπάω".to_string(), "to love; love".to_string()),
            ("G2316".to_string(), "θεός".to_string(), "God; a deity".to_string()),
            ("G2889".to_string(), "κόσμος".to_string(), "world; the universe".to_string()),
            ("H430".to_string(), "אֱלֹהִים".to_string(), "gods; God".to_string()),
        ])
    }

    fn run(q: &str, c: &Corpus) -> Vec<(i64, i64, i64, Vec<usize>)> {
        let node = parse(q).unwrap();
        Compiled::new(&node, c, &lexicon())
            .run(c)
            .into_iter()
            .map(|h| (h.verse.book, h.verse.chapter, h.verse.verse, h.positions))
            .collect()
    }

    #[test]
    fn folding() {
        assert_eq!(fold("Ἀγαπάω"), "αγαπαω");
        assert_eq!(fold("θεὸς"), "θεοσ");
        assert_eq!(fold("אֱלֹהִ֑ים"), "אלהים");
    }

    #[test]
    fn strongs_and_lemma_find_both_verses() {
        let c = greek();
        assert_eq!(run("G25", &c).len(), 2);
        assert_eq!(run("g0025", &c).len(), 2); // normalised
        assert_eq!(run("lemma:ἀγαπάω", &c).len(), 2);
        assert_eq!(run("lemma:αγαπαω", &c).len(), 2); // unaccented
        assert_eq!(run("gloss:love", &c).len(), 2);
    }

    #[test]
    fn morphology_narrows() {
        let c = greek();
        let aorist = run("[G25 aorist]", &c);
        assert_eq!(aorist.len(), 1);
        assert_eq!((aorist[0].0, aorist[0].1, aorist[0].2), (43, 3, 16));
        assert_eq!(aorist[0].3, vec![1]);
        assert_eq!(run("[lemma:ἀγαπάω tense:present]", &c).len(), 1);
        assert_eq!(run("[G25 tense:aorist|present]", &c).len(), 2);
        assert_eq!(run("[G25 person:3 singular]", &c).len(), 1);
        assert_eq!(run("[verb imperative]", &c).len(), 0);
    }

    #[test]
    fn booleans() {
        let c = greek();
        assert_eq!(run("G25 AND G2316", &c).len(), 1);
        assert_eq!(run("G25 G2316", &c).len(), 1); // juxtaposition = AND
        assert_eq!(run("G2316 OR G1473", &c).len(), 2);
        assert_eq!(run("G25 NOT G2316", &c).len(), 1);
        assert_eq!(run("(G2316 OR G1473) AND aorist", &c).len(), 1);
    }

    #[test]
    fn proximity_and_phrase() {
        let c = greek();
        // the aorist verb immediately before the article
        assert_eq!(run("[verb aorist] BEFORE/1 article", &c).len(), 1);
        assert_eq!(run("article BEFORE/1 [verb aorist]", &c).len(), 0);
        assert_eq!(run("[verb aorist] AFTER/1 G3779", &c).len(), 1);
        assert_eq!(run("G2316 NEAR/2 G25", &c).len(), 1);
        assert_eq!(run("G2889 NEAR/2 G25", &c).len(), 0);
        assert_eq!(run("G25 BEFORE G2889", &c).len(), 1); // anywhere before
        assert_eq!(run("\"ὁ θεὸς\"", &c).len(), 1);
        assert_eq!(run("\"θεὸς ὁ\"", &c).len(), 0);
        assert_eq!(run("θε*", &c).len(), 1); // prefix
    }

    #[test]
    fn hebrew_features() {
        let c = Corpus::build(
            vec![
                w(1, 1, 1, "בְּרֵאשִׁ֖ית", "", "H7225", "HR/Ncfsa"),
                w(1, 1, 1, "בָּרָ֣א", "", "H1254", "HVqp3ms"),
                w(1, 1, 1, "אֱלֹהִ֑ים", "", "H430", "HNcmpa"),
                w(1, 1, 3, "וַיֹּ֥אמֶר", "", "H559", "HC/Vqw3ms"),
            ],
            true,
        );
        assert_eq!(run("[verb qal perfect]", &c).len(), 1);
        assert_eq!(run("[verb wayyiqtol]", &c).len(), 1);
        assert_eq!(run("[noun plural]", &c).len(), 1);
        assert_eq!(run("[preposition] BEFORE/1 [noun feminine]", &c).len(), 0); // same word, different morphemes
        assert_eq!(run("lemma:אלהים", &c).len(), 1); // via the lexicon
        assert_eq!(run("[conjunction verb]", &c).len(), 0); // constraints must share a morpheme
        assert_eq!(run("conjunction AND verb", &c).len(), 1); // only 1:3 has both (in one word)
    }

    #[test]
    fn parse_errors_are_messages() {
        assert!(parse("[").is_err());
        assert!(parse("tense:bogus").is_err());
        assert!(parse("nothing:here").is_err());
        assert!(parse("G25 NEAR/x G26").is_err());
        assert!(parse("").is_err());
    }

    #[test]
    fn precedence_and_spelling() {
        let c = greek();
        // proximity binds tighter than AND; AND tighter than OR:
        // G1473 OR (G2316 AND G2889) = both verses; (G1473 OR G2316) AND G2889 = one
        assert_eq!(run("G1473 OR G2316 AND G2889", &c).len(), 2);
        assert_eq!(run("(G1473 OR G2316) AND G2889", &c).len(), 1);
        assert_eq!(run("G1473 OR [verb aorist] BEFORE/1 article", &c).len(), 2);
        // operator spellings
        assert_eq!(run("G25 && G2316", &c).len(), 1);
        assert_eq!(run("G25 -G2316", &c).len(), 1);
        assert_eq!(run("G2316 WITHIN/2 G25", &c).len(), 1);
        assert_eq!(run("G2316 near/2 G25", &c).len(), 1); // case-insensitive
        // typographic quotes are quotes too
        assert_eq!(run("“ὁ θεὸς”", &c).len(), 1);
        assert_eq!(run("«ὁ θεὸς»", &c).len(), 1);
        // Strong's spellings normalise
        assert_eq!(parse("g0025").unwrap(), parse("G25").unwrap());
        assert_eq!(parse("strongs:G25").unwrap(), parse("G25").unwrap());
    }

    #[test]
    fn morphology_decoders() {
        // Greek: verb, participle, and a bare conjunction
        let v = greek_morph("V- 3AAI-S--")[0];
        assert_eq!((v.pos, v.person, v.tense, v.voice, v.mood, v.number), (1, 3, 4, 1, 1, 1));
        let p = greek_morph("V- -PMPGSF-")[0];
        assert_eq!((p.mood, p.voice, p.case, p.gender), (6, 2, 2, 2));
        assert_eq!(greek_morph("C- --------")[0].pos, 7);
        assert_eq!(greek_morph("A- ----NSFC")[0].degree, 1);
        // Hebrew: each morpheme separately, prefix letter stripped, Aramaic too
        let segs = hebrew_morph("HC/Vqw3ms");
        assert_eq!(segs.len(), 2);
        assert_eq!(segs[0].pos, 7);
        assert_eq!((segs[1].pos, segs[1].stem, segs[1].tense, segs[1].person), (1, 1, 7, 3));
        let part = hebrew_morph("HVqrmsa")[0];
        assert_eq!((part.mood, part.voice, part.gender, part.number, part.state), (6, 1, 1, 1, 1));
        let inf = hebrew_morph("HR/Vqc")[1];
        assert_eq!((inf.mood, inf.tense), (5, 12));
        assert_eq!(hebrew_morph("ANp")[0].pos, 11); // Aramaic proper noun
        assert_eq!(hebrew_morph("HTd/Ncmpa")[0].pos, 4); // article
        assert_eq!(hebrew_morph("HSp3ms")[0].pos, 12); // suffix
    }

    #[test]
    fn hit_lines_split_and_wrap() {
        let (lines, rtl) = hit_lines("In the \u{E000}beginning\u{E001} God created", '\u{E000}', '\u{E001}', 100);
        assert!(!rtl);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0][2], ("beginning".to_string(), true));
        assert_eq!(lines[0][3], ("God".to_string(), false));
        // punctuation outside the markers stays attached to the hit
        let (lines, _) = hit_lines("sa: \u{E000}Gud\u{E001}: lys", '\u{E000}', '\u{E001}', 100);
        assert_eq!(lines[0][1], ("Gud:".to_string(), true));
        // real «» in the text are plain characters, not markers
        let (lines, _) = hit_lines("«Det skal bli lys!» sa \u{E000}Gud\u{E001}", '\u{E000}', '\u{E001}', 100);
        assert_eq!(lines[0].iter().filter(|(_, h)| *h).count(), 1);
        assert_eq!(lines[0][0], ("«Det".to_string(), false));
        // wraps at the budget
        let (lines, _) = hit_lines("a b c d e f", '\u{E000}', '\u{E001}', 5);
        assert_eq!(lines.len(), 3);
        // Hebrew is detected and each line reversed into reading order
        let (lines, rtl) = hit_lines("\u{E000}בְּרֵאשִׁית\u{E001} בָּרָא אֱלֹהִים", '\u{E000}', '\u{E001}', 100);
        assert!(rtl);
        assert!(lines[0][2].1); // the first word, now last
        assert!(!lines[0][0].1);
    }

    #[test]
    fn routing() {
        assert!(!is_advanced("love"));
        assert!(!is_advanced("\"in the beginning\""));
        assert!(is_advanced("G25"));
        assert!(is_advanced("lemma:ἀγαπάω"));
        assert!(is_advanced("love AND God"));
        assert!(is_advanced("[verb aorist]"));
        assert!(is_advanced("aorist"));
    }
}
