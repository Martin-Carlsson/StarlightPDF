// Runs the conformance suite (tests/conformance/*.tsv) through the package, in Node.js or a browser.

/**
 * @import { Api, Boundary, Document, ObjectId, Page, Source } from "./api.mts"
 * @typedef {object} Environment Where case files come from.
 * @property {(path: string) => Promise<Uint8Array>} bytes The bytes of the file at `path`.
 * @property {(path: string) => Promise<Source | string>} file A source over the file at `path`, or the outcome.
 * @typedef {(api: Api, document: Document, given: Map<string, string>) => Promise<string>} Read
 * @typedef {{ needs: string[], read: Read, takes: string[] }} Family
 */

/** Each limit's argument and option name. @type {[string, string][]} */
const LIMITS = [
  ["decoded_limit", "decodedLimit"],
  ["object_limit", "objectLimit"],
  ["warning_limit", "warningLimit"],
];
/** The keys every operation but `header_version` takes. */
const OPEN = new Set(["password", "strict", ...LIMITS.map(([key]) => key)]);
/** The keys without a value. */
const FLAGS = new Set(["decode", "strict"]);
/** Each `box=` value. */
const BOXES = new Set(["Art", "Bleed", "Crop", "Media", "Trim"]);

/** @type {(text: string | undefined) => number} */
function integer(text) {
  const value = Number(text);
  if (text === undefined || text === "" || !Number.isSafeInteger(value)) {
    throw new Error(`bad number '${text}'`);
  }
  return value;
}

/** `value` as the shortest decimal that round-trips, without exponent; `-0` keeps its sign. @type {(value: number) => string} */
function decimal(value) {
  const text = Object.is(value, -0) ? "-0" : String(value);
  const match = /^(-?)(\d)(?:\.(\d+))?e([+-]\d+)$/u.exec(text);
  if (match === null) {
    return text;
  }
  const [, sign = "", lead = "", rest = "", exponent = ""] = match;
  const point = 1 + Number(exponent);
  return point > 0 ? sign + (lead + rest).padEnd(point, "0") : `${sign}0.${"0".repeat(-point)}${lead}${rest}`;
}

/** @type {(given: Map<string, string>) => ObjectId} */
function objectId(given) {
  const [number, generation, ...rest] = (given.get("id") ?? "").split(" ");
  if (rest.length > 0) {
    throw new Error(`bad id '${given.get("id")}'`);
  }
  return { generation: integer(generation), number: integer(number) };
}

/** @type {(document: Document, given: Map<string, string>) => Promise<Page>} */
async function page(document, given) {
  return document.page({ index: integer(given.get("index")) });
}

/** The data of the stream `id=` names, through an in-memory sink. @type {(api: Api, document: Document, given: Map<string, string>, decode: boolean) => Promise<Uint8Array>} */
async function streamData(api, document, given, decode) {
  const sink = await api.Sink.inMemory();
  await document.streamData(sink, { decode, id: objectId(given) });
  return sink.takeBytes();
}

/** Counts the pages and, with `id=`, reads that object and any stored stream data. @type {(api: Api, document: Document, given: Map<string, string>) => Promise<void>} */
async function touch(api, document, given) {
  await document.pageCount();
  if (given.has("id")) {
    await document.object({ id: objectId(given) });
    try {
      await streamData(api, document, given, false);
    } catch (error) {
      if (!(error instanceof api.PdfError && error.code === "InvalidArgument")) {
        throw error;
      }
    }
  }
}

