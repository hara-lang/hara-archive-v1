import assert from "node:assert/strict";
import { mkdir, readFile, writeFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import test from "node:test";

import { HtaContext, HtaKeyword } from "./packages/hta/index.js";

const wasmUrl = new URL(
  "../raw/target/wasm32-unknown-unknown/browser-release/hara-wasm-core.wasm",
  import.meta.url,
);
const runnerUrl = new URL(
  "../../lib/test-fixtures/std/foundation/stack_safety_conformance.hal",
  import.meta.url,
);
const corpusUrl = new URL(
  "../../../../hara-specs-registry/01-lang/001-language/draft/conformance/stack-safety.edn",
  import.meta.url,
);
const reportUrl = new URL("../../target/conformance/wasm/stack-safety.edn", import.meta.url);

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
  await import(`./packages/hta/worker.mjs?stack-safety-conformance=${Date.now()}`);
  const worker = {
    addEventListener: (type, handler) => {
      bridge.listeners[type] = handler;
    },
    postMessage: (message) => bridge.selfListeners.message({ data: message }),
    terminate: () => {},
  };
  return new HtaContext({ worker, moduleBytes: wasmBytes });
}

function mapValue(map, name) {
  const entries = map instanceof Map ? map : map.entries;
  for (const [key, value] of entries) {
    if (key instanceof HtaKeyword && key.name === name) return value;
  }
  return undefined;
}

function reportSource(results) {
  const cases = results.map((result) => {
    const id = mapValue(result, "id");
    const status = mapValue(result, "status");
    return `{:id :${id.name} :status :${status.name}}`;
  });
  const passed = results.filter((result) => mapValue(result, "status")?.name === "passed").length;
  const status = passed === results.length ? ":passed" : ":failed";
  return `{ :report/schema :hara.conformance.runtime/0-alpha :report/suite :hal/stack-safety :report/runtime :wasm :report/status ${status} :report/passed ${passed} :report/total ${results.length} :report/cases [${cases.join(" ")}] }\n`;
}

test(
  "shared stack-safety corpus runs through the production raw WASM boundary",
  { skip: wasmBytes === null },
  async () => {
    const [runner, corpus] = await Promise.all([
      readFile(runnerUrl, "utf8"),
      readFile(corpusUrl, "utf8"),
    ]);
    const context = await spawnKernel();
    try {
      const result = await context.call("eval-bound", [
        `${runner}\n(stack-safety-conformance-run __hta_arg_0)`,
        [corpus],
      ]);
      const results = await context.call("eval-bound", [
        "(stack-safety-conformance-results (read-string __hta_arg_0))",
        [corpus],
      ]);
      assert.ok(Array.isArray(results));
      assert.equal(results.length, 9);
      if (result !== 42) {
        const failure = await context.call("eval-bound", [
          "(stack-safety-conformance-failure (read-string __hta_arg_0))",
          [corpus],
        ]);
        assert.equal(failure, null, `WASM stack-safety conformance failed: ${failure}`);
      }
      const reportDirectory = fileURLToPath(new URL("../../target/conformance/wasm/", import.meta.url));
      await mkdir(reportDirectory, { recursive: true });
      await writeFile(fileURLToPath(reportUrl), reportSource(results));
    } finally {
      context.close();
    }
  },
);
