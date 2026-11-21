// StarlightPDF: the core in this thread, or on a page's main thread in a worker this package starts
// (.architecture-decisions/long-operations.md).
import type { Name, Signatures } from "./core.js";
import { type Callback, remote } from "./table.js";

/** Every failure; `null` when unknown. */
export class PdfError extends Error {
  readonly code: string;
  readonly ioCause: string | null;
  readonly number: number;
  readonly objectNumber: number | null;
  readonly offset: number | null;
  readonly operation: string;

  constructor(
    details: Pick<PdfError, "code" | "ioCause" | "message" | "number" | "objectNumber" | "offset" | "operation">,
  ) {
    super(details.message);
    this.code = details.code;
    this.ioCause = details.ioCause;
    this.number = details.number;
    this.objectNumber = details.objectNumber;
    this.offset = details.offset;
    this.operation = details.operation;
  }

  override toString(): string {
    return `${this.operation}: ${this.message}`;
  }
}

/** A failure's details as the core gives them. */
interface Details extends Omit<PdfError, "offset" | "toString" | keyof Error> {
  readonly message: string;
  readonly offset: bigint | null;
}

/** Whether `reason` is the core's details of a failure. */
function isDetails(reason: unknown): reason is Details {
  return typeof reason === "object" && reason !== null && !(reason instanceof Error) && "code" in reason;
}

/** The package's worker, or why it failed; once it failed, every call rejects. */
let worker: Worker | string | undefined;
/** The rejections, given why the worker failed, of calls it has yet to answer. */
const pending = new Set<(why: string) => void>();

/** `name`'s failure as the core's `Io` because the package failed `why`. */
function unloaded<K extends Name>(name: K, arg: Signatures[K][0], why: string): PdfError {
  const operations: Record<Name, string> = {
    appendSink: "Sink.fromAppend",
    boundary: "Page.boundary",
    catalog: "Document.catalog",
    create: "Document.create",
    drawImage: "Page.drawImage",
    droppedWarnings: "Document.droppedWarnings",
    encryption: "Document.encryption",
    free: "free",
    headerVersion: "headerVersion",
    infoDictionary: "Document.infoDictionary",
    insertBlankPage: "Document.insertBlankPage",
    libraryVersion: "libraryVersion",
    nextPart: "Documents.next",
    nextWarning: "Deviations.next",
    object: "Document.object",
    open: "Document.open",
    page: "Document.page",
    pageCount: "Document.pageCount",
    policy: "ResourcePolicy.new",
    rotation: "Page.rotation",
    save: "Document.save",
    setInfoDictionary: "Document.setInfoDictionary",
    sink: "Sink.inMemory",
    source: arg instanceof Blob ? "Source.fromBlob" : "Source.fromBytes",
    split: "Document.split",
    streamData: "Document.streamData",
    takeBytes: "Sink.takeBytes",
    unsupported: typeof arg === "object" && "operation" in arg ? arg.operation : "",
    warnings: "Document.warnings",
  };
  return new PdfError({
    code: "Io",
    ioCause: "Other",
    message: why,
    number: 10, // The core's `ErrorCode::Io`.
    objectNumber: null,
    offset: null,
    operation: operations[name],
  });
}

/** `fn` as the core takes it, and what posting it moves (.architecture-decisions/long-operations.md). */
async function passed<A extends unknown[]>(
  fn: (...args: A) => boolean,
  thrown: unknown[],
  operation: string,
): Promise<[Callback<A>, Transferable[]]> {
  const guarded = (...args: A): boolean => {
    try {
      return fn(...args);
    } catch (error) {
      thrown.push(error);
      throw error;
    }
  };
  if (typeof window === "undefined") {
    return [guarded, []];
  }
  if (!crossOriginIsolated) {
    return call("unsupported", {
      message: "a callback on a page's main thread needs cross-origin isolation",
      operation,
    });
  }
  const made = remote(guarded);
  return [made, [made.port]];
}

/** What `result` gives, or if it fails, the first of `thrown` or its error. */
async function rethrown<T>(thrown: unknown[], result: Promise<T>): Promise<T> {
  try {
    return await result;
  } catch (error) {
    throw thrown.length > 0 ? thrown[0] : error;
  }
}

