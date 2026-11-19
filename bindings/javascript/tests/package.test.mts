// The npm package: installing, declarations, loading WebAssembly.
import { deepEqual, equal, ok } from "node:assert/strict";
import { readdir, readFile, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { test } from "node:test";
import { browse, corpus, exec, installed, NESTED, npm, ROOT, scenario } from "./harness.mts";

/** A strict caller of every export; `Runtime` is the names the package exports at run time. */
const CALLER = `import * as api from "starlightpdf";
import { Document, PdfError, Source, headerVersion, libraryVersion } from "starlightpdf";

const bytes = new Uint8Array([]);
const source: Source = await Source.fromBytes(bytes);
const fromBlob: Promise<Source> = Source.fromBlob(new Blob([bytes]));
const document: Document = await Document.open(source);
const pages: number = await document.pageCount();
const version: [number, number] = await headerVersion(bytes);
const library: string = await libraryVersion();
const failure: unknown = await Document.open(source).catch((error: unknown) => error);

export function details(
  error: PdfError,
): [Error, string, number, string, string, number | null, number | null, string | null] {
  return [error, error.code, error.number, error.message, error.operation, error.offset, error.objectNumber, error.ioCause];
}

type Declared = keyof typeof api;
const exact: [Runtime, Declared] extends [Declared, Runtime] ? true : false = true;

export const used = [fromBlob, pages, version, library, failure instanceof PdfError, exact];
`;

test("installs from its tarball without Rust or a build step", async () => {
  const { api, pkg } = await installed();
  const manifest: { readonly scripts?: Record<string, string> } = JSON.parse(
    await readFile(join(pkg, "package.json"), "utf8"),
  );
  // npm's install lifecycle scripts (docs.npmjs.com, "Scripts"), and node-gyp's implicit one.
  const lifecycle = Object.keys(manifest.scripts ?? {}).filter((name) =>
    /^(?:pre|post)?(?:install|prepare)$/u.test(name),
  );
  deepEqual(lifecycle, []);
  const files = await readdir(pkg, { recursive: true });
  ok(!files.some((file) => file.endsWith("binding.gyp")), "ships binding.gyp");
  ok(
    files.some((file) => file.endsWith(".wasm")),
    "ships no .wasm",
  );
  const document = await api.Document.open(await api.Source.fromBytes(await corpus(NESTED)));
  equal(await document.pageCount(), 3);
});

test("declarations cover every export and type-check a strict caller", async () => {
  const { api, dir } = await installed();
  const names = Object.keys(api);
  ok(names.length > 0, "no exports");
  const runtime = names.map((name) => JSON.stringify(name)).join(" | ");
  await writeFile(join(dir, "caller.mts"), `${CALLER}\ntype Runtime = ${runtime};\n`);
  const config = {
    compilerOptions: { noEmit: true, rootDir: dir },
    extends: join(ROOT, "bindings", "javascript", "tsconfig.json"),
    files: ["caller.mts"],
    include: [],
  };
  await writeFile(join(dir, "tsconfig.json"), JSON.stringify(config));
  await exec(
    process.execPath,
    [npm("npx-cli.js"), "--offline", "--yes", "--package", "typescript@7.0.2", "tsc", "--project", dir],
    dir,
  );
});

test("importing fetches no WebAssembly until the first call", async () => {
  const { requests } = await browse(await installed(), scenario("lazy"));
  const mark = requests.indexOf("/mark");
  const wasm = requests.flatMap((path, at) => (path.endsWith(".wasm") ? [at] : []));
  ok(mark >= 0 && wasm.length > 0 && wasm.every((at) => at > mark), requests.join("\n"));
});
