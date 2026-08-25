import assert from "node:assert/strict";
import test from "node:test";
import { decodeHta, encodeHta, HtaKeyword } from "./index.js";

class FakePort {
  messages = [];
  listener;

  addEventListener(event, listener) {
    assert.equal(event, "message");
    this.listener = listener;
  }

  start() {}

  postMessage(message) {
    this.messages.push(message);
  }

  send(message) {
    this.listener?.({ data: message });
  }
}

function tick() {
  return new Promise(resolve => setTimeout(resolve, 0));
}

test("shared workers can host the same generic provider backend", async () => {
  const previousSelf = globalThis.self;
  globalThis.self = {};
  try {
    await import(`./shared-worker.js?provider-test=${Date.now()}`);
    const port = new FakePort();
    globalThis.self.onconnect({ ports: [port] });
    const providerUrl = `data:text/javascript,${encodeURIComponent(
      "export default async (operation, args) => ({ operation, value: args[0] });"
    )}`;

    port.send({
      type: "init",
      backend: "provider",
      providerUrl,
      instrumentation: true
    });
    await tick();
    assert.equal(port.messages.find(message => message.type === "ready")?.type, "ready");

    port.send({ type: "call", id: 5, frame: encodeHta(["echo", [42]]) });
    await tick();
    const result = port.messages.find(message => message.type === "result");
    assert.ok(result);
    assert.deepEqual(decodeHta(result.frame), new Map([
      [new HtaKeyword("operation"), "echo"],
      [new HtaKeyword("value"), 42]
    ]));

    port.send({ type: "close" });
    await tick();
    assert.equal(port.messages.at(-1).type, "closed");
    assert.deepEqual(
      port.messages
        .filter(message => message.type === "provider-event")
        .map(message => message.event.event),
      ["start", "call-enter", "call-return", "shutdown"]
    );
  } finally {
    globalThis.self = previousSelf;
  }
});
