import assert from "node:assert/strict";
import test from "node:test";
import { readFile } from "node:fs/promises";

import { HtaContext, HtaKeyword } from "./packages/hta/index.js";
import { compileAnonymousDocument, createBrowserBroker, KernelBroker } from "./studio/broker.js";

// Mock HtaContext-shaped kernel: records calls, echoes evals with the kernel name.
function mockSpawn({ failFor } = {}) {
  const spawned = [];
  const spawn = async (name) => {
    if (failFor === name) throw new Error(`spawn failed for ${name}`);
    const kernel = {
      name,
      context: {
        calls: [],
        closed: false,
        sessions: new Map(),
        async call(target, args) {
          this.calls.push([target, args]);
          if (target === "eval") return `${name}:${args[0]}`;
          return true;
        },
        session(sessionName) {
          let session = this.sessions.get(sessionName);
          if (!session) {
            const context = this;
            session = {
              name: sessionName,
              call(target, args = []) {
                return context.call(
                  target === "eval" ? "session/eval" : target,
                  target === "eval" ? [sessionName, ...args] : args
                );
              },
              close() {
                context.sessions.delete(sessionName);
                return context.call("session/close", [sessionName]);
              }
            };
            this.sessions.set(sessionName, session);
          }
          return session;
        },
        close() {
          this.closed = true;
        }
      },
      worker: {
        terminated: false,
        terminate() {
          this.terminated = true;
        }
      }
    };
    spawned.push(kernel);
    return kernel;
  };
  return { spawn, spawned };
}

test("create, list, size, and eval route to the named kernel", async () => {
  const { spawn } = mockSpawn();
  const broker = new KernelBroker({ spawn });
  const alpha = await broker.create("alpha");
  const beta = await broker.create("beta");

  assert.equal(await broker.eval("alpha", "(+ 1 2)"), "alpha:(+ 1 2)");
  assert.equal(await broker.eval("beta", "(* 3 4)"), "beta:(* 3 4)");
  assert.deepEqual(alpha.context.calls, [["eval", ["(+ 1 2)"]]]);
  assert.deepEqual(beta.context.calls, [["eval", ["(* 3 4)"]]]);

  const names = broker.list();
  assert.ok(names.includes("ROOT"));
  assert.ok(names.includes("alpha"));
  assert.ok(names.includes("beta"));
  assert.equal(broker.size(), 3);
});

test("one browser kernel hosts isolated named sessions", async () => {
  const { spawn, spawned } = mockSpawn();
  const broker = new KernelBroker({ spawn });
  await broker.create("page");

  await broker.createSession("page", "example-1");
  await broker.createSession("page", "example-2");
  await broker.evalSession("page", "example-1", "(def answer 41)");
  await broker.evalSession("page", "example-2", "(def answer 6)");
  await broker.closeSession("page", "example-1");

  assert.equal(spawned.filter(({ name }) => name === "page").length, 1);
  assert.deepEqual(spawned[0].context.calls, [
    ["session/create", ["example-1"]],
    ["session/create", ["example-2"]],
    ["session/eval", ["example-1", "(def answer 41)"]],
    ["session/eval", ["example-2", "(def answer 6)"]],
    ["session/close", ["example-1"]]
  ]);
  await assert.rejects(
    broker.evalSession("page", "example-1", "answer"),
    /^Error: NO_SESSION example-1$/
  );
});

test("traceForms evaluates top-level forms in a disposable session and stops on error", async () => {
  const { spawn, spawned } = mockSpawn();
  const broker = new KernelBroker({
    spawn: async (name) => {
      const kernel = await spawn(name);
      kernel.context.call = async function (target, args) {
        this.calls.push([target, args]);
        if (target === "session/eval" && args[1] === "(broken)") throw new Error("broken form");
        if (target === "session/eval") return `value:${args[1]}`;
        return true;
      };
      return kernel;
    }
  });
  const forms = [
    { start: 0, end: 4, source: "(+ 1 2)" },
    { start: 5, end: 13, source: "(broken)" },
    { start: 14, end: 20, source: "(+ 3 4)" }
  ];
  const ticks = [10, 12, 20, 25];
  const rows = await broker.traceForms("ROOT", forms, { now: () => ticks.shift(), bootstrap: "(boot)" });

  assert.deepEqual(rows.map(({ source, status, value, error, duration }) => ({ source, status, value, error, duration })), [
    { source: "(+ 1 2)", status: "ok", value: "value:(+ 1 2)", error: undefined, duration: 2 },
    { source: "(broken)", status: "error", value: undefined, error: "broken form", duration: 5 }
  ]);
  const calls = spawned[0].context.calls;
  assert.deepEqual(
    calls.filter(([target]) => target === "session/eval").map(([, args]) => args[1]),
    ["(boot)", "(+ 1 2)", "(broken)"]
  );
  assert.equal(calls.at(-1)[0], "session/close");
});

