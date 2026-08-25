import assert from "node:assert/strict";
import test from "node:test";
import { readFile } from "node:fs/promises";
import "fake-indexeddb/auto";

import { HtaContext, HtaKeyword } from "./packages/hta/index.js";
import { KernelBroker } from "./studio/broker.js";
import { defaultBootstrap } from "./studio/boot.js";
import { createHostServices } from "./studio/host-services.js";
import { NodeRuntime } from "./studio/node-runtime.js";
import { SessionRouter } from "./studio/session-router.js";
import { normalizeCreative } from "../../../../../website/hara-www/creative.js";

// Real-wasm integration tests for the studio hara libraries
// (rust/web/studio/hal/*.hal): store/boot and canonical file behaviour is asserted by
// evaluating hara source in actual HTA kernels. Skipped when the raw wasm
// artifact has not been built.
const wasmUrl = new URL("../raw/target/wasm32-unknown-unknown/browser-release/hara-wasm-vm.wasm", import.meta.url);
const wasmBytes = await readFile(wasmUrl).catch(() => null);
const hal = (name) => readFile(new URL(`./studio/hal/${name}.hal`, import.meta.url), "utf8");
const supersonicHal = () => readFile(
  new URL("../../../../hara-specs-registry/00-unsorted/contrib/greenways/supersonic/src/hal/gw/audio/supersonic.hal", import.meta.url),
  "utf8"
);
const substrateResources = async () => {
  const output = {};
  for (const name of [
    "core", "frame", "json", "protocol", "pubsub", "request", "router",
    "space", "transport_memory", "util", "util_handlers"
  ]) {
    output[`std.substrate.${name.replaceAll("_", "-")}`] = await readFile(
      new URL(`../../lib/src/std/substrate/${name}.hal`, import.meta.url), "utf8"
    );
  }
  output["std.substrate"] = await readFile(new URL("../../lib/src/std/substrate.hal", import.meta.url), "utf8");
  return output;
};
const resources = wasmBytes === null
  ? null
  : {
      "studio.store": await hal("store"),
      "studio.boot": await hal("boot"),
      "studio.node": await hal("node"),
      "studio.program": await hal("program"),
      "studio.graph": await hal("graph"),
      "studio.session": await hal("session"),
      "gw.audio.supersonic": await supersonicHal(),
      ...await substrateResources()
    };

const LISTING_URL = "https://data.jsdelivr.com/v1/packages/gh/octo/lessons@main";
const CDN_PREFIX = "https://cdn.jsdelivr.net/gh/octo/lessons@main";
const LISTING = JSON.stringify({
  type: "package",
  name: "octo/lessons",
  version: "main",
  files: [
    { type: "file", name: "/README.md", size: 10 },
    {
      type: "directory",
      name: "/src",
      files: [
        { type: "file", name: "/src/intro.hal", size: 7 },
        { type: "file", name: "/src/advanced.hal", size: 7 }
      ]
    }
  ]
});
const FILES = {
  "/README.md": "# Lessons",
  "/src/intro.hal": "(+ 1 2)",
  "/src/advanced.hal": "(+ 3 4)"
};

function mockFetch({ failFor } = {}) {
  return async (url) => {
    if (url === LISTING_URL) return { ok: true, status: 200, text: async () => LISTING };
    if (url.startsWith(CDN_PREFIX)) {
      const path = url.slice(CDN_PREFIX.length);
      if (failFor !== path && path in FILES) {
        return { ok: true, status: 200, text: async () => FILES[path] };
      }
    }
    return { ok: false, status: 404, text: async () => "not found" };
  };
}

// Each kernel needs its own hta-worker instance behind its own `self` bridge;
// the cache-busting query forces node to evaluate the module afresh so it
// binds to the bridge installed just before the import.
let kernelCounter = 0;
let brokerCounter = 0;
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
  await import(`./packages/hta/worker.mjs?kernel=${kernelCounter}`);
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
  return { context: new HtaContext({ worker, moduleBytes: wasmBytes, hostCalls }), worker };
}

