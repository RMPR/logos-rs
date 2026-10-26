//! Builds `bible.db` (SQLite + FTS5) from several open sources:
//!
//! - **getbible.net v2** JSON — plain text for KJV, Louis Segond, Septuagint.
//! - **OpenScriptures morphhb** (OSIS XML) — word-tagged Hebrew OT (lemma +
//!   OSHM morphology), imported as `WLC`.
//! - **MorphGNT SBLGNT** — word-tagged Greek NT (parse code + lemma), as `SBLGNT`.
//! - **OpenScriptures Strong's lexicons** (Hebrew XML, Greek JSON) → `lexicon`.
//!
//! The result has `translations`, `books`, `verses` (+ `verses_fts`), `words`
//! (per-word lemma/Strong's/morphology), and `lexicon` (Strong's definitions).
//!
//! Run with `cargo run --bin import`.

use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};

use rusqlite::{Connection, Transaction};
use serde::Deserialize;
use unicode_normalization::UnicodeNormalization;

use crate::canon::Section;
use crate::morph;

/// Read a vendored source file, transparently handling a `.gz` copy: tries
/// `path`, then `path` + ".gz" (gunzipped). The repo ships the `.gz` versions
/// so the database can be rebuilt offline; raw downloads are cached uncompressed.
fn read_source(path: &Path) -> std::io::Result<String> {
    if path.exists() {
        return std::fs::read_to_string(path);
    }
    let mut gz = path.as_os_str().to_owned();
    gz.push(".gz");
    let file = std::fs::File::open(PathBuf::from(gz))?;
    let mut out = String::new();
    flate2::read::GzDecoder::new(file).read_to_string(&mut out)?;
    Ok(out)
}

/// Path to the database: the prebuilt copy shipped with an installed package
/// if there is one, else `data/bible.db` beside the crate (built on first run).
pub fn db_path() -> PathBuf {
    bundled_db().unwrap_or_else(|| data_dir().join("bible.db"))
}

/// The read-only database an installer put next to the executable:
/// `<exe dir>/data/bible.db` (Windows) or `<prefix>/share/leverbe/bible.db`
/// for an executable in `<prefix>/bin` (the .deb).
fn bundled_db() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    let share = dir.join("..").join("share").join("leverbe");
    [dir.join("data").join("bible.db"), share.join("bible.db")]
        .into_iter()
        .find(|p| p.is_file())
}

/// Localized book names: language code → (book nr → name).
type BookNames = HashMap<String, HashMap<i64, String>>;

fn load_book_names(dir: &Path) -> BookNames {
    let raw = match read_source(&dir.join("book_names.json")) {
        Ok(r) => r,
        Err(_) => return BookNames::new(),
    };
    let parsed: HashMap<String, HashMap<String, String>> =
        serde_json::from_str(&raw).unwrap_or_default();
    parsed
        .into_iter()
        .map(|(lang, m)| {
            let inner = m.into_iter().filter_map(|(k, v)| k.parse().ok().map(|n| (n, v))).collect();
            (lang, inner)
        })
        .collect()
}

/// Localized book name for a (language, nr), falling back to the canonical
/// English name, then a generic label.
fn book_name(names: &BookNames, lang: &str, nr: i64) -> String {
    names
        .get(lang)
        .and_then(|m| m.get(&nr))
        .cloned()
        .or_else(|| crate::canon::name(nr).map(str::to_string))
        .unwrap_or_else(|| format!("Book {nr}"))
}

// --- getbible plain-text translations -------------------------------------

struct Source {
    slug: &'static str,
    code: &'static str,
    name: &'static str,
    sort: i64,
    /// Keep only the 66-book Protestant canon (drop deuterocanon / extras).
    canonical_only: bool,
    /// Network-downloadable (getbible). When false, the source must already be
    /// present locally; it is skipped if absent and never downloaded.
    downloadable: bool,
}

/// A translation imported by `import_strongs_tagged` from bolls-style JSON:
/// the KJV with its own Strong's tagging, or a text whose JSON the former
/// tools/build_strongs.py made (its tags are dropped: every text is linked
/// afresh by `link_translations`).
struct TaggedSource {
    slug: &'static str,
    code: &'static str,
    name: &'static str,
    lang: &'static str,
    sort: i64,
}

const GETBIBLE: &[Source] = &[
    // KJV and LSG come from Strong's-tagged sources (see import_strongs_tagged).
    Source { slug: "lxx", code: "LXX", name: "Septuagint (Greek OT)", sort: 5, canonical_only: false, downloadable: true },
    Source { slug: "vulgate", code: "VUL", name: "Vulgata Clementina", sort: 6, canonical_only: true, downloadable: true },
    // The optional EPUB-derived texts (ESV, BFC, NB) are imported as tagged
    // reverse interlinears via import_strongs_tagged in run().
];

/// ULT USFM file stems (canonical nr = leading number, minus 1 for NT 41–67).
const ULT_FILES: &[&str] = &[
    "01-GEN", "02-EXO", "03-LEV", "04-NUM", "05-DEU", "06-JOS", "07-JDG", "08-RUT", "09-1SA",
    "10-2SA", "11-1KI", "12-2KI", "13-1CH", "14-2CH", "15-EZR", "16-NEH", "17-EST", "18-JOB",
    "19-PSA", "20-PRO", "21-ECC", "22-SNG", "23-ISA", "24-JER", "25-LAM", "26-EZK", "27-DAN",
    "28-HOS", "29-JOL", "30-AMO", "31-OBA", "32-JON", "33-MIC", "34-NAM", "35-HAB", "36-ZEP",
    "37-HAG", "38-ZEC", "39-MAL", "41-MAT", "42-MRK", "43-LUK", "44-JHN", "45-ACT", "46-ROM",
    "47-1CO", "48-2CO", "49-GAL", "50-EPH", "51-PHP", "52-COL", "53-1TH", "54-2TH", "55-1TI",
    "56-2TI", "57-TIT", "58-PHM", "59-HEB", "60-JAS", "61-1PE", "62-2PE", "63-1JN", "64-2JN",
    "65-3JN", "66-JUD", "67-REV",
];

