#!/usr/bin/env python3
"""Convert publisher EPUB Bibles into the getbible-style JSON the Rust importer
consumes ({lang, books:[{nr, name, chapters:[{verses:[{chapter,verse,text}]}]}]}).

Three publisher layouts are supported (each marks verses differently):
  esv  — ESV (Crossway): *.text.html, book from filename `bNN`,
         span.chapter-num / span.verse-num markers, a.footnote stripped.
  fr   — La Bible en français courant: Book-N.xml scripture files,
         <a class="verse" id="vCode.C.V"/> markers; notes (-notes.xml files,
         span.displayReference, a.note-link-in-text) and deuterocanonical
         books (codes not in the 66-book canon) dropped.
  no   — Bibelen 2011 (Bokmål): NNNCODE.xhtml, book from the 3-digit prefix,
         span.versenumberdropcap = chapter start, span.versenumber = verse.

Usage: epub2json.py <esv|fr|no> <epub_path> <out.json> <lang> "<name>" <ABBR>
"""
import json
import re
import sys
import zipfile

from bs4 import BeautifulSoup, NavigableString

OSIS_NR = {
    "Gen": 1, "Exod": 2, "Lev": 3, "Num": 4, "Deut": 5, "Josh": 6, "Judg": 7, "Ruth": 8,
    "1Sam": 9, "2Sam": 10, "1Kgs": 11, "2Kgs": 12, "1Chr": 13, "2Chr": 14, "Ezra": 15,
    "Neh": 16, "Esth": 17, "Job": 18, "Ps": 19, "Prov": 20, "Eccl": 21, "Song": 22,
    "Isa": 23, "Jer": 24, "Lam": 25, "Ezek": 26, "Dan": 27, "Hos": 28, "Joel": 29,
    "Amos": 30, "Obad": 31, "Jonah": 32, "Mic": 33, "Nah": 34, "Hab": 35, "Zeph": 36,
    "Hag": 37, "Zech": 38, "Mal": 39, "Matt": 40, "Mark": 41, "Luke": 42, "John": 43,
    "Acts": 44, "Rom": 45, "1Cor": 46, "2Cor": 47, "Gal": 48, "Eph": 49, "Phil": 50,
    "Col": 51, "1Thess": 52, "2Thess": 53, "1Tim": 54, "2Tim": 55, "Titus": 56,
    "Phlm": 57, "Heb": 58, "Jas": 59, "1Pet": 60, "2Pet": 61, "1John": 62, "2John": 63,
    "3John": 64, "Jude": 65, "Rev": 66,
}

WS = re.compile(r"\s+")

# Elements that end a line or paragraph. Publishers mark poetry with one block
# per line and no whitespace between them (BFC: <div class="osis_l">…,</div>
# <div class="osis_l">…), so the text on either side would otherwise fuse —
# "Dieu,la voûte". A block boundary is always a word boundary in a Bible, and
# clean() collapses any doubled spaces afterwards.
BLOCK = {
    "p", "div", "br", "li", "ul", "ol", "tr", "td", "th", "table", "blockquote",
    "section", "article", "hr", "h1", "h2", "h3", "h4", "h5", "h6",
}


def clean(text):
    return WS.sub(" ", text).strip()


def classes(tag):
    return tag.get("class") or []


# verses: dict[(nr, chapter, verse)] -> accumulated text
def add(verses, key, s):
    if key is not None and key[1] and key[2]:
        verses[key] = verses.get(key, "") + s


def descend(verses, key, tag, walk):
    """Recurse into `tag`, padding a block element with spaces so the text
    before and after it never runs together."""
    block = tag.name in BLOCK
    if block:
        add(verses, key, " ")
    walk(tag)
    if block:
        add(verses, key, " ")


# --------------------------------------------------------------------------- ESV
def parse_esv(zf, verses):
    for name in sorted(zf.namelist()):
        m = re.search(r"/b(\d{2})\.\d+\.[^/]*\.text\.html$", name)
        if not m:
            continue
        book = int(m.group(1))
        soup = BeautifulSoup(zf.read(name), "lxml")
        for a in soup.select(".footnote, .crossref"):  # class is on the wrapping span
            a.decompose()
        body = soup.body
        if body is None:
            continue
        state = {"c": None, "v": None}

        def walk(node):
            for ch in node.children:
                if isinstance(ch, NavigableString):
                    add(verses, (book, state["c"], state["v"]), str(ch))
                elif ch.name:
                    cls = classes(ch)
                    if "heading" in cls:
                        continue
                    if "chapter-num" in cls:
                        num = re.sub(r"\D", "", ch.get_text())
                        state["c"] = int(num) if num else state["c"]
                        state["v"] = 1
                        continue
                    if "verse-num" in cls:
                        num = re.sub(r"\D", "", ch.get_text())
                        state["v"] = int(num) if num else state["v"]
                        continue
                    if "book-name" in cls:
                        continue
                    descend(verses, (book, state["c"], state["v"]), ch, walk)

        walk(body)


