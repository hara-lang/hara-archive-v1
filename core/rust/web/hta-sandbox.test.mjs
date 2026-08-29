import assert from "node:assert/strict";
import test from "node:test";

import {
  BROWSER_WASM_SANDBOX_PROTOCOL,
  BrowserWasmSandbox,
  MCP_PURE_PROFILE,
  SANDBOX_EVAL_TARGET,
  projectBrowserSandboxValue,
  validateBrowserSandboxRequest,
} from "./packages/hta/sandbox.js";
import { HtaHandle, HtaKeyword, HtaMapEntry, HtaStruct } from "./packages/hta/index.js";

function cancellable(value) {
  let rejectPromise;
  const promise = new Promise((resolve, reject) => {
    rejectPromise = reject;
    queueMicrotask(() => resolve(value));
  });
  promise.cancel = () => {
    rejectPromise(new Error("cancelled"));
    return true;
  };
  return promise;
}

function harness({ result = 42, ready = Promise.resolve() } = {}) {
  const calls = [];
  const contexts = [];
  const workers = [];
  const workerFactory = (url) => {
    const worker = {
      url,
      terminations: 0,
      terminate() {
        this.terminations += 1;
      },
    };
    workers.push(worker);
    return worker;
  };
  const contextFactory = (options) => {
    const context = {
      options,
      ready,
      closes: 0,
      call(target, arguments_) {
        calls.push([target, arguments_]);
        return cancellable(result);
      },
      close() {
        this.closes += 1;
        options.worker.terminate();
      },
    };
    contexts.push(context);
    return context;
  };
  return { calls, contexts, workers, workerFactory, contextFactory };
}

function sandbox(overrides = {}) {
  return new BrowserWasmSandbox({
  workerUrl: new URL("file:///packages/hta/worker.mjs"),
    moduleBytes: new Uint8Array([0, 97, 115, 109]),
    ...overrides,
  });
}

test("validates the one-operation closed request", () => {
  const request = validateBrowserSandboxRequest({ operation: "sandbox.eval", source: "(+ 40 2)" });
  assert.equal(request.source, "(+ 40 2)");
  assert.equal(request.limits.wallMs, 5000);
  assert.equal(
    validateBrowserSandboxRequest({
      operation: "sandbox.eval",
      source: "1",
      limits: { sourceBytes: 128 },
    }).limits.sourceBytes,
    128,
  );
  assert.throws(
    () => validateBrowserSandboxRequest({ operation: "sandbox.call", source: "(+ 40 2)" }),
    { code: "sandbox/capability-unsupported" },
  );
  assert.throws(
    () => validateBrowserSandboxRequest({ operation: "sandbox.eval", source: "1", mode: "ROOT" }),
    { code: "sandbox/request-invalid" },
  );
});

test("rejects ambiguous runtime configuration and source overflow", () => {
  assert.throws(
    () => new BrowserWasmSandbox({ workerUrl: "worker.mjs" }),
    { code: "sandbox/config-invalid" },
  );
  assert.throws(
    () =>
      new BrowserWasmSandbox({
    workerUrl: "worker.mjs",
        moduleUrl: "hara.wasm",
        moduleBytes: new Uint8Array([0]),
      }),
    { code: "sandbox/config-invalid" },
  );
  assert.throws(
    () =>
      validateBrowserSandboxRequest({
        operation: "sandbox.eval",
        source: "x".repeat(65_537),
      }),
    { code: "sandbox/source-limit" },
  );
  assert.throws(
    () =>
      validateBrowserSandboxRequest({
        operation: "sandbox.eval",
        source: "1",
        limits: { wallMs: 30_001 },
      }),
    { code: "sandbox/limit-invalid" },
  );
  assert.throws(
    () =>
      validateBrowserSandboxRequest({
        operation: "sandbox.eval",
        source: "12",
        limits: { sourceBytes: 1 },
      }),
    { code: "sandbox/source-limit" },
  );
});

test("the deadline also covers worker initialization", async () => {
  let timer;
  let rejectReady;
  const state = harness();
  state.contextFactory = (options) => ({
    options,
    ready: new Promise((_resolve, reject) => {
      rejectReady = reject;
    }),
    close() {
      rejectReady(new Error("sandbox closed"));
    },
  });
  const instance = sandbox({
    workerFactory: state.workerFactory,
    contextFactory: state.contextFactory,
    setTimer(callback) {
      timer = callback;
      return 1;
    },
    clearTimer() {},
  });
  const pending = instance.run({ operation: "sandbox.eval", source: "(+ 40 2)" });
  await new Promise((resolve) => setImmediate(resolve));
  timer();
  await assert.rejects(pending, { code: "sandbox/timed-out" });
  assert.equal(instance.snapshot().state, "closed");
});

