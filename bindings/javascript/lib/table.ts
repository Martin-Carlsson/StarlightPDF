// Inputs the core reads, by id; the WebAssembly glue imports `read` and `release`.

/** Reads a Blob synchronously; workers only (File API, FileReaderSync). */
declare const FileReaderSync: new () => { readAsArrayBuffer(blob: Blob): ArrayBuffer };

const inputs = new Map<number, Blob | Uint8Array>();
let last = 0;

/** Adds `input`; its id. */
export function add(input: Blob | Uint8Array): number {
  last += 1;
  inputs.set(last, input);
  return last;
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

/** Forgets input `id`. */
export function release(id: number): void {
  inputs.delete(id);
}
