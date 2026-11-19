// Installs the packed package into a temporary directory and serves it to headless Chrome.
import { spawn } from "node:child_process";
import { existsSync } from "node:fs";
import { mkdtemp, readdir, readFile, rm, writeFile } from "node:fs/promises";
import { createServer, type IncomingMessage, type ServerResponse } from "node:http";
import { tmpdir } from "node:os";
import { delimiter, dirname, extname, join, relative, resolve, sep } from "node:path";
import { after } from "node:test";
import { pathToFileURL } from "node:url";
import type { Api } from "./api.mts";

/** The repository root. */
export const ROOT = resolve(import.meta.dirname, "..", "..", "..");

/** A corpus file of three pages (tests/conformance/page_count.tsv). */
export const NESTED = "tests/corpus/page-count/nested-tree.pdf";

/** The longest a browser page may take to report. */
const TIMEOUT = 300_000;

/** These tests' directory as the test server serves it. */
const HERE = `/repo/${relative(ROOT, import.meta.dirname)
  .split(sep)
  .join("/")}`;

/** Media types the test server sends. */
const TYPES = new Map([
  [".html", "text/html; charset=utf-8"],
  [".js", "text/javascript"],
  [".mjs", "text/javascript"],
  [".wasm", "application/wasm"],
]);

/** Chrome's switches: headless, and no network or prompts of its own. */
const SWITCHES = [
  "--headless=new",
  "--no-first-run",
  "--no-default-browser-check",
  "--disable-background-networking",
  "--disable-component-update",
  "--disable-default-apps",
  "--disable-extensions",
  "--disable-sync",
  "--use-mock-keychain",
];

/** The package as installed from its tarball. */
export interface Installed {
  /** Its exports, loaded in this process. */
  readonly api: Api;
  /** The directory it is installed in, beside `node_modules`. */
  readonly dir: string;
  /** Its own directory. */
  readonly pkg: string;
}

/** What a browser page reported and every path it requested, in order. */
export interface Browsed {
  readonly requests: readonly string[];
  readonly result: unknown;
}

/** Temporary directories to remove when the file's tests end. */
const temporary: string[] = [];
after(async () => {
  await Promise.all(temporary.map(remove));
});

let installing: Promise<Installed> | undefined;
let pages = 0;

/** Removes `path` and everything in it. */
async function remove(path: string): Promise<void> {
  await rm(path, { force: true, maxRetries: 10, recursive: true });
}

/** Runs `file` with `args` in `cwd`; its output, or a rejection with it unless it exits 0. */
export async function exec(
  file: string,
  args: readonly string[],
  cwd: string,
  env: Record<string, string | undefined> = process.env,
): Promise<string> {
  const child = spawn(file, args, { cwd, env, stdio: ["ignore", "pipe", "pipe"] });
  let output = "";
  for (const stream of [child.stdout, child.stderr]) {
    const decoder = new TextDecoder();
    stream?.on("data", (chunk) => {
      output += decoder.decode(chunk, { stream: true });
    });
  }
  const code = await new Promise<number | null>((done, fail) => {
    child.once("error", fail);
    child.once("close", done);
  });
  if (code !== 0) {
    throw new Error(`${[file, ...args].join(" ")} exited with ${code}:\n${output}`);
  }
  return output;
}

/** npm's script `name` (`npm-cli.js` or `npx-cli.js`) shipped with this Node.js. */
export function npm(name: string): string {
  const bin = dirname(process.execPath);
  const found = [
    join(bin, "node_modules", "npm", "bin", name),
    join(bin, "..", "lib", "node_modules", "npm", "bin", name),
  ].find((path) => existsSync(path));
  if (found === undefined) {
    throw new Error(`no npm beside ${process.execPath}`);
  }
  return found;
}

/** This process's environment without Rust: no Cargo or rustup variables, no PATH entry with their tools. */
function withoutRust(): Record<string, string | undefined> {
  const exe = process.platform === "win32" ? ".exe" : "";
  const env: Record<string, string | undefined> = {};
  for (const [key, value] of Object.entries(process.env)) {
    if (/^(?:cargo|rust)/iu.test(key)) {
      continue;
    }
    env[key] =
      /^path$/iu.test(key) && value !== undefined
        ? value
            .split(delimiter)
            .filter((dir) => !["cargo", "rustc", "rustup"].some((tool) => existsSync(join(dir, tool + exe))))
            .join(delimiter)
        : value;
  }
  return env;
}

/** Installs the one tarball in target/javascript offline, without Rust or install scripts. */
async function install(): Promise<Installed> {
  const out = join(ROOT, "target", "javascript");
  const tarballs = (await readdir(out)).filter((name) => /^starlightpdf-.+\.tgz$/u.test(name));
  const [tarball] = tarballs;
  if (tarball === undefined || tarballs.length !== 1) {
    throw new Error(`${out}: expected one starlightpdf-*.tgz, found ${tarballs.length}`);
  }
  const dir = await mkdtemp(join(tmpdir(), "starlightpdf-"));
  temporary.push(dir);
  await writeFile(join(dir, "package.json"), JSON.stringify({ private: true, type: "module" }));
  await exec(
    process.execPath,
    [
      npm("npm-cli.js"),
      "install",
      "--offline",
      "--ignore-scripts",
      "--no-audit",
      "--no-fund",
      "--no-package-lock",
      join(out, tarball),
    ],
    dir,
    withoutRust(),
  );
  await writeFile(join(dir, "load.mjs"), 'export * as api from "starlightpdf";\n');
  const loaded: { readonly api: Api } = await import(pathToFileURL(join(dir, "load.mjs")).href);
  return { api: loaded.api, dir, pkg: join(dir, "node_modules", "starlightpdf") };
}

