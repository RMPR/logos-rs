// No console window behind the app in Windows release builds.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

use logos_rs::app::{self, Options};

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
    // LEVERBE_TOUCH=1 runs the tablet's layout, as on Android
    let touch = std::env::var_os("LEVERBE_TOUCH").is_some();
    app::run(Options {
        db,
        session: logos_rs::session::path(),
        imports: logos_rs::personal::store::path(),
        touch,
        // the tablet layout as the tablet has it: no import card
        can_import: !touch,
        prepare: None,
    })
}
