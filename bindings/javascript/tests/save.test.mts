// Saving.
import { deepEqual } from "node:assert/strict";
import { test } from "node:test";
import { corpus, installed, NESTED } from "./harness.mts";

test("sinks receive the same bytes", async () => {
  const { api } = await installed();
  const document = await api.Document.open(await api.Source.fromBytes(await corpus(NESTED)));
  const memory = await api.Sink.inMemory();
  await document.save(memory);
  const expected = await memory.takeBytes();
  const chunks: Uint8Array[] = [];
  await document.save(await api.Sink.fromAppend((chunk) => chunks.push(chunk)));
  const appended = new Uint8Array(chunks.reduce((total, chunk) => total + chunk.length, 0));
  chunks.reduce((at, chunk) => {
    appended.set(chunk, at);
    return at + chunk.length;
  }, 0);
  deepEqual([new TextDecoder().decode(expected.subarray(0, 5)), Array.from(appended)], ["%PDF-", Array.from(expected)]);
});
