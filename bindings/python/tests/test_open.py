"""Sources, opening and `PdfError`."""

import pathlib
import sys
import unittest

import starlightpdf

HEADER = pathlib.Path(__file__).resolve().parents[3] / "tests" / "corpus" / "header"
VALID = HEADER / "version-1.7.pdf"
INVALID = HEADER / "not-pdf.pdf"


class Open(unittest.TestCase):
    """Sources, open and `PdfError`."""

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

    def test_failure_raises_pdf_error_with_name_and_number(self) -> None:
        """An invalid header raises `PdfError` with the code's name and number."""
        source = starlightpdf.Source.from_bytes(bytes=INVALID.read_bytes())
        with self.assertRaises(starlightpdf.PdfError) as caught:
            starlightpdf.Document.open(source)
        error = caught.exception
        self.assertEqual((error.code, error.number), ("InvalidHeader", 8))
