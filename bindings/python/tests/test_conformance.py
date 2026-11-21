"""Runs the conformance suite in tests/conformance/*.tsv through the binding."""

import decimal
import functools
import hashlib
import itertools
import pathlib
import re
import unittest
from collections.abc import Callable
from typing import NamedTuple, TypeAlias

import starlightpdf

ROOT = pathlib.Path(__file__).resolve().parents[3]
SUITE_DIR = ROOT / "tests" / "conformance"
LIMITS = ("decoded_limit", "object_limit", "warning_limit")
OPEN = {"strict", "password", *LIMITS}
FLAGS = {
    "allow_loss",
    "break_signatures",
    "cancel",
    "decode",
    "ignore_orientation",
    "saved",
    "strict",
}
BOXES = {"Art", "Bleed", "Crop", "Media", "Trim"}
SAVE = {
    "allow_loss",
    "break_signatures",
    "cancel",
    "cipher",
    "color_space",
    "image_quality",
    "image_resolution",
    "mode",
    "optimizations",
    "owner_password",
    "recipients",
    "restrictions",
    "standards",
    "user_password",
    "version",
}
SPLIT = {"allow_loss", "by", "count", "starts"}
DRAW = {
    "format",
    "height",
    "ignore_orientation",
    "mask",
    "opacity",
    "page_height",
    "page_width",
    "read",
    "width",
    "x",
    "y",
}
WHITE = "\0\t\n\f\r "
DELIMITERS = "()<>[]{}/%"


class Name(NamedTuple):
    """A name object without its slash."""

    value: str


class Ref(NamedTuple):
    """An indirect reference."""

    number: int
    generation: int


# An object parsed from its syntax (ISO 32000-2:2020, 7.3); other tokens stay text.
Obj: TypeAlias = "dict[str, Obj] | list[Obj] | Name | Ref | str"


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


def stream_bytes(
    document: starlightpdf.Document, found: starlightpdf.ObjectId, *, decode: bool
) -> bytes:
    """Read the stream `found` into an in-memory sink."""
    sink = starlightpdf.Sink.in_memory()
    document.stream_data(sink, found, decode=decode)
    return sink.take_bytes()


def stream_data(
    document: starlightpdf.Document, given: dict[str, str], *, decode: bool
) -> bytes:
    """Read the stream `id=` gives into an in-memory sink."""
    return stream_bytes(document, object_id(given), decode=decode)


def number(value: float) -> str:
    """`value` as the shortest decimal that round-trips, without exponent or `.0`."""
    return format(decimal.Decimal(repr(value)).normalize(), "f")


def rect(found: starlightpdf.Rect) -> str:
    """`found` as `x y width height`."""
    return " ".join(
        number(value) for value in (found.x, found.y, found.width, found.height)
    )


def read_page(document: starlightpdf.Document, given: dict[str, str]) -> str:
    """Read the page: `ok`."""
    page(document, given)
    return "ok"


def boundary(document: starlightpdf.Document, given: dict[str, str]) -> str:
    """Give the page's `box=` boundary as `x y width height`."""
    if given["box"] not in BOXES:
        msg = f"bad box {given['box']!r}"
        raise ValueError(msg)
    return rect(page(document, given).boundary(kind=given["box"]))


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


# Each `key=` of `info`: its entry.
INFO: dict[str, Callable[[starlightpdf.InfoDictionary], str]] = {
    "author": lambda found: found.author,
    "created": lambda found: found.created,
    "creator": lambda found: found.creator,
    "keywords": lambda found: found.keywords,
    "modified": lambda found: found.modified,
    "producer": lambda found: found.producer,
    "subject": lambda found: found.subject,
    "title": lambda found: found.title,
}


def info(document: starlightpdf.Document, given: dict[str, str]) -> str:
    """Give the `key=` entry of the document information dictionary, or `none`."""
    if given["key"] not in INFO:
        msg = f"bad key {given['key']!r}"
        raise ValueError(msg)
    return INFO[given["key"]](document.info_dictionary) or "none"


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
    "info": ({"key", *(f"set_{key}" for key in INFO)}, {"key"}, info),
}


