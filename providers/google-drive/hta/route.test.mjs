import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { parseHtaManifest } from "../../../core/rust/web/packages/hta/index.js";

test("Google Drive declares the rich HTA Wasm route and complete filesystem export surface", async () => {
  const source = await readFile(fileURLToPath(new URL("./extension.edn", import.meta.url)), "utf8");
  const manifest = parseHtaManifest(source);
  assert.equal(manifest.namespace, "fs.google-drive.wasm");
  assert.equal(manifest.identity, "hara/filesystem-google-drive");
  assert.equal(manifest.provider, "wasm");
  assert.equal(manifest.root, "provider");
  assert.equal(manifest.module, "provider.wasm");
  assert.equal(manifest.abi, "hta.v1");
  assert.deepEqual(manifest.hostCalls["filesystem.google-drive"], ["describe", "open", "request", "cancel", "close"]);
  assert.deepEqual([...manifest.exports].sort(), [
    "close", "copy", "delete", "describe", "descriptor", "entries-page",
    "mkdir", "move", "open", "read", "stat", "write"
  ]);
});