function makeBroker({ fetch, nodeRuntime, supersonic } = {}) {
  const hostCalls = createHostServices({
    dbName: `hara-studio-hal-test-${++brokerCounter}`,
    fetch: fetch ?? mockFetch(),
    nodeRuntime,
    supersonic
  });
  return new KernelBroker({
    resources,
    spawn: () => spawnRealKernel(hostCalls),
    onKernelStarting: async (kernel) => {
      const mount = await kernel.context.createFilesystem({ provider: "memory" });
      await kernel.context.session().attachFilesystem(mount);
    }
  });
}

const REQUIRE_ALL =
  "(require [studio.store :as store]) " +
  "(require [studio.boot :as boot])";
const evaluate = (broker, source) => broker.eval("ROOT", `(do ${REQUIRE_ALL} ${source})`);

// hta maps decode to JS Maps with HtaKeyword (or string) keys.
function mapGet(map, name) {
  const entries = map instanceof Map ? map : map.entries;
  for (const [key, value] of entries) {
    if (key instanceof HtaKeyword && key.name === name) return value;
    if (key === name) return value;
  }
  return undefined;
}

function sequenceValues(value) {
  return Array.isArray(value) ? value : value.values;
}

test("defaultBootstrap renders the shared bootstrap template", { skip: wasmBytes === null }, () => {
  assert.equal(
    defaultBootstrap("boot-space"),
    '(do (require [studio.boot :as boot]) (boot/boot! "boot-space"))'
  );
});

test("rigged-cube creative data survives wasm evaluation and normalization", { skip: wasmBytes === null }, async () => {
  const broker = makeBroker();
  const value = await broker.eval("ROOT", `{:creative/version 1
    :background "#020408"
    :entities [{:id "mesh/hero"
                :mesh {:primitive :box}
                :material {:color "#41f5e4"}
                :transform {:rotation [0 0 0]}
                :rig {:bones [{:id "bone/root" :length 1}
                              {:id "bone/arm" :parent "bone/root" :length 1}]}}]
    :audio {:tempo 120 :midi true :voices []}}`);
  const scene = normalizeCreative(value);
  assert.equal(scene.entities.length, 1);
  assert.deepEqual(scene.entities[0].rotation, [0, 0, 0]);
  assert.equal(scene.entities[0].rig.bones.length, 2);
});

test("Host exposes the generic browser host descriptor", { skip: wasmBytes === null }, async () => {
  const broker = makeBroker();
  const description = await evaluate(broker, "(deref (Host/describe))");
  assert.equal(mapGet(description, "host/version"), "hara.host.v1");
  assert.equal(await evaluate(broker, '(deref (Host/capability? "filesystem"))'), true);
  assert.equal(await evaluate(broker, '(deref (Host/capability? "missing"))'), false);
});

test("gw.audio.supersonic exposes the portable graph lifecycle", { skip: wasmBytes === null }, async () => {
  const calls = [];
  const supersonic = {
    start: async (graph) => {
      calls.push(["start", graph]);
      return { "graph/id": "test", generation: 1, status: "running" };
    },
    update: async (...args) => {
      calls.push(["update", ...args]);
      return { "graph/id": "test", generation: 1, status: "running" };
    },
    status: async (id) => ({ "graph/id": id, generation: 1, status: "running" }),
    stop: async (id) => ({ "graph/id": id, generation: 1, status: "stopped" })
  };
  const broker = makeBroker({ supersonic });
  const result = await broker.eval("ROOT",
    '(do (require [gw.audio.supersonic :as sonic]) ' +
    '(sonic/start {"graph/id" "test" "nodes" [] "connections" []}))');
  assert.equal(mapGet(result, "status"), "running");
  await broker.eval("ROOT",
    '(sonic/update "test" "gain" "volume" 0.5)');
  assert.deepEqual(calls.map(([operation]) => operation), ["start", "update"]);
  assert.equal(mapGet(await broker.eval("ROOT", '(sonic/stop "test")'), "status"), "stopped");
});

