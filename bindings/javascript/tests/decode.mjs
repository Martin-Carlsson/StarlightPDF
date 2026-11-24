// Reads the PNG and JPEG files a render writes, in Node.js or a browser, sharing no code with the library.
//
// Only what the writers make, so that a change to them fails here: PNG (W3C, third edition) of 8-bit grey or RGB
// without alpha; baseline JPEG (ITU-T T.81) of 1x1 sampling without restarts, by its DC terms alone, which are each
// block's mean (A.3.3): right where a block is flat.

/**
 * @typedef {{ channels: number, height: number, jpeg: boolean, samples: Uint8Array, width: number }} Image
 * @typedef {Map<number, number>} Table A Huffman table: each code after a leading one bit, and its symbol.
 * @typedef {{ id: number, quantiser: number }} Component
 */

/** A PNG's first bytes (PNG, 5.2). */
const SIGNATURE = [0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a];

/** The CRC-32 polynomial, reversed (PNG, 5.5). */
const POLYNOMIAL = 0xedb8_8320;

/** @type {(condition: boolean, what: string) => asserts condition} */
function need(condition, what) {
  if (!condition) {
    throw new Error(what);
  }
}

/** @type {(data: Uint8Array, at: number) => number} */
function byte(data, at) {
  const value = data[at];
  need(value !== undefined, "the file ends early");
  return value;
}

/** The big-endian number of the `count` bytes at `at`. @type {(data: Uint8Array, at: number, count: number) => number} */
function number(data, at, count) {
  let value = 0;
  for (let index = at; index < at + count; index += 1) {
    value = value * 256 + byte(data, index);
  }
  return value;
}

/** @type {(data: Uint8Array) => number} */
function crc(data) {
  let value = 0xffff_ffff;
  for (const next of data) {
    value ^= next;
    for (let bit = 0; bit < 8; bit += 1) {
      value = (value >>> 1) ^ (value & 1 ? POLYNOMIAL : 0);
    }
  }
  return (value ^ 0xffff_ffff) >>> 0;
}

/** `count` codes' base and extra bits from `first` (RFC 1951, 3.2.5). @type {(count: number, first: number, step: number) => [number, number][]} */
function runs(count, first, step) {
  /** @type {[number, number][]} */
  const pairs = [];
  let base = first;
  for (let code = 0; code < count; code += 1) {
    const extra = Math.max(Math.floor(code / step) - 1, 0);
    pairs.push([base, extra]);
    base += 1 << extra;
  }
  return pairs;
}

/** Each length code's base and extra bits from 257, and each distance code's. */
const [LENGTHS, DISTANCES] = [[...runs(28, 3, 4), [258, 0]], runs(30, 1, 2)];

/** The fixed codes: each run's bits, first code, end and first symbol (RFC 1951, 3.2.6). */
const FIXED = [
  [7, 0x00, 0x18, 256],
  [8, 0x30, 0xc0, 0],
  [8, 0xc0, 0xc8, 280],
  [9, 0x190, 0x200, 144],
];

