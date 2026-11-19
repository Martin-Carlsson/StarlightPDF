// Runs the conformance suite (tests/conformance/*.tsv) through the package, in Node.js or a browser.

/**
 * @import { Api, Source } from "./api.mts"
 * @typedef {object} Environment Where case files come from.
 * @property {(path: string) => Promise<Uint8Array>} bytes The bytes of the file at `path`.
 * @property {(path: string) => Promise<Source | string>} file A source over the file at `path`, or the outcome.
 * @typedef {(api: Api, environment: Environment, path: string) => Promise<string>} Operation
 */

/** @type {(api: Api, made: Source | string) => Promise<string>} */
async function open(api, made) {
  if (typeof made === "string") {
    return made;
  }
  await api.Document.open(made);
  return "ok";
}

/** @type {(api: Api, made: Source | string) => Promise<string>} */
async function pageCount(api, made) {
  if (typeof made === "string") {
    return made;
  }
  return String(await (await api.Document.open(made)).pageCount());
}

/** @type {(api: Api, environment: Environment, path: string) => Promise<Source>} */
async function fromBytes(api, environment, path) {
  return api.Source.fromBytes(await environment.bytes(path));
}

/** @type {Map<string, Operation>} */
const OPERATIONS = new Map([
  [
    "header_version",
    async (api, environment, path) => (await api.headerVersion(await environment.bytes(path))).join("."),
  ],
  ["open_bytes", async (api, environment, path) => open(api, await fromBytes(api, environment, path))],
  ["open_file", async (api, environment, path) => open(api, await environment.file(path))],
  ["page_count_bytes", async (api, environment, path) => pageCount(api, await fromBytes(api, environment, path))],
  ["page_count_file", async (api, environment, path) => pageCount(api, await environment.file(path))],
]);

/**
 * Why `operation` on `args` does not give `expected`, or undefined.
 * @param {Api} api
 * @param {Environment} environment
 * @param {string} operation
 * @param {string[]} args
 * @param {string} expected
 * @returns {Promise<string | undefined>}
 */
async function check(api, environment, operation, args, expected) {
  const perform = OPERATIONS.get(operation);
  if (perform === undefined) {
    return `unknown operation '${operation}'`;
  }
  const [path] = args;
  if (path === undefined || args.length !== 1) {
    return `${operation} takes 1 argument, got ${args.length}`;
  }
  let actual;
  try {
    actual = await perform(api, environment, path);
  } catch (error) {
    if (!(error instanceof api.PdfError)) {
      return `${operation}: ${error instanceof Error ? error.message : String(error)}`;
    }
    actual = error.code;
  }
  return actual === expected ? undefined : `${operation}: expected '${expected}', got '${actual}'`;
}

/**
 * Runs every case of the named suites; one line per bad case or disagreeing pair, by `file:line`.
 * @param {Api} api
 * @param {Environment} environment
 * @param {readonly (readonly [string, string])[]} suites
 * @returns {Promise<string[]>}
 */
export async function run(api, environment, suites) {
  /** @type {string[]} */
  const failures = [];
  /** @type {Map<string, readonly [string, string]>} */
  const first = new Map();
  let found = false;
  for (const [name, text] of suites) {
    const lines = text.split("\n");
    if (lines.at(-1) === "") {
      lines.pop();
    }
    for (const [index, raw] of lines.entries()) {
      const line = raw.endsWith("\r") ? raw.slice(0, -1) : raw;
      if (line.startsWith("#")) {
        continue;
      }
      found = true;
      const position = `${name}:${index + 1}`;
      const [operation, ...rest] = line.split("\t");
      const expected = rest.pop();
      if (operation === undefined || expected === undefined) {
        failures.push(`${position}: malformed line '${line}'`);
        continue;
      }
      const key = [operation.replace(/_(?:bytes|file)$/u, ""), ...rest].join("\t");
      const earlier = first.get(key);
      if (earlier === undefined) {
        first.set(key, [position, expected]);
      } else if (earlier[1] !== expected) {
        failures.push(`${position}: expects '${expected}', ${earlier[0]} expects '${earlier[1]}'`);
      }
      const failure = await check(api, environment, operation, rest, expected);
      if (failure !== undefined) {
        failures.push(`${position}: ${failure}`);
      }
    }
  }
  if (!found) {
    failures.push("no cases");
  }
  return failures;
}
