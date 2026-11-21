// The browser side of the tests: scenarios on a page or, through `worker`, in a worker.
import { run } from "./runner.mjs";

/**
 * @import { Api, Document, Sink } from "./api.mts"
 * @import { Environment } from "./runner.mjs"
 * @typedef {{ ok: unknown } | { error: string }} Outcome
 * @typedef {{ make: () => BlobPart, size: number }} Piece
 * @typedef {{ decoded: number, file: Layout }} Virtual
 * @typedef {{ save(sink: Sink): Promise<unknown> }} Part
 * @typedef {{ by: "Pages", count: number } | { by: "Ranges", starts: number[] }} Split
 * @typedef {Document & { split(options: Split): AsyncIterable<Part> | Promise<AsyncIterable<Part>> }} Splittable
 * @typedef {Api["Sink"] & { fromAppend(append: (chunk: Uint8Array) => void): Promise<Sink> }} Sinks
 */

/** Bytes in a mebibyte. */
const MIB = 1 << 20;

/** Bytes of each virtual file of the memory rule. */
const SIZE = 2 ** 30;

/** Most data bytes of a stored block (RFC 1951, 3.2.4). */
const BLOCK = 0xffff;

/** Bytes of a full stored block: its header byte, `LEN`, `NLEN` and data (RFC 1951, 3.2.4). */
const FULL_BLOCK = BLOCK + 5;

/** Bytes of a zlib stream besides its full blocks and final data: header, final block header, Adler-32 (RFC 1950, 2.2). */
const ZLIB = 11;

/** The modulus of Adler-32 (RFC 1950, 8.2). */
const ADLER = 65_521;

/** Decoded bytes of each stream of the file of large streams but the last. */
const LARGE = 2 ** 28;

/** Kids of a page tree node. */
const FANOUT = 64;

/** Pages of the file of small objects. */
const LEAVES = 7_400_000;

/** Bytes of a page object, a page tree node object and a cross-reference entry (ISO 32000-2:2020, 7.5.4), padded with spaces. */
const [LEAF, NODE, ENTRY] = [96, 1024, 20];

/** The shortest page object a saved part can hold: its type, its parent and its contents (ISO 32000-2:2020, 7.3.10, 7.7.3.3). */
const PAGE_OBJECT = "1 0 obj<</Type/Page/Parent 1 0 R/Contents 1 0 R>>endobj";

/**
 * The package at `entry`.
 * @param {string} entry
 * @returns {Promise<Api>}
 */
async function load(entry) {
  return import(entry);
}

/**
 * The bytes of the repository file at `path`.
 * @param {string} path
 */
async function read(path) {
  const response = await fetch(`/repo/${path}`);
  if (!response.ok) {
    throw new Error(`${path}: HTTP ${response.status}`);
  }
  return new Uint8Array(await response.arrayBuffer());
}

/**
 * A File in the origin private file system with the bytes of `path`; for a file the server lacks,
 * one whose entry is removed, so reading it fails.
 * @param {string} path
 */
async function stored(path) {
  const root = await navigator.storage.getDirectory();
  const name = path.replaceAll("/", "_");
  const handle = await root.getFileHandle(name, { create: true });
  const writable = await handle.createWritable();
  const response = await fetch(`/repo/${path}`);
  await writable.write(response.ok ? await response.arrayBuffer() : "gone");
  await writable.close();
  const file = await handle.getFile();
  if (!response.ok) {
    await root.removeEntry(name);
  }
  return file;
}

/**
 * Case files over HTTP; `*_file` cases through a File in the origin private file system.
 * @param {Api} api
 * @returns {Environment}
 */
function environment(api) {
  return { bytes: read, file: async (path) => api.Source.fromBlob(await stored(path)) };
}