/** The one zlib stream `parts` hold, of fixed-code blocks (RFC 1950, 2.2; RFC 1951, 3.2.6). @type {(parts: Uint8Array[]) => Uint8Array} */
function inflate(parts) {
  const packed = new Uint8Array(parts.reduce((sum, part) => sum + part.length, 0));
  let filled = 0;
  for (const part of parts) {
    packed.set(part, filled);
    filled += part.length;
  }
  const [method, flags] = [byte(packed, 0), byte(packed, 1)];
  need((method & 15) === 8 && (flags & 0x20) === 0 && (method * 256 + flags) % 31 === 0, "not a zlib stream");
  let at = 16;
  /** The next `count` bits, the first lowest. @type {(count: number) => number} */
  const take = (count) => {
    let value = 0;
    for (let place = 0; place < count; place += 1) {
      value |= ((byte(packed, at >> 3) >> (at & 7)) & 1) << place;
      at += 1;
    }
    return value;
  };
  /** `value` then the next `count` bits of a code, the first highest. @type {(count: number, value: number) => number} */
  const code = (count, value) => {
    let found = value;
    for (let step = 0; step < count; step += 1) {
      found = (found << 1) | take(1);
    }
    return found;
  };
  const symbol = () => {
    let [found, length] = [0, 0];
    for (const [bits = 0, first = 0, end = 0, value = 0] of FIXED) {
      [found, length] = [code(bits - length, found), bits];
      if (found >= first && found < end) {
        return found - first + value;
      }
    }
    throw new Error("a code past the fixed ones");
  };
  /** @type {number[]} */
  const out = [];
  let last = false;
  while (!last) {
    last = take(1) === 1;
    need(take(2) === 1, "a block not of fixed codes");
    for (let next = symbol(); next !== 256; next = symbol()) {
      if (next < 256) {
        out.push(next);
        continue;
      }
      const [base = 0, extra = 0] = LENGTHS[next - 257] ?? [];
      need(base > 0, "a wrong length");
      const length = base + take(extra);
      const [start = 0, far = 0] = DISTANCES[code(5, 0)] ?? [];
      need(start > 0, "a wrong distance");
      const distance = start + take(far);
      need(distance <= out.length, "a distance before the start");
      for (let copy = 0; copy < length; copy += 1) {
        out.push(out[out.length - distance] ?? 0);
      }
    }
  }
  let [low, high] = [1, 0];
  for (const value of out) {
    low = (low + value) % 65_521;
    high = (high + low) % 65_521;
  }
  const end = (at + 7) >> 3;
  need(
    end + 4 === packed.length && number(packed, end, 4) === high * 65_536 + low,
    "a wrong Adler-32 or bytes after the stream",
  );
  return Uint8Array.from(out);
}

/** What filter `kind` predicts (PNG, 9.2, 9.4). @type {(kind: number, left: number, over: number, corner: number) => number} */
function predicted(kind, left, over, corner) {
  if (kind === 1) {
    return left;
  }
  if (kind === 2) {
    return over;
  }
  if (kind === 3) {
    return (left + over) >> 1;
  }
  const near = left + over - corner;
  const [toLeft, toOver, toCorner] = [Math.abs(near - left), Math.abs(near - over), Math.abs(near - corner)];
  return toLeft <= toOver && toLeft <= toCorner ? left : toOver <= toCorner ? over : corner;
}

/** A PNG without transparency (PNG, 5.6, 11.2.1). @type {(data: Uint8Array) => Promise<Image>} */
async function png(data) {
  const name = new TextDecoder("latin1");
  /** @type {Uint8Array[]} */
  const parts = [];
  let header = new Uint8Array();
  let ended = false;
  let afterData = false;
  for (let at = SIGNATURE.length; at < data.length;) {
    const end = at + 8 + number(data, at, 4);
    need(!ended && crc(data.subarray(at + 4, end)) === number(data, end, 4), "a wrong CRC or bytes after IEND");
    const [kind, body] = [name.decode(data.subarray(at + 4, at + 8)), data.subarray(at + 8, end)];
    if (at === SIGNATURE.length) {
      need(kind === "IHDR" && body.length === 13, "IHDR is not first");
      header = new Uint8Array(body);
    } else if (kind === "IDAT") {
      need(!afterData, "IDATs apart");
      parts.push(body);
    } else if (kind === "IEND") {
      need(body.length === 0, "IEND with data");
      ended = true;
    } else {
      need(/^[a-z]/u.test(kind) && kind !== "tRNS" && kind !== "acTL", "a chunk out of place or transparency");
      afterData = parts.length > 0;
    }
    at = end + 4;
  }
  need(ended && header.length === 13 && number(header, 10, 3) === 0 && byte(header, 8) === 8, "not 8 bits");
  const [width, height, colour] = [number(header, 0, 4), number(header, 4, 4), byte(header, 9)];
  need(width > 0 && height > 0 && (colour === 0 || colour === 2), "not opaque grey or RGB");
  const channels = colour === 0 ? 1 : 3;
  const stride = width * channels;
  const raw = inflate(parts);
  need(raw.length === height * (stride + 1), "rows of another size");
  const samples = new Uint8Array(height * stride);
  for (let row = 0; row < height; row += 1) {
    const [from, to] = [row * (stride + 1), row * stride];
    const kind = byte(raw, from);
    need(kind <= 4, "an unknown filter");
    for (let at = 0; at < stride; at += 1) {
      let value = byte(raw, from + 1 + at);
      if (kind > 0) {
        const left = at >= channels ? byte(samples, to + at - channels) : 0;
        const over = row > 0 ? byte(samples, to - stride + at) : 0;
        const corner = row > 0 && at >= channels ? byte(samples, to - stride + at - channels) : 0;
        value += predicted(kind, left, over, corner);
      }
      samples[to + at] = value & 0xff;
    }
  }
  return { channels, height, jpeg: false, samples, width };
}

