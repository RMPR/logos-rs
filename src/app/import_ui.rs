//! The import card: choosing a file, confirming what was read in it, importing
//! it in the background, and removing imported translations.
//!
//! Reading, aligning and saving all run on worker threads — a whole Bible
//! takes seconds — and report back over a channel that the main window's poll
//! timer drains (the UI state is not `Send`, so a worker cannot touch it).

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::mpsc;

use crate::personal::{self, Document, Report, Request, store};
use rusqlite::{Connection, OpenFlags};
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

use super::{MAX_COLUMNS, MainWindow, State, TranslationItem, refresh_books, refresh_reading};

pub enum Msg {
    Read(Result<(PathBuf, Document), String>),
    Progress(String, f32),
    Imported(Result<Report, String>),
    Removed(Result<(), String>),
    /// Imports linked again by a newer aligner, at start-up.
    Relinked(Result<usize, String>),
}

/// Stages of the card, as `import-stage` has them.
const CHOOSE: i32 = 0;
const CONFIRM: i32 = 1;
const WORKING: i32 = 2;
const DONE: i32 = 3;

/// The pickers' labels, and the list of imported translations.
pub fn set_translation_lists(ui: &MainWindow, st: &State) {
    let labels: Vec<SharedString> =
        st.translations.iter().map(|t| format!("{} — {}", t.code, t.name).into()).collect();
    ui.set_translation_labels(ModelRc::new(VecModel::from(labels)));
    let imported: Vec<TranslationItem> = st
        .translations
        .iter()
        .filter(|t| t.id >= store::FIRST_ID)
        .map(|t| TranslationItem { code: t.code.clone().into(), name: t.name.clone().into() })
        .collect();
    ui.set_imported(ModelRc::new(VecModel::from(imported)));
}

fn language_name(code: &str) -> &str {
    match code {
        "en" => "English",
        "fr" => "French",
        "nb" => "Norwegian",
        "de" => "German",
        "es" => "Spanish",
        "pt" => "Portuguese",
        "it" => "Italian",
        "nl" => "Dutch",
        "" => "language not recognised",
        other => other,
    }
}

fn working(ui: &MainWindow, text: &str, progress: f32) {
    ui.set_import_error(SharedString::default());
    ui.set_import_stage(WORKING);
    ui.set_import_progress_text(text.into());
    ui.set_import_progress(progress);
}

