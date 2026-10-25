//! Where the reader left off: the passage, the panes that were open and the
//! texts in them, restored on the next launch.
//!
//! Everything here is a *hint*, never a constraint. Translations are keyed by
//! code and the reader's cut-off is stored as its value, so a saved session
//! stays meaningful when the database is rebuilt with a different set of texts
//! or the list of cut-offs changes. Anything that no longer resolves is simply
//! dropped and the default takes over — a stale session file must never stop
//! the app from starting.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Session {
    /// The open columns, left to right, as translation codes rather than
    /// list indices: indices shift whenever the database gains or loses a text.
    pub columns: Vec<String>,
    /// Before `columns`, a session held a primary and a secondary text and
    /// whether the second was shown. Read so an older file still restores
    /// its texts; never written.
    #[serde(skip_serializing)]
    pub primary: String,
    #[serde(skip_serializing)]
    pub secondary: String,
    #[serde(skip_serializing)]
    pub parallel: bool,
    pub book: i64,
    pub chapter: i64,
    pub scroll_lock: bool,
    pub reader: bool,
    /// The cut-off the reader chose by hand, as a count. `None` leaves each
    /// corpus on its own conventional default.
    pub threshold: Option<i64>,
    pub dark: bool,
    /// Scale of the Bible text (1.0 is the normal size).
    pub zoom: f32,
    /// Window size in *logical* pixels — physical ones would be re-scaled by
    /// the display factor on restore, so a HiDPI window would double each run,
    /// and the file would not survive moving between screens.
    pub width: Option<u32>,
    pub height: Option<u32>,
}

impl Default for Session {
    fn default() -> Self {
        Self {
            columns: Vec::new(),
            primary: String::new(),
            secondary: String::new(),
            parallel: false,
            book: 1,
            chapter: 1,
            scroll_lock: true,
            reader: false,
            threshold: None,
            dark: true,
            zoom: 1.0,
            width: None,
            height: None,
        }
    }
}

impl Session {
    /// The open columns, from `columns` or, in an older file, from the
    /// primary and secondary texts.
    pub fn column_codes(&self) -> Vec<String> {
        if !self.columns.is_empty() {
            return self.columns.clone();
        }
        let mut codes = vec![self.primary.clone()];
        if self.parallel {
            codes.push(self.secondary.clone());
        }
        codes
    }
}

/// `$LOGOS_SESSION`, else the platform config directory.
pub fn path() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("LOGOS_SESSION") {
        return Some(PathBuf::from(p));
    }
    directories::ProjectDirs::from("", "", "logos-rs")
        .map(|d| d.config_dir().join("session.json"))
}

/// Load the last session, or the defaults if there is nothing readable. A
/// corrupt file is treated as absent rather than reported: the reader cannot
/// act on the error, and losing the last passage is not worth a failed start.
pub fn load() -> Session {
    let Some(path) = path() else { return Session::default() };
    let Ok(raw) = std::fs::read_to_string(&path) else { return Session::default() };
    serde_json::from_str(&raw).unwrap_or_default()
}

/// Write the session, creating the config directory if needed. Failures are
/// silent for the same reason.
pub fn save(session: &Session) {
    let Some(path) = path() else { return };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(raw) = serde_json::to_string_pretty(session) {
        let _ = std::fs::write(&path, raw);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_and_missing_fields_fall_back() {
        // A file written by an older or newer build must still load.
        let s: Session = serde_json::from_str(r#"{"book":40,"whatever":true}"#).unwrap();
        assert_eq!(s.book, 40);
        assert_eq!(s.chapter, 1); // defaulted
        assert!(s.scroll_lock); // defaulted, not false
        assert_eq!(s.threshold, None);
    }

    #[test]
    fn a_two_text_session_becomes_columns() {
        let old: Session = serde_json::from_str(
            r#"{"primary":"KJV","secondary":"LSG","parallel":true}"#,
        )
        .unwrap();
        assert_eq!(old.column_codes(), ["KJV", "LSG"]);
        let single: Session =
            serde_json::from_str(r#"{"primary":"KJV","secondary":"LSG"}"#).unwrap();
        assert_eq!(single.column_codes(), ["KJV"]);
        let new: Session = serde_json::from_str(r#"{"columns":["WLC","KJV","LSG"]}"#).unwrap();
        assert_eq!(new.column_codes(), ["WLC", "KJV", "LSG"]);
        // the old fields are only ever read
        assert!(!serde_json::to_string(&new).unwrap().contains("primary"));
    }

    #[test]
    fn garbage_loads_as_default() {
        assert!(serde_json::from_str::<Session>("not json").is_err());
        let round = serde_json::to_string(&Session::default()).unwrap();
        let back: Session = serde_json::from_str(&round).unwrap();
        assert_eq!(back.chapter, 1);
    }
}