/** A scan's entropy-coded data. */
class Entropy {
  /** @param {Uint8Array} data */
  constructor(data) {
    this.data = data;
    this.at = 0;
    this.held = 0;
    this.left = 0;
  }

  take() {
    this.at += 1;
    return byte(this.data, this.at - 1);
  }

  /** The next bit, past stuffed bytes (ITU-T T.81, F.2.2.5). */
  bit() {
    if (this.left === 0) {
      this.held = this.take();
      need(this.held !== 0xff || this.take() === 0, "a marker within the scan's data");
      this.left = 8;
    }
    this.left -= 1;
    return (this.held >> this.left) & 1;
  }

  /** @param {Table} table */
  symbol(table) {
    let code = 1;
    for (let length = 0; length < 16; length += 1) {
      code = code * 2 + this.bit();
      const symbol = table.get(code);
      if (symbol !== undefined) {
        return symbol;
      }
    }
    throw new Error("no such Huffman code");
  }

  /**
   * The next signed number of `size` bits (ITU-T T.81, F.2.2.1).
   * @param {number} size
   */
  number(size) {
    let value = 0;
    for (let bit = 0; bit < size; bit += 1) {
      value = value * 2 + this.bit();
    }
    return size > 0 && value < 2 ** (size - 1) ? value - 2 ** size + 1 : value;
  }

  /**
   * Passes padding of ones and the marker `code` (ITU-T T.81, B.1.1.5).
   * @param {number} code
   */
  marker(code) {
    while (this.left > 0) {
      need(this.bit() === 1, "padding other than ones");
    }
    need(this.take() === 0xff && this.take() === code, "a marker missing after the scan's data");
  }
}

/** Takes a DHT segment's tables (ITU-T T.81, B.2.4.2, Annex C). @type {(body: Uint8Array, tables: Map<number, Table>) => void} */
function huffman(body, tables) {
  for (let at = 0; at < body.length;) {
    const key = byte(body, at);
    need([0x00, 0x01, 0x10, 0x11].includes(key), "a table baseline JPEG lacks");
    /** @type {Table} */
    const table = new Map();
    let next = at + 17;
    let code = 0;
    for (let length = 1; length <= 16; length += 1) {
      for (let count = byte(body, at + length); count > 0; count -= 1) {
        table.set(2 ** length + code, byte(body, next));
        next += 1;
        code += 1;
      }
      need(code <= 2 ** length, "a Huffman code with too many codes");
      code *= 2;
    }
    tables.set(key, table);
    at = next;
  }
}

/** Takes a DQT segment's DC quantisers (ITU-T T.81, B.2.4.1). @type {(body: Uint8Array, steps: Map<number, number>) => void} */
function quantise(body, steps) {
  for (let at = 0; at < body.length; at += 65) {
    const table = body.subarray(at + 1, at + 65);
    need(byte(body, at) <= 3 && table.length === 64 && !table.includes(0), "a quantiser baseline JPEG lacks");
    steps.set(byte(body, at), byte(table, 0));
  }
}

/** A SOF0 segment's components (ITU-T T.81, B.2.2). @type {(body: Uint8Array) => Component[]} */
function components(body) {
  const count = byte(body, 5);
  need(byte(body, 0) === 8 && (count === 1 || count === 3) && body.length === 6 + 3 * count, "not 1 or 3 of 8 bits");
  return Array.from({ length: count }, (_, index) => {
    const at = 6 + 3 * index;
    need(byte(body, at + 1) === 0x11, "sampling other than 1x1");
    return { id: byte(body, at), quantiser: byte(body, at + 2) };
  });
}

