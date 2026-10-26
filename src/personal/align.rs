//! Attaching Strong's numbers to a translation's words.
//!
//! A translation with no tagged edition can still become a reverse
//! interlinear: verse by verse, its words sit beside the words of the same
//! verse in the tagged Hebrew (WLC) and Greek (SBLGNT), each carrying a
//! Strong's number. Across the whole Bible that parallel corpus is enough to
//! learn which word translates which lexeme.
//!
//! The model is that of `fast_align` (Dyer, Chahuneau & Smith, 2013), a
//! reparameterised IBM Model 2, trained in both directions:
//!
//! - **Words are compared by stem**, with the Snowball stemmer of the
//!   translation's language: "sønn", "sønnen", "sønner" and "sønnene" pool
//!   their evidence instead of each being learned — or not — on its own.
//! - **Word order counts**: a word is likelier to translate an original word
//!   at about the same relative place in the verse (the "diagonal" prior).
//!   Plain IBM Model 1 has no notion of order at all.
//! - **Sparse, variational-Bayes estimates**: a rare word does not soak up a
//!   rare name in its verse merely because both are rare — Model 1's
//!   "garbage collector" effect, which tied "sønnen" to the names of the
//!   genealogies.
//! - **Both directions must agree**: one model reads the translation as
//!   generated from the original, the other the reverse, and a word is
//!   linked only where the product of their posteriors is high.
//! - Hebrew and Greek numbers are kept apart: H430 (God) and G430 (to bear
//!   with) are different words.

use std::collections::HashMap;
use std::hash::{BuildHasherDefault, Hasher};

use rust_stemmers::{Algorithm, Stemmer};

use super::greek::GreekLexemes;
use super::text::Verses;
use crate::versification::{Shape, map_to_originals};

fn iterations() -> usize { std::env::var("ITERATIONS").ok().and_then(|v| v.parse().ok()).unwrap_or(5) }
/// The prior probability that a word translates nothing in the other text.
fn p_null() -> f64 { std::env::var("P_NULL").ok().and_then(|v| v.parse().ok()).unwrap_or(0.08) }
/// How strongly words keep to the diagonal (fast_align's default tension).
fn tension() -> f64 { std::env::var("TENSION").ok().and_then(|v| v.parse().ok()).unwrap_or(12.0) }
/// The Dirichlet concentration of the variational-Bayes estimates.
fn alpha() -> f64 { std::env::var("ALPHA").ok().and_then(|v| v.parse().ok()).unwrap_or(0.001) }
/// The least agreement (geometric mean of the two directions' posteriors) at
/// which a word is linked.
fn link_min() -> f64 { std::env::var("LINK_MIN").ok().and_then(|v| v.parse().ok()).unwrap_or(0.2) }

/// Characters trimmed from a word before it is compared.
const PUNCT: &[char] = &[
    ' ', '\t', '\n', '.', ',', ';', ':', '!', '?', '«', '»', '"', '\'', '’', '“', '”', '(', ')', '[',
    ']', '—', '–', '-', '…', '·', '‘', '{', '}', '*', '\u{a0}',
];

/// Hashing for keys that are already well-spread integers.
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

/// The Strong's numbers of each verse of the tagged originals, bare (the
/// H/G prefix follows from the testament), in the order of the words.
pub type VerseStrongs = HashMap<(i64, i64, i64), Vec<u32>>;

/// The original-language text as the aligner needs it: each verse's Strong's
/// numbers in order, and the verses' shape for matching numbering.
pub struct Originals {
    pub strongs: VerseStrongs,
    pub shape: Shape,
    /// The Greek lexemes the originals and the lexicon know, to name the
    /// words of a Greek text that has no original (the Septuagint's own).
    pub greek: Option<GreekLexemes>,
}

impl Originals {
    pub fn new(strongs: VerseStrongs) -> Originals {
        let shape = Shape::from_lengths(strongs.iter().map(|(k, v)| (*k, v.len())));
        Originals { strongs, shape, greek: None }
    }
}