pub fn connect(ui: &MainWindow, state: &Rc<RefCell<State>>) {
    // No file dialog on Android: the card is not offered there.
    #[cfg(not(target_os = "android"))]
    {
        let (w, state) = (ui.as_weak(), state.clone());
        ui.on_import_choose(move || {
            let ui = w.unwrap();
            if let Some(path) = rfd::FileDialog::new()
                .set_title("Import a translation")
                .add_filter("Bibles (EPUB, PDF, DOCX, text)", personal::EXTENSIONS)
                .pick_file()
            {
                start_read(&ui, &state, path);
            }
        });
    }
    {
        let (w, state) = (ui.as_weak(), state.clone());
        ui.on_import_confirm(move || {
            let ui = w.unwrap();
            let mut st = state.borrow_mut();
            let req = Request {
                code: ui.get_import_code().trim().to_uppercase(),
                name: ui.get_import_name().trim().to_string(),
                lang: personal::normalize_language(&ui.get_import_lang()),
            };
            let taken: Vec<String> = st.translations.iter().map(|t| t.code.clone()).collect();
            if let Err(e) = personal::validate(&req, &taken) {
                ui.set_import_error(e.into());
                return;
            }
            let (Some(imports), Some((_, doc))) = (st.imports.clone(), st.pending_import.take()) else {
                ui.set_import_error("Imported translations cannot be stored on this computer".into());
                return;
            };
            working(&ui, "Starting…", 0.0);
            let (tx, rx) = mpsc::channel();
            st.import_rx = Some(rx);
            let db = st.db.clone();
            std::thread::spawn(move || {
                let mut last = -1.0;
                let result = personal::import(&db, &imports, &req, &doc, &mut |text, f| {
                    // a message per percent is plenty
                    if f - last >= 0.01 || f >= 1.0 {
                        last = f;
                        let _ = tx.send(Msg::Progress(text.to_string(), f));
                    }
                });
                let _ = tx.send(Msg::Imported(result.map_err(|e| e.to_string())));
            });
        });
    }
    {
        let (w, state) = (ui.as_weak(), state.clone());
        ui.on_import_reset(move || {
            let ui = w.unwrap();
            state.borrow_mut().pending_import = None;
            ui.set_import_error(SharedString::default());
            ui.set_import_stage(CHOOSE);
        });
    }
    {
        let (w, state) = (ui.as_weak(), state.clone());
        ui.on_import_open(move || {
            let ui = w.unwrap();
            {
                let mut st = state.borrow_mut();
                let Some(idx) = st.last_import.and_then(|id| st.translations.iter().position(|t| t.id == id))
                else {
                    return;
                };
                // a new column, or the last one when all four are open
                if st.columns.len() < MAX_COLUMNS {
                    st.columns.push(idx);
                } else {
                    *st.columns.last_mut().unwrap() = idx;
                }
            }
            ui.set_show_import(false);
            ui.set_import_stage(CHOOSE);
            refresh_reading(&ui, &state);
        });
    }
    {
        let (w, state) = (ui.as_weak(), state.clone());
        ui.on_import_remove(move |code| {
            let ui = w.unwrap();
            let mut st = state.borrow_mut();
            let Some(id) = st.translations.iter().find(|t| t.code == code.as_str()).map(|t| t.id) else { return };
            let Some(imports) = st.imports.clone() else { return };
            working(&ui, &format!("Removing {code}…"), -1.0);
            let (tx, rx) = mpsc::channel();
            st.import_rx = Some(rx);
            std::thread::spawn(move || {
                let result = store::open(&imports).and_then(|mut conn| store::delete(&mut conn, id));
                let _ = tx.send(Msg::Removed(result.map_err(|e| e.to_string())));
            });
        });
    }
}

/// Link again, on a worker, the imports an older aligner linked.
pub fn relink_stale(state: &Rc<RefCell<State>>) {
    let mut st = state.borrow_mut();
    let Some(imports) = st.imports.clone() else { return };
    let db = st.db.clone();
    let (tx, rx) = mpsc::channel();
    st.relink_rx = Some(rx);
    std::thread::spawn(move || {
        let _ = tx.send(Msg::Relinked(personal::relink_stale(&db, &imports).map_err(|e| e.to_string())));
    });
}

