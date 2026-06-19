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
use std::path::{Path, PathBuf};

use rusqlite::{Connection, Transaction};
use serde::Deserialize;
use unicode_normalization::UnicodeNormalization;

use crate::canon::Section;
use crate::morph;

// --- getbible plain-text translations -------------------------------------

struct Source {
    slug: &'static str,
    code: &'static str,
    name: &'static str,
    sort: i64,
}

const GETBIBLE: &[Source] = &[
    Source { slug: "kjv", code: "KJV", name: "King James Version", sort: 0 },
    Source { slug: "ls1910", code: "LSG", name: "Louis Segond (1910)", sort: 2 },
    Source { slug: "lxx", code: "LXX", name: "Septuagint (Greek OT)", sort: 5 },
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

fn data_dir() -> PathBuf {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR")).join("data");
    if manifest.exists() { manifest } else { PathBuf::from("data") }
}

fn fetch(dir: &Path, slug: &str) -> Result<String, Box<dyn std::error::Error>> {
    let path = dir.join(format!("{slug}.json"));
    if path.exists() {
        return Ok(std::fs::read_to_string(&path)?);
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
    let hb = std::fs::read_to_string(dir.join("lexicon/HebrewStrong.xml"))?;
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
    let raw = std::fs::read_to_string(dir.join("lexicon/strongs-greek-dictionary.js"))?;
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
    let raw = fetch(dir, src.slug)?;
    let raw = raw.trim_start_matches('\u{feff}');
    let parsed: RawTranslation = serde_json::from_str(raw)?;
    let rtl = parsed.lang == "he" || parsed.lang == "hbo";
    let tid = insert_translation(tx, src.code, src.name, &parsed.lang, rtl, false, src.sort)?;

    let mut extra = 100i64;
    let mut vstmt = tx.prepare(
        "INSERT INTO verses (translation_id, book, chapter, verse, text) VALUES (?1,?2,?3,?4,?5)",
    )?;
    for book in &parsed.books {
        let nr = book.nr.unwrap_or_else(|| { let n = extra; extra += 1; n });
        let name = crate::canon::name(nr).map(str::to_string).unwrap_or_else(|| book.name.trim().to_string());
        insert_book(tx, tid, nr, &name, book.chapters.len() as i64)?;
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

fn import_oshb(tx: &Transaction, dir: &Path) -> Result<(), Box<dyn std::error::Error>> {
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
        let xml = std::fs::read_to_string(dir.join(format!("morphhb/{stem}.xml")))?;
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
        insert_book(tx, tid, nr, crate::canon::name(nr).unwrap_or(stem), max_chapter)?;
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
        let txt = std::fs::read_to_string(dir.join(format!("morphgnt/{stem}-morphgnt.txt")))?;
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
        insert_book(tx, tid, nr, crate::canon::name(nr).unwrap_or(stem), max_chapter)?;
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

fn import_ult(tx: &Transaction, dir: &Path) -> Result<(), Box<dyn std::error::Error>> {
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
        let text = std::fs::read_to_string(dir.join(format!("ult/{stem}.usfm")))?;

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
        insert_book(tx, tid, nr, crate::canon::name(nr).unwrap_or(stem), max_chapter)?;
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

    eprintln!("importing lexicons");
    let greek_map = import_lexicons(&tx, &dir)?;

    for src in GETBIBLE {
        eprintln!("importing {} ({})", src.code, src.slug);
        import_getbible(&tx, &dir, src)?;
    }

    eprintln!("importing WLC (morphhb, word-tagged Hebrew)");
    import_oshb(&tx, &dir)?;

    eprintln!("importing SBLGNT (MorphGNT, word-tagged Greek)");
    import_sblgnt(&tx, &dir, &greek_map)?;

    eprintln!("importing ULT (unfoldingWord, word-aligned English)");
    import_ult(&tx, &dir)?;

    tx.commit()?;

    eprintln!("building full-text index…");
    conn.execute_batch("INSERT INTO verses_fts(verses_fts) VALUES('rebuild');")?;
    conn.execute_batch("PRAGMA journal_mode = WAL; VACUUM; ANALYZE;")?;

    let verses: i64 = conn.query_row("SELECT count(*) FROM verses", [], |r| r.get(0))?;
    let words: i64 = conn.query_row("SELECT count(*) FROM words", [], |r| r.get(0))?;
    let tagged: i64 = conn.query_row("SELECT count(*) FROM words WHERE strongs IS NOT NULL", [], |r| r.get(0))?;
    eprintln!("done → {} ({verses} verses, {words} words, {tagged} with Strong's)", db_path.display());
    Ok(())
}
