# logos-rs

A Linux desktop Bible study app inspired by [Logos](https://www.logos.com/),
built with **Rust** + **[Slint](https://slint.dev/)**. The goal is a Logos-like
study experience that runs natively on Linux with a cleaner, faster UX.

![reading view](docs/reading.png)

## Features

- **Reading view** with hanging-indent verse numbers and a Logos-style three-pane
  layout (translation/book/chapter nav · reading pane).
- **Parallel view** — read two translations side by side, aligned by verse.
- **Right-to-left** rendering for Hebrew.
- **Command box** — type a reference (`John 3:16`, `1 Cor 13:4-7`, `Ps 23`) to jump
  there, or any other text to run a full-text search.
- **Full-text search** (SQLite FTS5), ranked, with match highlighting. Greek search
  is accent-insensitive.
- **Dark / light** themes.

## Translations

All texts are imported from [getbible.net v2](https://getbible.net/) into a local
SQLite database. The five bundled translations:

| Code | Translation | Language | Notes |
|------|-------------|----------|-------|
| KJV  | King James Version | English | Public domain |
| LSG  | Louis Segond (1910) | French | Public domain |
| WLC  | Westminster Leningrad Codex | Hebrew (OT) | Open digital edition of the Leningrad Codex — the same base text as *Biblia Hebraica Stuttgartensia* (BHS itself is copyright Deutsche Bibelgesellschaft, so the open WLC is used in its place) |
| WH   | Westcott–Hort | Greek (NT) | Public-domain critical Greek text (the modern UBS/NA "Greek New Testament" is copyrighted) |
| LXX  | Septuagint | Greek (OT) | Includes deuterocanonical books |

Book numbering is aligned across translations using getbible's canonical book id
(1–39 OT, 40–66 NT), so parallel view lines up correctly between KJV/LSG/WH and the
original-language texts.

## Building & running

Requires a recent stable Rust toolchain.

```sh
# 1. Build the local Bible database (downloads ~70 MB of JSON the first time,
#    caches it under data/, and produces data/bible.db).
cargo run --bin import

# 2. Run the app.
cargo run --bin logos-rs
```

The app looks for the database via `$LOGOS_DB`, then `data/bible.db`.

## Architecture

```
src/
  canon.rs      Canonical 66-book table + abbreviations (reference parsing)
  reference.rs  Forgiving Scripture-reference parser ("1 Cor 13:4-7")
  bible.rs      Read-only data model over SQLite (books, chapters, FTS search)
  importer.rs   Downloads getbible JSON → builds bible.db (+ FTS5 index)
  main.rs       Slint UI wiring (state, callbacks)
ui/
  app.slint     UI: theme, nav, reading columns, search, parallel view
```

## Known limitations / TODO

- **Hebrew search**: FTS5's `remove_diacritics` strips Greek accents but not Hebrew
  vowel points/cantillation, so unpointed Hebrew searches don't match yet. Planned
  fix: index a points-stripped copy of the Hebrew text.
- No cross-references, notes, highlighting, or original-language lemma lookup yet.
- The database is generated locally rather than bundled.
