import assert from "node:assert/strict";
import test from "node:test";
import "fake-indexeddb/auto";

import { createGraphHostServices, createHostServices } from "./studio/host-services.js";
import { SessionRouter } from "./studio/session-router.js";

test("store put/get round trips string values", async () => {
  const host = createHostServices({ dbName: "test-round-trip" });
  assert.equal(await host["store/put"]("alpha", "one"), true);
  assert.equal(await host["store/get"]("alpha"), "one");
  assert.equal(await host["store/put"]("alpha", "two"), true);
  assert.equal(await host["store/get"]("alpha"), "two");
});

test("store get of a missing key returns null", async () => {
  const host = createHostServices({ dbName: "test-missing" });
  assert.equal(await host["store/get"]("absent"), null);
});

test("generic host introspection reports versioned capability families", async () => {
  const host = createHostServices({ capabilities: ["custom/example"] });
  const description = await host["host/describe"]();
  assert.equal(description.get("host/version"), "hara.host.v1");
  assert.deepEqual(description.get("host/capabilities"), [
    "custom/example", "filesystem", "network/http", "store"
  ]);
  assert.equal(await host["host/capability?"]("filesystem"), true);
  assert.equal(await host["host/capability?"]("missing"), false);
});

test("store del removes a key and returns true", async () => {
  const host = createHostServices({ dbName: "test-del" });
  await host["store/put"]("gone", "value");
  assert.equal(await host["store/del"]("gone"), true);
  assert.equal(await host["store/get"]("gone"), null);
});

test("store keys lists all keys without a prefix and filters with one", async () => {
  const host = createHostServices({ dbName: "test-keys" });
  await host["store/put"]("notes/a", "a");
  await host["store/put"]("notes/b", "b");
  await host["store/put"]("scratch", "c");

  const all = await host["store/keys"]();
  assert.deepEqual([...all].sort(), ["notes/a", "notes/b", "scratch"]);
  assert.deepEqual(await host["store/keys"](null), [...all].sort());
  assert.deepEqual(await host["store/keys"]("notes/"), ["notes/a", "notes/b"]);
  assert.deepEqual(await host["store/keys"]("nothing/"), []);
});

test("two instances sharing a db name see the same IndexedDB data", async () => {
  const first = createHostServices({ dbName: "test-shared" });
  const second = createHostServices({ dbName: "test-shared" });
  await first["store/put"]("shared-key", "shared-value");
  assert.equal(await second["store/get"]("shared-key"), "shared-value");
  assert.deepEqual(await second["store/keys"](), ["shared-key"]);
});

test("kernel-issued memory mounts isolate files and enforce directory semantics", async () => {
  const host = createHostServices({ dbName: "test-session-memory" });
  const context = {};
  await host.filesystemHost.register(context, 1, { provider: "memory" });
  await host.filesystemHost.register(context, 2, { provider: "memory" });
  const alpha = { kernelContext: context, mountId: 1 };
  const beta = { kernelContext: context, mountId: 2 };
  await host["file/mkdir"].call(alpha, "/src");
  await host["file/mkdir"].call(beta, "/src");
  await host["file/write"].call(alpha, "/src/main.hal", new Uint8Array([1]));
  await host["file/write"].call(beta, "/src/main.hal", new Uint8Array([2]));
  assert.deepEqual(await host["file/read"].call(alpha, "/src/main.hal"), new Uint8Array([1]));
  assert.deepEqual(await host["file/read"].call(beta, "/src/main.hal"), new Uint8Array([2]));
  assert.deepEqual(await host["file/list"].call(alpha, "/src"), ["/src/main.hal"]);
  await assert.rejects(
    host["file/write"].call(alpha, "/missing/main.hal", new Uint8Array()),
    /parent-missing/
  );
});

test("equal mount ids in different kernels do not collide", async () => {
  const host = createHostServices({ dbName: "test-kernel-local-mount-ids" });
  const firstContext = {};
  const secondContext = {};
  await host.filesystemHost.register(firstContext, 1, { provider: "memory" });
  await host.filesystemHost.register(secondContext, 1, { provider: "memory" });
  await host["file/write"].call(
    { kernelContext: firstContext, mountId: 1 }, "/value.bin", new Uint8Array([1])
  );
  await host["file/write"].call(
    { kernelContext: secondContext, mountId: 1 }, "/value.bin", new Uint8Array([2])
  );
  assert.deepEqual(
    await host["file/read"].call({ kernelContext: firstContext, mountId: 1 }, "/value.bin"),
    new Uint8Array([1])
  );
  assert.deepEqual(
    await host["file/read"].call({ kernelContext: secondContext, mountId: 1 }, "/value.bin"),
    new Uint8Array([2])
  );
});

