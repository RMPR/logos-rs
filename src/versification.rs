//! Matching verses across numbering traditions.
//!
//! Bibles do not all number their verses alike. The Hebrew Bible counts a
//! psalm's title as verse 1, so Psalm 3:1 in the KJV is 3:2 in the Hebrew;
//! Malachi 4 is Malachi 3:19–24 there; the Septuagint and the Vulgate have
//! their own orders again. Pairing verses by number alone pairs different
//! verses, and the word alignment learned from such pairs is wrong.
//!
//! STEPBible's TVTMS ("Translators Versification Traditions with Methodology
//! for Standardisation", CC BY 4.0, `data/tvtms.txt.gz`) records every such
//! difference as rows of: a verse as numbered in some tradition, the
//! *standard* (KJV-based) verse or verses it holds, and tests that identify
//! the tradition from the Bible itself — "Psa.3:9 is the last verse of its
//! chapter", "Gen.6:1 is longer than Gen.6:2". This module evaluates those
//! tests on a Bible's own verses, so any Bible, bundled or imported, is
//! resolved to standard references without being told its tradition.
//! Verses no row mentions are standard already.

use std::collections::{BTreeMap, HashMap};
use std::io::Read;
use std::sync::LazyLock;

/// (book, chapter, verse); verse 0 is a psalm's title.
pub type Ref = (i64, i64, i64);

/// The SIL book codes TVTMS uses, in canonical order (Genesis = 1).
const BOOKS: [&str; 66] = [
    "Gen", "Exo", "Lev", "Num", "Deu", "Jos", "Jdg", "Rut", "1Sa", "2Sa", "1Ki", "2Ki", "1Ch",
    "2Ch", "Ezr", "Neh", "Est", "Job", "Psa", "Pro", "Ecc", "Sng", "Isa", "Jer", "Lam", "Ezk",
    "Dan", "Hos", "Jol", "Amo", "Oba", "Jon", "Mic", "Nam", "Hab", "Zep", "Hag", "Zec", "Mal",
    "Mat", "Mrk", "Luk", "Jhn", "Act", "Rom", "1Co", "2Co", "Gal", "Eph", "Php", "Col", "1Th",
    "2Th", "1Ti", "2Ti", "Tit", "Phm", "Heb", "Jas", "1Pe", "2Pe", "1Jn", "2Jn", "3Jn", "Jud",
    "Rev",
];

fn book_number(code: &str) -> Option<i64> {
    BOOKS.iter().position(|b| *b == code).map(|i| i as i64 + 1)
}

/// One verse as a test sees it.
#[derive(Debug, Clone, Copy, PartialEq)]
enum TestRef {
    Verse(Ref),
    /// Something the verses of a database cannot show: a part of a verse
    /// ("Gen.6:1.2"), the text before a psalm's first verse, the lettered
    /// chapters of Greek Esther.
    Unknowable,
}

#[derive(Debug, Clone, PartialEq)]
enum Test {
    Exists(TestRef, bool),
    Last(TestRef),
    /// left (times `factor`) shorter than right, in words
    Shorter(TestRef, f64, TestRef),
    Longer(TestRef, f64, TestRef),
}

#[derive(Debug, Clone)]
struct Row {
    tradition: String,
    source: Ref,
    standard: Vec<Ref>,
    tests: Vec<Test>,
}

/// "Gen.32:1" → (1, 32, 1); "Psa.3:Title" → (19, 3, 0). Books outside the
/// 66 and anything else give None.
fn parse_ref(s: &str) -> Option<Ref> {
    let (book, rest) = s.trim().split_once('.')?;
    let book = book_number(book)?;
    let (chapter, verse) = rest.split_once(':')?;
    let chapter = chapter.parse().ok()?;
    let verse = verse.split('!').next()?;
    let verse = if verse == "Title" { 0 } else { verse.parse().ok()? };
    Some((book, chapter, verse))
}

