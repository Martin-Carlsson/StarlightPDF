// The worker a page's main thread runs the core in; each call brings the port to answer on.
import type { Name } from "./core.js";

const core = import("./core.js");

addEventListener("message", (event: MessageEvent<{ arg: never; name: Name; port: MessagePort }>) => {
  const { arg, name, port } = event.data;
  core
    .then(
      ({ perform }) => ({ ok: perform(name, arg) }),
      (error: unknown) => ({ unloaded: `the StarlightPDF WebAssembly failed to load: ${String(error)}` }),
    )
    .then(
      (reply) => {
        port.postMessage(reply);
      },
      (error: unknown) => {
        port.postMessage({ error });
      },
    );
});