test("previewDocument retains structured traces until disposal", async () => {
  const { spawn, spawned } = mockSpawn();
  const kw = (name) => new HtaKeyword(name);
  const trace = (id, source) => new Map([
    [kw("schema"), "hara.trace/0-alpha"],
    [kw("trace-id"), id],
    [kw("status"), kw(source === "(broken)" ? "error" : "ok")],
    [kw("result"), new Map([[kw("type"), "number"], [kw("display"), "42"]])],
    [kw("error"), source === "(broken)" ? "broken form" : null],
    [kw("events"), []]
  ]);
  const broker = new KernelBroker({
    spawn: async (name) => {
      const kernel = await spawn(name);
      let nextTrace = 1;
      const original = kernel.context.call.bind(kernel.context);
      kernel.context.call = async (target, args) => {
        if (target === "session/trace-eval") return trace(`trace-${nextTrace++}`, args[1]);
        return original(target, args);
      };
      return kernel;
    }
  });
  const forms = [
    { start: 0, end: 9, source: "(+ 19 23)" },
    { start: 10, end: 18, source: "(broken)" },
    { start: 19, end: 28, source: "(+ 20 22)" }
  ];
  const preview = await broker.previewDocument("ROOT", "project:home:/demo.hal", forms, {
    bootstrap: "(boot)"
  });
  assert.deepEqual(preview.rows.map(({ status, value, traceId }) => ({ status, value, traceId })), [
    { status: "ok", value: "42", traceId: "trace-1" },
    { status: "error", value: "42", traceId: "trace-2" },
    { status: "skipped", value: undefined, traceId: null }
  ]);
  assert.equal(broker.getPreviewTrace(preview.generationId, "trace-1") instanceof Map, true);
  assert.equal(await broker.disposePreview(preview.generationId), true);
  assert.throws(() => broker.getPreviewTrace(preview.generationId, "trace-1"), /NO_PREVIEW/);
  assert.equal(spawned[0].context.calls.at(-1)[0], "session/close");
});

test("resources are registered before the bootstrap eval", async () => {
  const { spawn } = mockSpawn();
  const broker = new KernelBroker({
    spawn,
    resources: { "lib/a.hal": "(ns lib.a)", "lib/b.hal": "(ns lib.b)" }
  });
  const kernel = await broker.create("booted", { bootstrap: "(require 'lib.a)" });
  assert.deepEqual(kernel.context.calls, [
    ["register-resources", [[
      ["lib/a.hal", "(ns lib.a)"],
      ["lib/b.hal", "(ns lib.b)"]
    ]]],
    ["eval", ["(require 'lib.a)"]]
  ]);
});

test("kernel starting hook establishes host scope before bootstrap", async () => {
  const { spawn } = mockSpawn();
  const order = [];
  const broker = new KernelBroker({
    spawn: async (name) => {
      const kernel = await spawn(name);
      const call = kernel.context.call.bind(kernel.context);
      kernel.context.call = async (...args) => {
        order.push(args[0]);
        return call(...args);
      };
      return kernel;
    },
    onKernelStarting: ({ name }) => order.push(`scope:${name}`),
    resources: { "lib/a.hal": "(ns lib.a)" }
  });
  await broker.create("scoped", { bootstrap: "(+ 1 2)" });
  assert.deepEqual(order, ["scope:scoped", "register-resources", "eval"]);
});

test("create without bootstrap only registers resources", async () => {
  const { spawn } = mockSpawn();
  const broker = new KernelBroker({ spawn, resources: { "lib/a.hal": "(ns lib.a)" } });
  const kernel = await broker.create("plain");
  assert.deepEqual(kernel.context.calls, [[
    "register-resources", [[["lib/a.hal", "(ns lib.a)"]]]
  ]]);
});

