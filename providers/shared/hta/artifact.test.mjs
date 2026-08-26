import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";
import { decodeHta, encodeHta, HtaKeyword } from "../../../core/rust/web/packages/hta/index.js";

const providers = [
  ["github", "github", "hara/filesystem-github"],
  ["webdav", "webdav", "hara/filesystem-webdav"],
  ["s3", "s3", "hara/filesystem-s3"],
  ["google-drive", "google-drive", "hara/filesystem-google-drive"],
  ["sftp", "sftp", "hara/filesystem-sftp"],
  ["indexeddb", "indexeddb", "hara/filesystem-indexeddb"]
];

function field(value, name) {
  if (!(value instanceof Map)) return undefined;
  for (const [key, item] of value) {
    if (key === name || key?.name === name) return item;
  }
  return undefined;
}

async function exerciseArtifact(name, provider, identity) {
  const artifactUrl = new URL(`../../${name}/hta/provider/provider.wasm`, import.meta.url);
  const moduleBytes = new Uint8Array(await readFile(artifactUrl));
  const messages = [];
  const listeners = new Map();
  const previousSelf = globalThis.self;
  globalThis.self = {
    addEventListener(type, handler) { listeners.set(type, handler); },
    postMessage(message) { messages.push(message); },
    close() {}
  };
  try {
    await import(`../../../core/rust/web/packages/hta/worker.mjs?shared-artifact=${name}-${Date.now()}`);
    const receive = listeners.get("message");
    await receive({ data: { type: "init", moduleBytes } });
    assert.equal(messages.at(-1).type, "ready", `${name} Wasm artifact did not initialize`);

    messages.length = 0;
    await receive({ data: { type: "call", id: 1, frame: encodeHta(["describe", []]) } });
    const describeCall = messages.find(message => message.type === "host-call");
    assert.deepEqual(
      [describeCall.service, describeCall.method],
      [`filesystem.${provider}`, "describe"]
    );
    await receive({
      data: {
        type: "delivery",
        call: describeCall.call,
        ok: true,
        frame: encodeHta(new Map([
          [new HtaKeyword("provider"), provider],
          [new HtaKeyword("identity"), identity]
        ]))
      }
    });
    const result = messages.find(message => message.type === "result");
    assert.equal(result.ok, true);
    assert.equal(field(decodeHta(result.frame), "identity"), identity);

    messages.length = 0;
    await receive({ data: { type: "call", id: 2, frame: encodeHta(["open", [new Map()]]) } });
    const openCall = messages.find(message => message.type === "host-call");
    assert.equal(openCall.method, "open");
    await receive({ data: { type: "cancel", id: 2 } });
    const cancellation = messages.find(message => message.type === "host-cancel");
    assert.equal(cancellation.calls[0].service, `filesystem.${provider}`);
    assert.equal(cancellation.calls[0].method, "open");
    assert.equal(messages.some(message => message.type === "result" && message.id === 2), false);
  } finally {
    if (previousSelf === undefined) delete globalThis.self;
    else globalThis.self = previousSelf;
  }
}

test("all rich filesystem provider artifacts load the shared protocol façade", async () => {
  for (const [name, provider, identity] of providers) {
    await exerciseArtifact(name, provider, identity);
  }
});