/// A Strong's number with its language: H1121 (בֵּן, son), G5207 (υἱός).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Strong {
    pub hebrew: bool,
    pub number: u32,
}

impl Strong {
    pub fn hebrew(number: u32) -> Strong {
        Strong { hebrew: true, number }
    }
    pub fn greek(number: u32) -> Strong {
        Strong { hebrew: false, number }
    }
    /// The number of a verse's original word: Hebrew in the Old Testament.
    pub fn of_book(book: i64, number: u32) -> Strong {
        Strong { hebrew: book < 40, number }
    }
}

impl std::fmt::Display for Strong {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}{}", if self.hebrew { 'H' } else { 'G' }, self.number)
    }
}

/// Per verse, the Strong's number given to each word of the verse (as
/// [`super::words_of`] counts them), if any.
pub type Alignment = HashMap<(i64, i64, i64), Vec<Option<Strong>>>;

/// How the words of a language are reduced to their stems.
enum Stemming {
    Snowball(Stemmer),
    Latin,
    Greek,
    None,
}

fn stemmer(lang: &str) -> Stemming {
    let algorithm = match lang {
        "la" => return Stemming::Latin,
        // Snowball's Greek is Modern Greek: it leaves λόγον whole
        "grc" | "el" => return Stemming::Greek,
        "en" => Algorithm::English,
        "fr" => Algorithm::French,
        "nb" | "nn" | "no" => Algorithm::Norwegian,
        "de" => Algorithm::German,
        "es" => Algorithm::Spanish,
        "pt" => Algorithm::Portuguese,
        "it" => Algorithm::Italian,
        "nl" => Algorithm::Dutch,
        "sv" => Algorithm::Swedish,
        "da" => Algorithm::Danish,
        "fi" => Algorithm::Finnish,
        "ro" => Algorithm::Romanian,
        "ru" => Algorithm::Russian,
        "hu" => Algorithm::Hungarian,
        "tr" => Algorithm::Turkish,
        "ar" => Algorithm::Arabic,
        _ => return Stemming::None,
    };
    Stemming::Snowball(Stemmer::create(algorithm))
}

/// Latin words that end in -que without it being the enclitic "and".
const QUE_WORDS: &[&str] = &[
    "atque", "quoque", "neque", "itaque", "absque", "apsque", "abusque", "adaeque", "adusque",
    "denique", "deque", "susque", "oblique", "peraeque", "plenisque", "quandoque", "quisque",
    "quaeque", "cuiusque", "cuique", "quemque", "quamque", "quaque", "quique", "quorumque",
    "quarumque", "quibusque", "quosque", "quasque", "quotusquisque", "quousque", "ubique",
    "undique", "usque", "uterque", "utique", "utroque", "utribique", "torque", "coque",
    "concoque", "contorque", "detorque", "decoque", "excoque", "extorque", "obtorque", "optorque",
    "retorque", "recoque", "attorque", "incoque", "intorque", "praetorque",
];

/// The Schinke Latin stemmer (Schinke et al., 1996), noun form: j → i and
/// v → u, the enclitic -que dropped, then the longest case ending, as long as
/// two letters are left. Schinke's -ius is left out: it parts "filius" from
/// "filii" and "filium", where -us keeps the three together.
fn latin_stem(word: &str) -> String {
    let w: String = word
        .chars()
        .map(|c| match c {
            'j' => 'i',
            'v' => 'u',
            c => c,
        })
        .collect::<String>()
        .replace('æ', "ae")
        .replace('œ', "oe");
    if QUE_WORDS.contains(&w.as_str()) {
        return w;
    }
    let w = w.strip_suffix("que").map_or(w.clone(), str::to_string);
    for ending in ["ibus", "ae", "am", "as", "em", "es", "ia", "is", "nt", "os", "ud", "um", "us", "a", "e", "i", "o", "u"] {
        if let Some(stem) = w.strip_suffix(ending)
            && stem.chars().count() >= 2
        {
            return stem.to_string();
        }
    }
    w
}