test("duplicate names are rejected, including concurrent creates", async () => {
  const { spawn, spawned } = mockSpawn();
  const broker = new KernelBroker({ spawn });
  await broker.create("alpha");
  await assert.rejects(broker.create("alpha"), /^Error: SESSION_EXISTS alpha$/);

  const [first, second] = await Promise.allSettled([broker.create("race"), broker.create("race")]);
  assert.equal(first.status === "fulfilled" ? second.status : first.status, "rejected");
  const rejection = first.status === "rejected" ? first : second;
  assert.match(rejection.reason.message, /^SESSION_EXISTS race$/);
  assert.equal(spawned.filter((kernel) => kernel.name === "race").length, 1);
});

test("invalid names are rejected like the JVM broker", async () => {
  const { spawn } = mockSpawn();
  const broker = new KernelBroker({ spawn });
  for (const bad of ["", "bad name", "bad/name", "bad:name", null, undefined]) {
    await assert.rejects(broker.create(bad), /^Error: INVALID_SESSION_NAME$/);
    await assert.rejects(broker.close(bad), /^Error: INVALID_SESSION_NAME$/);
  }
  // The JVM broker does not lowercase; valid names pass through unchanged.
  const kernel = await broker.create("Mixed_Case-1.2");
  assert.equal(kernel.name, "Mixed_Case-1.2");
});

test("require returns the kernel record and rejects unknown names", async () => {
  const { spawn } = mockSpawn();
  const broker = new KernelBroker({ spawn });
  const created = await broker.create("alpha");
  assert.equal(await broker.require("alpha"), created);
  await assert.rejects(broker.require("ghost"), /^Error: NO_SESSION ghost$/);
  await assert.rejects(broker.eval("ghost", "(+ 1 2)"), /^Error: NO_SESSION ghost$/);
});

test("close terminates context and worker and removes the kernel", async () => {
  const { spawn } = mockSpawn();
  const broker = new KernelBroker({ spawn });
  const kernel = await broker.create("alpha");
  await broker.close("alpha");

  assert.equal(kernel.context.closed, true);
  assert.equal(kernel.worker.terminated, true);
  assert.ok(!broker.list().includes("alpha"));
  await assert.rejects(broker.require("alpha"), /^Error: NO_SESSION alpha$/);
  await assert.rejects(broker.close("alpha"), /^Error: NO_SESSION alpha$/);
});

test("kernel lifecycle hooks observe created sessions before release", async () => {
  const { spawn } = mockSpawn();
  const events = [];
  const broker = new KernelBroker({
    spawn,
    onKernelCreated: async (kernel) => events.push(["created", kernel.name]),
    onKernelClosed: async (kernel) => events.push(["closed", kernel.name])
  });
  await broker.require("ROOT");
  await broker.create("alpha");
  await broker.close("alpha");
  assert.deepEqual(events, [["created", "ROOT"], ["created", "alpha"], ["closed", "alpha"]]);
});

test("ns+ documents activate isolated generations and roll back failed reloads", async () => {
  const { spawn, spawned } = mockSpawn();
  const broker = new KernelBroker({
    spawn: async (name) => {
      const kernel = await spawn(name);
      kernel.context.call = async function (target, args) {
        this.calls.push([target, args]);
        if (target === "register-resources") return true;
        const source = target === "session/eval" ? args[1] : args[0];
        if (target === "session/eval" && source.includes("BROKEN")) throw new Error("bad reload");
        return source?.includes("(+ value 1)") ? 43 : 42;
      };
      return kernel;
    },
    resources: { "studio.node": "(ns studio.node)" }
  });
  const first = await broker.evalDocument("ROOT", "document/a", "(ns+)\n(def value 42)\nvalue", {
    nodeId: "node/a"
  });
  assert.equal(first.value, 42);
  assert.equal(first.generation, 1);
  const active = broker.requireDocument("ROOT", "document/a");
  assert.equal(await broker.evalForm("ROOT", "document/a", "(+ value 1)"), 43);

  await assert.rejects(
    broker.evalDocument("ROOT", "document/a", "(ns+)\nBROKEN", { nodeId: "node/a" }),
    /bad reload/
  );
  assert.equal(broker.requireDocument("ROOT", "document/a"), active);
  assert.equal(spawned.length, 1);
  assert.equal(active.worker.terminated, false);
  assert.ok(active.context.name.startsWith("DOC.document.a."));

  const third = await broker.evalDocument("ROOT", "document/a", "(ns+)\n43", { nodeId: "node/a" });
  assert.equal(third.generation, 2);
  assert.equal(active.worker.terminated, false);
  assert.equal(spawned.length, 1);
  assert.equal(broker.releaseDocument("ROOT", "document/a"), true);
  assert.throws(() => broker.requireDocument("ROOT", "document/a"), /NO_DOCUMENT/);
});