/** Records every WebAssembly instance this thread makes from now on. */
function record() {
  /** @type {WebAssembly.Instance[]} */
  const instances = [];
  const { Instance, instantiate, instantiateStreaming } = WebAssembly;
  /** @param {Function} original */
  const wrap =
    (original) =>
    async (/** @type {unknown[]} */ ...args) => {
      /** @type {WebAssembly.WebAssemblyInstantiatedSource | WebAssembly.Instance} */
      const made = await Reflect.apply(original, WebAssembly, args);
      instances.push(made instanceof Instance ? made : made.instance);
      return made;
    };
  class Recorded extends Instance {
    /**
     * @param {WebAssembly.Module} module
     * @param {WebAssembly.Imports} [imports]
     */
    constructor(module, imports) {
      super(module, imports);
      instances.push(this);
    }
  }
  Object.assign(WebAssembly, {
    Instance: Recorded,
    instantiate: wrap(instantiate),
    instantiateStreaming: wrap(instantiateStreaming),
  });
  return instances;
}

/**
 * A valid two-page PDF of over 1 GiB, never held whole: its objects 256 MiB apart, between comment lines,
 * found through a classic cross-reference table (ISO 32000-2:2020, 7.2.4 and 7.5).
 */
function huge() {
  const lines = new Uint8Array(MIB).fill(0x61);
  for (let at = 0; at < MIB; at += 64) {
    lines[at] = 0x25;
    lines[at + 63] = 0x0a;
  }
  let filler = new Blob([lines]);
  for (let doubling = 0; doubling < 8; doubling += 1) {
    filler = new Blob([filler, filler]);
  }
  const leaf = "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << >> >>";
  const objects = ["<< /Type /Catalog /Pages 2 0 R >>", "<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >>", leaf, leaf];
  const header = "%PDF-1.7\n";
  /** @type {BlobPart[]} */
  const parts = [header];
  let offset = header.length;
  let entries = "";
  for (const [index, object] of objects.entries()) {
    parts.push(filler);
    offset += filler.size;
    entries += `${String(offset).padStart(10, "0")} 00000 n\r\n`;
    const text = `${index + 1} 0 obj\n${object}\nendobj\n`;
    parts.push(text);
    offset += text.length;
  }
  const size = objects.length + 1;
  parts.push(
    `xref\n0 ${size}\n0000000000 65535 f\r\n${entries}trailer\n<< /Size ${size} /Root 1 0 R >>\nstartxref\n${offset}\n%%EOF\n`,
  );
  return new Blob(parts);
}

/** A file of pieces, each made only when the Blob is. */
class Layout {
  /** @type {Piece[]} */
  pieces = [];
  size = 0;

  /**
   * Appends `piece`; where it starts.
   * @param {Piece} piece
   */
  push(piece) {
    const start = this.size;
    this.pieces.push(piece);
    this.size += piece.size;
    return start;
  }

  /**
   * Appends ASCII `text`; where it starts.
   * @param {string} text
   */
  text(text) {
    return this.push({ make: () => text, size: text.length });
  }

  blob() {
    return new Blob(this.pieces.map((piece) => piece.make()));
  }
}

/**
 * `count` copies of `blob`, by doubling.
 * @param {Blob} blob
 * @param {number} count
 */
function repeat(blob, count) {
  /** @type {Blob[]} */
  const parts = [];
  let doubled = blob;
  for (let rest = count; rest > 0; rest = Math.floor(rest / 2)) {
    if (rest % 2 === 1) {
      parts.push(doubled);
    }
    doubled = new Blob([doubled, doubled]);
  }
  return new Blob(parts);
}

/**
 * `FlateDecode` data of `decoded` zeros in stored blocks (RFC 1950; RFC 1951, 3.2.4).
 * @param {number} decoded
 * @returns {Piece}
 */