/// Koine endings, longest first: the cases of nouns, adjectives and
/// participles, and the commonest verb endings.
const GREEK_ENDINGS: &[&str] = &[
    "ομενους", "ομενοις", "ομενων", "ομενος", "ομενοι", "ομενον", "ομενην", "ομενη", "ουσιν",
    "οντες", "οντος", "οντων", "οντας", "θησαν", "σαντες", "μενος", "μενοι", "μενον", "ησαν",
    "ουσι", "οντα", "οντι", "εσθαι", "σεται", "σουσιν", "σουσι", "σομαι", "σεις", "σει", "σαν",
    "σας", "σεν", "ομεν", "ετε", "ειν", "εις", "ουν", "ους", "οις", "αις", "ων", "ου", "ον", "ος",
    "οι", "ας", "ης", "ην", "αι", "ες", "εν", "ει", "ω", "α", "η", "ε", "ο", "ι", "υ",
];

/// A light Koine stemmer: the longest ending off, two letters left at
/// least (θε-ός). Rough, but it lets λόγος, λόγου and λόγον pool their evidence.
pub(crate) fn greek_stem(plain: &str) -> String {
    for ending in GREEK_ENDINGS {
        if let Some(stem) = plain.strip_suffix(ending)
            && stem.chars().count() >= 2
        {
            return stem.to_string();
        }
    }
    plain.to_string()
}

/// Greek without accents and breathings: the forms a stemmer can compare.
pub(crate) fn plain_greek(word: &str) -> String {
    use unicode_normalization::UnicodeNormalization;
    word.nfd()
        .filter(|c| !unicode_normalization::char::is_combining_mark(*c))
        .collect()
}

/// The form a word is learned under: trimmed, lower-cased and stemmed.
fn key(token: &str, stemming: &Stemming) -> String {
    let w = token.trim_matches(PUNCT).to_lowercase();
    if w.is_empty() {
        return w;
    }
    match stemming {
        Stemming::Snowball(s) => s.stem(&w).into_owned(),
        Stemming::Latin => latin_stem(&w),
        Stemming::Greek => greek_stem(&plain_greek(&w)),
        Stemming::None => w,
    }
}

/// ψ, the digamma function (asymptotic series after shifting x above 6).
fn digamma(mut x: f64) -> f64 {
    let mut r = 0.0;
    while x < 6.0 {
        r -= 1.0 / x;
        x += 1.0;
    }
    let f = 1.0 / (x * x);
    r + x.ln() - 0.5 / x - f * (1.0 / 12.0 - f * (1.0 / 120.0 - f * (1.0 / 252.0 - f * (1.0 / 240.0 - f / 132.0))))
}

/// The diagonal prior over the n positions of the other side for position
/// `i` (1-based) of m: proportional to exp(-tension·|i/m − j/n|), summing to
/// 1 − p_null(). Cached by (m, n): the same shapes recur all through a Bible.
struct Diagonal(HashMap<(usize, usize), Vec<f64>>);

impl Diagonal {
    fn row(&mut self, m: usize, n: usize, i: usize) -> &[f64] {
        let table = self.0.entry((m, n)).or_insert_with(|| {
            let mut t = Vec::with_capacity(m * n);
            for i in 1..=m {
                let w: Vec<f64> = (1..=n)
                    .map(|j| (-tension() * (i as f64 / m as f64 - j as f64 / n as f64).abs()).exp())
                    .collect();
                let z: f64 = w.iter().sum();
                t.extend(w.into_iter().map(|x| (1.0 - p_null()) * x / z));
            }
            t
        });
        &table[(i - 1) * n..i * n]
    }
}

/// One direction of the model: P(output word | input word), with input id 0
/// standing for NULL.
struct Direction {
    pair_input: Vec<u32>,
    t: Vec<f64>,
    /// per verse, per output position, the pair ids of its input positions
    /// (NULL first)
    cells: Vec<Vec<u32>>,
    shapes: Vec<(usize, usize)>,
}

