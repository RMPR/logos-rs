//! Bibles in EPUB.
//!
//! Publishers mark verses each their own way. Three layouts are read exactly,
//! as the former `tools/epub2json.py` read them:
//!
//! - **Crossway (ESV)**: `bNN.….text.html` files, book from the file name,
//!   `span.chapter-num` / `span.verse-num` markers, footnotes and
//!   cross-references stripped.
//! - **Société biblique française (BFC)**: `Book-N.xml` chapter files and
//!   `<a class="verse" id="vGen.1.1"/>` markers; notes and the
//!   deuterocanonical books dropped.
//! - **Bibelselskapet (Bokmål 2011)**: `NNNCODE.xhtml` files, book from the
//!   number, `span.versenumberdropcap` starting a chapter and
//!   `span.versenumber` a verse.
//!
//! Any other EPUB is read in its spine order by the general parser in
//! [`super::text`], from what most Bible EPUBs share: headings naming books
//! and chapters, and verse numbers in elements of their own (a class with
//! "verse" in it, or a bare `<sup>`). Notes and cross-references are skipped.

use std::collections::HashMap;
use std::io::Read;
use std::path::Path;

use regex::Regex;
use scraper::{ElementRef, Html, Node};

use super::books::BookMatcher;
use super::text::{Event, Parser, Verses, clean};
use super::{Document, Error};

type Archive = zip::ZipArchive<std::fs::File>;

/// Elements that end a line or paragraph. Publishers set poetry one block per
/// line with no whitespace between the blocks, so the words either side would
/// fuse ("Dieu,la voûte"); a block boundary is always a word boundary.
const BLOCK: &[&str] = &[
    "p", "div", "br", "li", "ul", "ol", "tr", "td", "th", "table", "blockquote", "section",
    "article", "hr", "h1", "h2", "h3", "h4", "h5", "h6",
];

const OSIS: &[&str] = &[
    "Gen", "Exod", "Lev", "Num", "Deut", "Josh", "Judg", "Ruth", "1Sam", "2Sam", "1Kgs", "2Kgs",
    "1Chr", "2Chr", "Ezra", "Neh", "Esth", "Job", "Ps", "Prov", "Eccl", "Song", "Isa", "Jer",
    "Lam", "Ezek", "Dan", "Hos", "Joel", "Amos", "Obad", "Jonah", "Mic", "Nah", "Hab", "Zeph",
    "Hag", "Zech", "Mal", "Matt", "Mark", "Luke", "John", "Acts", "Rom", "1Cor", "2Cor", "Gal",
    "Eph", "Phil", "Col", "1Thess", "2Thess", "1Tim", "2Tim", "Titus", "Phlm", "Heb", "Jas",
    "1Pet", "2Pet", "1John", "2John", "3John", "Jude", "Rev",
];

/// The publisher whose layout an EPUB has, if it is one read exactly:
/// "Crossway", "Société biblique française" or "Bibelselskapet".
pub fn publisher(path: &Path) -> Option<&'static str> {
    let zip = zip::ZipArchive::new(std::fs::File::open(path).ok()?).ok()?;
    let names: Vec<String> = zip.file_names().filter_map(|n| n.ok().map(|n| n.into_owned())).collect();
    let count = |re: &str| {
        let re = Regex::new(re).unwrap();
        names.iter().filter(|n| re.is_match(n)).count()
    };
    if count(r"/b(\d{2})\.\d+\.[^/]*\.text\.html$") > 0 {
        Some("Crossway")
    } else if count(r"(?:^|/)[A-Za-z0-9]+-\d+\.xml$") > 50 {
        Some("Société biblique française")
    } else if count(r"(?:^|/)\d{3}[A-Z0-9]+(?:-[A-Z])?\.xhtml$") > 50 {
        Some("Bibelselskapet")
    } else {
        None
    }
}