function flate(decoded) {
  const blocks = Math.floor(decoded / BLOCK);
  const rest = decoded % BLOCK;
  const make = () => {
    const sixteen = new Uint8Array(16 * FULL_BLOCK);
    for (let at = 0; at < sixteen.length; at += FULL_BLOCK) {
      sixteen.set([0, 0xff, 0xff, 0, 0], at);
    }
    const adler = (decoded % ADLER) * 0x1_0000 + 1;
    return new Blob([
      new Uint8Array([0x78, 0x01]),
      repeat(new Blob([sixteen]), Math.floor(blocks / 16)),
      repeat(new Blob([sixteen.subarray(0, FULL_BLOCK)]), blocks % 16),
      new Uint8Array([1, rest % 256, Math.floor(rest / 256), (BLOCK - rest) % 256, Math.floor((BLOCK - rest) / 256)]),
      new Uint8Array(rest),
      new Uint8Array([adler >>> 24, (adler >>> 16) % 256, (adler >>> 8) % 256, adler % 256]),
    ]);
  };
  return { make, size: ZLIB + decoded + 5 * blocks };
}

/**
 * `count` records of `width` bytes, each the text `render` gives its index padded with spaces.
 * @param {number} count
 * @param {number} width
 * @param {(index: number) => string} render
 * @returns {Piece}
 */
function records(count, width, render) {
  const make = () => {
    const per = Math.floor(MIB / width);
    const buffer = new Uint8Array(per * width);
    const encoder = new TextEncoder();
    /** @type {Blob[]} */
    const blobs = [];
    for (let first = 0; first < count; first += per) {
      const end = Math.min(first + per, count);
      buffer.fill(0x20);
      for (let index = first; index < end; index += 1) {
        const text = render(index);
        const at = (index - first) * width;
        if (encoder.encodeInto(text, buffer.subarray(at, at + width)).read !== text.length || text.length > width) {
          throw new Error(`record ${index} is over ${width} bytes`);
        }
      }
      blobs.push(new Blob([buffer.subarray(0, (end - first) * width)]));
    }
    return new Blob(blobs);
  };
  return { make, size: count * width };
}

/**
 * `offset` as the 10 digits of a cross-reference entry (ISO 32000-2:2020, 7.5.4).
 * @param {number} offset
 */
function digits(offset) {
  return String(offset).padStart(10, "0");
}

/**
 * Four `FlateDecode` streams of about 256 MiB each, the last of `last` decoded bytes, then the catalog, the page
 * tree root and its one page whose contents they are, after `pad` spaces; as the Rust memory tests build it.
 * @param {number} pad
 * @param {number} last
 * @returns {Virtual}
 */
function fewLargeStreams(pad, last) {
  const file = new Layout();
  file.text(`%PDF-2.0\n${" ".repeat(pad)}`);
  const offsets = [];
  for (const [number, decoded] of [
    [1, LARGE],
    [2, LARGE],
    [3, LARGE],
    [4, last],
  ]) {
    const data = flate(decoded ?? 0);
    offsets.push(file.text(`${number} 0 obj\n<< /Filter /FlateDecode /Length ${data.size} >>\nstream\n`));
    file.push(data);
    file.text("\nendstream\nendobj\n");
  }
  for (const object of [
    "5 0 obj\n<< /Type /Catalog /Pages 6 0 R >>\nendobj\n",
    "6 0 obj\n<< /Type /Pages /Kids [7 0 R] /Count 1 /MediaBox [0 0 612 792] >>\nendobj\n",
    "7 0 obj\n<< /Type /Page /Parent 6 0 R /Contents [1 0 R 2 0 R 3 0 R 4 0 R] >>\nendobj\n",
  ]) {
    offsets.push(file.text(object));
  }
  const entries = offsets.map((offset) => `${digits(offset)} 00000 n\r\n`).join("");
  const xref = file.text(`xref\n0 8\n0000000000 65535 f\r\n${entries}`);
  file.text(`trailer\n<< /Root 5 0 R /Size 8 >>\nstartxref\n${xref}\n%%EOF\n`);
  return { decoded: last, file };
}

/**
 * A page tree of `leaves` pages, nodes of {@link FANOUT} kids (ISO 32000-2:2020, 7.7.3.2); pages are objects 1 to
 * `leaves`, then nodes bottom level first, the stream and the catalog.
 */
