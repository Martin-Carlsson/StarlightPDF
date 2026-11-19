// The package's API as the tests expect it (issue #147).

export type Source = object;

export interface Document {
  pageCount(): Promise<number>;
}

export interface PdfError extends Error {
  readonly code: string;
  readonly ioCause: string | null;
  readonly number: number;
  readonly objectNumber: number | null;
  readonly offset: number | null;
  readonly operation: string;
}

export interface Api {
  readonly Document: (abstract new (...args: never) => Document) & {
    open(source: Source): Promise<Document>;
  };
  readonly PdfError: abstract new (...args: never) => PdfError;
  readonly Source: (abstract new (...args: never) => Source) & {
    fromBlob(blob: Blob): Promise<Source>;
    fromBytes(bytes: Uint8Array): Promise<Source>;
  };
  headerVersion(bytes: Uint8Array): Promise<[number, number]>;
  libraryVersion(): Promise<string>;
}