test("indexeddb provider keys persist across fresh kernel-local mount ids", async () => {
  const firstHost = createHostServices({ dbName: "test-session-persistent" });
  const firstContext = {};
  await firstHost.filesystemHost.register(firstContext, 1, { provider: "indexeddb", key: "tutorial-board" });
  await firstHost["file/write"].call({ kernelContext: firstContext, mountId: 1 }, "/board.hal", new Uint8Array([7, 8]));
  await firstHost.filesystemHost.close(firstContext, 1);
  const secondHost = createHostServices({ dbName: "test-session-persistent" });
  const secondContext = {};
  await secondHost.filesystemHost.register(secondContext, 9, { provider: "indexeddb", key: "tutorial-board" });
  assert.deepEqual(
    await secondHost["file/read"].call({ kernelContext: secondContext, mountId: 9 }, "/board.hal"),
    new Uint8Array([7, 8])
  );
  await assert.rejects(
    secondHost["file/read"].call({ kernelContext: secondContext, mountId: 1 }, "/board.hal"),
    /mount-closed/
  );
});

test("canvas host calls route by originating session", async () => {
  const calls = [];
  const runtimes = new Map(["alpha", "beta"].map((session) => [
    session,
    {
      nextFrame: async (...args) => calls.push([session, "next", ...args]),
      render: async (...args) => calls.push([session, "render", ...args])
    }
  ]));
  const host = createHostServices({
    canvasRuntimeForSession: (session) => runtimes.get(session)
  });

  await host["studio.canvas/next-frame"].call({ sessionId: "alpha" }, "node", "canvas");
  await host["studio.canvas/render"].call({ sessionId: "beta" }, "node", "canvas", new Map());

  assert.deepEqual(calls, [
    ["alpha", "next", "node", "canvas"],
    ["beta", "render", "node", "canvas", new Map()]
  ]);
});

test("canvas render callbacks accept the node-aware HAL call shape", async () => {
  const calls = [];
  const host = createHostServices({
    renderCanvas: async (...args) => calls.push(args)
  });
  const scene = new Map([["bins", [1, 2, 3]]]);

  assert.equal(
    await host["studio.canvas/render"]("node/visualizer", "canvas/visualizer", scene),
    true
  );
  assert.deepEqual(calls, [["canvas/visualizer", scene]]);
});

test("node input preserves nested maps and vectors for HAL", async () => {
  const runtime = {
    in: async () => new Map([
      ["bins", [1, 2, 3]],
      ["meta", new Map([["source", "fft"]])]
    ])
  };
  const host = createHostServices({ nodeRuntime: runtime });

  const value = await host["node/in"]("node/visualizer", "fft/bins");
  assert.ok(value instanceof Map);
  assert.deepEqual(value.get("bins"), [1, 2, 3]);
  assert.deepEqual(value.get("meta"), new Map([["source", "fft"]]));
});

test("workspace scoped stores cannot read, write, or list another workspace", async () => {
  const contexts = new Map([["alpha-context", "alpha"], ["beta-context", "beta"]]);
  const host = createHostServices({
    dbName: "test-workspace-scope",
    scopeForContext: (context) => contexts.get(context)
  });
  const alpha = Object.fromEntries(Object.entries(host).map(([name, handler]) => [
    name, (...args) => handler.call({ context: "alpha-context" }, ...args)
  ]));
  const beta = Object.fromEntries(Object.entries(host).map(([name, handler]) => [
    name, (...args) => handler.call({ context: "beta-context" }, ...args)
  ]));

  await alpha["store/put"]("spaces/alpha/files/src/main.hal", "alpha");
  await beta["store/put"]("spaces/beta/files/src/main.hal", "beta");
  assert.equal(await alpha["store/get"]("spaces/alpha/files/src/main.hal"), "alpha");
  assert.deepEqual(await alpha["store/keys"](), ["spaces/alpha/files/src/main.hal"]);
  await assert.rejects(
    alpha["store/get"]("spaces/beta/files/src/main.hal"),
    /workspace-scope-denied:alpha/
  );
  await assert.rejects(
    alpha["store/put"]("spaces/beta/files/leak.hal", "no"),
    /workspace-scope-denied:alpha/
  );
});