/** A baseline JPEG (ITU-T T.81, B.2, F.2.2) as each block's mean, RGB by ITU-T T.871, 7. @type {(data: Uint8Array) => Image} */
function jpeg(data) {
  /** @type {Map<number, Table>} */
  const tables = new Map();
  /** @type {Map<number, number>} */
  const steps = new Map();
  /** @type {Component[]} */
  let parts = [];
  let [width, height] = [0, 0];
  let at = 2;
  let scan = new Uint8Array();
  while (scan.length === 0) {
    need(byte(data, at) === 0xff, "no marker where one is due");
    const kind = byte(data, at + 1);
    const length = number(data, at + 2, 2);
    const end = at + 2 + length;
    need(length >= 2 && end <= data.length, "a short segment");
    const body = data.subarray(at + 4, end);
    if (kind === 0xc0) {
      need(parts.length === 0, "two frames");
      parts = components(body);
      [height, width] = [number(body, 1, 2), number(body, 3, 2)];
      need(width > 0 && height > 0, "no pixels");
    } else if (kind === 0xc4) {
      huffman(body, tables);
    } else if (kind === 0xdb) {
      quantise(body, steps);
    } else if (kind === 0xda) {
      need(body.length > 0, "an empty SOS");
      scan = new Uint8Array(body);
    } else {
      need((kind >= 0xe0 && kind <= 0xef) || kind === 0xfe, "not baseline JPEG");
    }
    at = end;
  }
  need(parts.length > 0, "no SOF0 frame");
  const count = byte(scan, 0);
  need(
    count === parts.length && scan.length === 4 + 2 * count && number(scan, 1 + 2 * count, 3) === 63 * 256,
    "not one scan of every component",
  );
  const [columns, rows] = [Math.ceil(width / 8), Math.ceil(height / 8)];
  const planes = parts.map((part, index) => {
    need(byte(scan, 1 + 2 * index) === part.id, "a scan out of the frame's order");
    const selected = byte(scan, 2 + 2 * index);
    const [direct, others, step] = [
      tables.get(selected >> 4),
      tables.get(0x10 | (selected & 15)),
      steps.get(part.quantiser),
    ];
    need(direct !== undefined && others !== undefined && step !== undefined, "a table is missing");
    return { before: 0, direct, levels: new Int32Array(columns * rows), others, step };
  });
  const entropy = new Entropy(data.subarray(at));
  for (let mcu = 0; mcu < columns * rows; mcu += 1) {
    for (const plane of planes) {
      const size = entropy.symbol(plane.direct);
      need(size <= 11, "a DC category baseline JPEG lacks");
      plane.before += entropy.number(size);
      for (let index = 1; index < 64;) {
        const symbol = entropy.symbol(plane.others);
        if (symbol === 0) {
          break;
        }
        need((symbol & 15) <= 10 && ((symbol & 15) > 0 || symbol === 0xf0), "a bad AC symbol");
        index += (symbol >> 4) + 1;
        need(index <= 64, "a block of over 64 coefficients");
        entropy.number(symbol & 15);
      }
      plane.levels[mcu] = plane.before * plane.step;
    }
  }
  entropy.marker(0xd9);
  need(entropy.at === entropy.data.length, "bytes after EOI");
  const channels = planes.length;
  const samples = new Uint8Array(width * height * channels);
  let next = 0;
  for (let row = 0; row < height; row += 1) {
    for (let column = 0; column < width; column += 1) {
      const at = (row >> 3) * columns + (column >> 3);
      const [luma = 0, blue = 0, red = 0] = planes.map((plane) => plane.levels[at] ?? 0);
      // In eighths: the DC term, less the level shift of 128.
      const base = (luma + 1024) * 0x1_0000;
      const mixed =
        channels === 1 ? [base] : [base + red * 91_881, base - blue * 22_554 - red * 46_802, base + blue * 116_130];
      for (const value of mixed) {
        samples[next] = Math.min(255, Math.max(0, Math.floor((value + 2 ** 18) / 2 ** 19)));
        next += 1;
      }
    }
  }
  return { channels, height, jpeg: true, samples, width };
}

/**
 * The PNG or baseline JPEG `data`.
 * @param {Uint8Array} data
 * @returns {Promise<Image>}
 */
export async function decode(data) {
  if (SIGNATURE.every((value, at) => data[at] === value)) {
    return png(data);
  }
  need(data[0] === 0xff && data[1] === 0xd8, "neither PNG nor JPEG");
  return jpeg(data);
}

/**
 * The samples of `image`'s pixel at `column` and `row`, if it has one.
 * @param {Image} image
 * @param {number} column
 * @param {number} row
 * @returns {Uint8Array | undefined}
 */
export function pixel(image, column, row) {
  if (column >= image.width || row >= image.height) {
    return undefined;
  }
  const start = (row * image.width + column) * image.channels;
  return image.samples.subarray(start, start + image.channels);
}