/** Starts the worker, or why it failed; if it fails later, rejects every pending call. */
function start(): Worker | string {
  let started: Worker;
  try {
    started = new Worker(new URL("worker.js", import.meta.url), { type: "module" });
  } catch (error) {
    return `the StarlightPDF worker failed to start: ${String(error)}`;
  }
  started.addEventListener("error", () => {
    started.terminate();
    const why = "the StarlightPDF worker failed";
    worker = why;
    for (const reject of pending) {
      reject(why);
    }
    pending.clear();
  });
  return started;
}

/** Runs `name` on `arg` in the core, moving `transfer` to a worker; every failure rejects with `PdfError`. */
async function call<K extends Name>(
  name: K,
  arg: Signatures[K][0],
  transfer: readonly Transferable[] = [],
): Promise<Signatures[K][1]> {
  try {
    if (typeof window === "undefined") {
      const core = await import("./core.js").catch((error: unknown) => {
        throw unloaded(name, arg, `the StarlightPDF WebAssembly failed to load: ${String(error)}`);
      });
      return core.perform(name, arg);
    }
    worker ??= start();
    if (typeof worker === "string") {
      throw unloaded(name, arg, worker);
    }
    const { port1, port2 } = new MessageChannel();
    type Reply = { ok: Signatures[K][1] } | { error: unknown } | { unloaded: string };
    const reply = Promise.withResolvers<Reply>();
    const reject = (why: string): void => {
      reply.reject(unloaded(name, arg, why));
    };
    port1.onmessage = (event: MessageEvent<Reply>) => {
      pending.delete(reject);
      port1.close();
      reply.resolve(event.data);
    };
    worker.postMessage({ arg, name, port: port2 }, [...transfer, port2]);
    // After posting, so a failed post leaves nothing pending; the worker's `error` event comes later.
    pending.add(reject);
    const replied = await reply.promise;
    if ("unloaded" in replied) {
      throw unloaded(name, arg, replied.unloaded);
    }
    if ("error" in replied) {
      throw replied.error;
    }
    return replied.ok;
  } catch (error) {
    throw isDetails(error)
      ? new PdfError({ ...error, offset: error.offset === null ? null : Number(error.offset) })
      : error;
  }
}

/** Each object's handle in the core. */
const handles = new WeakMap<object, number>();
/** Frees an object's handle once the object is collected. */
const collected = new FinalizationRegistry<number>((handle) => {
  // A failed worker holds nothing, and its call rejects.
  void call("free", handle).catch(() => undefined);
});

/** `made`, holding `handle`. */
function hold<T extends object>(made: T, handle: number): T {
  handles.set(made, handle);
  collected.register(made, handle, made);
  return made;
}

/** Frees the handle `made` holds now, not when collected. */
async function release(made: object): Promise<void> {
  const handle = handles.get(made);
  if (handle !== undefined) {
    handles.delete(made);
    collected.unregister(made);
    await call("free", handle).catch(() => undefined);
  }
}

/** The core's unsigned integer types' largest values. */
const U16 = 0xffff;
const U32 = 0xffff_ffff;

/** `value`, which the core takes as an unsigned integer up to `max`; a `RangeError` if it is not one, so it never wraps. */
function unsigned<T extends number | undefined>(value: T, max: number, name: string): T {
  if (value !== undefined && !(Number.isSafeInteger(value) && value >= 0 && value <= max)) {
    throw new RangeError(`${name} must be an integer from 0 to ${String(max)}, not ${String(value)}`);
  }
  return value;
}

/** `id`, checked by `unsigned`. */
function objectId(id: ObjectId): ObjectId {
  return { generation: unsigned(id.generation, U16, "generation"), number: unsigned(id.number, U32, "number") };
}

/** Input with a size and reads at an offset. */
export class Source {
  private constructor() {}

  /** A source over a Blob, read a part at a time; in a browser. */
  static async fromBlob(blob: Blob): Promise<Source> {
    return hold(new Source(), await call("source", blob));
  }

  /**
   * A source over `bytes`, not copied, so they must stay unchanged while it or a document opened from it lives;
   * on a page's main thread, copied once to the package's worker.
   */
  static async fromBytes(bytes: Uint8Array): Promise<Source> {
    if (typeof window === "undefined") {
      return hold(new Source(), await call("source", bytes));
    }
    // Of the view only: posting `bytes` would copy its whole buffer.
    const copy = bytes.slice();
    return hold(new Source(), await call("source", copy, [copy.buffer]));
  }
}

