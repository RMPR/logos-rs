//! Turning a stream of headings, markers and text into verses.
//!
//! Every reader that does not know its source's layout exactly — a PDF, an
//! EPUB from an unknown publisher, a DOCX without Logos milestones — reduces
//! the document to [`Event`]s and lets this one state machine decide where
//! each book, chapter and verse begins. It trusts structure where the source
//! has it (an EPUB's verse-number elements) and checks it where it is only
//! inferred (a number at the start of a PDF line might be a verse, or the
//! start of "12 men went up").

use std::collections::BTreeMap;

use super::books::BookMatcher;

/// (book, chapter, verse) → text.
pub type Verses = BTreeMap<(i64, i64, i64), String>;

#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// A heading: a book's name, "Genesis 3", "Chapter 3", or a section title
    /// (which is not Bible text and is dropped).
    Heading(String),
    /// A book the source identifies by something other than its name (a
    /// file name, an element id). 0 for one outside the canon: its text is
    /// dropped until the next book.
    Book(i64),
    /// A chapter number the source marks as such. Its verse 1 often carries
    /// no number of its own, so text that follows belongs to verse 1.
    Chapter(i64),
    /// A verse number the source marks as such.
    Verse(i64),
    /// Running text, part of the current verse.
    Text(String),
    /// A line of plain text (PDF, DOCX): a leading number may begin a verse,
    /// and a line that names a book and a chapter is a header.
    Line(String),
}

/// Words that introduce a chapter number in a heading, in the languages the
/// bundled texts use and a few more.
const CHAPTER_WORDS: &[&str] = &[
    "chapter", "chapitre", "kapittel", "kapitel", "capitulo", "capitolo", "hoofdstuk", "psalm",
    "psaume", "salme", "psalme", "ps",
];

fn chapter_heading(text: &str) -> Option<i64> {
    let words: Vec<&str> = text.split_whitespace().collect();
    match words.as_slice() {
        [w, n] if CHAPTER_WORDS.contains(&super::books::normalize(w).as_str()) => n.parse().ok(),
        // a bare number on its own: "3"
        [n] => n.parse().ok().filter(|c| (1..=150).contains(c)),
        _ => None,
    }
}

/// A verse number at the start of a line: "12 The gold…" → (12, "The gold…").
fn leading_number(line: &str) -> Option<(i64, &str)> {
    let digits = line.len() - line.trim_start_matches(|c: char| c.is_ascii_digit()).len();
    if digits == 0 || digits > 3 {
        return None;
    }
    let n = line[..digits].parse().ok()?;
    let rest = line[digits..].trim_start();
    // the number must be followed by text, and separated from it
    (rest.len() < line.len() - digits && !rest.is_empty()).then_some((n, rest))
}

pub struct Parser<'a> {
    books: &'a BookMatcher,
    book: i64,
    chapter: i64,
    verse: i64,
    pub verses: Verses,
    /// Books entered by position rather than by name, for the report.
    pub inferred_books: usize,
    /// A heading was seen that named no book: the next chapter 1, verse 1
    /// begins the next book of the canon.
    pending_book: bool,
    /// The source marks its chapters. Only then does a restart at verse 1
    /// after an unknown heading mean a new book: without chapter markers it
    /// is as likely chapter 2 after a section title.
    chapters_marked: bool,
}

