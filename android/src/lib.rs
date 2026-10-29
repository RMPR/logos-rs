//! LeVerbe on Android — written with e-ink tablets in mind. The window is the
//! desktop app's (`logos_rs::app`) in its touch layout; what is Android's own
//! is where the files live, and the Bible database, which ships compressed
//! in the APK and is unpacked into the app's storage on first launch (SQLite
//! cannot read it from inside the package).
//!
//! Elsewhere the crate is empty, so the workspace still builds on a desktop.
#![cfg(target_os = "android")]

use std::ffi::CString;
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use logos_rs::app::{self, Options};
use slint::android::AndroidApp;

/// The database among the APK's assets. The APK's zip compression packs it
/// to a third; the asset manager inflates it as it is read.
const ASSET: &str = "bible.db";

#[unsafe(no_mangle)]
fn android_main(app: AndroidApp) {
    // Nothing reads an Android app's stderr; a failure to start leaves the
    // screen blank, so there is no one to tell beyond that.
    if let Err(e) = run(app) {
        eprintln!("LeVerbe: {e}");
    }
}

fn run(app: AndroidApp) -> Result<(), Box<dyn std::error::Error>> {
    let data = app.internal_data_path().ok_or("Android gave the app no storage")?;
    slint::android::init(app.clone())?;
    let db = data.join("bible.db");
    let stamp = data.join("bible.db.version");
    // The copy is current when it came from this very asset: same app
    // version, same size. An update that ships a new database replaces it.
    let size = asset(&app)?.length();
    let wanted = format!("{} {size}", env!("CARGO_PKG_VERSION"));
    let current = db.is_file() && fs::read_to_string(&stamp).is_ok_and(|s| s == wanted);
    let prepare: Option<app::Prepare> = (!current).then(|| -> app::Prepare {
        let db = db.clone();
        Box::new(move |progress: &dyn Fn(f32)| {
            unpack(&app, &db, size as u64, progress).map_err(|e| e.to_string())?;
            fs::write(&stamp, &wanted).map_err(|e| e.to_string())
        })
    });
    app::run(Options {
        db,
        session: Some(data.join("session.json")),
        // importing needs a file picker, which this build does not have yet
        imports: None,
        touch: true,
        can_import: false,
        prepare,
    })
}

fn asset(app: &AndroidApp) -> io::Result<ndk::asset::Asset> {
    let name = CString::new(ASSET).expect("asset name");
    app.asset_manager()
        .open(&name)
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, format!("{ASSET} is missing from the APK")))
}

/// Unpack the database next to where it goes and move it into place, so a
/// launch interrupted half-way never leaves a truncated database behind.
fn unpack(app: &AndroidApp, db: &Path, size: u64, progress: &dyn Fn(f32)) -> io::Result<()> {
    let part = PathBuf::from(format!("{}.part", db.display()));
    let mut reader = Counting { inner: asset(app)?, read: 0, size, reported: -1, progress };
    let mut out = io::BufWriter::with_capacity(1 << 20, fs::File::create(&part)?);
    io::copy(&mut reader, &mut out)?;
    out.into_inner().map_err(|e| e.into_error())?.sync_all()?;
    // the old database's journal must not be replayed into the new one
    for suffix in ["-wal", "-shm"] {
        let _ = fs::remove_file(format!("{}{suffix}", db.display()));
    }
    fs::rename(&part, db)?;
    progress(1.0);
    Ok(())
}

/// Reports how much of the asset has been read, a percent at a time — each report is a redraw, which an e-ink screen pays for.
struct Counting<'a, R> {
    inner: R,
    read: u64,
    size: u64,
    reported: i32,
    progress: &'a dyn Fn(f32),
}

impl<R: Read> Read for Counting<'_, R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.read += n as u64;
        let percent = (self.read * 100 / self.size.max(1)) as i32;
        if percent > self.reported {
            self.reported = percent;
            (self.progress)(percent as f32 / 100.0);
        }
        Ok(n)
    }
}
