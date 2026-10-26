//! The words of every translation linked to the Hebrew and Greek, against the
//! built database: coverage, the sense of the links, the verse mapping, and
//! the aligner's agreement with tagging done by people. Like the engine tests
//! these skip when `data/bible.db` has not been built; the copyrighted texts
//! built only locally (ESV, BFC, NB) are checked when present.

use std::collections::HashMap;

use logos_rs::bible::Bible;
use logos_rs::personal::{align, store, text::Verses};
use logos_rs::versification::{Shape, VerseMap};
use rusqlite::{Connection, OpenFlags};

fn database() -> Option<(Bible, Connection)> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("data/bible.db");
    if !path.exists() {
        eprintln!("skipping: {} not built", path.display());
        return None;
    }
    let conn = Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    Some((Bible::open(&path).unwrap(), conn))
}

fn has(conn: &Connection, code: &str) -> bool {
    conn.query_row("SELECT COUNT(*) FROM translations WHERE code = ?1", [code], |r| r.get::<_, i64>(0)).unwrap() > 0
}

/// Of a translation's words (those with a letter or digit, in the books
/// `books`), the share linked to an original word.
fn coverage(conn: &Connection, code: &str, books: std::ops::RangeInclusive<i64>) -> f64 {
    let mut stmt = conn
        .prepare(
            "SELECT w.text, w.strongs IS NOT NULL AND w.strongs <> '' FROM words w
             JOIN translations t ON t.id = w.translation_id WHERE t.code = ?1 AND w.book BETWEEN ?2 AND ?3",
        )
        .unwrap();
    let rows = stmt
        .query_map(rusqlite::params![code, books.start(), books.end()], |r| Ok((r.get::<_, String>(0)?, r.get::<_, bool>(1)?)))
        .unwrap();
    let (mut words, mut linked) = (0usize, 0usize);
    for (text, is_linked) in rows.map(Result::unwrap) {
        if text.chars().any(char::is_alphanumeric) {
            words += 1;
            linked += is_linked as usize;
        }
    }
    assert!(words > 0, "{code} has no words");
    linked as f64 / words as f64
}

#[test]
fn every_translation_is_linked() {
    let Some((_, conn)) = database() else { return };
    for code in ["KJV", "ULT", "LSG", "VUL", "ESV", "BFC", "NB"] {
        if !has(&conn, code) {
            continue;
        }
        let share = coverage(&conn, code, 1..=66);
        assert!(share >= 0.9999, "{code}: only {share:.5} of words linked");
    }
    // The Septuagint: everything in the books the Hebrew Bible has; in its own
    // books only the names and rare words no lexicon we have knows are left.
    let canonical = coverage(&conn, "LXX", 1..=66);
    assert!(canonical >= 0.9999, "LXX canonical: {canonical:.5}");
    let all = coverage(&conn, "LXX", 1..=200);
    assert!(all >= 0.997, "LXX: {all:.5}");
}

/// What a word is most often linked to, in a translation.
fn commonest_link(conn: &Connection, code: &str, word: &str) -> String {
    conn.query_row(
        "SELECT w.strongs FROM words w JOIN translations t ON t.id = w.translation_id
         WHERE t.code = ?1 AND rtrim(w.text, ' .,;:!?»\u{a0}') = ?2 AND w.strongs IS NOT NULL
         GROUP BY w.strongs ORDER BY COUNT(*) DESC LIMIT 1",
        [code, word],
        |r| r.get(0),
    )
    .unwrap_or_else(|_| panic!("{code} has no linked word {word:?}"))
}

#[test]
fn links_mean_what_they_should() {
    let Some((_, conn)) = database() else { return };
    let expect = |code: &str, word: &str, strongs: &str| {
        if has(&conn, code) {
            assert_eq!(commonest_link(&conn, code, word), strongs, "{code} {word}");
        }
    };
    // inflected forms pool their evidence with the stem
    expect("NB", "sønn", "H1121");
    expect("NB", "sønnen", "H1121");
    expect("NB", "sønnene", "H1121");
    expect("NB", "sønner", "H1121");
    expect("NB", "kongen", "H4428");
    expect("LSG", "fils", "H1121");
    expect("LSG", "roi", "H4428");
    expect("ESV", "son", "H1121");
    expect("ESV", "king", "H4428");
    // the Septuagint renders the divine name as κύριος
    expect("LXX", "κύριος", "H3068");
    expect("LXX", "υἱὸς", "H1121");
    expect("LXX", "βασιλεὺς", "H4428");
    expect("VUL", "Dominus", "H3068");
    expect("VUL", "rex", "H4428");
    expect("VUL", "filius", "H1121");
    expect("VUL", "Deus", "H430");
    // tagging done by people is kept
    expect("KJV", "God", "H430");
    expect("ULT", "God", "H430");
}