/// morphhb book file stems in canonical order (Genesis=1 … Malachi=39).
const OSHB_BOOKS: &[&str] = &[
    "Gen", "Exod", "Lev", "Num", "Deut", "Josh", "Judg", "Ruth", "1Sam", "2Sam", "1Kgs", "2Kgs",
    "1Chr", "2Chr", "Ezra", "Neh", "Esth", "Job", "Ps", "Prov", "Eccl", "Song", "Isa", "Jer",
    "Lam", "Ezek", "Dan", "Hos", "Joel", "Amos", "Obad", "Jonah", "Mic", "Nah", "Hab", "Zeph",
    "Hag", "Zech", "Mal",
];

/// MorphGNT file stems (matched 61-Mt-… → 87-Re-…) in canonical NT order.
const SBLGNT_BOOKS: &[&str] = &[
    "61-Mt", "62-Mk", "63-Lk", "64-Jn", "65-Ac", "66-Ro", "67-1Co", "68-2Co", "69-Ga", "70-Eph",
    "71-Php", "72-Col", "73-1Th", "74-2Th", "75-1Ti", "76-2Ti", "77-Tit", "78-Phm", "79-Heb",
    "80-Jas", "81-1Pe", "82-2Pe", "83-1Jn", "84-2Jn", "85-3Jn", "86-Jud", "87-Re",
];

#[derive(Deserialize)]
struct RawTranslation {
    lang: String,
    books: Vec<RawBook>,
}
#[derive(Deserialize)]
struct RawBook {
    nr: Option<i64>,
    name: String,
    chapters: Vec<RawChapter>,
}
#[derive(Deserialize)]
struct RawChapter {
    verses: Vec<RawVerse>,
}
#[derive(Deserialize)]
struct RawVerse {
    chapter: i64,
    verse: i64,
    text: String,
}

const SCHEMA: &str = r#"
PRAGMA journal_mode = MEMORY;
PRAGMA synchronous = OFF;

DROP TABLE IF EXISTS verses_fts;
DROP TABLE IF EXISTS words;
DROP TABLE IF EXISTS lexicon;
DROP TABLE IF EXISTS verses;
DROP TABLE IF EXISTS books;
DROP TABLE IF EXISTS translations;

CREATE TABLE translations (
    id   INTEGER PRIMARY KEY,
    code TEXT NOT NULL UNIQUE,
    name TEXT NOT NULL,
    lang TEXT NOT NULL,
    rtl  INTEGER NOT NULL,
    tagged INTEGER NOT NULL,        -- 1 if it has word-level morphology
    sort INTEGER NOT NULL
);

CREATE TABLE books (
    translation_id INTEGER NOT NULL,
    nr       INTEGER NOT NULL,
    name     TEXT NOT NULL,
    section  INTEGER NOT NULL,
    chapters INTEGER NOT NULL,
    PRIMARY KEY (translation_id, nr)
);

CREATE TABLE verses (
    id INTEGER PRIMARY KEY,
    translation_id INTEGER NOT NULL,
    book INTEGER NOT NULL, chapter INTEGER NOT NULL, verse INTEGER NOT NULL,
    text TEXT NOT NULL
);
CREATE INDEX idx_verses_loc ON verses(translation_id, book, chapter, verse);

CREATE TABLE words (
    translation_id INTEGER NOT NULL,
    book INTEGER NOT NULL, chapter INTEGER NOT NULL, verse INTEGER NOT NULL,
    position INTEGER NOT NULL,
    text    TEXT NOT NULL,           -- surface form
    lemma   TEXT,                    -- dictionary form (Greek); empty for Hebrew
    strongs TEXT,                    -- H#### / G#### (nullable)
    morph   TEXT,                    -- raw morphology code
    parsing TEXT                     -- decoded, human-readable
);
CREATE INDEX idx_words_loc ON words(translation_id, book, chapter, verse, position);

CREATE TABLE lexicon (
    strongs    TEXT PRIMARY KEY,     -- H#### / G####
    lemma      TEXT,
    translit   TEXT,
    gloss      TEXT,
    definition TEXT,
    lang       TEXT
);

CREATE VIRTUAL TABLE verses_fts USING fts5(
    text, content='verses', content_rowid='id',
    tokenize="unicode61 remove_diacritics 2"
);
"#;

pub fn data_dir() -> PathBuf {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR")).join("data");
    if manifest.exists() { manifest } else { PathBuf::from("data") }
}

fn fetch(dir: &Path, slug: &str, downloadable: bool) -> Result<String, Box<dyn std::error::Error>> {
    let path = dir.join(format!("{slug}.json"));
    // Use the vendored (.json or .json.gz) copy if present; only download if absent.
    if let Ok(body) = read_source(&path) {
        return Ok(body);
    }
    if !downloadable {
        return Err(format!("{slug}: source not present").into());
    }
    let url = format!("https://api.getbible.net/v2/{slug}.json");
    eprintln!("  downloading {url}");
    let body = ureq::get(&url)
        .call()?
        .body_mut()
        .with_config()
        .limit(64 * 1024 * 1024)
        .read_to_string()?;
    std::fs::write(&path, &body)?;
    Ok(body)
}

fn insert_translation(
    tx: &Transaction,
    code: &str,
    name: &str,
    lang: &str,
    rtl: bool,
    tagged: bool,
    sort: i64,
) -> rusqlite::Result<i64> {
    tx.execute(
        "INSERT INTO translations (code, name, lang, rtl, tagged, sort) VALUES (?1,?2,?3,?4,?5,?6)",
        rusqlite::params![code, name, lang, rtl as i64, tagged as i64, sort],
    )?;
    Ok(tx.last_insert_rowid())
}