/** The package, installed once per test file. */
export async function installed(): Promise<Installed> {
  installing ??= install();
  return installing;
}

/** The bytes of the repository file at `path`. */
export async function corpus(path: string): Promise<Uint8Array> {
  return new Uint8Array(await readFile(join(ROOT, path)));
}

/** Google Chrome: `CHROME`, else where it installs on Linux, macOS or Windows. */
function chrome(): string {
  const env = process.env;
  const found = [
    env["CHROME"],
    "/usr/bin/google-chrome",
    "/usr/bin/google-chrome-stable",
    "/opt/google/chrome/chrome",
    "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
    ...[env["PROGRAMFILES"], env["PROGRAMFILES(X86)"], env["LOCALAPPDATA"]].map((base) =>
      base === undefined ? undefined : join(base, "Google", "Chrome", "Application", "chrome.exe"),
    ),
  ].find((path) => path !== undefined && existsSync(path));
  if (found === undefined) {
    throw new Error("Google Chrome not found; set CHROME to its executable");
  }
  return found;
}

/** Serves `/repo/…` from the repository and the rest from `dir`; a POST to `/result` reports. */
async function respond(
  dir: string,
  request: IncomingMessage,
  response: ServerResponse,
  requests: string[],
  report: (body: unknown) => void,
): Promise<void> {
  const path = decodeURIComponent(new URL(request.url ?? "/", "http://localhost").pathname);
  requests.push(path);
  if (request.method === "POST" && path === "/result") {
    const decoder = new TextDecoder();
    let body = "";
    for await (const chunk of request) {
      body += decoder.decode(chunk, { stream: true });
    }
    report(JSON.parse(body + decoder.decode()));
  }
  const [base, rest] = path.startsWith("/repo/") ? [ROOT, path.slice(6)] : [dir, path.slice(1)];
  const file = resolve(base, rest);
  const body = file.startsWith(base + sep) ? await readFile(file).catch(() => undefined) : undefined;
  if (request.method !== "GET" || body === undefined) {
    response.writeHead(request.method === "GET" ? 404 : 204).end();
    return;
  }
  const type = TYPES.get(extname(file)) ?? "application/octet-stream";
  response.writeHead(200, { "cache-control": "no-store", "content-type": type }).end(body);
}

/** Opens `html` in a fresh headless Chrome; its report `{ ok }`, or a rejection. */
export async function browse(installed: Installed, html: string): Promise<Browsed> {
  pages += 1;
  const name = `page-${pages}.html`;
  await writeFile(join(installed.dir, name), html);
  const requests: string[] = [];
  const reported = Promise.withResolvers<unknown>();
  const server = createServer((request, response) => {
    respond(installed.dir, request, response, requests, reported.resolve).catch(reported.reject);
  });
  await new Promise<void>((done) => {
    server.listen(0, "127.0.0.1", done);
  });
  const address = server.address();
  const port = typeof address === "object" && address !== null ? address.port : 0;
  const profile = await mkdtemp(join(tmpdir(), "starlightpdf-chrome-"));
  const browser = spawn(chrome(), [...SWITCHES, `--user-data-dir=${profile}`, `http://127.0.0.1:${port}/${name}`], {
    stdio: "ignore",
  });
  const exited = new Promise<void>((done) => {
    browser.once("exit", () => {
      done();
    });
    browser.once("error", () => {
      done();
    });
  });
  let timer = 0;
  try {
    const result = await Promise.race([
      reported.promise,
      exited.then(() => {
        throw new Error("Chrome exited");
      }),
      new Promise((_, fail) => {
        timer = setTimeout(() => {
          fail(new Error(`no report in ${TIMEOUT} ms`));
        }, TIMEOUT);
      }),
    ]);
    if (typeof result === "object" && result !== null && "ok" in result) {
      return { requests, result: result.ok };
    }
    throw new Error(`the page reported ${JSON.stringify(result)}`);
  } catch (error) {
    throw new Error(`${String(error)}\nrequested:\n${requests.join("\n")}`);
  } finally {
    clearTimeout(timer);
    if (browser.exitCode === null && browser.signalCode === null) {
      browser.kill();
    }
    await exited;
    server.closeAllConnections();
    await new Promise<void>((done) => {
      server.close(done);
    });
    await remove(profile);
  }
}

/** A page that runs `module` and reports any uncaught failure. */
function page(module: string): string {
  return `<!doctype html>
<meta charset="utf-8">
<title>starlightpdf</title>
<script>
  const fail = (error) => fetch("/result", { method: "POST", body: JSON.stringify({ error: String(error) }) });
  addEventListener("error", (event) => fail(event.error ?? event.message ?? "a script failed to load"), true);
  addEventListener("unhandledrejection", (event) => fail(event.reason));
</script>
<script type="module">
${module}
</script>
`;
}

/** A page that runs `name` of browser.mjs with `args`, loading the package as static files. */
export function scenario(name: string, args: unknown = null): string {
  return page(`import { main } from ${JSON.stringify(`${HERE}/browser.mjs`)};
await main(${JSON.stringify(name)}, "/node_modules/starlightpdf/index.js", ${JSON.stringify(args)});`);
}