#[test]
fn hand_tagging_is_kept() {
    let Some((_, conn)) = database() else { return };
    let gold = logos_rs::importer::read_tagged(&logos_rs::importer::data_dir(), "kjv_strongs").unwrap();
    let mut stmt = conn
        .prepare(
            "SELECT w.book, w.chapter, w.verse, w.strongs FROM words w JOIN translations t ON t.id = w.translation_id
             WHERE t.code = 'KJV' ORDER BY w.book, w.chapter, w.verse, w.position",
        )
        .unwrap();
    let mut stored: HashMap<(i64, i64, i64), Vec<Option<String>>> = HashMap::new();
    for row in stmt.query_map([], |r| Ok(((r.get(0)?, r.get(1)?, r.get(2)?), r.get(3)?))).unwrap() {
        let (k, s) = row.unwrap();
        stored.entry(k).or_default().push(s);
    }
    let (mut tagged, mut kept) = (0, 0);
    for (k, words) in &gold {
        let text: Vec<&str> = words.iter().map(|(w, _)| w.as_str()).collect();
        let spans = logos_rs::personal::word_spans(&text.join(" "));
        for (position, (_, span)) in spans.iter().enumerate() {
            let Some(hand) = span.clone().find_map(|i| words[i].1.clone()) else { continue };
            tagged += 1;
            kept += (stored[k][position].as_deref() == Some(hand.as_str())) as usize;
        }
    }
    assert!(tagged > 340_000, "{tagged}");
    assert_eq!(kept, tagged, "hand tags lost");
}

#[test]
fn verses_meet_their_originals() {
    let Some((bible, conn)) = database() else { return };
    let id = |code: &str| conn.query_row("SELECT id FROM translations WHERE code = ?1", [code], |r| r.get::<_, i64>(0)).unwrap();
    let shape = |codes: &[&str]| {
        Shape::from_lengths(codes.iter().flat_map(|c| bible.verse_lengths(id(c)).unwrap()))
    };
    let originals = shape(&["WLC", "SBLGNT"]);
    let kjv = VerseMap::new(&shape(&["KJV"]), &originals);
    // the KJV prints a psalm's title in its verse 1; the Hebrew numbers it 1
    assert_eq!(kjv.originals((19, 3, 1)), vec![(19, 3, 1), (19, 3, 2)]);
    assert_eq!(kjv.originals((19, 3, 8)), vec![(19, 3, 9)]);
    assert_eq!(kjv.originals((39, 4, 1)), vec![(39, 3, 19)]); // Malachi 4
    assert_eq!(kjv.originals((29, 2, 28)), vec![(29, 3, 1)]); // Joel 2:28
    assert_eq!(kjv.originals((1, 31, 55)), vec![(1, 32, 1)]);
    assert_eq!(kjv.originals((43, 3, 16)), vec![(43, 3, 16)]);
    // the Septuagint joins Psalms 9 and 10: its 10 is the Hebrew 11
    let lxx = VerseMap::new(&shape(&["LXX"]), &originals);
    assert_eq!(lxx.originals((19, 10, 1)), vec![(19, 11, 1)]);
    assert_eq!(lxx.verses_for(&kjv.originals((19, 23, 1))), vec![(19, 22, 1)]);
    let lsg = VerseMap::new(&shape(&["LSG"]), &originals);
    // Louis Segond numbers the psalms as the Hebrew does
    assert_eq!(lsg.originals((19, 3, 2)), vec![(19, 3, 2)]);
    // and the word study finds the Hebrew of the KJV's title in its verse 1
    let absalom = bible.words_with_strongs(id("WLC"), 19, 3, 1, "H53").unwrap();
    assert_eq!(absalom.len(), 1);
}

#[test]
fn the_lexicon_knows_the_septuagints_greek() {
    let Some((_, conn)) = database() else { return };
    let (greek, marked_up): (i64, i64) = conn
        .query_row(
            "SELECT COUNT(*), SUM(definition LIKE '%<b>%' OR definition LIKE '%<BR%') FROM lexicon WHERE strongs LIKE 'G%'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert!(greek > 10_000, "{greek} Greek entries");
    assert_eq!(marked_up, 0);
    // a word of the Septuagint's own books, named by an extended number
    let in_tobit: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM words w JOIN translations t ON t.id = w.translation_id
             JOIN lexicon l ON l.strongs = w.strongs WHERE t.code = 'LXX' AND w.book = 69",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(in_tobit > 4_000, "{in_tobit} words of Tobit with a lexicon entry");
}

/// The aligner, re-run on the KJV's text, against the KJV's tagging done by
/// people: their content words should mostly get the same Strong's number.
/// The plain IBM Model 1 of tools/build_strongs.py managed 59%.
#[test]
fn the_aligner_agrees_with_tagging_done_by_people() {
    let Some((_, conn)) = database() else { return };
    let gold = logos_rs::importer::read_tagged(&logos_rs::importer::data_dir(), "kjv_strongs").unwrap();
    let mut verses = Verses::new();
    for (k, words) in &gold {
        let text: Vec<&str> = words.iter().map(|(w, _)| w.as_str()).collect();
        verses.insert(*k, text.join(" "));
    }
    let originals = store::originals(&conn).unwrap();
    let alignment = align::align(&verses, &originals, "en", &mut |_| {});
    let (mut tagged, mut agree, mut words, mut linked) = (0, 0, 0, 0);
    for (k, gold_words) in &gold {
        let spans = logos_rs::personal::word_spans(&verses[k]);
        for (position, (word, span)) in spans.iter().enumerate() {
            let mine = alignment[k][position];
            if word.chars().any(char::is_alphanumeric) {
                words += 1;
                linked += mine.is_some() as usize;
            }
            let Some(hand) = span.clone().find_map(|i| gold_words[i].1.clone()) else { continue };
            tagged += 1;
            agree += (mine.map(|s| s.to_string()) == Some(hand)) as usize;
        }
    }
    let agreement = agree as f64 / tagged as f64;
    let share = linked as f64 / words as f64;
    eprintln!("KJV re-aligned: {agreement:.3} agreement, {share:.4} linked");
    assert!(agreement >= 0.85, "agreement fell to {agreement:.3}");
    assert!(share >= 0.998, "coverage fell to {share:.4}");
}
