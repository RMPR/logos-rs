//! Importing a translation into the reader's own database and reading it back
//! through the bundled one. Skips, like the engine tests, when `data/bible.db`
//! has not been built.

use logos_rs::bible::Bible;
use logos_rs::personal::{self, store};

#[test]
fn import_read_back_and_remove() {
    let bundled = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("data/bible.db");
    if !bundled.exists() {
        eprintln!("skipping: {} not built", bundled.display());
        return;
    }
    let dir = std::env::temp_dir().join(format!("leverbe-import-test-{}", std::process::id()));
    let imports = dir.join("imports.db");

    let books = personal::books::BookMatcher::new([]);
    let text = "Genesis 1\n\
        1 In the beginning God created the heavens and the earth.\n\
        2 The earth was formless and void.\n\
        John 1\n\
        1 In the beginning was the Word, and the Word was with God, and the Word was God.\n";
    let doc = personal::pdf::read_text(text, &books);
    assert_eq!(doc.verses.len(), 3);
    let req = personal::Request { code: "TST".into(), name: "Test Bible".into(), lang: "en".into() };
    let report = personal::import(&bundled, &imports, &req, &doc, &mut |_, _| {}).unwrap();
    assert_eq!(report.id, store::FIRST_ID);
    assert_eq!(report.verses, 3);

    let bible = Bible::open_with_imports(&bundled, &imports).unwrap();
    let translations = bible.translations().unwrap();
    assert!(translations.iter().any(|t| t.code == "KJV"), "the bundled texts are still there");
    let tst = translations.iter().find(|t| t.code == "TST").expect("the import is listed");
    assert!(tst.tagged);
    assert_eq!(bible.books(tst.id).unwrap().iter().map(|b| b.nr).collect::<Vec<_>>(), [1, 43]);
    let john = bible.chapter(tst.id, 43, 1).unwrap();
    assert_eq!(john[0].text, "In the beginning was the Word, and the Word was with God, and the Word was God.");
    let words = bible.words(tst.id, 1, 1, 1).unwrap();
    assert_eq!(words.len(), 10);
    assert_eq!(words[3].text, "God");
    // the full-text index of the imports, not the bundled one
    let hits = bible.search(tst.id, "formless", 10).unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!((hits[0].book, hits[0].chapter, hits[0].verse), (1, 1, 2));

    // a code can only be used once
    assert!(personal::import(&bundled, &imports, &req, &doc, &mut |_, _| {}).is_err());

    // linked by an older aligner: linked again, once
    {
        let conn = store::open(&imports).unwrap();
        assert!(store::stale(&conn).unwrap().is_empty());
        conn.execute("UPDATE meta SET value = '1' WHERE key = ?1", [format!("linked:{}", tst.id)]).unwrap();
        assert_eq!(store::stale(&conn).unwrap(), vec![(tst.id, "en".to_string())]);
    }
    assert_eq!(personal::relink_stale(&bundled, &imports).unwrap(), 1);
    assert_eq!(personal::relink_stale(&bundled, &imports).unwrap(), 0);
    let words = bible.words(tst.id, 43, 1, 1).unwrap();
    assert_eq!(words.len(), 17);
    assert!(words.iter().all(|w| !w.strongs.is_empty()), "every word linked");

    let mut conn = store::open(&imports).unwrap();
    store::delete(&mut conn, tst.id).unwrap();
    assert!(bible.translations().unwrap().iter().all(|t| t.code != "TST"));
    assert!(bible.chapter(tst.id, 43, 1).unwrap().is_empty());

    drop(bible);
    let _ = std::fs::remove_dir_all(&dir);
}
