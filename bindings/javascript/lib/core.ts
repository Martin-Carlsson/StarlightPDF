// The core in this thread, its objects held by handle.
import init, { Document, Source, headerVersion, libraryVersion } from "./wasm/starlightpdf_javascript.js";
import { add } from "./table.js";

/** Each operation's argument and result. */
export interface Signatures {
  free: [number, undefined];
  headerVersion: [Uint8Array, [number, number]];
  libraryVersion: [undefined, string];
  open: [number, number];
  pageCount: [number, number];
  source: [Blob | Uint8Array, number];
}

/** An operation's name. */
export type Name = keyof Signatures;

const url = new URL("wasm/starlightpdf_javascript_bg.wasm", import.meta.url);
// Node.js fetches no file: URLs.
await init({
  module_or_path: url.protocol === "file:" ? (await import("node:fs/promises")).readFile(url) : url,
});

const sources = new Map<number, Source>();
const documents = new Map<number, Document>();
let last = 0;

/** A new handle. */
function next(): number {
  last += 1;
  return last;
}

/** The object `handle` holds in `held`. */
function get<T>(held: Map<number, T>, handle: number): T {
  const found = held.get(handle);
  if (found === undefined) {
    throw new TypeError(`no object ${handle}`);
  }
  return found;
}

const operations: { [K in Name]: (arg: Signatures[K][0]) => Signatures[K][1] } = {
  free(handle) {
    (sources.get(handle) ?? documents.get(handle))?.free();
    sources.delete(handle);
    documents.delete(handle);
  },
  headerVersion(bytes) {
    const [major = 0, minor = 0] = headerVersion(bytes);
    return [major, minor];
  },
  libraryVersion() {
    return libraryVersion();
  },
  open(source) {
    const handle = next();
    documents.set(handle, Document.open(get(sources, source)));
    return handle;
  },
  pageCount(document) {
    return get(documents, document).pageCount();
  },
  source(input) {
    const handle = next();
    sources.set(handle, Source.fromTable(add(input), BigInt(input instanceof Blob ? input.size : input.length)));
    return handle;
  },
};

/** Runs operation `name` on `arg`. */
export function perform<K extends Name>(name: K, arg: Signatures[K][0]): Signatures[K][1] {
  return operations[name](arg);
}
