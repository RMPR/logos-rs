//! Attaching Strong's numbers to an imported translation's words.
//!
//! A translation with no tagged edition can still become a reverse
//! interlinear: verse by verse, its words sit beside the Strong's numbers of
//! the same verse in the tagged Hebrew (WLC) and Greek (SBLGNT). Across the
//! whole Bible that parallel corpus is enough to learn which word translates
//! which lexeme. This is IBM Model 1, trained by expectation–maximisation: it
//! estimates P(word | Strong's), then gives each word the best-fitting
//! Strong's number *present in its own verse* — or none, when nothing fits
//! well enough. A port of `tools/build_strongs.py`, which built the
//! correspondences of the bundled Louis Segond, ESV, BFC and Bokmål texts;
//! the constants are the same.

use std::collections::HashMap;
use std::hash::{BuildHasherDefault, Hasher};

use super::text::Verses;

const EM_ITERATIONS: usize = 6;
/// Probabilities below this are dropped between iterations, bounding memory.
const PRUNE: f64 = 1e-3;
/// The least P(word | Strong's) at which a word is given the number.
const ASSIGN_MIN: f64 = 0.12;
/// Characters trimmed from a word before it is compared.
const PUNCT: &[char] = &[
    ' ', '\t', '\n', '.', ',', ';', ':', '!', '?', '«', '»', '"', '\'', '’', '“', '”', '(', ')', '[',
    ']', '—', '–', '-', '…', '·', '‘',
];

/// Hashing for keys that are already well-spread integers: the table is
/// probed hundreds of millions of times, and SipHash would dominate.
#[derive(Default)]
struct IntHasher(u64);

impl Hasher for IntHasher {
    fn finish(&self) -> u64 {
        self.0
    }
    fn write(&mut self, _: &[u8]) {
        unreachable!("only integer keys are hashed")
    }
    fn write_u64(&mut self, n: u64) {
        self.0 = n.wrapping_mul(0x9E37_79B9_7F4A_7C15);
    }
}

type IntMap<V> = HashMap<u64, V, BuildHasherDefault<IntHasher>>;

fn normalize(token: &str) -> String {
    token.trim_matches(PUNCT).to_lowercase()
}

/// Strong's numbers present in each verse of the tagged originals, as bare
/// numbers (the H/G prefix follows from the testament).
pub type VerseStrongs = HashMap<(i64, i64, i64), Vec<u32>>;

/// Per verse, the Strong's number (bare) given to each whitespace-separated
/// word of the verse's text, if any.
pub type Alignment = HashMap<(i64, i64, i64), Vec<Option<u32>>>;

/// Align `verses` to the originals. `progress` hears the fraction done.
pub fn align(verses: &Verses, originals: &VerseStrongs, progress: &mut dyn FnMut(f32)) -> Alignment {
    // Intern words; the Strong's numbers are already integers. Id 0 of the
    // sources is NULL, the "translates nothing" source every verse has.
    const NULL: u32 = 0;
    let mut words: HashMap<String, u32> = HashMap::new();
    let mut corpus: Vec<(Vec<u32>, Vec<u32>)> = Vec::new();
    let mut max_source = 0u32;
    for (key, text) in verses {
        let Some(sources) = originals.get(key) else { continue };
        let mut fr: Vec<u32> = text
            .split_whitespace()
            .map(normalize)
            .filter(|w| !w.is_empty())
            .map(|w| {
                let n = words.len() as u32;
                *words.entry(w).or_insert(n)
            })
            .collect();
        fr.sort_unstable();
        fr.dedup();
        if fr.is_empty() {
            continue;
        }
        let mut src: Vec<u32> = sources.iter().map(|s| s + 1).collect();
        src.push(NULL);
        src.sort_unstable();
        src.dedup();
        max_source = max_source.max(*src.last().unwrap());
        corpus.push((fr, src));
    }
    let distinct_sources = {
        let mut all: Vec<u32> = corpus.iter().flat_map(|(_, s)| s.iter().copied()).collect();
        all.sort_unstable();
        all.dedup();
        all.len().max(1)
    };
    let init = 1.0 / distinct_sources as f64;
    let key = |f: u32, s: u32| ((f as u64) << 32) | s as u64;

    // The (word, source) pairs that co-occur are fixed by the corpus, so
    // they are numbered once and EM runs over flat arrays: each verse becomes
    // its |words| × |sources| pair numbers, row by row.
    let mut pair_ids: IntMap<u32> = IntMap::default();
    let mut pair_source: Vec<u32> = Vec::new();
    let mut cells: Vec<Vec<u32>> = Vec::with_capacity(corpus.len());
    for (fr, src) in &corpus {
        let mut ids = Vec::with_capacity(fr.len() * src.len());
        for &f in fr {
            for &s in src {
                let next = pair_source.len() as u32;
                let id = *pair_ids.entry(key(f, s)).or_insert(next);
                if id == next {
                    pair_source.push(s);
                }
                ids.push(id);
            }
        }
        cells.push(ids);
    }

    // A pair pruned away counts at the uniform start value again — as in
    // build_strongs.py, whose results the bundled texts carry.
    let mut t = vec![init; pair_source.len()];
    for iteration in 0..EM_ITERATIONS {
        let mut count = vec![0f64; pair_source.len()];
        let mut total = vec![0f64; max_source as usize + 1];
        for ((_, src), ids) in corpus.iter().zip(&cells) {
            for row in ids.chunks(src.len()) {
                let denom: f64 = row.iter().map(|&id| t[id as usize]).sum();
                if denom <= 0.0 {
                    continue;
                }
                for &id in row {
                    let c = t[id as usize] / denom;
                    count[id as usize] += c;
                    total[pair_source[id as usize] as usize] += c;
                }
            }
        }
        for (id, c) in count.into_iter().enumerate() {
            let p = c / total[pair_source[id] as usize];
            t[id] = if p >= PRUNE { p } else { init };
        }
        progress((iteration + 1) as f32 / EM_ITERATIONS as f32);
    }
    let probability = |f: u32, s: u32| pair_ids.get(&key(f, s)).map_or(init, |&id| t[id as usize]);

    let mut out = Alignment::new();
    for (k, text) in verses {
        let sources = originals.get(k);
        let tags = text
            .split_whitespace()
            .map(|token| {
                let w = normalize(token);
                let (sources, &f) = (sources?, words.get(&w)?);
                let mut best = None;
                let mut best_p = ASSIGN_MIN;
                for &s in sources {
                    let p = probability(f, s + 1);
                    if p > best_p {
                        (best, best_p) = (Some(s), p);
                    }
                }
                best
            })
            .collect();
        out.insert(*k, tags);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn learns_which_word_translates_which_number() {
        // "God" always beside 430, "said" beside 559, "light" beside 216.
        let mut verses = Verses::new();
        let mut originals = VerseStrongs::new();
        let lines = [
            ("God said light", vec![430, 559, 216]),
            ("God said", vec![430, 559]),
            ("God saw the light", vec![430, 7200, 216]),
            ("the light was good", vec![216, 2896]),
            ("God saw", vec![430, 7200]),
        ];
        for (i, (text, strongs)) in lines.iter().enumerate() {
            verses.insert((1, 1, i as i64 + 1), text.to_string());
            originals.insert((1, 1, i as i64 + 1), strongs.clone());
        }
        let a = align(&verses, &originals, &mut |_| {});
        assert_eq!(a[&(1, 1, 1)], vec![Some(430), Some(559), Some(216)]);
        assert_eq!(a[&(1, 1, 3)][0], Some(430));
        assert_eq!(a[&(1, 1, 3)][3], Some(216));
    }
}
