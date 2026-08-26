import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { parseHtaManifest } from "../../../core/rust/web/packages/hta/index.js";

const descriptorUrl = new URL("./extension.edn", import.meta.url);

function sorted(values) {
  return [...values].sort();
}

test("GitHub declares one rich HTA Wasm route and trusted host boundary", async () => {
  const source = await readFile(fileURLToPath(descriptorUrl), "utf8");
  const manifest = parseHtaManifest(source);
  assert.equal(manifest.namespace, "fs.github.wasm");
  assert.equal(manifest.identity, "hara/filesystem-github");
  assert.equal(manifest.provider, "wasm");
  assert.equal(manifest.root, "provider");
  assert.equal(manifest.module, "provider.wasm");
  assert.equal(manifest.abi, "hta.v1");
  assert.deepEqual(sorted(manifest.capabilities), ["filesystem", "network"]);
  assert.deepEqual(manifest.hostCalls["filesystem.github"], [
    "describe", "open", "request", "cancel", "close"
  ]);
  assert.deepEqual(
    sorted(manifest.exports),
    sorted([
      "describe", "open", "descriptor", "stat", "read", "write",
      "entries-page", "mkdir", "delete", "copy", "move", "close"
    ])
  );
  assert.equal(manifest.exportArity.write, 5);
  assert.equal(manifest.exportSpecs.read.async, true);
  assert.deepEqual(
    manifest.hostCallCapabilities["filesystem.github/request"],
    ["filesystem", "network"]
  );
});
