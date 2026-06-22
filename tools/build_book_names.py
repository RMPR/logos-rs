#!/usr/bin/env python3
"""Build data/book_names.json — localized book names per language, so the
sidebar shows book names in each translation's own language.

Names are pulled from authoritative getbible translations (book names are
factual, not copyrightable). Languages map to the tagged translations:
  fr  (Louis Segond, BFC)      <- ls1910
  hbo (Westminster Leningrad)  <- codex
  grc (SBL Greek NT)           <- westcotthort
  nb  (Bibelen 2011)           <- bibelselskap
English (KJV/ULT/ESV) falls back to the canonical names in canon.rs; Latin
(Vulgate) and the LXX keep the names from their own getbible source.

Usage: build_book_names.py <out.json>
"""
import json
import sys
import urllib.request

SOURCES = {"fr": "ls1910", "hbo": "codex", "grc": "westcotthort", "nb": "bibelselskap"}


def fetch(slug):
    url = f"https://api.getbible.net/v2/{slug}.json"
    req = urllib.request.Request(url, headers={"User-Agent": "logos-rs/1.0"})
    with urllib.request.urlopen(req, timeout=60) as r:
        return json.loads(r.read().decode("utf-8-sig"))


def main():
    out = sys.argv[1]
    names = {}
    for lang, slug in SOURCES.items():
        d = fetch(slug)
        m = {}
        for b in d["books"]:
            nr = b.get("nr")
            if isinstance(nr, int) and 1 <= nr <= 66 and b.get("name"):
                m[str(nr)] = b["name"].strip()
        names[lang] = m
        print(f"{lang} ({slug}): {len(m)} names")
    json.dump(names, open(out, "w", encoding="utf-8"), ensure_ascii=False, indent=0)
    print(f"-> {out}")


if __name__ == "__main__":
    main()
