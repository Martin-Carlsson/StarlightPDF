// StarlightPDF: the core in this thread, or on a page's main thread in a worker this package starts
// (.architecture-decisions/long-operations.md).
import type { Name, Signatures } from "./core.js";

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
    free: "free",
    headerVersion: "headerVersion",
    libraryVersion: "libraryVersion",
    open: "Document.open",
    pageCount: "Document.pageCount",
    source: arg instanceof Blob ? "Source.fromBlob" : "Source.fromBytes",
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
  collected.register(made, handle);
  return made;
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

/** An open PDF. */
export class Document {
  private constructor() {}

  /** Opens `source`, checking only its header. */
  static async open(source: Source): Promise<Document> {
    return hold(new Document(), await call("open", handles.get(source) ?? 0));
  }

  /** The number of pages. */
  async pageCount(): Promise<number> {
    return call("pageCount", handles.get(this) ?? 0);
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
