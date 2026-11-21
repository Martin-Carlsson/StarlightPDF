// Saving.
import { deepEqual, equal } from "node:assert/strict";
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

test("save and streamData rethrow what a callback throws", async () => {
  const { api } = await installed();
  const open = async (path: string) => api.Document.open(await api.Source.fromBytes(await corpus(path)));
  const [pages, streams] = [await open(NESTED), await open("tests/corpus/streams/filters.pdf")];
  const thrown = new Error("append");
  const failing = await api.Sink.fromAppend(() => {
    throw thrown;
  });
  const reason = async (call: Promise<unknown>): Promise<unknown> =>
    call.then(
      () => undefined,
      (error: unknown) => error,
    );
  equal(await reason(pages.save(failing)), thrown);
  equal(await reason(streams.streamData(failing, { id: { generation: 0, number: 3 } })), thrown);
});
