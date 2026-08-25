import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

import { HtaContext } from "./packages/hta/index.js";

const wasmUrl = new URL(
  "../raw/target/wasm32-unknown-unknown/browser-release/hara-wasm-core.wasm",
  import.meta.url,
);
const runnerUrl = new URL(
  "../../lib/test-fixtures/std/foundation/exception_conformance.hal",
  import.meta.url,
);
const corpusUrl = new URL(
  "../../../../hara-specs-registry/01-lang/001-language/draft/conformance/exceptions.edn",
  import.meta.url,
);

const wasmBytes = await readFile(wasmUrl).catch(() => null);

async function spawnKernel() {
  const bridge = { listeners: {}, selfListeners: {} };
  bridge.self = {
    addEventListener: (type, handler) => {
      bridge.selfListeners[type] = handler;
    },
    postMessage: (data) => bridge.listeners.message?.({ data }),
    close: () => {},
  };
  globalThis.self = bridge.self;
  await import(`./packages/hta/worker.mjs?exception-conformance=${Date.now()}`);
  const worker = {
    addEventListener: (type, handler) => {
      bridge.listeners[type] = handler;
    },
    postMessage: (message) => bridge.selfListeners.message({ data: message }),
    terminate: () => {},
  };
  return new HtaContext({ worker, moduleBytes: wasmBytes });
}

test(
  "portable exception corpus runs through the production raw WASM boundary",
  { skip: wasmBytes === null },
  async () => {
    const [runner, corpus] = await Promise.all([
      readFile(runnerUrl, "utf8"),
      readFile(corpusUrl, "utf8"),
    ]);
    const context = await spawnKernel();
    try {
      const result = await context.call("eval-bound", [
        `${runner}\n(exception-conformance-run __hta_arg_0)`,
        [corpus],
      ]);
      if (result !== 42) {
        const failure = await context.call("eval-bound", [
          "(exception-conformance-failure (read-string __hta_arg_0))",
          [corpus],
        ]);
        assert.equal(failure, null, `WASM exception conformance failed: ${failure}`);
      }
      const directCases = await context.call("eval-bound", [
        "(exception-conformance-direct-cases (read-string __hta_arg_0))",
        [corpus],
      ]);
      for (const directCase of directCases.values) {
        const [id, source] = directCase.values;
        assert.equal(await context.call("eval", [source]), 42, id.name);
      }
    } finally {
      context.close();
    }
  },
);