test("studio.node sends std.substrate.frame envelopes through the browser adapter", { skip: wasmBytes === null }, async () => {
  const runtime = new NodeRuntime({ space: "workspace/studio-hal" });
  runtime.registerNode({ id: "node/a" });
  runtime.registerNode({ id: "node/b" });
  runtime.connect({
    id: "a-to-b",
    from: ["node/a", "signal/out"],
    to: ["node/b", "signal/in"]
  });
  runtime.handle("node/b", "double", ([value]) => value * 2);
  const broker = makeBroker({ nodeRuntime: runtime });

  const document = await broker.evalDocument(
    "ROOT",
    "document/substrate-node",
    '(ns+) (node/emit "signal/out" {:answer 42} {:cause "evt-0"})',
    { nodeId: "node/a" }
  );
  const frame = await runtime.inFrame("node/b", "signal/in");
  assert.equal(frame.version, "substrate.v1");
  assert.equal(frame.kind, "stream");
  assert.equal(frame.source, "node/a");
  assert.equal(frame.cause, "evt-0");
  assert.deepEqual(frame.data, { answer: 42 });

  const value = await broker.evalForm(
    "ROOT",
    "document/substrate-node",
    '(node/call "node/b" "double" [21] {:id "req-1" :meta {:trace "studio"}})'
  );
  assert.equal(value, 42);
});

test("studio.program and studio.graph bridge their host-call operations", { skip: wasmBytes === null }, async () => {
  const calls = [];
  const graphHost = {
    programs: { release: async (id) => { calls.push(["program/release", id]); return true; } },
    install: async (descriptor, options) => { calls.push(["program/install", descriptor, options]); return { programId: descriptor["program/id"] }; },
    programInfo: (id) => ({ programId: id }),
    spawn: async (descriptor) => ({ nodeId: descriptor["node/id"] }),
    release: async () => true,
    connect: () => "connection-1",
    disconnect: () => true,
    sendFrame: async (_source, frame) => ({ accepted: true, frame }),
    callFrame: async () => ({ data: 42 }),
    info: (id) => ({ nodeId: id }),
    list: () => []
  };
  const hostCalls = createHostServices({
    dbName: `hara-studio-graph-test-${++brokerCounter}`,
    fetch: mockFetch(),
    graphHost
  });
  const broker = new KernelBroker({ resources, spawn: () => spawnRealKernel(hostCalls) });
  const value = await broker.eval("ROOT", "(do " +
    "(require [studio.program :as program]) " +
    "(require [studio.graph :as graph]) " +
    '(program/install {"program/id" "example/transform"} {"sessionId" "ROOT"}) ' +
    '(graph/send-frame "node/source" {"kind" "stream" "id" "evt-1" "signal" "out" "data" 42}))');
  assert.equal(mapGet(value, "accepted"), true);
  assert.deepEqual(calls[0], ["program/install", { "program/id": "example/transform" }, { sessionId: "ROOT" }]);
});

test("studio.session registers a callback and receives only its subscribed frame", { skip: wasmBytes === null }, async () => {
  const sessions = new SessionRouter();
  const released = [];
  const graphHost = { releaseSession: async (id) => released.push(id) };
  const hostCalls = createHostServices({
    dbName: `hara-studio-session-test-${++brokerCounter}`,
    fetch: mockFetch(),
    graphHost,
    graphHostOptions: { sessionRouter: sessions }
  });
  const broker = new KernelBroker({ resources, spawn: () => spawnRealKernel(hostCalls) });
  const callbackId = await broker.eval("ROOT", "(do " +
    "(require [studio.session :as session]) " +
    '(session/register-ingress! "ROOT") ' +
    '(session/on "ROOT" "selected" (fn [event] (get event "data"))))');
  assert.equal(typeof callbackId, "string");
  const delivered = await sessions.deliver("ROOT", {
    version: "substrate.v1", kind: "stream", id: "evt-selected", signal: "selected", data: 7,
    meta: { "session/callback": callbackId }
  });
  assert.deepEqual(delivered, { accepted: true, delivered: 1 });
  assert.equal(await broker.eval("ROOT", '(session/unregister-ingress! "ROOT")'), true);
  assert.deepEqual(released, ["ROOT"]);
});

