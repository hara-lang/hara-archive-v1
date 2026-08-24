import assert from "node:assert/strict";
import test from "node:test";
import { decodeHta, encodeHta, HtaKeyword } from "./index.js";
import { serveBrowserProvider } from "./provider-browser.mjs";

class FakeWorkerScope {
  constructor() {
    this.listeners = [];
    this.messages = [];
    this.closed = false;
  }

  addEventListener(type, listener) {
    if (type === "message") this.listeners.push(listener);
  }

  postMessage(message) {
    this.messages.push(message);
  }

  async emit(data) {
    await Promise.all(this.listeners.map(listener => listener({ data })));
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
  serveBrowserProvider(
    async (_operation, _args, context) => {
      const reply = await context.hostCall("filesystem.webdav", "open", ["request-1"]);
      return { answer: field(reply, "answer") };
    },
    { scope }
  );

  await scope.emit({ type: "init" });
  assert.equal(scope.messages.shift().type, "ready");

  const invocation = scope.emit({
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

  await scope.emit({
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
  serveBrowserProvider(
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

  const invocation = scope.emit({
    type: "call",
    id: 9,
    frame: encodeHta(["read", []])
  });
  await new Promise(resolve => setTimeout(resolve, 0));
  await scope.emit({ type: "cancel", id: 9 });
  await invocation;
  assert.equal(aborted, true);
  assert.equal(scope.messages.some(message => message.type === "result" && message.id === 9), false);
});

test("worker close invokes provider cleanup and closes the scope once", async () => {
  const scope = new FakeWorkerScope();
  let closes = 0;
  serveBrowserProvider(async () => null, {
    scope,
    close: async () => { closes += 1; }
  });
  await scope.emit({ type: "close" });
  await scope.emit({ type: "close" });
  assert.equal(closes, 1);
  assert.equal(scope.closed, true);
});

test("worker close still terminates the scope when provider cleanup fails", async () => {
  const scope = new FakeWorkerScope();
  serveBrowserProvider(async () => null, {
    scope,
    close: async () => { throw new Error("cleanup failed"); }
  });
  await scope.emit({ type: "close" });
  assert.equal(scope.closed, true);
  assert.equal(scope.messages.at(-1).type, "fatal");
  assert.match(scope.messages.at(-1).error.message, /cleanup failed/);
});
