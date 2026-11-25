// Inputs the core reads and callbacks it calls, by id; the WebAssembly glue imports `append`, `read`, `release` and
// `update`.

/** Reads a Blob synchronously; workers only (File API, FileReaderSync). */
declare const FileReaderSync: new () => { readAsArrayBuffer(blob: Blob): ArrayBuffer };

/**
 * A callback on a page's main thread, which answers a call posted to `port` in `cells`: 1 in the first once
 * answered, the second 1 to go on, 0 to stop or 2 for a throw.
 */
export interface Remote {
  readonly cells: Int32Array;
  readonly port: MessagePort;
}

/** A callback with `A`, returning whether to go on; or one on a page's main thread. */
export type Callback<A extends unknown[]> = ((...args: A) => boolean) | Remote;

/** The answer for a throw. */
const THREW = 2;

/** Each kind of entry, by id. */
export const inputs = new Map<number, Blob | Uint8Array>();
export const appends = new Map<number, Callback<[Uint8Array]>>();
export const updates = new Map<number, Callback<[number, number]>>();
let last = 0;

/** Adds `entry` to `kind`; its id. */
export function add<T>(kind: Map<number, T>, entry: T): number {
  last += 1;
  kind.set(last, entry);
  return last;
}

/** `fn` for a worker to call from this thread, a page's main thread; needs cross-origin isolation. */
export function remote<A extends unknown[]>(fn: (...args: A) => boolean): Remote {
  const { port1, port2 } = new MessageChannel();
  const cells = new Int32Array(new SharedArrayBuffer(8));
  const answer = (args: A): number => {
    try {
      return fn(...args) === true ? 1 : 0;
    } catch {
      return THREW;
    }
  };
  port1.onmessage = (event: MessageEvent<A>) => {
    Atomics.store(cells, 1, answer(event.data));
    Atomics.store(cells, 0, 1);
    Atomics.notify(cells, 0);
  };
  return { cells, port: port2 };
}

/** Calls callback `id` of `kind` with `args`, a remote one moving `transfer` and blocking; whether to go on. */
function invoke<A extends unknown[]>(
  kind: Map<number, Callback<A>>,
  id: number,
  args: A,
  transfer: Transferable[] = [],
): boolean {
  const callback = kind.get(id);
  if (callback === undefined) {
    throw new RangeError(`no callback ${id}`);
  }
  if (typeof callback === "function") {
    // Only true goes on, as for a callback on a page's main thread.
    return callback(...args) === true;
  }
  Atomics.store(callback.cells, 0, 0);
  callback.port.postMessage(args, transfer);
  Atomics.wait(callback.cells, 0, 0);
  const answer = Atomics.load(callback.cells, 1);
  if (answer === THREW) {
    throw new Error("the callback threw");
  }
  return answer === 1;
}

/** Appends a copy of `bytes`, a view of WebAssembly memory, with callback `id`. */
export function append(id: number, bytes: Uint8Array): void {
  const copy = bytes.slice();
  invoke(appends, id, [copy], [copy.buffer]);
}

/** Reads input `id` from `offset` into `buffer`, a Blob only the part asked for; the count read. */
export function read(id: number, offset: bigint, buffer: Uint8Array): number {
  const input = inputs.get(id);
  if (input === undefined) {
    throw new RangeError(`no input ${id}`);
  }
  const start = Number(offset);
  const end = start + buffer.length;
  const bytes =
    input instanceof Blob
      ? new Uint8Array(new FileReaderSync().readAsArrayBuffer(input.slice(start, end)))
      : input.subarray(start, end);
  buffer.set(bytes);
  return bytes.length;
}

/** Forgets input or callback `id`. */
export function release(id: number): void {
  for (const kind of [appends, updates]) {
    const callback = kind.get(id);
    if (typeof callback === "object") {
      callback.port.close();
    }
    kind.delete(id);
  }
  inputs.delete(id);
}

/** Reports `done` of `total` with callback `id`; whether to go on. */
export function update(id: number, done: bigint, total: bigint): boolean {
  return invoke(updates, id, [Number(done), Number(total)]);
}