test("studio.node registers kernel-owned request handlers", { skip: wasmBytes === null }, async () => {
  const runtime = new NodeRuntime({ space: "workspace/studio-hal" });
  runtime.registerNode({ id: "node/a" });
  runtime.registerNode({ id: "node/b" });
  const broker = makeBroker({ nodeRuntime: runtime });

  const prepared = await broker.prepareDocument(
    "ROOT",
    "document/substrate-handler",
    '(ns+) (node/handle "double" (fn [args] (* 2 (nth args 0))))',
    { nodeId: "node/b" }
  );
  await runtime.activateDocument("node/b", {
    documentId: "document/substrate-handler",
    generation: prepared.generation,
    moduleId: prepared.moduleId,
    kernelContext: prepared.context
  });
  broker.commitDocument(prepared);
  assert.equal(prepared.value, "handler-1");
  assert.equal(await broker.evalForm("ROOT", "document/substrate-handler", '(studio.node/invoke-handler "handler-1" [21] nil)'), 42);
  const response = await runtime.call("node/a", "node/b", "double", [21], { id: "handler-req" });
  assert.equal(response.data, 42);
  assert.equal(response.reply_to, "handler-req");

  const failed = await broker.prepareDocument(
    "ROOT",
    "document/substrate-handler",
    '(ns+) (node/handle "double" (fn [args] (* 3 (nth args 0))))',
    { nodeId: "node/b" }
  );
  await assert.rejects(runtime.activateDocument("node/b", {
    documentId: "document/substrate-handler",
    generation: failed.generation,
    moduleId: failed.moduleId,
    kernelContext: failed.context,
    prepare() { throw new Error("candidate failed"); }
  }), /candidate failed/);
  broker.discardDocument(failed);
  assert.equal((await runtime.call("node/a", "node/b", "double", [21])).data, 42);
});

test("Studio kernels load the atom-backed std.substrate node", { skip: wasmBytes === null }, async () => {
  const broker = makeBroker();
  const value = await broker.eval(
    "ROOT",
    "(do " +
      "(require [std.substrate :as substrate]) " +
      "(require [std.substrate.protocol :as protocol]) " +
      '(def node (substrate/node-create "node/studio")) ' +
      '(protocol/set-service node "answer" 42) ' +
      '(protocol/get-service node "answer"))'
  );
  assert.equal(value, 42);
});

test("Studio kernels run the atom-backed substrate request stream and cancellation lifecycle", { skip: wasmBytes === null }, async () => {
  const broker = makeBroker();
  const fixture = await readFile(
    new URL("../../lib/test-fixtures/std/substrate/node_lifecycle_conformance.hal", import.meta.url),
    "utf8"
  );
  assert.deepEqual(await broker.eval("ROOT", fixture), [84, 42, new HtaKeyword("rejected")]);
});

test("Studio runs the shared substrate protocol fixture", { skip: wasmBytes === null }, async () => {
  const broker = makeBroker();
  const protocolFixture = await readFile(
    new URL("../../lib/test-fixtures/std/substrate/protocol_conformance.hal", import.meta.url),
    "utf8"
  );
  assert.deepEqual((await broker.eval("ROOT", protocolFixture)).values, [40, 42]);
});