impl<'a> Parser<'a> {
    pub fn new(books: &'a BookMatcher) -> Parser<'a> {
        Parser {
            books,
            book: 0,
            chapter: 0,
            verse: 0,
            verses: Verses::new(),
            inferred_books: 0,
            pending_book: false,
            chapters_marked: false,
        }
    }

    pub fn feed(&mut self, event: Event) {
        match event {
            Event::Heading(text) => self.heading(&text),
            Event::Book(n) => self.set_book(n),
            Event::Chapter(n) => {
                if n != self.chapter {
                    self.set_chapter(n);
                    self.verse = 1;
                }
            }
            Event::Verse(n) => self.set_verse(n),
            Event::Text(text) => self.append(&text, ""),
            Event::Line(line) => self.line(&line),
        }
    }

    fn set_book(&mut self, book: i64) {
        if book != self.book {
            self.book = book;
            self.chapter = 0;
            self.verse = 0;
        }
        self.pending_book = false;
    }

    fn set_chapter(&mut self, chapter: i64) {
        self.chapters_marked = true;
        self.pending_book = false;
        if chapter != self.chapter {
            self.chapter = chapter;
            self.verse = 0;
        }
    }

    fn set_verse(&mut self, verse: i64) {
        if self.pending_book && self.chapters_marked && verse == 1 && self.book > 0 && self.book < 66 {
            // An unrecognised heading followed by a fresh start: the books are
            // in canonical order, so this is the next one.
            self.book += 1;
            self.chapter = 1;
            self.verse = 0;
            self.inferred_books += 1;
        }
        self.pending_book = false;
        if self.book == 0 {
            return;
        }
        if self.chapter == 0 {
            self.chapter = 1; // a book of one chapter, or no chapter marker
        } else if verse == 1 && self.verse > 1 {
            // verse numbering restarted with no chapter marker in between
            self.chapter += 1;
        }
        self.verse = verse;
    }

    fn heading(&mut self, text: &str) {
        let text = text.trim();
        if let Some(book) = self.books.book(text) {
            self.set_book(book);
        } else if let Some((book, chapter)) = self.books.book_and_chapter(text) {
            self.set_book(book);
            self.set_chapter(chapter);
        } else if let Some(chapter) = chapter_heading(text) {
            self.set_chapter(chapter);
        } else if self.verse > 0 && text.split_whitespace().count() <= 6 {
            // A short heading that is neither a book, nor a chapter: a section
            // title — or, between books, a name the matcher does not know.
            self.pending_book = true;
        }
    }

    fn line(&mut self, line: &str) {
        let line = line.trim();
        if line.is_empty() {
            return;
        }
        // Plain lines are text first: a verse wrapped onto a line of its own
        // can read "John." or "Joel," — people share their names with
        // books. So a lone name never changes the book, and a line naming a
        // book and chapter is a header only when it steps forward as headers
        // do: to the next chapter, or to a later book's first chapter.
        if let Some((book, chapter)) = self.books.book_and_chapter(line)
            && ((book > self.book && (chapter == 1 || self.book == 0))
                || (book == self.book && (chapter == self.chapter || chapter == self.chapter + 1)))
        {
            // a running header repeats the chapter already open: no change
            if (book, chapter) != (self.book, self.chapter) {
                self.set_book(book);
                self.set_chapter(chapter);
            }
            return;
        }
        let mut text = line;
        if let Some((n, rest)) = leading_number(line)
            && self.plausible_verse(n)
        {
            self.set_verse(n);
            text = rest;
        }
        // The next verse can also begin mid-line: "…with the blood. 39 Thus
        // they became unclean". Only right after the end of a sentence, and
        // only the very next verse number, followed by a capital.
        let mut separator = " ";
        while let Some((at, n, after)) = self.inline_verse(text) {
            self.append(&text[..at], separator);
            self.set_verse(n);
            text = after;
            separator = "";
        }
        self.append(text, separator);
    }

    /// The next verse starting inside `text`: (where its number is, the
    /// number, the text after it).
    fn inline_verse<'t>(&self, text: &'t str) -> Option<(usize, i64, &'t str)> {
        if self.verse == 0 {
            return None;
        }
        let number = (self.verse + 1).to_string();
        let needle = format!(" {number} ");
        let mut from = 0;
        while let Some(i) = text[from..].find(&needle) {
            let at = from + i;
            let before = text[..at].trim_end().chars().last();
            let after = &text[at + needle.len()..];
            let next = after.chars().next();
            if before.is_some_and(|c| ".;:!?\"”’'".contains(c))
                && next.is_some_and(|c| c.is_uppercase() || "\"“‘'".contains(c))
            {
                return Some((at, self.verse + 1, after));
            }
            from = at + 1;
        }
        None
    }

    /// Whether a number at the start of a line can be the next verse: the
    /// one after the current verse, give or take a verse a translation omits,
    /// or the first of a new chapter.
    fn plausible_verse(&self, n: i64) -> bool {
        self.book > 0 && ((n > self.verse && n <= self.verse + 3) || (n == 1 && self.verse > 1))
    }

    /// Add text to the current verse, after `separator`: a space between
    /// lines, nothing between the inline pieces of marked-up text
    /// (`L<span>ORD</span>` is one word).
    fn append(&mut self, text: &str, separator: &str) {
        if self.book == 0 || self.chapter == 0 || self.verse == 0 {
            return;
        }
        let slot = self.verses.entry((self.book, self.chapter, self.verse)).or_default();
        slot.push_str(separator);
        slot.push_str(text);
    }

    /// The verses, whitespace collapsed and empty ones dropped.
    pub fn finish(self) -> Verses {
        self.verses
            .into_iter()
            .map(|(k, v)| (k, clean(&v)))
            .filter(|(_, v)| !v.is_empty())
            .collect()
    }
}