/** Limits for `ResourcePolicy.create`; unset means a safe library default. */
export interface ResourcePolicyOptions {
  /** Bytes one stream may decode to. */
  readonly decodedLimit?: number;
  /** Objects per document. */
  readonly objectLimit?: number;
  /** Warnings recorded per document. */
  readonly warningLimit?: number;
}

/** Limits shared by the documents opened with it. */
export class ResourcePolicy {
  private constructor() {}

  /** A policy with these limits. */
  static async create(options: ResourcePolicyOptions): Promise<ResourcePolicy> {
    unsigned(options.decodedLimit, Number.MAX_SAFE_INTEGER, "decodedLimit");
    unsigned(options.objectLimit, Number.MAX_SAFE_INTEGER, "objectLimit");
    unsigned(options.warningLimit, U32, "warningLimit");
    return hold(new ResourcePolicy(), await call("policy", options));
  }
}

/** Options for `Document.open`. */
export interface OpenOptions {
  /** The user or owner password; unset is the empty one. */
  readonly password?: string;
  /** Fail with `Malformed` on any deviation instead of tolerating it. */
  readonly strict?: boolean;
}

/** What each sink's append callback threw during the current save. */
const failures = new WeakMap<Sink, unknown[]>();

/** Output that only appends. */
export class Sink {
  private constructor() {}

  /**
   * A sink over `append`, called on this thread with a copy of each chunk as it is written; `save` rethrows what it
   * throws. On a page's main thread, needs cross-origin isolation.
   */
  static async fromAppend(append: (chunk: Uint8Array) => void): Promise<Sink> {
    const thrown: unknown[] = [];
    const appended = (chunk: Uint8Array): boolean => {
      append(chunk);
      return true;
    };
    const [made, transfer] = await passed(appended, thrown, "Sink.fromAppend");
    const sink = hold(new Sink(), await call("appendSink", made, transfer));
    failures.set(sink, thrown);
    return sink;
  }

  /** A sink that holds its bytes in memory, for `takeBytes`. */
  static async inMemory(): Promise<Sink> {
    return hold(new Sink(), await call("sink", undefined));
  }

  /** Takes the bytes appended so far, leaving the sink empty; copies them once out of WebAssembly memory. */
  async takeBytes(): Promise<Uint8Array> {
    return call("takeBytes", handles.get(this) ?? 0);
  }
}

/** An indirect object's number and generation. */
export interface ObjectId {
  readonly generation: number;
  readonly number: number;
}

/** A rectangle in default user space, y up. */
export interface Rect {
  readonly height: number;
  readonly width: number;
  readonly x: number;
  readonly y: number;
}

/** A page boundary (ISO 32000-2:2020, 14.11.2). */
export type Boundary = "Art" | "Bleed" | "Crop" | "Media" | "Trim";

/** How a document is protected. */
export interface Encryption {
  /** Certificate-based rather than password-based. */
  readonly certificate: boolean;
  /** The algorithm: `Aes128`, `Aes256`, `Rc4` or `Unencrypted`. */
  readonly cipher: string;
  /** What is denied, as `Permission` names. */
  readonly restrictions: readonly string[];
}

/** Called with work done of the total; `false` cancels. */
export type Progress = (done: number, total: number) => boolean;

/** Options for `Document.save`; unset is the default. */
export interface SaveOptions {
  /** Let `standards` or `version` drop what they forbid. */
  readonly allowLoss?: boolean;
  /** Allow invalidating signatures. */
  readonly breakSignatures?: boolean;
  /** A `Cipher` name; unset keeps the protection, `Unencrypted` removes it. */
  readonly cipher?: string;
  /** Convert every color to this `ColorSpace`. */
  readonly colorSpace?: string;
  /** Recompress images as JPEG at this quality, 1 to 100. */
  readonly imageQuality?: number;
  /** Downsample images above this many pixels per inch. */
  readonly imageResolution?: number;
  /** A `SaveMode` name; unset is `Rewrite`. */
  readonly mode?: string;
  /** `Optimization` names. */
  readonly optimizations?: readonly string[];
  readonly ownerPassword?: string;
  /** Certificates to encrypt for, instead of passwords. */
  readonly recipients?: Uint8Array;
  /** `Permission` names the user password denies. */
  readonly restrictions?: readonly string[];
  /** `Standard` names to conform to. */
  readonly standards?: readonly string[];
  readonly userPassword?: string;
  /** The PDF version to write, `[major, minor]`. */
  readonly version?: readonly [number, number];
}

