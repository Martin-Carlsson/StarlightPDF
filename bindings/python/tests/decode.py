"""Reads the PNG and JPEG files a render writes, sharing no code with the library.

Only what the writers make, so that a change to them fails here: PNG (W3C, third
edition) of 8-bit grey or RGB without alpha; baseline JPEG (ITU-T T.81) of 1x1
sampling without restarts, by its DC terms alone, which are each block's mean
(A.3.3): right where a block is flat.
"""

from __future__ import annotations

import zlib
from typing import NamedTuple

PNG = b"\x89PNG\r\n\x1a\n"
SOI = b"\xff\xd8"
CHANNELS = {0: 1, 2: 3}
DEPTH = 8
RGB = 3
WORD = 4
FILTER_LEFT, FILTER_UP, FILTER_AVERAGE, FILTER_PAETH = 1, 2, 3, 4
MARKER = 0xFF
DQT, DHT, SOF0, SOS, EOI = 0xDB, 0xC4, 0xC0, 0xDA, 0xD9
APPLICATION = range(0xE0, 0xF0)
COMMENT = 0xFE
COEFFICIENTS = 64
CODE_LENGTHS = 16
ONE_BY_ONE = 0x11
TABLES = (0x00, 0x01, 0x10, 0x11)
QUANTISERS = 3
MOST_DC, MOST_AC = 11, 10
RUN_OF_ZEROS = 0xF0
ADLER = 65521
LENGTH = 2  # A JPEG segment's length counts its own 2 bytes.
END_OF_BLOCK = 256


