//! Core (non-UI) library for logos-rs: canonical book data, the Bible data
//! model over SQLite, reference parsing, the data importer, the advanced
//! search engine, the saved session, and the reader's own imported
//! translations.

pub mod bible;
pub mod canon;
pub mod importer;
pub mod morph;
pub mod personal;
pub mod reference;
pub mod search;
pub mod session;