pub fn read(path: &Path, books: &BookMatcher) -> Result<Document, Error> {
    let mut zip = zip::ZipArchive::new(std::fs::File::open(path)?)
        .map_err(|_| "This EPUB could not be opened (not a ZIP archive)")?;
    let names: Vec<String> = zip.file_names().filter_map(|n| n.ok().map(|n| n.into_owned())).collect();
    let (title, language, spine) = package(&mut zip).unwrap_or_default();

    let crossway = Regex::new(r"/b(\d{2})\.\d+\.[^/]*\.text\.html$")?;
    let bsf = Regex::new(r"(?:^|/)[A-Za-z0-9]+-\d+\.xml$")?;
    let bibelselskapet = Regex::new(r"(?:^|/)\d{3}[A-Z0-9]+(?:-[A-Z])?\.xhtml$")?;

    let mut verses = Verses::new();
    let mut notes = Vec::new();
    let layout;
    if names.iter().any(|n| crossway.is_match(n)) {
        layout = "Crossway";
        crossway_layout(&mut zip, &names, &crossway, &mut verses)?;
    } else if names.iter().filter(|n| bsf.is_match(n)).count() > 50 {
        layout = "Société biblique française";
        bsf_layout(&mut zip, &names, &bsf, &mut verses)?;
    } else if names.iter().filter(|n| bibelselskapet.is_match(n)).count() > 50 {
        layout = "Bibelselskapet";
        bibelselskapet_layout(&mut zip, &names, &mut verses)?;
    } else {
        layout = "";
        let mut parser = Parser::new(books);
        // the spine as the archive names its entries, else the file order
        let spine: Vec<String> = spine.into_iter().filter(|n| names.contains(n)).collect();
        let files = if spine.is_empty() { sorted_documents(&names) } else { spine };
        for name in files {
            let Ok(doc) = document(&mut zip, &name) else { continue };
            walk_generic(doc.root_element(), &mut parser);
        }
        if parser.inferred_books > 0 {
            notes.push(format!(
                "{} book(s) were recognised by their position, not their name.",
                parser.inferred_books
            ));
        }
        verses = parser.finish();
    }
    if !layout.is_empty() {
        notes.push(format!("Read as a {layout} EPUB."));
    }
    let verses = verses.into_iter().map(|(k, v)| (k, clean(&v))).filter(|(_, v)| !v.is_empty()).collect();
    Ok(Document { title, language, verses, notes })
}

/// An XHTML entry, parsed. The parser is an HTML one, which reads
/// `<a id="v1"/>` as an *open* `a` that swallows the text after it; XHTML
/// means it as empty, so such tags are expanded first.
fn document(zip: &mut Archive, name: &str) -> Result<Html, Error> {
    static SELF_CLOSED: std::sync::LazyLock<Regex> =
        std::sync::LazyLock::new(|| Regex::new(r"<([A-Za-z][\w:-]*)(\s[^<>]*?)?/>").unwrap());
    const VOID: &[&str] = &["br", "hr", "img", "meta", "link", "input", "col", "area", "base", "wbr"];
    let raw = entry(zip, name)?;
    let expanded = SELF_CLOSED.replace_all(&raw, |c: &regex::Captures| {
        let tag = &c[1];
        let attrs = c.get(2).map_or("", |m| m.as_str());
        if VOID.contains(&tag.to_ascii_lowercase().as_str()) {
            format!("<{tag}{attrs}/>")
        } else {
            format!("<{tag}{attrs}></{tag}>")
        }
    });
    Ok(Html::parse_document(&expanded))
}

fn entry(zip: &mut Archive, name: &str) -> Result<String, Error> {
    let mut file = zip.by_name(name)?;
    // a chapter is a few hundred kilobytes; anything vastly larger is not one
    if file.size() > 64 * 1024 * 1024 {
        return Err("EPUB entry too large".into());
    }
    let mut out = String::new();
    file.read_to_string(&mut out)?;
    Ok(out)
}

/// The book's title, language and spine (reading order), from its package
/// document.
fn package(zip: &mut Archive) -> Result<(String, String, Vec<String>), Error> {
    let container = entry(zip, "META-INF/container.xml")?;
    let doc = roxmltree::Document::parse(&container)?;
    let opf_path = doc
        .descendants()
        .find(|n| n.has_tag_name("rootfile"))
        .and_then(|n| n.attribute("full-path"))
        .ok_or("no package document")?
        .to_string();
    let base = opf_path.rsplit_once('/').map_or("", |(dir, _)| dir).to_string();
    let opf = entry(zip, &opf_path)?;
    let doc = roxmltree::Document::parse(&opf)?;
    let text_of = |tag: &str| {
        doc.descendants()
            .find(|n| n.tag_name().name() == tag)
            .and_then(|n| n.text())
            .unwrap_or("")
            .trim()
            .to_string()
    };
    let (title, language) = (text_of("title"), text_of("language"));
    let manifest: HashMap<&str, &str> = doc
        .descendants()
        .filter(|n| n.tag_name().name() == "item")
        .filter_map(|n| Some((n.attribute("id")?, n.attribute("href")?)))
        .collect();
    let spine = doc
        .descendants()
        .filter(|n| n.tag_name().name() == "itemref")
        .filter_map(|n| manifest.get(n.attribute("idref")?))
        .map(|href| {
            let href = percent_decode(href.split('#').next().unwrap_or(href));
            if base.is_empty() { href.to_string() } else { format!("{base}/{href}") }
        })
        .collect();
    Ok((title, language, spine))
}

