"""Runs the conformance suite in tests/conformance/*.tsv through the binding."""

import pathlib
import unittest

import starlightpdf

ROOT = pathlib.Path(__file__).resolve().parents[3]
SUITE_DIR = ROOT / "tests" / "conformance"


def header_version(path: str) -> str:
    """Render the header version of the file at `path`, or `error`."""
    try:
        major, minor = starlightpdf.header_version((ROOT / path).read_bytes())
    except starlightpdf.PdfError:
        return "error"
    return f"{major}.{minor}"


def open_bytes(path: str) -> str:
    """Open the file at `path` from its bytes: `ok` or the error code's name."""
    try:
        source = starlightpdf.Source.from_bytes(bytes=(ROOT / path).read_bytes())
        starlightpdf.Document.open(source)
    except starlightpdf.PdfError as error:
        return error.code
    return "ok"


def open_file(path: str) -> str:
    """Open the file at `path` from the file: `ok` or the error code's name."""
    try:
        starlightpdf.Document.open(starlightpdf.Source.from_file(file=ROOT / path))
    except starlightpdf.PdfError as error:
        return error.code
    return "ok"


OPERATIONS = {
    "header_version": header_version,
    "open_bytes": open_bytes,
    "open_file": open_file,
}


class SuiteError(Exception):
    """A case failed."""


def run_case(line: str) -> None:
    """Run one case: operation, arguments and expected result, tab-separated."""
    *head, expected = line.split("\t")
    if not head:
        msg = f"malformed line {line!r}"
        raise SuiteError(msg)
    operation, *args = head
    if operation not in OPERATIONS:
        msg = f"unknown operation {operation!r}"
        raise SuiteError(msg)
    actual = OPERATIONS[operation](*args)
    if actual != expected:
        msg = f"{operation}: expected {expected!r}, got {actual!r}"
        raise SuiteError(msg)


def run(suite: str) -> None:
    """Run every case of `suite`, skipping `#` lines."""
    cases = [
        line
        for line in suite.removesuffix("\n").split("\n")
        if not line.startswith("#")
    ]
    if not cases:
        msg = "no cases"
        raise SuiteError(msg)
    for case in cases:
        run_case(case)


class Conformance(unittest.TestCase):
    """The suite and the runner."""

    def test_suite_passes(self) -> None:
        """Every case passes."""
        texts = [
            path.read_text(encoding="utf-8") for path in sorted(SUITE_DIR.glob("*.tsv"))
        ]
        run("".join(text if text.endswith("\n") else text + "\n" for text in texts))

    def test_unknown_operation_fails(self) -> None:
        """An unknown operation fails."""
        with self.assertRaises(SuiteError):
            run("nope\tx\ty\n")

    def test_suite_without_cases_fails(self) -> None:
        """A suite with no cases fails."""
        with self.assertRaises(SuiteError):
            run("# operation\targuments\texpected\n")