/** Per operation: the keys it takes, those it needs, and what it reads after open. */
const FAMILIES = new Map(
  /** @type {[string, Family][]} */ ([
    ["open", { needs: [], read: async () => "ok", takes: [] }],
    ["page_count", { needs: [], read: async (_api, document) => String(await document.pageCount()), takes: [] }],
    [
      "catalog",
      {
        needs: [],
        read: async (_api, document) => {
          const id = await document.catalog();
          return `${id.number} ${id.generation}`;
        },
        takes: [],
      },
    ],
    [
      "object",
      { needs: ["id"], read: async (_api, document, given) => document.object({ id: objectId(given) }), takes: ["id"] },
    ],
    [
      "stream_data",
      {
        needs: ["id"],
        read: async (api, document, given) => {
          const bytes = await streamData(api, document, given, given.has("decode"));
          return bytes.length === 0
            ? "empty"
            : Array.from(bytes, (byte) => byte.toString(16).padStart(2, "0")).join("");
        },
        takes: ["decode", "id"],
      },
    ],
    [
      "page",
      {
        needs: ["index"],
        read: async (_api, document, given) => {
          await page(document, given);
          return "ok";
        },
        takes: ["index"],
      },
    ],
    [
      "boundary",
      {
        needs: ["box", "index"],
        read: async (_api, document, given) => {
          const kind = given.get("box") ?? "";
          if (!BOXES.has(kind)) {
            throw new Error(`bad box '${kind}'`);
          }
          const rect = await (await page(document, given)).boundary({ kind: /** @type {Boundary} */ (kind) });
          return [rect.x, rect.y, rect.width, rect.height].map(decimal).join(" ");
        },
        takes: ["box", "index"],
      },
    ],
    [
      "rotation",
      {
        needs: ["index"],
        read: async (_api, document, given) => String(await (await page(document, given)).rotation()),
        takes: ["index"],
      },
    ],
    [
      "encryption",
      {
        needs: [],
        read: async (_api, document) => {
          const { certificate, cipher, restrictions } = await document.encryption();
          return `${cipher} ${certificate} ${restrictions.join(",") || "none"}`;
        },
        takes: [],
      },
    ],
    [
      "warnings",
      {
        needs: [],
        read: async (api, document, given) => {
          await touch(api, document, given);
          const clauses = [];
          for await (const deviation of await document.warnings()) {
            clauses.push(deviation.clause);
          }
          return clauses.join(";") || "none";
        },
        takes: ["id"],
      },
    ],
    [
      "dropped_warnings",
      {
        needs: [],
        read: async (api, document, given) => {
          await touch(api, document, given);
          return String(await document.droppedWarnings());
        },
        takes: ["id"],
      },
    ],
  ]),
);

/**
 * Opens the file at the first of `args` with the options after it, then reads as `family` does.
 * @param {Api} api
 * @param {Environment} environment
 * @param {boolean} file
 * @param {Family} family
 * @param {string[]} args
 * @returns {Promise<string>}
 */
async function perform(api, environment, file, family, args) {
  const [path, ...options] = args;
  if (path === undefined) {
    throw new Error("takes a path");
  }
  /** @type {Map<string, string>} */
  const given = new Map();
  for (const arg of options) {
    const equals = arg.indexOf("=");
    const key = equals < 0 ? arg : arg.slice(0, equals);
    if (!(OPEN.has(key) || family.takes.includes(key)) || FLAGS.has(key) !== equals < 0 || given.has(key)) {
      throw new Error(`bad argument '${arg}'`);
    }
    given.set(key, arg.slice(equals + 1));
  }
  const missing = family.needs.filter((key) => !given.has(key));
  if (missing.length > 0) {
    throw new Error(`needs ${missing.join(", ")}`);
  }
  const made = file ? await environment.file(path) : await api.Source.fromBytes(await environment.bytes(path));
  if (typeof made === "string") {
    return made;
  }
  const limits = Object.fromEntries(
    LIMITS.filter(([key]) => given.has(key)).map(([key, name]) => [name, integer(given.get(key))]),
  );
  const policy = Object.keys(limits).length === 0 ? null : await api.ResourcePolicy.create(limits);
  const document = await api.Document.open(made, policy, {
    password: given.get("password") ?? "",
    strict: given.has("strict"),
  });
  return family.read(api, document, given);
}

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
  const [, name, kind] = /^(.+)_(bytes|file)$/u.exec(operation) ?? [];
  const family = name === undefined ? undefined : FAMILIES.get(name);
  if (family === undefined && operation !== "header_version") {
    return `unknown operation '${operation}'`;
  }
  let actual;
  try {
    if (family !== undefined) {
      actual = await perform(api, environment, kind === "file", family, args);
    } else if (args.length === 1 && args[0] !== undefined) {
      actual = (await api.headerVersion(await environment.bytes(args[0]))).join(".");
    } else {
      throw new Error(`takes 1 argument, got ${args.length}`);
    }
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
