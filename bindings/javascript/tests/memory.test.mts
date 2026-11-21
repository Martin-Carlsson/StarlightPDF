// Memory is bounded by work, not file size (.architecture-decisions/memory-is-bounded-by-work-not-file-size.md).
import { deepEqual, ok } from "node:assert/strict";
import { test } from "node:test";
import { browse, installed, scenario } from "./harness.mts";

/** Most bytes of WebAssembly memory the count may take: the first measurement, 1179648, with headroom. */
const BOUND = 2 * 1024 * 1024;

/** Most bytes of WebAssembly memory opening, splitting and saving may take; 1 until the first measurement replaces it. */
const SPLIT_BOUND = 1;

test("counting the pages of a 1 GiB Blob keeps WebAssembly memory within its bound", async (context) => {
  const { result } = await browse(await installed(), scenario("worker", ["memory"]));
  const { bytes, count, memories, size } = result as {
    bytes: number;
    count: number;
    memories: number;
    size: number;
  };
  context.diagnostic(`peak WebAssembly memory: ${bytes} bytes`);
  deepEqual({ count, large: size > 2 ** 30, used: memories > 0 }, { count: 2, large: true, used: true });
  ok(bytes <= BOUND, `${bytes} bytes`);
});

test("splitting and saving two 1 GiB files keeps WebAssembly memory within its bound", async (context) => {
  const { result } = await browse(await installed(), scenario("worker", ["splitting"]));
  const { bytes, floors, memories, saved } = result as {
    bytes: number;
    floors: number[][];
    memories: number;
    saved: number[][];
  };
  context.diagnostic(`peak WebAssembly memory: ${bytes} bytes; parts saved: ${JSON.stringify(saved)} bytes`);
  deepEqual(
    saved.map((parts, file) => parts.map((part, index) => part >= (floors[file]?.[index] ?? Infinity))),
    floors.map((parts) => parts.map(() => true)),
  );
  ok(memories > 0, "no WebAssembly memory");
  ok(bytes <= SPLIT_BOUND, `${bytes} bytes`);
});