test("Studio runs the specs-owned protocol behavioral corpus", { skip: wasmBytes === null }, async () => {
  const broker = makeBroker();
  const protocolCorpus = await readFile(
    new URL(
      "../../../../hara-specs-registry/01-lang/001-language/draft/conformance/fixtures/protocol_behavioral.hal",
      import.meta.url
    ),
    "utf8"
  );
  assert.doesNotMatch(protocolCorpus, /std\.protocol\.[^\s/]+\/I[A-Z]/);
  const results = await broker.eval("ROOT", protocolCorpus);
  assert.equal(results.length, 105);
  assert.equal(results.filter((result) => mapGet(result, "pass") === true).length, 105);
  const capabilityResults = await broker.eval("ROOT", "(capability-protocol-results)");
  assert.equal(capabilityResults.length, 20);
  assert.equal(capabilityResults.filter((result) => mapGet(result, "pass") === true).length, 20);
  const receiverMatrix = await broker.eval("ROOT", "(protocol-receiver-matrix-results)");
  assert.equal(receiverMatrix.length, 10);
  assert.equal(receiverMatrix.filter((result) => mapGet(result, "pass") === true).length, 10);
  const crossCutting = await broker.eval("ROOT", "(protocol-cross-cutting-results)");
  const crossCuttingValues = sequenceValues(crossCutting);
  assert.equal(crossCuttingValues.length, 6);
  assert.equal(crossCuttingValues.filter((result) => mapGet(result, "pass") === true).length, 6);
  const capabilityReceivers = await broker.eval("ROOT", "(protocol-capability-receiver-results)");
  const capabilityReceiverValues = sequenceValues(capabilityReceivers);
  assert.equal(capabilityReceiverValues.length, 8);
  assert.equal(
    capabilityReceiverValues.filter((result) => mapGet(result, "pass") === true).length,
    8
  );
  const nativeValues = sequenceValues(
    await broker.eval("ROOT", "(protocol-native-value-results)")
  );
  assert.equal(nativeValues.length, 15);
  assert.equal(nativeValues.filter((result) => mapGet(result, "pass") === true).length, 15);
  const predicateValues = sequenceValues(
    await broker.eval("ROOT", "(protocol-predicate-results)")
  );
  assert.equal(predicateValues.length, 7);
  assert.equal(predicateValues.filter((result) => mapGet(result, "pass") === true).length, 7);
});

test("Studio runs the specs-owned protocol surface corpus", { skip: wasmBytes === null }, async () => {
  const broker = makeBroker();
  const protocolCorpus = await readFile(
    new URL(
      "../../../../hara-specs-registry/01-lang/001-language/draft/conformance/fixtures/protocol_surface.hal",
      import.meta.url
    ),
    "utf8"
  );
  assert.doesNotMatch(protocolCorpus, /std\.protocol\.[^\s/]+\/I[A-Z]/);
  const results = await broker.eval("ROOT", protocolCorpus);
  assert.equal(results.length, 55);
  assert.equal(results.filter((result) => mapGet(result, "pass") === true).length, 55);
});

test("Studio runs the shared substrate frame fixture", { skip: wasmBytes === null }, async () => {
  const broker = makeBroker();
  const frameFixture = await readFile(
    new URL("../../lib/test-fixtures/std/substrate/frame_conformance.hal", import.meta.url),
    "utf8"
  );
  const frame = await broker.eval("ROOT", frameFixture);
  assert.equal(frame.length, 15);
  assert.deepEqual(frame.slice(0, 6), [
    "substrate.v1", "request", "req-1", "client/a", "server/b", "workspace/main"
  ]);
  assert.deepEqual(frame[6].entries, [["trace", "trace-1"]]);
  assert.equal(frame[7], "math/add");
  assert.deepEqual(frame[8].values, [19, 23]);
  assert.deepEqual(frame.slice(9), [null, null, null, null, null, null]);
});

test("studio.store round trips string values and lists keys", { skip: wasmBytes === null }, async () => {
  const broker = makeBroker();
  assert.equal(await evaluate(broker, '(store/put! "test-store/alpha" "one")'), true);
  assert.equal(await evaluate(broker, '(store/put! "test-store/beta" "two")'), true);
  assert.equal(await evaluate(broker, '(store/get "test-store/alpha")'), "one");
  assert.equal(await evaluate(broker, '(store/get "test-store/missing")'), null);
  assert.deepEqual(await evaluate(broker, '(store/keys "test-store/")'), ["test-store/alpha", "test-store/beta"]);
  assert.equal(await evaluate(broker, '(store/del! "test-store/alpha")'), true);
  assert.equal(await evaluate(broker, '(store/get "test-store/alpha")'), null);
  assert.deepEqual(await evaluate(broker, '(store/keys "test-store/")'), ["test-store/beta"]);
  assert.ok((await evaluate(broker, "(count (store/keys))")) > 0);
});