fn parse_test_ref(s: &str) -> Option<TestRef> {
    let s = s.trim();
    let (book, rest) = s.split_once('.')?;
    // a ref in a book we do not have (the Apocrypha) cannot be tested
    if book_number(book).is_none() {
        return Some(TestRef::Unknowable);
    }
    let (chapter, verse) = rest.split_once(':')?;
    if verse.contains('.') || verse.parse::<i64>().is_err() || chapter.parse::<i64>().is_err() {
        return Some(TestRef::Unknowable);
    }
    parse_ref(s).map(TestRef::Verse)
}

/// "Gen.5:32; 6:1", "Gen.2:25-3:1", "Psa.3:2-8" → each standard verse.
/// A range across chapters ends at the given verse of the later chapter;
/// its first chapter runs to `last_in` of it.
fn parse_standard(s: &str) -> Vec<Ref> {
    let mut out = Vec::new();
    let mut book = 0;
    let mut chapter = 0;
    for part in s.split(';') {
        let part = part.trim();
        let part = match part.split_once('.') {
            Some((b, rest)) if book_number(b).is_some() => {
                book = book_number(b).unwrap();
                rest
            }
            _ => part,
        };
        if book == 0 {
            return Vec::new();
        }
        let (from, to) = part.split_once('-').map_or((part, None), |(a, b)| (a, Some(b)));
        let point = |p: &str, chapter: i64| -> Option<(i64, i64)> {
            match p.split_once(':') {
                Some((c, v)) => {
                    let v = v.split('!').next()?;
                    let v = if v == "Title" { 0 } else { v.parse().ok()? };
                    Some((c.parse().ok()?, v))
                }
                None => Some((chapter, p.split('!').next()?.parse().ok()?)),
            }
        };
        let Some((c1, v1)) = point(from, chapter) else { continue };
        chapter = c1;
        match to.and_then(|t| point(t, chapter)) {
            Some((c2, v2)) if c2 == c1 => out.extend((v1..=v2).map(|v| (book, c1, v))),
            Some((c2, v2)) => {
                // across a chapter boundary: to the end of the first (at most
                // 176 verses, Psalm 119), then from 1 in the second
                out.extend((v1..=176).map(|v| (book, c1, v)));
                out.extend((1..=v2).map(|v| (book, c2, v)));
                chapter = c2;
            }
            None => out.push((book, c1, v1)),
        }
    }
    out
}

fn parse_tests(s: &str) -> Vec<Test> {
    let mut tests = Vec::new();
    for t in s.split('&').map(str::trim).filter(|t| !t.is_empty()) {
        let test = if let Some((r, what)) = t.split_once('=') {
            let r = parse_test_ref(r).unwrap_or(TestRef::Unknowable);
            match what.trim() {
                "Exist" => Test::Exists(r, true),
                "NotExist" => Test::Exists(r, false),
                "Last" => Test::Last(r),
                _ => continue,
            }
        } else if let Some((a, b)) = t.split_once('<').or_else(|| t.split_once('>')) {
            let shorter = t.contains('<');
            let (a, factor) = match a.split_once('*') {
                Some((a, f)) => (a, f.trim().parse().unwrap_or(1.0)),
                None => (a, 1.0),
            };
            let a = parse_test_ref(a).unwrap_or(TestRef::Unknowable);
            let b = parse_test_ref(b).unwrap_or(TestRef::Unknowable);
            if shorter { Test::Shorter(a, factor, b) } else { Test::Longer(a, factor, b) }
        } else {
            continue;
        };
        tests.push(test);
    }
    tests
}

fn parse(text: &str) -> Vec<Row> {
    let mut rows = Vec::new();
    let mut inside = false;
    for line in text.lines() {
        if line.starts_with("#DataStart(Expanded)") {
            inside = true;
            continue;
        }
        if line.starts_with("#DataEnd(Expanded)") {
            break;
        }
        if !inside || line.starts_with(['#', '\'']) {
            continue;
        }
        let cols: Vec<&str> = line.split('\t').collect();
        if cols.len() < 9 || cols[0].trim().is_empty() || cols[0] == "SourceType" {
            continue;
        }
        let Some(source) = parse_ref(cols[1]) else { continue };
        let standard = parse_standard(cols[2]);
        if standard.is_empty() {
            continue;
        }
        rows.push(Row { tradition: cols[0].trim().to_string(), source, standard, tests: parse_tests(cols[8]) });
    }
    rows
}