fn insert_book(tx: &Transaction, tid: i64, nr: i64, name: &str, chapters: i64) -> rusqlite::Result<()> {
    let section = match Section::of(nr) {
        Section::OldTestament => 0,
        Section::NewTestament => 1,
        Section::Deuterocanon => 2,
    };
    tx.execute(
        "INSERT INTO books (translation_id, nr, name, section, chapters) VALUES (?1,?2,?3,?4,?5)",
        rusqlite::params![tid, nr, name, section, chapters],
    )
    .map(|_| ())
}

// ---------------------------------------------------------------------------
// Lexicons
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct GreekEntry {
    lemma: Option<String>,
    translit: Option<String>,
    kjv_def: Option<String>,
    strongs_def: Option<String>,
}

/// Normalise Greek for lemma matching: NFD, drop combining accents, lower-case,
/// unify final/medial sigma.
fn normalize_greek(s: &str) -> String {
    s.nfd()
        .filter(|c| !('\u{0300}'..='\u{036F}').contains(c))
        .collect::<String>()
        .to_lowercase()
        .replace('ς', "σ")
}

/// Import both Strong's lexicons; return a Greek lemma→Strong's map for tagging.
fn import_lexicons(
    tx: &Transaction,
    dir: &Path,
) -> Result<HashMap<String, String>, Box<dyn std::error::Error>> {
    // --- Hebrew (OSIS-flavoured XML) ---
    let hb = read_source(&dir.join("lexicon/HebrewStrong.xml"))?;
    let doc = roxmltree::Document::parse(&hb)?;
    let mut stmt = tx.prepare(
        "INSERT OR REPLACE INTO lexicon (strongs, lemma, translit, gloss, definition, lang) \
         VALUES (?1,?2,?3,?4,?5,?6)",
    )?;
    for entry in doc.descendants().filter(|n| n.has_tag_name("entry")) {
        let Some(id) = entry.attribute("id") else { continue };
        let w = entry.children().find(|n| n.has_tag_name("w"));
        let lemma = w.and_then(|n| n.text()).unwrap_or("").trim().to_string();
        let translit = w.and_then(|n| n.attribute("xlit")).unwrap_or("").to_string();
        let defs: Vec<String> = entry
            .descendants()
            .filter(|n| n.has_tag_name("def"))
            .filter_map(|n| n.text())
            .map(|t| t.trim().to_string())
            .collect();
        let usage = entry
            .children()
            .find(|n| n.has_tag_name("usage"))
            .map(node_text)
            .unwrap_or_default();
        let meaning = entry
            .children()
            .find(|n| n.has_tag_name("meaning"))
            .map(node_text)
            .unwrap_or_default();
        let gloss = if !defs.is_empty() { defs.join("; ") } else { usage.clone() };
        let mut definition = meaning;
        if !usage.is_empty() {
            definition = format!("{definition} — KJV: {usage}").trim_start_matches(" — ").to_string();
        }
        stmt.execute(rusqlite::params![id, lemma, translit, gloss.trim(), definition.trim(), "heb"])?;
    }

    // --- Greek (JS-wrapped JSON) ---
    let raw = read_source(&dir.join("lexicon/strongs-greek-dictionary.js"))?;
    let start = raw.find('{').ok_or("greek dict: no '{'")?;
    let end = raw.rfind('}').ok_or("greek dict: no '}'")?;
    let map: HashMap<String, GreekEntry> = serde_json::from_str(&raw[start..=end])?;
    let mut greek_map: HashMap<String, String> = HashMap::new();
    for (strongs, e) in &map {
        let lemma = e.lemma.clone().unwrap_or_default();
        let gloss = e.kjv_def.clone().unwrap_or_default();
        let definition = e.strongs_def.clone().unwrap_or_default();
        stmt.execute(rusqlite::params![
            strongs,
            lemma,
            e.translit.clone().unwrap_or_default(),
            gloss.trim(),
            definition.trim(),
            "grc"
        ])?;
        if !lemma.is_empty() {
            let key = normalize_greek(&lemma);
            // Prefer the lowest Strong's number on collision (deterministic).
            let better = match greek_map.get(&key) {
                Some(existing) => strongs_num(strongs) < strongs_num(existing),
                None => true,
            };
            if better {
                greek_map.insert(key, strongs.clone());
            }
        }
    }
    eprintln!("  lexicon: {} Hebrew + {} Greek entries", doc.descendants().filter(|n| n.has_tag_name("entry")).count(), map.len());
    Ok(greek_map)
}

/// STEPBible's brief lexicon of extended Strong's numbers for Greek (TBESG,
/// CC BY 4.0): Abbott-Smith's definitions, and numbers beyond Strong's for
/// the Septuagint's vocabulary the New Testament does not use. The entries
/// Strong's already has keep their definitions; the others are added, so a
/// Septuagint word named by one of them has an entry to show.
fn import_tbesg(tx: &Transaction, dir: &Path) -> Result<usize, Box<dyn std::error::Error>> {
    let raw = read_source(&dir.join("lexicon/tbesg.txt"))?;
    let mut stmt = tx.prepare(
        "INSERT OR IGNORE INTO lexicon (strongs, lemma, translit, gloss, definition, lang) VALUES (?1,?2,?3,?4,?5,'grc')",
    )?;
    let mut added = 0;
    for line in raw.lines() {
        let cols: Vec<&str> = line.split('\t').collect();
        if cols.len() < 8 || !cols[0].starts_with('G') {
            continue;
        }
        let Ok(n) = cols[0][1..].parse::<u32>() else { continue };
        added += stmt.execute(rusqlite::params![
            format!("G{n}"),
            cols[3].trim(),
            cols[4].trim(),
            cols[6].trim(),
            strip_markup(cols[7]),
        ])?;
    }
    Ok(added)
}

