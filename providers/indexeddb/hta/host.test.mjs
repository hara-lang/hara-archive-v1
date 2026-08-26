import assert from "node:assert/strict";
import test from "node:test";
import "../../../core/rust/web/node_modules/fake-indexeddb/auto/index.js";
import { createIndexedDbWasmHost, plain } from "./host.mjs";

let sequence = 0;

function call(host, method, args, receiver) {
  return host.hostCalls[`filesystem.indexeddb/${method}`].call(receiver, ...args);
}

function receiver(context, callId = undefined) {
  return {
    kernelContext: context,
    signal: new AbortController().signal,
    ...(callId === undefined ? {} : { call: callId })
  };
}

async function removeDatabase(name) {
  await new Promise((resolve, reject) => {
    const request = indexedDB.deleteDatabase(name);
    request.onsuccess = resolve;
    request.onerror = () => reject(request.error);
    request.onblocked = () => reject(new Error(`database deletion blocked: ${name}`));
  });
}

test("IndexedDB rich host exposes redacted descriptors and exact filesystem operations", async () => {
  const database = `hara-rich-indexeddb-${++sequence}`;
  const host = createIndexedDbWasmHost({ databaseName: database });
  const context = {};
  const callReceiver = receiver(context);
  try {
    const description = plain(await call(host, "describe", [], callReceiver));
    assert.equal(description.provider, "indexeddb");
    const opened = plain(await call(host, "open", [{ namespace: "workspace" }], callReceiver));
    assert.equal(opened.descriptor.kind, "indexeddb");
    assert.equal(JSON.stringify(opened).includes(database), false);

    await call(host, "request", [opened.mount, "mkdir", ["/docs", { parents: true }, {}]], callReceiver);
    await call(host, "request", [
      opened.mount,
      "write",
      ["/docs/value.bin", Uint8Array.of(0, 255, 1), { mode: "create" }, {}]
    ], callReceiver);
    assert.deepEqual(
      [...await call(host, "request", [opened.mount, "read", ["/docs/value.bin"]], callReceiver)],
      [0, 255, 1]
    );
    const page = plain(await call(host, "request", [
      opened.mount, "entries-page", ["/docs", { limit: 1 }]
    ], callReceiver));
    assert.deepEqual(page.entries.map(entry => entry.path), ["/docs/value.bin"]);
    const stat = plain(await call(host, "request", [opened.mount, "stat", ["/docs/value.bin"]], callReceiver));
    assert.equal(stat.size, 3);
    await call(host, "close", [opened.mount], callReceiver);
  } finally {
    await host.closeAll();
    await removeDatabase(database);
  }
});

test("IndexedDB rich host maps cancellation to the transactional provider", async () => {
  const database = `hara-rich-indexeddb-cancel-${++sequence}`;
  const host = createIndexedDbWasmHost({ databaseName: database });
  const context = {};
  const openReceiver = receiver(context);
  try {
    const opened = plain(await call(host, "open", [{ namespace: "workspace" }], openReceiver));
    const controller = new AbortController();
    const pendingReceiver = { ...receiver(context, "cancel-1"), signal: controller.signal };
    const pending = call(host, "request", [opened.mount, "stat", ["/missing"]], pendingReceiver);
    assert.equal(await call(host, "cancel", [opened.mount, "cancel-1"], pendingReceiver), true);
    controller.abort();
    await assert.rejects(pending, error => ["file/cancelled", "file/not-found"].includes(error.code));
  } finally {
    await host.closeAll();
    await removeDatabase(database);
  }
});