static RULES: LazyLock<Vec<Row>> = LazyLock::new(|| {
    let mut text = String::new();
    flate2::read::GzDecoder::new(&include_bytes!("../data/tvtms.txt.gz")[..])
        .read_to_string(&mut text)
        .expect("the vendored TVTMS table");
    parse(&text)
});

/// A Bible as the tests see it: which verses it has, how long they are.
pub struct Shape {
    /// verse → its length in words
    words: HashMap<Ref, usize>,
    last: HashMap<(i64, i64), i64>,
}

impl Shape {
    /// From each verse and its text.
    pub fn new<'a>(verses: impl IntoIterator<Item = (Ref, &'a str)>) -> Shape {
        Shape::from_lengths(verses.into_iter().map(|(r, text)| (r, text.split_whitespace().count())))
    }

    /// From each verse and its length in words.
    pub fn from_lengths(verses: impl IntoIterator<Item = (Ref, usize)>) -> Shape {
        let mut words = HashMap::new();
        let mut last: HashMap<(i64, i64), i64> = HashMap::new();
        for (r, n) in verses {
            if n == 0 {
                continue;
            }
            words.insert(r, n);
            let l = last.entry((r.0, r.1)).or_insert(0);
            *l = (*l).max(r.2);
        }
        Shape { words, last }
    }

    pub fn contains(&self, r: &Ref) -> bool {
        self.words.contains_key(r)
    }

    fn len(&self, r: TestRef) -> Option<usize> {
        match r {
            TestRef::Verse(r) => Some(self.words.get(&r).copied().unwrap_or(0)),
            TestRef::Unknowable => None,
        }
    }

    /// Some(true / false), or None when the Bible cannot tell.
    fn holds(&self, test: &Test) -> Option<bool> {
        match *test {
            Test::Exists(r, want) => self.len(r).map(|n| (n > 0) == want),
            Test::Last(TestRef::Verse(r)) => Some(self.last.get(&(r.0, r.1)) == Some(&r.2)),
            Test::Last(TestRef::Unknowable) => None,
            Test::Shorter(a, f, b) => Some((self.len(a)? as f64) * f < self.len(b)? as f64),
            Test::Longer(a, f, b) => Some((self.len(a)? as f64) * f > self.len(b)? as f64),
        }
    }

    /// The standard verses each verse of this Bible holds, for the verses
    /// where they differ from its own number.
    pub fn standard_refs(&self) -> HashMap<Ref, Vec<Ref>> {
        // Per verse of this Bible, the rows whose tests do not fail, scored
        // by how many tests they pass: a row that merely cannot be ruled out
        // loses to one the Bible positively matches.
        let mut best: HashMap<Ref, (usize, String, Vec<Ref>)> = HashMap::new();
        for row in RULES.iter() {
            if !self.words.contains_key(&row.source) && row.source.2 != 0 {
                continue;
            }
            let mut score = 0;
            let mut failed = false;
            for t in &row.tests {
                match self.holds(t) {
                    Some(true) => score += 1,
                    Some(false) => failed = true,
                    None => {}
                }
            }
            if failed {
                continue;
            }
            match best.get_mut(&row.source) {
                // the parts of one verse (!a, !b) in one tradition add up
                Some(b) if b.0 == score && b.1 == row.tradition => {
                    for s in &row.standard {
                        if !b.2.contains(s) {
                            b.2.push(*s);
                        }
                    }
                }
                Some(b) if b.0 >= score => {}
                _ => {
                    best.insert(row.source, (score, row.tradition.clone(), row.standard.clone()));
                }
            }
        }
        best.into_iter().filter(|(r, b)| b.2 != [*r]).map(|(r, b)| (r, b.2)).collect()
    }
}