impl Direction {
    /// `corpus`: per verse (output ids, input ids); ids are ≥ 1.
    fn new(corpus: &[(&[u32], &[u32])]) -> Direction {
        let mut pair: IntMap<u32> = IntMap::default();
        let mut pair_input: Vec<u32> = Vec::new();
        let mut cells = Vec::with_capacity(corpus.len());
        for (out, inp) in corpus {
            let mut ids = Vec::with_capacity(out.len() * (inp.len() + 1));
            for &o in *out {
                for &i in std::iter::once(&0).chain(inp.iter()) {
                    let next = pair_input.len() as u32;
                    let id = *pair.entry(((o as u64) << 32) | i as u64).or_insert(next);
                    if id == next {
                        pair_input.push(i);
                    }
                    ids.push(id);
                }
            }
            cells.push(ids);
        }
        let t = vec![1.0; pair_input.len()];
        let shapes = corpus.iter().map(|(o, i)| (o.len(), i.len())).collect();
        Direction { pair_input, t, cells, shapes }
    }

    /// Posterior over input positions (NULL first) for each output word of
    /// verse `v`, written into `post` row by row.
    fn posteriors(&self, v: usize, diag: &mut Diagonal, post: &mut Vec<f64>) {
        let (m, n) = self.shapes[v];
        post.clear();
        let ids = &self.cells[v];
        for i in 0..m {
            let row = &ids[i * (n + 1)..(i + 1) * (n + 1)];
            let prior = diag.row(m, n, i + 1);
            let start = post.len();
            post.push(p_null() * self.t[row[0] as usize]);
            for j in 0..n {
                post.push(prior[j] * self.t[row[j + 1] as usize]);
            }
            let z: f64 = post[start..].iter().sum();
            if z > 0.0 {
                for p in &mut post[start..] {
                    *p /= z;
                }
            }
        }
    }

    /// One round of expectation–maximisation; `inputs` is the largest input
    /// id, `outputs` the number of output word types.
    fn train(&mut self, inputs: usize, outputs: usize, diag: &mut Diagonal) {
        let mut count = vec![0f64; self.t.len()];
        let mut post = Vec::new();
        for v in 0..self.cells.len() {
            self.posteriors(v, diag, &mut post);
            for (id, p) in self.cells[v].iter().zip(&post) {
                count[*id as usize] += p;
            }
        }
        let mut total = vec![0f64; inputs + 1];
        for (id, c) in count.iter().enumerate() {
            total[self.pair_input[id] as usize] += c;
        }
        // variational Bayes: exp ψ(c + α) / exp ψ(total + α·V)
        let norm: Vec<f64> = total.iter().map(|&t| digamma(t + alpha() * outputs as f64)).collect();
        for (id, c) in count.into_iter().enumerate() {
            self.t[id] = (digamma(c + alpha()) - norm[self.pair_input[id] as usize]).exp();
        }
    }
}