def parse(options: list[str], takes: set[str], needs: set[str]) -> dict[str, str]:
    """Return the keys and values of `options`: keys of `takes`, `needs` among them."""
    given: dict[str, str] = {}
    for arg in options:
        key, equals, value = arg.partition("=")
        if key not in takes or (key in FLAGS) == bool(equals) or key in given:
            msg = f"bad argument {arg!r}"
            raise ValueError(msg)
        given[key] = value
    if not needs <= given.keys():
        msg = f"needs {', '.join(sorted(needs - given.keys()))}"
        raise ValueError(msg)
    return given


def combine(given: dict[str, str]) -> None:
    """Fail unless save options come with `saved`, split ones with `part=`."""
    saved = "saved" in given
    part = "part" in given

    def lone(keys: set[str]) -> bool:
        return any(key in given for key in keys - {"allow_loss"})

    if (
        (saved and part)
        or (lone(SAVE) and not saved)
        or (lone(SPLIT) and not part)
        or ("allow_loss" in given and not saved and not part)
    ):
        msg = "save options need saved, split options part=, and not both"
        raise ValueError(msg)


def names(text: str | None) -> list[str]:
    """Return the comma-separated names of `text`."""
    return text.split(",") if text else []


def optional(text: str | None) -> float | None:
    """`text` as a number, or None."""
    return None if text is None else float(text)


def cancel(_done: int, _total: int) -> bool:
    """Cancel at the first progress."""
    return False


def save(
    document: starlightpdf.Document, sink: starlightpdf.Sink, given: dict[str, str]
) -> starlightpdf.SaveReport:
    """Save `document` to `sink` with the save options `given`."""
    version = given.get("version")
    major, _, minor = (version or "").partition(".")
    quality = given.get("image_quality")
    resolution = given.get("image_resolution")
    return document.save(
        sink,
        cancel if "cancel" in given else None,
        allow_loss="allow_loss" in given,
        break_signatures="break_signatures" in given,
        cipher=given.get("cipher"),
        color_space=given.get("color_space"),
        image_quality=None if quality is None else int(quality),
        image_resolution=None if resolution is None else int(resolution),
        mode=given.get("mode", "Rewrite"),
        optimizations=names(given.get("optimizations")),
        owner_password=given.get("owner_password", ""),
        recipients=bytes.fromhex(given.get("recipients", "")),
        restrictions=names(given.get("restrictions")),
        standards=names(given.get("standards")),
        user_password=given.get("user_password", ""),
        version=None if version is None else (int(major), int(minor)),
    )


def twice(document: starlightpdf.Document, given: dict[str, str]) -> bytes:
    """Save `document` twice to memory; the bytes, failing unless both are the same."""
    saved = []
    for _ in range(2):
        sink = starlightpdf.Sink.in_memory()
        save(document, sink, given)
        saved.append(sink.take_bytes())
    if saved[0] != saved[1]:
        msg = "two saves give different bytes"
        raise ValueError(msg)
    return saved[0]


def open_document(
    made: starlightpdf.Source, given: dict[str, str]
) -> starlightpdf.Document:
    """Open `made` with the open options `given`."""
    limits = {key: int(given[key]) for key in LIMITS if key in given}
    policy = starlightpdf.ResourcePolicy(**limits) if limits else None
    return starlightpdf.Document.open(
        made, policy, strict="strict" in given, password=given.get("password", "")
    )


def reopen(data: bytes, given: dict[str, str]) -> starlightpdf.Document:
    """Open `data` with the open options `given`."""
    return open_document(starlightpdf.Source.from_bytes(bytes=data), given)


def split(
    document: starlightpdf.Document, given: dict[str, str]
) -> list[starlightpdf.Document]:
    """Return the parts of `document` split as `given` says."""
    starts = given.get("starts")
    return list(
        document.split(
            by=given.get("by", "Pages"),
            count=int(given.get("count", "0")),
            starts=[int(start) for start in starts.split(",")] if starts else [],
            allow_loss="allow_loss" in given,
        )
    )


