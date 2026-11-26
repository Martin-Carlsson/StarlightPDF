// The core in this thread, its objects held by handle.
import type {
  AddAnnotationOptions,
  AnnotationInfo,
  Boundary,
  Deviation,
  DrawImageOptions,
  Encryption,
  InfoDictionary,
  InsertBlankPageOptions,
  ObjectId,
  OpenOptions,
  Rect,
  RenderOptions,
  ResourcePolicyOptions,
  SaveOptions,
  SaveReport,
  SplitOptions,
} from "./index.js";
import init, {
  Annotation,
  Annotations,
  Deviations,
  Document,
  Documents,
  Page,
  ResourcePolicy,
  Sink,
  Source,
  capabilities,
  headerVersion,
  libraryVersion,
  unsupported,
} from "./wasm/starlightpdf_javascript.js";
import { add, appends, type Callback, inputs, updates } from "./table.js";

/** Each operation's argument and result. */
export interface Signatures {
  addAnnotation: [{ data: number | null; options: AddAnnotationOptions; page: number }, number];
  annotationInfo: [number, AnnotationInfo];
  annotations: [number, number];
  appendSink: [Callback<[Uint8Array]>, number];
  applyRedactions: [
    { document: number; options: { readonly pages?: readonly number[] }; progress: Callback<[number, number]> | null },
    undefined,
  ];
  boundary: [{ kind: Boundary; page: number }, Rect];
  capabilities: [undefined, string[]];
  catalog: [number, ObjectId];
  create: [{ options: { readonly version?: readonly [number, number] }; policy: number | null }, number];
  deleteAnnotation: [number, undefined];
  drawImage: [{ image: number; mask: number | null; options: DrawImageOptions; page: number }, undefined];
  droppedWarnings: [number, number];
  encryption: [number, Encryption];
  free: [number, undefined];
  headerVersion: [Uint8Array, [number, number]];
  infoDictionary: [number, InfoDictionary];
  insertBlankPage: [{ document: number; options: InsertBlankPageOptions }, number];
  libraryVersion: [undefined, string];
  nextAnnotation: [number, number | null];
  nextPart: [number, number | null];
  nextWarning: [number, Deviation | null];
  object: [{ document: number; id: ObjectId }, string];
  open: [{ options: OpenOptions; policy: number | null; source: number }, number];
  page: [{ document: number; index: number }, number];
  pageCount: [number, number];
  policy: [ResourcePolicyOptions, number];
  render: [
    { options: RenderOptions; page: number; progress: Callback<[number, number]> | null; sink: number },
    undefined,
  ];
  rotation: [number, number];
  save: [
    { document: number; options: SaveOptions; progress: Callback<[number, number]> | null; sink: number },
    SaveReport,
  ];
  setInfoDictionary: [{ document: number; options: { readonly [K in keyof InfoDictionary]?: string } }, undefined];
  sink: [undefined, number];
  source: [Blob | Uint8Array, number];
  split: [{ document: number; options: SplitOptions }, number];
  streamData: [{ decode: boolean; document: number; id: ObjectId; sink: number }, undefined];
  takeBytes: [number, Uint8Array];
  unsupported: [{ message: string; operation: string }, never];
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
const parts = new Map<number, Documents>();
const annotations = new Map<number, Annotation>();
const listings = new Map<number, Annotations>();
const held: Map<number, { free(): void }>[] = [
  sources,
  documents,
  pages,
  policies,
  sinks,
  cursors,
  parts,
  annotations,
  listings,
];
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
  addAnnotation({ data, options, page }) {
    const shared = data === null ? undefined : get(sources, data).share();
    return hold(annotations, get(pages, page).addAnnotation(shared, options));
  },
  annotationInfo(annotation) {
    return get(annotations, annotation).info();
  },
  annotations(page) {
    return hold(listings, get(pages, page).annotations());
  },
  appendSink(callback) {
    return hold(sinks, Sink.fromAppend(add(appends, callback)));
  },
  applyRedactions({ document, options, progress }) {
    const redacted = get(documents, document);
    // Added last, as the core forgets it whatever then fails.
    redacted.applyRedactions(progress === null ? undefined : add(updates, progress), options);
  },
  boundary({ kind, page }) {
    return get(pages, page).boundary(kind);
  },
  capabilities() {
    return capabilities();
  },
  catalog(document) {
    return get(documents, document).catalog();
  },
  create({ options, policy }) {
    const shared = policy === null ? undefined : get(policies, policy).share();
    return hold(documents, Document.create(shared, options));
  },
  deleteAnnotation(annotation) {
    get(annotations, annotation).delete();
  },
  drawImage({ image, mask, options, page }) {
    const shared = mask === null ? undefined : get(sources, mask).share();
    get(pages, page).drawImage(get(sources, image), shared, options);
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
  infoDictionary(document) {
    return get(documents, document).infoDictionary();
  },
  insertBlankPage({ document, options }) {
    return hold(pages, get(documents, document).insertBlankPage(options));
  },
  libraryVersion() {
    return libraryVersion();
  },
  nextAnnotation(cursor) {
    const annotation = get(listings, cursor).next();
    return annotation === undefined ? null : hold(annotations, annotation);
  },
  nextPart(cursor) {
    const part = get(parts, cursor).next();
    return part === undefined ? null : hold(documents, part);
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
  render({ options, page, progress, sink }) {
    const [shown, into] = [get(pages, page), get(sinks, sink)];
    // Added last, as the core forgets it whatever then fails.
    shown.render(into, progress === null ? undefined : add(updates, progress), options);
  },
  rotation(page) {
    return get(pages, page).rotation();
  },
  save({ document, options, progress, sink }) {
    const [saved, into] = [get(documents, document), get(sinks, sink)];
    // Added last, as the core forgets it whatever then fails.
    const report = saved.save(into, progress === null ? undefined : add(updates, progress), options);
    return { brokenSignatures: report.brokenSignatures, length: Number(report.length) };
  },
  setInfoDictionary({ document, options }) {
    get(documents, document).setInfoDictionary(options);
  },
  sink() {
    return hold(sinks, Sink.inMemory());
  },
  source(input) {
    const size = BigInt(input instanceof Blob ? input.size : input.length);
    return hold(sources, Source.fromTable(add(inputs, input), size));
  },
  split({ document, options }) {
    return hold(parts, get(documents, document).split(options));
  },
  streamData({ decode, document, id, sink }) {
    get(documents, document).streamData(get(sinks, sink), id.number, id.generation, decode);
  },
  takeBytes(sink) {
    return get(sinks, sink).takeBytes();
  },
  unsupported({ message, operation }) {
    throw unsupported(operation, message);
  },
  warnings(document) {
    return hold(cursors, get(documents, document).warnings());
  },
};

/** Runs operation `name` on `arg`. */
export function perform<K extends Name>(name: K, arg: Signatures[K][0]): Signatures[K][1] {
  return operations[name](arg);
}
