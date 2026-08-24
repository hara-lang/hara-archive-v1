import assert from "node:assert/strict";
import test from "node:test";
import {
  createWebdavFetchHost,
  createWebdavProvider,
  normaliseLogicalPath
} from "./index.mjs";

const encoder = new TextEncoder();

function davResponse(entries) {
  const body = entries.map(item => `
    <d:response>
      <d:href>${item.href}</d:href>
      <d:propstat>
        <d:prop>
          <d:resourcetype>${item.type === "directory" ? "<d:collection/>" : ""}</d:resourcetype>
          ${item.size === undefined ? "" : `<d:getcontentlength>${item.size}</d:getcontentlength>`}
          <d:getlastmodified>Mon, 24 Aug 2026 00:00:00 GMT</d:getlastmodified>
          ${item.revision ? `<d:getetag>${item.revision}</d:getetag>` : ""}
        </d:prop>
        <d:status>HTTP/1.1 200 OK</d:status>
      </d:propstat>
    </d:response>`).join("");
  return `<?xml version="1.0"?><d:multistatus xmlns:d="DAV:">${body}</d:multistatus>`;
}

function response(body, status = 200, headers = {}) {
  return new Response(body, { status, headers });
}

function createFixture({ maliciousHref = false, hangingRead = false } = {}) {
  const requests = [];
  let readAborted = false;
  const fetch = async (url, init) => {
    requests.push({ url: String(url), init });
    const target = new URL(url);
    const depth = new Headers(init.headers).get("Depth");
    if (init.method === "PROPFIND" && target.pathname === "/dav/" && depth === "0") {
      return response(davResponse([
        { href: maliciousHref ? "https://evil.example/escape" : "/dav/", type: "directory", revision: '"root-1"' }
      ]), 207, { "Content-Type": "application/xml" });
    }
    if (init.method === "PROPFIND" && target.pathname === "/dav/docs" && depth === "0") {
      return response(davResponse([
        { href: "/dav/docs", type: "directory", revision: '"docs-1"' }
      ]), 207);
    }
    if (init.method === "PROPFIND" && target.pathname === "/dav/docs" && depth === "1") {
      return response(davResponse([
        { href: "/dav/docs", type: "directory", revision: '"docs-1"' },
        { href: "/dav/docs/zeta.txt", type: "file", size: 4, revision: '"z-1"' },
        { href: "/dav/docs/alpha.txt", type: "file", size: 3, revision: '"a-1"' }
      ]), 207);
    }
    if (init.method === "PROPFIND" && target.pathname === "/dav/docs/alpha.txt") {
      return response(davResponse([
        { href: "/dav/docs/alpha.txt", type: "file", size: 3, revision: '"a-1"' }
      ]), 207);
    }
    if (init.method === "GET" && target.pathname === "/dav/docs/alpha.txt") {
      if (hangingRead) {
        return await new Promise((resolve, reject) => {
          init.signal.addEventListener("abort", () => {
            readAborted = true;
            reject(new DOMException("aborted", "AbortError"));
          }, { once: true });
        });
      }
      return response(Uint8Array.of(0, 255, 1), 200, { ETag: '"a-1"' });
    }
    if (init.method === "PUT") return response(null, 204, { ETag: '"written-2"' });
    if (init.method === "MOVE") return response(null, 204, { ETag: '"moved-2"' });
    if (init.method === "COPY") return response(null, 201, { ETag: '"copied-2"' });
    if (init.method === "MKCOL") return response(null, 201, { ETag: '"dir-2"' });
    if (init.method === "DELETE") return response(null, 204);
    return response("missing", 404);
  };
  return { fetch, requests, readAborted: () => readAborted };
}

function providerContext(host, signal = new AbortController().signal) {
  return {
    signal,
    hostCall(service, method, args) {
      const handler = host.hostCalls[`${service}/${method}`];
      assert.equal(typeof handler, "function", `${service}/${method} is registered`);
      return handler(...args);
    }
  };
}

async function openFixture(fixture, options = {}) {
  const host = createWebdavFetchHost({
    rootUrl: "https://dav.example.test/dav/",
    fetch: fixture.fetch,
    headers: { Authorization: "Bearer secret" },
    capabilities: ["read", "entries", "write", "mkdir", "delete", "copy", "move", "revision-check"],
    ...options
  });
  const provider = createWebdavProvider();
  const context = providerContext(host);
  const opened = await provider.call("browser", "open", [{ display: "Documents" }], context);
  return { host, provider, context, opened };
}

