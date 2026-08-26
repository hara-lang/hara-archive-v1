import assert from "node:assert/strict";
import test from "node:test";
import { HtaKeyword, HtaObject } from "./packages/hta/index.js";
import { CanvasRuntime, resolutionUniform } from "./studio/canvas-runtime.js";

function fixture() {
  const callbacks = new Map();
  let next = 0;
  const listeners = new Map();
  const calls = [];
  const gradient = {
    addColorStop: (...args) => calls.push(["addColorStop", ...args])
  };
  const context = new Proxy({}, {
    get(target, property) {
      if (property === "createRadialGradient") {
        return (...args) => {
          calls.push([property, ...args]);
          return gradient;
        };
      }
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
    devicePixelRatio: 2,
    addEventListener: (name, handler) => listeners.set(name, handler),
    removeEventListener: (name) => listeners.delete(name),
    document: { createElement: () => ({ getContext: () => null }) }
  };
  const runtime = new CanvasRuntime({
    window,
    requestFrame: (callback) => { const id = ++next; callbacks.set(id, callback); return id; },
    cancelFrame: (id) => callbacks.delete(id)
  });
  runtime.register("canvas/background", canvas);
  return { runtime, canvas, calls, callbacks, listeners };
}

test("next-frame returns integer geometry, timing, and active input", async () => {
  const { runtime, callbacks, listeners } = fixture();
  runtime.claim("node/tron@1", "canvas/background");
  listeners.get("keydown")({
    key: "ArrowLeft", code: "ArrowLeft", repeat: false,
    ctrlKey: false, altKey: false, shiftKey: false, metaKey: false
  });
  const pending = runtime.nextFrame("node/tron@1", "canvas/background");
  callbacks.values().next().value(18.75);
  const frame = await pending;
  assert.equal(frame.get("frame/time-ms"), 18);
  assert.equal(frame.get("canvas/width"), 320);
  assert.equal(frame.get("canvas/pixel-ratio-milli"), 2000);
  assert.equal(frame.get("input/events").length, 1);
});

test("WebGL resolution uniforms use the physical Retina backing store", () => {
  assert.deepEqual(
    resolutionUniform("u_resolution", [320, 180], 640, 360),
    [640, 360]
  );
  assert.deepEqual(
    resolutionUniform("iResolution", [320, 180, 1], 640, 360),
    [640, 360, 1]
  );
  assert.deepEqual(
    resolutionUniform("u_pointer", [20, 30], 640, 360),
    [20, 30]
  );
});

test("only the active generation can render and replacement cancels its frame", async () => {
  const { runtime } = fixture();
  runtime.claim("node/tron@1", "canvas/background");
  const pending = runtime.nextFrame("node/tron@1", "canvas/background");
  runtime.claim("node/tron@2", "canvas/background");
  await assert.rejects(pending, /ownership replaced/);
  assert.throws(
    () => runtime.render("node/tron@1", "canvas/background", new Map()),
    /does not own/
  );
});

test("semantic canvas aliases cannot be owned by competing generations", async () => {
  const { runtime, canvas } = fixture();
  runtime.register("canvas/visualizer", canvas);
  runtime.claim("node/tron@1", "canvas/background");
  const pending = runtime.nextFrame("node/tron@1", "canvas/background");
  runtime.claim("node/fft@1", "canvas/visualizer");
  await assert.rejects(pending, /canvas surface ownership replaced/);
  assert.throws(
    () => runtime.render("node/tron@1", "canvas/background", new Map()),
    /does not own/
  );
});

test("Canvas2D frames execute declared commands without game-specific state", () => {
  const { runtime, calls } = fixture();
  runtime.claim("node/grid@1", "canvas/background");
  runtime.render("node/grid@1", "canvas/background", new Map([
    ["type", { constructor: { name: "HtaKeyword" }, name: "canvas-2d" }],
    ["background", "#020408"],
    ["commands", [
      [{ constructor: { name: "HtaKeyword" }, name: "grid" }, 24, "#123", 1],
      [{ constructor: { name: "HtaKeyword" }, name: "circle" }, 20, 30, 4, "#41f5e4"],
      [{ constructor: { name: "HtaKeyword" }, name: "mist" }, 80, 70, 24, "#31ff8d", 0.16]
    ]]
  ]));
  assert.ok(calls.some(([name]) => name === "fillRect"));
  assert.ok(calls.some(([name]) => name === "arc"));
  assert.ok(calls.some(([name]) => name === "createRadialGradient"));
  assert.equal(calls.filter(([name]) => name === "addColorStop").length, 2);
});

test("HTA object frames unwrap WebGL fallbacks before dispatch", () => {
  const { runtime, calls } = fixture();
  runtime.claim("node/ocean@1", "canvas/background");
  const keyword = (name) => new HtaKeyword(name);
  const fallback = new HtaObject([
    [keyword("type"), keyword("canvas-2d")],
    [keyword("background"), "#020817"],
    [keyword("commands"), [[keyword("text"), "OCEAN", 24, 156, "#6ee7ff", 11]]]
  ]);
  const frame = new HtaObject([
    [keyword("type"), keyword("webgl2")],
    [keyword("fallback"), fallback]
  ]);

  assert.equal(runtime.render("node/ocean@1", "canvas/background", frame), true);
  assert.ok(calls.some(([name]) => name === "fillText"));
});

test("stateful Tron frames retain trails in the canvas host", () => {
  const { runtime, calls } = fixture();
  runtime.claim("node/tron@1", "canvas/background");
  const frame = (stateful) => runtime.render("node/tron@1", "canvas/background", new Map([
    ["type", { constructor: { name: "HtaKeyword" }, name: "canvas-2d" }],
    ["background", "#020408"], ["stateful", stateful]
  ]));
  frame({ kind: { constructor: { name: "HtaKeyword" }, name: "tron" }, init: true, trails: [[[10, 20]], [[30, 40]], [[50, 60]], [[70, 80]]],
    heads: [10, 20, 30, 40, 50, 60, 70, 80] });
  frame({ kind: "tron", heads: [46, 20, 30, 40, 50, 60, 70, 80], append: [[0, 46, 20]] });
  assert.deepEqual(runtime.canvases.get("canvas/background").stateful.trails[0], [[10, 20], [46, 20]]);
  assert.ok(calls.filter(([name]) => name === "lineTo").length >= 2);
});

test("stateful Boid frames retain tails without replaying a published event", () => {
  const { runtime, callbacks } = fixture();
  runtime.claim("node/boids@1", "canvas/background");
  const frame = new Map([["type", { constructor: { name: "HtaKeyword" }, name: "canvas-2d" }], ["stateful", {
    kind: { constructor: { name: "HtaKeyword" }, name: "boids" }, init: true, boids: [[10, 20, 5, 0], [30, 40, 0, 5]]
  }]]);
  runtime.publish("node/boids@1", "canvas/background", frame);
  callbacks.values().next().value(16);
  assert.deepEqual(runtime.canvases.get("canvas/background").stateful.tails[0], [[10, 20]]);
});

test("published frames continue rendering from the latest event", () => {
  const { runtime, callbacks, calls } = fixture();
  runtime.claim("node/tron@1", "canvas/background");
  const frame = new Map([
    ["type", { constructor: { name: "HtaKeyword" }, name: "canvas-2d" }],
    ["background", "#020408"],
    ["stateful", { kind: "tron", init: true, trails: [[], [], [], []], heads: [10, 20, 30, 40, 50, 60, 70, 80] }]
  ]);
  runtime.publish("node/tron@1", "canvas/background", frame);
  assert.equal(runtime.canvases.get("canvas/background").live.nodeId, "node/tron@1");
  callbacks.values().next().value(16);
  assert.ok(calls.filter(([name]) => name === "fillRect").length >= 2);
});

test("hiding a workspace rejects outstanding animation frames", async () => {
  const { runtime } = fixture();
  runtime.claim("node/fire@1", "canvas/background");
  const pending = runtime.nextFrame("node/fire@1", "canvas/background");
  runtime.setVisible(false);
  await assert.rejects(pending, /workspace hidden/);
});

test("closing a canvas runtime releases surface ownership and queued input", async () => {
  const { runtime, listeners } = fixture();
  runtime.claim("node/fire@1", "canvas/background");
  listeners.get("keydown")({
    key: "x", code: "KeyX", repeat: false,
    ctrlKey: false, altKey: false, shiftKey: false, metaKey: false
  });
  const pending = runtime.nextFrame("node/fire@1", "canvas/background");
  runtime.close();
  await assert.rejects(pending, /canvas runtime closed/);
  assert.equal(runtime.canvases.size, 0);
  assert.equal(runtime.surfaces.size, 0);
  assert.equal(runtime.events.length, 0);
});
