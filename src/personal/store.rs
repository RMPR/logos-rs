//! Where imported translations live.
//!
//! Not in `bible.db`: an installed copy of that sits read-only under
//! `/usr/share` (or Program Files), and a rebuild replaces it. Imports go to a
//! database of their own in the user's data directory, with the same tables.
//! The app attaches it to the bundled one and reads both as one (see
//! [`crate::bible::Bible::open`]); an imported translation's id starts at
//! [`FIRST_ID`] so the two never collide.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use rusqlite::{Connection, OptionalExtension, params};

use super::align::{Alignment, Originals, VerseStrongs};
use super::text::Verses;
use super::Error;

/// Ids of imported translations start here; the bundled ones are below.
pub const FIRST_ID: i64 = 1_000_000;

/// Bumped whenever the aligner links words differently enough that texts
/// imported before should be linked again (see [`stale`]). 1: IBM Model 1;
/// 2: the fast_align model with stems and versification.
pub const ALIGNER_VERSION: i64 = 2;

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS translations (
    id   INTEGER PRIMARY KEY,
    code TEXT NOT NULL UNIQUE,
    name TEXT NOT NULL,
    lang TEXT NOT NULL,
    rtl  INTEGER NOT NULL,
    tagged INTEGER NOT NULL,
    sort INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS books (
    translation_id INTEGER NOT NULL,
    nr       INTEGER NOT NULL,
    name     TEXT NOT NULL,
    section  INTEGER NOT NULL,
    chapters INTEGER NOT NULL,
    PRIMARY KEY (translation_id, nr)
);
CREATE TABLE IF NOT EXISTS verses (
    id INTEGER PRIMARY KEY,
    translation_id INTEGER NOT NULL,
    book INTEGER NOT NULL, chapter INTEGER NOT NULL, verse INTEGER NOT NULL,
    text TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_verses_loc ON verses(translation_id, book, chapter, verse);
CREATE TABLE IF NOT EXISTS words (
    translation_id INTEGER NOT NULL,
    book INTEGER NOT NULL, chapter INTEGER NOT NULL, verse INTEGER NOT NULL,
    position INTEGER NOT NULL,
    text    TEXT NOT NULL,
    lemma   TEXT,
    strongs TEXT,
    morph   TEXT,
    parsing TEXT
);
CREATE INDEX IF NOT EXISTS idx_words_loc ON words(translation_id, book, chapter, verse, position);
-- the version of the aligner each translation was linked with: "linked:<id>"
CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
CREATE VIRTUAL TABLE IF NOT EXISTS verses_fts USING fts5(
    text, content='verses', content_rowid='id',
    tokenize="unicode61 remove_diacritics 2"
);
"#;

/// `$LOGOS_IMPORTS`, else `imports.db` in the platform data directory.
pub fn path() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("LOGOS_IMPORTS") {
        return Some(PathBuf::from(p));
    }
    directories::ProjectDirs::from("", "", "logos-rs").map(|d| d.data_dir().join("imports.db"))
}

/// Open the imports database for writing, creating it if need be.
pub fn open(path: &Path) -> Result<Connection, Error> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let conn = Connection::open(path)?;
    // WAL: the app keeps reading while an import writes
    conn.execute_batch("PRAGMA journal_mode = WAL;")?;
    conn.execute_batch(SCHEMA)?;
    Ok(conn)
}

/// The Strong's numbers of each verse of the tagged Hebrew and Greek, in
/// the order of their words, from the bundled database.
///
/// The SBLGNT, a critical text, leaves out verses the older Greek text has
/// (Matthew 17:21, John 7:53–8:11, Romans 16:25–27…). Translations made from
/// that text have them, so for those verses the originals are taken from the
/// KJV's own Strong's tagging, which follows it.
pub fn originals(bundled: &Connection) -> Result<Originals, Error> {
    let mut out = VerseStrongs::new();
    let mut read = |code: &str, only_missing: bool| -> Result<(), Error> {
        let mut stmt = bundled.prepare(
            "SELECT w.book, w.chapter, w.verse, w.strongs FROM words w
             JOIN translations t ON t.id = w.translation_id
             WHERE t.code = ?1 AND w.strongs IS NOT NULL AND w.strongs <> ''
             ORDER BY w.book, w.chapter, w.verse, w.position",
        )?;
        let mut rows = stmt.query([code])?;
        let had: std::collections::HashSet<(i64, i64, i64)> = out.keys().copied().collect();
        while let Some(r) = rows.next()? {
            let key: (i64, i64, i64) = (r.get(0)?, r.get(1)?, r.get(2)?);
            if only_missing && (key.0 < 40 || had.contains(&key)) {
                continue;
            }
            let strongs: String = r.get(3)?;
            let digits: String = strongs.chars().filter(char::is_ascii_digit).collect();
            if let Ok(n) = digits.parse::<u32>() {
                out.entry(key).or_default().push(n);
            }
        }
        Ok(())
    };
    read("WLC", false)?;
    read("SBLGNT", false)?;
    read("KJV", true)?;
    let mut originals = Originals::new(out);
    originals.greek = Some(greek_lexemes(bundled)?);
    Ok(originals)
}