test("logical paths match the canonical mounted-filesystem normalization", () => {
  assert.equal(normaliseLogicalPath("/a//b/./c"), "/a/b/c");
  assert.equal(normaliseLogicalPath("a/b/../c"), "/a/c");
  assert.throws(() => normaliseLogicalPath("../../escape"), /file\/outside-root/);
  assert.throws(() => normaliseLogicalPath("C:\\secret"), /file\/invalid-path/);
});

test("WebDAV HTA provider opens through host authority and redacts root and credentials", async () => {
  const fixture = createFixture();
  const { host, provider, context, opened } = await openFixture(fixture);
  assert.equal(opened.descriptor.kind, "webdav");
  assert.equal(opened.descriptor.display, "Documents");
  assert.equal(opened.descriptor.extensions["provider/route"], "hta-wasm");
  assert.equal(JSON.stringify(opened).includes("dav.example.test"), false);
  assert.equal(JSON.stringify(opened).includes("secret"), false);
  assert.equal(new Headers(fixture.requests[0].init.headers).get("Authorization"), "Bearer secret");
  await provider.call("browser", "close", [opened.id], context);
  await host.closeAll();
});

test("stat, exact-byte read, and stable paged entries use the same provider state", async () => {
  const fixture = createFixture();
  const { provider, context, opened } = await openFixture(fixture);
  const stat = await provider.call("browser", "stat", [opened.id, "/docs/alpha.txt"], context);
  assert.deepEqual(
    { path: stat.path, name: stat.name, type: stat.type, size: stat.size, revision: stat.revision },
    { path: "/docs/alpha.txt", name: "alpha.txt", type: "file", size: 3, revision: '"a-1"' }
  );
  assert.deepEqual(
    [...await provider.call("browser", "read", [opened.id, "/docs/alpha.txt"], context)],
    [0, 255, 1]
  );
  const first = await provider.call("browser", "entries-page", [opened.id, "/docs", { limit: 1 }], context);
  assert.deepEqual(first.entries.map(item => item.name), ["alpha.txt"]);
  assert.match(first["next-token"], /^webdav-page-/);
  const second = await provider.call(
    "browser",
    "entries-page",
    [opened.id, "/docs", { limit: 1, token: first["next-token"] }],
    context
  );
  assert.deepEqual(second.entries.map(item => item.name), ["zeta.txt"]);
  assert.equal(second["next-token"], null);
});

test("write and move preserve revision fencing and safe DAV headers", async () => {
  const fixture = createFixture();
  const { provider, context, opened } = await openFixture(fixture);
  const created = await provider.call(
    "browser",
    "write",
    [opened.id, "/docs/new.bin", Uint8Array.of(4, 5), { mode: "create", parents: false }, {}],
    context
  );
  assert.equal(created.revision, '"written-2"');
  const put = fixture.requests.find(item => item.init.method === "PUT");
  assert.equal(new Headers(put.init.headers).get("If-None-Match"), "*");
  assert.equal(put.url, "https://dav.example.test/dav/docs/new.bin");

  await assert.rejects(
    provider.call(
      "browser",
      "move",
      [opened.id, "/docs/alpha.txt", "/docs/alpha.txt", {}, { "expected-revision": '"stale"' }],
      context
    ),
    /file\/conflict/
  );
  assert.equal(fixture.requests.some(item => item.init.method === "MOVE"), false);
});

test("mounted-root mutations and same-path copy are rejected before transport", async () => {
  const fixture = createFixture();
  const { provider, context, opened } = await openFixture(fixture);
  const before = fixture.requests.length;
  await assert.rejects(
    provider.call("browser", "delete", [opened.id, "/", {}, {}], context),
    /file\/denied/
  );
  await assert.rejects(
    provider.call("browser", "move", [opened.id, "/", "/docs/root", {}, {}], context),
    /file\/denied/
  );
  await assert.rejects(
    provider.call("browser", "copy", [opened.id, "/docs/alpha.txt", "/docs/alpha.txt", {}, {}], context),
    /file\/already-exists/
  );
  assert.equal(fixture.requests.length, before);
});

test("revision mutations fail when the trusted transport does not advertise fencing", async () => {
  const fixture = createFixture();
  const { provider, context, opened } = await openFixture(fixture, {
    capabilities: ["read", "entries", "move"]
  });
  await assert.rejects(
    provider.call(
      "browser",
      "move",
      [opened.id, "/docs/alpha.txt", "/docs/alpha.txt", {}, { "expected-revision": '"a-1"' }],
      context
    ),
    /revision checks are unavailable/
  );
});

