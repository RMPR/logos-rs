//! Translations the reader imports from their own files.
//!
//! A licensed Bible a reader owns — an EPUB, a PDF, a Logos personal-book
//! DOCX — becomes a translation like the bundled ones: readable in a column,
//! searchable, and, once aligned to the Hebrew and Greek, a clickable reverse
//! interlinear. Its text stays on the reader's machine, in a database of its
//! own beside the bundled one (see [`store`]).
//!
//! Each reader turns its format into verses; [`read`] picks the reader by the
//! file's extension.

pub mod align;
pub mod books;
pub mod docx;
pub mod epub;
pub mod greek;
pub mod pdf;
pub mod store;
pub mod text;

use std::collections::HashMap;
use std::path::Path;

use books::BookMatcher;
use rusqlite::{Connection, OpenFlags};
use text::Verses;

pub type Error = Box<dyn std::error::Error + Send + Sync>;

/// A Bible read from a file, before it is stored.
#[derive(Debug, Default)]
pub struct Document {
    /// The title the file gives itself, else its name.
    pub title: String,
    /// A language code ("en", "fr", "nb"…), from the file's metadata or
    /// guessed from its words; empty when neither says.
    pub language: String,
    pub verses: Verses,
    /// What the reader should know about how the file was read.
    pub notes: Vec<String>,
}

/// The file types [`read`] accepts, for the file picker.
pub const EXTENSIONS: &[&str] = &["epub", "pdf", "docx", "txt"];

pub fn read(path: &Path, books: &BookMatcher) -> Result<Document, Error> {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    let mut doc = match ext.as_str() {
        "epub" => epub::read(path, books)?,
        "docx" => docx::read(path, books)?,
        "pdf" => pdf::read(path, books)?,
        "txt" => pdf::read_text(&std::fs::read_to_string(path)?, books),
        _ => return Err(format!("Cannot import .{ext} files: choose an EPUB, PDF, DOCX or text file").into()),
    };
    // only the 66 books of the canon are kept
    doc.verses.retain(|&(b, _, _), _| (1..=66).contains(&b));
    if doc.verses.is_empty() {
        return Err("No Bible verses were found in this file".into());
    }
    if doc.title.trim().is_empty() {
        doc.title = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    }
    doc.language = normalize_language(&doc.language);
    if doc.language.is_empty() {
        doc.language = guess_language(&doc.verses).to_string();
    }
    Ok(doc)
}

/// The words of a verse as a reader clicks them: split at spaces, with
/// punctuation that stands on its own (the French and Latin " : ", an
/// opening « ) joined to its neighbour by a no-break space, so a colon is
/// never a word of its own. The aligner and the stored words both count
/// words this way.
pub fn words_of(text: &str) -> Vec<String> {
    word_spans(text).into_iter().map(|(w, _)| w).collect()
}

/// [`words_of`], with the whitespace-separated tokens each word is made of
/// (their indices): what carries over a tagging made token by token.
pub fn word_spans(text: &str) -> Vec<(String, std::ops::Range<usize>)> {
    let mut words: Vec<(String, std::ops::Range<usize>)> = Vec::new();
    // leading punctuation, waiting for a word
    let mut pending: Option<(String, usize)> = None;
    for (i, token) in text.split_whitespace().enumerate() {
        if token.chars().any(char::is_alphanumeric) {
            words.push(match pending.take() {
                Some((p, start)) => (p + "\u{a0}" + token, start..i + 1),
                None => (token.to_string(), i..i + 1),
            });
        } else if let Some((last, span)) = words.last_mut() {
            last.push('\u{a0}');
            last.push_str(token);
            span.end = i + 1;
        } else {
            match &mut pending {
                Some((p, _)) => {
                    p.push('\u{a0}');
                    p.push_str(token);
                }
                None => pending = Some((token.to_string(), i)),
            }
        }
    }
    if let Some((p, start)) = pending {
        let end = text.split_whitespace().count();
        words.push((p, start..end));
    }
    words
}

/// A book matcher that knows every book name in the bundled database, in
/// all its languages.
pub fn matcher(bundled: &Connection) -> Result<BookMatcher, Error> {
    let mut stmt = bundled.prepare("SELECT DISTINCT nr, name FROM books")?;
    let names = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<Result<Vec<_>, _>>()?;
    Ok(BookMatcher::new(names))
}

/// What the reader chose for a translation about to be stored.
pub struct Request {
    pub code: String,
    pub name: String,
    pub lang: String,
}