test("workspace scoped stores resolve scope from the owning kernel context", async () => {
  const kernelContext = {};
  const host = createHostServices({
    dbName: "test-workspace-kernel-scope",
    scopeForContext: (context) => context === kernelContext ? "alpha" : null
  });
  const invocation = { context: { session: "ROOT" }, kernelContext };

  await host["store/put"].call(invocation, "spaces/alpha/files/src/main.hal", "alpha");
  assert.equal(
    await host["store/get"].call(invocation, "spaces/alpha/files/src/main.hal"),
    "alpha"
  );
});

test("http/get returns the response body as text", async () => {
  const calls = [];
  const fetch = async (url) => {
    calls.push(url);
    return { ok: true, status: 200, text: async () => "body text" };
  };
  const host = createHostServices({ dbName: "test-http-ok", fetch });
  assert.equal(await host["http/get"]("https://example.test/data"), "body text");
  assert.deepEqual(calls, ["https://example.test/data"]);
});

test("http/get rejects with the status code on HTTP errors", async () => {
  const fetch = async () => ({ ok: false, status: 404, text: async () => "nope" });
  const host = createHostServices({ dbName: "test-http-error", fetch });
  await assert.rejects(host["http/get"]("https://example.test/missing"), /404/);
});

test("json/parse decodes JSON text into maps, arrays, and scalars", async () => {
  const host = createHostServices({ dbName: "test-json" });
  const value = await host["json/parse"](
    '{"name":"x","count":2,"flag":true,"missing":null,"files":[{"path":"/a.hal"}]}'
  );
  assert.ok(value instanceof Map);
  assert.equal(value.get("name"), "x");
  assert.equal(value.get("count"), 2);
  assert.equal(value.get("flag"), true);
  assert.equal(value.get("missing"), null);
  assert.deepEqual(value.get("files"), [new Map([["path", "/a.hal"]])]);
});

test("json/parse preserves integers beyond JavaScript's safe range", async () => {
  const host = createHostServices({ dbName: "test-json-bigint" });
  const value = await host["json/parse"]('{"small":9223372036854775807,"large":9223372036854775808}');
  assert.equal(value.get("small"), 9223372036854775807n);
  assert.equal(value.get("large"), 9223372036854775808n);
});

test("json/parse rejects invalid JSON", async () => {
  const host = createHostServices({ dbName: "test-json-bad" });
  await assert.rejects(host["json/parse"]("{nope"));
});

test("graph host services expose exact generated-program host-call keys", async () => {
  const calls = [];
  const graph = {
    programs: { release: async (id) => { calls.push(["program/release", id]); return true; } },
    install: async (descriptor, options) => { calls.push(["program/install", descriptor, options]); return { programId: "example/node" }; },
    programInfo: (id) => ({ programId: id }),
    spawn: async (descriptor) => ({ nodeId: descriptor["node/id"] }),
    release: async () => true,
    connect: () => "connection-1",
    disconnect: () => true,
    sendFrame: async () => ({ accepted: true }),
    callFrame: async () => ({ data: 42 }),
    info: (id) => ({ id }),
    list: () => []
  };
  const services = createGraphHostServices(graph, { capabilities: ["surface/canvas-2d"] });
  const installed = await services["program/install"](
    new Map([["program/id", "example/node"]]),
    new Map([["sessionId", "UI"]])
  );
  assert.ok(installed instanceof Map);
  assert.equal(installed.get("programId"), "example/node");
  assert.deepEqual(calls[0], ["program/install", { "program/id": "example/node" }, { sessionId: "UI" }]);
  const described = await services["host/describe"]();
  assert.equal(described.get("host/version"), "hara.host.v1");
  assert.deepEqual(await services["host/capabilities"](), ["surface/canvas-2d"]);
});

test("session host calls register explicit ingress and release its graph partition", async () => {
  const released = [];
  const sessions = new SessionRouter();
  const graph = { releaseSession: async (id) => released.push(id) };
  const services = createGraphHostServices(graph, { sessionRouter: sessions });
  const context = { call: async () => null };
  const registered = services["session/register-ingress"].call(
    { context }, "UI", ["input/keyboard"]
  );
  assert.ok(registered instanceof Map);
  assert.equal(registered.get("sessionId"), "UI");
  const subscription = await services["session/subscribe"]("UI", "selected", "callback/1");
  assert.equal(typeof subscription, "string");
  assert.equal(await services["session/unsubscribe"](subscription), true);
  assert.equal(await services["session/unregister-ingress"]("UI"), true);
  assert.deepEqual(released, ["UI"]);
});