test("runs only the sandbox target with no host authority and closes the worker", async () => {
  const state = harness();
  const instance = sandbox({ workerFactory: state.workerFactory, contextFactory: state.contextFactory });
  const result = await instance.run({ operation: "sandbox.eval", source: "(+ 40 2)" });

  assert.deepEqual(state.calls, [[SANDBOX_EVAL_TARGET, ["(+ 40 2)"]]]);
  assert.deepEqual([...state.contexts[0].options.moduleBytes], [0, 97, 115, 109]);
  assert.deepEqual(state.contexts[0].options.hostCalls, {});
  assert.equal(state.contexts[0].options.filesystemHost, null);
  assert.equal(state.contexts[0].options.kernelId, null);
  assert.equal(state.contexts[0].closes, 1);
  assert.ok(state.workers[0].terminations >= 1);
  assert.deepEqual(result, {
    protocol: BROWSER_WASM_SANDBOX_PROTOCOL,
    profile: MCP_PURE_PROFILE,
    status: "completed",
    value: { text: "42", json: 42 },
    cleanup: "completed",
  });
  assert.equal(instance.snapshot().state, "closed");
  await assert.rejects(
    instance.run({ operation: "sandbox.eval", source: "1" }),
    { code: "sandbox/not-reusable" },
  );
});

test("cancellation closes the active context and worker", async () => {
  let rejectCall;
  const state = harness();
  state.contextFactory = (options) => {
    const context = {
      options,
      ready: Promise.resolve(),
      call(target, arguments_) {
        state.calls.push([target, arguments_]);
        const pending = new Promise((_resolve, reject) => {
          rejectCall = reject;
        });
        pending.cancel = () => {
          rejectCall(new Error("cancelled"));
          return true;
        };
        return pending;
      },
      close() {
        context.closes = (context.closes ?? 0) + 1;
        options.worker.terminate();
      },
    };
    state.contexts.push(context);
    return context;
  };
  const instance = sandbox({ workerFactory: state.workerFactory, contextFactory: state.contextFactory });
  const pending = instance.run({ operation: "sandbox.eval", source: "(+ 40 2)" });
  await new Promise((resolve) => setImmediate(resolve));
  assert.equal(instance.cancel(), true);
  await assert.rejects(pending, { code: "sandbox/cancelled" });
  assert.equal(instance.snapshot().state, "closed");
  assert.equal(state.contexts[0].closes, 1);
});

test("factory failures close a partially initialized sandbox", async () => {
  const worker = { terminations: 0, terminate() { this.terminations += 1; } };
  const instance = sandbox({
    workerFactory: () => worker,
    contextFactory: () => {
      throw new Error("context failed");
    },
  });
  await assert.rejects(
    instance.run({ operation: "sandbox.eval", source: "1" }),
    { message: "context failed" },
  );
  assert.equal(worker.terminations, 1);
  assert.equal(instance.snapshot().state, "closed");
});

test("projects bounded transfer-safe values and rejects live values", () => {
  assert.deepEqual(
    projectBrowserSandboxValue(
      new Map([
        ["answer", 42],
        [new HtaKeyword("status"), new HtaKeyword("ok")],
      ]),
      100,
    ),
    { text: '{"answer":42,":status":":ok"}', json: { answer: 42, ":status": ":ok" } },
  );
  assert.deepEqual(
    projectBrowserSandboxValue(new HtaMapEntry(new HtaKeyword("key"), 42), 100),
    { text: '[":key",42]', json: [":key", 42] },
  );
  assert.throws(() => projectBrowserSandboxValue(new HtaHandle("owner", "type", 1), 100), {
    code: "sandbox/result-non-transferable",
  });
  assert.throws(() => projectBrowserSandboxValue(new HtaStruct("Box", ["value"], [42]), 100), {
    code: "sandbox/result-non-transferable",
  });
  assert.throws(() => projectBrowserSandboxValue(Infinity, 100), {
    code: "sandbox/result-non-transferable",
  });
  assert.throws(() => projectBrowserSandboxValue(42, 1), { code: "sandbox/output-limit" });
  const projected = projectBrowserSandboxValue(new Map([["__proto__", 42]]), 100);
  assert.equal(projected.json["__proto__"], 42);
  assert.equal(Object.getPrototypeOf(projected.json), Object.prototype);
});
