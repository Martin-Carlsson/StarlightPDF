// The npm package: installing, declarations, its two modules, loading WebAssembly.
import { deepEqual, equal, ok } from "node:assert/strict";
import { readdir, readFile, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { test } from "node:test";
import { browse, corpus, exec, installed, NESTED, npm, ROOT, scenario, withoutRendering } from "./harness.mts";

/** A strict caller of every export; `Runtime` is the names the package exports at run time. */
const CALLER = `import * as api from "starlightpdf";
import { Document, PdfError, Sink, Source, capabilities, headerVersion, libraryVersion } from "starlightpdf";

const bytes = new Uint8Array([]);
const source: Source = await Source.fromBytes(bytes);
const fromBlob: Promise<Source> = Source.fromBlob(new Blob([bytes]));
const document: Document = await Document.open(source);
const pages: number = await document.pageCount();
const version: [number, number] = await headerVersion(bytes);
const library: string = await libraryVersion();
const parts: readonly string[] = await capabilities();
const options = { colorSpace: "Gray", fit: 64, format: "Jpeg", quality: 85, resolution: 72, usage: "Print" } as const;
await (await document.page({ index: 0 })).render(await Sink.inMemory(), () => true, options);
const failure: unknown = await Document.open(source).catch((error: unknown) => error);

export function details(
  error: PdfError,
): [Error, string, number, string, string, number | null, number | null, string | null] {
  return [error, error.code, error.number, error.message, error.operation, error.offset, error.objectNumber, error.ioCause];
}

type Declared = keyof typeof api;
const exact: [Runtime, Declared] extends [Declared, Runtime] ? true : false = true;

export const used = [fromBlob, pages, version, library, parts, failure instanceof PdfError, exact];
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

test("the package holds a module with rendering and one without", async () => {
  // .architecture-decisions/packaging-capabilities-and-versions.md: the caller loads one of two prebuilt modules.
  const package_ = await installed();
  const full = package_.api;
  const without = await withoutRendering(package_);
  deepEqual(Object.keys(without.api).sort(), Object.keys(full).sort(), "the same API");
  ok((await full.capabilities()).includes("Rendering"), "Rendering is built in");
  ok(!(await without.api.capabilities()).includes("Rendering"), "Rendering is left out");

  const bytes = await corpus("tests/corpus/render/output-colours.pdf");
  const shown = await (await without.api.Document.open(await without.api.Source.fromBytes(bytes))).page({ index: 0 });
  for (const options of [{}, { quality: 0 }, { format: "Tiff" }]) {
    const error: unknown = await shown.render(await without.api.Sink.inMemory(), null, options).then(
      () => undefined,
      (reason: unknown) => reason,
    );
    ok(error instanceof without.api.PdfError, String(error));
    deepEqual([error.code, error.number, error.operation], ["CapabilityMissing", 4, "Page.render"]);
  }

  /** The module a page loading `html` downloads, by its size. */
  const downloaded = async (html: string): Promise<number> => {
    const { requests } = await browse(package_, html);
    const found = [...new Set(requests.filter((path) => path.endsWith(".wasm")))];
    equal(found.length, 1, `one module: ${found.join(", ")}`);
    return (await readFile(join(package_.dir, found[0] ?? ""))).length;
  };
  const [whole, less] = [await downloaded(scenario("lazy")), await downloaded(scenario("lazy", null, without.url))];
  ok(less < whole, `${less} bytes without rendering, ${whole} with it`);
});

test("importing fetches no WebAssembly until the first call", async () => {
  const { requests } = await browse(await installed(), scenario("lazy"));
  const mark = requests.indexOf("/mark");
  const wasm = requests.flatMap((path, at) => (path.endsWith(".wasm") ? [at] : []));
  ok(mark >= 0 && wasm.length > 0 && wasm.every((at) => at > mark), requests.join("\n"));
});
