# logos-rs

A Linux desktop Bible study app inspired by [Logos](https://www.logos.com/),
built with **Rust** + **[Slint](https://slint.dev/)**. The goal is a Logos-like
study experience that runs natively on Linux with a cleaner, faster UX.

![reading view](docs/reading.png)

## Features

- **Reading view** with hanging-indent verse numbers and a Logos-style three-pane
  layout (translation/book/chapter nav · reading pane).
- **Parallel view** — read two translations side by side, genuinely aligned:
  locked (the default) renders both texts as a *single* list of paired verses,
  so verse N always sits beside verse N and the columns cannot drift apart. The
  **🔗 Linked** pill unlocks them into two independent scrollers.
- **Search** — `Ctrl+F` jumps to the search box from anywhere. Hits are listed
  in canonical order (Genesis → Revelation), and opening one lands on that verse,
  tinted, rather than at the top of its chapter; **☰ Results** in the header
  brings the list back, **✕ Close** dismisses it.
- **Advanced search** — `Ctrl+Shift+F` (or **⌕ Advanced**) opens a reference card
  for a small query language, typed into the same box. It runs over the tagged
  Hebrew OT / Greek NT and shows the hits in whatever translation you are
  reading, with the matching words marked — search in Greek, read the hits in
  French. Whole-corpus, in memory, typically 20–150 ms.

  ```text
  G25   H430   lemma:ἀγαπάω   gloss:love      one word, by Strong's, lemma or gloss
  [lemma:ἀγαπάω aorist active]                constraints on the SAME word
  tense: voice: mood: case: number: gender: person: stem: state: — or the bare value
  A B · A AND B · A OR B · NOT A · ( … )      per verse
  A BEFORE/2 B · A AFTER/1 B · A NEAR/3 B    word order, within n words
  [verb aorist] BEFORE/1 lemma:θεός           an aorist verb right before "God"
  ```
- **Picks up where you left off** — the passage, both translations, which panes
  were open, the theme and the window size are restored on the next launch
  (`~/.config/logos-rs/session.json`, or `$LOGOS_SESSION`).
- **Reader's edition** — open the Hebrew OT (WLC) or Greek NT (SBLGNT) and hit
  **📖 Reader**: every word rarer than the chosen cut-off is ruled in the text and
  glossed below it with its dictionary form, full parsing and corpus frequency, so
  a chapter can be read straight through without a lexicon. Defaults follow the
  printed editions — fewer than 100× for the Hebrew Bible, fewer than 30× for the
  Greek NT — and proper names are left out, as they are in print. No complete
  parsed Hebrew OT exists in book or PDF form; this is that, generated per chapter.
- **Word study** — in the original-language texts (Hebrew OT, Greek NT) every word
  is clickable. A side panel shows the surface form, transliteration, lemma,
  Strong's number, full morphological parsing, gloss, and definition — inspired by
  [hb.openscriptures.org](https://hb.openscriptures.org/) and
  [biblewebapp.com](https://biblewebapp.com/reader/).
- **Reverse interlinear** — click a word in **KJV**, **ULT**, **Louis Segond** (or the **ESV**, when present) and the panel shows
  the original word it translates: the Greek for the NT, and the Hebrew **plus the
  parallel Septuagint verse** for the OT (lemma, Strong's, parsing, definition).
  The parallel **Vulgate (Latin)** verse is shown as a reference for both.
- **Localized book names** — the navigation shows each translation's book names in
  its own language (Genèse, בְּרֵאשִׁית, Κατά Ιωάννην, Første Mosebok, …).
- **Right-to-left** rendering for Hebrew.
- **Command box** — type a reference (`John 3:16`, `1 Cor 13:4-7`, `Ps 23`) to jump
  there, or any other text to run a full-text search.
- **Full-text search** (SQLite FTS5), ranked, with match highlighting. Greek search
  is accent-insensitive.
- **Dark / light** themes.

## Translations & data sources

The five bundled translations:

| Code | Translation | Language | Tagged | Source / notes |
|------|-------------|----------|:------:|----------------|
| KJV  | King James Version | English | ✓ | [bolls.life](https://bolls.life/) KJV (1769) with Strong's numbers — public domain; clickable reverse interlinear |
| ULT  | unfoldingWord Literal Text | English | ✓ | [unfoldingWord ULT](https://git.door43.org/unfoldingWord/en_ult) — every word aligned to the original Hebrew/Greek with Strong's (CC BY-SA); powers the reverse interlinear |
| LSG  | Louis Segond (1910) | French | ✓ | public-domain Segond + a Strong's correspondence **built by statistical word-alignment** (`tools/build_lsg_strongs.py`); clickable reverse interlinear |
| WLC  | Westminster Leningrad Codex | Hebrew (OT) | ✓ | [OpenScriptures morphhb](https://github.com/openscriptures/morphhb) — the open Leningrad text BHS is based on, with per-word lemma + OSHM morphology |
| SBLGNT | SBL Greek New Testament | Greek (NT) | ✓ | [MorphGNT](https://github.com/morphgnt/sblgnt) — per-word parse code + lemma |
| LXX  | Septuagint | Greek (OT) | | getbible.net — includes deuterocanonical books |
| VUL  | Vulgata Clementina | Latin | | getbible.net — 66 canonical books only (deuterocanon dropped); shown as the Latin reference in the word-study panel |

Optional, **user-provided** translations (copyrighted — supplied by the user from
licensed EPUBs, **not committed** to the repo). Place the EPUBs and run
`tools/epub2json.py` to generate `data/<slug>.json`, then optionally
`tools/build_strongs.py` to add a Strong's correspondence (making them clickable
reverse interlinears). The importer includes each if present and skips it if not:

| Code | Translation | Language | From |
|------|-------------|----------|------|
| ESV  | English Standard Version | English | `ESV Bible - Crossway.epub` |
| BFC  | Bible en français courant | French | `…français courant…Collectif.epub` (deuterocanon + notes stripped) |
| NB   | Bibelen 2011 (Bokmål) | Norwegian | `Bibelen - bokmal - Bibelselskapet.epub` |

All three also get a **built Strong's correspondence** (`tools/build_strongs.py`),
so they're clickable reverse interlinears when present.

"Tagged" translations carry word-level data: WLC/SBLGNT have full morphology;
ULT carries each English word's Strong's number (normalised to match WLC/SBLGNT,
so an English click resolves to the actual original word in the verse).
Definitions come from the OpenScriptures **Strong's lexicons** (Hebrew XML + Greek
JSON). Greek lemmas are bridged to Strong's numbers by accent-insensitive matching
(≈98% of NT words resolve). Hebrew lemmas are Strong's numbers directly.

Book numbering is aligned across translations using a canonical book id
(1–39 OT, 40–66 NT), so parallel view and cross-translation word lookup line up.

## Building & running

Requires a recent stable Rust toolchain.

```sh
cargo run --bin logos-rs
```

That's it. On first launch the app builds the SQLite database
(`data/bible.db`, ~185k verses, ~1.2M tagged words) from the source texts
**vendored in the repo** (compressed under `data/`, ~24 MB) — so it works
**fully offline** and takes about 10 seconds, once. The database itself is
generated, not committed.

- Rebuild the database explicitly: `cargo run --bin import`.
- Point the app at a prebuilt database: set `$LOGOS_DB=/path/to/bible.db`.
- The importer reads each source from its committed `.gz`; if a raw (un-gzipped)
  copy is present it uses that, and only downloads from the network if a source
  is missing entirely.

## Architecture

```
src/
  canon.rs      Canonical 66-book table + abbreviations (reference parsing)
  reference.rs  Forgiving Scripture-reference parser ("1 Cor 13:4-7")
  morph.rs      Morphology decoders: OSHM (Hebrew) + MorphGNT (Greek) → prose
  bible.rs      Read-only data model over SQLite (verses, words, lexicon, FTS)
  importer.rs   Builds bible.db from getbible/morphhb/MorphGNT/ULT + Strong's lexicons
  main.rs       Slint UI wiring (state, callbacks, word wrapping)
ui/
  app.slint     UI: theme, nav, reading columns, search, parallel, word study
```

Database tables: `translations`, `books`, `verses` (+ `verses_fts`),
`words` (per-word lemma/Strong's/morphology), `lexicon` (Strong's definitions).

## Known limitations / TODO

- **Some Strong's correspondences are statistical**: where no Strong's-tagged
  edition exists (Louis Segond, ESV), the correspondence is *built* by IBM-Model-1
  word alignment (translation ↔ the Strong's in each verse from WLC/SBLGNT). Content
  words align well; some words are left untagged or occasionally mis-attached.
  Regenerate with `tools/build_strongs.py <db> <CODE> <out.json>`; for the
  EPUB-derived texts add `--source data/<slug>.json` (the output of
  `tools/epub2json.py`), since they enter the database only through this
  script's output.
- **LXX reverse lookup is verse-level**: no open, license-clean Septuagint is
  word-aligned to the Hebrew or carries Strong's, so an OT English click shows the
  precise Hebrew word but the LXX as the whole parallel verse (not the single Greek
  word). Word-level LXX alignment can follow if suitable data appears.
- **Hebrew search**: FTS5's `remove_diacritics` strips Greek accents but not Hebrew
  vowel points/cantillation, so unpointed Hebrew searches don't match yet. Planned
  fix: index a points-stripped copy of the Hebrew text (the `words` lemmas now make
  this straightforward).
- Word lines in tagged texts are wrapped using a character-width estimate (Slint has
  no flow layout), so they don't reflow on window resize.
- No cross-references, notes, or highlighting yet.
- The database is generated locally rather than bundled.