/// An href as the archive names the entry: `%20` → space, and so on.
fn percent_decode(href: &str) -> String {
    let bytes = href.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && let Some(b) = href.get(i + 1..i + 3).and_then(|h| u8::from_str_radix(h, 16).ok())
        {
            out.push(b);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8(out).unwrap_or_else(|_| href.to_string())
}

fn sorted_documents(names: &[String]) -> Vec<String> {
    let mut docs: Vec<String> = names
        .iter()
        .filter(|n| n.ends_with(".html") || n.ends_with(".xhtml") || n.ends_with(".htm"))
        .cloned()
        .collect();
    docs.sort();
    docs
}

fn classes(e: &ElementRef) -> Vec<String> {
    e.value().classes().map(str::to_string).collect()
}

fn has_class(e: &ElementRef, class: &str) -> bool {
    e.value().classes().any(|c| c == class)
}

fn element_text(e: &ElementRef) -> String {
    e.text().collect()
}

/// The first number in a marker: "12" → 12, and a merged range "11–12" →
/// 11 (the verse it is filed under), never 1112.
fn number_in(text: &str) -> Option<i64> {
    let digits: String =
        text.chars().skip_while(|c| !c.is_ascii_digit()).take_while(char::is_ascii_digit).collect();
    digits.parse().ok()
}

fn body(doc: &Html) -> Option<ElementRef<'_>> {
    doc.root_element().children().filter_map(ElementRef::wrap).find(|e| e.value().name() == "body")
}

/// Where text accumulates in the exact layouts: (book, chapter, verse).
struct Sink<'a> {
    verses: &'a mut Verses,
    book: i64,
    chapter: i64,
    verse: i64,
}

impl Sink<'_> {
    fn add(&mut self, s: &str) {
        if self.book > 0 && self.chapter > 0 && self.verse > 0 {
            self.verses.entry((self.book, self.chapter, self.verse)).or_default().push_str(s);
        }
    }
}

/// Walk `e`'s children, handing elements to `visit`; a visitor returns true
/// when it has dealt with an element, else it is descended into, padded with
/// spaces if it is a block.
fn walk_exact(e: ElementRef, sink: &mut Sink, visit: &mut dyn FnMut(&ElementRef, &mut Sink) -> bool) {
    for child in e.children() {
        match child.value() {
            Node::Text(t) => sink.add(t),
            Node::Element(_) => {
                let Some(el) = ElementRef::wrap(child) else { continue };
                if visit(&el, sink) {
                    continue;
                }
                let block = BLOCK.contains(&el.value().name());
                if block {
                    sink.add(" ");
                }
                walk_exact(el, sink, visit);
                if block {
                    sink.add(" ");
                }
            }
            _ => {}
        }
    }
}

fn crossway_layout(zip: &mut Archive, names: &[String], file: &Regex, verses: &mut Verses) -> Result<(), Error> {
    let mut files: Vec<&String> = names.iter().filter(|n| file.is_match(n)).collect();
    files.sort();
    for name in files {
        let book: i64 = file.captures(name).and_then(|c| c[1].parse().ok()).unwrap_or(0);
        let html = document(zip, name)?;
        let Some(body) = body(&html) else { continue };
        let mut sink = Sink { verses: &mut *verses, book, chapter: 0, verse: 0 };
        walk_exact(body, &mut sink, &mut |el, sink| {
            let cls = classes(el);
            let has = |c: &str| cls.iter().any(|x| x == c);
            if has("footnote") || has("crossref") || has("heading") || has("book-name") {
                return true;
            }
            if has("chapter-num") {
                if let Some(n) = number_in(&element_text(el)) {
                    sink.chapter = n;
                }
                sink.verse = 1;
                return true;
            }
            if has("verse-num") {
                if let Some(n) = number_in(&element_text(el)) {
                    sink.verse = n;
                }
                return true;
            }
            false
        });
    }
    Ok(())
}

