// The package's API as the tests expect it (issues #147, #148 and #149).

export type Source = object;

export type Layer = object;

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

export type Progress = (done: number, total: number) => boolean;

export interface SaveOptions {
  readonly allowLoss?: boolean;
  readonly breakSignatures?: boolean;
  readonly cipher?: string;
  readonly colorSpace?: string;
  readonly imageQuality?: number;
  readonly imageResolution?: number;
  readonly mode?: string;
  readonly optimizations?: readonly string[];
  readonly ownerPassword?: string;
  readonly recipients?: Uint8Array;
  readonly restrictions?: readonly string[];
  readonly standards?: readonly string[];
  readonly userPassword?: string;
  readonly version?: readonly [number, number];
}

export interface SaveReport {
  readonly brokenSignatures: readonly string[];
  readonly length: number;
}

export interface SplitOptions {
  readonly allowLoss?: boolean;
  readonly by?: "Bookmarks" | "Pages" | "Ranges" | "Size";
  readonly count?: number;
  readonly starts?: readonly number[];
}

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

export interface InsertBlankPageOptions {
  readonly height?: number;
  readonly index?: number;
  readonly width?: number;
}

export interface DrawImageOptions {
  readonly format?: string;
  readonly frame?: number;
  readonly height?: number;
  readonly ignoreOrientation?: boolean;
  readonly opacity?: number;
  readonly width?: number;
  readonly x?: number;
  readonly y?: number;
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
  drawImage(image: Source, mask?: Source | null, layer?: Layer | null, options?: DrawImageOptions): Promise<void>;
  rotation(): Promise<number>;
}

export interface Document {
  addLayer(options: { readonly name: string; readonly visible?: boolean }): Promise<Layer>;
  catalog(): Promise<ObjectId>;
  droppedWarnings(): Promise<number>;
  encryption(): Promise<Encryption>;
  infoDictionary(): Promise<InfoDictionary>;
  insertBlankPage(options?: InsertBlankPageOptions): Promise<Page>;
  object(options: { readonly id: ObjectId }): Promise<string>;
  page(options: { readonly index: number }): Promise<Page>;
  pageCount(): Promise<number>;
  save(sink: Sink, progress?: Progress | null, options?: SaveOptions): Promise<SaveReport>;
  setInfoDictionary(options: { readonly [K in keyof InfoDictionary]?: string }): Promise<void>;
  split(options: SplitOptions): AsyncIterable<Document>;
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
    create(options?: { readonly version?: readonly [number, number] }): Promise<Document>;
    open(source: Source, policy?: ResourcePolicy | null, options?: OpenOptions): Promise<Document>;
  };
  readonly PdfError: abstract new (...args: never) => PdfError;
  readonly ResourcePolicy: (abstract new (...args: never) => ResourcePolicy) & {
    create(options: ResourcePolicyOptions): Promise<ResourcePolicy>;
  };
  readonly Sink: (abstract new (...args: never) => Sink) & {
    fromAppend(append: (chunk: Uint8Array) => void): Promise<Sink>;
    inMemory(): Promise<Sink>;
  };
  readonly Source: (abstract new (...args: never) => Source) & {
    fromBlob(blob: Blob): Promise<Source>;
    fromBytes(bytes: Uint8Array): Promise<Source>;
  };
  headerVersion(bytes: Uint8Array): Promise<[number, number]>;
  libraryVersion(): Promise<string>;
}