def runs(count: int, first: int, step: int) -> tuple[tuple[int, int], ...]:
    """Return `count` codes' base and extra bits from `first` (RFC 1951, 3.2.5)."""
    pairs, base = [], first
    for code in range(count):
        extra = max(code // step - 1, 0)
        pairs.append((base, extra))
        base += 1 << extra
    return tuple(pairs)


LENGTHS = (*runs(28, 3, 4), (258, 0))
DISTANCES = runs(30, 1, 2)
# RFC 1951, 3.2.6: the fixed codes: each run's bits, first code, end and first symbol.
FIXED = (
    (7, 0x00, 0x18, 256),
    (8, 0x30, 0xC0, 0),
    (8, 0xC0, 0xC8, 280),
    (9, 0x190, 0x200, 144),
)
# RFC 1950, 2.2: the method, the preset dictionary flag and the check's divisor.
DEFLATE, PRESET, FCHECK = 8, 0x20, 31


class Image(NamedTuple):
    """A decoded file: its samples interleaved, a row after another."""

    width: int
    height: int
    channels: int
    jpeg: bool
    samples: bytes

    def pixel(self, column: int, row: int) -> bytes | None:
        """Return the samples at `column` and `row`, if the image has that pixel."""
        if column >= self.width or row >= self.height:
            return None
        start = (row * self.width + column) * self.channels
        require(start + self.channels <= len(self.samples), "a row not decoded")
        return self.samples[start : start + self.channels]


def require(condition: object, what: str) -> None:
    """Fail with `what` unless `condition` holds."""
    if not condition:
        raise ValueError(what)


def read(data: bytes, rows: int | None = None) -> Image:
    """Decode a PNG, only its first `rows` rows if given, or a baseline JPEG."""
    if data.startswith(PNG):
        return png(data, rows)
    require(data.startswith(SOI), "neither PNG nor JPEG")
    return jpeg(data)


def chunks(data: bytes) -> list[tuple[bytes, bytes]]:
    """Return the PNG's chunks, each checked by its CRC (PNG, 5.3)."""
    found = []
    at = len(PNG)
    while at < len(data):
        end = at + 8 + int.from_bytes(data[at : at + WORD], "big")
        require(end + WORD <= len(data), "a short chunk")
        kind, body = data[at + WORD : at + 8], data[at + 8 : end]
        sum_ = int.from_bytes(data[end : end + WORD], "big")
        require(zlib.crc32(kind + body) == sum_, "a wrong CRC")
        found.append((kind, body))
        at = end + WORD
    return found


def png(data: bytes, rows: int | None = None) -> Image:
    """Decode an opaque PNG, its first `rows` rows if given (PNG, 5.6, 11.2.1)."""
    found = chunks(data)
    names = [kind for kind, _ in found]
    require(names[:1] == [b"IHDR"] and names[-1:] == [b"IEND"], "IHDR or IEND")
    inner = names[1:-1]
    critical = [kind for kind in inner if kind[:1].isupper()]
    require(all(kind == b"IDAT" for kind in critical), "a chunk out of place")
    require(b"tRNS" not in inner and b"acTL" not in inner, "not opaque and still")
    at = [index for index, kind in enumerate(inner) if kind == b"IDAT"]
    require(at and at == list(range(at[0], at[-1] + 1)), "IDATs apart")
    header = found[0][1]
    width = int.from_bytes(header[:WORD], "big")
    height = int.from_bytes(header[WORD : 2 * WORD], "big")
    depth, colour, *rest = header[2 * WORD :]
    plain = (depth, rest) == (DEPTH, [0, 0, 0])
    require(width and height and plain and not found[-1][1], "not 8 bits or no pixels")
    require(colour in CHANNELS, "not opaque grey or RGB")
    packed = b"".join(body for kind, body in found if kind == b"IDAT")
    channels = CHANNELS[colour]
    raw = inflate(packed)
    require(len(raw) == height * (width * channels + 1), "rows of another size")
    samples = unfilter(
        raw, width * channels, channels, height if rows is None else rows
    )
    return Image(width, height, channels, jpeg=False, samples=samples)


class Bits:
    """The bits of a deflate stream, each byte's lowest first (RFC 1951, 3.1.1)."""

    def __init__(self, data: bytes) -> None:
        """Read `data` from its first bit."""
        self.data = data
        self.at = 0

    def take(self, count: int) -> int:
        """Return the next `count` bits as a number, the first lowest."""
        value = 0
        for place in range(count):
            require(self.at >> 3 < len(self.data), "a short deflate stream")
            value |= (self.data[self.at >> 3] >> (self.at & 7) & 1) << place
            self.at += 1
        return value

    def code(self, count: int, value: int = 0) -> int:
        """Return `value` then the next `count` bits of a code, the first highest."""
        for _ in range(count):
            value = value << 1 | self.take(1)
        return value

    def symbol(self) -> int:
        """Return the next literal or length of the fixed codes (RFC 1951, 3.2.6)."""
        code, length = 0, 0
        for bits, first, end, symbol in FIXED:
            code, length = self.code(bits - length, code), bits
            if first <= code < end:
                return code - first + symbol
        msg = "a code past the fixed ones"
        raise ValueError(msg)


def inflate(packed: bytes) -> bytes:
    """Decode the one zlib stream `packed` is, of fixed codes (RFC 1950, 1951)."""
    method, flags = packed[:1], packed[1:2]
    require(
        method and flags and method[0] & 15 == DEFLATE and not flags[0] & PRESET,
        "not a zlib stream",
    )
    require(int.from_bytes(packed[:2], "big") % FCHECK == 0, "not a zlib stream")
    bits, out = Bits(packed[2:]), bytearray()
    last = False
    while not last:
        last = bits.take(1) == 1
        require(bits.take(2) == 1, "a block not of fixed codes")
        while (symbol := bits.symbol()) != END_OF_BLOCK:
            if symbol < END_OF_BLOCK:
                out.append(symbol)
                continue
            require(symbol - END_OF_BLOCK - 1 < len(LENGTHS), "a wrong length")
            base, extra = LENGTHS[symbol - END_OF_BLOCK - 1]
            length = base + bits.take(extra)
            code = bits.code(5)
            require(code < len(DISTANCES), "a wrong distance")
            start, extra = DISTANCES[code]
            distance = start + bits.take(extra)
            require(distance <= len(out), "a distance before the start")
            for _ in range(length):
                out.append(out[-distance])
    end = len(method + flags) + (bits.at + 7 >> 3)
    low, high = 1, 0
    for byte in out:
        low = (low + byte) % ADLER
        high = (high + low) % ADLER
    require(
        packed[end:] == (high << 16 | low).to_bytes(4, "big"),
        "a wrong Adler-32 or bytes after the stream",
    )
    return bytes(out)


def unfilter(raw: bytes, stride: int, step: int, rows: int) -> bytes:
    """Return the samples of the first `rows` filtered rows of `raw` (PNG, 9.2)."""
    samples = bytearray()
    above = bytes(stride)
    starts = range(0, len(raw), stride + 1)
    require(all(raw[start] <= FILTER_PAETH for start in starts), "an unknown filter")
    for start in starts[:rows]:
        kind, row = raw[start], bytearray(raw[start + 1 : start + 1 + stride])
        for at in range(stride) if kind else ():
            left = row[at - step] if at >= step else 0
            corner = above[at - step] if at >= step else 0
            row[at] = (row[at] + predicted(kind, left, above[at], corner)) & 0xFF
        samples += row
        above = bytes(row)
    return bytes(samples)


def predicted(kind: int, left: int, over: int, corner: int) -> int:
    """Return what filter `kind` predicts (PNG, 9.2, 9.4)."""
    if kind == FILTER_LEFT:
        return left
    if kind == FILTER_UP:
        return over
    if kind == FILTER_AVERAGE:
        return (left + over) // 2
    near = left + over - corner
    to_left, to_over, to_corner = abs(near - left), abs(near - over), abs(near - corner)
    if to_left <= to_over and to_left <= to_corner:
        return left
    return over if to_over <= to_corner else corner


class Entropy:
    """A scan's entropy-coded data."""

    def __init__(self, data: bytes) -> None:
        """Start at the data's first bit."""
        self.data = data
        self.at = self.byte = self.left = 0

    def take(self) -> int:
        """Return the next byte."""
        require(self.at < len(self.data), "the scan's data ends early")
        self.at += 1
        return self.data[self.at - 1]

    def bit(self) -> int:
        """Return the next bit, past stuffed bytes (ITU-T T.81, F.2.2.5)."""
        if not self.left:
            self.byte = self.take()
            stuffed = self.byte != MARKER or self.take() == 0
            require(stuffed, "a marker within the scan's data")
            self.left = 8
        self.left -= 1
        return self.byte >> self.left & 1

    def symbol(self, table: dict[tuple[int, int], int]) -> int:
        """Return the next symbol of `table`."""
        code = 0
        for length in range(1, CODE_LENGTHS + 1):
            code = code * 2 + self.bit()
            if (length, code) in table:
                return table[length, code]
        msg = "no such Huffman code"
        raise ValueError(msg)

    def number(self, size: int) -> int:
        """Return the next signed number of `size` bits (ITU-T T.81, F.2.2.1)."""
        value = 0
        for _ in range(size):
            value = value * 2 + self.bit()
        negative = size and value < 1 << (size - 1)
        return value - (1 << size) + 1 if negative else value

    def marker(self, code: int) -> None:
        """Pass padding of ones and the marker `code` (ITU-T T.81, B.1.1.5)."""
        while self.left:
            require(self.bit(), "padding other than ones")
        require(self.take() == MARKER and self.take() == code, "a marker missing")


class Component(NamedTuple):
    """A component of the frame."""

    ident: int
    quantiser: int


class Header(NamedTuple):
    """The segments before the scan's data."""

    segments: dict[int, list[bytes]]
    scan: bytes
    data: bytes


def header(data: bytes) -> Header:
    """Split a baseline JPEG's segments from its scan's data (ITU-T T.81, B.2)."""
    segments: dict[int, list[bytes]] = {}
    at = len(SOI)
    while at + WORD <= len(data):
        require(data[at] == MARKER, "no marker where one is due")
        kind = data[at + 1]
        length = int.from_bytes(data[at + 2 : at + WORD], "big")
        end = at + 2 + length
        require(length >= LENGTH and end <= len(data), "a short segment")
        body = data[at + WORD : end]
        if kind == SOS:
            return Header(segments, body, data[end:])
        known = kind in {DQT, DHT, SOF0, COMMENT} or kind in APPLICATION
        require(known, "not baseline JPEG")
        segments.setdefault(kind, []).append(body)
        at = end
    msg = "no scan"
    raise ValueError(msg)


def huffman(bodies: list[bytes]) -> dict[int, dict[tuple[int, int], int]]:
    """Return the tables of DHT segments (ITU-T T.81, B.2.4.2, Annex C)."""
    tables: dict[int, dict[tuple[int, int], int]] = {}
    for segment in bodies:
        rest = segment
        while rest:
            key, counts = rest[0], rest[1 : CODE_LENGTHS + 1]
            require(key in TABLES, "a table baseline JPEG lacks")
            end = CODE_LENGTHS + 1 + sum(counts)
            require(end <= len(rest), "a short DHT")
            symbols = iter(rest[CODE_LENGTHS + 1 : end])
            table: dict[tuple[int, int], int] = {}
            code = 0
            for length, count in enumerate(counts, 1):
                for _ in range(count):
                    table[length, code] = next(symbols)
                    code += 1
                require(code <= 1 << length, "a Huffman code with too many codes")
                code *= 2
            tables[key] = table
            rest = rest[end:]
    return tables


def quantisers(bodies: list[bytes]) -> dict[int, int]:
    """Return each table's DC quantiser (ITU-T T.81, B.2.4.1)."""
    found: dict[int, int] = {}
    for segment in bodies:
        for at in range(0, len(segment), COEFFICIENTS + 1):
            table = segment[at + 1 : at + COEFFICIENTS + 1]
            good = segment[at] <= QUANTISERS and len(table) == COEFFICIENTS
            require(good and 0 not in table, "a table baseline JPEG lacks")
            found[segment[at]] = table[0]
    return found


def frame(bodies: list[bytes]) -> tuple[int, int, list[Component]]:
    """Return the width, height and components of SOF0 (ITU-T T.81, B.2.2)."""
    require(len(bodies) == 1, "not one SOF0")
    depth, tall, low, wide, narrow, count, *specs = bodies[0]
    good = depth == DEPTH and count in CHANNELS.values() and len(specs) == count * RGB
    require(good, "not 1 or 3 components of 8 bits")
    components = []
    for at in range(0, len(specs), RGB):
        ident, factors, quantiser = specs[at : at + RGB]
        require(factors == ONE_BY_ONE, "sampling other than 1x1")
        components.append(Component(ident, quantiser))
    width, height = wide * 256 + narrow, tall * 256 + low
    require(width and height, "no pixels")
    return width, height, components


def selected(scan: bytes, components: list[Component]) -> list[tuple[int, int]]:
    """Return each component's DC and AC table (ITU-T T.81, B.2.3)."""
    count, *specs = scan
    size = 2 * count
    whole = count == len(components) and specs[size:] == [0, 63, 0]
    require(whole, "not one scan of every component")
    idents = specs[:size:2]
    require(idents == [part.ident for part in components], "a scan out of order")
    return [(value >> 4, 0x10 | value & 15) for value in specs[1:size:2]]


def pass_ac(entropy: Entropy, table: dict[tuple[int, int], int]) -> None:
    """Pass a block's AC symbols (ITU-T T.81, F.2.2.2)."""
    index = 1
    while index < COEFFICIENTS:
        symbol = entropy.symbol(table)
        if not symbol:
            return
        size = symbol & 15
        good = size <= MOST_AC and (size or symbol == RUN_OF_ZEROS)
        index += (symbol >> 4) + 1
        require(good and index <= COEFFICIENTS, "a bad AC symbol")
        entropy.number(size)


class Scan(NamedTuple):
    """What a scan's data is read with."""

    components: list[Component]
    tables: list[tuple[dict[tuple[int, int], int], dict[tuple[int, int], int]]]
    steps: list[int]
    columns: int
    rows: int


def levels(scan: Scan, data: bytes) -> list[list[int]]:
    """Return each component's dequantised DC terms, up to EOI (T.81, F.2.2)."""
    entropy = Entropy(data)
    found = [[0] * (scan.columns * scan.rows) for _ in scan.components]
    before = [0] * len(scan.components)
    for mcu in range(scan.columns * scan.rows):
        for index in range(len(scan.components)):
            direct, others = scan.tables[index]
            size = entropy.symbol(direct)
            require(size <= MOST_DC, "a DC category baseline JPEG lacks")
            before[index] += entropy.number(size)
            pass_ac(entropy, others)
            found[index][mcu] = before[index] * scan.steps[index]
    entropy.marker(EOI)
    require(entropy.at == len(data), "bytes after EOI")
    return found


def jpeg(data: bytes) -> Image:
    """Decode a baseline JPEG to each block's mean, RGB by ITU-T T.871, 7."""
    found = header(data)
    width, height, parts = frame(found.segments.get(SOF0, []))
    codes = huffman(found.segments.get(DHT, []))
    steps = quantisers(found.segments.get(DQT, []))
    chosen = selected(found.scan, parts)
    defined = all(low in codes and high in codes for low, high in chosen)
    require(defined and all(part.quantiser in steps for part in parts), "no table")
    scan = Scan(
        components=parts,
        tables=[(codes[low], codes[high]) for low, high in chosen],
        steps=[steps[part.quantiser] for part in parts],
        columns=-(-width // 8),
        rows=-(-height // 8),
    )
    samples = colours(scan, levels(scan, found.data), width, height)
    return Image(width, height, len(parts), jpeg=True, samples=samples)


def colours(scan: Scan, blocks: list[list[int]], width: int, height: int) -> bytes:
    """Return each pixel's colour from its blocks' DC terms (ITU-T T.871, 7)."""
    out = bytearray()
    for row in range(height):
        for column in range(width):
            at = row // 8 * scan.columns + column // 8
            means = [terms[at] for terms in blocks]
            # In eighths: the DC term, less the level shift of 128.
            base = (means[0] + 1024) * 65536
            mixed = [base]
            if len(means) == RGB:
                _, blue, red = means
                mixed = [
                    base + red * 91881,
                    base - blue * 22554 - red * 46802,
                    base + blue * 116130,
                ]
            out += bytes(min(255, max(0, (value + (1 << 18)) >> 19)) for value in mixed)
    return bytes(out)
