// The conformance suite in Node.js, a browser worker and a page's main thread, and its runner.
import { deepEqual } from "node:assert/strict";
import { readdir, readFile } from "node:fs/promises";
import { join } from "node:path";
import { test } from "node:test";
import type { Api } from "./api.mts";
import { browse, corpus, installed, ROOT, scenario } from "./harness.mts";
import { run, type Environment } from "./runner.mjs";

const DIR = join(ROOT, "tests", "conformance");

/** Every suite file's name. */
async function names(): Promise<string[]> {
  return (await readdir(DIR)).filter((name) => name.endsWith(".tsv")).sort();
}

/** Every suite, named. */
async function suites(): Promise<(readonly [string, string])[]> {
  return Promise.all((await names()).map(async (name) => [name, await readFile(join(DIR, name), "utf8")] as const));
}

/** Case files from disk; `*_file` takes their bytes, standing in for a path source, so an unreadable file is `Io`. */
function node(api: Api): Environment {
  return {
    bytes: corpus,
    file: async (path) => {
      const bytes = await corpus(path).catch(() => undefined);
      return bytes === undefined ? "Io" : api.Source.fromBytes(bytes);
    },
  };
}

test("conformance passes in Node.js", async () => {
  const { api } = await installed();
  deepEqual(await run(api, node(api), await suites()), []);
});

test("conformance passes inside a browser worker", async () => {
  const browsed = await browse(await installed(), scenario("worker", ["conformance", await names()]));
  deepEqual(browsed.result, { failures: [], instances: 1 });
});

test("conformance passes on a browser main thread", async () => {
  const browsed = await browse(await installed(), scenario("conformance", await names()));
  // No instance on the main thread: the core runs in a worker the package starts.
  deepEqual(browsed.result, { failures: [], instances: 0 });
});

test("the conformance runner reports as the other runners do", async (context) => {
  const { api } = await installed();
  const check = async (...given: (readonly [string, string])[]): Promise<string[]> => run(api, node(api), given);
  await context.test("an unknown operation fails", async () => {
    deepEqual(await check(["s.tsv", "nope\tx\ty\n"]), ["s.tsv:1: unknown operation 'nope'"]);
  });
  await context.test("a suite without cases fails", async () => {
    deepEqual(await check(["s.tsv", "# operation\targuments\texpected\n"]), ["no cases"]);
  });
  await context.test("a run names every failing case", async () => {
    const a = [
      "# c",
      "header_version\ttests/corpus/header/version-1.7.pdf\t1.7",
      "header_version\ttests/corpus/header/version-2.9.pdf\t1.7",
      "",
    ].join("\n");
    deepEqual(await check(["a.tsv", a], ["b.tsv", "open_bytes\ttests/corpus/header/empty.pdf\tok\n"]), [
      "a.tsv:3: header_version: expected '1.7', got '2.9'",
      "b.tsv:1: open_bytes: expected 'ok', got 'InvalidHeader'",
    ]);
  });
  await context.test("bytes and file expectations for one corpus file must agree", async () => {
    const opened = [
      "open_bytes\ttests/corpus/header/version-1.7.pdf\tok",
      "open_file\ttests/corpus/header/version-1.7.pdf\tInvalidHeader",
      "",
    ].join("\n");
    const counted = [
      "page_count_bytes\ttests/corpus/page-count/one-page.pdf\t1",
      "page_count_file\ttests/corpus/page-count/one-page.pdf\t2",
      "",
    ].join("\n");
    deepEqual(await check(["open.tsv", opened], ["page_count.tsv", counted]), [
      "open.tsv:2: expects 'InvalidHeader', open.tsv:1 expects 'ok'",
      "open.tsv:2: open_file: expected 'InvalidHeader', got 'ok'",
      "page_count.tsv:2: expects '2', page_count.tsv:1 expects '1'",
      "page_count.tsv:2: page_count_file: expected '2', got '1'",
    ]);
  });
  await context.test("a CR LF suite passes as its LF form does", async () => {
    deepEqual(await check(["s.tsv", "# c\r\nheader_version\ttests/corpus/header/version-1.7.pdf\t1.7\r\n"]), []);
  });
});
