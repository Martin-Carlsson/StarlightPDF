// Runs the conformance suite (tests/conformance/*.tsv) through the package, in Node.js or a browser.
import { decode, inflate, pixel } from "./decode.mjs";

/**
 * @import { Api, Boundary, Document, DrawImageOptions, InfoDictionary, ObjectId, Page, RenderOptions, ResourcePolicy, SaveOptions, SaveReport, Sink, Source, SplitOptions } from "./api.mts"
 * @typedef {object} Environment Where case files come from.
 * @property {(path: string) => Promise<Uint8Array>} bytes The bytes of the file at `path`.
 * @property {(path: string) => Promise<Source | string>} file A source over the file at `path`, or the outcome.
 * @typedef {(api: Api, document: Document, given: Map<string, string>) => Promise<string>} Read
 * @typedef {{ needs: string[], read: Read, takes: string[] }} Family
 * @typedef {{ name: string } | { ref: ObjectId } | { dict: Map<string, Obj> } | { array: Obj[] } | { other: string }} Obj
 */

/** Each limit's argument and option name. @type {[string, string][]} */
const LIMITS = [
  ["decoded_limit", "decodedLimit"],
  ["object_limit", "objectLimit"],
  ["warning_limit", "warningLimit"],
];
/** The keys every operation that opens a file takes. */
const OPEN = new Set(["password", "strict", ...LIMITS.map(([key]) => key)]);
/** The keys without a value. */
const FLAGS = new Set(["allow_loss", "break_signatures", "cancel", "decode", "ignore_orientation", "saved", "strict"]);
/** Each `box=` value. */
const BOXES = new Set(["Art", "Bleed", "Crop", "Media", "Trim"]);
/** The save options' keys. */
const SAVE = [
  "allow_loss",
  "break_signatures",
  "cancel",
  "cipher",
  "color_space",
  "image_quality",
  "image_resolution",
  "mode",
  "optimizations",
  "owner_password",
  "recipients",
  "restrictions",
  "standards",
  "user_password",
  "version",
];
/** The split options' keys. */
const SPLIT = ["allow_loss", "by", "count", "starts"];
/** The keys `draw_image` takes. */
const DRAW = [
  "decoded_limit",
  "draws",
  "format",
  "frame",
  "height",
  "ignore_orientation",
  "index",
  "mask",
  "on",
  "opacity",
  "page_height",
  "page_width",
  "read",
  "strict",
  "width",
  "x",
  "y",
];
/** The keys `render` takes besides those of open. */
const RENDER = ["cancel", "color_space", "fit", "format", "index", "quality", "read", "resolution", "usage"];
/** An expected render: its size, then each pixel named and its colour. */
const PIXELS = /^\d+x\d+(?: \d+,\d+=(?:[0-9a-f]{2}|[0-9a-f]{6})(?:~\d+)?)+$/u;
/** Each `key=` of `info`. @type {(keyof InfoDictionary)[]} */
const INFO = ["author", "created", "creator", "keywords", "modified", "producer", "subject", "title"];
/** Each `read=` of `draw_image`. */
const IMAGE_READS = new Set([
  "bits",
  "color_space",
  "decode",
  "filter",
  "icc_sha256",
  "mask_samples_sha256",
  "matrix",
  "raw_sha256",
  "samples_sha256",
  "size",
]);
/** White-space and delimiter characters (ISO 32000-2:2020, 7.2.3). */
const [WHITE, DELIMITERS] = ["\0\t\n\f\r ", "()<>[]{}/%"];

/** @type {(text: string | undefined) => number} */
function integer(text) {
  const value = Number(text);
  if (text === undefined || text === "" || !Number.isSafeInteger(value)) {
    throw new Error(`bad number '${text}'`);
  }
  return value;
}

