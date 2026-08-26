import assert from "node:assert/strict";
import { access, readFile } from "node:fs/promises";
import test from "node:test";
import { decodeHta, encodeHta, HtaKeyword } from "../../../core/rust/web/packages/hta/index.js";

const artifactUrl = new URL(
  "./provider/provider.wasm",
  import.meta.url
);

async function artifactAvailable() {
  try {
    await access(artifactUrl);
    return true;
  } catch {
    return false;
  }
}

test("the built GitHub Wasm source crosses the HTA host boundary", { skip: !(await artifactAvailable()) }, async () => {
  const messages = [];
  const listeners = new Map();
  const previousSelf = globalThis.self;
  globalThis.self = {
    addEventListener(type, handler) { listeners.set(type, handler); },
    postMessage(message) { messages.push(message); },
    close() {}
  };
  try {
    await import(`../../../core/rust/web/packages/hta/worker.mjs?github-provider=${Date.now()}`);
    const receive = listeners.get("message");
    const moduleBytes = new Uint8Array(await readFile(artifactUrl));
    await receive({ data: { type: "init", moduleBytes } });
    assert.equal(messages.at(-1).type, "ready");

    messages.length = 0;
    await receive({ data: { type: "call", id: 1, frame: encodeHta(["describe", []]) } });
    const describeCall = messages.find(message => message.type === "host-call");
    assert.deepEqual(
      [describeCall.service, describeCall.method],
      ["filesystem.github", "describe"]
    );
    await receive({
      data: {
        type: "delivery",
        call: describeCall.call,
        ok: true,
        frame: encodeHta(new Map([
          [new HtaKeyword("provider"), "github"],
          [new HtaKeyword("identity"), "hara/filesystem-github"]
        ]))
      }
    });
    const result = messages.find(message => message.type === "result");
    assert.equal(result.ok, true);
    const value = decodeHta(result.frame);
    const provider = [...value].find(([key]) => key === "provider" || key?.name === "provider")?.[1];
    assert.equal(provider, "github");

    messages.length = 0;
    await receive({ data: { type: "call", id: 2, frame: encodeHta(["open", [new Map()]]) } });
    const openCall = messages.find(message => message.type === "host-call");
    assert.equal(openCall.method, "open");
    await receive({ data: { type: "cancel", id: 2 } });
    const cancellation = messages.find(message => message.type === "host-cancel");
    assert.equal(cancellation.calls[0].call, openCall.call);
    assert.equal(cancellation.calls[0].service, "filesystem.github");
    assert.equal(messages.some(message => message.type === "result" && message.id === 2), false);
  } finally {
    if (previousSelf === undefined) delete globalThis.self;
    else globalThis.self = previousSelf;
  }
});
