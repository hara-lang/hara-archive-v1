import assert from "node:assert/strict";
import test from "node:test";
import { webcrypto } from "node:crypto";

globalThis.crypto ??= webcrypto;

import {
  activateLockedPackages,
  fetchVerifiedPackage,
  inspectHarp,
  sha256
} from "./package-cache.js";

async function signaturePair(statement) {
  const pair = await crypto.subtle.generateKey({ name: "Ed25519" }, true, ["sign", "verify"]);
  return {
    publicKey: await crypto.subtle.exportKey("raw", pair.publicKey),
    statement,
    signature: await crypto.subtle.sign({ name: "Ed25519" }, pair.privateKey, new TextEncoder().encode(statement))
  };
}

test("verified packages require both detached signatures and the declared digest", async () => {
  const bytes = new TextEncoder().encode("deterministic harp").buffer;
  const publisher = await signaturePair("publisher intent");
  const registry = await signaturePair("registry attestation");
  const cache = new Map();
  const response = { ok: true, arrayBuffer: async () => bytes };
  cache.match = async (key) => cache.get(key);
  cache.put = async (key, value) => cache.set(key, value);
  const result = await fetchVerifiedPackage({
    url: "https://github.example/release.harp",
    digest: await sha256(bytes), publisher, registry, cache,
    fetchImpl: async () => response
  });
  assert.deepEqual(new Uint8Array(result), new Uint8Array(bytes));
});

test("a missing or invalid attestation rejects before cache insertion", async () => {
  const bytes = new TextEncoder().encode("archive").buffer;
  const publisher = await signaturePair("publisher intent");
  await assert.rejects(
    fetchVerifiedPackage({
      url: "https://github.example/release.harp", digest: await sha256(bytes), publisher,
      fetchImpl: async () => ({ ok: true, arrayBuffer: async () => bytes })
    }),
    /signature-invalid/
  );
});

test("HARP inspection verifies every file and activates namespaces atomically", async () => {
  const encoder = new TextEncoder();
  const files = new Map([
    ["project.edn", encoder.encode("{:hara/type :project}\n")],
    ["src/example/main.hal", encoder.encode("(ns example.main) 42\n")],
    ["halc/example.fast.halc", Uint8Array.from([72, 65, 76, 67, 1])]
  ]);
  const tree = [];
  let declarations = "";
  for (const [path, bytes] of files) {
    tree.push(encoder.encode(path), Uint8Array.of(0), bytes);
    declarations += `  ${JSON.stringify(path)} {:sha256 ${JSON.stringify(await sha256(bytes))} :size ${bytes.length}}\n`;
  }
  const treeBytes = concat(tree);
  const manifest = encoder.encode(
    `{:harp/format \"0.0.0-alpha\"\n :package {:identity "example/app" :version "1.0.0"}\n`
    + ` :files {\n${declarations}} :resources {"example.main" "src/example/main.hal" "example.fast" "halc/example.fast.halc"}`
    + ` :extensions {demo.extension {:targets {:browser {:provider "src/example/main.hal"}}}}\n`
    + ` :integrity {:tree-sha256 ${JSON.stringify(await sha256(treeBytes))}}}\n`
  );
  const archiveBytes = storedZip(new Map([["package.edn", manifest], ...files]));
  const archive = await inspectHarp(archiveBytes);
  assert.deepEqual(archive.resources.get("example.main"), {
    format: "hal", source: "(ns example.main) 42\n"
  });
  assert.deepEqual(archive.resources.get("example.fast"), {
    format: "halc", bytes: Uint8Array.from([72, 65, 76, 67, 1])
  });
  assert.equal(archive.extensions[0].namespace, "demo.extension");

  const calls = [];
  await activateLockedPackages({
    packages: [{ bytes: archiveBytes, digest: await sha256(archiveBytes) }],
    context: { call: async (...args) => calls.push(args) }
  });
  assert.equal(calls.length, 2);
  assert.equal(calls[0][0], "register-resources");
  assert.deepEqual(calls[0][1][0], [["example.main", "(ns example.main) 42\n"]]);
  assert.equal(calls[1][0], "eval-halc");
  assert.deepEqual(calls[1][1][0], Uint8Array.from([72, 65, 76, 67, 1]));

  const tampered = archiveBytes.slice();
  tampered[tampered.lastIndexOf("4".charCodeAt(0))] ^= 1;
  await assert.rejects(inspectHarp(tampered), /package\/file-integrity|package\/manifest|package\/zip-malformed/);
});

function storedZip(entries) {
  const encoder = new TextEncoder();
  const local = [], central = [];
  let offset = 0;
  for (const [path, data] of entries) {
    const name = encoder.encode(path);
    const localHeader = header(30);
    write32(localHeader, 0, 0x04034b50);
    write16(localHeader, 4, 20);
    write32(localHeader, 18, data.length);
    write32(localHeader, 22, data.length);
    write16(localHeader, 26, name.length);
    local.push(localHeader, name, data);

    const centralHeader = header(46);
    write32(centralHeader, 0, 0x02014b50);
    write16(centralHeader, 4, 20);
    write16(centralHeader, 6, 20);
    write32(centralHeader, 20, data.length);
    write32(centralHeader, 24, data.length);
    write16(centralHeader, 28, name.length);
    write32(centralHeader, 42, offset);
    central.push(centralHeader, name);
    offset += localHeader.length + name.length + data.length;
  }
  const localBytes = concat(local), centralBytes = concat(central);
  const end = header(22);
  write32(end, 0, 0x06054b50);
  write16(end, 8, entries.size);
  write16(end, 10, entries.size);
  write32(end, 12, centralBytes.length);
  write32(end, 16, localBytes.length);
  return concat([localBytes, centralBytes, end]);
}

function header(size) { return new Uint8Array(size); }
function write16(bytes, offset, value) {
  new DataView(bytes.buffer).setUint16(offset, value, true);
}
function write32(bytes, offset, value) {
  new DataView(bytes.buffer).setUint32(offset, value, true);
}
function concat(parts) {
  const bytes = new Uint8Array(parts.reduce((size, part) => size + part.length, 0));
  let offset = 0;
  for (const part of parts) {
    bytes.set(part, offset);
    offset += part.length;
  }
  return bytes;
}