test("File performs canonical byte filesystem operations", { skip: wasmBytes === null }, async () => {
  const broker = makeBroker();
  assert.equal(await evaluate(broker, '(deref (File/mkdir "/docs"))'), null);
  assert.equal(
    await evaluate(broker, '(deref (File/write "/docs/note.bin" (bytes 1 2 255)))'),
    null
  );
  assert.equal(await evaluate(broker, '(deref (File/exists? "/docs/note.bin"))'), true);
  assert.deepEqual(await evaluate(broker, '(deref (File/read "/docs/note.bin"))'), new Uint8Array([1, 2, 255]));
  assert.deepEqual(await evaluate(broker, '(deref (File/list "/docs"))'), ["/docs/note.bin"]);
  assert.equal(await evaluate(broker, '(deref (File/delete "/docs/note.bin"))'), null);
  assert.equal(await evaluate(broker, '(deref (File/exists? "/docs/note.bin"))'), false);
});

test("browser sessions share or isolate mounts without losing language state", { skip: wasmBytes === null }, async () => {
  const hostCalls = createHostServices({ dbName: `hara-mount-parity-${++brokerCounter}` });
  const { context } = await spawnRealKernel(hostCalls);
  await context.ready;
  const alpha = await context.createSession("alpha");
  const beta = await context.createSession("beta");
  const isolated = await context.createSession("isolated");
  assert.equal(await alpha.eval("(def retained 42)"), 42);
  const sharedMount = await context.createFilesystem({ provider: "memory" });
  const isolatedMount = await context.createFilesystem({ provider: "memory" });
  await alpha.attachFilesystem(sharedMount);
  await beta.attachFilesystem(sharedMount);
  await isolated.attachFilesystem(isolatedMount);
  assert.equal(await alpha.eval("retained"), 42);
  await alpha.eval(
    '(do (deref (File/write "/shared.bin" (bytes 9))))'
  );
  assert.equal(
    await beta.eval(
      '(do (deref (File/exists? "/shared.bin")))'
    ),
    true
  );
  assert.equal(
    await isolated.eval(
      '(do (deref (File/exists? "/shared.bin")))'
    ),
    false
  );
  await assert.rejects(context.closeFilesystem(sharedMount), /FILESYSTEM_ATTACHED/);
  await alpha.detachFilesystem();
  await beta.detachFilesystem();
  await context.closeFilesystem(sharedMount);
  await isolated.detachFilesystem();
  await context.closeFilesystem(isolatedMount);
  context.close();
});

test("default bootstrap reports project identity in a mounted kernel", { skip: wasmBytes === null }, async () => {
  const broker = makeBroker();
  await broker.create("booted", { bootstrap: defaultBootstrap("boot-space") });
  const summary = await broker.eval(
    "booted",
    '(do (require [studio.boot :as boot]) (boot/boot! "boot-space"))'
  );
  assert.equal(mapGet(summary, "project"), "boot-space");
});

test("a custom bootstrap can build its own store layout without studio.fs", { skip: wasmBytes === null }, async () => {
  const broker = makeBroker();
  const bootstrap = [
    "(do",
    "  (require [studio.store :as store])",
    '  (store/put! "custom/layout/note" "custom-value")',
    '  (store/put! "custom/layout/index" "note"))'
  ].join("\n");
  await broker.create("custom", { bootstrap });

  assert.equal(
    await broker.eval("custom", '(do (require [studio.store :as store]) (store/get "custom/layout/note"))'),
    "custom-value"
  );
  assert.deepEqual(
    await broker.eval("custom", '(do (require [studio.store :as store]) (store/keys "custom/"))'),
    ["custom/layout/index", "custom/layout/note"]
  );
});
