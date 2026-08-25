import assert from "node:assert/strict";
import test from "node:test";
import { decodeHta, encodeHta, HtaKeyword } from "./index.js";
import { HTA_PROVIDER_EVENT, HTA_PROVIDER_EVENT_SCHEMA } from "./provider-common.mjs";
import { createBrowserProvider } from "./provider-browser.mjs";

class FakeWorkerScope {
  constructor() {
    this.messages = [];
    this.closed = false;
  }

  postMessage(message) {
    this.messages.push(message);
  }

  close() {
    this.closed = true;
  }
}

function field(map, name) {
  for (const [key, value] of map) {
    if ((typeof key?.name === "string" ? key.name : String(key)) === name) return value;
  }
}

test("browser providers can issue manifest-authorized host calls", async () => {
  const scope = new FakeWorkerScope();
  const provider = createBrowserProvider(
    async (_operation, _args, context) => {
      const reply = await context.hostCall("filesystem.webdav", "open", ["request-1"]);
      return { answer: field(reply, "answer") };
    },
    { scope }
  );

  const invocation = provider.handle({
    type: "call",
    id: 7,
    frame: encodeHta(["open", []])
  });
  await new Promise(resolve => setTimeout(resolve, 0));
  const outbound = scope.messages.shift();
  assert.equal(outbound.type, "host-call");
  assert.equal(outbound.service, "filesystem.webdav");
  assert.equal(outbound.method, "open");
  assert.deepEqual(decodeHta(outbound.frame), ["request-1"]);

  await provider.handle({
    type: "delivery",
    call: outbound.call,
    ok: true,
    frame: encodeHta(new Map([[new HtaKeyword("answer"), 42]]))
  });
  await invocation;

  const result = scope.messages.shift();
  assert.equal(result.type, "result");
  assert.equal(result.id, 7);
  assert.equal(result.ok, true);
  assert.equal(field(decodeHta(result.frame), "answer"), 42);
});

test("top-level cancellation aborts provider work and suppresses late delivery", async () => {
  const scope = new FakeWorkerScope();
  let aborted = false;
  const provider = createBrowserProvider(
    async (_operation, _args, context) => {
      await new Promise((resolve, reject) => {
        context.signal.addEventListener("abort", () => {
          aborted = true;
          reject(new Error("cancelled"));
        }, { once: true });
      });
    },
    { scope }
  );

  const invocation = provider.handle({
    type: "call",
    id: 9,
    frame: encodeHta(["read", []])
  });
  await new Promise(resolve => setTimeout(resolve, 0));
  await provider.handle({ type: "cancel", id: 9 });
  await invocation;
  assert.equal(aborted, true);
  assert.equal(scope.messages.some(message => message.type === "result" && message.id === 9), false);
});

test("provider close invokes cleanup exactly once", async () => {
  const scope = new FakeWorkerScope();
  let closes = 0;
  const provider = createBrowserProvider(async () => null, {
    scope,
    close: async () => { closes += 1; }
  });
  await provider.handle({ type: "close" });
  await provider.handle({ type: "close" });
  assert.equal(closes, 1);
  assert.equal(scope.closed, false);
});

test("provider cleanup failures are reported to the owning worker", async () => {
  const scope = new FakeWorkerScope();
  const provider = createBrowserProvider(async () => null, {
    scope,
    close: async () => { throw new Error("cleanup failed"); }
  });
  await provider.handle({ type: "close" });
  assert.equal(scope.closed, false);
  assert.equal(scope.messages.at(-1).type, "fatal");
  assert.match(scope.messages.at(-1).error.message, /cleanup failed/);
});

test("browser providers expose the same lifecycle contract as Node", async () => {
  const scope = new FakeWorkerScope();
  const events = [];
  const provider = createBrowserProvider(
    async (operation, args) => `${operation}:${args[0]}`,
    {
      scope,
      onEvent: event => events.push(event),
      release: async () => {}
    }
  );

  await provider.handle({ type: "call", id: 7, frame: encodeHta(["echo", [41]]) });
  await provider.handle({ type: "release", frame: encodeHta(null) });
  await provider.handle({ type: "close" });

  assert.equal(decodeHta(scope.messages[0].frame), "echo:41");
  assert.deepEqual(events.map(event => event.event), [
    HTA_PROVIDER_EVENT.START,
    HTA_PROVIDER_EVENT.CALL_ENTER,
    HTA_PROVIDER_EVENT.CALL_RETURN,
    HTA_PROVIDER_EVENT.RELEASE,
    HTA_PROVIDER_EVENT.SHUTDOWN
  ]);
  assert.ok(events.every(event => event.schema === HTA_PROVIDER_EVENT_SCHEMA));
  assert.deepEqual(events[1], {
    schema: HTA_PROVIDER_EVENT_SCHEMA,
    sequence: 2,
    origin: "browser",
    event: HTA_PROVIDER_EVENT.CALL_ENTER,
    request: 7,
    operation: "echo"
  });
});
