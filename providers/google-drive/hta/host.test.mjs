import assert from "node:assert/strict";
import test from "node:test";
import { createGoogleDriveHost, normalisePath, plain } from "./host.mjs";

function json(value, status = 200) {
  return new Response(JSON.stringify(value), { status, headers: { "Content-Type": "application/json" } });
}

function makeFixture() {
  const requests = [];
  const root = { id: "root-id", name: "Root", mimeType: "application/vnd.google-apps.folder", headRevisionId: "root-r1", parents: [] };
  const docs = { id: "docs-id", name: "docs", mimeType: "application/vnd.google-apps.folder", headRevisionId: "docs-r1", parents: ["root-id"] };
  const file = { id: "file-id", name: "a.txt", mimeType: "text/plain", size: "3", headRevisionId: "file-r1", parents: ["docs-id"] };
  const shortcut = { id: "shortcut-id", name: "shortcut", mimeType: "application/vnd.google-apps.shortcut", headRevisionId: "shortcut-r1", parents: ["root-id"], shortcutDetails: { targetId: "file-id" } };
  const duplicateA = { id: "ambiguous-a", name: "ambig", mimeType: "text/plain", size: "1", parents: ["root-id"] };
  const duplicateB = { id: "ambiguous-b", name: "ambig", mimeType: "text/plain", size: "1", parents: ["root-id"] };
  const fetch = async (url, init) => {
    const parsed = new URL(url);
    const path = parsed.pathname;
    requests.push({ url: String(url), init });
    if (path.endsWith("/files/root-id")) return json(root);
    if (path.endsWith("/files/file-id") && parsed.searchParams.get("alt") === "media") return new Response(Uint8Array.of(0, 255, 1), { status: 200 });
    if (path.endsWith("/files/file-id")) return json(file);
    if (path.endsWith("/files/shortcut-id")) return json(shortcut);
    if (path.endsWith("/files") && parsed.searchParams.get("q")?.includes("'root-id' in parents")) {
      if (parsed.searchParams.get("q").includes("name = 'docs'")) return json({ files: [docs] });
      if (parsed.searchParams.get("q").includes("name = 'shortcut'")) return json({ files: [shortcut] });
      if (parsed.searchParams.get("q").includes("name = 'ambig'")) return json({ files: [duplicateA, duplicateB] });
      return json({ files: [docs, shortcut], nextPageToken: "drive-next" });
    }
    if (path.endsWith("/files") && parsed.searchParams.get("q")?.includes("'docs-id' in parents")) {
      return json({ files: [file] });
    }
    return json({ error: { errors: [{ reason: "notFound" }] } }, 404);
  };
  return { fetch, requests };
}

function context(id = `call-${Math.random()}`) {
  return { call: id, signal: new AbortController().signal };
}

function call(host, method, args, receiver = context()) {
  const handler = host.hostCalls[`filesystem.google-drive/${method}`];
  assert.equal(typeof handler, "function");
  return handler.call(receiver, ...args);
}

test("Google Drive uses stable IDs, exact bytes, and keeps token authority out of descriptors", async () => {
  const fixture = makeFixture();
  const host = createGoogleDriveHost({
    rootId: "root-id",
    fetch: fixture.fetch,
    tokenProvider: async () => "secret-token",
    capabilities: ["read", "entries"]
  });
  const opened = plain(await call(host, "open", [{ display: "Drive" }]));
  assert.equal(opened.descriptor.kind, "google-drive");
  assert.equal(JSON.stringify(opened).includes("secret-token"), false);
  assert.deepEqual([...await call(host, "request", [opened.mount, "read", ["/docs/a.txt"]])], [0, 255, 1]);
  const page = plain(await call(host, "request", [opened.mount, "entries-page", ["/", { limit: 1 }]]));
  assert.deepEqual(page.entries.map(item => item.name), ["docs"]);
  assert.match(page["next-token"], /^drive-page-/);
  assert.equal(fixture.requests.at(-1).init.headers.get("Authorization"), "Bearer secret-token");
  await call(host, "close", [opened.mount]);
});

test("Google Drive rejects duplicate names and does not follow shortcuts or Workspace documents", async () => {
  const fixture = makeFixture();
  const host = createGoogleDriveHost({ rootId: "root-id", fetch: fixture.fetch, tokenProvider: () => "token" });
  const opened = plain(await call(host, "open", [{}]));
  await assert.rejects(call(host, "request", [opened.mount, "stat", ["/ambig"]]), /file\/ambiguous-path/);
  await assert.rejects(call(host, "request", [opened.mount, "read", ["/shortcut"]]), /file\/unsupported/);
  await call(host, "close", [opened.mount]);
});

test("Google Drive rejects unsafe paths and missing trusted token providers", () => {
  assert.throws(() => normalisePath("../escape"), /file\/outside-root/);
  assert.throws(() => createGoogleDriveHost({ rootId: "root-id", fetch: async () => json({}) }), /tokenProvider/);
});
