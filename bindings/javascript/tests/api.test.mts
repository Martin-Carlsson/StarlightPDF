// The API Python has, as promises, and its errors.
import { deepEqual, equal, ok } from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { join } from "node:path";
import { test } from "node:test";
import { browse, corpus, installed, NESTED, ROOT, scenario } from "./harness.mts";

test("every operation Python has returns a promise", async () => {
  const { api, pkg } = await installed();
  const made = api.Source.fromBytes(await corpus(NESTED));
  ok(made instanceof Promise, "Source.fromBytes");
  const source = await made;
  ok(source instanceof api.Source);
  const opening = api.Document.open(source);
  ok(opening instanceof Promise, "Document.open");
  const document = await opening;
  ok(document instanceof api.Document);
  const counting = document.pageCount();
  ok(counting instanceof Promise, "pageCount");
  await counting;
  const header = api.headerVersion(await corpus("tests/corpus/header/version-1.7.pdf"));
  ok(header instanceof Promise, "headerVersion");
  deepEqual(await header, [1, 7]);
  const version = api.libraryVersion();
  ok(version instanceof Promise, "libraryVersion");
  const manifest: { readonly version?: string } = JSON.parse(await readFile(join(pkg, "package.json"), "utf8"));
  const cargo = await readFile(join(ROOT, "Cargo.toml"), "utf8");
  const workspace = cargo.split(/^\[/mu).find((section) => section.startsWith("workspace.package]"));
  const crate = /^version = "([^"]+)"/mu.exec(workspace ?? "")?.[1];
  deepEqual([await version, manifest.version], [crate, crate]);
});

test("failures reject with PdfError carrying the same details as in Python", async () => {
  const package_ = await installed();
  const { api } = package_;
  const source = await api.Source.fromBytes(await corpus("tests/corpus/header/not-pdf.pdf"));
  const error: unknown = await api.Document.open(source).then(
    () => undefined,
    (reason: unknown) => reason,
  );
  ok(error instanceof api.PdfError, String(error));
  ok(error instanceof Error);
  deepEqual([error.code, error.number], ["InvalidHeader", 8]);
  ok(error.message.length > 0);
  deepEqual([error.offset, error.objectNumber, error.ioCause], [null, null, null]);
  equal(error.operation, "Document.open");
  equal(String(error), `Document.open: ${error.message}`);
  // The operation in JavaScript's casing.
  const malformed = await api.Document.open(
    await api.Source.fromBytes(await corpus("tests/corpus/page-count/count-negative.pdf")),
  );
  const counting: unknown = await malformed.pageCount().then(
    () => undefined,
    (reason: unknown) => reason,
  );
  ok(counting instanceof api.PdfError, String(counting));
  deepEqual([counting.code, counting.operation], ["Malformed", "Document.pageCount"]);
  // Python's missing file: on a page's main thread, a Blob whose file is removed.
  const missing = await browse(package_, scenario("missing"));
  deepEqual(missing.result, { code: "Io", ioCause: "NotFound" });
});

test("a callback on a page's main thread without cross-origin isolation fails Unsupported", async () => {
  // ErrorCode::Unsupported is 18 (core/src/lib.rs, fixed forever).
  deepEqual((await browse(await installed(), scenario("unisolated"), false)).result, [
    false,
    ["Unsupported", 18, "Sink.fromAppend"],
    ["Unsupported", 18, "Document.save"],
  ]);
});

test("once the page's worker fails to load, every call rejects", async () => {
  deepEqual((await browse(await installed(), scenario("broken"))).result, ["Io", "Io"]);
});

test("a number outside the core's integer type rejects with RangeError instead of wrapping", async () => {
  const { api } = await installed();
  const document = await api.Document.open(await api.Source.fromBytes(await corpus(NESTED)));
  const sink = await api.Sink.inMemory();
  const calls: readonly (() => Promise<unknown>)[] = [
    async () => api.ResourcePolicy.create({ decodedLimit: -1 }),
    async () => api.ResourcePolicy.create({ objectLimit: 2 ** 53 }),
    async () => api.ResourcePolicy.create({ warningLimit: 2 ** 32 }),
    async () => document.object({ id: { generation: 65_536, number: 1 } }),
    async () => document.object({ id: { generation: 0, number: -1 } }),
    async () => document.page({ index: 0.5 }),
    async () => document.streamData(sink, { id: { generation: 0, number: 2 ** 32 } }),
  ];
  for (const [index, made] of calls.entries()) {
    const reason: unknown = await made().then(
      () => undefined,
      (error: unknown) => error,
    );
    ok(reason instanceof RangeError, `call ${String(index)}: ${String(reason)}`);
  }
});
