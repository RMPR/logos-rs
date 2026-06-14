use std::cell::RefCell;
use std::rc::Rc;

use logos_rs::bible::{self, Bible, Book, SearchHit, Translation};
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
    refresh_reading(&ui, &state.borrow());

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
            refresh_reading(&ui, &state.borrow());
        });
    }
    {
        let (w, state) = ctx!();
        ui.on_select_chapter(move |ch| {
            let ui = w.unwrap();
            state.borrow_mut().chapter = ch as i64;
            refresh_reading(&ui, &state.borrow());
        });
    }
    {
        let (w, state) = ctx!();
        ui.on_prev_chapter(move || {
            let ui = w.unwrap();
            step_chapter(&state, -1);
            refresh_reading(&ui, &state.borrow());
        });
    }
    {
        let (w, state) = ctx!();
        ui.on_next_chapter(move || {
            let ui = w.unwrap();
            step_chapter(&state, 1);
            refresh_reading(&ui, &state.borrow());
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
            refresh_reading(&ui, &state.borrow());
        });
    }
    {
        let (w, state) = ctx!();
        ui.on_secondary_changed(move |idx| {
            let ui = w.unwrap();
            state.borrow_mut().secondary = idx as usize;
            refresh_reading(&ui, &state.borrow());
        });
    }
    {
        let (w, state) = ctx!();
        ui.on_toggle_parallel(move || {
            let ui = w.unwrap();
            ui.set_parallel(!ui.get_parallel());
            refresh_reading(&ui, &state.borrow());
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
                refresh_reading(&ui, &state.borrow());
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
                refresh_reading(&ui, &state.borrow());
            }
        });
    }
    {
        let (w, _state) = ctx!();
        ui.on_close_results(move || {
            w.unwrap().set_show_results(false);
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
fn refresh_reading(ui: &MainWindow, st: &State) {
    let pid = st.primary_id();
    let count = st.bible.chapter_count(pid, st.book).unwrap_or(0).max(1);
    let chapter = st.chapter.clamp(1, count);

    let chapters: Vec<i32> = (1..=count as i32).collect();
    ui.set_chapters(ModelRc::new(VecModel::from(chapters)));
    ui.set_current_book(st.book as i32);
    ui.set_current_chapter(chapter as i32);

    let verses = st.bible.chapter(pid, st.book, chapter).unwrap_or_default();
    ui.set_verses(verse_model(&verses));
    ui.set_primary_rtl(st.translations[st.primary].rtl);
    ui.set_primary_title(
        format!(
            "{} {} · {}",
            st.book_name(st.book),
            chapter,
            st.translations[st.primary].code
        )
        .into(),
    );

    // secondary (parallel)
    let sid = st.secondary_id();
    let verses2 = st.bible.chapter(sid, st.book, chapter).unwrap_or_default();
    ui.set_verses2(verse_model(&verses2));
    ui.set_secondary_rtl(st.translations[st.secondary].rtl);
    ui.set_secondary_title(st.translations[st.secondary].name.clone().into());
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