/// Greek words named by lexeme: the SBLGNT's tagged forms and the lexicon's
/// Greek headwords (Strong's and STEPBible's extended numbers).
pub fn greek_lexemes(bundled: &Connection) -> Result<super::greek::GreekLexemes, Error> {
    let number = |s: &str| s.trim_start_matches('G').parse::<u32>().ok();
    let mut stmt = bundled.prepare(
        "SELECT w.text, w.strongs FROM words w JOIN translations t ON t.id = w.translation_id
         WHERE t.code = 'SBLGNT' AND w.strongs LIKE 'G%'",
    )?;
    let tagged: Vec<(String, u32)> = stmt
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
        .filter_map(|r| r.ok().and_then(|(w, s)| Some((w, number(&s)?))))
        .collect();
    let mut stmt = bundled.prepare("SELECT lemma, strongs FROM lexicon WHERE strongs LIKE 'G%' AND lemma <> ''")?;
    let heads: Vec<(String, u32)> = stmt
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
        .filter_map(|r| r.ok().and_then(|(w, s)| Some((w, number(&s)?))))
        .collect();
    Ok(super::greek::GreekLexemes::new(
        tagged.iter().map(|(w, g)| (w.as_str(), *g)),
        heads.iter().map(|(w, g)| (w.as_str(), *g)),
    ))
}

/// What [`save`] needs besides the verses.
pub struct NewTranslation<'a> {
    pub code: &'a str,
    pub name: &'a str,
    pub lang: &'a str,
    /// Book number → name, in the translation's language where known.
    pub book_names: &'a HashMap<i64, String>,
}

/// Store a translation and its word alignment in one transaction; returns
/// its id.
pub fn save(conn: &mut Connection, t: &NewTranslation, verses: &Verses, alignment: &Alignment) -> Result<i64, Error> {
    let tx = conn.transaction()?;
    let exists: Option<i64> =
        tx.query_row("SELECT id FROM translations WHERE code = ?1", [t.code], |r| r.get(0)).optional()?;
    if exists.is_some() {
        return Err(format!("A translation coded {} is already imported", t.code).into());
    }
    let id: i64 = tx.query_row(
        "SELECT MAX(?1, COALESCE(MAX(id), 0) + 1) FROM translations",
        [FIRST_ID],
        |r| r.get(0),
    )?;
    let rtl = matches!(t.lang, "hbo" | "he" | "ar" | "arc");
    // after the bundled texts, in the order they were imported
    tx.execute(
        "INSERT INTO translations (id, code, name, lang, rtl, tagged, sort) VALUES (?1,?2,?3,?4,?5,1,?6)",
        params![id, t.code, t.name, t.lang, rtl as i64, 1000 + id - FIRST_ID],
    )?;

    let mut chapters: HashMap<i64, i64> = HashMap::new();
    {
        let mut vstmt = tx.prepare(
            "INSERT INTO verses (translation_id, book, chapter, verse, text) VALUES (?1,?2,?3,?4,?5)",
        )?;
        let mut wstmt = tx.prepare(
            "INSERT INTO words (translation_id, book, chapter, verse, position, text, lemma, strongs, morph, parsing)
             VALUES (?1,?2,?3,?4,?5,?6,'',?7,'','')",
        )?;
        for (&(book, chapter, verse), text) in verses {
            let max = chapters.entry(book).or_insert(0);
            *max = (*max).max(chapter);
            vstmt.execute(params![id, book, chapter, verse, text])?;
            let tags = alignment.get(&(book, chapter, verse));
            for (position, word) in super::words_of(text).iter().enumerate() {
                let strongs = tags.and_then(|t| t.get(position).copied().flatten()).map(|s| s.to_string());
                wstmt.execute(params![id, book, chapter, verse, position as i64, word, strongs])?;
            }
        }
        let mut bstmt = tx.prepare(
            "INSERT INTO books (translation_id, nr, name, section, chapters) VALUES (?1,?2,?3,?4,?5)",
        )?;
        let mut books: Vec<_> = chapters.into_iter().collect();
        books.sort();
        for (nr, max) in books {
            let name = t
                .book_names
                .get(&nr)
                .cloned()
                .or_else(|| crate::canon::name(nr).map(str::to_string))
                .unwrap_or_else(|| format!("Book {nr}"));
            bstmt.execute(params![id, nr, name, if nr < 40 { 0 } else { 1 }, max])?;
        }
    }
    tx.execute(
        "INSERT OR REPLACE INTO meta (key, value) VALUES (?1, ?2)",
        params![format!("linked:{id}"), ALIGNER_VERSION.to_string()],
    )?;
    tx.execute_batch("INSERT INTO verses_fts(verses_fts) VALUES('rebuild');")?;
    tx.commit()?;
    Ok(id)
}