test("failed provider activation rolls back the host mount", async () => {
  const closed = [];
  const host = {
    hostCalls: {
      "filesystem.webdav/open": async () => ({
        mount: "host-mount-1",
        "read-only": false,
        capabilities: ["not-a-capability"],
        "root-entry": {
          path: "/",
          type: "directory",
          size: null,
          "modified-at": null,
          revision: '"root"',
          extensions: {}
        }
      }),
      "filesystem.webdav/close": async mount => { closed.push(mount); }
    }
  };
  const provider = createWebdavProvider();
  await assert.rejects(
    provider.call("browser", "open", [{}], providerContext(host)),
    /unknown WebDAV capability/
  );
  assert.deepEqual(closed, ["host-mount-1"]);
});

test("top-level cancellation reaches the host fetch AbortSignal", async () => {
  const fixture = createFixture({ hangingRead: true });
  const { provider, host, opened } = await openFixture(fixture);
  const controller = new AbortController();
  const context = providerContext(host, controller.signal);
  const pending = provider.call("browser", "read", [opened.id, "/docs/alpha.txt"], context);
  await new Promise(resolve => setTimeout(resolve, 0));
  controller.abort();
  await assert.rejects(pending, /file\/cancelled/);
  assert.equal(fixture.readAborted(), true);
});

test("trusted-host teardown aborts requests and removes mount state", async () => {
  const fixture = createFixture({ hangingRead: true });
  const { provider, host, context, opened } = await openFixture(fixture);
  const pending = provider.call("browser", "read", [opened.id, "/docs/alpha.txt"], context);
  await new Promise(resolve => setTimeout(resolve, 0));
  await host.closeAll();
  await assert.rejects(pending, /file\/cancelled/);
  assert.equal(fixture.readAborted(), true);
});

test("host rejects escaped DAV hrefs before provider activation", async () => {
  const fixture = createFixture({ maliciousHref: true });
  const host = createWebdavFetchHost({
    rootUrl: "https://dav.example.test/dav/",
    fetch: fixture.fetch
  });
  const provider = createWebdavProvider();
  await assert.rejects(
    provider.call("browser", "open", [{}], providerContext(host)),
    /file\/outside-root/
  );
});

test("explicit close is idempotent and post-close calls fail", async () => {
  const fixture = createFixture();
  const { provider, context, opened } = await openFixture(fixture);
  await provider.call("browser", "close", [opened.id], context);
  await provider.call("browser", "close", [opened.id], context);
  await assert.rejects(
    provider.call("browser", "stat", [opened.id, "/docs"], context),
    /file\/provider-closed/
  );
});

test("trusted host configuration rejects credential-bearing or insecure roots", () => {
  assert.throws(
    () => createWebdavFetchHost({ rootUrl: "https://user:secret@dav.example.test/dav/", fetch: async () => {} }),
    /file\/descriptor-invalid/
  );
  assert.throws(
    () => createWebdavFetchHost({ rootUrl: "http://dav.example.test/dav/", fetch: async () => {} }),
    /requires HTTPS/
  );
  assert.throws(
    () => createWebdavFetchHost({ rootUrl: "https://dav.example.test/%zz/", fetch: async () => {} }),
    /invalid escaping/
  );
});

test("trusted host binds mount teardown to the owning HTA context", async () => {
  const fixture = createFixture({ hangingRead: true });
  let originalCloses = 0;
  const kernelContext = {
    async close() {
      originalCloses += 1;
    }
  };
  const host = createWebdavFetchHost({
    rootUrl: "https://dav.example.test/dav/",
    fetch: fixture.fetch,
    capabilities: ["read", "entries"]
  });
  const provider = createWebdavProvider();
  const context = {
    signal: new AbortController().signal,
    hostCall(service, method, args) {
      const handler = host.hostCalls[`${service}/${method}`];
      return handler.call({ kernelContext }, ...args);
    }
  };
  const opened = await provider.call("browser", "open", [{}], context);
  const pending = provider.call("browser", "read", [opened.id, "/docs/alpha.txt"], context);
  await new Promise(resolve => setTimeout(resolve, 0));

  const first = kernelContext.close();
  const second = kernelContext.close();
  await assert.rejects(pending, /file\/cancelled/);
  await Promise.all([first, second]);

  assert.equal(originalCloses, 1);
  assert.equal(fixture.readAborted(), true);
  await assert.rejects(
    provider.call("browser", "stat", [opened.id, "/docs"], context),
    /file\/provider-closed/
  );
});
