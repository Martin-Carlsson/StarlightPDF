// Runs the conformance suite (tests/conformance/*.tsv) through the package, in Node.js or a browser.

/**
 * @import { Api, Boundary, Document, DrawImageOptions, InfoDictionary, ObjectId, Page, SaveOptions, SaveReport, Sink, Source, SplitOptions } from "./api.mts"
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
const FLAGS = new Set([
  "allow_loss",
  "break_signatures",
  "cancel",
  "decode",
  "ignore_orientation",
  "layer",
  "saved",
  "strict",
]);
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
  "format",
  "height",
  "ignore_orientation",
  "layer",
  "mask",
  "opacity",
  "page_height",
  "page_width",
  "read",
  "width",
  "x",
  "y",
];
/** Each `key=` of `info`. @type {(keyof InfoDictionary)[]} */
const INFO = ["author", "created", "creator", "keywords", "modified", "producer", "subject", "title"];
/** Each `read=` of `draw_image`. */
const IMAGE_READS = new Set([
  "bits",
  "color_space",
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

/** Opens `made` with the open options `given`. @type {(api: Api, made: Source, given: Map<string, string>) => Promise<Document>} */
async function open(api, made, given) {
  const limits = Object.fromEntries(
    LIMITS.filter(([key]) => given.has(key)).map(([key, name]) => [name, integer(given.get(key))]),
  );
  const policy = Object.keys(limits).length === 0 ? null : await api.ResourcePolicy.create(limits);
  return api.Document.open(made, policy, {
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

/** The CTM at the `Do` painting the image: the product of every `cm` in effect (ISO 32000-2:2020, 8.3.4, 8.4.4). @type {(api: Api, document: Document, pageObject: Obj) => Promise<string>} */
async function matrix(api, document, pageObject) {
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
      return current.map(rounded).join(" ");
    }
    if (/^[A-Za-z'"]/u.test(word)) {
      operands.length = 0;
    } else {
      operands.push(word);
    }
  }
  throw new Error("no Do in the page's content");
}

/** The `Filter` names space-separated, or `none` (ISO 32000-2:2020, 7.4). @type {(value: Obj | undefined) => string} */
function filters(value) {
  if (value === undefined) {
    return "none";
  }
  return ("array" in value ? value.array : [value]).map(text).join(" ");
}

/** What `read` gives of the one image on the first page, found from the catalog. @type {(api: Api, document: Document, read: string) => Promise<string>} */
async function imageRead(api, document, read) {
  const root = await objectAt(document, await document.catalog());
  const kids = await resolve(document, entry(await resolve(document, entry(root, "Pages")), "Kids"));
  const pageObject = await resolve(document, "array" in kids ? kids.array[0] : undefined);
  const resources = await resolve(document, entry(pageObject, "Resources"));
  const xobjects = await resolve(document, entry(resources, "XObject"));
  const found = "dict" in xobjects ? [...xobjects.dict.values()][0] : undefined;
  if (found === undefined || !("ref" in found)) {
    throw new Error("no image XObject on the page");
  }
  const image = await objectAt(document, found.ref);
  /** @type {(id: ObjectId | undefined) => Promise<string>} */
  const digest = async (id) => (id === undefined ? "none" : sha256(await streamBytes(api, document, id, true)));
  /** @type {(key: string) => string} */
  const number = (key) => decimal(Number(text(entry(image, key))));
  const mask = entry(image, "SMask");
  switch (read) {
    case "matrix":
      return matrix(api, document, pageObject);
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
  const document = await api.Document.create();
  const created = await document.insertBlankPage({
    ...(pageHeight === undefined ? {} : { height: pageHeight }),
    ...(pageWidth === undefined ? {} : { width: pageWidth }),
  });
  const mask = given.has("mask") ? await api.Source.fromBytes(await environment.bytes(given.get("mask") ?? "")) : null;
  const layer = given.has("layer") ? await document.addLayer({ name: "layer" }) : null;
  /** @type {(key: string) => number | undefined} */
  const optional = (key) => (given.has(key) ? real(given.get(key)) : undefined);
  const [x, y, width, height, opacity] = ["x", "y", "width", "height", "opacity"].map(optional);
  const format = given.get("format");
  /** @type {DrawImageOptions} */
  const drawing = {
    ignoreOrientation: given.has("ignore_orientation"),
    ...(format === undefined ? {} : { format }),
    ...(height === undefined ? {} : { height }),
    ...(opacity === undefined ? {} : { opacity }),
    ...(width === undefined ? {} : { width }),
    ...(x === undefined ? {} : { x }),
    ...(y === undefined ? {} : { y }),
  };
  await created.drawImage(await api.Source.fromBytes(await environment.bytes(path)), mask, layer, drawing);
  return imageRead(api, await reopen(api, await twice(api, document, new Map()), new Map()), read);
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
  const known = name !== undefined && (FAMILIES.has(name) || name === "save" || name === "split");
  if (!known && !["draw_image", "header_version", "insert_blank_page"].includes(operation)) {
    return `unknown operation '${operation}'`;
  }
  let actual;
  try {
    if (known) {
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
