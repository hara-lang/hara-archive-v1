import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

import { HtaContext } from "./packages/hta/index.js";
import { KernelBroker } from "./host/broker.js";
import { createHostServices } from "./host/services.js";
import { CanvasRuntime } from "./studio/canvas-runtime.js";

const wasmUrl = new URL(
  "../raw/target/wasm32-unknown-unknown/browser-release/hara-wasm-vm.wasm",
  import.meta.url
);
const wasmBytes = await readFile(wasmUrl).catch(() => null);
const hal = (path) => readFile(new URL(path, import.meta.url), "utf8");

async function substrateResources() {
  const resources = {};
  for (const name of [
    "core", "frame", "json", "protocol", "pubsub", "request", "router",
    "space", "transport_memory", "util", "util_handlers"
  ]) {
    resources[`std.substrate.${name.replaceAll("_", "-")}`] = await hal(
      `../../lib/src/std/substrate/${name}.hal`
    );
  }
  resources["std.substrate"] = await hal("../../lib/src/std/substrate.hal");
  return resources;
}

const resources = wasmBytes === null
  ? null
  : {
      "studio.node": await hal("./studio/hal/node.hal"),
      "studio.draw": await hal("./studio/hal/draw.hal"),
      ...await substrateResources()
    };

let kernelCounter = 0;
async function spawnRealKernel(hostCalls) {
  kernelCounter += 1;
  const bridge = { listeners: {}, selfListeners: {} };
  bridge.self = {
    addEventListener: (type, handler) => {
      bridge.selfListeners[type] = handler;
    },
    postMessage: (data) => bridge.listeners.message?.({ data }),
    close: () => {}
  };
  globalThis.self = bridge.self;
  await import(`./packages/hta/worker.mjs?pong=${kernelCounter}`);
  const worker = {
    addEventListener: (type, handler) => {
      bridge.listeners[type] = handler;
    },
    postMessage: (message) => bridge.selfListeners.message({ data: message }),
    terminate() {}
  };
  return { context: new HtaContext({ worker, moduleBytes: wasmBytes, hostCalls }), worker };
}

function canvasFixture() {
  const callbacks = new Map();
  const calls = [];
  let nextFrame = 0;
  const listeners = new Map();
  const context = new Proxy({}, {
    get(target, property) {
      if (!(property in target)) target[property] = (...args) => calls.push([property, ...args]);
      return target[property];
    },
    set(target, property, value) {
      calls.push([property, value]);
      target[property] = value;
      return true;
    }
  });
  const canvas = {
    width: 1,
    height: 1,
    clientWidth: 320,
    clientHeight: 180,
    getContext: (kind) => kind === "2d" ? context : null
  };
  const window = {
    devicePixelRatio: 1,
    addEventListener: (name, handler) => listeners.set(name, handler),
    removeEventListener: (name) => listeners.delete(name),
    document: { createElement: () => ({ getContext: () => null }) }
  };
  const runtime = new CanvasRuntime({
    window,
    capabilities: ["canvas/2d"],
    requestFrame: (callback) => {
      const token = ++nextFrame;
      callbacks.set(token, callback);
      return token;
    },
    cancelFrame: (token) => callbacks.delete(token)
  });
  runtime.register("canvas/background", canvas);
  return { runtime, callbacks, calls };
}

async function takeAnimationFrame(callbacks) {
  for (let attempt = 0; attempt < 200; attempt += 1) {
    const entry = callbacks.entries().next().value;
    if (entry) {
      const [token, callback] = entry;
      callbacks.delete(token);
      callback(16.6);
      return;
    }
    await new Promise((resolve) => setTimeout(resolve, 1));
  }
  throw new Error("Pong did not request its first animation frame");
}

const pongStartupSource = `(ns+)

(require [studio.draw :as draw])

(node/start
  (fn []
    (let [frame (co/await (draw/next-frame "canvas/background"))]
      (co/await
        (draw/render
          "canvas/background"
          {:type :canvas-2d
           :background "#02050b"
           :commands
           [[:rect 0 0
             (get frame "canvas/width")
             (get frame "canvas/height")
             "#41f5e4" 1.0]]})))))`;

test("Pong restores its captured node before the deferred canvas task runs", {
  skip: wasmBytes === null
}, async () => {
  const { runtime, callbacks, calls } = canvasFixture();
  const hostCalls = createHostServices({ canvasRuntime: runtime });
  const broker = new KernelBroker({
    resources,
    spawn: () => spawnRealKernel(hostCalls)
  });
  const nodeId = "node/pong-regression";
  const canvasId = "canvas/background";
  const candidate = await broker.prepareDocument(
    "ROOT",
    "test/pong-canvas-startup",
    pongStartupSource,
    { nodeId }
  );

  assert.match(candidate.value, /^task-/);

  // Deferred tasks execute in a separate HTA evaluation. Simulate the dynamic
  // evaluator boundary explicitly: the task must carry its originating node
  // rather than depend on this mutable root surviving between evaluations.
  await candidate.context.eval("(set! studio.node/*node-id* nil)");

  runtime.stage(nodeId, canvasId);
  const rendered = runtime.waitForFirstRender(nodeId, canvasId, 2000);
  const task = broker.evalPreparedDocument(
    candidate,
    `(studio.node/run-task ${JSON.stringify(candidate.value)})`
  );

  await takeAnimationFrame(callbacks);
  await Promise.all([rendered, task]);

  assert.ok(calls.some(([name]) => name === "fillRect"));
  const lastFrame = runtime.canvases.get(canvasId).lastFrame;
  assert.equal(lastFrame?.type?.name, "canvas-2d");
  assert.equal(Array.isArray(lastFrame?.commands), true);

  runtime.discard(nodeId, canvasId);
  broker.discardDocument(candidate);
  runtime.close();
});
