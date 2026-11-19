// Runs a scenario of browser.mjs in this worker and posts its outcome back.
import { outcome } from "./browser.mjs";

addEventListener("message", (event) => {
  const { args, entry, scenario } = event.data;
  void outcome(scenario, entry, args).then((result) => {
    postMessage(result);
  });
});
