#!/usr/bin/env python3
"""Build a Strong's correspondence for a translation by statistical word
alignment — for translations with no existing Strong's-tagged edition.

Parallel corpus (per verse): the translation's tokens  ↔  the multiset of
Strong's numbers in that verse, taken from the already-tagged WLC (Hebrew OT)
and SBLGNT (Greek NT) in bible.db. We train IBM Model 1 (EM) to estimate
P(token | Strong's), then for each verse assign every word the best-fitting
Strong's *present in that verse* (NULL-aware, thresholded).

The Strong's source (WLC, SBLGNT) is open; the output reuses the translation's
own text, so its licensing follows that translation (the public-domain Louis
Segond may be committed; a copyrighted ESV correspondence stays local). Output
mimics the bolls format (text with inline <S>n</S>) so the Rust importer reuses
its tagged-text path.

Usage: build_strongs.py <bible.db> <TRANSLATION_CODE> <out.json> [--source <text.json>]

The translation's text is read from bible.db, or — with --source — from a
getbible-style JSON file (what tools/epub2json.py emits). The latter breaks a
circle for the EPUB-derived texts: they only enter the database *through* the
output of this script, so regenerating them from a fixed source needs the text
taken from the file rather than from the database.
"""
import json
import re
import sqlite3
import sys
from collections import defaultdict

EM_ITERS = 6
PRUNE = 1e-3      # drop tiny probabilities between iterations to bound memory
ASSIGN_MIN = 0.12  # minimum P(token|strong) to attach a Strong's number
NULL = "<null>"
PUNCT = " \t\n.,;:!?«»\"'’“”()[]—–-…·"


def norm(tok):
    return tok.strip(PUNCT).lower()


def load_source_json(path):
    """(book, chapter, verse) -> text, from epub2json's getbible-style layout."""
    data = json.load(open(path, encoding="utf-8"))
    out = {}
    for b in data["books"]:
        for ch in b["chapters"]:
            for v in ch["verses"]:
                out[(b["nr"], v["chapter"], v["verse"])] = v["text"]
    return out


def main():
    args = sys.argv[1:]
    source = None
    if "--source" in args:
        i = args.index("--source")
        source = args[i + 1]
        del args[i:i + 2]
    db, code, out = args[0], args[1], args[2]
    con = sqlite3.connect(db)

    tid = {c: i for (i, c) in con.execute(
        "SELECT id, code FROM translations").fetchall()}
    wlc, sblgnt = tid["WLC"], tid["SBLGNT"]

    # Strong's present per verse (bare number) from the tagged originals.
    strongs_by_verse = defaultdict(set)
    for tr in (wlc, sblgnt):
        for b, c, v, s in con.execute(
            "SELECT book, chapter, verse, strongs FROM words "
            "WHERE translation_id=? AND strongs IS NOT NULL AND strongs<>''", (tr,)):
            strongs_by_verse[(b, c, v)].add(re.sub(r"\D", "", s))  # drop H/G prefix

    # Target translation verses (plain text) and their tokens.
    tgt_verses = {}  # (b,c,v) -> original-token list
    if source:
        for ref, text in load_source_json(source).items():
            tgt_verses[ref] = text.split()
        print(f"target text: {len(tgt_verses)} verses from {source}")
    else:
        for b, c, v, text in con.execute(
            "SELECT book, chapter, verse, text FROM verses WHERE translation_id=?",
            (tid[code],)):
            tgt_verses[(b, c, v)] = text.split()

    # Parallel pairs (only verses present in both, deduped per verse).
    corpus = []
    all_strongs = set([NULL])
    for ref, toks in tgt_verses.items():
        st = strongs_by_verse.get(ref)
        if not st:
            continue
        fr = {norm(t) for t in toks if norm(t)}
        if not fr:
            continue
        srcs = set(st) | {NULL}
        all_strongs |= set(st)
        corpus.append((fr, srcs))
    print(f"corpus: {len(corpus)} verses, {len(all_strongs)} Strong's types")

    # IBM Model 1 EM.
    init = 1.0 / len(all_strongs)
    t = {}

    def tget(f, s):
        return t.get((f, s), init)

    for it in range(EM_ITERS):
        count = defaultdict(float)
        total = defaultdict(float)
        for fr, srcs in corpus:
            for f in fr:
                denom = 0.0
                for s in srcs:
                    denom += tget(f, s)
                if denom <= 0:
                    continue
                for s in srcs:
                    c = tget(f, s) / denom
                    count[(f, s)] += c
                    total[s] += c
        t = {fs: cnt / total[fs[1]] for fs, cnt in count.items()
             if cnt / total[fs[1]] >= PRUNE}
        print(f"  iter {it + 1}: {len(t)} pairs")

    # Decode: attach the best in-verse Strong's to each French token.
    rows = []
    assigned = total_tokens = 0
    for ref, toks in sorted(tgt_verses.items()):
        b, c, v = ref
        st = strongs_by_verse.get(ref)
        out_toks = []
        for tok in toks:
            total_tokens += 1
            key = norm(tok)
            best, best_p = None, ASSIGN_MIN
            if key and st:
                for s in st:
                    p = tget(key, s)
                    if p > best_p:
                        best, best_p = s, p
            if best:
                out_toks.append(f"{tok}<S>{best}</S>")
                assigned += 1
            else:
                out_toks.append(tok)
        rows.append({"book": b, "chapter": c, "verse": v, "text": " ".join(out_toks)})

    json.dump(rows, open(out, "w", encoding="utf-8"), ensure_ascii=False)
    print(f"assigned Strong's to {assigned}/{total_tokens} tokens "
          f"({100 * assigned / total_tokens:.0f}%) -> {out}")


if __name__ == "__main__":
    main()