fn bsf_layout(zip: &mut Archive, names: &[String], file: &Regex, verses: &mut Verses) -> Result<(), Error> {
    let marker = Regex::new(r"^v([A-Za-z0-9]+)\.(\d+)\.(\d+)$")?;
    let mut files: Vec<&String> = names.iter().filter(|n| file.is_match(n)).collect();
    files.sort();
    for name in files {
        let html = document(zip, name)?;
        let Some(body) = body(&html) else { continue };
        let mut sink = Sink { verses: &mut *verses, book: 0, chapter: 0, verse: 0 };
        walk_exact(body, &mut sink, &mut |el, sink| {
            let tag = el.value().name();
            if matches!(tag, "h1" | "h2" | "h3" | "h4") {
                return true;
            }
            if (tag == "span" && has_class(el, "displayReference"))
                || (tag == "a" && has_class(el, "note-link-in-text"))
            {
                return true;
            }
            if tag == "a" && has_class(el, "verse") {
                let id = el.value().attr("id").unwrap_or("");
                let found = marker.captures(id).and_then(|c| {
                    let book = OSIS.iter().position(|o| *o == &c[1])? as i64 + 1;
                    Some((book, c[2].parse().ok()?, c[3].parse().ok()?))
                });
                // a code outside the canon (a deuterocanonical book) mutes the
                // text until the next canonical marker
                (sink.book, sink.chapter, sink.verse) = found.unwrap_or((0, 0, 0));
                return true;
            }
            false
        });
    }
    Ok(())
}

fn bibelselskapet_layout(zip: &mut Archive, names: &[String], verses: &mut Verses) -> Result<(), Error> {
    let file = Regex::new(r"(?:^|/)(\d{3})([A-Z0-9]+?)(?:-([A-Z]))?\.xhtml$")?;
    let mut files: Vec<(i64, String, &String)> = names
        .iter()
        .filter_map(|n| {
            let c = file.captures(n)?;
            Some((c[1].parse().ok()?, c.get(3).map_or("", |m| m.as_str()).to_string(), n))
        })
        .collect();
    // the base file of a book before its continuations (-B, -C…)
    files.sort();
    let (mut book, mut chapter, mut verse) = (0, 0, 0);
    for (nr, _, name) in files {
        if nr > 66 {
            continue; // glossary and other back matter
        }
        if nr != book {
            (book, chapter, verse) = (nr, 0, 0);
        }
        let html = document(zip, name)?;
        let Some(body) = body(&html) else { continue };
        let mut sink = Sink { verses: &mut *verses, book, chapter, verse };
        walk_exact(body, &mut sink, &mut |el, sink| {
            let cls = classes(el);
            let has = |c: &str| cls.iter().any(|x| x == c);
            if has("versenumberdropcap") {
                let text = element_text(el);
                match number_in(&text) {
                    // a numeric drop cap is the chapter number
                    Some(n) => (sink.chapter, sink.verse) = (n, 1),
                    // a decorative initial letter (a one-chapter book): keep it
                    None => {
                        (sink.chapter, sink.verse) = (sink.chapter.max(1), 1);
                        sink.add(&text);
                    }
                }
                return true;
            }
            if has("dropcap") {
                (sink.chapter, sink.verse) = (sink.chapter.max(1), sink.verse.max(1));
                sink.add(&element_text(el));
                return true;
            }
            if has("versenumber") {
                if let Some(n) = number_in(&element_text(el)) {
                    sink.verse = n;
                }
                return true;
            }
            has("verseheader")
        });
        (chapter, verse) = (sink.chapter, sink.verse);
    }
    Ok(())
}

/// Whether an element is a note, a note reference, a cross-reference or a
/// reference label — apparatus, not text.
fn is_apparatus(el: &ElementRef) -> bool {
    let v = el.value();
    if matches!(v.name(), "script" | "style" | "head" | "aside" | "nav") {
        return true;
    }
    let epub_type = v.attrs().find(|(k, _)| k.ends_with("type")).map_or("", |(_, t)| t);
    if ["footnote", "noteref", "endnote", "rearnote"].iter().any(|t| epub_type.contains(t)) {
        return true;
    }
    v.classes().any(|c| {
        let c = c.to_ascii_lowercase();
        c.contains("note") || c.contains("crossref") || c.contains("xref") || c.contains("reference") || c == "fn"
    })
}

