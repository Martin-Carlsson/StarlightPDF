// The package's API as the tests expect it (issues #147 and #148).

export type Source = object;

export type ResourcePolicy = object;

export interface ResourcePolicyOptions {
  readonly decodedLimit?: number;
  readonly objectLimit?: number;
  readonly warningLimit?: number;
}

export interface OpenOptions {
  readonly password?: string;
  readonly strict?: boolean;
}

export interface Sink {
  takeBytes(): Promise<Uint8Array>;
}

export interface ObjectId {
  readonly generation: number;
  readonly number: number;
}

export interface Rect {
  readonly height: number;
  readonly width: number;
  readonly x: number;
  readonly y: number;
}

export type Boundary = "Art" | "Bleed" | "Crop" | "Media" | "Trim";

export interface Encryption {
  readonly certificate: boolean;
  readonly cipher: string;
  readonly restrictions: readonly string[];
}

export interface Deviation {
  readonly clause: string;
}

export interface Page {
  boundary(options?: { readonly kind?: Boundary }): Promise<Rect>;
  rotation(): Promise<number>;
}

export interface Document {
  catalog(): Promise<ObjectId>;
  droppedWarnings(): Promise<number>;
  encryption(): Promise<Encryption>;
  object(options: { readonly id: ObjectId }): Promise<string>;
  page(options: { readonly index: number }): Promise<Page>;
  pageCount(): Promise<number>;
  streamData(sink: Sink, options: { readonly decode?: boolean; readonly id: ObjectId }): Promise<void>;
  warnings(): Promise<AsyncIterable<Deviation>>;
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
    open(source: Source, policy?: ResourcePolicy | null, options?: OpenOptions): Promise<Document>;
  };
  readonly PdfError: abstract new (...args: never) => PdfError;
  readonly ResourcePolicy: (abstract new (...args: never) => ResourcePolicy) & {
    create(options: ResourcePolicyOptions): Promise<ResourcePolicy>;
  };
  readonly Sink: (abstract new (...args: never) => Sink) & {
    inMemory(): Promise<Sink>;
  };
  readonly Source: (abstract new (...args: never) => Source) & {
    fromBlob(blob: Blob): Promise<Source>;
    fromBytes(bytes: Uint8Array): Promise<Source>;
  };
  headerVersion(bytes: Uint8Array): Promise<[number, number]>;
  libraryVersion(): Promise<string>;
}