/// TBESG's definitions are marked up for its own viewer: tags, line breaks,
/// reference links, "__" indents. Plain text here.
fn strip_markup(html: &str) -> String {
    let with_breaks = html.replace("<BR />", "\n").replace("<br />", "\n");
    let mut out = String::with_capacity(with_breaks.len());
    let mut in_tag = false;
    for c in with_breaks.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            c if !in_tag => out.push(c),
            _ => {}
        }
    }
    let out = out.replace("__", "").replace("&nbsp;", " ").replace("&amp;", "&").replace("&lt;", "<").replace("&gt;", ">");
    out.lines().map(|l| l.split_whitespace().collect::<Vec<_>>().join(" ")).filter(|l| !l.is_empty()).collect::<Vec<_>>().join("\n")
}

fn strongs_num(s: &str) -> u32 {
    s.trim_start_matches(['G', 'H']).parse().unwrap_or(u32::MAX)
}

/// Concatenated text of all descendant *text* nodes, whitespace-collapsed.
/// (Only `is_text()` nodes — an element's own `text()` would double-count.)
fn node_text(n: roxmltree::Node) -> String {
    let raw: String = n
        .descendants()
        .filter(|d| d.is_text())
        .filter_map(|d| d.text())
        .collect::<Vec<_>>()
        .join(" ");
    raw.split_whitespace().collect::<Vec<_>>().join(" ")
}

// ---------------------------------------------------------------------------
// getbible plain translations
// ---------------------------------------------------------------------------

