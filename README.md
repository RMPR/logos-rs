# logos-rs

A Linux desktop Bible study app inspired by [Logos](https://www.logos.com/),
built with **Rust** + **[Slint](https://slint.dev/)**. The goal is a Logos-like
study experience that runs natively on Linux with a cleaner, faster UX.

![reading view](docs/reading.png)

## Features

- **Reading view** with hanging-indent verse numbers and a Logos-style three-pane
  layout (translation/book/chapter nav · reading pane).
- **Parallel view** — read up to four translations side by side (**＋ Column**;
  each column is headed by its own translation picker and a ✕), genuinely
  aligned: linked (the default) renders all the texts as a *single* list of
  aligned verses, so verse N always sits beside verse N and the columns cannot
  drift apart. The **🔗 Linked** pill unlinks them into independent scrollers.
  The words wrap to each column's width.
- **Text size** — `Ctrl+plus` / `Ctrl+minus` zoom the Bible text, `Ctrl+=` (or
  `Ctrl+0`) puts it back; the size is remembered.
- **Import your own Bibles** — **⇪ Import** reads an EPUB, a PDF, a Logos
  personal-book DOCX or a text file you own, finds its books, chapters and
  verses, and links its words to the Hebrew and Greek, so it reads, searches and
  clicks like the bundled texts. Imports stay on your computer, in
  `imports.db` in the user data directory (`~/.local/share/logos-rs/` on Linux,
  or `$LOGOS_IMPORTS`), and can be removed from the same card. See
  [Importing a translation](#importing-a-translation).
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
- **Copying** — drag across words to select them, across verses if need be,
  and `Ctrl+C` copies them (a plain click on a word still opens the word study).
  Click a verse number to mark it, shift-click another to mark the run, and
  `Ctrl+C` copies the verses headed by their reference (`Matthew 1:1–3 (ESV)`);
  `Esc` clears. Right-click a verse for *Copy verse* / *Copy chapter*. The
  word-study panel and the reader's apparatus are ordinary selectable text.
- **Localized book names** — the navigation shows each translation's book names in
  its own language (Genèse, בְּרֵאשִׁית, Κατά Ιωάννην, Første Mosebok, …).
- **Right-to-left** rendering for Hebrew.
- **Command box** — type a reference (`John 3:16`, `1 Cor 13:4-7`, `Ps 23`) to jump
  there, or any other text to run a full-text search.
- **Full-text search** (SQLite FTS5), ranked, with match highlighting. Greek search
  is accent-insensitive.
- **Dark / light** themes.

## Importing a translation

**⇪ Import** → **Choose a file…**, check the name, code and language the app
read from the file, and **Import**. Reading takes a second or two; linking the
words to the Hebrew and Greek a few seconds more.

| Format | How verses are found |
|--------|----------------------|
| EPUB | Crossway (ESV), Société biblique française (BFC) and Bibelselskapet (Bokmål 2011) layouts are read exactly. Any other EPUB is read from what Bible EPUBs share: headings naming the books and chapters, and verse numbers in elements of their own; notes and cross-references are skipped. |
| PDF | Best effort: a header naming book and chapter ("Genesis 3", often a running page header), and verses that start a line with their number. A PDF in two columns, or with verse numbers inside paragraphs, loses verses. |
| DOCX | Logos personal-book milestones (`[[@Bible:Gen 1:1]]`, with `{{field-on:bible}}…{{field-off:bible}}` fences); without them, read as plain text a paragraph a line. |
| Text | As a PDF. |

Its words are then linked to the Hebrew and Greek like every text's (see
[How words are linked](#how-words-are-linked)). Only import texts you are
entitled to use. Imports made before an improvement of the linking are linked
again, in the background, the next time the app starts.

## How words are linked

Every word of every translation — the KJV, ULT, Louis Segond, ESV, BFC,
Bokmål, the Septuagint, the Vulgate and your imports — is linked to the Hebrew
or Greek word it renders, so a click opens that word's study. Where people
tagged a text (the KJV's Strong's numbers, the ULT's alignment) their tagging
is kept and only the words it leaves untagged are filled in.

- **Verses are matched by content, not number.** The Hebrew counts a psalm's
  title as verse 1, Malachi 4 is its 3:19–24, the Septuagint joins Psalms 9 and
  10, translations print two verses as one. STEPBible's versification table
  (TVTMS) is evaluated against each Bible's own verses to work out its
  numbering tradition, without being told it; an original verse no verse of
  the translation claims (a title printed in verse 1, a "9–10") goes to the
  neighbour whose length it explains. The linked parallel view rows up verses
  the same way: the KJV's Psalm 3:1 sits beside the Septuagint's 1–2.
- **Words are aligned with the fast_align model** (Dyer et al., 2013), trained
  in both directions over the whole Bible: words compared by stem (Snowball for
  English, French, Norwegian…, a Schinke stemmer for Latin, a light Koine one
  for Greek), a preference for words at the same place in the verse, sparse
  (variational-Bayes) estimates, and links the two directions agree on. Against
  the KJV's own Strong's tagging, re-aligning its text gets 86% of the tagged
  words right (the IBM Model 1 used before: 59%).
- **Text with no original** — the Greek additions to Esther and Daniel, the
  Septuagint's own books — is linked by what its words mean elsewhere in the
  same translation, and Greek words by their lexeme: the SBLGNT's tagged forms
  and STEPBible's extended Strong's numbers for the Septuagint's vocabulary.

Coverage of the bundled texts: 100% of words for all but the Septuagint, which
is at 99.8% — the rest are names and rare words of its deuterocanonical books
(Holofernes, Bethulia…) that no lexicon here knows.

## Translations & data sources

The bundled translations:

| Code | Translation | Language | Tagged | Source / notes |
|------|-------------|----------|:------:|----------------|
| KJV  | King James Version | English | ✓ | [bolls.life](https://bolls.life/) KJV (1769) with Strong's numbers — public domain; clickable reverse interlinear |
| ULT  | unfoldingWord Literal Text | English | ✓ | [unfoldingWord ULT](https://git.door43.org/unfoldingWord/en_ult) — every word aligned to the original Hebrew/Greek with Strong's (CC BY-SA); powers the reverse interlinear |
| LSG  | Louis Segond (1910) | French | ✓ | public-domain Segond; words linked by the aligner; clickable reverse interlinear |
| WLC  | Westminster Leningrad Codex | Hebrew (OT) | ✓ | [OpenScriptures morphhb](https://github.com/openscriptures/morphhb) — the open Leningrad text BHS is based on, with per-word lemma + OSHM morphology |
| SBLGNT | SBL Greek New Testament | Greek (NT) | ✓ | [MorphGNT](https://github.com/morphgnt/sblgnt) — per-word parse code + lemma |
| LXX  | Septuagint | Greek (OT) | ✓ | getbible.net — includes deuterocanonical books; words linked to the Hebrew they render, those of its own books to their Greek lexeme |
| VUL  | Vulgata Clementina | Latin | ✓ | getbible.net — 66 canonical books only (deuterocanon dropped); words linked by the aligner |

Optional, **user-provided** translations built into `bible.db` itself
(copyrighted — supplied by the user from licensed EPUBs, **not committed** to
the repo). For your own use, importing them in the app does the same; for a
build that bundles them, place the publishers' EPUBs in `data/` (they are
recognised by their layout, whatever their file names) before building the
database. The importer includes each if present and skips it if not:

| Code | Translation | Language | From |
|------|-------------|----------|------|
| ESV  | English Standard Version | English | `ESV Bible - Crossway.epub` |
| BFC  | Bible en français courant | French | `…français courant…Collectif.epub` (deuterocanon + notes stripped) |
| NB   | Bibelen 2011 (Bokmål) | Norwegian | `Bibelen - bokmal - Bibelselskapet.epub` |

Their words are linked like every text's, so they're clickable reverse
interlinears when present.

"Tagged" translations carry word-level data: WLC/SBLGNT have full morphology;
ULT carries each English word's Strong's number (normalised to match WLC/SBLGNT,
so an English click resolves to the actual original word in the verse).
Definitions come from the OpenScriptures **Strong's lexicons** (Hebrew XML + Greek
JSON). Greek lemmas are bridged to Strong's numbers by accent-insensitive matching
(≈98% of NT words resolve). Hebrew lemmas are Strong's numbers directly.

Two datasets from [STEPBible](https://www.STEPBible.org) (Tyndale House
Cambridge, CC BY 4.0) are vendored under `data/`: **TVTMS**, the versification
traditions table that matches verses across Bibles (`data/tvtms.txt.gz`), and
**TBESG**, the brief lexicon of extended Strong's numbers for Greek, which
names the Septuagint's words and gives them definitions
(`data/lexicon/tbesg.txt.gz`).

Book numbering is aligned across translations using a canonical book id
(1–39 OT, 40–66 NT), so parallel view and cross-translation word lookup line up.

## Installing

Prebuilt packages of **LeVerbe** are attached to each
[GitHub release](https://github.com/rmpr/logos-rs/releases). Both include the
complete Bible database, so the app works offline from the first launch.

- **Ubuntu 26.04 (Resolute Raccoon)**: `sudo apt install ./leverbe_<version>_amd64.deb`,
  then start **LeVerbe** from the app grid or run `leverbe`.
- **Windows (x64)**: run `LeVerbe-<version>-windows-x64-setup.exe` (installs per
  user, no admin rights needed), or unzip the portable
  `LeVerbe-<version>-windows-x64.zip` and run `LeVerbe.exe`.

Releases are built by `.github/workflows/release.yml`: push a tag `vX.Y.Z` to
publish one; running the workflow by hand only uploads the packages as build
artifacts. The packaging files live under `packaging/`.

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

- **Most links are statistical**: outside the KJV's and ULT's own tagging,
  words are linked by the aligner. Content words link well (86% agreement with
  the KJV's tagging); function words go to the original word nearest in sense
  and place, as in a printed reverse interlinear, and some links are wrong.
- **The Septuagint's own books** keep about 1,400 words unlinked: names and
  rare words no bundled lexicon has an entry for.
- **Hebrew search**: FTS5's `remove_diacritics` strips Greek accents but not Hebrew
  vowel points/cantillation, so unpointed Hebrew searches don't match yet. Planned
  fix: index a points-stripped copy of the Hebrew text (the `words` lemmas now make
  this straightforward).
- No cross-references, notes, or highlighting yet.
- The database is generated locally rather than bundled.
