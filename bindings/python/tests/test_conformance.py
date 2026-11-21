"""Runs the conformance suite in tests/conformance/*.tsv through the binding."""

import decimal
import functools
import pathlib
import re
import unittest
from collections.abc import Callable

import starlightpdf

ROOT = pathlib.Path(__file__).resolve().parents[3]
SUITE_DIR = ROOT / "tests" / "conformance"
LIMITS = ("decoded_limit", "object_limit", "warning_limit")
OPEN = {"strict", "password", *LIMITS}
FLAGS = {"strict", "decode"}
BOXES = {"Art", "Bleed", "Crop", "Media", "Trim"}


def source(kind: str, path: str) -> starlightpdf.Source:
    """Make the file at `path` a `bytes` or `file` source."""
    if kind == "bytes":
        return starlightpdf.Source.from_bytes(bytes=(ROOT / path).read_bytes())
    return starlightpdf.Source.from_file(file=ROOT / path)


def header_version(path: str) -> str:
    """Header version of the file at `path`."""
    major, minor = starlightpdf.header_version((ROOT / path).read_bytes())
    return f"{major}.{minor}"


def object_id(given: dict[str, str]) -> starlightpdf.ObjectId:
    """Name the object `id=` gives."""
    number, generation = given["id"].split(" ")
    return starlightpdf.ObjectId(number=int(number), generation=int(generation))


def page(document: starlightpdf.Document, given: dict[str, str]) -> starlightpdf.Page:
    """Get the page `index=` gives."""
    return document.page(index=int(given["index"]))


def stream_data(
    document: starlightpdf.Document, given: dict[str, str], *, decode: bool
) -> bytes:
    """Read the stream `id=` gives into an in-memory sink."""
    sink = starlightpdf.Sink.in_memory()
    document.stream_data(sink, object_id(given), decode=decode)
    return sink.take_bytes()


def number(value: float) -> str:
    """`value` as the shortest decimal that round-trips, without exponent or `.0`."""
    return format(decimal.Decimal(repr(value)).normalize(), "f")


def read_page(document: starlightpdf.Document, given: dict[str, str]) -> str:
    """Read the page: `ok`."""
    page(document, given)
    return "ok"


def boundary(document: starlightpdf.Document, given: dict[str, str]) -> str:
    """Give the page's `box=` boundary as `x y width height`."""
    if given["box"] not in BOXES:
        msg = f"bad box {given['box']!r}"
        raise ValueError(msg)
    rect = page(document, given).boundary(kind=given["box"])
    return " ".join(
        number(value) for value in (rect.x, rect.y, rect.width, rect.height)
    )


def encryption(document: starlightpdf.Document, _given: dict[str, str]) -> str:
    """Give the cipher, whether certificates are used, and what is denied."""
    found = document.encryption
    certificate = "true" if found.certificate else "false"
    return f"{found.cipher} {certificate} {','.join(found.restrictions) or 'none'}"


def touch(document: starlightpdf.Document, given: dict[str, str]) -> None:
    """Count the pages and, with `id=`, read that object and any stored stream data."""
    _ = document.page_count
    if "id" in given:
        document.object(object_id(given))
        try:
            stream_data(document, given, decode=False)
        except starlightpdf.PdfError as error:
            if error.code != "InvalidArgument":
                raise


def warnings(document: starlightpdf.Document, given: dict[str, str]) -> str:
    """List each warning's clause after `touch`."""
    touch(document, given)
    return ";".join(deviation.clause for deviation in document.warnings) or "none"


def dropped_warnings(document: starlightpdf.Document, given: dict[str, str]) -> str:
    """Count the warnings dropped after `touch`."""
    touch(document, given)
    return str(document.dropped_warnings)


# Per operation: the keys it takes, those it needs, and what it reads after open.
READS: dict[
    str,
    tuple[set[str], set[str], Callable[[starlightpdf.Document, dict[str, str]], str]],
] = {
    "open": (set(), set(), lambda _document, _given: "ok"),
    "page_count": (set(), set(), lambda document, _given: str(document.page_count)),
    "catalog": (
        set(),
        set(),
        lambda document, _given: (
            f"{document.catalog.number} {document.catalog.generation}"
        ),
    ),
    "object": (
        {"id"},
        {"id"},
        lambda document, given: document.object(object_id(given)),
    ),
    "stream_data": (
        {"id", "decode"},
        {"id"},
        lambda document, given: (
            stream_data(document, given, decode="decode" in given).hex() or "empty"
        ),
    ),
    "page": ({"index"}, {"index"}, read_page),
    "boundary": ({"index", "box"}, {"index", "box"}, boundary),
    "rotation": (
        {"index"},
        {"index"},
        lambda document, given: str(page(document, given).rotation),
    ),
    "encryption": (set(), set(), encryption),
    "warnings": ({"id"}, set(), warnings),
    "dropped_warnings": ({"id"}, set(), dropped_warnings),
}


def perform(family: str, kind: str, args: list[str]) -> str:
    """Open the path first in `args` with the options after it; read as `family`."""
    path, *options = args
    takes, needs, read = READS[family]
    given: dict[str, str] = {}
    for arg in options:
        key, equals, value = arg.partition("=")
        if key not in OPEN | takes or (key in FLAGS) == bool(equals) or key in given:
            msg = f"bad argument {arg!r}"
            raise ValueError(msg)
        given[key] = value
    if not needs <= given.keys():
        msg = f"needs {', '.join(sorted(needs - given.keys()))}"
        raise ValueError(msg)
    made = source(kind, path)
    limits = {key: int(given[key]) for key in LIMITS if key in given}
    policy = starlightpdf.ResourcePolicy(**limits) if limits else None
    document = starlightpdf.Document.open(
        made, policy, strict="strict" in given, password=given.get("password", "")
    )
    return read(document, given)


OPERATIONS: dict[str, Callable[[list[str]], str]] = {
    "header_version": lambda args: header_version(*args),
    **{
        f"{family}_{kind}": functools.partial(perform, family, kind)
        for family in READS
        for kind in ("bytes", "file")
    },
}


class SuiteError(Exception):
    """A case failed."""


def check(operation: str, args: list[str], expected: str) -> str | None:
    """Return why `operation` on `args` does not give `expected`, or None."""
    if operation not in OPERATIONS:
        return f"unknown operation {operation!r}"
    try:
        actual = OPERATIONS[operation](args)
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
        words = (word.rstrip(":,") for word in str(error).split())
        return {word for word in words if re.fullmatch(r"[\w.-]+\.tsv:\d+", word)}
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

    def test_unknown_argument_fails(self) -> None:
        """An unknown argument fails."""
        with self.assertRaises(SuiteError):
            run(
                [
                    (
                        "s.tsv",
                        "open_bytes\ttests/corpus/header/version-1.7.pdf\tnope\tok\n",
                    )
                ]
            )

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
        self.assertSetEqual(
            failures([("a.tsv", a), ("b.tsv", b)]), {"a.tsv:3", "b.tsv:1"}
        )

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
        self.assertSetEqual(
            failures([("open.tsv", opened), ("page_count.tsv", counted)]),
            {"open.tsv:1", "open.tsv:2", "page_count.tsv:1", "page_count.tsv:2"},
        )

    def test_crlf_suite_matches_lf(self) -> None:
        """A CR LF suite passes as its LF form does."""
        text = "# c\r\nheader_version\ttests/corpus/header/version-1.7.pdf\t1.7\r\n"
        run([("s.tsv", text)])
