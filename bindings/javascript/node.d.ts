// The Node.js APIs the package and its tests use, declared here rather than adding @types/node.

interface ImportMeta {
  readonly dirname: string;
}

declare const process: {
  readonly env: Record<string, string | undefined>;
  readonly execPath: string;
  readonly platform: string;
};

declare module "node:assert/strict" {
  export function deepEqual<T>(actual: unknown, expected: T, message?: string): asserts actual is T;
  export function equal<T>(actual: unknown, expected: T, message?: string): asserts actual is T;
  export function ok(value: unknown, message?: string): asserts value;
}

declare module "node:child_process" {
  interface Readable {
    on(event: "data", listener: (chunk: Uint8Array) => void): this;
  }
  export interface ChildProcess {
    readonly exitCode: number | null;
    readonly signalCode: string | null;
    readonly stderr: Readable | null;
    readonly stdout: Readable | null;
    kill(): boolean;
    once(event: "close" | "exit", listener: (code: number | null) => void): this;
    once(event: "error", listener: (error: Error) => void): this;
  }
  export function spawn(
    command: string,
    args: readonly string[],
    options: {
      cwd?: string;
      env?: Record<string, string | undefined>;
      stdio: "ignore" | readonly ("ignore" | "pipe")[];
    },
  ): ChildProcess;
}

declare module "node:fs" {
  export function existsSync(path: string): boolean;
}

declare module "node:fs/promises" {
  export function mkdtemp(prefix: string): Promise<string>;
  export function readdir(path: string, options?: { recursive: boolean }): Promise<string[]>;
  export function readFile(path: string | URL): Promise<Uint8Array>;
  export function readFile(path: string, encoding: "utf8"): Promise<string>;
  export function rm(path: string, options: { force: boolean; maxRetries: number; recursive: boolean }): Promise<void>;
  export function writeFile(path: string, data: string): Promise<void>;
}

declare module "node:http" {
  export interface IncomingMessage extends AsyncIterable<Uint8Array> {
    readonly method: string | undefined;
    readonly url: string | undefined;
  }
  export interface ServerResponse {
    end(body?: Uint8Array): this;
    writeHead(status: number, headers?: Record<string, string>): this;
  }
  export interface Server {
    address(): { readonly port: number } | string | null;
    close(callback: () => void): this;
    closeAllConnections(): void;
    listen(port: number, host: string, callback: () => void): this;
  }
  export function createServer(listener: (request: IncomingMessage, response: ServerResponse) => void): Server;
}

declare module "node:os" {
  export function tmpdir(): string;
}

declare module "node:path" {
  export const delimiter: string;
  export const sep: string;
  export function dirname(path: string): string;
  export function extname(path: string): string;
  export function join(...paths: string[]): string;
  export function relative(from: string, to: string): string;
  export function resolve(...paths: string[]): string;
}

declare module "node:test" {
  export interface TestContext {
    diagnostic(message: string): void;
    test(name: string, fn: () => Promise<void>): Promise<void>;
  }
  export function after(fn: () => Promise<void>): void;
  export function test(name: string, fn: (context: TestContext) => Promise<void>): Promise<void>;
}

declare module "node:url" {
  export function pathToFileURL(path: string): URL;
}
