//! Bibles in Word documents.
//!
//! The format this is for is the Logos *personal book*: a DOCX in which each
//! verse is introduced by a milestone naming it, optionally with the verse's
//! words fenced as a Bible field:
//!
//! ```text
//! [[@BibleNB:Gen 1:1]]1 {{field-on:bible}}I begynnelsen skapte Gud…{{field-off:bible}}
//! ```
//!
//! The words up to the next milestone are the verse — only those inside the
//! fields when the document has fields, so headings and notes between verses
//! stay out. The label after `@Bible` is the document's reference scheme, not
//! a claim about which translation it holds, and is ignored.
//!
//! A DOCX without milestones is read as plain text, a paragraph a line, like a
//! PDF.
//!
//! Only `word/document.xml` (and the title in `docProps/core.xml`) is read: no
//! macro, link or embedded object is ever touched.

use std::io::Read;
use std::path::Path;

use regex::Regex;

use super::books::BookMatcher;
use super::text::{Event, Parser, Verses, clean};
use super::{Document, Error};

const W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";

fn part(zip: &mut zip::ZipArchive<std::fs::File>, name: &str) -> Result<String, Error> {
    let mut file = zip.by_name(name)?;
    if file.size() > 256 * 1024 * 1024 {
        return Err(format!("{name} is too large").into());
    }
    let mut out = String::new();
    file.read_to_string(&mut out)?;
    Ok(out)
}

pub fn read(path: &Path, books: &BookMatcher) -> Result<Document, Error> {
    let mut zip = zip::ZipArchive::new(std::fs::File::open(path)?)
        .map_err(|_| "This DOCX could not be opened (not a ZIP archive)")?;
    let xml = part(&mut zip, "word/document.xml").map_err(|_| "This DOCX has no document body")?;
    let (title, language) = part(&mut zip, "docProps/core.xml")
        .ok()
        .and_then(|core| {
            let doc = roxmltree::Document::parse(&core).ok()?;
            let get = |tag: &str| {
                doc.descendants()
                    .find(|n| n.tag_name().name() == tag)
                    .and_then(|n| n.text())
                    .unwrap_or("")
                    .trim()
                    .to_string()
            };
            Some((get("title"), get("language")))
        })
        .unwrap_or_default();

    let doc = roxmltree::Document::parse(&xml)?;
    let paragraphs: Vec<(String, bool)> = doc
        .descendants()
        .filter(|n| n.has_tag_name((W, "p")))
        .map(|p| {
            let text: String =
                p.descendants().filter(|n| n.has_tag_name((W, "t"))).filter_map(|n| n.text()).collect();
            let heading = p
                .descendants()
                .find(|n| n.has_tag_name((W, "pStyle")))
                .and_then(|s| s.attribute((W, "val")))
                .is_some_and(|v| v.to_ascii_lowercase().contains("heading"));
            (text, heading)
        })
        .collect();

    let all: String = paragraphs.iter().map(|(t, _)| t.as_str()).collect::<Vec<_>>().join("\n");
    let (verses, mut notes) = milestones(&all)?;
    if !verses.is_empty() {
        notes.insert(0, "Read Logos verse milestones.".into());
        return Ok(Document { title, language, verses, notes });
    }

    let mut parser = Parser::new(books);
    for (text, heading) in paragraphs {
        parser.feed(if heading { Event::Heading(text) } else { Event::Line(text) });
    }
    let notes = vec!["No Logos milestones: read as plain text, a paragraph a line.".into()];
    Ok(Document { title, language, verses: parser.finish(), notes })
}

/// The verses of a text marked with Logos milestones, and notes on any
/// milestone that named no verse.
fn milestones(text: &str) -> Result<(Verses, Vec<String>), Error> {
    let milestone = Regex::new(r"\[\[@Bible[A-Za-z0-9_]*:\s*([^\]]+?)\s+(\d+):(\d+)\]\]")?;
    let field = Regex::new(r"(?is)\{\{field-on:bible\}\}(.*?)(?:\{\{field-off:bible\}\}|$)")?;
    let marks: Vec<_> = milestone.captures_iter(text).collect();
    let mut verses = Verses::new();
    let mut unknown = 0usize;
    let mut duplicate = 0usize;
    for (i, cap) in marks.iter().enumerate() {
        let start = cap.get(0).unwrap().end();
        let end = marks.get(i + 1).map_or(text.len(), |next| next.get(0).unwrap().start());
        let span = &text[start..end];
        let Some(r) = crate::reference::parse(&format!("{} {}:{}", &cap[1], &cap[2], &cap[3])) else {
            unknown += 1;
            continue;
        };
        let (Some(chapter), Some(verse)) = (r.chapter, r.verse_start) else {
            unknown += 1;
            continue;
        };
        let words = if field.is_match(span) {
            field.captures_iter(span).map(|c| c[1].to_string()).collect::<Vec<_>>().join(" ")
        } else {
            // without a field the verse number is printed before the words
            let s = span.trim_start();
            let digits = s.len() - s.trim_start_matches(|c: char| c.is_ascii_digit()).len();
            if digits > 0 && s[..digits].parse() == Ok(verse) { s[digits..].to_string() } else { s.to_string() }
        };
        let words = clean(&words);
        if words.is_empty() {
            continue;
        }
        if verses.insert((r.book, chapter, verse), words).is_some() {
            duplicate += 1;
        }
    }
    let mut notes = Vec::new();
    if unknown > 0 {
        notes.push(format!("{unknown} milestone(s) named no verse and were skipped."));
    }
    if duplicate > 0 {
        notes.push(format!("{duplicate} verse(s) appeared twice; the later copy was kept."));
    }
    Ok((verses, notes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn milestones_with_and_without_fields() {
        let text = "Første Mosebok\n\
            [[@BibleNB:Gen 1:1]]1 {{field-on:bible}}I begynnelsen skapte Gud himmelen og jorden.{{field-off:bible}}\n\
            Overskrift\n\
            [[@BibleNB:Gen 1:2]]2 {{field-on:bible}}Jorden var øde og tom,{{field-off:bible}}\n\
            [[@Bible:1Pet 2:3]]3 if you have tasted\n\
            [[@Bible:Nowhere 1:1]] nothing";
        let (v, notes) = milestones(text).unwrap();
        assert_eq!(v[&(1, 1, 1)], "I begynnelsen skapte Gud himmelen og jorden.");
        // the heading between the verses stays out
        assert_eq!(v[&(1, 1, 2)], "Jorden var øde og tom,");
        assert_eq!(v[&(60, 2, 3)], "if you have tasted");
        assert_eq!(v.len(), 3);
        assert_eq!(notes.len(), 1);
    }
}
