"""Sources and opening."""

import pathlib
import sys
import unittest

import starlightpdf

HEADER = pathlib.Path(__file__).resolve().parents[3] / "tests" / "corpus" / "header"
VALID = HEADER / "version-1.7.pdf"


class Open(unittest.TestCase):
    """Sources and opening."""

    def test_source_over_str_path_opens(self) -> None:
        """A source over a `str` path opens."""
        source = starlightpdf.Source.from_file(file=str(VALID))
        self.assertIsInstance(starlightpdf.Document.open(source), starlightpdf.Document)

    def test_source_over_bytes_shares_the_object(self) -> None:
        """The source holds the caller's `bytes` object, not a copy."""
        data = VALID.read_bytes()
        before = sys.getrefcount(data)
        _source = starlightpdf.Source.from_bytes(bytes=data)
        self.assertGreater(sys.getrefcount(data), before)