/// The imported translations linked by an older aligner: (id, language).
pub fn stale(conn: &Connection) -> Result<Vec<(i64, String)>, Error> {
    let mut stmt = conn.prepare(
        "SELECT t.id, t.lang FROM translations t
         LEFT JOIN meta m ON m.key = 'linked:' || t.id
         WHERE COALESCE(CAST(m.value AS INTEGER), 1) < ?1",
    )?;
    let rows = stmt.query_map([ALIGNER_VERSION], |r| Ok((r.get(0)?, r.get(1)?)))?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// An imported translation's verses, as stored.
pub fn verses(conn: &Connection, id: i64) -> Result<Verses, Error> {
    let mut stmt = conn.prepare("SELECT book, chapter, verse, text FROM verses WHERE translation_id = ?1")?;
    let rows = stmt.query_map([id], |r| Ok(((r.get(0)?, r.get(1)?, r.get(2)?), r.get(3)?)))?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// Replace an imported translation's word links with `alignment`.
pub fn relink(conn: &mut Connection, id: i64, verses: &Verses, alignment: &Alignment) -> Result<(), Error> {
    let tx = conn.transaction()?;
    tx.execute("DELETE FROM words WHERE translation_id = ?1", [id])?;
    {
        let mut wstmt = tx.prepare(
            "INSERT INTO words (translation_id, book, chapter, verse, position, text, lemma, strongs, morph, parsing)
             VALUES (?1,?2,?3,?4,?5,?6,'',?7,'','')",
        )?;
        for (&(book, chapter, verse), text) in verses {
            let tags = alignment.get(&(book, chapter, verse));
            for (position, word) in super::words_of(text).iter().enumerate() {
                let strongs = tags.and_then(|t| t.get(position).copied().flatten()).map(|s| s.to_string());
                wstmt.execute(params![id, book, chapter, verse, position as i64, word, strongs])?;
            }
        }
    }
    tx.execute(
        "INSERT OR REPLACE INTO meta (key, value) VALUES (?1, ?2)",
        params![format!("linked:{id}"), ALIGNER_VERSION.to_string()],
    )?;
    tx.commit()?;
    Ok(())
}

/// Remove an imported translation.
pub fn delete(conn: &mut Connection, id: i64) -> Result<(), Error> {
    if id < FIRST_ID {
        return Err("Only imported translations can be removed".into());
    }
    let tx = conn.transaction()?;
    for table in ["words", "verses", "books"] {
        tx.execute(&format!("DELETE FROM {table} WHERE translation_id = ?1"), [id])?;
    }
    tx.execute("DELETE FROM translations WHERE id = ?1", [id])?;
    tx.execute("DELETE FROM meta WHERE key = ?1", [format!("linked:{id}")])?;
    tx.execute_batch("INSERT INTO verses_fts(verses_fts) VALUES('rebuild');")?;
    tx.commit()?;
    Ok(())
}
