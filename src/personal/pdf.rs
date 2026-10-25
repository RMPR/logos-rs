//! Bibles in PDF, and in plain text.
//!
//! A PDF has no structure to read, only lines of text in reading order. The
//! layout of a Bible still shows through them: a header naming the book and
//! chapter ("Genesis 21", often repeated on every page as a running header),
//! and verses that start a line with their number. The line parser in
//! [`super::text`] works from exactly that, so this is best-effort by nature:
//! a PDF laid out in two columns, or with verse numbers inside a paragraph,
//! comes out with verses missing, and the import report says how many.

use std::path::Path;

use super::books::BookMatcher;
use super::text::{Event, Parser};
use super::{Document, Error};

pub fn read(path: &Path, books: &BookMatcher) -> Result<Document, Error> {
    let bytes = std::fs::read(path)?;
    // The extractor has panics on PDFs it does not understand; one of those
    // must fail the import, not take the app down.
    let text = std::panic::catch_unwind(|| pdf_extract::extract_text_from_mem(&bytes))
        .map_err(|_| "This PDF could not be read")?
        .map_err(|e| format!("This PDF could not be read: {e}"))?;
    let mut doc = read_text(&text, books);
    doc.notes.push(
        "Read from a PDF: verses are found by their numbers at the start of a line.".into(),
    );
    Ok(doc)
}

/// Verses from text laid out a line at a time.
pub fn read_text(text: &str, books: &BookMatcher) -> Document {
    let mut parser = Parser::new(books);
    for line in text.lines() {
        parser.feed(Event::Line(line.to_string()));
    }
    let mut notes = Vec::new();
    if parser.inferred_books > 0 {
        notes.push(format!(
            "{} book(s) were recognised by their position, not their name.",
            parser.inferred_books
        ));
    }
    Document { verses: parser.finish(), notes, ..Document::default() }
}