/** @type {(text: string | undefined) => number} */
function real(text) {
  const value = Number(text);
  if (text === undefined || text.trim() === "" || !Number.isFinite(value)) {
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

/** `value` rounded to 4 fraction digits, trailing zeros and point dropped, -0 as 0. @type {(value: number) => string} */
function rounded(value) {
  const text = value.toFixed(4).replace(/\.?0+$/u, "");
  return text === "-0" ? "0" : text;
}

/** @type {(bytes: Uint8Array) => string} */
function hex(bytes) {
  return Array.from(bytes, (byte) => byte.toString(16).padStart(2, "0")).join("");
}

/** SHA-256 (FIPS 180-4) of `bytes` as lowercase hex. @type {(bytes: Uint8Array) => Promise<string>} */
async function sha256(bytes) {
  return hex(new Uint8Array(await crypto.subtle.digest("SHA-256", new Uint8Array(bytes))));
}

/** @type {(given: Map<string, string>) => ObjectId} */
function objectId(given) {
  const [number, generation, ...rest] = (given.get("id") ?? "").split(" ");
  if (rest.length > 0) {
    throw new Error(`bad id '${given.get("id")}'`);
  }
  return { generation: integer(generation), number: integer(number) };
}

/** The object `id=` names or `path=` reaches. @type {(api: Api, document: Document, given: Map<string, string>) => Promise<string>} */
async function readObject(_api, document, given) {
  const path = given.get("path");
  if (given.has("id") === (path !== undefined)) {
    throw new Error("object takes id= or path=");
  }
  return document.object({ id: path === undefined ? objectId(given) : await reached(document, path) });
}

/** The reference `path`, keys and indices from the catalog, reaches. @type {(document: Document, path: string) => Promise<ObjectId>} */
async function reached(document, path) {
  /** @type {Obj | undefined} */
  let value = { ref: await document.catalog() };
  for (const step of path.split("/")) {
    const held = await resolve(document, value);
    value = "array" in held ? (/^\d+$/u.test(step) ? held.array[Number(step)] : undefined) : entry(held, step);
    if (value === undefined) {
      throw new Error(`path ${path}: no ${step}`);
    }
  }
  if (!("ref" in value)) {
    throw new Error(`path ${path}: no reference at its end`);
  }
  return value.ref;
}

/** @type {(document: Document, given: Map<string, string>) => Promise<Page>} */
async function page(document, given) {
  return document.page({ index: integer(given.get("index")) });
}

/** The data of the stream `id`, through an in-memory sink. @type {(api: Api, document: Document, id: ObjectId, decode: boolean) => Promise<Uint8Array>} */
async function streamBytes(api, document, id, decode) {
  const sink = await api.Sink.inMemory();
  await document.streamData(sink, { decode, id });
  return sink.takeBytes();
}

/** The data of the stream `id=` names. @type {(api: Api, document: Document, given: Map<string, string>, decode: boolean) => Promise<Uint8Array>} */
async function streamData(api, document, given, decode) {
  return streamBytes(api, document, objectId(given), decode);
}

/** `rect` as `x y width height`. @type {(rect: { x: number, y: number, width: number, height: number }) => string} */
function rect(rect) {
  return [rect.x, rect.y, rect.width, rect.height].map(decimal).join(" ");
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
    ["object", { needs: [], read: readObject, takes: ["id", "path"] }],
    [
      "stream_data",
      {
        needs: ["id"],
        read: async (api, document, given) => {
          const bytes = await streamData(api, document, given, given.has("decode"));
          return bytes.length === 0 ? "empty" : hex(bytes);
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
          return rect(await (await page(document, given)).boundary({ kind: /** @type {Boundary} */ (kind) }));
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
      "info",
      {
        needs: ["key"],
        read: async (_api, document, given) => {
          const key = INFO.find((name) => name === given.get("key"));
          if (key === undefined) {
            throw new Error(`bad key '${given.get("key")}'`);
          }
          return (await document.infoDictionary())[key] || "none";
        },
        takes: ["key", ...INFO.map((key) => `set_${key}`)],
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
 * The keys and values of `options`, each of `takes` once; fails unless `needs` are among them.
 * @param {string[]} options
 * @param {Set<string>} takes
 * @param {string[]} needs
 * @returns {Map<string, string>}
 */
function parse(options, takes, needs) {
  /** @type {Map<string, string>} */
  const given = new Map();
  for (const arg of options) {
    const equals = arg.indexOf("=");
    const key = equals < 0 ? arg : arg.slice(0, equals);
    if (!takes.has(key) || FLAGS.has(key) !== equals < 0 || given.has(key)) {
      throw new Error(`bad argument '${arg}'`);
    }
    given.set(key, arg.slice(equals + 1));
  }
  const missing = needs.filter((key) => !given.has(key));
  if (missing.length > 0) {
    throw new Error(`needs ${missing.join(", ")}`);
  }
  return given;
}

/** Fails unless save options come with `saved`, split options with `part=`, not both. @type {(given: Map<string, string>) => void} */
function combine(given) {
  const saved = given.has("saved");
  const part = given.has("part");
  /** @type {(keys: string[]) => boolean} */
  const lone = (keys) => keys.some((key) => key !== "allow_loss" && given.has(key));
  if (
    (saved && part) ||
    (lone(SAVE) && !saved) ||
    (lone(SPLIT) && !part) ||
    (given.has("allow_loss") && !saved && !part)
  ) {
    throw new Error("save options need saved, split options part=, and not both");
  }
}

/** The comma-separated names of `text`; none for empty text. @type {(text: string | undefined) => string[]} */
function names(text) {
  return text === undefined || text === "" ? [] : text.split(",");
}

/** Saves `document` to `sink` with the save options `given`. @type {(document: Document, sink: Sink, given: Map<string, string>) => Promise<SaveReport>} */
async function save(document, sink, given) {
  /** @type {[string, unknown][]} */
  const entries = [];
  /** @type {(key: string, name: string, convert: (text: string) => unknown) => void} */
  const option = (key, name, convert) => {
    const value = given.get(key);
    if (value !== undefined) {
      entries.push([name, convert(value)]);
    }
  };
  option("allow_loss", "allowLoss", () => true);
  option("break_signatures", "breakSignatures", () => true);
  option("cipher", "cipher", String);
  option("color_space", "colorSpace", String);
  option("image_quality", "imageQuality", integer);
  option("image_resolution", "imageResolution", integer);
  option("mode", "mode", String);
  option("optimizations", "optimizations", names);
  option("owner_password", "ownerPassword", String);
  option("recipients", "recipients", (text) =>
    Uint8Array.from(text.match(/../gu) ?? [], (pair) => Number.parseInt(pair, 16)),
  );
  option("restrictions", "restrictions", names);
  option("standards", "standards", names);
  option("user_password", "userPassword", String);
  option("version", "version", (text) => text.split(".").map(integer));
  const progress = given.has("cancel") ? () => false : null;
  return document.save(sink, progress, /** @type {SaveOptions} */ (Object.fromEntries(entries)));
}

/** Saves `document` twice to memory; the bytes, failing unless both are the same. @type {(api: Api, document: Document, given: Map<string, string>) => Promise<Uint8Array>} */
async function twice(api, document, given) {
  const saved = [];
  for (let round = 0; round < 2; round += 1) {
    const sink = await api.Sink.inMemory();
    await save(document, sink, given);
    saved.push(await sink.takeBytes());
  }
  const [first = new Uint8Array(), second = new Uint8Array()] = saved;
  if (first.length !== second.length || first.some((byte, at) => byte !== second[at])) {
    throw new Error("two saves give different bytes");
  }
  return first;
}

/** The resource policy of the limits `given`, or null. @type {(api: Api, given: Map<string, string>) => Promise<ResourcePolicy | null>} */
async function policyOf(api, given) {
  const limits = Object.fromEntries(
    LIMITS.filter(([key]) => given.has(key)).map(([key, name]) => [name, integer(given.get(key))]),
  );
  return Object.keys(limits).length === 0 ? null : api.ResourcePolicy.create(limits);
}

/** Opens `made` with the open options `given`. @type {(api: Api, made: Source, given: Map<string, string>) => Promise<Document>} */
async function open(api, made, given) {
  return api.Document.open(made, await policyOf(api, given), {
    password: given.get("password") ?? "",
    strict: given.has("strict"),
  });
}

/** Opens `bytes` with the open options `given`. @type {(api: Api, bytes: Uint8Array, given: Map<string, string>) => Promise<Document>} */
async function reopen(api, bytes, given) {
  return open(api, await api.Source.fromBytes(bytes), given);
}

/** The parts of `document` split by the split options `given`. @type {(document: Document, given: Map<string, string>) => Promise<Document[]>} */
async function split(document, given) {
  const starts = given.get("starts");
  /** @type {SplitOptions} */
  const options = {
    allowLoss: given.has("allow_loss"),
    by: /** @type {SplitOptions["by"] & string} */ (given.get("by") ?? "Pages"),
    count: integer(given.get("count") ?? "0"),
    starts: starts === undefined || starts === "" ? [] : starts.split(",").map(integer),
  };
  const parts = [];
  for await (const part of await document.split(options)) {
    parts.push(part);
  }
  return parts;
}

/**
 * Opens the file at the first of `args` with the options after it, then performs `name`.
 * @param {Api} api
 * @param {Environment} environment
 * @param {boolean} file
 * @param {string} name
 * @param {string[]} args
 * @returns {Promise<string>}
 */
async function perform(api, environment, file, name, args) {
  const [path, ...options] = args;
  if (path === undefined) {
    throw new Error("takes a path");
  }
  const family = FAMILIES.get(name);
  /** @type {Map<string, string>} */
  let given;
  if (name === "save") {
    given = parse(options, new Set([...OPEN, ...SAVE]), []);
  } else if (name === "split") {
    given = parse(options, new Set([...OPEN, ...SPLIT]), []);
  } else if (family === undefined) {
    throw new Error(`unknown family '${name}'`);
  } else {
    given = parse(options, new Set([...OPEN, ...family.takes, "saved", "part", ...SAVE, ...SPLIT]), family.needs);
    combine(given);
  }
  const made = file ? await environment.file(path) : await api.Source.fromBytes(await environment.bytes(path));
  if (typeof made === "string") {
    return made;
  }
  let document = await open(api, made, given);
  if ([...given.keys()].some((key) => key.startsWith("set_"))) {
    await document.setInfoDictionary(
      Object.fromEntries(
        INFO.filter((key) => given.has(`set_${key}`)).map((key) => [key, given.get(`set_${key}`) ?? ""]),
      ),
    );
  }
  if (name === "save") {
    const report = await save(document, await api.Sink.inMemory(), given);
    return report.brokenSignatures.join(",") || "ok";
  }
  if (name === "split") {
    const counts = await Promise.all(
      (await split(document, given)).map(async (part) => String(await part.pageCount())),
    );
    return counts.join(" ") || "none";
  }
  if (given.has("saved")) {
    document = await reopen(api, await twice(api, document, given), given);
  } else if (given.has("part")) {
    const index = integer(given.get("part"));
    const part = (await split(document, given))[index];
    if (part === undefined) {
      throw new Error(`no part ${index}`);
    }
    document = await reopen(api, await twice(api, part, new Map()), given);
  }
  if (family === undefined) {
    throw new Error(`unknown family '${name}'`);
  }
  return family.read(api, document, given);
}

/** `token` where `image` has its colour within its tolerance, else the colour it has. @type {(image: import("./decode.mjs").Image, token: string) => string} */
function matched(image, token) {
  const [place = "", value = ""] = token.split("=");
  const [colour = "", allowed = "0"] = value.split("~");
  const [column, row] = place.split(",");
  const found = pixel(image, integer(column), integer(row));
  if (found === undefined) {
    return `${place}=none`;
  }
  const wanted = stringBytes(`<${colour}>`);
  const tolerance = integer(allowed);
  if (tolerance > 255) {
    throw new Error(`bad tolerance '${allowed}'`);
  }
  const close =
    found.length === wanted.length && found.every((have, at) => Math.abs(have - (wanted[at] ?? Infinity)) <= tolerance);
  return close ? token : `${place}=${hex(found)}`;
}

/**
 * Renders the page as `args` say; its size and, of the pixels `expected` names, each one's token or true colour.
 * @param {Api} api
 * @param {Environment} environment
 * @param {boolean} file
 * @param {string[]} args
 * @param {string} expected
 * @returns {Promise<string>}
 */
async function rendered(api, environment, file, args, expected) {
  const [path, ...options] = args;
  if (path === undefined) {
    throw new Error("takes a path");
  }
  // `cancel=N` is `cancel` letting N progress calls through.
  const counts = options.filter((arg) => arg.startsWith("cancel=")).map((arg) => arg.slice("cancel=".length));
  if (!counts.every((count) => /^[0-9]+$/u.test(count))) {
    throw new Error(`bad cancel '${counts.join(" ")}'`);
  }
  const allowed = counts.map(integer);
  const named = options.map((arg) => (arg.startsWith("cancel=") ? "cancel" : arg));
  const given = parse(named, new Set([...OPEN, ...RENDER]), []);
  const reads = given.get("read");
  if (reads !== undefined && reads !== "warnings") {
    throw new Error(`bad read '${reads}'`);
  }
  const code = /^[A-Za-z]+$/u.test(expected);
  if (reads === undefined && !code && !PIXELS.test(expected)) {
    throw new Error(`bad expected value '${expected}'`);
  }
  /** @type {RenderOptions} */
  const rendering = {
    ...(given.has("color_space") ? { colorSpace: given.get("color_space") ?? "" } : {}),
    ...(given.has("fit") ? { fit: integer(given.get("fit")) } : {}),
    ...(given.has("format") ? { format: given.get("format") ?? "" } : {}),
    ...(given.has("quality") ? { quality: integer(given.get("quality")) } : {}),
    ...(given.has("resolution") ? { resolution: integer(given.get("resolution")) } : {}),
    ...(given.has("usage") ? { usage: given.get("usage") ?? "" } : {}),
  };
  const made = file ? await environment.file(path) : await api.Source.fromBytes(await environment.bytes(path));
  if (typeof made === "string") {
    return made;
  }
  const document = await open(api, made, given);
  const shown = await document.page({ index: integer(given.get("index") ?? "0") });
  const sink = await api.Sink.inMemory();
  let calls = 0;
  const stop = () => {
    calls += 1;
    return calls <= (allowed[0] ?? 0);
  };
  await shown.render(sink, given.has("cancel") ? stop : null, rendering);
  const image = await decode(await sink.takeBytes());
  if (image.jpeg !== (given.get("format") === "Jpeg")) {
    throw new Error("the file is of another format");
  }
  if (reads !== undefined) {
    const warnings = FAMILIES.get("warnings");
    if (warnings === undefined) {
      throw new Error("no warnings family");
    }
    return warnings.read(api, document, given);
  }
  const tokens = code ? [] : expected.split(" ").slice(1);
  return [`${image.width}x${image.height}`, ...tokens.map((token) => matched(image, token))].join(" ");
}

/** Creates a document, inserts each `page=[WxH][@I]`, saves, reopens: each MediaBox. @type {(api: Api, args: string[]) => Promise<string>} */
async function inserted(api, args) {
  const [path, ...options] = args;
  if (path !== "-") {
    throw new Error("takes the path -");
  }
  const document = await api.Document.create();
  for (const arg of options) {
    const match = /^page=(?:([^x@]+)x([^@]+))?(?:@(.*))?$/u.exec(arg);
    if (match === null) {
      throw new Error(`bad argument '${arg}'`);
    }
    const [, width, height, index] = match;
    await document.insertBlankPage({
      ...(width === undefined ? {} : { height: real(height), width: real(width) }),
      ...(index === undefined ? {} : { index: integer(index) }),
    });
  }
  const saved = await reopen(api, await twice(api, document, new Map()), new Map());
  const boxes = [];
  for (let index = 0; index < (await saved.pageCount()); index += 1) {
    boxes.push(rect(await (await saved.page({ index })).boundary({ kind: "Media" })));
  }
  return boxes.join(";");
}

/** PDF syntax split into tokens (ISO 32000-2:2020, 7.2): strings whole, comments dropped. @type {(syntax: string) => string[]} */
function tokens(syntax) {
  const found = [];
  let at = 0;
  while (at < syntax.length) {
    const character = syntax.charAt(at);
    let end = at + 1;
    if (character === "(") {
      let depth = 0;
      end = at;
      while (end < syntax.length) {
        const next = syntax.charAt(end);
        if (next === "\\") {
          end += 2;
          continue;
        }
        depth += next === "(" ? 1 : next === ")" ? -1 : 0;
        end += 1;
        if (depth === 0) {
          break;
        }
      }
    } else if (syntax.startsWith("<<", at) || syntax.startsWith(">>", at)) {
      end = at + 2;
    } else if (character === "<") {
      end = syntax.indexOf(">", at) + 1 || syntax.length;
    } else if (character === "%" || WHITE.includes(character)) {
      while (character === "%" && end < syntax.length && !"\r\n".includes(syntax.charAt(end))) {
        end += 1;
      }
      at = end;
      continue;
    } else if (!DELIMITERS.includes(character) || character === "/") {
      while (end < syntax.length && !(WHITE + DELIMITERS).includes(syntax.charAt(end))) {
        end += 1;
      }
    }
    found.push(syntax.slice(at, end));
    at = end;
  }
  return found;
}

/** The object at `words[at]` and the index after it. @type {(words: string[], at: number) => [Obj, number]} */
function parseObject(words, at) {
  const first = words[at];
  if (first === undefined) {
    throw new Error("unexpected end of object");
  }
  if (first === "<<") {
    /** @type {Map<string, Obj>} */
    const dict = new Map();
    let next = at + 1;
    while (words[next] !== ">>") {
      const key = (words[next] ?? "").replace(/^\//u, "");
      const [value, after] = parseObject(words, next + 1);
      dict.set(key, value);
      next = after;
    }
    return [{ dict }, next + 1];
  }
  if (first === "[") {
    /** @type {Obj[]} */
    const array = [];
    let next = at + 1;
    while (words[next] !== "]") {
      const [value, after] = parseObject(words, next);
      array.push(value);
      next = after;
    }
    return [{ array }, next + 1];
  }
  if (first.startsWith("/")) {
    return [{ name: first.slice(1) }, at + 1];
  }
  const generation = words[at + 1];
  if (/^\d+$/u.test(first) && generation !== undefined && /^\d+$/u.test(generation) && words[at + 2] === "R") {
    return [{ ref: { generation: Number(generation), number: Number(first) } }, at + 3];
  }
  return [{ other: first }, at + 1];
}

/** The object `id`, parsed. @type {(document: Document, id: ObjectId) => Promise<Obj>} */
async function objectAt(document, id) {
  return parseObject(tokens(await document.object({ id })), 0)[0];
}

/** The object `value` is, read when it is a reference. @type {(document: Document, value: Obj | undefined) => Promise<Obj>} */
async function resolve(document, value) {
  if (value === undefined) {
    throw new Error("a needed entry is missing");
  }
  return "ref" in value ? objectAt(document, value.ref) : value;
}

/** The value of `key` in `dictionary`, if it is a dictionary holding it. @type {(dictionary: Obj, key: string) => Obj | undefined} */
function entry(dictionary, key) {
  return "dict" in dictionary ? dictionary.dict.get(key) : undefined;
}

/** The token `value` is: a name without its slash, or a number or string. @type {(value: Obj | undefined) => string} */
function text(value) {
  if (value !== undefined && "name" in value) {
    return value.name;
  }
  if (value !== undefined && "other" in value) {
    return value.other;
  }
  throw new Error("a needed name or number is missing");
}

/** The bytes of a string token, hexadecimal or literal (ISO 32000-2:2020, 7.3.4). @type {(token: string) => Uint8Array} */
function stringBytes(token) {
  if (token.startsWith("<")) {
    const digits = token.slice(1, -1).replace(/\s/gu, "");
    return Uint8Array.from((digits.length % 2 === 0 ? digits : `${digits}0`).match(/../gu) ?? [], (pair) =>
      Number.parseInt(pair, 16),
    );
  }
  /** @type {Record<string, number>} */
  const escapes = { b: 8, f: 12, n: 10, r: 13, t: 9 };
  const bytes = [];
  for (const [part] of token.slice(1, -1).matchAll(/\\(?:[0-7]{1,3}|\r\n|[^])|[^\\]/gu)) {
    if (!part.startsWith("\\")) {
      bytes.push(part.charCodeAt(0) & 0xff);
    } else if (/^\\[0-7]/u.test(part)) {
      bytes.push(Number.parseInt(part.slice(1), 8) & 0xff);
    } else if (!["\r", "\n", "\r\n"].includes(part.slice(1))) {
      bytes.push(escapes[part.slice(1)] ?? part.charCodeAt(1));
    }
  }
  return Uint8Array.from(bytes);
}

/** `DeviceGray`, `DeviceRGB`, `DeviceCMYK`, `ICCBased N`, or `Indexed <base> <hival> <hex>` (ISO 32000-2:2020, 8.6). @type {(api: Api, document: Document, value: Obj | undefined) => Promise<string>} */
async function colorSpace(api, document, value) {
  const resolved = await resolve(document, value);
  if (!("array" in resolved)) {
    return text(resolved);
  }
  const [head, ...rest] = resolved.array;
  const family = text(head);
  if (family === "ICCBased") {
    return `ICCBased ${decimal(Number(text(entry(await resolve(document, rest[0]), "N"))))}`;
  }
  if (family === "Indexed") {
    const [base, high, lookup] = rest;
    const table =
      lookup !== undefined && "ref" in lookup
        ? await streamBytes(api, document, lookup.ref, true)
        : stringBytes(text(lookup));
    return `Indexed ${await colorSpace(api, document, base)} ${decimal(Number(text(high)))} ${hex(table)}`;
  }
  return family;
}

/** The ICC profile of the color space `value`, directly or as an `Indexed` base. @type {(document: Document, value: Obj | undefined) => Promise<ObjectId | undefined>} */
async function icc(document, value) {
  const resolved = await resolve(document, value);
  if (!("array" in resolved)) {
    return undefined;
  }
  const [head, second] = resolved.array;
  const family = text(head);
  if (family === "ICCBased" && second !== undefined && "ref" in second) {
    return second.ref;
  }
  return family === "Indexed" ? icc(document, second) : undefined;
}

/** The name the last `Do` paints and the CTM at each `Do`, `;`-separated: the product of every `cm` in effect (ISO 32000-2:2020, 8.3.4, 8.4.4). @type {(api: Api, document: Document, pageObject: Obj) => Promise<[string, string]>} */
async function painted(api, document, pageObject) {
  const contents = entry(pageObject, "Contents");
  const items = contents !== undefined && "array" in contents ? contents.array : [contents];
  const decoder = new TextDecoder("latin1");
  let content = "";
  for (const item of items) {
    if (item !== undefined && "ref" in item) {
      content += `${decoder.decode(await streamBytes(api, document, item.ref, true))}\n`;
    }
  }
  /** @type {number[][]} */
  const states = [];
  let current = [1, 0, 0, 1, 0, 0];
  /** @type {string[]} */
  const operands = [];
  /** @type {string | undefined} */
  let last;
  /** @type {string[]} */
  const ctms = [];
  for (const word of tokens(content)) {
    if (word === "q") {
      states.push(current);
    } else if (word === "Q") {
      current = states.pop() ?? current;
    } else if (word === "cm") {
      const [ma = 0, mb = 0, mc = 0, md = 0, me = 0, mf = 0] = operands.slice(-6).map(Number);
      const [ca = 0, cb = 0, cc = 0, cd = 0, ce = 0, cf = 0] = current;
      current = [
        ma * ca + mb * cc,
        ma * cb + mb * cd,
        mc * ca + md * cc,
        mc * cb + md * cd,
        me * ca + mf * cc + ce,
        me * cb + mf * cd + cf,
      ];
    } else if (word === "Do") {
      last = (operands.at(-1) ?? "").replace(/^\//u, "");
      ctms.push(current.map(rounded).join(" "));
    }
    if (/^[A-Za-z'"]/u.test(word)) {
      operands.length = 0;
    } else {
      operands.push(word);
    }
  }
  if (last === undefined) {
    throw new Error("no Do in the page's content");
  }
  return [last, ctms.join(";")];
}

/** The `Decode` numbers space-separated, or `none` (ISO 32000-2:2020, 8.9.5.2). @type {(value: Obj | undefined) => string} */
function decodeArray(value) {
  if (value === undefined) {
    return "none";
  }
  if (!("array" in value)) {
    throw new Error("Decode is not an array");
  }
  return value.array.map((item) => decimal(Number(text(item)))).join(" ");
}

/** The `Filter` names space-separated, or `none` (ISO 32000-2:2020, 7.4). @type {(value: Obj | undefined) => string} */
function filters(value) {
  if (value === undefined) {
    return "none";
  }
  return ("array" in value ? value.array : [value]).map(text).join(" ");
}

/** What `read` gives of the image the last `Do` paints on kid `index` of the root page tree node. @type {(api: Api, document: Document, index: number, read: string) => Promise<string>} */
async function imageRead(api, document, index, read) {
  const root = await objectAt(document, await document.catalog());
  const kids = await resolve(document, entry(await resolve(document, entry(root, "Pages")), "Kids"));
  const pageObject = await resolve(document, "array" in kids ? kids.array[index] : undefined);
  const [label, ctm] = await painted(api, document, pageObject);
  const resources = await resolve(document, entry(pageObject, "Resources"));
  const found = entry(await resolve(document, entry(resources, "XObject")), label);
  if (found === undefined || !("ref" in found)) {
    throw new Error(`no XObject /${label} on the page`);
  }
  const image = await objectAt(document, found.ref);
  const subtype = entry(image, "Subtype");
  if (subtype === undefined || !("name" in subtype) || subtype.name !== "Image") {
    throw new Error(`/${label} is not an image XObject`);
  }
  /** @type {(id: ObjectId | undefined) => Promise<string>} */
  const digest = async (id) => (id === undefined ? "none" : sha256(await streamBytes(api, document, id, true)));
  /** @type {(key: string) => string} */
  const number = (key) => decimal(Number(text(entry(image, key))));
  const mask = entry(image, "SMask");
  switch (read) {
    case "matrix":
      return ctm;
    case "decode":
      return decodeArray(entry(image, "Decode"));
    case "size":
      return `${number("Width")} ${number("Height")}`;
    case "bits":
      return number("BitsPerComponent");
    case "color_space":
      return colorSpace(api, document, entry(image, "ColorSpace"));
    case "filter":
      return filters(entry(image, "Filter"));
    case "raw_sha256":
      return sha256(await streamBytes(api, document, found.ref, false));
    case "samples_sha256":
      return digest(found.ref);
    case "mask_samples_sha256":
      return digest(mask !== undefined && "ref" in mask ? mask.ref : undefined);
    default:
      return digest(await icc(document, entry(image, "ColorSpace")));
  }
}

/** Creates a page, draws the image first in `args` as the options after it say, saves, reopens and reads. @type {(api: Api, environment: Environment, args: string[]) => Promise<string>} */
async function drawn(api, environment, args) {
  const [path, ...options] = args;
  if (path === undefined) {
    throw new Error("takes a path");
  }
  const given = parse(options, new Set(DRAW), ["read"]);
  const read = given.get("read") ?? "";
  if (!IMAGE_READS.has(read)) {
    throw new Error(`bad read '${read}'`);
  }
  const [pageWidth, pageHeight] = ["page_width", "page_height"].map((key) =>
    given.has(key) ? real(given.get(key)) : undefined,
  );
  const on = given.get("on");
  const index = given.has("index") ? integer(given.get("index")) : 0;
  const document =
    on === undefined
      ? await api.Document.create(await policyOf(api, given))
      : await open(api, await api.Source.fromBytes(await environment.bytes(on)), given);
  const created =
    on === undefined
      ? await document.insertBlankPage({
          ...(pageHeight === undefined ? {} : { height: pageHeight }),
          ...(pageWidth === undefined ? {} : { width: pageWidth }),
        })
      : await document.page({ index });
  const mask = given.has("mask") ? await api.Source.fromBytes(await environment.bytes(given.get("mask") ?? "")) : null;
  /** @type {(key: string) => number | undefined} */
  const optional = (key) => (given.has(key) ? real(given.get(key)) : undefined);
  const [x, y, width, height, opacity, frame] = ["x", "y", "width", "height", "opacity", "frame"].map(optional);
  const format = given.get("format");
  /** @type {DrawImageOptions} */
  const drawing = {
    ignoreOrientation: given.has("ignore_orientation"),
    ...(format === undefined ? {} : { format }),
    ...(frame === undefined ? {} : { frame }),
    ...(height === undefined ? {} : { height }),
    ...(opacity === undefined ? {} : { opacity }),
    ...(width === undefined ? {} : { width }),
    ...(x === undefined ? {} : { x }),
    ...(y === undefined ? {} : { y }),
  };
  const image = await api.Source.fromBytes(await environment.bytes(path));
  for (let draw = 0; draw < Math.max(given.has("draws") ? integer(given.get("draws")) : 1, 1); draw += 1) {
    await created.drawImage(image, mask, null, drawing);
  }
  return imageRead(api, await reopen(api, await twice(api, document, new Map()), given), index, read);
}

/** The keys ±annotate± takes besides those of open. */
const ANNOTATE = ["color", "delete", "height", "index", "kind", "quads", "saved", "width", "x", "y"];
/** The keys ±redact± takes besides those of open. */
const REDACT = ["bytes", "cancel", "do", "except", "index", "marks", "mode", "pages", "read"];
/** Each ±read=± of ±redact±, and whether it takes ±index=±. */
const REDACT_READS = new Map([
  ["annotations", true],
  ["image_raw", false],
  ["image_samples", false],
  ["mask_samples", false],
  ["ok", false],
  ["painted", true],
  ["pixels", true],
  ["points", true],
  ["render", true],
  ["text", false],
  ["thumb", true],
]);
/** The reads of the ±do=±-th image. */
const IMAGE_SAMPLES = new Set(["image_raw", "image_samples", "mask_samples"]);
/** How far inside a mark a point counts, in points: room for rounding where a path is cut at its edge. */
const MARGIN = 0.01;
/** Deepest nesting of forms whose points are counted. */
const DEPTH = 16;
/** The numbers each path operator takes (ISO 32000-2:2020, 8.5.2.1). */
const COORDINATES = new Map([
  ["c", 6],
  ["l", 2],
  ["m", 2],
  ["v", 4],
  ["y", 4],
]);
/** An inline image's data, from ID to EI (ISO 32000-2:2020, 8.9.7). */
const INLINE = /(?:(?<=[\0\t\n\f\r ])|^)ID[\0\t\n\f\r ][^]*?(?<=[\0\t\n\f\r ])EI(?=[\0\t\n\f\r ]|$)/gu;

/**
 * @typedef {{ high: number, left: number, low: number, right: number }} Area An axis-aligned area between two x and two y.
 * @typedef {{ api: Api, document: Document, marks: Area[] }} Scene A document and its marks in default user space.
 * @typedef {{ before: import("./decode.mjs").Image | undefined, document: Document, marks: Area[], saved: Uint8Array }} Applied
 */

/** Whether ±x±, ±y± is inside ±area± by more than ±MARGIN±. @type {(area: Area, x: number, y: number) => boolean} */
function holds(area, x, y) {
  return area.left + MARGIN < x && x < area.right - MARGIN && area.low + MARGIN < y && y < area.high - MARGIN;
}

/** Whether the unit square from ±column±, ±row± overlaps ±area±. @type {(area: Area, column: number, row: number) => boolean} */
function overlaps(area, column, row) {
  return column + 1 > area.left && column < area.right && row + 1 > area.low && row < area.high;
}

/** The numbers of ±x,y,w,h±. @type {(text: string) => number[]} */
function places(text) {
  const values = text.split(",").map(real);
  if (values.length !== 4) {
    throw new Error(`bad area '${text}'`);
  }
  return values;
}

/** The area ±x,y,w,h±. @type {(place: number[]) => Area} */
function area(place) {
  const [x = 0, y = 0, width = 0, height = 0] = place;
  return { high: y + height, left: x, low: y, right: x + width };
}

/** The page, area and colour, ±#rrggbb± or empty, of ±page@x,y,w,h[@rrggbb]±. @type {(text: string) => { colour: string, page: number, place: number[] }} */
function mark(text) {
  const [page = "", place = "", ...colour] = text.split("@");
  if (colour.length > 1 || (colour[0] !== undefined && !/^[0-9a-f]{6}$/u.test(colour[0]))) {
    throw new Error(`bad mark '${text}'`);
  }
  return { colour: colour[0] === undefined ? "" : `#${colour[0]}`, page: integer(page), place: places(place) };
}

/** ±values± rounded and comma-separated. @type {(values: readonly number[]) => string} */
function joined(values) {
  return values.map(rounded).join(",");
}

/** Page ±index±'s annotations as a case gives them, and its marks' areas (ISO 32000-2:2020, 12.5.6.23). @type {(document: Document, index: number) => Promise<[string, Area[]]>} */
async function listing(document, index) {
  const items = [];
  /** @type {Area[]} */
  const marks = [];
  for await (const annotation of await (await document.page({ index })).annotations()) {
    const info = await annotation.info();
    const place = [info.x, info.y, info.width, info.height];
    const quads = [...info.quads];
    let item = `${info.kind} ${joined(place)}`;
    if (quads.length > 0) {
      item += ` q=${joined(quads)}`;
    }
    if (info.color !== "") {
      item += ` c=${info.color.replace(/^#/u, "")}`;
    }
    items.push(item);
    if (info.kind !== "Redact") {
      continue;
    }
    if (quads.length === 0) {
      marks.push(area(place));
    }
    for (let start = 0; start < quads.length; start += 8) {
      const quad = quads.slice(start, start + 8);
      const xs = quad.filter((_value, at) => at % 2 === 0);
      const ys = quad.filter((_value, at) => at % 2 === 1);
      marks.push({ high: Math.max(...ys), left: Math.min(...xs), low: Math.min(...ys), right: Math.max(...xs) });
    }
  }
  return [items.join(";") || "none", marks];
}

/** Every page's annotations and marks. @type {(document: Document) => Promise<[string, Area[][]]>} */
async function survey(document) {
  const shown = [];
  const marks = [];
  for (let index = 0; index < (await document.pageCount()); index += 1) {
    const [listed, areas] = await listing(document, index);
    shown.push(listed);
    marks.push(areas);
  }
  return [shown.join("|"), marks];
}

/** The source of the file at ±path±, or the outcome when there is none. @type {(api: Api, environment: Environment, file: boolean, path: string) => Promise<Source | string>} */
async function sourceOf(api, environment, file, path) {
  return file ? environment.file(path) : api.Source.fromBytes(await environment.bytes(path));
}

/** Adds and deletes as ±args± say, saving if ±saved±; page ±index=±'s annotations. @type {(api: Api, environment: Environment, file: boolean, args: string[]) => Promise<string>} */
async function annotate(api, environment, file, args) {
  const [path, ...options] = args;
  if (path === undefined) {
    throw new Error("takes a path");
  }
  const given = parse(options, new Set([...OPEN, ...ANNOTATE]), ["index"]);
  const adding = ["x", "y", "width", "height", "quads", "color"].some((key) => given.has(key));
  if ((adding && !given.has("kind")) || (given.get("kind") ?? "Redact") !== "Redact") {
    throw new Error("a mark needs kind=Redact");
  }
  const made = await sourceOf(api, environment, file, path);
  if (typeof made === "string") {
    return made;
  }
  let document = await open(api, made, given);
  const index = integer(given.get("index"));
  const shown = await document.page({ index });
  if (given.has("kind")) {
    /** @type {(key: string) => number} */
    const value = (key) => (given.has(key) ? real(given.get(key)) : 0);
    const quads = given.get("quads");
    await shown.addAnnotation(null, {
      color: given.get("color") ?? "",
      height: value("height"),
      kind: "Redact",
      quads: quads === undefined ? [] : quads.split(",").map(real),
      width: value("width"),
      x: value("x"),
      y: value("y"),
    });
  }
  if (given.has("delete")) {
    const nth = integer(given.get("delete"));
    let at = 0;
    let deleted = false;
    for await (const annotation of await shown.annotations()) {
      if (at === nth) {
        await annotation.delete();
        deleted = true;
        break;
      }
      at += 1;
    }
    if (!deleted) {
      throw new Error(`no annotation ${nth}`);
    }
  }
  if (given.has("saved")) {
    document = await reopen(api, await twice(api, document, new Map()), given);
  }
  return (await listing(document, index))[0];
}

/** ±raw± decoded through the filters ±names±: none or FlateDecode (RFC 1950). @type {(names: string, raw: Uint8Array) => Uint8Array} */
function unfiltered(names, raw) {
  if (names === "none") {
    return raw;
  }
  if (names === "FlateDecode") {
    return inflate([raw]);
  }
  throw new Error(`a stream filtered by ${names}`);
}

/** The data of the stream ±id±, inflated by the runner. @type {(api: Api, document: Document, id: ObjectId) => Promise<Uint8Array>} */
async function decoded(api, document, id) {
  const dictionary = await objectAt(document, id);
  if (entry(dictionary, "DecodeParms") !== undefined) {
    throw new Error("a stream with DecodeParms");
  }
  return unfiltered(filters(entry(dictionary, "Filter")), await streamBytes(api, document, id, false));
}

/** The index of ±needle± in ±haystack± from ±from± on, or -1. @type {(haystack: Uint8Array, needle: Uint8Array, from: number) => number} */
function find(haystack, needle, from) {
  for (let at = from; at + needle.length <= haystack.length; at += 1) {
    if (needle.every((byte, offset) => haystack[at + offset] === byte)) {
      return at;
    }
  }
  return -1;
}

/** The bytes of ±text±, a character each. @type {(text: string) => Uint8Array} */
function latin1(text) {
  return Uint8Array.from(text, (character) => character.charCodeAt(0) & 0xff);
}

/** The data of each stream of the file ±saved±, decoded by the runner (ISO 32000-2:2020, 7.3.8). @type {(saved: Uint8Array) => Uint8Array[]} */
function streams(saved) {
  const decoder = new TextDecoder("latin1");
  const found = [];
  let from = 0;
  for (let at = find(saved, latin1("stream"), from); at >= 0; at = find(saved, latin1("stream"), from)) {
    const after = at + "stream".length;
    from = after;
    const head = decoder.decode(saved.subarray(0, at)).replace(/[\t\n\f\r ]+$/u, "");
    if (!head.endsWith(">>")) {
      continue;
    }
    const eol = saved[after] === 13 && saved[after + 1] === 10 ? 2 : saved[after] === 10 ? 1 : 0;
    if (eol === 0) {
      continue;
    }
    const end = find(saved, latin1("endstream"), after + eol);
    if (end < 0) {
      throw new Error("a stream without endstream");
    }
    from = end + "endstream".length;
    let data = saved.subarray(after + eol, end);
    const tail = data.length >= 2 && data[data.length - 2] === 13 && data[data.length - 1] === 10 ? 2 : 0;
    data = data.subarray(
      0,
      data.length - (tail || (data[data.length - 1] === 10 || data[data.length - 1] === 13 ? 1 : 0)),
    );
    const opened = head.lastIndexOf("obj");
    const dictionary = parseObject(tokens(head.slice(opened < 0 ? 0 : opened + "obj".length)), 0)[0];
    found.push(unfiltered(filters(entry(dictionary, "Filter")), data));
  }
  return found;
}

/** ±present± when ±needle± or its hex is in ±saved± or a stream of it decoded, else ±absent±. @type {(saved: Uint8Array, needle: Uint8Array) => string} */
function foundText(saved, needle) {
  const lower = hex(needle);
  const forms = [needle, latin1(lower), latin1(lower.toUpperCase())];
  const placesFound = [saved, ...streams(saved)];
  return placesFound.some((place) => forms.some((form) => find(place, form, 0) >= 0)) ? "present" : "absent";
}

/** The tokens of ±content±, inline images' data left out (ISO 32000-2:2020, 8.9.7). @type {(content: Uint8Array) => string[]} */
function contentTokens(content) {
  return tokens(new TextDecoder("latin1").decode(content).replace(INLINE, "ID EI"));
}

/** Whether ±word± is an operator (ISO 32000-2:2020, 7.8.2). @type {(word: string) => boolean} */
function isOperator(word) {
  return /^[A-Za-z'"]/u.test(word);
}

/** The page object of kid ±index± of the root page tree node. @type {(document: Document, index: number) => Promise<Obj>} */
async function kid(document, index) {
  const root = await objectAt(document, await document.catalog());
  const kids = await resolve(document, entry(await resolve(document, entry(root, "Pages")), "Kids"));
  return resolve(document, "array" in kids ? kids.array[index] : undefined);
}

/** The contents ±value± names, decoded, each followed by a newline. @type {(api: Api, document: Document, value: Obj | undefined) => Promise<Uint8Array>} */
async function contents(api, document, value) {
  const items = value !== undefined && "array" in value ? value.array : [value];
  const parts = [];
  for (const item of items) {
    if (item !== undefined && "ref" in item) {
      parts.push(...(await decoded(api, document, item.ref)), 10);
    }
  }
  return Uint8Array.from(parts);
}

/** The value ±key± names in ±category± of ±resources±, if any. @type {(document: Document, resources: Obj | undefined, category: string, key: string) => Promise<Obj | undefined>} */
async function named(document, resources, category, key) {
  if (resources === undefined) {
    return undefined;
  }
  const items = entry(await resolve(document, resources), category);
  return items === undefined ? undefined : entry(await resolve(document, items), key);
}

/** The last ±count± of ±operands± as numbers. @type {(operands: string[], count: number) => number[]} */
function numbers(operands, count) {
  if (operands.length < count) {
    throw new Error(`an operator takes ${count} numbers`);
  }
  return operands.slice(operands.length - count).map(real);
}

/** The product of ±first± and ±second± (ISO 32000-2:2020, 8.3.4). @type {(first: number[], second: number[]) => number[]} */
function product(first, second) {
  const [ma = 0, mb = 0, mc = 0, md = 0, me = 0, mf = 0] = first;
  const [ca = 0, cb = 0, cc = 0, cd = 0, ce = 0, cf = 0] = second;
  return [
    ma * ca + mb * cc,
    ma * cb + mb * cd,
    mc * ca + md * cc,
    mc * cb + md * cd,
    me * ca + mf * cc + ce,
    me * cb + mf * cd + cf,
  ];
}

/** The coordinates of the path operator ±word±, a corner each of ±re±. @type {(word: string, operands: string[]) => number[]} */
function pathPoints(word, operands) {
  const taken = COORDINATES.get(word);
  if (taken !== undefined) {
    return numbers(operands, taken);
  }
  if (word === "re") {
    const [x = 0, y = 0, width = 0, height = 0] = numbers(operands, 4);
    return [x, y, x + width, y, x, y + height, x + width, y + height];
  }
  return [];
}

/** How many points of ±coordinates±, mapped by ±ctm±, lie inside ±marks±. @type {(marks: Area[], ctm: number[], coordinates: number[]) => number} */
function count(marks, ctm, coordinates) {
  const [ma = 0, mb = 0, mc = 0, md = 0, me = 0, mf = 0] = ctm;
  let inside = 0;
  for (let at = 0; at + 1 < coordinates.length; at += 2) {
    const [x = 0, y = 0] = coordinates.slice(at, at + 2);
    if (marks.some((found) => holds(found, ma * x + mc * y + me, mb * x + md * y + mf))) {
      inside += 1;
    }
  }
  return inside;
}

/** The points of ±content± inside the marks, through cm and forms (ISO 32000-2:2020, 8.4.4, 8.10.1). @type {(scene: Scene, content: Uint8Array, resources: Obj | undefined, ctm: number[], depth: number) => Promise<number>} */
async function walk(scene, content, resources, ctm, depth) {
  /** @type {number[][]} */
  const states = [];
  let current = ctm;
  /** @type {string[]} */
  const operands = [];
  let inside = 0;
  for (const word of contentTokens(content)) {
    if (word === "q") {
      states.push(current);
    } else if (word === "Q") {
      current = states.pop() ?? current;
    } else if (word === "cm") {
      current = product(numbers(operands, 6), current);
    } else if (word === "Do") {
      inside += await form(scene, (operands.at(-1) ?? "").replace(/^\//u, ""), resources, current, depth);
    } else {
      inside += count(scene.marks, current, pathPoints(word, operands));
    }
    if (isOperator(word)) {
      operands.length = 0;
    } else {
      operands.push(word);
    }
  }
  return inside;
}

/** The points inside the marks of the form ±label± names, if it is one (ISO 32000-2:2020, 8.10.1). @type {(scene: Scene, label: string, resources: Obj | undefined, ctm: number[], depth: number) => Promise<number>} */
async function form(scene, label, resources, ctm, depth) {
  const found = await named(scene.document, resources, "XObject", label);
  if (found === undefined || !("ref" in found)) {
    throw new Error(`no XObject /${label}`);
  }
  const dictionary = await objectAt(scene.document, found.ref);
  const subtype = entry(dictionary, "Subtype");
  if (subtype === undefined || !("name" in subtype) || subtype.name !== "Form") {
    return 0;
  }
  if (depth >= DEPTH) {
    throw new Error("forms nest too deep");
  }
  const given = entry(dictionary, "Matrix");
  const matrix =
    given !== undefined && "array" in given ? given.array.map((item) => real(text(item))) : [1, 0, 0, 1, 0, 0];
  const content = await decoded(scene.api, scene.document, found.ref);
  return walk(scene, content, entry(dictionary, "Resources") ?? resources, product(matrix, ctm), depth + 1);
}

/** The path points of page ±index± strictly inside ±marks± (ISO 32000-2:2020, 8.5.2.1). @type {(api: Api, document: Document, index: number, marks: Area[]) => Promise<string>} */
async function points(api, document, index, marks) {
  const shown = await document.page({ index });
  if ((await shown.rotation()) !== 0) {
    throw new Error("points needs an unrotated page");
  }
  const crop = await shown.boundary({ kind: "Crop" });
  const top = crop.y + crop.height;
  const scene = {
    api,
    document,
    marks: marks.map((found) => ({
      high: top - found.low,
      left: crop.x + found.left,
      low: top - found.high,
      right: crop.x + found.right,
    })),
  };
  const pageObject = await kid(document, index);
  const content = await contents(api, document, entry(pageObject, "Contents"));
  return String(await walk(scene, content, entry(pageObject, "Resources"), [1, 0, 0, 1, 0, 0], 0));
}

/** Each image a Do of page ±index± paints, and its inline images (ISO 32000-2:2020, 8.8, 8.9.7). @type {(api: Api, document: Document, index: number) => Promise<[[ObjectId, Obj][], number]>} */
async function paintedImages(api, document, index) {
  const pageObject = await kid(document, index);
  const resources = entry(pageObject, "Resources");
  /** @type {string[]} */
  const operands = [];
  /** @type {[ObjectId, Obj][]} */
  const images = [];
  let inline = 0;
  for (const word of contentTokens(await contents(api, document, entry(pageObject, "Contents")))) {
    if (word === "BI") {
      inline += 1;
    }
    if (word === "Do") {
      const found = await named(document, resources, "XObject", (operands.at(-1) ?? "").replace(/^\//u, ""));
      if (found !== undefined && "ref" in found) {
        const image = await objectAt(document, found.ref);
        const subtype = entry(image, "Subtype");
        if (subtype !== undefined && "name" in subtype && subtype.name === "Image") {
          images.push([found.ref, image]);
        }
      }
    }
    if (isOperator(word)) {
      operands.length = 0;
    } else {
      operands.push(word);
    }
  }
  return [images, inline];
}

/** A stencil mask's samples, 1 for masked whatever its Decode (ISO 32000-2:2020, 8.9.6.2). @type {(dictionary: Obj, samples: Uint8Array) => Uint8Array} */
function stencil(dictionary, samples) {
  const ends = entry(dictionary, "Decode");
  if (
    ends === undefined ||
    !("array" in ends) ||
    ends.array.map((item) => decimal(real(text(item)))).join(" ") !== "1 0"
  ) {
    return samples;
  }
  const width = integer(text(entry(dictionary, "Width")));
  const stride = Math.max(Math.ceil(width / 8), 1);
  const flipped = Uint8Array.from(samples);
  for (let start = 0; start < flipped.length; start += stride) {
    for (let column = 0; column < width; column += 1) {
      const at = start + (column >> 3);
      if (column >> 3 < stride && at < flipped.length) {
        flipped[at] = (flipped[at] ?? 0) ^ (0x80 >> (column & 7));
      }
    }
  }
  return flipped;
}

/** The samples of an image's soft (ISO 32000-2:2020, 11.6.5.3), explicit (8.9.6.3) or stencil mask, or its key (8.9.6.4). @type {(api: Api, document: Document, image: Obj, id: ObjectId) => Promise<string>} */
async function maskSamples(api, document, image, id) {
  const soft = entry(image, "SMask");
  if (soft !== undefined && "ref" in soft) {
    return hex(await decoded(api, document, soft.ref));
  }
  const explicit = entry(image, "Mask");
  if (explicit !== undefined && "ref" in explicit) {
    return hex(stencil(await objectAt(document, explicit.ref), await decoded(api, document, explicit.ref)));
  }
  if (explicit !== undefined && "array" in explicit) {
    return `key ${explicit.array.map((item) => decimal(real(text(item)))).join(" ")}`;
  }
  const flag = entry(image, "ImageMask");
  if (flag !== undefined && "other" in flag && flag.other === "true") {
    return hex(stencil(image, await decoded(api, document, id)));
  }
  return "none";
}

/** What ±read=± gives of the ±do=±-th image page 0 paints. @type {(api: Api, document: Document, given: Map<string, string>) => Promise<string>} */
async function imageAfter(api, document, given) {
  const [images] = await paintedImages(api, document, 0);
  const at = integer(given.get("do"));
  const [id, image] = images[at] ?? [];
  if (id === undefined || image === undefined) {
    throw new Error(`no image ${at}`);
  }
  if (given.get("read") === "image_raw") {
    return sha256(await streamBytes(api, document, id, false));
  }
  if (given.get("read") === "image_samples") {
    return hex(await decoded(api, document, id));
  }
  return maskSamples(api, document, image, id);
}

/** Page ±index± rendered at the defaults and read back. @type {(api: Api, document: Document, index: number) => Promise<import("./decode.mjs").Image>} */
async function renderedImage(api, document, index) {
  const sink = await api.Sink.inMemory();
  await (await document.page({ index })).render(sink);
  return decode(await sink.takeBytes());
}

/** ±same± when the renders before and after agree outside ±excluded±, else how many pixels differ. @type {(api: Api, applied: Applied, index: number, excluded: Area[]) => Promise<string>} */
async function same(api, applied, index, excluded) {
  const [earlier, later] = [applied.before, await renderedImage(api, applied.document, index)];
  if (earlier === undefined || earlier.width !== later.width || earlier.height !== later.height) {
    throw new Error("no render before, or one of another size");
  }
  let differ = 0;
  for (let row = 0; row < later.height; row += 1) {
    for (let column = 0; column < later.width; column += 1) {
      if (excluded.some((found) => overlaps(found, column, row))) {
        continue;
      }
      const [one, two] = [pixel(earlier, column, row) ?? [], pixel(later, column, row) ?? []];
      if (one.length !== two.length || one.some((value, at) => value !== two[at])) {
        differ += 1;
      }
    }
  }
  return differ === 0 ? "same" : `${differ} differ`;
}

/** The bytes ±document± saves to. @type {(api: Api, document: Document) => Promise<Uint8Array>} */
async function snapshot(api, document) {
  const sink = await api.Sink.inMemory();
  await save(document, sink, new Map());
  return sink.takeBytes();
}

/** A redact case's keys, checked, and the progress calls ±cancel=N± lets through. @type {(options: string[], expected: string) => [Map<string, string>, number]} */
function redactGiven(options, expected) {
  const counts = options.filter((arg) => arg.startsWith("cancel=")).map((arg) => arg.slice("cancel=".length));
  if (!counts.every((found) => /^[0-9]+$/u.test(found))) {
    throw new Error(`bad cancel '${counts.join(" ")}'`);
  }
  const named = options.map((arg) => (arg.startsWith("cancel=") ? "cancel" : arg));
  const given = parse(named, new Set([...OPEN, ...REDACT]), ["read"]);
  const read = given.get("read") ?? "";
  const indexed = REDACT_READS.get(read);
  const code = /^[A-Za-z]+$/u.test(expected);
  if (
    indexed === undefined ||
    (given.has("index") && !indexed) ||
    given.has("bytes") !== (read === "text") ||
    given.get("bytes") === "" ||
    given.has("do") !== IMAGE_SAMPLES.has(read) ||
    (given.has("except") && read !== "pixels") ||
    (read === "render" && !code && !PIXELS.test(expected))
  ) {
    throw new Error(`keys that do not go together, or a bad expected value '${expected}'`);
  }
  return [given, counts.length === 0 ? 0 : integer(counts[0])];
}

/** What ±read=± gives of the document redacted. @type {(api: Api, applied: Applied, given: Map<string, string>, expected: string) => Promise<string>} */
async function redactRead(api, applied, given, expected) {
  const { document } = applied;
  const index = given.has("index") ? integer(given.get("index")) : 0;
  switch (given.get("read")) {
    case "ok":
      return "ok";
    case "text":
      return foundText(applied.saved, stringBytes(`<${given.get("bytes") ?? ""}>`));
    case "annotations":
      return (await listing(document, index))[0];
    case "thumb":
      return entry(await kid(document, index), "Thumb") === undefined ? "none" : "kept";
    case "painted": {
      const [images, inline] = await paintedImages(api, document, index);
      return `images=${images.length} inline=${inline}`;
    }
    case "points":
      return points(api, document, index, applied.marks);
    case "pixels": {
      const excluded = (given.get("except") ?? "")
        .split(";")
        .filter((item) => item !== "")
        .map((item) => area(places(item)));
      return same(api, applied, index, [...applied.marks, ...excluded]);
    }
    case "render": {
      const image = await renderedImage(api, document, index);
      return [
        `${image.width}x${image.height}`,
        ...expected
          .split(" ")
          .slice(1)
          .map((token) => matched(image, token)),
      ].join(" ");
    }
    default:
      return imageAfter(api, document, given);
  }
}

/** Adds the marks, applies them, saves, reopens and reads; a failed apply gives its code. @type {(api: Api, environment: Environment, file: boolean, args: string[], expected: string) => Promise<string>} */
async function redact(api, environment, file, args, expected) {
  const [path, ...options] = args;
  if (path === undefined) {
    throw new Error("takes a path");
  }
  const [given, allowed] = redactGiven(options, expected);
  const marks = given.has("marks") ? (given.get("marks") ?? "").split(";").map(mark) : [];
  const made = await sourceOf(api, environment, file, path);
  if (typeof made === "string") {
    return made;
  }
  const document = await open(api, made, given);
  for (const { colour, page: index, place } of marks) {
    const [x = 0, y = 0, width = 0, height = 0] = place;
    await (await document.page({ index })).addAnnotation(null, { color: colour, height, kind: "Redact", width, x, y });
  }
  const [listed, areas] = await survey(document);
  const index = given.has("index") ? integer(given.get("index")) : 0;
  const before = await snapshot(api, document);
  const shown = given.get("read") === "pixels" ? await renderedImage(api, document, index) : undefined;
  const pages = given.get("pages");
  let calls = 0;
  const stop = () => {
    calls += 1;
    return calls <= allowed;
  };
  try {
    await document.applyRedactions(
      given.has("cancel") ? stop : null,
      pages === undefined
        ? {}
        : {
            pages: pages
              .split(",")
              .filter((item) => item !== "")
              .map(integer),
          },
    );
  } catch (error) {
    if (!(error instanceof api.PdfError)) {
      throw error;
    }
    const after = await snapshot(api, document);
    const kept =
      after.length === before.length &&
      after.every((byte, at) => byte === before[at]) &&
      (await survey(document))[0] === listed;
    return kept ? error.code : `${error.code} changed`;
  }
  const saved = await twice(api, document, new Map(given.has("mode") ? [["mode", given.get("mode") ?? ""]] : []));
  /** @type {Applied} */
  const applied = { before: shown, document: await reopen(api, saved, given), marks: areas[index] ?? [], saved };
  return redactRead(api, applied, given, expected);
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
  const known =
    name !== undefined && (FAMILIES.has(name) || ["annotate", "redact", "render", "save", "split"].includes(name));
  if (!known && !["draw_image", "header_version", "insert_blank_page"].includes(operation)) {
    return `unknown operation '${operation}'`;
  }
  let actual;
  try {
    if (known && name === "render") {
      actual = await rendered(api, environment, kind === "file", args, expected);
    } else if (known && name === "redact") {
      actual = await redact(api, environment, kind === "file", args, expected);
    } else if (known && name === "annotate") {
      actual = await annotate(api, environment, kind === "file", args);
    } else if (known) {
      actual = await perform(api, environment, kind === "file", name, args);
    } else if (operation === "insert_blank_page") {
      actual = await inserted(api, args);
    } else if (operation === "draw_image") {
      actual = await drawn(api, environment, args);
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
