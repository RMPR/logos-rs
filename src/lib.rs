//! The library behind LeVerbe: canonical book data, the Bible data model over
//! SQLite, reference parsing, the data importer, the advanced search engine,
//! the saved session, the reader's own imported translations — and `app`, the
//! window, which the desktop executable and the Android app both run.

pub mod app;
pub mod bible;
pub mod canon;
pub mod importer;
pub mod morph;
pub mod personal;
pub mod reference;
pub mod search;
pub mod session;
pub mod versification;
