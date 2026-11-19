"""Runs the conformance suite in tests/conformance/*.tsv through the binding."""

import functools
import pathlib
import re
import unittest
from collections.abc import Callable

import starlightpdf

ROOT = pathlib.Path(__file__).resolve().parents[3]
SUITE_DIR = ROOT / "tests" / "conformance"


def source(kind: str, path: str) -> starlightpdf.Source:
    """Make the file at `path` a `bytes` or `file` source."""
    if kind == "bytes":
        return starlightpdf.Source.from_bytes(bytes=(ROOT / path).read_bytes())
    return starlightpdf.Source.from_file(file=ROOT / path)


def header_version(path: str) -> str:
    """Header version of the file at `path`."""
    major, minor = starlightpdf.header_version((ROOT / path).read_bytes())
    return f"{major}.{minor}"


def open_document(kind: str, path: str) -> str:
    """Open the file at `path`: `ok`."""
    starlightpdf.Document.open(source(kind, path))
    return "ok"


def page_count(kind: str, path: str) -> str:
    """Page count of the file at `path`."""
    return str(starlightpdf.Document.open(source(kind, path)).page_count)


OPERATIONS: dict[str, Callable[..., str]] = {
    "header_version": header_version,
    "open_bytes": functools.partial(open_document, "bytes"),
    "open_file": functools.partial(open_document, "file"),
    "page_count_bytes": functools.partial(page_count, "bytes"),
    "page_count_file": functools.partial(page_count, "file"),
}


class SuiteError(Exception):
    """A case failed."""


def check(operation: str, args: list[str], expected: str) -> str | None:
    """Return why `operation` on `args` does not give `expected`, or None."""
    if operation not in OPERATIONS:
        return f"unknown operation {operation!r}"
    try:
        actual = OPERATIONS[operation](*args)
    except starlightpdf.PdfError as error:
        actual = error.code
    except (OSError, TypeError, ValueError) as error:
        return f"{operation}: {error}"
    if actual != expected:
        return f"{operation}: expected {expected!r}, got {actual!r}"
    return None


def run(suites: list[tuple[str, str]]) -> None:
    """Run every case; fail naming each bad case and disagreeing pair by `file:line`."""
    errors = []
    first: dict[tuple[str, ...], tuple[str, str]] = {}
    found = False
    for name, text in suites:
        lines = text.split("\n")
        if lines[-1] == "":
            lines.pop()
        for number, raw in enumerate(lines, 1):
            line = raw.removesuffix("\r")
            if line.startswith("#"):
                continue
            found = True
            position = f"{name}:{number}"
            *head, expected = line.split("\t")
            if not head:
                errors.append(f"{position}: malformed line {line!r}")
                continue
            operation, *args = head
            family = re.sub(r"_(?:bytes|file)\Z", "", operation)
            earlier, wanted = first.setdefault((family, *args), (position, expected))
            if wanted != expected:
                errors.append(
                    f"{position}: expects {expected!r}, {earlier} expects {wanted!r}"
                )
            error = check(operation, args, expected)
            if error:
                errors.append(f"{position}: {error}")
    if not found:
        errors.append("no cases")
    if errors:
        raise SuiteError("\n".join(errors))


def failures(suites: list[tuple[str, str]]) -> set[str]:
    """Return the `file:line` positions a failing run names."""
    try:
        run(suites)
    except SuiteError as error:
        return set(re.findall(r"[\w.-]+\.tsv:\d+", str(error)))
    msg = "the run passed"
    raise AssertionError(msg)


class Conformance(unittest.TestCase):
    """The suite and the runner."""

    def test_suite_passes(self) -> None:
        """Every case passes."""
        run(
            [
                (path.name, path.read_bytes().decode())
                for path in sorted(SUITE_DIR.glob("*.tsv"))
            ]
        )

    def test_unknown_operation_fails(self) -> None:
        """An unknown operation fails."""
        with self.assertRaises(SuiteError):
            run([("s.tsv", "nope\tx\ty\n")])

    def test_suite_without_cases_fails(self) -> None:
        """A suite with no cases fails."""
        with self.assertRaises(SuiteError):
            run([("s.tsv", "# operation\targuments\texpected\n")])

    def test_names_every_failing_case(self) -> None:
        """A run names each failing case by file and line."""
        a = (
            "# c\n"
            "header_version\ttests/corpus/header/version-1.7.pdf\t1.7\n"
            "header_version\ttests/corpus/header/version-2.9.pdf\t1.7\n"
        )
        b = "open_bytes\ttests/corpus/header/empty.pdf\tok\n"
        self.assertEqual(failures([("a.tsv", a), ("b.tsv", b)]), {"a.tsv:3", "b.tsv:1"})

    def test_rejects_expectations_that_differ_by_source(self) -> None:
        """Bytes and file expectations for one corpus file must agree."""
        opened = (
            "open_bytes\ttests/corpus/header/version-1.7.pdf\tok\n"
            "open_file\ttests/corpus/header/version-1.7.pdf\tInvalidHeader\n"
        )
        counted = (
            "page_count_bytes\ttests/corpus/page-count/one-page.pdf\t1\n"
            "page_count_file\ttests/corpus/page-count/one-page.pdf\t2\n"
        )
        self.assertEqual(
            failures([("open.tsv", opened), ("page_count.tsv", counted)]),
            {"open.tsv:1", "open.tsv:2", "page_count.tsv:1", "page_count.tsv:2"},
        )

    def test_crlf_suite_matches_lf(self) -> None:
        """A CR LF suite passes as its LF form does."""
        text = "# c\r\nheader_version\ttests/corpus/header/version-1.7.pdf\t1.7\r\n"
        run([("s.tsv", text)])
