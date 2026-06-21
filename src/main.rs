use std::cell::RefCell;
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
    let db = bible::default_db_path();
    if !db.exists() {
        eprintln!(
            "bible.db not found at {}. Run `cargo run --bin import` first.",
            db.display()
        );
        std::process::exit(1);
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
                if !st.primary_books.iter().any(|b| b.nr == st.book) {
                    if let Some(first) = st.primary_books.first() {
                        st.book = first.nr;
                        st.chapter = 1;
                    }
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

            let is_english = trans.lang == "en";
            let is_ot = word.book < 40;
            let orig_rtl = is_ot; // Hebrew is RTL, Greek LTR

            // Reverse lookup: when clicking the English text, resolve the word
            // to its original-language word(s) in the same verse.
            let mut originals_label = String::new();
            let mut originals: Vec<OriginalWord> = Vec::new();
            let mut parsing = word.parsing.clone();
            if is_english && !word.strongs.is_empty() {
                let orig_code = if is_ot { "WLC" } else { "SBLGNT" };
                if let Some(ot) = st.translations.iter().find(|t| t.code == orig_code) {
                    let ws = st
                        .bible
                        .words_with_strongs(ot.id, word.book, word.chapter, word.verse, &word.strongs)
                        .unwrap_or_default();
                    if parsing.is_empty() {
                        if let Some(first) = ws.first() {
                            parsing = first.parsing.clone();
                        }
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
            if is_ot {
                if let Some(lt) = st.translations.iter().find(|t| t.code == "LXX") {
                    if let Ok(Some(text)) =
                        st.bible.verse_text(lt.id, word.book, word.chapter, word.verse)
                    {
                        lxx = text;
                        show_lxx = true;
                    }
                }
            }

            // Vulgate (Latin) verse reference — for both OT and NT words.
            let mut vulgate = String::new();
            let mut show_vulgate = false;
            if let Some(vt) = st.translations.iter().find(|t| t.code == "VUL") {
                if let Ok(Some(text)) =
                    st.bible.verse_text(vt.id, word.book, word.chapter, word.verse)
                {
                    vulgate = text;
                    show_vulgate = true;
                }
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
    ui.set_primary_rtl(primary_rtl);
    ui.set_primary_title(
        format!("{} {} · {}", st.book_name(book), chapter, st.translations[primary].code).into(),
    );
    if st.translations[primary].tagged {
        let budget = if parallel { 32 } else { 60 };
        let (blocks, flat) = build_blocks(&st.bible, pid, book, chapter, primary_rtl, budget);
        st.primary_words = flat;
        ui.set_primary_words(blocks);
        ui.set_primary_tagged(true);
        ui.set_verses(ModelRc::new(VecModel::default()));
    } else {
        let verses = st.bible.chapter(pid, book, chapter).unwrap_or_default();
        ui.set_verses(verse_model(&verses));
        ui.set_primary_tagged(false);
        st.primary_words.clear();
    }

    // ----- secondary column (parallel) -----
    let sid = st.secondary_id();
    let secondary_rtl = st.translations[secondary].rtl;
    ui.set_secondary_rtl(secondary_rtl);
    ui.set_secondary_title(st.translations[secondary].name.clone().into());
    if st.translations[secondary].tagged {
        let (blocks, flat) = build_blocks(&st.bible, sid, book, chapter, secondary_rtl, 32);
        st.secondary_words = flat;
        ui.set_secondary_words(blocks);
        ui.set_secondary_tagged(true);
        ui.set_verses2(ModelRc::new(VecModel::default()));
    } else {
        let verses2 = st.bible.chapter(sid, book, chapter).unwrap_or_default();
        ui.set_verses2(verse_model(&verses2));
        ui.set_secondary_tagged(false);
        st.secondary_words.clear();
    }

    // clear word-selection highlight (indices change with the chapter)
    ui.set_selected_col(-1);
    ui.set_selected_idx(-1);
}

/// Display width of a word, ignoring Hebrew points / Greek combining accents.
fn display_width(s: &str) -> usize {
    s.chars()
        .filter(|c| !('\u{0300}'..='\u{036F}').contains(c) && !('\u{0591}'..='\u{05C7}').contains(c))
        .count()
}

/// Build wrapped verse blocks for a tagged translation, plus the flat word
/// list (indexed by the chip `idx` used for click lookups).
fn build_blocks(
    bible: &Bible,
    tid: i64,
    book: i64,
    chapter: i64,
    rtl: bool,
    budget: usize,
) -> (ModelRc<VerseBlock>, Vec<Word>) {
    let verses = bible.chapter(tid, book, chapter).unwrap_or_default();
    let mut flat: Vec<Word> = Vec::new();
    let mut blocks: Vec<VerseBlock> = Vec::new();

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
            });
            width += ww;
        }
        if !cur.is_empty() {
            lines.push(make_line(cur, rtl));
        }
        flat.extend(words);
        blocks.push(VerseBlock {
            verse: v.verse as i32,
            lines: ModelRc::new(VecModel::from(lines)),
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

fn verse_model(verses: &[bible::Verse]) -> ModelRc<VerseItem> {
    let items: Vec<VerseItem> = verses
        .iter()
        .map(|v| VerseItem { verse: v.verse as i32, text: v.text.clone().into() })
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