/// How an import went.
pub struct Report {
    pub id: i64,
    pub verses: usize,
    pub words: usize,
    /// Words given a Strong's number, i.e. clickable into the originals.
    pub linked: usize,
}

/// Check a reader's choices against the translations already there.
pub fn validate(req: &Request, taken: &[String]) -> Result<(), String> {
    let code = req.code.trim();
    if !(2..=12).contains(&code.len()) || !code.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_') {
        return Err("The code must be 2 to 12 capital letters or digits".into());
    }
    if taken.iter().any(|t| t == code) {
        return Err(format!("{code} is already in use: choose another code"));
    }
    if req.name.trim().is_empty() {
        return Err("Give the translation a name".into());
    }
    Ok(())
}

/// A short code for a title, not one of `taken`: a capitalised abbreviation
/// already in it ("ESV Bible" → ESV), else its initials ("New American
/// Standard Bible" → NASB).
pub fn suggest_code(title: &str, taken: &[String]) -> String {
    let words: Vec<&str> = title.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).collect();
    let abbreviation = words
        .iter()
        .find(|w| (2..=6).contains(&w.len()) && w.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit()))
        .map(|w| w.to_string());
    let base = abbreviation.unwrap_or_else(|| {
        words
            .iter()
            .filter(|w| w.chars().next().is_some_and(char::is_uppercase))
            .filter_map(|w| w.chars().next())
            .flat_map(|c| unicode_normalization::UnicodeNormalization::nfkd(c).next())
            .filter(char::is_ascii_alphabetic)
            .map(|c| c.to_ascii_uppercase())
            .take(6)
            .collect()
    });
    // a one-word title: its first letters
    let base = if base.len() < 2 {
        let first: String = words
            .first()
            .map(|w| unicode_normalization::UnicodeNormalization::nfkd(*w).filter(char::is_ascii_alphabetic).take(4).collect())
            .unwrap_or_default();
        if first.len() < 2 { "MY".to_string() } else { first.to_ascii_uppercase() }
    } else {
        base
    };
    let mut code = base.clone();
    let mut n = 2;
    while taken.contains(&code) {
        code = format!("{base}{n}");
        n += 1;
    }
    code
}

/// Align `doc` to the Hebrew and Greek and store it. `progress` hears what
/// is happening and the fraction done.
pub fn import(
    bundled: &Path,
    imports: &Path,
    req: &Request,
    doc: &Document,
    progress: &mut dyn FnMut(&str, f32),
) -> Result<Report, Error> {
    let main = Connection::open_with_flags(bundled, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    progress("Reading the Hebrew and Greek…", 0.0);
    let originals = store::originals(&main)?;
    progress("Linking words to the Hebrew and Greek…", 0.05);
    let alignment = align::align(&doc.verses, &originals, &req.lang, &mut |f| progress("Linking words to the Hebrew and Greek…", 0.05 + 0.85 * f));

    // book names in the translation's language, if a bundled text has it
    let mut book_names: HashMap<i64, String> = HashMap::new();
    let mut stmt = main.prepare(
        "SELECT b.nr, b.name FROM books b JOIN translations t ON t.id = b.translation_id
         WHERE t.lang = ?1 ORDER BY t.sort DESC",
    )?;
    for row in stmt.query_map([&req.lang], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))? {
        let (nr, name) = row?;
        book_names.insert(nr, name);
    }

    progress("Saving…", 0.9);
    let mut conn = store::open(imports)?;
    let t = store::NewTranslation { code: req.code.trim(), name: req.name.trim(), lang: req.lang.trim(), book_names: &book_names };
    let id = store::save(&mut conn, &t, &doc.verses, &alignment)?;
    let words = doc.verses.values().map(|v| words_of(v).len()).sum();
    let linked = alignment.values().flatten().filter(|t| t.is_some()).count();
    progress("Done", 1.0);
    Ok(Report { id, verses: doc.verses.len(), words, linked })
}