class Tree {
  /** @param {number} leaves */
  constructor(leaves) {
    this.leaves = leaves;
    /** Nodes per level, bottom up; the last level is the root. */
    this.levels = [];
    for (let below = leaves; ;) {
      const nodes = Math.ceil(below / FANOUT);
      this.levels.push(nodes);
      if (nodes <= 1) {
        break;
      }
      below = nodes;
    }
    this.nodes = this.levels.reduce((sum, nodes) => sum + nodes, 0);
    this.root = leaves + this.nodes;
    this.stream = this.root + 1;
    this.catalog = this.stream + 1;
  }

  /**
   * The object number of item `index` of `level`; level 0 holds the pages.
   * @param {number} level
   * @param {number} index
   */
  id(level, index) {
    return level === 0
      ? index + 1
      : this.levels.slice(0, level - 1).reduce((sum, nodes) => sum + nodes, this.leaves + 1) + index;
  }

  /**
   * Page `index`.
   * @param {number} index
   */
  leaf(index) {
    const parent = this.id(1, Math.floor(index / FANOUT));
    return `${index + 1} 0 obj\n<< /Type /Page /Parent ${parent} 0 R /Contents ${this.stream} 0 R >>\nendobj\n`;
  }

  /**
   * Node `record`, counted over all levels bottom up.
   * @param {number} record
   */
  node(record) {
    let level = 1;
    let index = record;
    for (const nodes of this.levels) {
      if (index < nodes) {
        break;
      }
      index -= nodes;
      level += 1;
    }
    const top = level === this.levels.length;
    const above = top ? "/MediaBox [0 0 612 792] " : `/Parent ${this.id(level + 1, Math.floor(index / FANOUT))} 0 R `;
    const span = FANOUT ** level;
    const count = Math.min(index * span + span, this.leaves) - index * span;
    const children = level === 1 ? this.leaves : (this.levels[level - 2] ?? 0);
    let kids = "";
    for (let kid = index * FANOUT; kid < Math.min(index * FANOUT + FANOUT, children); kid += 1) {
      kids += `${this.id(level - 1, kid)} 0 R `;
    }
    return `${this.id(level, index)} 0 obj\n<< /Type /Pages ${above}/Count ${count} /Kids [${kids}] >>\nendobj\n`;
  }
}

/**
 * {@link LEAVES} pages in a page tree, then one `FlateDecode` stream of `decoded` bytes that is every page's contents,
 * after `pad` spaces; as the Rust memory tests build it.
 * @param {number} pad
 * @param {number} decoded
 * @returns {Virtual}
 */
function manySmallObjects(pad, decoded) {
  const tree = new Tree(LEAVES);
  const file = new Layout();
  file.text(`%PDF-2.0\n${" ".repeat(pad)}`);
  const leaves = file.push(records(LEAVES, LEAF, (index) => tree.leaf(index)));
  const nodes = file.push(records(tree.nodes, NODE, (record) => tree.node(record)));
  const data = flate(decoded);
  const stream = file.text(`${tree.stream} 0 obj\n<< /Filter /FlateDecode /Length ${data.size} >>\nstream\n`);
  file.push(data);
  file.text("\nendstream\nendobj\n");
  const catalog = file.text(`${tree.catalog} 0 obj\n<< /Type /Catalog /Pages ${tree.root} 0 R >>\nendobj\n`);
  const objects = tree.catalog + 1;
  /** @param {number} number */
  const offset = (number) => {
    if (number <= LEAVES) {
      return leaves + (number - 1) * LEAF;
    }
    if (number <= tree.root) {
      return nodes + (number - LEAVES - 1) * NODE;
    }
    return number === tree.stream ? stream : catalog;
  };
  const xref = file.text(`xref\n0 ${objects}\n`);
  file.push(
    records(objects, ENTRY, (number) =>
      number === 0 ? "0000000000 65535 f\r\n" : `${digits(offset(number))} 00000 n\r\n`,
    ),
  );
  file.text(`trailer\n<< /Root ${tree.catalog} 0 R /Size ${objects} >>\nstartxref\n${xref}\n%%EOF\n`);
  return { decoded, file };
}

