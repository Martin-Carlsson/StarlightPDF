// The core in this thread, its objects held by handle.
import type { Boundary, Deviation, Encryption, ObjectId, OpenOptions, Rect, ResourcePolicyOptions } from "./index.js";
import init, {
  Deviations,
  Document,
  Page,
  ResourcePolicy,
  Sink,
  Source,
  headerVersion,
  libraryVersion,
} from "./wasm/starlightpdf_javascript.js";
import { add } from "./table.js";

/** Each operation's argument and result. */
export interface Signatures {
  boundary: [{ kind: Boundary; page: number }, Rect];
  catalog: [number, ObjectId];
  droppedWarnings: [number, number];
  encryption: [number, Encryption];
  free: [number, undefined];
  headerVersion: [Uint8Array, [number, number]];
  libraryVersion: [undefined, string];
  nextWarning: [number, Deviation | null];
  object: [{ document: number; id: ObjectId }, string];
  open: [{ options: OpenOptions; policy: number | null; source: number }, number];
  page: [{ document: number; index: number }, number];
  pageCount: [number, number];
  policy: [ResourcePolicyOptions, number];
  rotation: [number, number];
  sink: [undefined, number];
  source: [Blob | Uint8Array, number];
  streamData: [{ decode: boolean; document: number; id: ObjectId; sink: number }, undefined];
  takeBytes: [number, Uint8Array];
  warnings: [number, number];
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
const pages = new Map<number, Page>();
const policies = new Map<number, ResourcePolicy>();
const sinks = new Map<number, Sink>();
const cursors = new Map<number, Deviations>();
const held: Map<number, { free(): void }>[] = [sources, documents, pages, policies, sinks, cursors];
let last = 0;

/** Holds `made` in `kind`; its handle. */
function hold<T>(kind: Map<number, T>, made: T): number {
  last += 1;
  kind.set(last, made);
  return last;
}

/** The object `handle` holds in `kind`. */
function get<T>(kind: Map<number, T>, handle: number): T {
  const found = kind.get(handle);
  if (found === undefined) {
    throw new TypeError(`no object ${handle}`);
  }
  return found;
}

const operations: { [K in Name]: (arg: Signatures[K][0]) => Signatures[K][1] } = {
  boundary({ kind, page }) {
    return get(pages, page).boundary(kind);
  },
  catalog(document) {
    return get(documents, document).catalog();
  },
  droppedWarnings(document) {
    return Number(get(documents, document).droppedWarnings());
  },
  encryption(document) {
    return get(documents, document).encryption();
  },
  free(handle) {
    for (const kind of held) {
      kind.get(handle)?.free();
      kind.delete(handle);
    }
  },
  headerVersion(bytes) {
    const [major = 0, minor = 0] = headerVersion(bytes);
    return [major, minor];
  },
  libraryVersion() {
    return libraryVersion();
  },
  nextWarning(cursor) {
    const deviation = get(cursors, cursor).next();
    return deviation && { ...deviation, offset: deviation.offset === null ? null : Number(deviation.offset) };
  },
  object({ document, id }) {
    return get(documents, document).object(id.number, id.generation);
  },
  open({ options, policy, source }) {
    const shared = policy === null ? undefined : get(policies, policy).share();
    return hold(
      documents,
      Document.open(get(sources, source), shared, options.password ?? "", options.strict ?? false),
    );
  },
  page({ document, index }) {
    return hold(pages, get(documents, document).page(index));
  },
  pageCount(document) {
    return get(documents, document).pageCount();
  },
  policy({ decodedLimit, objectLimit, warningLimit }) {
    const big = (limit: number | undefined): bigint | undefined => (limit === undefined ? undefined : BigInt(limit));
    return hold(policies, ResourcePolicy.create(big(decodedLimit), big(objectLimit), warningLimit));
  },
  rotation(page) {
    return get(pages, page).rotation();
  },
  sink() {
    return hold(sinks, Sink.inMemory());
  },
  source(input) {
    const size = BigInt(input instanceof Blob ? input.size : input.length);
    return hold(sources, Source.fromTable(add(input), size));
  },
  streamData({ decode, document, id, sink }) {
    get(documents, document).streamData(get(sinks, sink), id.number, id.generation, decode);
  },
  takeBytes(sink) {
    return get(sinks, sink).takeBytes();
  },
  warnings(document) {
    return hold(cursors, get(documents, document).warnings());
  },
};

/** Runs operation `name` on `arg`. */
export function perform<K extends Name>(name: K, arg: Signatures[K][0]): Signatures[K][1] {
  return operations[name](arg);
}