def perform(family: str, kind: str, args: list[str]) -> str:
    """Open the path first in `args` with the options after it; read as `family`."""
    path, *options = args
    if family == "save":
        given = parse(options, OPEN | SAVE, set())
    elif family == "split":
        given = parse(options, OPEN | SPLIT, set())
    else:
        takes, needs, _ = READS[family]
        given = parse(options, OPEN | takes | {"saved", "part"} | SAVE | SPLIT, needs)
        combine(given)
    document = open_document(source(kind, path), given)
    if any(key.startswith("set_") for key in given):
        document.set_info_dictionary(
            author=given.get("set_author"),
            created=given.get("set_created"),
            creator=given.get("set_creator"),
            keywords=given.get("set_keywords"),
            modified=given.get("set_modified"),
            producer=given.get("set_producer"),
            subject=given.get("set_subject"),
            title=given.get("set_title"),
        )
    if family == "save":
        report = save(document, starlightpdf.Sink.in_memory(), given)
        return ",".join(report.broken_signatures) or "ok"
    if family == "split":
        counts = [str(part.page_count) for part in split(document, given)]
        return " ".join(counts) or "none"
    if "saved" in given:
        document = reopen(twice(document, given), given)
    elif "part" in given:
        index = int(given["part"])
        part = next(itertools.islice(split(document, given), index, None), None)
        if part is None:
            msg = f"no part {index}"
            raise ValueError(msg)
        document = reopen(twice(part, {}), given)
    return READS[family][2](document, given)


def inserted(args: list[str]) -> str:
    """Create a document, insert each `page=[WxH][@I]`, save, reopen: each MediaBox."""
    path, *options = args
    if path != "-":
        msg = "takes the path -"
        raise ValueError(msg)
    document = starlightpdf.Document.create()
    for arg in options:
        key, equals, value = arg.partition("=")
        size, at, index = value.partition("@")
        width, by, height = size.partition("x")
        if key != "page" or not equals or bool(size) != bool(by):
            msg = f"bad argument {arg!r}"
            raise ValueError(msg)
        document.insert_blank_page(
            width=float(width) if size else None,
            height=float(height) if size else None,
            index=int(index) if at else None,
        )
    saved = reopen(twice(document, {}), {})
    return ";".join(
        rect(saved.page(index=index).boundary(kind="Media"))
        for index in range(saved.page_count)
    )


def string_end(syntax: str, at: int) -> int:
    """Return the index after the literal string at `syntax[at]`."""
    depth = 0
    end = at
    while end < len(syntax):
        if syntax[end] == "\\":
            end += 2
            continue
        depth += {"(": 1, ")": -1}.get(syntax[end], 0)
        end += 1
        if depth == 0:
            break
    return end


def tokens(syntax: str) -> list[str]:
    """Split PDF syntax into tokens, comments dropped (ISO 32000-2:2020, 7.2)."""
    found = []
    at = 0
    while at < len(syntax):
        character = syntax[at]
        end = at + 1
        if character == "(":
            end = string_end(syntax, at)
        elif syntax.startswith(("<<", ">>"), at):
            end = at + 2
        elif character == "<":
            end = syntax.find(">", at) + 1 or len(syntax)
        elif character in WHITE:
            at = end
            continue
        elif character == "%":
            while end < len(syntax) and syntax[end] not in "\r\n":
                end += 1
            at = end
            continue
        elif character not in DELIMITERS or character == "/":
            while end < len(syntax) and syntax[end] not in WHITE + DELIMITERS:
                end += 1
        found.append(syntax[at:end])
        at = end
    return found


def parse_object(words: list[str], at: int) -> tuple[Obj, int]:
    """Return the object at `words[at]` and the index after it."""
    first = words[at]
    if first == "<<":
        entries: dict[str, Obj] = {}
        at += 1
        while words[at] != ">>":
            key = words[at].removeprefix("/")
            entries[key], at = parse_object(words, at + 1)
        return entries, at + 1
    if first == "[":
        items: list[Obj] = []
        at += 1
        while words[at] != "]":
            item, at = parse_object(words, at)
            items.append(item)
        return items, at + 1
    if first.startswith("/"):
        return Name(first[1:]), at + 1
    if (
        first.isdigit()
        and words[at + 1 : at + 2] != []
        and words[at + 1].isdigit()
        and words[at + 2 : at + 3] == ["R"]
    ):
        return Ref(int(first), int(words[at + 1])), at + 3
    return first, at + 1


def object_at(document: starlightpdf.Document, found: Ref) -> Obj:
    """Return the object `found`, parsed."""
    syntax = document.object(
        starlightpdf.ObjectId(number=found.number, generation=found.generation)
    )
    return parse_object(tokens(syntax), 0)[0]


def resolve(document: starlightpdf.Document, value: Obj | None) -> Obj:
    """Return the object `value` is, read when it is a reference."""
    if value is None:
        msg = "a needed entry is missing"
        raise ValueError(msg)
    return object_at(document, value) if isinstance(value, Ref) else value