/// Collapse whitespace; drop the braces some editions put around words
/// supplied in translation (`{the help of}`), keeping the words.
pub fn clean(text: &str) -> String {
    text.replace(['{', '}'], "").split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(events: Vec<Event>) -> Verses {
        let books = BookMatcher::new([]);
        let mut p = Parser::new(&books);
        for e in events {
            p.feed(e);
        }
        p.finish()
    }

    fn lines(text: &str) -> Vec<Event> {
        text.lines().map(|l| Event::Line(l.to_string())).collect()
    }

    #[test]
    fn pdf_lines_with_running_headers() {
        let v = parse(lines(
            "About this Bible\n12 pages of front matter\n\
             Genesis 1        New American Standard Bible\n\
             1 In the beginning God created the heavens and the earth.\n\
             2 The earth was formless and void, and darkness was over the surface of the deep, and\n\
             the Spirit of God was moving.\n\
             Genesis 1        New American Standard Bible\n\
             3 Then God said, \"Let there be light\";\n\
             Genesis 2        New American Standard Bible\n\
             1 Thus the heavens and the earth were completed.\n\
             2 By the seventh day God completed His work which\n\
             John.\n\
             Zechariah\n\
             3 Then God blessed the seventh day\n\
             Genesis 1 recounts. 4 And it was so.\n\
             12 The gold of that land is good; {and} the onyx.\n",
        ));
        assert_eq!(v.len(), 7);
        // a verse starting mid-line after a full stop; "12 The gold" cannot
        // follow verse 4, so it stays text
        assert_eq!(v[&(1, 2, 4)], "And it was so. 12 The gold of that land is good; and the onyx.");
        assert_eq!(v[&(1, 1, 1)], "In the beginning God created the heavens and the earth.");
        assert!(v[&(1, 1, 2)].ends_with("and the Spirit of God was moving."));
        assert_eq!(v[&(1, 1, 3)], "Then God said, \"Let there be light\";");
        assert_eq!(v[&(1, 2, 1)], "Thus the heavens and the earth were completed.");
        // a wrapped line naming a book is still the verse's text
        assert_eq!(v[&(1, 2, 2)], "By the seventh day God completed His work which John. Zechariah");
        // "Genesis 1" mid-verse goes backwards: text, not a header
        assert_eq!(v[&(1, 2, 3)], "Then God blessed the seventh day Genesis 1 recounts.");
    }

    #[test]
    fn marked_up_structure() {
        let v = parse(vec![
            Event::Heading("Ruth".into()),
            Event::Chapter(1),
            Event::Text("In the days ".into()),
            Event::Text("when the judges ru".into()),
            Event::Text("led".into()),
            Event::Heading("Naomi Widowed".into()), // a section title
            Event::Verse(2),
            Event::Text("The name of the man".into()),
            Event::Chapter(2),
            Event::Verse(1),
            Event::Text("Now Naomi had a relative".into()),
        ]);
        assert_eq!(v[&(8, 1, 1)], "In the days when the judges ruled");
        assert_eq!(v[&(8, 1, 2)], "The name of the man");
        assert_eq!(v[&(8, 2, 1)], "Now Naomi had a relative");
    }

    #[test]
    fn chapters_and_books_without_markers() {
        let v = parse(vec![
            Event::Heading("Amos".into()),
            Event::Chapter(9),
            Event::Verse(15),
            Event::Text("I will also plant them.".into()),
            Event::Heading("Obadiah".into()),
            Event::Verse(1),
            Event::Text("The vision of Obadiah.".into()),
            Event::Verse(2),
            Event::Text("Behold.".into()),
            // a section title inside the book changes nothing
            Event::Heading("Edom Will Be Humbled".into()),
            Event::Verse(3),
            Event::Text("The arrogance of your heart.".into()),
            // unknown name for Jonah; then its chapter 1 restarts at verse 1
            Event::Heading("Jonas le prophète".into()),
            Event::Verse(1),
            Event::Text("The word of the LORD came to Jonah.".into()),
            Event::Verse(2),
            Event::Text("Arise.".into()),
            // chapter 2, no marker: numbering restarts
            Event::Verse(1),
            Event::Text("Then Jonah prayed.".into()),
        ]);
        assert_eq!(v[&(30, 9, 15)], "I will also plant them.");
        assert_eq!(v[&(31, 1, 2)], "Behold.");
        assert_eq!(v[&(31, 1, 3)], "The arrogance of your heart.");
        assert_eq!(v[&(32, 1, 1)], "The word of the LORD came to Jonah.");
        assert_eq!(v[&(32, 2, 1)], "Then Jonah prayed.");
    }
}