/// Align the words of `verses`, a translation in language `lang`, to the
/// originals. `progress` hears the fraction done.
pub fn align(verses: &Verses, originals: &Originals, lang: &str, progress: &mut dyn FnMut(f32)) -> Alignment {
    let stemmer = stemmer(lang);
    // Each verse is paired with the original verses that hold its text,
    // whatever their numbers (see crate::versification).
    let mapping = map_to_originals(&Shape::new(verses.iter().map(|(k, v)| (*k, v.as_str()))), &originals.shape);
    let mut words: HashMap<String, u32> = HashMap::new();
    // Per verse with an original: its key, the ids of its tokens (0 for one
    // that is only punctuation), and the ids of the original words. Ids
    // start at 1; 0 is NULL.
    let mut keys = Vec::new();
    let mut targets: Vec<Vec<u32>> = Vec::new();
    let mut sources: Vec<Vec<u32>> = Vec::new();
    for (k, text) in verses {
        let strongs: Vec<u32> = mapping
            .get(k)
            .into_iter()
            .flatten()
            .filter_map(|o| originals.strongs.get(o))
            .flatten()
            .copied()
            .collect();
        if strongs.is_empty() {
            continue;
        }
        let ids: Vec<u32> = super::words_of(text)
            .iter()
            .map(|tok| {
                let w = key(tok, &stemmer);
                if w.is_empty() {
                    return 0;
                }
                let n = words.len() as u32 + 1;
                *words.entry(w).or_insert(n)
            })
            .collect();
        // Hebrew and Greek numbers apart: H430 is not G430
        let nt = (k.0 >= 40) as u32;
        keys.push(*k);
        targets.push(ids);
        sources.push(strongs.iter().map(|&s| s * 2 + nt + 1).collect());
    }
    let max_source = sources.iter().flatten().copied().max().unwrap_or(0) as usize;
    let vocabulary = words.len();

    let content: Vec<Vec<u32>> = targets.iter().map(|t| t.iter().copied().filter(|&w| w > 0).collect()).collect();
    let forward_corpus: Vec<(&[u32], &[u32])> =
        content.iter().zip(&sources).map(|(t, s)| (t.as_slice(), s.as_slice())).collect();
    let backward_corpus: Vec<(&[u32], &[u32])> =
        content.iter().zip(&sources).map(|(t, s)| (s.as_slice(), t.as_slice())).collect();
    let mut forward = Direction::new(&forward_corpus); // translation | original
    let mut backward = Direction::new(&backward_corpus); // original | translation
    drop((forward_corpus, backward_corpus));
    let mut diag = Diagonal(HashMap::new());
    for it in 0..iterations() {
        forward.train(max_source, vocabulary, &mut diag);
        backward.train(vocabulary, max_source, &mut diag);
        progress((it + 1) as f32 / iterations() as f32);
    }

    // Link each word to the original word both directions agree on most;
    // a word they do not agree on goes to the original word the translation
    // model finds likeliest for it — every word is linked, as a reverse
    // interlinear links "In the beginning" all three to בְּרֵאשִׁית.
    let mut out = Alignment::new();
    let (mut pf, mut pb) = (Vec::new(), Vec::new());
    for (v, k) in keys.iter().enumerate() {
        let (m, n) = forward.shapes[v];
        forward.posteriors(v, &mut diag, &mut pf);
        backward.posteriors(v, &mut diag, &mut pb);
        let links: Vec<Option<Strong>> = (0..m)
            .map(|i| {
                let mut agreed = (link_min(), None);
                let mut likeliest = (0.0, None);
                for j in 0..n {
                    let a = pf[i * (n + 1) + j + 1];
                    let score = (a * pb[j * (m + 1) + i + 1]).sqrt();
                    if score > agreed.0 {
                        agreed = (score, Some(j));
                    }
                    if a > likeliest.0 {
                        likeliest = (a, Some(j));
                    }
                }
                agreed.1.or(likeliest.1).map(|j| {
                    let id = sources[v][j] - 1;
                    Strong { hebrew: id.is_multiple_of(2), number: id / 2 }
                })
            })
            .collect();
        // back onto every token, punctuation-only ones unlinked
        let mut content_links = links.into_iter();
        let tags = targets[v].iter().map(|&w| if w == 0 { None } else { content_links.next().flatten() }).collect();
        out.insert(*k, tags);
    }
    // Verses with no original text — additions found only in Greek, a book
    // the Hebrew does not have — are linked by what their words mean
    // everywhere else.
    let fallback = Fallback::new(&out, verses, &stemmer);
    for (k, text) in verses {
        let tags = out.entry(*k).or_insert_with(|| vec![None; super::words_of(text).len()]);
        for (tag, word) in tags.iter_mut().zip(super::words_of(text)) {
            if tag.is_none() && word.chars().any(char::is_alphanumeric) {
                *tag = originals
                    .greek
                    .as_ref()
                    .filter(|_| matches!(stemmer, Stemming::Greek))
                    .and_then(|g| g.lexeme(&word))
                    .map(Strong::greek)
                    .or_else(|| fallback.link(&key(&word, &stemmer)));
            }
        }
    }
    out
}

/// A translation's own dictionary: what each of its words (by stem) is most
/// often linked to where it has an original. A word in a verse without one
/// takes that: "Ιωναθαν" in 1 Maccabees the Hebrew name it renders in 1
/// Samuel, "rex" in the Vulgate's additions to Esther H4428, king. A word the
/// dictionary lacks takes the link of the known word sharing its longest
/// beginning (at least four letters), the nearest thing to a stem of its own.
struct Fallback {
    by_key: std::collections::BTreeMap<String, Strong>,
}

