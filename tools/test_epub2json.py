"""Tests for the EPUB converter's text extraction.

Run with:  .venv/bin/python -m unittest tools/test_epub2json.py
"""
import os
import sys
import unittest

sys.path.insert(0, os.path.dirname(__file__))

from bs4 import BeautifulSoup  # noqa: E402

import epub2json  # noqa: E402


def extract(html):
    """Verse texts from a BFC-style chapter fragment, cleaned as the tool does."""
    soup = BeautifulSoup(f"<html><body>{html}</body></html>", "lxml")
    verses = {}
    epub2json.walk_fr(soup.body, verses)
    return {k: epub2json.clean(v) for k, v in verses.items()}


class BlockBoundaries(unittest.TestCase):
    def test_poetry_lines_are_separated(self):
        # The BFC marks each poetic line as its own <div> with no whitespace
        # between them; the text on either side used to fuse ("Dieu,la").
        html = (
            '<div class="lg">'
            '<div class="osis_l"><a class="verse" id="vPs.19.2"/>Le ciel proclame la gloire de Dieu,</div>'
            '<div class="osis_l">la voûte étoilée révèle ce qu’il a fait.</div>'
            '<div class="osis_l"><a class="verse" id="vPs.19.3"/>Chaque jour en parle au jour suivant,</div>'
            '<div class="osis_l">et chaque nuit l’annonce à celle qui la suit.</div>'
            "</div>"
        )
        got = extract(html)
        self.assertEqual(got[(19, 19, 2)], "Le ciel proclame la gloire de Dieu, la voûte étoilée révèle ce qu’il a fait.")
        self.assertEqual(got[(19, 19, 3)], "Chaque jour en parle au jour suivant, et chaque nuit l’annonce à celle qui la suit.")

    def test_inline_markup_does_not_split_words(self):
        # A gloss link inside a word run is inline: no space must be inserted.
        html = '<p><a class="verse" id="vGen.1.1"/>Au commen<a class="w-gloss" href="#x">cement</a>, Dieu créa.</p>'
        self.assertEqual(extract(html)[(1, 1, 1)], "Au commencement, Dieu créa.")

    def test_br_and_paragraphs_separate_too(self):
        html = '<p><a class="verse" id="vGen.1.2"/>La terre était vide,<br/>il y avait des ténèbres.</p><p>Et l’esprit planait.</p>'
        self.assertEqual(extract(html)[(1, 1, 2)], "La terre était vide, il y avait des ténèbres. Et l’esprit planait.")

    def test_deuterocanonical_and_headings_are_dropped(self):
        html = (
            '<h3>Titre</h3>'
            '<p><a class="verse" id="vTob.1.1"/>Tobit ignoré.</p>'
            '<p><a class="verse" id="vGen.1.3"/>Dieu dit.</p>'
        )
        got = extract(html)
        self.assertEqual(list(got), [(1, 1, 3)])
        self.assertEqual(got[(1, 1, 3)], "Dieu dit.")


if __name__ == "__main__":
    unittest.main()