/// Whether an element's class says it is a heading of some kind: a section
/// title, or a book's name set as a banner.
fn is_heading_class(el: &ElementRef) -> bool {
    el.value().classes().any(|c| {
        let c = c.to_ascii_lowercase();
        ["heading", "title", "header", "bookname", "book-name"].iter().any(|k| c.contains(k))
    })
}

/// The number an element marks, if its class names it as a `kind` marker
/// ("chap", "drop", "vers") — or, for verses, it is a bare `<sup>`.
fn marker(el: &ElementRef, kind: &str) -> Option<i64> {
    let named = el.value().classes().any(|c| c.to_ascii_lowercase().contains(kind));
    let bare_sup = kind == "vers"
        && el.value().name() == "sup"
        && !el.children().any(|c| ElementRef::wrap(c).is_some_and(|e| e.value().name() == "a"));
    if !named && !bare_sup {
        return None;
    }
    let text = element_text(el);
    let text = text.trim();
    // only the number: "12", "12 ", "12:" — not a sentence that contains one
    if text.is_empty() || text.chars().any(|c| c.is_alphabetic()) || text.len() > 8 {
        return None;
    }
    number_in(text).filter(|n| (1..=176).contains(n))
}

/// A verse reference carried by an element's id, as some publishers mark
/// verses with an empty anchor: `vGen.1.1` → (book, chapter, verse), book 0
/// for a code outside the canon.
fn id_reference(el: &ElementRef) -> Option<(i64, i64, i64)> {
    static ID: std::sync::LazyLock<Regex> =
        std::sync::LazyLock::new(|| Regex::new(r"^v?([1-3]?[A-Z][A-Za-z]+)\.(\d+)\.(\d+)$").unwrap());
    let c = ID.captures(el.value().attr("id")?)?;
    let book = OSIS.iter().position(|o| *o == &c[1]).map_or(0, |i| i as i64 + 1);
    Some((book, c[2].parse().ok()?, c[3].parse().ok()?))
}

fn walk_generic(e: ElementRef, parser: &mut Parser) {
    for child in e.children() {
        match child.value() {
            Node::Text(t) => parser.feed(Event::Text(t.to_string())),
            Node::Element(_) => {
                let Some(el) = ElementRef::wrap(child) else { continue };
                if is_apparatus(&el) {
                    continue;
                }
                let tag = el.value().name();
                if matches!(tag, "h1" | "h2" | "h3" | "h4" | "h5" | "h6") || is_heading_class(&el) {
                    parser.feed(Event::Heading(clean(&element_text(&el))));
                    continue;
                }
                // A chapter number is often set as a drop cap; the class may
                // well say "verse" too ("versenumberdropcap"), so chapters are
                // tried first.
                if let Some(n) = marker(&el, "chap").or_else(|| marker(&el, "drop")) {
                    parser.feed(Event::Chapter(n));
                    continue;
                }
                if let Some(n) = marker(&el, "vers") {
                    parser.feed(Event::Verse(n));
                    continue;
                }
                let empty = el.text().all(|t| t.trim().is_empty());
                if empty && let Some((book, chapter, verse)) = id_reference(&el) {
                    parser.feed(Event::Book(book));
                    parser.feed(Event::Chapter(chapter));
                    parser.feed(Event::Verse(verse));
                    continue;
                }
                let block = BLOCK.contains(&tag);
                if block {
                    parser.feed(Event::Text(" ".into()));
                }
                walk_generic(el, parser);
                if block {
                    parser.feed(Event::Text(" ".into()));
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn verse_markers() {
        assert_eq!(super::number_in("12"), Some(12));
        assert_eq!(super::number_in("\u{a0}11–12 "), Some(11));
        assert_eq!(super::number_in("v. 3"), Some(3));
        assert_eq!(super::number_in("A"), None);
    }

    #[test]
    fn hrefs_decode() {
        assert_eq!(super::percent_decode("Text/My%20Bible%C3%A9.xhtml"), "Text/My Bibleé.xhtml");
        assert_eq!(super::percent_decode("100%"), "100%");
    }
}