/** What a save did. */
export interface SaveReport {
  /** Fields of the signatures it invalidated. */
  readonly brokenSignatures: readonly string[];
  /** Bytes written. */
  readonly length: number;
}

/** Options for `Document.split`. */
export interface SplitOptions {
  /** Leave out what a part cannot keep instead of failing with `LossRefused`. */
  readonly allowLoss?: boolean;
  /** Where parts start; unset is `Pages`. */
  readonly by?: "Bookmarks" | "Pages" | "Ranges" | "Size";
  /** Pages per part, for `Pages`. */
  readonly count?: number;
  /** First pages of the parts, for `Ranges`. */
  readonly starts?: readonly number[];
}

/** The document information dictionary (ISO 32000-2:2020, 14.3.3); an absent entry empty, dates ISO 8601. */
export interface InfoDictionary {
  readonly author: string;
  readonly created: string;
  readonly creator: string;
  readonly keywords: string;
  readonly modified: string;
  readonly producer: string;
  readonly subject: string;
  readonly title: string;
}

/** Options for `Document.insertBlankPage`; sizes in points, unset A4. */
export interface InsertBlankPageOptions {
  readonly height?: number;
  /** The new page's position; unset after the last page. */
  readonly index?: number;
  readonly width?: number;
}

/** Options for `Page.drawImage`; points from the top-left of the displayed page. */
export interface DrawImageOptions {
  /** An `ImageFormat` name; unset detects it. */
  readonly format?: string;
  /** Which image of a multi-image file. */
  readonly frame?: number;
  /** Unset: from `width`, or the image's height at 72 DPI. */
  readonly height?: number;
  readonly ignoreOrientation?: boolean;
  /** Unset: 1. */
  readonly opacity?: number;
  /** Unset: from `height`, or the image's width at 72 DPI. */
  readonly width?: number;
  readonly x?: number;
  readonly y?: number;
}

/** A deviation from a standard; `null` when unknown. */
export interface Deviation {
  /** Standard and clause, e.g. `ISO 32000-2:2020, 7.5.4`. */
  readonly clause: string;
  readonly message: string;
  readonly objectNumber: number | null;
  /** The byte offset in the source. */
  readonly offset: number | null;
  /** The page index. */
  readonly page: number | null;
}

/** A page holding `handle`; set by `Page`, whose constructor is private. */
let newPage: (handle: number) => Page;

/** A page. */
export class Page {
  static {
    newPage = (handle) => hold(new Page(), handle);
  }

  private constructor() {}

  /** A boundary: inherited, defaulted, clipped to the media box and normalized. */
  async boundary(options: { readonly kind?: Boundary } = {}): Promise<Rect> {
    return call("boundary", { kind: options.kind ?? "Crop", page: handles.get(this) ?? 0 });
  }

  /** Draws `image`, with soft mask `mask`, in `layer`; no layer can exist yet. */
  async drawImage(
    image: Source,
    mask: Source | null = null,
    _layer: null = null,
    options: DrawImageOptions = {},
  ): Promise<void> {
    return call("drawImage", {
      image: handles.get(image) ?? 0,
      mask: mask === null ? null : (handles.get(mask) ?? 0),
      options,
      page: handles.get(this) ?? 0,
    });
  }

  /** The rotation in degrees: 0, 90, 180 or 270. */
  async rotation(): Promise<number> {
    return call("rotation", handles.get(this) ?? 0);
  }
}

/** An open PDF. */
export class Document {
  private constructor() {}

  /** A new document with no pages, with `policy`, or the default one, of PDF `version` `[major, minor]`, unset 2.0. */
  static async create(
    policy?: ResourcePolicy | null,
    options: { readonly version?: readonly [number, number] } = {},
  ): Promise<Document> {
    const arg = { options, policy: policy === undefined || policy === null ? null : (handles.get(policy) ?? 0) };
    return hold(new Document(), await call("create", arg));
  }

  /** Opens `source` with `policy`, or the default one. */
  static async open(source: Source, policy?: ResourcePolicy | null, options: OpenOptions = {}): Promise<Document> {
    const arg = {
      options,
      policy: policy === undefined || policy === null ? null : (handles.get(policy) ?? 0),
      source: handles.get(source) ?? 0,
    };
    return hold(new Document(), await call("open", arg));
  }

  /** The catalog's id. */
  async catalog(): Promise<ObjectId> {
    return call("catalog", handles.get(this) ?? 0);
  }