fn import_getbible(tx: &Transaction, dir: &Path, src: &Source) -> Result<(), Box<dyn std::error::Error>> {
    let raw = fetch(dir, src.slug, src.downloadable)?;
    let raw = raw.trim_start_matches('\u{feff}');
    let parsed: RawTranslation = serde_json::from_str(raw)?;
    let rtl = parsed.lang == "he" || parsed.lang == "hbo";
    let tid = insert_translation(tx, src.code, src.name, &parsed.lang, rtl, false, src.sort)?;

    let mut extra = 100i64;
    let mut vstmt = tx.prepare(
        "INSERT INTO verses (translation_id, book, chapter, verse, text) VALUES (?1,?2,?3,?4,?5)",
    )?;
    for book in &parsed.books {
        // For canonical-only sources (e.g. the Vulgate) drop deuterocanonical
        // and non-canonical books (no nr, or outside the 66-book canon).
        if src.canonical_only && !matches!(book.nr, Some(1..=66)) {
            continue;
        }
        let nr = book.nr.unwrap_or_else(|| { let n = extra; extra += 1; n });
        // getbible provides localized book names in the translation's language.
        let name = book.name.trim();
        insert_book(tx, tid, nr, name, book.chapters.len() as i64)?;
        for ch in &book.chapters {
            for v in &ch.verses {
                vstmt.execute(rusqlite::params![tid, nr, v.chapter, v.verse, v.text.trim()])?;
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// OpenScriptures morphhb (Hebrew, word-tagged)
// ---------------------------------------------------------------------------

/// Primary Strong's number for an OSHM lemma string like `b/1471 a` → `H1471`.
fn hebrew_strongs(lemma: &str) -> Option<String> {
    let mut last = None;
    for seg in lemma.split('/') {
        let num: String = seg.trim().chars().take_while(|c| c.is_ascii_digit()).collect();
        if !num.is_empty() {
            last = Some(num);
        }
    }
    last.map(|n| format!("H{n}"))
}

fn import_oshb(tx: &Transaction, dir: &Path, names: &BookNames) -> Result<(), Box<dyn std::error::Error>> {
    let tid = insert_translation(
        tx,
        "WLC",
        "Westminster Leningrad Codex (OSHB)",
        "hbo",
        true,
        true,
        3,
    )?;
    let mut vstmt = tx.prepare(
        "INSERT INTO verses (translation_id, book, chapter, verse, text) VALUES (?1,?2,?3,?4,?5)",
    )?;
    let mut wstmt = tx.prepare(
        "INSERT INTO words (translation_id, book, chapter, verse, position, text, lemma, strongs, morph, parsing) \
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
    )?;

    for (i, stem) in OSHB_BOOKS.iter().enumerate() {
        let nr = (i + 1) as i64;
        let xml = read_source(&dir.join(format!("morphhb/{stem}.xml")))?;
        let doc = roxmltree::Document::parse(&xml)?;
        let mut max_chapter = 0i64;

        for verse in doc.descendants().filter(|n| n.has_tag_name("verse")) {
            let Some(osis) = verse.attribute("osisID") else { continue };
            let parts: Vec<&str> = osis.split('.').collect();
            if parts.len() < 3 {
                continue;
            }
            let chapter: i64 = lead_int(parts[1]);
            let vnum: i64 = lead_int(parts[2]);
            max_chapter = max_chapter.max(chapter);

            let mut position = 0i64;
            let mut surfaces: Vec<String> = Vec::new();
            for w in verse.children().filter(|n| n.has_tag_name("w")) {
                let lemma = w.attribute("lemma").unwrap_or("");
                let morph = w.attribute("morph").unwrap_or("");
                let surface = node_text(w).replace('/', "");
                if surface.is_empty() {
                    continue;
                }
                let strongs = hebrew_strongs(lemma);
                let parsing = morph::decode_hebrew(morph);
                wstmt.execute(rusqlite::params![
                    tid, nr, chapter, vnum, position,
                    surface, "", strongs, morph, parsing
                ])?;
                surfaces.push(surface);
                position += 1;
            }
            vstmt.execute(rusqlite::params![tid, nr, chapter, vnum, surfaces.join(" ")])?;
        }
        insert_book(tx, tid, nr, &book_name(names, "hbo", nr), max_chapter)?;
    }
    Ok(())
}

fn lead_int(s: &str) -> i64 {
    s.chars().take_while(|c| c.is_ascii_digit()).collect::<String>().parse().unwrap_or(0)
}

// ---------------------------------------------------------------------------
// MorphGNT SBLGNT (Greek, word-tagged)
// ---------------------------------------------------------------------------

fn import_sblgnt(
    tx: &Transaction,
    dir: &Path,
    greek_map: &HashMap<String, String>,
    names: &BookNames,
) -> Result<(), Box<dyn std::error::Error>> {
    let tid = insert_translation(tx, "SBLGNT", "SBL Greek New Testament", "grc", false, true, 4)?;
    let mut vstmt = tx.prepare(
        "INSERT INTO verses (translation_id, book, chapter, verse, text) VALUES (?1,?2,?3,?4,?5)",
    )?;
    let mut wstmt = tx.prepare(
        "INSERT INTO words (translation_id, book, chapter, verse, position, text, lemma, strongs, morph, parsing) \
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
    )?;

    for (i, stem) in SBLGNT_BOOKS.iter().enumerate() {
        let nr = 40 + i as i64;
        let txt = read_source(&dir.join(format!("morphgnt/{stem}-morphgnt.txt")))?;
        let mut max_chapter = 0i64;

        let mut cur: Option<(i64, i64)> = None;
        let mut position = 0i64;
        let mut surfaces: Vec<String> = Vec::new();

        let flush = |loc: (i64, i64), surfaces: &mut Vec<String>, vstmt: &mut rusqlite::Statement| {
            if !surfaces.is_empty() {
                let _ = vstmt.execute(rusqlite::params![tid, nr, loc.0, loc.1, surfaces.join(" ")]);
                surfaces.clear();
            }
        };

        for line in txt.lines() {
            let f: Vec<&str> = line.split_whitespace().collect();
            if f.len() < 7 {
                continue;
            }
            let bcv = f[0];
            if bcv.len() < 6 {
                continue;
            }
            let chapter: i64 = bcv[2..4].parse().unwrap_or(0);
            let vnum: i64 = bcv[4..6].parse().unwrap_or(0);
            max_chapter = max_chapter.max(chapter);

            let pos = f[1];
            let parse = f[2];
            let surface = f[3];
            let lemma = f[6];

            if cur != Some((chapter, vnum)) {
                if let Some(loc) = cur {
                    flush(loc, &mut surfaces, &mut vstmt);
                }
                cur = Some((chapter, vnum));
                position = 0;
            }
            let strongs = greek_map.get(&normalize_greek(lemma)).cloned();
            let parsing = morph::decode_greek(pos, parse);
            let morph_code = format!("{pos} {parse}");
            wstmt.execute(rusqlite::params![
                tid, nr, chapter, vnum, position,
                surface, lemma, strongs, morph_code, parsing
            ])?;
            surfaces.push(surface.to_string());
            position += 1;
        }
        if let Some(loc) = cur {
            flush(loc, &mut surfaces, &mut vstmt);
        }
        insert_book(tx, tid, nr, &book_name(names, "grc", nr), max_chapter)?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// unfoldingWord ULT (word-aligned English reverse interlinear)
// ---------------------------------------------------------------------------

/// Normalise a USFM `x-strong` value to a lexicon key.
/// Hebrew `b:H7225`/`H1254a` → `H7225`/`H1254` (last segment, drop homonym
/// letter). Greek `G17220` → `G1722` (unfoldingWord encodes Greek as ×10).
fn normalize_strong(raw: &str) -> Option<String> {
    let seg = raw.rsplit(':').next()?.trim();
    let mut chars = seg.chars();
    let lang = chars.next()?;
    if lang != 'H' && lang != 'G' {
        return None;
    }
    let digits: String = chars.take_while(|c| c.is_ascii_digit()).collect();
    let n: u32 = digits.parse().ok()?;
    let num = if lang == 'G' { n / 10 } else { n };
    if num == 0 {
        return None;
    }
    Some(format!("{lang}{num}"))
}

fn import_ult(tx: &Transaction, dir: &Path, names: &BookNames) -> Result<(), Box<dyn std::error::Error>> {
    let tid = insert_translation(tx, "ULT", "unfoldingWord Literal Text", "en", false, true, 1)?;
    let mut vstmt = tx.prepare(
        "INSERT INTO verses (translation_id, book, chapter, verse, text) VALUES (?1,?2,?3,?4,?5)",
    )?;
    let mut wstmt = tx.prepare(
        "INSERT INTO words (translation_id, book, chapter, verse, position, text, lemma, strongs, morph, parsing) \
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
    )?;

    // One pass over the USFM stream, dispatching on marker type.
    let re = regex::Regex::new(
        r#"(?x)
        \\c\s+(?P<cnum>\d+)
        | \\v\s+(?P<vnum>\d+)
        | \\zaln-s\s*\|(?P<attrs>[^\\]*?)\\\*
        | (?P<ze>\\zaln-e\\\*)
        | \\w\s+(?P<wtext>[^|\\]*)\|[^\\]*?\\w\*
        "#,
    )?;
    let re_strong = regex::Regex::new(r#"x-strong="([^"]*)""#)?;
    let re_lemma = regex::Regex::new(r#"x-lemma="([^"]*)""#)?;

    for stem in ULT_FILES {
        let filenum: i64 = stem[..2].parse().unwrap_or(0);
        let nr = if filenum <= 39 { filenum } else { filenum - 1 };
        let text = read_source(&dir.join(format!("ult/{stem}.usfm")))?;

        let mut chapter = 0i64;
        let mut max_chapter = 0i64;
        let mut cur: (i64, i64) = (0, 0);
        let mut position = 0i64;
        let mut surfaces: Vec<String> = Vec::new();
        // Active alignment scopes (innermost last): (Strong's, original lemma).
        let mut stack: Vec<(Option<String>, String)> = Vec::new();

        for cap in re.captures_iter(&text) {
            if let Some(c) = cap.name("cnum") {
                chapter = c.as_str().parse().unwrap_or(chapter);
            } else if let Some(v) = cap.name("vnum") {
                // flush the verse we were accumulating
                if cur.1 != 0 && !surfaces.is_empty() {
                    vstmt.execute(rusqlite::params![tid, nr, cur.0, cur.1, surfaces.join(" ")])?;
                }
                surfaces.clear();
                position = 0;
                let vnum = v.as_str().parse().unwrap_or(0);
                cur = (chapter, vnum);
                max_chapter = max_chapter.max(chapter);
            } else if let Some(a) = cap.name("attrs") {
                let attrs = a.as_str();
                let strong = re_strong
                    .captures(attrs)
                    .and_then(|c| normalize_strong(&c[1]));
                let lemma = re_lemma.captures(attrs).map(|c| c[1].to_string()).unwrap_or_default();
                stack.push((strong, lemma));
            } else if cap.name("ze").is_some() {
                stack.pop();
            } else if let Some(w) = cap.name("wtext") {
                let word = w.as_str().trim();
                if word.is_empty() {
                    continue;
                }
                let (strong, lemma) = stack.last().cloned().unwrap_or((None, String::new()));
                wstmt.execute(rusqlite::params![
                    tid, nr, cur.0, cur.1, position,
                    word, lemma, strong, "", ""
                ])?;
                surfaces.push(word.to_string());
                position += 1;
            }
        }
        if cur.1 != 0 && !surfaces.is_empty() {
            vstmt.execute(rusqlite::params![tid, nr, cur.0, cur.1, surfaces.join(" ")])?;
        }
        insert_book(tx, tid, nr, &book_name(names, "en", nr), max_chapter)?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// KJV with Strong's numbers (bolls.life) — a clickable English reverse interlinear
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct BollsVerse {
    book: i64,
    chapter: i64,
    verse: i64,
    text: String,
}

/// A publisher's EPUB of one of the optional texts (licensed, never
/// committed): its verses; the words are linked afterwards, like every text's.
fn import_epub(
    tx: &Transaction,
    path: &Path,
    src: &TaggedSource,
    names: &BookNames,
) -> Result<(), Box<dyn std::error::Error>> {
    let books = crate::personal::books::BookMatcher::new([]);
    let doc = crate::personal::epub::read(path, &books).map_err(|e| e.to_string())?;
    let tid = insert_translation(tx, src.code, src.name, src.lang, false, true, src.sort)?;
    let mut stmt =
        tx.prepare("INSERT INTO verses (translation_id, book, chapter, verse, text) VALUES (?1,?2,?3,?4,?5)")?;
    let mut max_chapter: HashMap<i64, i64> = HashMap::new();
    for (&(book, chapter, verse), text) in doc.verses.iter().filter(|((b, _, _), _)| (1..=66).contains(b)) {
        stmt.execute(rusqlite::params![tid, book, chapter, verse, text])?;
        let max = max_chapter.entry(book).or_insert(0);
        *max = (*max).max(chapter);
    }
    for (book, chapters) in max_chapter {
        insert_book(tx, tid, book, &book_name(names, src.lang, book), chapters)?;
    }
    Ok(())
}

/// The former tools/epub2json.py read a merged range "11–12" by its digits, as verse
/// 1112. Such a number, read after verse 10 of the same chapter, is the
/// range's first verse, 11 (its second is matched up by versification).
fn split_joined_range(previous: (i64, i64, i64), (book, chapter, verse): (i64, i64, i64)) -> i64 {
    let next = previous.2 + 1;
    if (previous.0, previous.1) == (book, chapter) && verse > next + 8 {
        let digits = verse.to_string();
        if let Some(rest) = digits.strip_prefix(&next.to_string())
            && rest.parse::<i64>().is_ok_and(|end| end > next && end < next + 9)
        {
            return next;
        }
    }
    verse
}

/// Per verse, each word and the Strong's number it carries, if any.
pub type TaggedText = std::collections::BTreeMap<(i64, i64, i64), Vec<(String, Option<String>)>>;

/// A bolls-style Strong's-tagged text (`data/<slug>.json`): per verse of the
/// 66 books, each word and the Strong's number it carries (H in the Old
/// Testament, G in the New). The KJV's is tagging done by people, which the
/// tests also score the aligner against.
pub fn read_tagged(dir: &Path, slug: &str) -> Result<TaggedText, Box<dyn std::error::Error>> {
    let raw = read_source(&dir.join(format!("{slug}.json")))?;
    let verses: Vec<BollsVerse> = serde_json::from_str(&raw)?;
    // <sup>…</sup> are translator footnotes (stripped); <S>n</S> are Strong's.
    let re_sup = regex::Regex::new(r"(?s)<sup>.*?</sup>")?;
    let re_strong = regex::Regex::new(r"<S>(\d+)</S>")?;
    let re_strong_full = regex::Regex::new(r"<S>\d+</S>")?;
    let mut out = std::collections::BTreeMap::new();
    let mut previous = (0, 0, 0);
    for v in &verses {
        if !(1..=66).contains(&v.book) {
            continue; // drop Apocrypha (bolls numbers them 67+)
        }
        let verse = split_joined_range(previous, (v.book, v.chapter, v.verse));
        previous = (v.book, v.chapter, verse);
        let cleaned = re_sup.replace_all(&v.text, "");
        let mut words = Vec::new();
        for token in cleaned.split_whitespace() {
            // first Strong's on the token → H#### (OT) / G#### (NT)
            let strongs = re_strong
                .captures(token)
                .map(|c| if v.book < 40 { format!("H{}", &c[1]) } else { format!("G{}", &c[1]) });
            let display = re_strong_full.replace_all(token, "");
            let display = display.trim();
            if display.is_empty() {
                continue; // e.g. the untranslated <S>853</S> object marker
            }
            words.push((display.to_string(), strongs));
        }
        out.insert((v.book, v.chapter, verse), words);
    }
    Ok(out)
}

/// Import a bolls-style Strong's-tagged translation (`<S>n</S>` after words):
/// the public-domain KJV and the texts kept in that form (Louis Segond, and
/// the optional ones when no EPUB of theirs is in data/).
fn import_strongs_tagged(
    tx: &Transaction,
    dir: &Path,
    src: &TaggedSource,
    names: &BookNames,
) -> Result<(), Box<dyn std::error::Error>> {
    let verses = read_tagged(dir, src.slug)?;
    let tid = insert_translation(tx, src.code, src.name, src.lang, false, true, src.sort)?;

    let mut vstmt = tx.prepare(
        "INSERT INTO verses (translation_id, book, chapter, verse, text) VALUES (?1,?2,?3,?4,?5)",
    )?;
    let mut wstmt = tx.prepare(
        "INSERT INTO words (translation_id, book, chapter, verse, position, text, lemma, strongs, morph, parsing) \
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
    )?;

    let mut max_chapter: HashMap<i64, i64> = HashMap::new();
    for ((book, chapter, verse), words) in &verses {
        max_chapter.entry(*book).and_modify(|m| *m = (*m).max(*chapter)).or_insert(*chapter);
        for (position, (display, strongs)) in words.iter().enumerate() {
            wstmt.execute(rusqlite::params![
                tid, book, chapter, verse, position as i64, display, "", strongs, "", ""
            ])?;
        }
        let text: Vec<&str> = words.iter().map(|(d, _)| d.as_str()).collect();
        vstmt.execute(rusqlite::params![tid, book, chapter, verse, text.join(" ")])?;
    }

    for book in 1..=66 {
        if let Some(&mc) = max_chapter.get(&book) {
            insert_book(tx, tid, book, &book_name(names, src.lang, book), mc)?;
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Linking every translation's words to the originals
// ---------------------------------------------------------------------------

/// The translations linked word by word to the Hebrew and Greek, with their
/// language (for stemming), and whether their own Strong's tagging — made by
/// people — is kept, the aligner only filling the words it leaves untagged.
const LINKED: &[(&str, &str, bool)] = &[
    ("KJV", "en", true),
    ("ULT", "en", true),
    ("LSG", "fr", false),
    ("ESV", "en", false),
    ("BFC", "fr", false),
    ("NB", "nb", false),
    ("LXX", "grc", false),
    ("VUL", "la", false),
];

/// Link every word of every translation to the Hebrew or Greek word it
/// renders (see [`crate::personal::align`]), so each is a clickable reverse
/// interlinear. Replaces the correspondences the former
/// tools/build_strongs.py built.
fn link_translations(tx: &Transaction) -> Result<(), Box<dyn std::error::Error>> {
    use crate::personal::{align, store, word_spans};
    let originals = store::originals(tx).map_err(|e| e.to_string())?;
    for &(code, lang, keep_human) in LINKED {
        let Some(tid) = tx
            .query_row("SELECT id FROM translations WHERE code = ?1", [code], |r| r.get::<_, i64>(0))
            .ok()
        else {
            continue;
        };
        let started = std::time::Instant::now();
        let mut verses = crate::personal::text::Verses::new();
        {
            let mut stmt = tx.prepare("SELECT book, chapter, verse, text FROM verses WHERE translation_id = ?1")?;
            let mut rows = stmt.query([tid])?;
            while let Some(r) = rows.next()? {
                verses.insert((r.get(0)?, r.get(1)?, r.get(2)?), r.get(3)?);
            }
        }
        // the tagging already there, token by token: (strongs, lemma, morph, parsing)
        type Tag = (Option<String>, String, String, String);
        let mut human: HashMap<(i64, i64, i64), Vec<Tag>> = HashMap::new();
        if keep_human {
            let mut stmt = tx.prepare(
                "SELECT book, chapter, verse, NULLIF(strongs, ''), COALESCE(lemma,''), COALESCE(morph,''), COALESCE(parsing,'')
                 FROM words WHERE translation_id = ?1 ORDER BY book, chapter, verse, position",
            )?;
            let mut rows = stmt.query([tid])?;
            while let Some(r) = rows.next()? {
                human.entry((r.get(0)?, r.get(1)?, r.get(2)?)).or_default().push((r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?));
            }
        }
        let alignment = align::align(&verses, &originals, lang, &mut |_| {});

        tx.execute("DELETE FROM words WHERE translation_id = ?1", [tid])?;
        let mut insert = tx.prepare(
            "INSERT INTO words (translation_id, book, chapter, verse, position, text, lemma, strongs, morph, parsing)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
        )?;
        let (mut words, mut linked, mut kept) = (0usize, 0usize, 0usize);
        for (key, text) in &verses {
            let tags = alignment.get(key);
            let theirs = human.get(key);
            for (position, (word, span)) in word_spans(text).into_iter().enumerate() {
                // a tag made by people wins; the first in a merged word
                let mine = theirs.and_then(|t| span.clone().filter_map(|i| t.get(i)).find(|t| t.0.is_some()));
                let aligned = tags.and_then(|t| t.get(position).copied().flatten()).map(|s| s.to_string());
                let strongs = mine.and_then(|t| t.0.clone()).or(aligned);
                let empty = (None, String::new(), String::new(), String::new());
                let (_, lemma, morph, parsing) = mine.unwrap_or(&empty);
                words += 1;
                linked += strongs.is_some() as usize;
                kept += mine.is_some() as usize;
                insert.execute(rusqlite::params![
                    tid, key.0, key.1, key.2, position as i64, word, lemma, strongs, morph, parsing
                ])?;
            }
        }
        tx.execute("UPDATE translations SET tagged = 1 WHERE id = ?1", [tid])?;
        eprintln!(
            "  {code}: {linked} of {words} words linked ({:.1}%, {kept} by hand) in {:.0?}",
            100.0 * linked as f64 / words.max(1) as f64,
            started.elapsed()
        );
    }
    Ok(())
}

// ---------------------------------------------------------------------------

pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    let dir = data_dir();
    std::fs::create_dir_all(&dir)?;
    let db_path = dir.join("bible.db");
    let _ = std::fs::remove_file(&db_path);

    let mut conn = Connection::open(&db_path)?;
    conn.execute_batch(SCHEMA)?;

    let tx = conn.transaction()?;

    let names = load_book_names(&dir);

    eprintln!("importing lexicons");
    let greek_map = import_lexicons(&tx, &dir)?;
    let extended = import_tbesg(&tx, &dir)?;
    eprintln!("  lexicon: {extended} extended Greek entries (STEPBible TBESG)");

    eprintln!("importing KJV (bolls.life, with Strong's numbers)");
    let kjv = TaggedSource {
        slug: "kjv_strongs", code: "KJV", name: "King James Version", lang: "en", sort: 0,
    };
    import_strongs_tagged(&tx, &dir, &kjv, &names)?;

    eprintln!("importing LSG (Louis Segond + built Strong's correspondence)");
    let lsg = TaggedSource {
        slug: "lsg_strongs", code: "LSG", name: "Louis Segond (1910)", lang: "fr", sort: 2,
    };
    import_strongs_tagged(&tx, &dir, &lsg, &names)?;

    // EPUB-derived texts, tagged with a built Strong's correspondence. Their
    // text is copyrighted → the *_strongs.json are local-only, skipped if absent.
    // Read from the publishers' EPUBs placed in data/, or from the JSON once
    // made from them by the former tools/epub2json.py.
    let epubs: Vec<PathBuf> = std::fs::read_dir(&dir)
        .map(|d| d.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "epub")).collect())
        .unwrap_or_default();
    for (src, publisher) in [
        (
            TaggedSource { slug: "esv_strongs", code: "ESV", name: "English Standard Version", lang: "en", sort: 7 },
            "Crossway",
        ),
        (
            TaggedSource { slug: "fc_strongs", code: "BFC", name: "Bible en français courant", lang: "fr", sort: 8 },
            "Société biblique française",
        ),
        (
            TaggedSource { slug: "nb_strongs", code: "NB", name: "Bibelen 2011 (Bokmål)", lang: "nb", sort: 9 },
            "Bibelselskapet",
        ),
    ] {
        let epub = epubs.iter().find(|p| crate::personal::epub::publisher(p) == Some(publisher));
        let imported = match epub {
            Some(path) => import_epub(&tx, path, &src, &names).map(|()| format!("from {}", path.display())),
            None => import_strongs_tagged(&tx, &dir, &src, &names).map(|()| format!("from {}.json", src.slug)),
        };
        match imported {
            Ok(from) => eprintln!("importing {} {from}", src.code),
            Err(_) => eprintln!("skipping {} — optional source not present", src.code),
        }
    }

    for src in GETBIBLE {
        match import_getbible(&tx, &dir, src) {
            Ok(()) => eprintln!("importing {} ({})", src.code, src.slug),
            // Optional (non-downloadable) sources just skip when not present.
            Err(_) if !src.downloadable => {
                eprintln!("skipping {} — optional source not present", src.code)
            }
            Err(e) => return Err(e),
        }
    }

    eprintln!("importing WLC (morphhb, word-tagged Hebrew)");
    import_oshb(&tx, &dir, &names)?;

    eprintln!("importing SBLGNT (MorphGNT, word-tagged Greek)");
    import_sblgnt(&tx, &dir, &greek_map, &names)?;

    eprintln!("importing ULT (unfoldingWord, word-aligned English)");
    import_ult(&tx, &dir, &names)?;

    eprintln!("linking every translation's words to the Hebrew and Greek");
    link_translations(&tx)?;

    tx.commit()?;

    eprintln!("building full-text index…");
    conn.execute_batch("INSERT INTO verses_fts(verses_fts) VALUES('rebuild');")?;
    // A rollback journal, not WAL: the app only ever reads the database, and a
    // WAL database cannot be opened from a read-only directory (/usr/share).
    conn.execute_batch("PRAGMA journal_mode = DELETE; VACUUM; ANALYZE;")?;

    let verses: i64 = conn.query_row("SELECT count(*) FROM verses", [], |r| r.get(0))?;
    let words: i64 = conn.query_row("SELECT count(*) FROM words", [], |r| r.get(0))?;
    let tagged: i64 = conn.query_row("SELECT count(*) FROM words WHERE strongs IS NOT NULL", [], |r| r.get(0))?;
    eprintln!("done → {} ({verses} verses, {words} words, {tagged} with Strong's)", db_path.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn joined_ranges_split() {
        assert_eq!(split_joined_range((45, 9, 10), (45, 9, 1112)), 11);
        assert_eq!(split_joined_range((49, 1, 14), (49, 1, 1516)), 15);
        // real verses are left alone
        assert_eq!(split_joined_range((19, 119, 99), (19, 119, 100)), 100);
        assert_eq!(split_joined_range((27, 3, 99), (27, 3, 100)), 100);
        assert_eq!(split_joined_range((1, 1, 31), (1, 2, 1)), 1);
    }

    #[test]
    fn markup_is_stripped() {
        let html = " <b>ἀγγέλλω</b> (ἄγγελος), <BR /> [in LXX for נגד hi. ;] <BR /><b>to announce, report</b>: <ref='Jhn.4.51'>Jhn.4:51</ref> (WHR omit).†<BR /> __I. Of";
        assert_eq!(
            strip_markup(html),
            "ἀγγέλλω (ἄγγελος),\n[in LXX for נגד hi. ;]\nto announce, report: Jhn.4:51 (WHR omit).†\nI. Of"
        );
    }
}