/// For each verse of a translation, the verses of the originals that hold
/// the same text — by way of the standard reference both resolve to. Most
/// are the verse of the same number; some are another, a few are two
/// (where one tradition joins verses another divides), and some are none.
pub fn map_to_originals(translation: &Shape, originals: &Shape) -> BTreeMap<Ref, Vec<Ref>> {
    let ours = translation.standard_refs();
    let theirs = originals.standard_refs();
    let mut by_standard: HashMap<Ref, Vec<Ref>> = HashMap::new();
    for &r in originals.words.keys() {
        for s in theirs.get(&r).cloned().unwrap_or_else(|| vec![r]) {
            by_standard.entry(s).or_default().push(r);
        }
    }
    for v in by_standard.values_mut() {
        v.sort_unstable();
    }
    let mut out: BTreeMap<Ref, Vec<Ref>> = BTreeMap::new();
    for &r in translation.words.keys() {
        let mut found: Vec<Ref> = Vec::new();
        for s in ours.get(&r).cloned().unwrap_or_else(|| vec![r]) {
            // A verse holding only a psalm's title, where the originals keep
            // the title inside verse 1 (Psalm 11 in Hebrew), maps to that.
            let originals = by_standard.get(&s).or_else(|| if s.2 == 0 { by_standard.get(&(s.0, s.1, 1)) } else { None });
            for o in originals.into_iter().flatten() {
                if !found.contains(o) {
                    found.push(*o);
                }
            }
        }
        found.sort_unstable();
        out.insert(r, found);
    }
    // An original verse no verse of the translation holds was most likely
    // printed together with a neighbour: "9–10" as one verse, numbered 9 or
    // 10, or a psalm's title in verse 1. It goes to the neighbour — the
    // verse before it or the verse after it, in the same chapter — whose
    // length it explains, at the chapter's own rate of the translation's
    // words to the original's; to neither if it explains neither.
    let claimed: std::collections::HashSet<Ref> = out.values().flatten().copied().collect();
    let mut orphans: Vec<Ref> = originals.words.keys().filter(|o| !claimed.contains(o)).copied().collect();
    orphans.sort_unstable();
    let length = |refs: &[Ref]| refs.iter().filter_map(|r| originals.words.get(r)).sum::<usize>() as f64;
    let mut original_chapters: HashMap<(i64, i64), f64> = HashMap::new();
    for (r, n) in &originals.words {
        *original_chapters.entry((r.0, r.1)).or_default() += *n as f64;
    }
    for &o in &orphans {
        let chapter = (o.0, o.1, 0)..(o.0, o.1 + 1, 0);
        let previous = out.range(chapter.clone()).rev().find(|(_, held)| held.iter().any(|h| (h.0, h.1) == (o.0, o.1) && h.2 < o.2)).map(|(r, _)| *r);
        let next = out.range(chapter.clone()).find(|(_, held)| held.iter().any(|h| (h.0, h.1) == (o.0, o.1) && h.2 > o.2)).map(|(r, _)| *r);
        let ours: f64 = out.range(chapter).map(|(r, _)| translation.words.get(r).copied().unwrap_or(0) as f64).sum();
        let theirs = original_chapters.get(&(o.0, o.1)).copied().unwrap_or(0.0);
        if theirs == 0.0 {
            continue;
        }
        let ratio = ours / theirs;
        let extra = originals.words.get(&o).copied().unwrap_or(0) as f64 * ratio;
        let gain = |r: Ref| {
            let mine = translation.words.get(&r).copied().unwrap_or(0) as f64;
            let without = length(&out[&r]) * ratio;
            (mine - without).abs() - (mine - without - extra).abs()
        };
        let best = [previous, next]
            .into_iter()
            .flatten()
            .map(|r| (gain(r), r))
            .max_by(|a, b| a.0.total_cmp(&b.0));
        // it must account for most of its own length, not a sliver of it
        if let Some((g, r)) = best
            && g > 0.2 * extra
        {
            let held = out.get_mut(&r).unwrap();
            held.push(o);
            held.sort_unstable();
        }
    }
    out
}