impl Fallback {
    fn new(linked: &Alignment, verses: &Verses, stemming: &Stemming) -> Fallback {
        let mut counts: HashMap<String, HashMap<Strong, usize>> = HashMap::new();
        for (k, tags) in linked {
            let Some(text) = verses.get(k) else { continue };
            for (tag, word) in tags.iter().zip(super::words_of(text)) {
                if let Some(s) = tag {
                    *counts.entry(key(&word, stemming)).or_default().entry(*s).or_default() += 1;
                }
            }
        }
        let by_key = counts
            .into_iter()
            .filter(|(k, _)| !k.is_empty())
            .filter_map(|(k, c)| c.into_iter().max_by_key(|&(s, n)| (n, std::cmp::Reverse(s))).map(|(s, _)| (k, s)))
            .collect();
        Fallback { by_key }
    }

    fn link(&self, key: &str) -> Option<Strong> {
        if key.is_empty() {
            return None;
        }
        if let Some(s) = self.by_key.get(key) {
            return Some(*s);
        }
        let chars: Vec<char> = key.chars().collect();
        for len in (4..chars.len()).rev() {
            let prefix: String = chars[..len].iter().collect();
            if let Some((_, s)) = self.by_key.range(prefix.clone()..).next().filter(|(k, _)| k.starts_with(&prefix)) {
                return Some(*s);
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn digamma_values() {
        assert!((digamma(1.0) + 0.577_215_664_9).abs() < 1e-9);
        assert!((digamma(10.0) - 2.251_752_589_066).abs() < 1e-9);
    }

    #[test]
    fn stems_pool_inflections() {
        let nb = stemmer("nb");
        let k = |w| key(w, &nb);
        assert_eq!(k("sønnen"), k("sønn"));
        assert_eq!(k("sønnene"), k("sønn"));
        assert_eq!(k("Sønner,"), k("sønn"));
        let fr = stemmer("fr");
        assert_eq!(key("paroles", &fr), key("parole", &fr));
        let en = stemmer("en");
        assert_eq!(key("kings", &en), key("king", &en));
    }

    #[test]
    fn latin_stems() {
        let la = stemmer("la");
        let k = |w| key(w, &la);
        assert_eq!(k("filius"), k("filii"));
        assert_eq!(k("filium"), k("filio"));
        assert_eq!(k("Dominus"), k("domini"));
        assert_eq!(k("cælum"), k("caelum"));
        assert_eq!(k("terramque"), k("terra"));
        assert_eq!(k("atque"), "atque");
        assert_eq!(k("vivus"), k("uiuum"));
    }

    #[test]
    fn greek_stems() {
        let grc = stemmer("grc");
        let k = |w| key(w, &grc);
        assert_eq!(k("λόγος"), k("λόγον"));
        assert_eq!(k("λόγου"), k("λόγος"));
        assert_eq!(k("Θεὸς"), k("θεοῦ"));
        assert_eq!(k("ἐποίησεν"), k("ἐποίησας"));
        assert_eq!(k("οὐρανὸν"), k("οὐρανοῦ"));
    }

    #[test]
    fn learns_which_word_translates_which_number() {
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
        let a = align(&verses, &Originals::new(originals.clone()), "en", &mut |_| {});
        let h = |n| Some(Strong::hebrew(n));
        assert_eq!(a[&(1, 1, 1)], vec![h(430), h(559), h(216)]);
        assert_eq!(a[&(1, 1, 3)][0], h(430));
        assert_eq!(a[&(1, 1, 3)][3], h(216));
        // a verse with no original: its words as they are linked elsewhere
        let mut with_addition = verses;
        with_addition.insert((1, 50, 1), "God saw lights".into());
        let a = align(&with_addition, &Originals::new(originals), "en", &mut |_| {});
        assert_eq!(a[&(1, 50, 1)], vec![h(430), h(7200), h(216)]);
    }
}
