import assert from "node:assert/strict";
import test from "node:test";
import { decodeHta, encodeHta } from "./index.js";
import { HTA_PROVIDER_EVENT, HTA_PROVIDER_EVENT_SCHEMA } from "./provider-common.mjs";
import { serveNodeProvider } from "./provider-node.mjs";

class FakeInput {
  listeners = new Map();

  on(event, listener) {
    this.listeners.set(event, listener);
  }

  send(value) {
    this.listeners.get("data")?.(value);
  }

  end() {
    this.listeners.get("end")?.();
  }
}

class FakeOutput {
  chunks = [];

  write(value) {
    this.chunks.push(new Uint8Array(value));
  }

  frames() {
    const bytes = new Uint8Array(this.chunks.reduce((size, chunk) => size + chunk.length, 0));
    let offset = 0;
    for (const chunk of this.chunks) {
      bytes.set(chunk, offset);
      offset += chunk.length;
    }
    const result = [];
    offset = 0;
    while (offset < bytes.length) {
      const size = new DataView(bytes.buffer, bytes.byteOffset + offset, 4).getUint32(0, false);
      offset += 4;
      result.push(decodeHta(bytes.slice(offset, offset + size)));
      offset += size;
    }
    return result;
  }
}

function send(input, value) {
  const frame = encodeHta(value);
  const bytes = new Uint8Array(frame.length + 4);
  new DataView(bytes.buffer).setUint32(0, frame.length, false);
  bytes.set(frame, 4);
  input.send(bytes);
}

function tick() {
  return new Promise(resolve => setTimeout(resolve, 0));
}

test("Node providers expose the shared lifecycle contract", async () => {
  const input = new FakeInput();
  const output = new FakeOutput();
  const events = [];
  let releases = 0;
  let closes = 0;
  let exited;
  serveNodeProvider(
    async (operation, args) => `${operation}:${args[0]}`,
    {
      input,
      output,
      redirectConsole: false,
      onEvent: event => events.push(event),
      release: async () => { releases += 1; },
      close: async () => { closes += 1; },
      exit: code => { exited = code; }
    }
  );

  send(input, ["handshake", 1, "demo.provider", ["echo"]]);
  await tick();
  send(input, ["call", 7, "echo", [41]]);
  await tick();
  send(input, ["release", null]);
  await tick();
  send(input, ["shutdown"]);
  await tick();

  assert.deepEqual(output.frames(), [["ready", 1], ["result", 7, "echo:41"]]);
  assert.equal(releases, 1);
  assert.equal(closes, 1);
  assert.equal(exited, 0);
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
    origin: "node",
    event: HTA_PROVIDER_EVENT.CALL_ENTER,
    request: 7,
    operation: "echo"
  });
});

test("Node cancellation aborts provider work without late delivery", async () => {
  const input = new FakeInput();
  const output = new FakeOutput();
  const events = [];
  let exited;
  serveNodeProvider(
    async (_operation, _args, context) => new Promise((resolve, reject) => {
      context.signal.addEventListener("abort", () => reject(new Error("cancelled")), { once: true });
    }),
    {
      input,
      output,
      redirectConsole: false,
      onEvent: event => events.push(event),
      exit: code => { exited = code; }
    }
  );

  send(input, ["handshake", 1, "demo.provider", ["wait"]]);
  send(input, ["call", 9, "wait", []]);
  await tick();
  send(input, ["cancel", 9]);
  await tick();
  send(input, ["shutdown"]);
  await tick();

  assert.deepEqual(output.frames(), [["ready", 1]]);
  assert.deepEqual(events.map(event => event.event), [
    HTA_PROVIDER_EVENT.START,
    HTA_PROVIDER_EVENT.CALL_ENTER,
    HTA_PROVIDER_EVENT.CANCEL,
    HTA_PROVIDER_EVENT.SHUTDOWN
  ]);
  assert.equal(exited, 0);
});

test("Node provider host calls use the browser context shape", async () => {
  const input = new FakeInput();
  const output = new FakeOutput();
  const events = [];
  let exited;
  serveNodeProvider(
    async (_operation, _args, context) => context.hostCall("store", "get", ["answer"]),
    {
      input,
      output,
      redirectConsole: false,
      onEvent: event => events.push(event),
      hostCall: async (service, method, args) => `${service}/${method}:${args[0]}`,
      exit: code => { exited = code; }
    }
  );

  send(input, ["handshake", 1, "demo.provider", ["read"]]);
  await tick();
  send(input, ["call", 4, "read", []]);
  await tick();
  send(input, ["shutdown"]);
  await tick();

  assert.deepEqual(output.frames(), [["ready", 1], ["result", 4, "store/get:answer"]]);
  assert.deepEqual(events.map(event => event.event), [
    HTA_PROVIDER_EVENT.START,
    HTA_PROVIDER_EVENT.CALL_ENTER,
    HTA_PROVIDER_EVENT.HOST_CALL,
    HTA_PROVIDER_EVENT.HOST_CALL,
    HTA_PROVIDER_EVENT.CALL_RETURN,
    HTA_PROVIDER_EVENT.SHUTDOWN
  ]);
  assert.equal(events[2].status, "enter");
  assert.equal(events[3].status, "ok");
  assert.equal(exited, 0);
});