/**
 * The file of `build` with its stream's decoded bytes and padding set so it is {@link SIZE} bytes, from `decoded`.
 * @param {(pad: number, decoded: number) => Virtual} build
 * @param {number} decoded
 */
function fit(build, decoded) {
  let data = decoded;
  let pad = 0;
  for (let tries = 0; tries < 4; tries += 1) {
    const made = build(pad, data);
    if (made.file.size === SIZE) {
      return made;
    }
    const blocksAndData = flate(data).size + pad + SIZE - made.file.size - ZLIB;
    const within = blocksAndData % FULL_BLOCK;
    data = Math.floor(blocksAndData / FULL_BLOCK) * BLOCK + Math.min(within, BLOCK - 1);
    pad = within - Math.min(within, BLOCK - 1);
  }
  throw new Error("no file of 1 GiB");
}

/**
 * Imports the package, has the server log `/mark`, then makes the first call.
 * @param {string} entry
 */
async function lazy(entry) {
  const api = await load(entry);
  await fetch("/mark");
  return api.libraryVersion();
}

/**
 * Has the package's worker fail to load; how a call fares then, and once a worker could load: its code if a PdfError.
 * @param {string} entry
 */
async function broken(entry) {
  const api = await load(entry);
  const { Worker } = globalThis;
  globalThis.Worker = class extends Worker {
    /**
     * @param {string | URL} _url
     * @param {WorkerOptions} [options]
     */
    constructor(_url, options) {
      super("/missing.js", options);
    }
  };
  /** @param {Promise<unknown>} call */
  const fate = async (call) =>
    Promise.race([
      call.then(
        () => "resolved",
        (/** @type {unknown} */ reason) => (reason instanceof api.PdfError ? reason.code : "not a PdfError"),
      ),
      new Promise((done) => {
        setTimeout(done, 10_000, "pending");
      }),
    ]);
  const first = await fate(api.libraryVersion());
  globalThis.Worker = Worker;
  return [first, await fate(api.libraryVersion())];
}

/**
 * Opens a Blob whose file is gone, as Python opens a missing file; the rejection's details.
 * @param {string} entry
 */
async function missing(entry) {
  const api = await load(entry);
  const file = await stored("tests/corpus/header/missing.pdf");
  const error = await api.Source.fromBlob(file)
    .then(async (source) => api.Document.open(source))
    .then(
      () => undefined,
      (/** @type {unknown} */ reason) => reason,
    );
  if (!(error instanceof api.PdfError)) {
    throw new Error(`not a PdfError: ${String(error)}`);
  }
  return { code: error.code, ioCause: error.ioCause };
}

/**
 * Runs the suites `names`; the failures and how many WebAssembly instances this thread made.
 * @param {string} entry
 * @param {string[]} names
 */
async function conformance(entry, names) {
  const instances = record();
  const api = await load(entry);
  const suites = await Promise.all(
    names.map(async (name) => {
      const text = await (await fetch(`/repo/tests/conformance/${name}`)).text();
      return /** @type {const} */ ([name, text]);
    }),
  );
  return { failures: await run(api, environment(api), suites), instances: instances.length };
}

/**
 * In this worker, counts the pages of a 1 GiB Blob; the count, its size and the WebAssembly memory.
 * @param {string} entry
 */
async function memory(entry) {
  const instances = record();
  const api = await load(entry);
  const file = huge();
  const count = await (await api.Document.open(await api.Source.fromBlob(file))).pageCount();
  const memories = instances.flatMap((instance) =>
    Object.values(instance.exports).filter((value) => value instanceof WebAssembly.Memory),
  );
  const bytes = memories.reduce((sum, used) => sum + used.buffer.byteLength, 0);
  return { bytes, count, memories: memories.length, size: file.size };
}