test("anonymous document compiler requires ns+ first and binds node context", () => {
  assert.throws(() => compileAnonymousDocument("(ns public.name) 42", {
    documentId: "document/a"
  }), /NS_PLUS_MUST_BE_FIRST/);
  const compiled = compileAnonymousDocument(
    "; comment\n#_ (ignored)\n(ns+ (:require [studio.draw :as draw]))\n(node/info)",
    { documentId: "document/a", nodeId: "node/a" }
  );
  assert.ok(compiled.source.includes("ns anonymous.document.a."));
  assert.ok(compiled.source.includes("require [studio.node :as node]"));
  assert.ok(compiled.source.includes('set! node/*node-id* "node/a"'));
  assert.ok(!compiled.moduleId.includes("node/a"));
});

test("document compiler resolves built-in co/ without changing strings or comments", () => {
  const compiled = compileAnonymousDocument(
    '(ns+)\n; co/await stays documentation\n(def label "co/await")\n(co/await (node/in "x"))',
    { documentId: "document/co", nodeId: "node/co" }
  );
  assert.ok(compiled.source.includes("(std.foundation.coroutine/await (node/in"));
  assert.ok(compiled.source.includes('"co/await"'));
  assert.ok(compiled.source.includes("; co/await stays documentation"));
});

test("ROOT exists without explicit creation and cannot be closed or recreated", async () => {
  const { spawn } = mockSpawn();
  const broker = new KernelBroker({ spawn });

  assert.equal(await broker.eval("ROOT", "(+ 1 2)"), "ROOT:(+ 1 2)");
  const root = await broker.require("ROOT");
  assert.equal(root.name, "ROOT");
  assert.ok(broker.list().includes("ROOT"));

  await assert.rejects(broker.close("ROOT"), /^Error: ROOT_CANNOT_CLOSE$/);
  await assert.rejects(broker.create("ROOT"), /^Error: SESSION_EXISTS ROOT$/);
  // A failed close attempt leaves ROOT alive.
  assert.equal(await broker.eval("ROOT", "still-here"), "ROOT:still-here");
});

test("concurrent first access spawns ROOT exactly once", async () => {
  const { spawn, spawned } = mockSpawn();
  const broker = new KernelBroker({ spawn });
  const results = await Promise.all([
    broker.eval("ROOT", "a"),
    broker.require("ROOT"),
    broker.eval("ROOT", "b"),
    broker.eval("ROOT", "c")
  ]);
  assert.equal(results[0], "ROOT:a");
  assert.equal(results[1].name, "ROOT");
  assert.equal(results[2], "ROOT:b");
  assert.equal(results[3], "ROOT:c");
  assert.equal(spawned.filter((kernel) => kernel.name === "ROOT").length, 1);
});

test("a failed ROOT spawn does not poison later access", async () => {
  let attempts = 0;
  const { spawn, spawned } = mockSpawn();
  const broker = new KernelBroker({
    spawn: async (name) => {
      if (name === "ROOT" && attempts++ === 0) throw new Error("transient");
      return spawn(name);
    }
  });
  await assert.rejects(broker.eval("ROOT", "a"), /transient/);
  assert.equal(await broker.eval("ROOT", "b"), "ROOT:b");
  assert.equal(spawned.filter((kernel) => kernel.name === "ROOT").length, 1);
});

