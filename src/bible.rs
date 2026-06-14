//! Read-only data model over `bible.db`.

use std::path::{Path, PathBuf};

use rusqlite::{Connection, OptionalExtension};

#[derive(Clone, Debug)]
pub struct Translation {
    pub id: i64,
    pub code: String,
    pub name: String,
    pub lang: String,
    pub rtl: bool,
}

#[derive(Clone, Debug)]
pub struct Book {
    pub nr: i64,
    pub name: String,
    pub section: i64, // 0 OT, 1 NT, 2 deuterocanon
    pub chapters: i64,
}

#[derive(Clone, Debug)]
pub struct Verse {
    pub book: i64,
    pub chapter: i64,
    pub verse: i64,
    pub text: String,
}

#[derive(Clone, Debug)]
pub struct SearchHit {
    pub book: i64,
    pub book_name: String,
    pub chapter: i64,
    pub verse: i64,
    /// Verse text with matched terms wrapped in `«…»`.
    pub snippet: String,
}

pub struct Bible {
    conn: Connection,
}

/// Locate `bible.db`: `$LOGOS_DB`, then `data/` beside the manifest, then `./data`.
pub fn default_db_path() -> PathBuf {
    if let Ok(p) = std::env::var("LOGOS_DB") {
        return PathBuf::from(p);
    }
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR")).join("data/bible.db");
    if manifest.exists() {
        return manifest;
    }
    PathBuf::from("data/bible.db")
}

impl Bible {
    pub fn open(path: &Path) -> rusqlite::Result<Bible> {
        let conn = Connection::open_with_flags(
            path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        Ok(Bible { conn })
    }

    pub fn translations(&self) -> rusqlite::Result<Vec<Translation>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, code, name, lang, rtl FROM translations ORDER BY sort")?;
        let rows = stmt.query_map([], |r| {
            Ok(Translation {
                id: r.get(0)?,
                code: r.get(1)?,
                name: r.get(2)?,
                lang: r.get(3)?,
                rtl: r.get::<_, i64>(4)? != 0,
            })
        })?;
        rows.collect()
    }

    pub fn books(&self, translation_id: i64) -> rusqlite::Result<Vec<Book>> {
        let mut stmt = self.conn.prepare(
            "SELECT nr, name, section, chapters FROM books WHERE translation_id = ?1 ORDER BY nr",
        )?;
        let rows = stmt.query_map([translation_id], |r| {
            Ok(Book {
                nr: r.get(0)?,
                name: r.get(1)?,
                section: r.get(2)?,
                chapters: r.get(3)?,
            })
        })?;
        rows.collect()
    }

    /// Number of chapters in a book for a translation (0 if absent).
    pub fn chapter_count(&self, translation_id: i64, book: i64) -> rusqlite::Result<i64> {
        self.conn
            .query_row(
                "SELECT chapters FROM books WHERE translation_id = ?1 AND nr = ?2",
                [translation_id, book],
                |r| r.get(0),
            )
            .optional()
            .map(|o| o.unwrap_or(0))
    }

    pub fn chapter(
        &self,
        translation_id: i64,
        book: i64,
        chapter: i64,
    ) -> rusqlite::Result<Vec<Verse>> {
        let mut stmt = self.conn.prepare(
            "SELECT book, chapter, verse, text FROM verses
             WHERE translation_id = ?1 AND book = ?2 AND chapter = ?3
             ORDER BY verse",
        )?;
        let rows = stmt.query_map([translation_id, book, chapter], |r| {
            Ok(Verse {
                book: r.get(0)?,
                chapter: r.get(1)?,
                verse: r.get(2)?,
                text: r.get(3)?,
            })
        })?;
        rows.collect()
    }

    /// Full-text search within one translation. Returns ranked verse snippets.
    pub fn search(
        &self,
        translation_id: i64,
        query: &str,
        limit: i64,
    ) -> rusqlite::Result<Vec<SearchHit>> {
        let match_expr = build_match(query);
        if match_expr.is_empty() {
            return Ok(Vec::new());
        }
        let mut stmt = self.conn.prepare(
            "SELECT v.book, b.name, v.chapter, v.verse,
                    snippet(verses_fts, 0, '«', '»', '…', 16) AS snip
             FROM verses_fts
             JOIN verses v ON v.id = verses_fts.rowid
             JOIN books b ON b.translation_id = v.translation_id AND b.nr = v.book
             WHERE verses_fts MATCH ?1 AND v.translation_id = ?2
             ORDER BY rank
             LIMIT ?3",
        )?;
        let rows = stmt.query_map(rusqlite::params![match_expr, translation_id, limit], |r| {
            Ok(SearchHit {
                book: r.get(0)?,
                book_name: r.get(1)?,
                chapter: r.get(2)?,
                verse: r.get(3)?,
                snippet: r.get(4)?,
            })
        })?;
        rows.collect()
    }
}

/// Turn free-text into a safe FTS5 MATCH expression: each token becomes a
/// quoted term ANDed together, so punctuation never breaks the query syntax.
fn build_match(query: &str) -> String {
    query
        .split_whitespace()
        .map(|t| t.replace('"', ""))
        .filter(|t| !t.is_empty())
        .map(|t| format!("\"{t}\""))
        .collect::<Vec<_>>()
        .join(" ")
}
