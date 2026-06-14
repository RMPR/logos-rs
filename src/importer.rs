//! Builds `bible.db` (SQLite + FTS5) from getbible.net v2 JSON dumps.
//!
//! Run with `cargo run --bin import`. Source JSON is cached under `data/`;
//! missing files are downloaded. The resulting database has three tables
//! (`translations`, `books`, `verses`) plus a full-text index `verses_fts`.

use std::path::{Path, PathBuf};

use rusqlite::Connection;
use serde::Deserialize;

use crate::canon::Section;

/// A translation we ship. `slug` is the getbible filename/endpoint; `code` is
/// the short label shown in the UI.
struct Source {
    slug: &'static str,
    code: &'static str,
    name: &'static str,
}

const SOURCES: &[Source] = &[
    Source { slug: "kjv", code: "KJV", name: "King James Version" },
    Source { slug: "ls1910", code: "LSG", name: "Louis Segond (1910)" },
    Source { slug: "codex", code: "WLC", name: "Westminster Leningrad Codex (Hebrew OT)" },
    Source { slug: "westcotthort", code: "WH", name: "Westcott-Hort (Greek NT)" },
    Source { slug: "lxx", code: "LXX", name: "Septuagint (Greek OT)" },
];

#[derive(Deserialize)]
struct RawTranslation {
    #[serde(default)]
    direction: Option<String>,
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
DROP TABLE IF EXISTS verses;
DROP TABLE IF EXISTS books;
DROP TABLE IF EXISTS translations;

CREATE TABLE translations (
    id        INTEGER PRIMARY KEY,
    code      TEXT NOT NULL UNIQUE,   -- KJV, LSG, WLC, WH, LXX
    name      TEXT NOT NULL,
    lang      TEXT NOT NULL,          -- en, fr, hbo, grc
    rtl       INTEGER NOT NULL,       -- 1 if right-to-left
    sort      INTEGER NOT NULL
);

CREATE TABLE books (
    translation_id INTEGER NOT NULL REFERENCES translations(id),
    nr             INTEGER NOT NULL,  -- canonical book id (1..66, >=100 extra)
    name           TEXT NOT NULL,     -- localized display name
    section        INTEGER NOT NULL,  -- 0 OT, 1 NT, 2 deuterocanon
    chapters       INTEGER NOT NULL,
    PRIMARY KEY (translation_id, nr)
);

CREATE TABLE verses (
    id             INTEGER PRIMARY KEY,
    translation_id INTEGER NOT NULL REFERENCES translations(id),
    book           INTEGER NOT NULL,
    chapter        INTEGER NOT NULL,
    verse          INTEGER NOT NULL,
    text           TEXT NOT NULL
);
CREATE INDEX idx_verses_loc ON verses(translation_id, book, chapter, verse);

-- External-content FTS over verse text. Diacritics/cantillation stripped so
-- Greek and Hebrew are searchable without accents or vowel points.
CREATE VIRTUAL TABLE verses_fts USING fts5(
    text,
    content='verses',
    content_rowid='id',
    tokenize="unicode61 remove_diacritics 2"
);
"#;

fn section_code(section: Section) -> i64 {
    match section {
        Section::OldTestament => 0,
        Section::NewTestament => 1,
        Section::Deuterocanon => 2,
    }
}

/// Resolve `data/` next to the crate manifest (dev) or the current dir.
fn data_dir() -> PathBuf {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR")).join("data");
    if manifest.exists() {
        manifest
    } else {
        PathBuf::from("data")
    }
}

/// Read a cached source file, downloading it from getbible.net if absent.
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

pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    let dir = data_dir();
    std::fs::create_dir_all(&dir)?;
    let db_path = dir.join("bible.db");
    let _ = std::fs::remove_file(&db_path);

    let mut conn = Connection::open(&db_path)?;
    conn.execute_batch(SCHEMA)?;

    for (sort, src) in SOURCES.iter().enumerate() {
        eprintln!("importing {} ({})", src.code, src.slug);
        let raw = fetch(&dir, src.slug)?;
        // Sources are UTF-8 with an occasional BOM; trim it for serde.
        let raw = raw.trim_start_matches('\u{feff}');
        let parsed: RawTranslation = serde_json::from_str(raw)?;
        let rtl = matches!(parsed.direction.as_deref(), Some("RTL"));

        let tx = conn.transaction()?;
        tx.execute(
            "INSERT INTO translations (code, name, lang, rtl, sort) VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![src.code, src.name, parsed.lang, rtl as i64, sort as i64],
        )?;
        let translation_id = tx.last_insert_rowid();

        let mut extra_nr = 100; // synthetic ids for null-nr (LXX deuterocanon)
        let mut verse_count = 0usize;
        {
            let mut book_stmt = tx.prepare(
                "INSERT INTO books (translation_id, nr, name, section, chapters) VALUES (?1, ?2, ?3, ?4, ?5)",
            )?;
            let mut verse_stmt = tx.prepare(
                "INSERT INTO verses (translation_id, book, chapter, verse, text) VALUES (?1, ?2, ?3, ?4, ?5)",
            )?;
            for book in &parsed.books {
                let nr = book.nr.unwrap_or_else(|| {
                    let n = extra_nr;
                    extra_nr += 1;
                    n
                });
                let section = section_code(Section::of(nr));
                book_stmt.execute(rusqlite::params![
                    translation_id,
                    nr,
                    book.name.trim(),
                    section,
                    book.chapters.len() as i64,
                ])?;
                for chapter in &book.chapters {
                    for v in &chapter.verses {
                        verse_stmt.execute(rusqlite::params![
                            translation_id,
                            nr,
                            v.chapter,
                            v.verse,
                            v.text.trim(),
                        ])?;
                        verse_count += 1;
                    }
                }
            }
        }
        tx.commit()?;
        eprintln!("  {} books, {} verses", parsed.books.len(), verse_count);
    }

    eprintln!("building full-text index…");
    conn.execute_batch("INSERT INTO verses_fts(verses_fts) VALUES('rebuild');")?;
    conn.execute_batch("PRAGMA journal_mode = WAL; VACUUM; ANALYZE;")?;

    let total: i64 = conn.query_row("SELECT count(*) FROM verses", [], |r| r.get(0))?;
    eprintln!("done → {} ({} verses)", db_path.display(), total);
    Ok(())
}