test("a failed create terminates the half-booted kernel", async () => {
  const { spawn, spawned } = mockSpawn();
  const broker = new KernelBroker({
    spawn: async (name) => {
      const kernel = await spawn(name);
      kernel.context.call = async function (target, args) {
        this.calls.push([target, args]);
        if (target === "register-resources") throw new Error("bad resource");
        return true;
      };
      return kernel;
    },
    resources: { "lib/broken.hal": "not hara" }
  });
  await assert.rejects(broker.create("alpha"), /bad resource/);
  const kernel = spawned.find((candidate) => candidate.name === "alpha");
  assert.equal(kernel.context.closed, true);
  assert.equal(kernel.worker.terminated, true);
  await assert.rejects(broker.require("alpha"), /^Error: NO_SESSION alpha$/);
});

test("createBrowserBroker wires Worker and HtaContext", async () => {
  const originalWorker = globalThis.Worker;
  const made = [];
  globalThis.Worker = class {
    constructor(url, options) {
      this.url = url;
      this.options = options;
      this.listeners = {};
      made.push(this);
    }
    addEventListener(type, handler) {
      this.listeners[type] = handler;
    }
    postMessage(message) {
      if (message.type === "init") {
        this.initMessage = message;
        queueMicrotask(() => this.listeners.message({ data: { type: "ready" } }));
      }
    }
    terminate() {
      this.terminated = true;
    }
  };
  try {
    const hostCalls = { "store/get": async () => null };
    const moduleBytes = new Uint8Array([1, 2, 3]);
    const broker = createBrowserBroker({ workerUrl: "/kernel.js", moduleBytes, hostCalls });
    const root = await broker.require("ROOT");

    assert.equal(made.length, 1);
    assert.equal(made[0].url, "/kernel.js");
    assert.deepEqual(made[0].options, { type: "module", name: "hara-kernel-ROOT" });
    assert.ok(root.context instanceof HtaContext);
    assert.equal(root.context.hostCalls, hostCalls);
    assert.equal(made[0].initMessage.moduleBytes, moduleBytes);
  } finally {
    globalThis.Worker = originalWorker;
  }
});

// Optional integration: drive the canonical HTA worker package in
// node via a `self` shim, against the real raw wasm. Skipped when the artifact
// has not been built.
const wasmUrl = new URL("../raw/target/wasm32-unknown-unknown/browser-release/hara-wasm-vm.wasm", import.meta.url);
const wasmBytes = await readFile(wasmUrl).catch(() => null);

test("real wasm kernel evals hara source through the broker", { skip: wasmBytes === null }, async () => {
  const bridge = { listeners: {}, selfListeners: {} };
  bridge.self = {
    addEventListener: (type, handler) => {
      bridge.selfListeners[type] = handler;
    },
    postMessage: (data) => bridge.listeners.message?.({ data }),
    close: () => {}
  };
  globalThis.self = bridge.self;
  await import("./packages/hta/worker.mjs");

  const worker = {
    terminated: false,
    addEventListener: (type, handler) => {
      bridge.listeners[type] = handler;
    },
    postMessage: (message) => bridge.selfListeners.message({ data: message }),
    terminate() {
      this.terminated = true;
    }
  };
  const broker = new KernelBroker({
    resources: { "lib/probe.hal": "(ns lib.probe)" },
    spawn: async () => ({ context: new HtaContext({ worker, moduleBytes: wasmBytes }), worker })
  });

  assert.equal(await broker.eval("ROOT", "(+ 1 2)"), 3);
  const trace = await broker.traceForms("ROOT", [{ start: 0, end: 9, source: "(+ 19 23)" }]);
  assert.deepEqual(
    trace.map(({ source, status, value }) => ({ source, status, value })),
    [{ source: "(+ 19 23)", status: "ok", value: 42 }]
  );
  const structured = await broker.traceEval(
    "ROOT", "ROOT",
    "(defn observed [x] (+ x 1)) (observed 41)"
  );
  const field = (map, name) => [...map].find(([key]) => key?.name === name)?.[1];
  assert.equal(field(structured, "schema"), "hara.trace/0-alpha");
  assert.equal(field(field(structured, "result"), "display"), "42");
  assert.ok(field(structured, "events").some(
    (event) => field(event, "kind")?.name === "operation-enter"
  ));
  assert.deepEqual(await broker.listSessions("ROOT"), ["ROOT"]);
  const root = await broker.require("ROOT");
  assert.equal(await root.context.call("register-resource", ["lib/extra.hal", "(ns lib.extra)"]), true);
  assert.ok(broker.list().includes("ROOT"));
});
