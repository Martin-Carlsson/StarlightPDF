"""Details of `PdfError`."""

import pathlib
import unittest

import starlightpdf

HEADER = pathlib.Path(__file__).resolve().parents[3] / "tests" / "corpus" / "header"
INVALID = HEADER / "not-pdf.pdf"
MISSING = HEADER / "missing.pdf"


def open_invalid() -> starlightpdf.PdfError:
    """Return the error from opening a file that is not a PDF."""
    source = starlightpdf.Source.from_bytes(bytes=INVALID.read_bytes())
    try:
        starlightpdf.Document.open(source)
    except starlightpdf.PdfError as error:
        return error
    raise AssertionError


class Errors(unittest.TestCase):
    """Details of `PdfError`."""

    def test_details_beside_code_and_number(self) -> None:
        """Code, number and every detail are present; an unknown one is `None`."""
        error = open_invalid()
        self.assertEqual((error.code, error.number), ("InvalidHeader", 8))
        self.assertTrue(error.message)
        details = (error.offset, error.object_number, error.io_cause)
        self.assertEqual(details, (None, None, None))

    def test_missing_file_io_cause_is_file_not_found(self) -> None:
        """A missing file's `io_cause` is a `FileNotFoundError`."""
        with self.assertRaises(starlightpdf.PdfError) as caught:
            starlightpdf.Document.open(starlightpdf.Source.from_file(file=str(MISSING)))
        self.assertIsInstance(caught.exception.io_cause, FileNotFoundError)

    def test_operation_spelled_as_in_python(self) -> None:
        """The operation is named as Python spells it."""
        error = open_invalid()
        self.assertEqual(error.operation, "Document.open")
        self.assertEqual(str(error), f"Document.open: {error.message}")
