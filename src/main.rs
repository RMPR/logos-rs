use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use logos_rs::bible::{self, Bible, Book, SearchHit, Translation, Word};
use logos_rs::canon::Section;
use logos_rs::reference;
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

slint::include_modules!();

/// Mutable application state shared across UI callbacks (single-threaded).
struct State {
    bible: Bible,
    translations: Vec<Translation>,
    primary: usize,
    secondary: usize,
    primary_books: Vec<Book>,
    book: i64,
    chapter: i64,
    last_results: Vec<SearchHit>,
    /// Flat per-chapter word lists for the two columns, indexed by chip idx.
    primary_words: Vec<Word>,
    secondary_words: Vec<Word>,
    /// Corpus frequency of every lexeme in `freq_tid`, for the reader's
    /// edition. Built once per translation — it is a whole-corpus count.
    freq: HashMap<String, i64>,
    freq_tid: i64,
    threshold_idx: usize,
    /// Set once the reader picks a cut-off by hand, after which the per-corpus
    /// default stops overriding it.
    threshold_pinned: bool,
    /// Language the current cut-off was chosen for.
    threshold_lang: String,
}

/// Frequency cut-offs offered for the reader's apparatus. A word is glossed
/// when its lexeme occurs fewer than this many times in the corpus.
const THRESHOLDS: [i64; 6] = [10, 25, 30, 50, 100, 200];

/// A reader's edition is an original-language text: the apparatus glosses the
/// Hebrew or Greek a reader is working through. The Strong's-tagged modern
/// translations carry no morphology of their own to parse.
fn is_original_language(lang: &str) -> bool {
    lang == "hbo" || lang == "grc"
}

/// The cut-off printed reader's editions settle on for each corpus: fewer than
/// 100× for the Hebrew Bible (as in Zondervan's reader's Hebrew Bible), fewer
/// than 30× for the Greek New Testament (as in the UBS reader's edition).
fn default_threshold(lang: &str) -> usize {
    match lang {
        "hbo" => 4, // 100
        "grc" => 2, // 30
        _ => 3,     // 50
    }
}

/// Proper names carry nothing a reader needs looked up — every translation
/// simply transliterates them — so printed reader's editions leave them out of
/// the apparatus. The Hebrew morphology tags them (`Np` opening any segment of
/// the OSHM code); MorphGNT has no such tag, but the Greek text capitalises the
/// lemma of a name and nothing else.
fn is_proper_noun(w: &Word) -> bool {
    if w.lemma.chars().next().is_some_and(char::is_uppercase) {
        return true;
    }
    // The language letter (H Hebrew / A Aramaic) prefixes only the first
    // segment, so strip it before testing. Guards against `VNp…` — a Niphal
    // perfect verb, which merely contains "Np".
    w.morph
        .split('/')
        .any(|seg| seg.strip_prefix(['H', 'A']).unwrap_or(seg).starts_with("Np"))
}

/// What the apparatus needs while a chapter is laid out.
struct Reader<'a> {
    freq: &'a HashMap<String, i64>,
    max_freq: i64,
}

impl Reader<'_> {
    /// Worth glossing: rare enough in the corpus, known to the lexicon, and
    /// not a name.
    fn glosses(&self, w: &Word) -> bool {
        !w.strongs.is_empty()
            && self.freq.get(&w.strongs).is_some_and(|&n| n < self.max_freq)
            && !is_proper_noun(w)
    }

    fn count(&self, strongs: &str) -> i64 {
        self.freq.get(strongs).copied().unwrap_or(0)
    }
}