/// Link again, with the current aligner, every imported translation linked
/// by an older one; returns how many were. Runs at start-up, off the UI.
pub fn relink_stale(bundled: &Path, imports: &Path) -> Result<usize, Error> {
    let mut conn = store::open(imports)?;
    let stale = store::stale(&conn)?;
    if stale.is_empty() {
        return Ok(0);
    }
    let main = Connection::open_with_flags(bundled, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let originals = store::originals(&main)?;
    for (id, lang) in &stale {
        let verses = store::verses(&conn, *id)?;
        let alignment = align::align(&verses, &originals, lang, &mut |_| {});
        store::relink(&mut conn, *id, &verses, &alignment)?;
    }
    Ok(stale.len())
}

/// "en-US" → "en"; Norwegian in any of its codes → "nb", the code the
/// bundled Bokmål text uses.
pub fn normalize_language(code: &str) -> String {
    let base = code.trim().split(['-', '_']).next().unwrap_or("").to_ascii_lowercase();
    match base.as_str() {
        "no" | "nob" | "nor" => "nb".into(),
        "he" | "heb" => "hbo".into(),
        "el" | "gre" | "ell" => "grc".into(),
        "eng" => "en".into(),
        "fre" | "fra" => "fr".into(),
        _ => base,
    }
}

/// The language whose commonest words turn up most in the text, or "" when
/// none stands out.
pub fn guess_language(verses: &Verses) -> &'static str {
    const WORDS: &[(&str, &[&str])] = &[
        ("en", &["the", "and", "of", "to", "that", "he", "shall", "his"]),
        ("fr", &["et", "le", "la", "les", "de", "des", "il", "que"]),
        ("nb", &["og", "det", "som", "han", "til", "for", "på", "ikke"]),
        ("de", &["und", "der", "die", "das", "er", "zu", "nicht", "sie"]),
        ("es", &["y", "de", "el", "la", "que", "los", "en", "a"]),
        ("pt", &["e", "de", "o", "a", "que", "os", "do", "da"]),
        ("it", &["e", "di", "il", "che", "la", "a", "non", "si"]),
        ("nl", &["en", "de", "het", "van", "die", "zij", "hij", "niet"]),
    ];
    let mut counts = vec![0usize; WORDS.len()];
    for text in verses.values().take(2000) {
        for word in text.split_whitespace() {
            let w = word.trim_matches(|c: char| !c.is_alphanumeric()).to_lowercase();
            for (i, (_, common)) in WORDS.iter().enumerate() {
                if common.contains(&w.as_str()) {
                    counts[i] += 1;
                }
            }
        }
    }
    let (best, &n) = counts.iter().enumerate().max_by_key(|(_, n)| **n).unwrap();
    let second = counts.iter().enumerate().filter(|(i, _)| *i != best).map(|(_, n)| *n).max().unwrap_or(0);
    if n > 50 && n * 10 > second * 13 { WORDS[best].0 } else { "" }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn language_codes() {
        assert_eq!(normalize_language("en-US"), "en");
        assert_eq!(normalize_language("no"), "nb");
        assert_eq!(normalize_language("nb-NO"), "nb");
        assert_eq!(normalize_language("FR"), "fr");
    }

    #[test]
    fn punctuation_joins_its_word() {
        assert_eq!(words_of("super faciem abyssi : et spiritus"), ["super", "faciem", "abyssi\u{a0}:", "et", "spiritus"]);
        assert_eq!(words_of("« Que la lumière paraisse ! »"), ["«\u{a0}Que", "la", "lumière", "paraisse\u{a0}!\u{a0}»"]);
        assert_eq!(words_of("…"), ["…"]);
        let spans: Vec<_> = word_spans("« Oui , dit-il").into_iter().map(|(_, s)| s).collect();
        assert_eq!(spans, [0..3, 3..4]);
        assert!(words_of("  ").is_empty());
    }

    #[test]
    fn codes_from_titles() {
        let taken = vec!["ESV".to_string(), "NB".to_string()];
        assert_eq!(suggest_code("New American Standard Bible", &taken), "NASB");
        assert_eq!(suggest_code("ESV Bible", &taken), "ESV2");
        assert_eq!(suggest_code("Bibelen - bokmål", &taken), "BIBE");
        assert_eq!(suggest_code("La Bible en français courant", &taken), "LB");
        assert!(validate(&Request { code: "NASB".into(), name: "x".into(), lang: "en".into() }, &taken).is_ok());
        assert!(validate(&Request { code: "ESV".into(), name: "x".into(), lang: "en".into() }, &taken).is_err());
        assert!(validate(&Request { code: "nasb".into(), name: "x".into(), lang: "en".into() }, &taken).is_err());
    }

    #[test]
    fn guesses_from_common_words() {
        let mut v = Verses::new();
        for i in 1..40 {
            v.insert((1, 1, i), "And God said to the man that he shall keep his way".into());
        }
        assert_eq!(guess_language(&v), "en");
        let mut v = Verses::new();
        for i in 1..40 {
            v.insert((1, 1, i), "Og Gud sa til mannen at han ikke skulle spise det som var på treet".into());
        }
        assert_eq!(guess_language(&v), "nb");
    }
}