/// A Bible's verses against the originals': which hold the same text,
/// whatever their numbers — the KJV's Psalm 3:1 is the Hebrew's 3:2, the
/// Septuagint's Psalm 10 the Hebrew's 11.
pub struct VerseMap {
    to_originals: BTreeMap<Ref, Vec<Ref>>,
    from_originals: HashMap<Ref, Vec<Ref>>,
}

impl VerseMap {
    pub fn new(bible: &Shape, originals: &Shape) -> VerseMap {
        let to_originals = map_to_originals(bible, originals);
        let mut from_originals: HashMap<Ref, Vec<Ref>> = HashMap::new();
        for (r, os) in &to_originals {
            for o in os {
                from_originals.entry(*o).or_default().push(*r);
            }
        }
        VerseMap { to_originals, from_originals }
    }

    /// The original verses holding verse `r` of the Bible.
    pub fn originals(&self, r: Ref) -> Vec<Ref> {
        self.to_originals.get(&r).cloned().unwrap_or_else(|| vec![r])
    }

    /// The Bible's verses holding any of `originals`.
    pub fn verses_for(&self, originals: &[Ref]) -> Vec<Ref> {
        let mut out: Vec<Ref> = originals.iter().filter_map(|o| self.from_originals.get(o)).flatten().copied().collect();
        out.sort_unstable();
        out.dedup();
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn references() {
        assert_eq!(parse_ref("Gen.32:1"), Some((1, 32, 1)));
        assert_eq!(parse_ref("Psa.3:Title"), Some((19, 3, 0)));
        assert_eq!(parse_ref("Gen.3:1!a"), Some((1, 3, 1)));
        assert_eq!(parse_ref("Tob.5:8"), None);
        assert_eq!(parse_standard("Gen.5:32; 6:1"), vec![(1, 5, 32), (1, 6, 1)]);
        assert_eq!(parse_standard("Psa.3:2-4"), vec![(19, 3, 2), (19, 3, 3), (19, 3, 4)]);
        assert_eq!(parse_test_ref("Gen.6:1.2"), Some(TestRef::Unknowable));
        assert_eq!(parse_test_ref("Psa.9:TextBeforeV1"), Some(TestRef::Unknowable));
        assert_eq!(parse_test_ref("Est.C:30"), Some(TestRef::Unknowable));
    }

    #[test]
    fn tests_parse() {
        let t = parse_tests("Gen.32:33=Last & Gen.6:1*2<Gen.6:2 & Gen.6:1.2=NotExist");
        assert_eq!(t.len(), 3);
        assert_eq!(t[0], Test::Last(TestRef::Verse((1, 32, 33))));
        assert_eq!(t[1], Test::Shorter(TestRef::Verse((1, 6, 1)), 2.0, TestRef::Verse((1, 6, 2))));
        assert_eq!(t[2], Test::Exists(TestRef::Unknowable, false));
    }

    #[test]
    fn the_table_loads() {
        assert!(RULES.len() > 15_000, "{} rows", RULES.len());
    }

    /// A Bible with `chapters` (chapter, number of verses), every verse
    /// three words long.
    fn shape(book: i64, chapters: &[(i64, i64)]) -> Shape {
        let text = "three words long";
        Shape::new(chapters.iter().flat_map(|&(c, n)| (1..=n).map(move |v| ((book, c, v), text))))
    }

    #[test]
    fn psalm_titles_shift_verses() {
        // Psalm 3: 8 verses in the KJV, 9 in the Hebrew (the title is 1)
        let kjv = shape(19, &[(3, 8)]);
        let hebrew = shape(19, &[(3, 9)]);
        let map = map_to_originals(&kjv, &hebrew);
        assert_eq!(map[&(19, 3, 1)], vec![(19, 3, 2)]);
        assert_eq!(map[&(19, 3, 8)], vec![(19, 3, 9)]);
        // a Bible numbered like the Hebrew maps one to one
        let map = map_to_originals(&hebrew, &hebrew);
        assert_eq!(map[&(19, 3, 1)], vec![(19, 3, 1)]);
    }

    #[test]
    fn a_title_printed_in_verse_1_is_found() {
        // the Hebrew: the title (6 words) is 3:1, then 8 verses of 8 words
        let hebrew = Shape::from_lengths((1..=9).map(|v| ((19, 3, v), if v == 1 { 6 } else { 8 })));
        // English Bibles of 8 verses, one printing the title in verse 1
        let with_title = Shape::from_lengths((1..=8).map(|v| ((19, 3, v), if v == 1 { 14 } else { 8 })));
        let without = Shape::from_lengths((1..=8).map(|v| ((19, 3, v), 8)));
        assert_eq!(map_to_originals(&with_title, &hebrew)[&(19, 3, 1)], vec![(19, 3, 1), (19, 3, 2)]);
        assert_eq!(map_to_originals(&without, &hebrew)[&(19, 3, 1)], vec![(19, 3, 2)]);
    }

    #[test]
    fn merged_verses_collect_their_originals() {
        // verse 12 printed together with 11, under 11
        // verse 12 printed together with 11, under 11
        let merged = Shape::from_lengths([((45, 9, 10), 4), ((45, 9, 11), 8), ((45, 9, 13), 4)]);
        let greek = Shape::from_lengths((10..=13).map(|v| ((45, 9, v), 4)));
        let map = map_to_originals(&merged, &greek);
        assert_eq!(map[&(45, 9, 11)], vec![(45, 9, 11), (45, 9, 12)]);
        assert_eq!(map[&(45, 9, 13)], vec![(45, 9, 13)]);
        // verse 9 printed together with 10, under 10 (as the BFC does)
        let merged = Shape::from_lengths([((1, 6, 8), 4), ((1, 6, 10), 8), ((1, 6, 11), 4)]);
        let hebrew = Shape::from_lengths((8..=11).map(|v| ((1, 6, v), 4)));
        let map = map_to_originals(&merged, &hebrew);
        assert_eq!(map[&(1, 6, 8)], vec![(1, 6, 8)]);
        assert_eq!(map[&(1, 6, 10)], vec![(1, 6, 9), (1, 6, 10)]);
    }

    #[test]
    fn parallel_verses_across_traditions() {
        // the Hebrew's Psalms 9 (21 verses) and 10 (18); the Greek joins
        // them as its Psalm 9 (39 verses), so its Psalm 10 is the Hebrew 11
        let hebrew = shape(19, &[(9, 21), (10, 18), (11, 7), (12, 9)]);
        let greek = shape(19, &[(9, 39), (10, 7), (11, 9)]);
        let kjv = shape(19, &[(9, 20), (10, 18), (11, 7), (12, 8)]);
        let greek = VerseMap::new(&greek, &hebrew);
        let kjv = VerseMap::new(&kjv, &hebrew);
        // KJV Psalm 11:1 → Hebrew 11:1 → the Greek's 10:1
        assert_eq!(greek.verses_for(&kjv.originals((19, 11, 1))), vec![(19, 10, 1)]);
        assert_eq!(greek.originals((19, 9, 22)), vec![(19, 10, 1)]);
    }

    #[test]
    fn chapter_boundaries_move() {
        // Malachi: 4 chapters in the KJV (3:18 + 4:6), 3 in the Hebrew (3:24)
        let kjv = shape(39, &[(1, 14), (2, 17), (3, 18), (4, 6)]);
        let hebrew = shape(39, &[(1, 14), (2, 17), (3, 24)]);
        let map = map_to_originals(&kjv, &hebrew);
        assert_eq!(map[&(39, 4, 1)], vec![(39, 3, 19)]);
        assert_eq!(map[&(39, 4, 6)], vec![(39, 3, 24)]);
        assert_eq!(map[&(39, 3, 18)], vec![(39, 3, 18)]);
    }
}
