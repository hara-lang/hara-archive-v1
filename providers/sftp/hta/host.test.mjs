import assert from "node:assert/strict";
import test from "node:test";
import { createSftpHost, normalisePath, plain } from "./host.mjs";

function makeClient() {
  const nodes = new Map([
    ["/srv/data", { type: "directory", revision: "root-r1" }],
    ["/srv/data/docs", { type: "directory", revision: "docs-r1" }],
    ["/srv/data/docs/a.txt", { type: "file", size: 3, revision: "a-r1", bytes: Uint8Array.of(0, 255, 1) }],
    ["/srv/data/docs/link", { type: "symlink", revision: "link-r1" }]
  ]);
  const client = {
    authenticated: true,
    hostKeyVerified: true,
    capabilities: ["read", "entries", "write", "mkdir", "delete", "copy", "move", "revision-check", "atomic-move"],
    async lstat(path) {
      const value = nodes.get(path);
      if (!value) throw Object.assign(new Error("missing"), { code: "ENOENT" });
      return { ...value, size: value.size ?? (value.bytes?.byteLength ?? null), mtimeMs: 1000, id: path };
    },
    async readFile(path) {
      const value = nodes.get(path);
      if (!value) throw Object.assign(new Error("missing"), { code: "ENOENT" });
      return value.bytes ?? new Uint8Array();
    },
    async readdir(path) {
      const prefix = `${path}/`;
      return [...nodes.keys()]
        .filter(value => value.startsWith(prefix) && !value.slice(prefix.length).includes("/"))
        .map(value => value.slice(prefix.length));
    },
    async writeFile(path, bytes) {
      nodes.set(path, { type: "file", size: bytes.byteLength, revision: `${path}-written`, bytes: new Uint8Array(bytes) });
    },
    async mkdir(path) { nodes.set(path, { type: "directory", revision: `${path}-mkdir` }); },
    async unlink(path) { nodes.delete(path); },
    async rmdir(path) { nodes.delete(path); },
    async copyFile(source, target) {
      const value = nodes.get(source);
      nodes.set(target, { ...value, revision: `${target}-copy`, bytes: new Uint8Array(value.bytes ?? []) });
    },
    async rename(source, target) {
      const value = nodes.get(source);
      nodes.delete(source);
      nodes.set(target, { ...value, revision: `${target}-move` });
    },
    async close() { client.closed = true; }
  };
  return client;
}

function context(id = `call-${Math.random()}`) {
  return { call: id, signal: new AbortController().signal };
}

function call(host, method, args, receiver = context()) {
  const handler = host.hostCalls[`filesystem.sftp/${method}`];
  assert.equal(typeof handler, "function");
  return handler.call(receiver, ...args);
}

test("SFTP host requires trusted transport, confines root, and preserves exact bytes", async () => {
  const client = makeClient();
  let factoryOptions;
  const host = createSftpHost({
    root: "/srv/data",
    credentialRef: "fixture-credential",
    hostKeyPolicy: { type: "pinned", fingerprints: ["sha256:fixture"] },
    connectionFactory: async options => { factoryOptions = options; return client; }
  });
  const opened = plain(await call(host, "open", [{}]));
  assert.equal(opened.descriptor.kind, "sftp");
  assert.equal(opened.descriptor.extensions["provider/host-key-verified?"], true);
  assert.equal(factoryOptions.credentialRef, "fixture-credential");
  assert.equal(JSON.stringify(opened).includes("/srv/data"), false);
  assert.deepEqual([...await call(host, "request", [opened.mount, "read", ["/docs/a.txt"]])], [0, 255, 1]);
  const page = plain(await call(host, "request", [opened.mount, "entries-page", ["/docs", { limit: 1 }]]));
  assert.deepEqual(page.entries.map(item => item.name), ["a.txt"]);
  assert.match(page["next-token"], /^sftp-page-/);
  await assert.rejects(call(host, "request", [opened.mount, "read", ["/docs/link"]]), /file\/unsupported/);
  await call(host, "close", [opened.mount]);
  assert.equal(client.closed, true);
});

test("SFTP negotiated capabilities gate mutations and unverified clients fail closed", async () => {
  const readOnlyHost = createSftpHost({
    root: "/srv/data", credentialRef: "fixture", hostKeyPolicy: { type: "pinned", fingerprints: ["key"] },
    connectionFactory: async () => ({ authenticated: true, hostKeyVerified: true, capabilities: ["read", "entries"], lstat: async () => ({ type: "directory" }), close: async () => {} })
  });
  const opened = plain(await call(readOnlyHost, "open", [{}]));
  await assert.rejects(call(readOnlyHost, "request", [opened.mount, "write", ["/x", Uint8Array.of(1), {}, {}]]), /file\/unsupported/);
  await call(readOnlyHost, "close", [opened.mount]);
  const unverified = createSftpHost({
    root: "/srv/data", credentialRef: "fixture", hostKeyPolicy: { type: "known-hosts", reference: "trusted-hosts" },
    connectionFactory: async () => ({ authenticated: true, hostKeyVerified: false })
  });
  await assert.rejects(call(unverified, "open", [{}]), /file\/authentication-failed/);
});

test("SFTP rejects unsafe logical and remote roots", () => {
  assert.throws(() => normalisePath("../escape"), /file\/outside-root/);
  assert.throws(() => createSftpHost({ root: "relative", credentialRef: "fixture", hostKeyPolicy: { type: "pinned", fingerprints: ["key"] }, connectionFactory: async () => ({}) }), /file\/descriptor-invalid/);
});