def entry(dictionary: Obj, key: str) -> Obj | None:
    """Return the value of `key` in `dictionary`, if any."""
    return dictionary.get(key) if isinstance(dictionary, dict) else None


def text(value: Obj | None) -> str:
    """Return the token `value` is; a name without its slash."""
    if isinstance(value, Name):
        return value.value
    if isinstance(value, str):
        return value
    msg = "a needed name or number is missing"
    raise ValueError(msg)


def string_bytes(token: str) -> bytes:
    """Return the bytes of a string token (ISO 32000-2:2020, 7.3.4)."""
    if token.startswith("<"):
        digits = "".join(token[1:-1].split())
        return bytes.fromhex(digits + "0" * (len(digits) % 2))
    escapes = {"n": b"\n", "r": b"\r", "t": b"\t", "b": b"\b", "f": b"\f"}
    out = bytearray()
    pattern = r"\\(?:[0-7]{1,3}|\r\n|.)|[^\\]"
    for match in re.finditer(pattern, token[1:-1], re.DOTALL):
        part = match.group()
        if not part.startswith("\\"):
            out += part.encode("latin-1")
        elif part[1:].isdigit():
            out.append(int(part[1:], 8) & 0xFF)
        elif part[1:] not in {"\r", "\n", "\r\n"}:
            out += escapes.get(part[1:], part[1:].encode("latin-1"))
    return bytes(out)


def color_space(document: starlightpdf.Document, value: Obj | None) -> str:
    """Return the color space `value` names (ISO 32000-2:2020, 8.6)."""
    resolved = resolve(document, value)
    if not isinstance(resolved, list):
        return text(resolved)
    family = text(resolved[0])
    if family == "ICCBased":
        profile = resolve(document, resolved[1])
        return f"ICCBased {number(float(text(entry(profile, 'N'))))}"
    if family == "Indexed":
        _, base, high, lookup = resolved
        table = (
            stream_bytes(document, as_id(lookup), decode=True)
            if isinstance(lookup, Ref)
            else string_bytes(text(lookup))
        )
        return (
            f"Indexed {color_space(document, base)} "
            f"{number(float(text(high)))} {table.hex()}"
        )
    return family


def icc(document: starlightpdf.Document, value: Obj | None) -> Ref | None:
    """Return the ICC profile of the color space `value` or its `Indexed` base."""
    resolved = resolve(document, value)
    if not isinstance(resolved, list):
        return None
    family = text(resolved[0])
    if family == "ICCBased" and isinstance(resolved[1], Ref):
        return resolved[1]
    return icc(document, resolved[1]) if family == "Indexed" else None


def as_id(found: Ref) -> starlightpdf.ObjectId:
    """`found` as an object identifier."""
    return starlightpdf.ObjectId(number=found.number, generation=found.generation)


def rounded(value: float) -> str:
    """Round `value` to 4 fraction digits; no trailing zeros or point, -0 as 0."""
    fixed = f"{value:.4f}".rstrip("0").rstrip(".")
    return "0" if fixed == "-0" else fixed


def matrix(document: starlightpdf.Document, page_object: Obj) -> str:
    """Return the CTM at the `Do`: the product of each `cm` in effect (8.3.4)."""
    contents = entry(page_object, "Contents")
    refs = contents if isinstance(contents, list) else [contents]
    content = b"\n".join(
        stream_bytes(document, as_id(item), decode=True)
        for item in refs
        if isinstance(item, Ref)
    )
    states: list[list[float]] = []
    current = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0]
    operands: list[str] = []
    for word in tokens(content.decode("latin-1")):
        if word == "q":
            states.append(current)
        elif word == "Q" and states:
            current = states.pop()
        elif word == "cm":
            ma, mb, mc, md, me, mf = (float(operand) for operand in operands[-6:])
            ca, cb, cc, cd, ce, cf = current
            current = [
                ma * ca + mb * cc,
                ma * cb + mb * cd,
                mc * ca + md * cc,
                mc * cb + md * cd,
                me * ca + mf * cc + ce,
                me * cb + mf * cd + cf,
            ]
        elif word == "Do":
            return " ".join(rounded(value) for value in current)
        if word[:1].isalpha() or word[:1] in {"'", '"'}:
            operands.clear()
        else:
            operands.append(word)
    msg = "no Do in the page's content"
    raise ValueError(msg)


