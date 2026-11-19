// The browser side of the tests: scenarios on a page or, through `worker`, in a worker.
import { run } from "./runner.mjs";

/**
 * @import { Api } from "./api.mts"
 * @import { Environment } from "./runner.mjs"
 * @typedef {{ ok: unknown } | { error: string }} Outcome
 */

/** Bytes in a mebibyte. */
const MIB = 1 << 20;

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