# --------------------------------------------------------------------------- French
FR_VERSE_ID = re.compile(r"^v([A-Za-z0-9]+)\.(\d+)\.(\d+)$")


def walk_fr(body, verses):
    """Accumulate the verse text under `body` (one BFC chapter file's <body>)
    into `verses`. Split out from parse_fr so it can be tested on a snippet."""
    state = {"key": None}

    def walk(node):
        for ch in node.children:
            if isinstance(ch, NavigableString):
                add(verses, state["key"], str(ch))
            elif ch.name:
                if ch.name in ("h1", "h2", "h3", "h4"):
                    continue
                if ch.name == "a" and "verse" in classes(ch):
                    m = FR_VERSE_ID.match(ch.get("id", ""))
                    if m and m.group(1) in OSIS_NR:
                        state["key"] = (OSIS_NR[m.group(1)], int(m.group(2)), int(m.group(3)))
                    else:
                        state["key"] = None  # deuterocanonical / unknown
                    continue
                descend(verses, state["key"], ch, walk)

    walk(body)


def parse_fr(zf, verses):
    for name in sorted(zf.namelist()):
        base = name.rsplit("/", 1)[-1]
        if not re.match(r"^[A-Za-z0-9]+-\d+\.xml$", base):  # scripture chapters only
            continue
        soup = BeautifulSoup(zf.read(name), "lxml")
        for s in soup.select("span.displayReference, a.note-link-in-text"):
            s.decompose()
        body = soup.body
        if body is None:
            continue
        walk_fr(body, verses)


# --------------------------------------------------------------------------- Norwegian
def parse_no(zf, verses):
    def sort_key(n):
        base = n.rsplit("/", 1)[-1]
        m = re.match(r"^(\d{3})([A-Z0-9]+?)(?:-([A-Z]))?\.xhtml$", base)
        if not m:
            return (999, "", "")
        return (int(m.group(1)), m.group(3) or "")  # base file ("") before -B/-C

    files = [n for n in zf.namelist() if re.search(r"(?:^|/)\d{3}[A-Z0-9]+(?:-[A-Z])?\.xhtml$", n)]
    cur_book = None
    state = {"c": None, "v": None}
    for name in sorted(files, key=sort_key):
        base = name.rsplit("/", 1)[-1]
        book = int(base[:3])
        if book > 66:  # glossary etc.
            continue
        if book != cur_book:
            cur_book, state["c"], state["v"] = book, None, None
        soup = BeautifulSoup(zf.read(name), "lxml")
        body = soup.body
        if body is None:
            continue

        def walk(node):
            for ch in node.children:
                if isinstance(ch, NavigableString):
                    add(verses, (book, state["c"], state["v"]), str(ch))
                elif ch.name:
                    cls = classes(ch)
                    if "versenumberdropcap" in cls:
                        txt = ch.get_text()
                        num = re.sub(r"\D", "", txt)
                        if num:  # numeric drop-cap = chapter number (skip the digits)
                            state["c"], state["v"] = int(num), 1
                        else:  # decorative letter drop-cap (single-chapter book): keep it
                            state["c"], state["v"] = state["c"] or 1, 1
                            add(verses, (book, state["c"], state["v"]), txt)
                        continue
                    if "dropcap" in cls:  # plain decorative initial (e.g. Obadiah)
                        state["c"], state["v"] = state["c"] or 1, state["v"] or 1
                        add(verses, (book, state["c"], state["v"]), ch.get_text())
                        continue
                    if "versenumber" in cls:
                        num = re.sub(r"\D", "", ch.get_text())
                        state["v"] = int(num) if num else state["v"]
                        continue
                    if "verseheader" in cls:
                        continue
                    descend(verses, (book, state["c"], state["v"]), ch, walk)

        walk(body)


def build(verses, lang, name, abbrev):
    books = {}
    for (nr, c, v), txt in verses.items():
        books.setdefault(nr, {}).setdefault(c, {})[v] = clean(txt)
    out = []
    for nr in sorted(books):
        chapters = [
            {"chapter": c, "verses": [
                {"chapter": c, "verse": v, "text": books[nr][c][v]}
                for v in sorted(books[nr][c]) if books[nr][c][v]
            ]}
            for c in sorted(books[nr])
        ]
        out.append({"nr": nr, "name": f"Book {nr}", "chapters": chapters})
    return {"translation": name, "abbreviation": abbrev, "lang": lang,
            "direction": "LTR", "books": out}


def main():
    fmt, epub, out, lang, name, abbrev = sys.argv[1:7]
    verses = {}
    with zipfile.ZipFile(epub) as zf:
        {"esv": parse_esv, "fr": parse_fr, "no": parse_no}[fmt](zf, verses)
    data = build(verses, lang, name, abbrev)
    nb = len(data["books"])
    nv = sum(len(c["verses"]) for b in data["books"] for c in b["chapters"])
    json.dump(data, open(out, "w", encoding="utf-8"), ensure_ascii=False)
    print(f"{abbrev}: {nb} books, {nv} verses -> {out}")


if __name__ == "__main__":
    main()