impl State {
    fn primary_id(&self) -> i64 {
        self.translations[self.primary].id
    }
    fn secondary_id(&self) -> i64 {
        self.translations[self.secondary].id
    }
    fn book_name(&self, nr: i64) -> String {
        self.primary_books
            .iter()
            .find(|b| b.nr == nr)
            .map(|b| b.name.clone())
            .unwrap_or_else(|| format!("Book {nr}"))
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Locate the database; build it from the vendored sources on first run.
    let db = match std::env::var("LOGOS_DB") {
        Ok(p) => std::path::PathBuf::from(p),
        Err(_) => logos_rs::importer::db_path(),
    };
    if !db.exists() {
        eprintln!("Building Bible database (one-time, ~10s, offline from bundled sources)…");
        logos_rs::importer::run()?;
    }
    let bible = Bible::open(&db)?;
    let translations = bible.translations()?;
    if translations.is_empty() {
        eprintln!("no translations in database");
        std::process::exit(1);
    }
    // Default secondary = a different translation if available (LSG).
    let secondary = if translations.len() > 1 { 1 } else { 0 };
    let primary_books = bible.books(translations[0].id)?;
    let start_lang = translations[0].lang.clone();
    let threshold_idx = default_threshold(&start_lang);

    let state = Rc::new(RefCell::new(State {
        bible,
        translations,
        primary: 0,
        secondary,
        primary_books,
        book: 1,
        chapter: 1,
        last_results: Vec::new(),
        primary_words: Vec::new(),
        secondary_words: Vec::new(),
        freq: HashMap::new(),
        freq_tid: 0,
        threshold_idx,
        threshold_pinned: false,
        threshold_lang: start_lang,
    }));

    let ui = MainWindow::new()?;

    // Static: translation labels for the combo boxes.
    {
        let st = state.borrow();
        let labels: Vec<SharedString> = st
            .translations
            .iter()
            .map(|t| format!("{} — {}", t.code, t.name).into())
            .collect();
        ui.set_translation_labels(ModelRc::new(VecModel::from(labels)));

        let thresholds: Vec<SharedString> =
            THRESHOLDS.iter().map(|n| format!("fewer than {n}×").into()).collect();
        ui.set_threshold_labels(ModelRc::new(VecModel::from(thresholds)));
        ui.set_threshold_index(st.threshold_idx as i32);
    }

    refresh_books(&ui, &state.borrow());
    refresh_reading(&ui, &state);

    // ---- callbacks ----
    // Each callback captures a weak UI handle and a clone of the shared state.
    macro_rules! ctx {
        () => {{
            (ui.as_weak(), state.clone())
        }};
    }

    {
        let (w, state) = ctx!();
        ui.on_select_book(move |nr| {
            let ui = w.unwrap();
            {
                let mut st = state.borrow_mut();
                st.book = nr as i64;
                st.chapter = 1;
            }
            refresh_reading(&ui, &state);
        });
    }
    {
        let (w, state) = ctx!();
        ui.on_select_chapter(move |ch| {
            let ui = w.unwrap();
            state.borrow_mut().chapter = ch as i64;
            refresh_reading(&ui, &state);
        });
    }
    {
        let (w, state) = ctx!();
        ui.on_prev_chapter(move || {
            let ui = w.unwrap();
            step_chapter(&state, -1);
            refresh_reading(&ui, &state);
        });
    }
    {
        let (w, state) = ctx!();
        ui.on_next_chapter(move || {
            let ui = w.unwrap();
            step_chapter(&state, 1);
            refresh_reading(&ui, &state);
        });
    }
    {
        let (w, state) = ctx!();
        ui.on_primary_changed(move |idx| {
            let ui = w.unwrap();
            {
                let mut st = state.borrow_mut();
                st.primary = idx as usize;
                let id = st.primary_id();
                st.primary_books = st.bible.books(id).unwrap_or_default();
                // If the current book is missing in this translation, jump to its first.
                if !st.primary_books.iter().any(|b| b.nr == st.book)
                    && let Some(first) = st.primary_books.first()
                {
                    st.book = first.nr;
                    st.chapter = 1;
                }
            }
            refresh_books(&ui, &state.borrow());
            refresh_reading(&ui, &state);
        });
    }
    {
        let (w, state) = ctx!();
        ui.on_secondary_changed(move |idx| {
            let ui = w.unwrap();
            state.borrow_mut().secondary = idx as usize;
            refresh_reading(&ui, &state);
        });
    }
    {
        let (w, state) = ctx!();
        ui.on_toggle_parallel(move || {
            let ui = w.unwrap();
            ui.set_parallel(!ui.get_parallel());
            refresh_reading(&ui, &state);
        });
    }
    {
        let (w, state) = ctx!();
        ui.on_toggle_reader(move || {
            let ui = w.unwrap();
            ui.set_reader(!ui.get_reader());
            refresh_reading(&ui, &state);
        });
    }
    {
        let (w, state) = ctx!();
        ui.on_threshold_changed(move |idx| {
            let ui = w.unwrap();
            {
                let mut st = state.borrow_mut();
                st.threshold_idx = (idx as usize).min(THRESHOLDS.len() - 1);
                st.threshold_pinned = true;
            }
            refresh_reading(&ui, &state);
        });
    }
    {
        let (w, _state) = ctx!();
        ui.on_toggle_theme(move || {
            let ui = w.unwrap();
            let t = ui.global::<Theme>();
            t.set_dark(!t.get_dark());
        });
    }
    {
        let (w, state) = ctx!();
        ui.on_go(move |text| {
            let ui = w.unwrap();
            let text = text.to_string();
            // Try to interpret as a reference first; fall back to full-text search.
            if navigate_reference(&state, &text) {
                ui.set_show_results(false);
                refresh_reading(&ui, &state);
            } else {
                run_search(&ui, &state, &text);
            }
        });
    }
    {
        let (w, state) = ctx!();
        ui.on_open_result(move |idx| {
            let ui = w.unwrap();
            let hit = state.borrow().last_results.get(idx as usize).cloned();
            if let Some(hit) = hit {
                {
                    let mut st = state.borrow_mut();
                    st.book = hit.book;
                    st.chapter = hit.chapter;
                }
                ui.set_show_results(false);
                refresh_reading(&ui, &state);
            }
        });
    }
    {
        let (w, _state) = ctx!();
        ui.on_close_results(move || {
            w.unwrap().set_show_results(false);
        });
    }
    {
        let (w, state) = ctx!();
        ui.on_word_clicked(move |col, idx| {
            let ui = w.unwrap();
            let st = state.borrow();
            let tindex = if col == 0 { st.primary } else { st.secondary };
            let trans = st.translations[tindex].clone();
            let words = if col == 0 { &st.primary_words } else { &st.secondary_words };
            let Some(word) = words.get(idx as usize).cloned() else { return };

            // A modern-language translation (KJV/ULT/LSG…) — as opposed to the
            // original-language texts (Hebrew/Greek), whose words ARE the original.
            let is_translation = trans.lang != "hbo" && trans.lang != "grc";
            let is_ot = word.book < 40;
            let orig_rtl = is_ot; // Hebrew is RTL, Greek LTR

            // Reverse lookup: clicking a translation word resolves it to its
            // original-language word(s) in the same verse.
            let mut originals_label = String::new();
            let mut originals: Vec<OriginalWord> = Vec::new();
            let mut parsing = word.parsing.clone();
            if is_translation && !word.strongs.is_empty() {
                let orig_code = if is_ot { "WLC" } else { "SBLGNT" };
                if let Some(ot) = st.translations.iter().find(|t| t.code == orig_code) {
                    let ws = st
                        .bible
                        .words_with_strongs(ot.id, word.book, word.chapter, word.verse, &word.strongs)
                        .unwrap_or_default();
                    if parsing.is_empty() && let Some(first) = ws.first() {
                        parsing = first.parsing.clone();
                    }
                    originals = ws
                        .iter()
                        .map(|o| OriginalWord {
                            text: o.text.clone().into(),
                            parsing: o.parsing.clone().into(),
                        })
                        .collect();
                    originals_label = format!(
                        "In the {} ({orig_code})",
                        if is_ot { "Hebrew" } else { "Greek" }
                    );
                }
            }

            // Septuagint verse for OT words.
            let mut lxx = String::new();
            let mut show_lxx = false;
            if is_ot
                && let Some(lt) = st.translations.iter().find(|t| t.code == "LXX")
                && let Ok(Some(text)) =
                    st.bible.verse_text(lt.id, word.book, word.chapter, word.verse)
            {
                lxx = text;
                show_lxx = true;
            }

            // Vulgate (Latin) verse reference — for both OT and NT words.
            let mut vulgate = String::new();
            let mut show_vulgate = false;
            if let Some(vt) = st.translations.iter().find(|t| t.code == "VUL")
                && let Ok(Some(text)) =
                    st.bible.verse_text(vt.id, word.book, word.chapter, word.verse)
            {
                vulgate = text;
                show_vulgate = true;
            }

            ui.set_word(WordInfo {
                surface: word.text.clone().into(),
                translit: word.translit.clone().into(),
                lemma: word.lemma.clone().into(),
                strongs: word.strongs.clone().into(),
                parsing: parsing.into(),
                gloss: word.gloss.clone().into(),
                definition: word.definition.clone().into(),
                rtl: trans.rtl,
                orig_rtl,
                originals_label: originals_label.into(),
                originals: ModelRc::new(VecModel::from(originals)),
                lxx: lxx.into(),
                show_lxx,
                vulgate: vulgate.into(),
                show_vulgate,
            });
            ui.set_selected_col(col);
            ui.set_selected_idx(idx);
            ui.set_show_word(true);
        });
    }
    {
        let (w, _state) = ctx!();
        ui.on_close_word(move || {
            let ui = w.unwrap();
            ui.set_show_word(false);
            ui.set_selected_col(-1);
            ui.set_selected_idx(-1);
        });
    }

    ui.run()?;
    Ok(())
}

/// Build the sidebar book list with Old/New/Deuterocanon section headers.
fn refresh_books(ui: &MainWindow, st: &State) {
    let mut items: Vec<BookItem> = Vec::new();
    let mut last_section = -1i64;
    for b in &st.primary_books {
        if b.section != last_section {
            last_section = b.section;
            items.push(BookItem {
                nr: -1,
                name: Section::of(if b.section == 0 { 1 } else if b.section == 1 { 40 } else { 100 })
                    .label()
                    .into(),
                header: true,
            });
        }
        items.push(BookItem { nr: b.nr as i32, name: b.name.clone().into(), header: false });
    }
    ui.set_books(ModelRc::new(VecModel::from(items)));
}

/// Refresh chapter grid, verse columns and titles for the current location.
fn refresh_reading(ui: &MainWindow, state: &Rc<RefCell<State>>) {
    let mut st = state.borrow_mut();
    let pid = st.primary_id();
    let count = st.bible.chapter_count(pid, st.book).unwrap_or(0).max(1);
    let chapter = st.chapter.clamp(1, count);
    st.chapter = chapter;

    let chapters: Vec<i32> = (1..=count as i32).collect();
    ui.set_chapters(ModelRc::new(VecModel::from(chapters)));
    ui.set_current_book(st.book as i32);
    ui.set_current_chapter(chapter as i32);

    let parallel = ui.get_parallel();
    let (book, primary, secondary) = (st.book, st.primary, st.secondary);

    // ----- primary column -----
    let primary_rtl = st.translations[primary].rtl;
    let lang = st.translations[primary].lang.clone();
    let reader_capable = st.translations[primary].tagged && is_original_language(&lang);

    // Each corpus has its own conventional cut-off, so follow the text unless
    // the reader has chosen one explicitly.
    if !st.threshold_pinned && st.threshold_lang != lang {
        st.threshold_idx = default_threshold(&lang);
        st.threshold_lang = lang.clone();
        ui.set_threshold_index(st.threshold_idx as i32);
    }
    ui.set_primary_title(
        format!("{} {} · {}", st.book_name(book), chapter, st.translations[primary].code).into(),
    );
    ui.set_primary_subtitle(st.translations[primary].name.clone().into());

    // The reader's apparatus needs whole-corpus counts; build them once per
    // translation, and only when the reader actually asks for them.
    ui.set_reader_available(reader_capable);
    let reader_on = reader_capable && ui.get_reader();
    if reader_on && st.freq_tid != pid {
        let freq = st.bible.lexeme_frequencies(pid).unwrap_or_default();
        st.freq = freq;
        st.freq_tid = pid;
    }

    if st.translations[primary].tagged {
        let budget = if parallel { 32 } else { 60 };
        let max_freq = THRESHOLDS[st.threshold_idx.min(THRESHOLDS.len() - 1)];
        let reader = reader_on.then(|| Reader { freq: &st.freq, max_freq });
        let (rows, flat) =
            tagged_rows(&st.bible, pid, book, chapter, primary_rtl, budget, reader.as_ref());
        ui.set_rows(rows);
        if let Some(r) = &reader {
            let notes = gloss_notes(&flat, r, primary_rtl);
            ui.set_reader_caption(
                reader_caption(&st.translations[primary], max_freq, notes.len()).into(),
            );
            ui.set_notes(ModelRc::new(VecModel::from(notes)));
        }
        st.primary_words = flat;
    } else {
        let verses = st.bible.chapter(pid, book, chapter).unwrap_or_default();
        ui.set_rows(plain_rows(&verses, primary_rtl));
        st.primary_words.clear();
    }
    if !reader_on {
        ui.set_notes(ModelRc::new(VecModel::default()));
    }

    // ----- secondary column (parallel only) -----
    let sid = st.secondary_id();
    let secondary_rtl = st.translations[secondary].rtl;
    ui.set_secondary_title(st.translations[secondary].name.clone().into());
    if !parallel {
        // keep the hidden column empty so it costs nothing to lay out
        ui.set_rows2(ModelRc::new(VecModel::default()));
        st.secondary_words.clear();
    } else if st.translations[secondary].tagged {
        let (rows, flat) = tagged_rows(&st.bible, sid, book, chapter, secondary_rtl, 32, None);
        st.secondary_words = flat;
        ui.set_rows2(rows);
    } else {
        let verses2 = st.bible.chapter(sid, book, chapter).unwrap_or_default();
        ui.set_rows2(plain_rows(&verses2, secondary_rtl));
        st.secondary_words.clear();
    }

    // clear word-selection highlight (indices change with the chapter)
    ui.set_selected_col(-1);
    ui.set_selected_idx(-1);
    // send both columns back to the top of the new passage
    ui.set_nav_epoch(ui.get_nav_epoch() + 1);
}

/// Display width of a word, ignoring Hebrew points / Greek combining accents.
fn display_width(s: &str) -> usize {
    s.chars()
        .filter(|c| !('\u{0300}'..='\u{036F}').contains(c) && !('\u{0591}'..='\u{05C7}').contains(c))
        .count()
}

/// Build wrapped word rows for a tagged translation, plus the flat word list
/// (indexed by the chip `idx` used for click lookups).
fn tagged_rows(
    bible: &Bible,
    tid: i64,
    book: i64,
    chapter: i64,
    rtl: bool,
    budget: usize,
    reader: Option<&Reader>,
) -> (ModelRc<VerseRow>, Vec<Word>) {
    let verses = bible.chapter(tid, book, chapter).unwrap_or_default();
    let mut flat: Vec<Word> = Vec::new();
    let mut blocks: Vec<VerseRow> = Vec::new();

    for v in &verses {
        let words = bible.words(tid, book, chapter, v.verse).unwrap_or_default();
        let base = flat.len();
        // greedy line wrapping
        let mut lines: Vec<WordLine> = Vec::new();
        let mut cur: Vec<WordChip> = Vec::new();
        let mut width = 0usize;
        for (i, w) in words.iter().enumerate() {
            let ww = display_width(&w.text) + 1;
            if width + ww > budget && !cur.is_empty() {
                lines.push(make_line(std::mem::take(&mut cur), rtl));
                width = 0;
            }
            cur.push(WordChip {
                idx: (base + i) as i32,
                text: w.text.clone().into(),
                tagged: !w.strongs.is_empty(),
                rare: reader.is_some_and(|r| r.glosses(w)),
            });
            width += ww;
        }
        if !cur.is_empty() {
            lines.push(make_line(cur, rtl));
        }
        flat.extend(words);
        blocks.push(VerseRow {
            verse: v.verse as i32,
            text: SharedString::default(),
            lines: ModelRc::new(VecModel::from(lines)),
            tagged: true,
            rtl,
        });
    }
    (ModelRc::new(VecModel::from(blocks)), flat)
}

fn make_line(mut chips: Vec<WordChip>, rtl: bool) -> WordLine {
    if rtl {
        chips.reverse(); // right-to-left reading order within the line
    }
    WordLine { words: ModelRc::new(VecModel::from(chips)) }
}

/// The reader's-edition apparatus for one chapter: every lexeme rarer than the
/// chosen cut-off, in reading order.
///
/// A word is listed once per verse, as printed reader's editions do — a rare
/// word repeated within a verse is looked up once, but a reader returning to it
/// in a later verse should not have to hunt back up the page.
fn gloss_notes(words: &[Word], reader: &Reader, rtl: bool) -> Vec<GlossItem> {
    let mut seen: HashSet<(i64, &str)> = HashSet::new();
    let mut notes: Vec<GlossItem> = Vec::new();
    for w in words {
        if !reader.glosses(w) || !seen.insert((w.verse, &w.strongs)) {
            continue;
        }
        notes.push(GlossItem {
            verse: w.verse as i32,
            surface: w.text.clone().into(),
            lemma: w.lemma.clone().into(),
            parsing: w.parsing.clone().into(),
            gloss: short_gloss(&w.gloss, &w.definition).into(),
            count: format!("{}×", reader.count(&w.strongs)).into(),
            rtl,
        });
    }
    notes
}

/// Shorten a lexicon entry to the senses that fit one apparatus line. Falls
/// back to the long definition for the entries that carry no short gloss.
fn short_gloss(gloss: &str, definition: &str) -> String {
    let source = if gloss.trim().is_empty() { definition } else { gloss };
    // The Hebrew entries separate senses with semicolons and the Greek ones
    // with commas, so split on whichever this entry actually uses.
    let sep = if source.contains(';') { ';' } else { ',' };
    let senses: Vec<&str> =
        source.split(sep).map(str::trim).filter(|s| !s.is_empty()).take(3).collect();
    let joined = senses.join(", ");
    match joined.char_indices().nth(64) {
        Some((cut, _)) => format!("{}…", joined[..cut].trim_end()),
        None => joined,
    }
}

/// The line above the apparatus, naming the corpus the counts are taken over.
///
/// Deliberately free of the book name: the header already carries it, and a
/// Hebrew name opening a Latin sentence flips the whole line under the
/// bidirectional text algorithm.
fn reader_caption(trans: &Translation, max_freq: i64, entries: usize) -> String {
    let corpus = match trans.lang.as_str() {
        "hbo" => "the Hebrew Bible".to_string(),
        "grc" => "the Greek New Testament".to_string(),
        _ => trans.name.clone(),
    };
    format!(
        "{entries} word{} occurring fewer than {max_freq}× in {corpus}",
        if entries == 1 { "" } else { "s" }
    )
}

/// Build plain prose rows for an untagged translation.
fn plain_rows(verses: &[bible::Verse], rtl: bool) -> ModelRc<VerseRow> {
    let items: Vec<VerseRow> = verses
        .iter()
        .map(|v| VerseRow {
            verse: v.verse as i32,
            text: v.text.clone().into(),
            lines: ModelRc::new(VecModel::default()),
            tagged: false,
            rtl,
        })
        .collect();
    ModelRc::new(VecModel::from(items))
}

/// Move the current chapter by `delta`, rolling over to adjacent books.
fn step_chapter(state: &Rc<RefCell<State>>, delta: i64) {
    let mut st = state.borrow_mut();
    let pid = st.primary_id();
    let count = st.bible.chapter_count(pid, st.book).unwrap_or(1).max(1);
    let next = st.chapter + delta;
    if next >= 1 && next <= count {
        st.chapter = next;
        return;
    }
    // roll over to an adjacent book that exists in this translation
    let books: Vec<i64> = st.primary_books.iter().map(|b| b.nr).collect();
    let pos = books.iter().position(|&n| n == st.book);
    if let Some(pos) = pos {
        if delta > 0 && pos + 1 < books.len() {
            st.book = books[pos + 1];
            st.chapter = 1;
        } else if delta < 0 && pos > 0 {
            let prev = books[pos - 1];
            st.book = prev;
            st.chapter = st.bible.chapter_count(pid, prev).unwrap_or(1).max(1);
        }
    }
}

/// Try to parse `text` as a reference and move there. Returns false if it
/// isn't a recognisable reference (so the caller can search instead).
fn navigate_reference(state: &Rc<RefCell<State>>, text: &str) -> bool {
    let Some(r) = reference::parse(text) else { return false };
    let mut st = state.borrow_mut();
    // The book must exist in the primary translation.
    if !st.primary_books.iter().any(|b| b.nr == r.book) {
        return false;
    }
    st.book = r.book;
    st.chapter = r.chapter.unwrap_or(1);
    true
}

/// Run a full-text search in the primary translation and show the results.
fn run_search(ui: &MainWindow, state: &Rc<RefCell<State>>, query: &str) {
    let hits = {
        let st = state.borrow();
        st.bible.search(st.primary_id(), query, 200).unwrap_or_default()
    };
    let items: Vec<SearchItem> = hits
        .iter()
        .map(|h| SearchItem {
            label: format!("{} {}:{}", h.book_name, h.chapter, h.verse).into(),
            snippet: h.snippet.clone().into(),
            book: h.book as i32,
            chapter: h.chapter as i32,
            verse: h.verse as i32,
        })
        .collect();
    let n = items.len();
    ui.set_results(ModelRc::new(VecModel::from(items)));
    ui.set_results_summary(
        format!("{n} result{} for “{query}”", if n == 1 { "" } else { "s" }).into(),
    );
    ui.set_show_results(true);
    state.borrow_mut().last_results = hits;
}
