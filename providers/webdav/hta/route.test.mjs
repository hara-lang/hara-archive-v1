import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { parseHtaManifest } from "../../../core/rust/web/packages/hta/index.js";

const descriptorUrl = new URL("./extension.edn", import.meta.url);

function sorted(values) {
  return [...values].sort();
}

test("WebDAV declares one exact HTA browser route and host authority", async () => {
  const source = await readFile(fileURLToPath(descriptorUrl), "utf8");
  const manifest = parseHtaManifest(source);
  assert.equal(manifest.namespace, "fs.webdav.wasm.hta");
  assert.equal(manifest.identity, "hara/filesystem-webdav");
  assert.equal(manifest.provider, "hta");
  assert.equal(manifest.abi, "hta.v1");
  assert.deepEqual(manifest.browserTarget, {
    module: "browser/worker.mjs",
    runtime: "web-worker"
  });
  assert.deepEqual(sorted(manifest.capabilities), ["filesystem", "network"]);
  assert.deepEqual(manifest.hostCalls["filesystem.webdav"], ["open", "request", "cancel", "close"]);
  assert.deepEqual(
    sorted(manifest.exports),
    sorted([
      "describe",
      "open",
      "descriptor",
      "stat",
      "read",
      "write",
      "entries-page",
      "mkdir",
      "delete",
      "copy",
      "move",
      "close"
    ])
  );
  assert.equal(manifest.exportArity.write, 5);
  assert.equal(manifest.exportArity["entries-page"], 3);
  assert.deepEqual(
    manifest.hostCallCapabilities["filesystem.webdav/request"],
    ["filesystem", "network"]
  );
});
