//! Canonical book metadata for the 66-book Protestant canon.
//!
//! `getbible`'s `nr` field is our canonical book id: 1..=39 Old Testament,
//! 40..=66 New Testament. LXX-only / deuterocanonical books are imported with
//! synthetic ids >= 100 and are not represented here.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Section {
    OldTestament,
    NewTestament,
    Deuterocanon,
}

impl Section {
    pub fn of(book_nr: i64) -> Section {
        match book_nr {
            1..=39 => Section::OldTestament,
            40..=66 => Section::NewTestament,
            _ => Section::Deuterocanon,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Section::OldTestament => "Old Testament",
            Section::NewTestament => "New Testament",
            Section::Deuterocanon => "Deuterocanonical",
        }
    }
}

/// English name + recognised abbreviations for a canonical book.
pub struct BookInfo {
    pub nr: i64,
    pub name: &'static str,
    /// Lower-case spellings/abbreviations accepted by the reference parser.
    pub aliases: &'static [&'static str],
}

/// The 66 canonical books with common English abbreviations. Aliases are
/// matched case-insensitively after stripping spaces/dots by the parser.
pub const BOOKS: &[BookInfo] = &[
    BookInfo { nr: 1, name: "Genesis", aliases: &["genesis", "gen", "ge", "gn"] },
    BookInfo { nr: 2, name: "Exodus", aliases: &["exodus", "exo", "exod", "ex"] },
    BookInfo { nr: 3, name: "Leviticus", aliases: &["leviticus", "lev", "lv"] },
    BookInfo { nr: 4, name: "Numbers", aliases: &["numbers", "num", "nu", "nm"] },
    BookInfo { nr: 5, name: "Deuteronomy", aliases: &["deuteronomy", "deut", "dt"] },
    BookInfo { nr: 6, name: "Joshua", aliases: &["joshua", "josh", "jos", "jsh"] },
    BookInfo { nr: 7, name: "Judges", aliases: &["judges", "judg", "jdg", "jg"] },
    BookInfo { nr: 8, name: "Ruth", aliases: &["ruth", "rth", "ru"] },
    BookInfo { nr: 9, name: "1 Samuel", aliases: &["1samuel", "1sam", "1sa", "1s"] },
    BookInfo { nr: 10, name: "2 Samuel", aliases: &["2samuel", "2sam", "2sa", "2s"] },
    BookInfo { nr: 11, name: "1 Kings", aliases: &["1kings", "1kgs", "1ki", "1k"] },
    BookInfo { nr: 12, name: "2 Kings", aliases: &["2kings", "2kgs", "2ki", "2k"] },
    BookInfo { nr: 13, name: "1 Chronicles", aliases: &["1chronicles", "1chron", "1chr", "1ch"] },
    BookInfo { nr: 14, name: "2 Chronicles", aliases: &["2chronicles", "2chron", "2chr", "2ch"] },
    BookInfo { nr: 15, name: "Ezra", aliases: &["ezra", "ezr", "ez"] },
    BookInfo { nr: 16, name: "Nehemiah", aliases: &["nehemiah", "neh", "ne"] },
    BookInfo { nr: 17, name: "Esther", aliases: &["esther", "esth", "est", "es"] },
    BookInfo { nr: 18, name: "Job", aliases: &["job", "jb"] },
    BookInfo { nr: 19, name: "Psalms", aliases: &["psalms", "psalm", "psa", "ps", "pss"] },
    BookInfo { nr: 20, name: "Proverbs", aliases: &["proverbs", "prov", "prv", "pr"] },
    BookInfo { nr: 21, name: "Ecclesiastes", aliases: &["ecclesiastes", "eccl", "ecc", "ec", "qoh"] },
    BookInfo { nr: 22, name: "Song of Solomon", aliases: &["songofsolomon", "songofsongs", "song", "sos", "ss", "cant"] },
    BookInfo { nr: 23, name: "Isaiah", aliases: &["isaiah", "isa", "is"] },
    BookInfo { nr: 24, name: "Jeremiah", aliases: &["jeremiah", "jer", "je"] },
    BookInfo { nr: 25, name: "Lamentations", aliases: &["lamentations", "lam", "la"] },
    BookInfo { nr: 26, name: "Ezekiel", aliases: &["ezekiel", "ezek", "eze", "ezk"] },
    BookInfo { nr: 27, name: "Daniel", aliases: &["daniel", "dan", "dn", "da"] },
    BookInfo { nr: 28, name: "Hosea", aliases: &["hosea", "hos", "ho"] },
    BookInfo { nr: 29, name: "Joel", aliases: &["joel", "joe", "jl"] },
    BookInfo { nr: 30, name: "Amos", aliases: &["amos", "amo", "am"] },
    BookInfo { nr: 31, name: "Obadiah", aliases: &["obadiah", "obad", "oba", "ob"] },
    BookInfo { nr: 32, name: "Jonah", aliases: &["jonah", "jon", "jnh"] },
    BookInfo { nr: 33, name: "Micah", aliases: &["micah", "mic", "mc"] },
    BookInfo { nr: 34, name: "Nahum", aliases: &["nahum", "nah", "na"] },
    BookInfo { nr: 35, name: "Habakkuk", aliases: &["habakkuk", "hab", "hb"] },
    BookInfo { nr: 36, name: "Zephaniah", aliases: &["zephaniah", "zeph", "zep", "zp"] },
    BookInfo { nr: 37, name: "Haggai", aliases: &["haggai", "hag", "hg"] },
    BookInfo { nr: 38, name: "Zechariah", aliases: &["zechariah", "zech", "zec", "zc"] },
    BookInfo { nr: 39, name: "Malachi", aliases: &["malachi", "mal", "ml"] },
    BookInfo { nr: 40, name: "Matthew", aliases: &["matthew", "matt", "mat", "mt"] },
    BookInfo { nr: 41, name: "Mark", aliases: &["mark", "mrk", "mk", "mr"] },
    BookInfo { nr: 42, name: "Luke", aliases: &["luke", "luk", "lk"] },
    BookInfo { nr: 43, name: "John", aliases: &["john", "joh", "jn", "jhn"] },
    BookInfo { nr: 44, name: "Acts", aliases: &["acts", "act", "ac"] },
    BookInfo { nr: 45, name: "Romans", aliases: &["romans", "rom", "ro", "rm"] },
    BookInfo { nr: 46, name: "1 Corinthians", aliases: &["1corinthians", "1cor", "1co"] },
    BookInfo { nr: 47, name: "2 Corinthians", aliases: &["2corinthians", "2cor", "2co"] },
    BookInfo { nr: 48, name: "Galatians", aliases: &["galatians", "gal", "ga"] },
    BookInfo { nr: 49, name: "Ephesians", aliases: &["ephesians", "eph", "ep"] },
    BookInfo { nr: 50, name: "Philippians", aliases: &["philippians", "phil", "php", "pp"] },
    BookInfo { nr: 51, name: "Colossians", aliases: &["colossians", "col", "co"] },
    BookInfo { nr: 52, name: "1 Thessalonians", aliases: &["1thessalonians", "1thess", "1th"] },
    BookInfo { nr: 53, name: "2 Thessalonians", aliases: &["2thessalonians", "2thess", "2th"] },
    BookInfo { nr: 54, name: "1 Timothy", aliases: &["1timothy", "1tim", "1ti"] },
    BookInfo { nr: 55, name: "2 Timothy", aliases: &["2timothy", "2tim", "2ti"] },
    BookInfo { nr: 56, name: "Titus", aliases: &["titus", "tit", "ti"] },
    BookInfo { nr: 57, name: "Philemon", aliases: &["philemon", "philem", "phlm", "phm"] },
    BookInfo { nr: 58, name: "Hebrews", aliases: &["hebrews", "heb", "he"] },
    BookInfo { nr: 59, name: "James", aliases: &["james", "jas", "jm"] },
    BookInfo { nr: 60, name: "1 Peter", aliases: &["1peter", "1pet", "1pe", "1pt"] },
    BookInfo { nr: 61, name: "2 Peter", aliases: &["2peter", "2pet", "2pe", "2pt"] },
    BookInfo { nr: 62, name: "1 John", aliases: &["1john", "1joh", "1jn", "1jo"] },
    BookInfo { nr: 63, name: "2 John", aliases: &["2john", "2joh", "2jn", "2jo"] },
    BookInfo { nr: 64, name: "3 John", aliases: &["3john", "3joh", "3jn", "3jo"] },
    BookInfo { nr: 65, name: "Jude", aliases: &["jude", "jud", "jd"] },
    BookInfo { nr: 66, name: "Revelation", aliases: &["revelation", "rev", "re", "rv", "apocalypse"] },
];

/// English name for a canonical book number, if it is one of the 66.
pub fn name(nr: i64) -> Option<&'static str> {
    BOOKS.iter().find(|b| b.nr == nr).map(|b| b.name)
}