def image_read(document: starlightpdf.Document, read: str) -> str:
    """Return what `read` gives of the image on the first page."""
    catalog = document.catalog
    root = object_at(document, Ref(catalog.number, catalog.generation))
    kids = resolve(document, entry(resolve(document, entry(root, "Pages")), "Kids"))
    page_object = resolve(document, kids[0] if isinstance(kids, list) else None)
    resources = resolve(document, entry(page_object, "Resources"))
    xobjects = resolve(document, entry(resources, "XObject"))
    found = next(iter(xobjects.values()), None) if isinstance(xobjects, dict) else None
    if not isinstance(found, Ref):
        msg = "no image XObject on the page"
        raise TypeError(msg)
    image = object_at(document, found)

    def digest(value: Obj | None) -> str:
        if not isinstance(value, Ref):
            return "none"
        return hashlib.sha256(
            stream_bytes(document, as_id(value), decode=True)
        ).hexdigest()

    def integer(key: str) -> str:
        return number(float(text(entry(image, key))))

    reads: dict[str, Callable[[], str]] = {
        "matrix": lambda: matrix(document, page_object),
        "size": lambda: f"{integer('Width')} {integer('Height')}",
        "bits": lambda: integer("BitsPerComponent"),
        "color_space": lambda: color_space(document, entry(image, "ColorSpace")),
        "filter": lambda: filters(entry(image, "Filter")),
        "raw_sha256": lambda: hashlib.sha256(
            stream_bytes(document, as_id(found), decode=False)
        ).hexdigest(),
        "samples_sha256": lambda: digest(found),
        "mask_samples_sha256": lambda: digest(entry(image, "SMask")),
        "icc_sha256": lambda: digest(icc(document, entry(image, "ColorSpace"))),
    }
    return reads[read]()


def filters(value: Obj | None) -> str:
    """Return the `Filter` names, or `none` (ISO 32000-2:2020, 7.4)."""
    if value is None:
        return "none"
    return " ".join(
        text(item) for item in (value if isinstance(value, list) else [value])
    )


IMAGE_READS = {
    "bits",
    "color_space",
    "filter",
    "icc_sha256",
    "mask_samples_sha256",
    "matrix",
    "raw_sha256",
    "samples_sha256",
    "size",
}


def drawn(args: list[str]) -> str:
    """Draw the image first in `args` on a new page, save, reopen and read."""
    path, *options = args
    given = parse(options, DRAW, {"read"})
    if given["read"] not in IMAGE_READS:
        msg = f"bad read {given['read']!r}"
        raise ValueError(msg)
    document = starlightpdf.Document.create()
    new = document.insert_blank_page(
        width=optional(given.get("page_width")),
        height=optional(given.get("page_height")),
    )
    mask = source("bytes", given["mask"]) if "mask" in given else None
    new.draw_image(
        source("bytes", path),
        mask,
        None,
        x=float(given.get("x", "0")),
        y=float(given.get("y", "0")),
        width=optional(given.get("width")),
        height=optional(given.get("height")),
        ignore_orientation="ignore_orientation" in given,
        format=given.get("format"),
        opacity=optional(given.get("opacity")),
    )
    return image_read(reopen(twice(document, {}), {}), given["read"])


OPERATIONS: dict[str, Callable[[list[str]], str]] = {
    "header_version": lambda args: header_version(*args),
    "insert_blank_page": inserted,
    "draw_image": drawn,
    **{
        f"{family}_{kind}": functools.partial(perform, family, kind)
        for family in (*READS, "save", "split")
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
    except (IndexError, OSError, TypeError, ValueError) as error:
        return f"{operation}: {error}"
    if actual != expected:
        return f"{operation}: expected {expected!r}, got {actual!r}"
    return None


def run(suites: list[tuple[str, str]]) -> None:
    """Run every case; fail naming each bad case and disagreeing pair by `file:line`."""
    errors = []
    first: dict[tuple[str, ...], tuple[str, str]] = {}
    found = False
    for name, suite in suites:
        lines = suite.split("\n")
        if lines[-1] == "":
            lines.pop()
        for line_number, raw in enumerate(lines, 1):
            line = raw.removesuffix("\r")
            if line.startswith("#"):
                continue
            found = True
            position = f"{name}:{line_number}"
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