/**
 * In this worker, opens, splits and saves each virtual 1 GiB file of the memory rule, the parts to sinks that
 * discard their bytes; the bytes each part saved, the fewest it could, and the WebAssembly memory.
 * @param {string} entry
 */
async function splitting(entry) {
  const instances = record();
  const api = await load(entry);
  const sinks = /** @type {Sinks} */ (api.Sink);
  /** @type {[(pad: number, decoded: number) => Virtual, number, Split, (decoded: number) => number[]][]} */
  const files = [
    [fewLargeStreams, LARGE, { by: "Pages", count: 1 }, (last) => [3 * flate(LARGE).size + flate(last).size]],
    [
      manySmallObjects,
      2 ** 26,
      { by: "Ranges", starts: [0, LEAVES / 2] },
      (decoded) => [LEAVES / 2, LEAVES / 2].map((pages) => pages * PAGE_OBJECT.length + flate(decoded).size),
    ],
  ];
  const floors = [];
  const saved = [];
  for (const [build, decoded, options, floor] of files) {
    const made = fit(build, decoded);
    floors.push(floor(made.decoded));
    const opened = await api.Document.open(await api.Source.fromBlob(made.file.blob()));
    const document = /** @type {Splittable} */ (opened);
    const parts = [];
    for await (const part of await document.split(options)) {
      let bytes = 0;
      await part.save(
        await sinks.fromAppend((chunk) => {
          bytes += chunk.length;
        }),
      );
      parts.push(bytes);
    }
    saved.push(parts);
  }
  const memories = instances.flatMap((instance) =>
    Object.values(instance.exports).filter((value) => value instanceof WebAssembly.Memory),
  );
  const bytes = memories.reduce((sum, used) => sum + used.buffer.byteLength, 0);
  return { bytes, floors, memories: memories.length, saved };
}

/**
 * Runs `scenario` with `args` in a module worker this page starts.
 * @param {string} entry
 * @param {[string, unknown]} args
 */
async function worker(entry, [scenario, args]) {
  const started = new Worker(new URL("worker.mjs", import.meta.url), { type: "module" });
  try {
    /** @type {PromiseWithResolvers<Outcome>} */
    const done = Promise.withResolvers();
    started.addEventListener("message", (event) => {
      done.resolve(event.data);
    });
    started.addEventListener("error", (event) => {
      done.reject(new Error(`the worker failed: ${event.message}`));
    });
    started.postMessage({ args, entry, scenario });
    const result = await done.promise;
    if ("error" in result) {
      throw new Error(result.error);
    }
    return result.ok;
  } finally {
    started.terminate();
  }
}

/** @type {[string, (entry: string, args: never) => Promise<unknown>][]} */
const NAMED = [
  ["broken", broken],
  ["conformance", conformance],
  ["lazy", lazy],
  ["memory", memory],
  ["missing", missing],
  ["splitting", splitting],
  ["worker", worker],
];
const SCENARIOS = new Map(NAMED);

/**
 * `{ ok }` with the value of `scenario` with the package at `entry`, or `{ error }`.
 * @param {string} scenario
 * @param {string} entry
 * @param {unknown} args
 * @returns {Promise<Outcome>}
 */
export async function outcome(scenario, entry, args) {
  try {
    const perform = SCENARIOS.get(scenario);
    if (perform === undefined) {
      throw new Error(`unknown scenario ${scenario}`);
    }
    return { ok: await perform(entry, /** @type {never} */ (args)) };
  } catch (error) {
    return { error: error instanceof Error ? (error.stack ?? String(error)) : String(error) };
  }
}

/**
 * Runs `scenario` and posts its outcome to the test server.
 * @param {string} scenario
 * @param {string} entry
 * @param {unknown} args
 */
export async function main(scenario, entry, args) {
  const body = JSON.stringify(await outcome(scenario, entry, args));
  await fetch("/result", { body, method: "POST" });
}