/// Read `path` on a worker; the card shows what was found when it is done.
#[cfg_attr(target_os = "android", allow(dead_code))]
pub fn start_read(ui: &MainWindow, state: &Rc<RefCell<State>>, path: PathBuf) {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    working(ui, &format!("Reading {name}…"), -1.0);
    let (tx, rx) = mpsc::channel();
    let db = {
        let mut st = state.borrow_mut();
        st.import_rx = Some(rx);
        st.db.clone()
    };
    std::thread::spawn(move || {
        let read = || -> Result<Document, personal::Error> {
            let main = Connection::open_with_flags(&db, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
            personal::read(&path, &personal::matcher(&main)?)
        };
        let _ = tx.send(Msg::Read(read().map(|doc| (path, doc)).map_err(|e| e.to_string())));
    });
}

/// Take in what the workers have sent since the last tick.
pub fn poll(ui: &MainWindow, state: &Rc<RefCell<State>>) {
    let messages: Vec<Msg> = {
        let st = state.borrow();
        let card = st.import_rx.iter().flat_map(|rx| rx.try_iter());
        let relinking = st.relink_rx.iter().flat_map(|rx| rx.try_iter());
        card.chain(relinking).collect()
    };
    for msg in messages {
        match msg {
            Msg::Read(Ok((path, doc))) => {
                let taken: Vec<String> = state.borrow().translations.iter().map(|t| t.code.clone()).collect();
                let books = doc.verses.keys().map(|k| k.0).collect::<std::collections::BTreeSet<_>>().len();
                ui.set_import_file(path.display().to_string().into());
                ui.set_import_summary(
                    format!(
                        "{} verses in {} book{} · {}",
                        thousands(doc.verses.len()),
                        books,
                        if books == 1 { "" } else { "s" },
                        language_name(&doc.language)
                    )
                    .into(),
                );
                ui.set_import_notes(doc.notes.join(" ").into());
                ui.set_import_name(doc.title.clone().into());
                ui.set_import_code(personal::suggest_code(&doc.title, &taken).into());
                ui.set_import_lang(doc.language.clone().into());
                ui.set_import_error(SharedString::default());
                ui.set_import_stage(CONFIRM);
                state.borrow_mut().pending_import = Some((path, doc));
            }
            Msg::Read(Err(e)) => {
                ui.set_import_stage(CHOOSE);
                ui.set_import_error(e.into());
            }
            Msg::Progress(text, f) => {
                ui.set_import_progress_text(text.into());
                ui.set_import_progress(f);
            }
            Msg::Imported(Ok(report)) => {
                reload_translations(ui, state);
                let code = ui.get_import_code();
                let share = (report.linked * 100).checked_div(report.words).unwrap_or(0);
                ui.set_import_summary(
                    format!(
                        "{code} is imported: {} verses. {share}% of its words are linked to the Hebrew and Greek; click one to study it.",
                        thousands(report.verses)
                    )
                    .into(),
                );
                state.borrow_mut().last_import = Some(report.id);
                ui.set_import_stage(DONE);
            }
            Msg::Imported(Err(e)) | Msg::Removed(Err(e)) => {
                ui.set_import_stage(CHOOSE);
                ui.set_import_error(e.into());
            }
            Msg::Removed(Ok(())) => {
                reload_translations(ui, state);
                ui.set_import_stage(CHOOSE);
            }
            // the open chapter shows the new links
            Msg::Relinked(Ok(n)) if n > 0 => refresh_reading(ui, state),
            Msg::Relinked(_) => {}
        }
    }
}

fn thousands(n: usize) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// Read the translation list again after an import or a removal, keeping
/// the open columns on their texts — or closing one whose text is gone.
fn reload_translations(ui: &MainWindow, state: &Rc<RefCell<State>>) {
    {
        let mut st = state.borrow_mut();
        let Ok(list) = st.bible.translations() else { return };
        let open: Vec<String> = st.columns.iter().map(|&i| st.translations[i].code.clone()).collect();
        st.translations = list;
        let mut columns: Vec<usize> =
            open.iter().filter_map(|c| st.translations.iter().position(|t| &t.code == c)).collect();
        if columns.is_empty() {
            columns.push(0);
        }
        let primary_changed = open.first() != Some(&st.translations[columns[0]].code);
        st.columns = columns;
        // an id can be reused by the next import: forget what was built
        // from imported texts
        st.corpora.retain(|&id, _| id < store::FIRST_ID);
        if st.freq_tid >= store::FIRST_ID {
            st.freq_tid = 0;
        }
        if primary_changed {
            let id = st.primary_id();
            st.primary_books = st.bible.books(id).unwrap_or_default();
            if !st.primary_books.iter().any(|b| b.nr == st.book)
                && let Some(first) = st.primary_books.first()
            {
                (st.book, st.chapter) = (first.nr, 1);
            }
        }
        set_translation_lists(ui, &st);
    }
    refresh_books(ui, &state.borrow());
    refresh_reading(ui, state);
}

#[cfg(test)]
mod tests {
    #[test]
    fn thousands() {
        assert_eq!(super::thousands(31101), "31,101");
        assert_eq!(super::thousands(999), "999");
        assert_eq!(super::thousands(1000000), "1,000,000");
    }
}