  /** The number of deviations not kept in `warnings`. */
  async droppedWarnings(): Promise<number> {
    return call("droppedWarnings", handles.get(this) ?? 0);
  }

  /** How the document is protected. */
  async encryption(): Promise<Encryption> {
    return call("encryption", handles.get(this) ?? 0);
  }

  /** The document information dictionary. */
  async infoDictionary(): Promise<InfoDictionary> {
    return call("infoDictionary", handles.get(this) ?? 0);
  }

  /** Inserts a blank page. */
  async insertBlankPage(options: InsertBlankPageOptions = {}): Promise<Page> {
    return newPage(await call("insertBlankPage", { document: handles.get(this) ?? 0, options }));
  }

  /** An object as PDF syntax; the string `"null"` for a free or missing one. */
  async object(options: { readonly id: ObjectId }): Promise<string> {
    return call("object", { document: handles.get(this) ?? 0, id: objectId(options.id) });
  }

  /** The page at `index`. */
  async page(options: { readonly index: number }): Promise<Page> {
    return newPage(
      await call("page", { document: handles.get(this) ?? 0, index: unsigned(options.index, U32, "index") }),
    );
  }

  /** The number of pages. */
  async pageCount(): Promise<number> {
    return call("pageCount", handles.get(this) ?? 0);
  }

  /**
   * Writes the document to `sink`, calling `progress` on this thread; rethrows what a callback throws. On a page's
   * main thread, `progress` needs cross-origin isolation.
   */
  async save(sink: Sink, progress: Progress | null = null, options: SaveOptions = {}): Promise<SaveReport> {
    const thrown = failures.get(sink) ?? [];
    thrown.length = 0;
    const [made, transfer] = progress === null ? [null, []] : await passed(progress, thrown, "Document.save");
    const arg = { document: handles.get(this) ?? 0, options, progress: made, sink: handles.get(sink) ?? 0 };
    return rethrown(thrown, call("save", arg, transfer));
  }

  /** Sets the given entries; an empty one removes its entry. */
  async setInfoDictionary(options: { readonly [K in keyof InfoDictionary]?: string }): Promise<void> {
    return call("setInfoDictionary", { document: handles.get(this) ?? 0, options });
  }

  /** The parts, in order, one at a time; after this document's next edit, they and the iteration fail `StaleHandle`. */
  split(options: SplitOptions): AsyncIterable<Document> {
    const arg = { document: handles.get(this) ?? 0, options };
    return {
      async *[Symbol.asyncIterator](): AsyncGenerator<Document> {
        const cursor = hold({}, await call("split", arg));
        try {
          for (;;) {
            const part = await call("nextPart", handles.get(cursor) ?? 0);
            if (part === null) {
              return;
            }
            yield hold(new Document(), part);
          }
        } finally {
          await release(cursor);
        }
      },
    };
  }

  /** Writes a stream's stored data to `sink`, or with `decode` its decoded data; rethrows what `sink` throws. */
  async streamData(sink: Sink, options: { readonly decode?: boolean; readonly id: ObjectId }): Promise<void> {
    const arg = {
      decode: options.decode ?? false,
      document: handles.get(this) ?? 0,
      id: objectId(options.id),
      sink: handles.get(sink) ?? 0,
    };
    const thrown = failures.get(sink) ?? [];
    thrown.length = 0;
    return rethrown(thrown, call("streamData", arg));
  }

  /** The deviations tolerated so far, up to the policy's limit. */
  async warnings(): Promise<AsyncIterable<Deviation>> {
    const cursor = hold({}, await call("warnings", handles.get(this) ?? 0));
    return {
      async *[Symbol.asyncIterator](): AsyncGenerator<Deviation> {
        for (;;) {
          const deviation = await call("nextWarning", handles.get(cursor) ?? 0);
          if (deviation === null) {
            return;
          }
          yield deviation;
        }
      },
    };
  }
}

/** The PDF version in the header of `bytes`, as `[major, minor]`; copies only the header's 9 bytes. */
export async function headerVersion(bytes: Uint8Array): Promise<[number, number]> {
  // `%PDF-x.y` and an end-of-line marker (ISO 32000-2:2020, 7.5.2); `slice`, so a page posts no more.
  return call("headerVersion", bytes.slice(0, 9));
}

/** The library's version. */
export async function libraryVersion(): Promise<string> {
  return call("libraryVersion", undefined);
}
