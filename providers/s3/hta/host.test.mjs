import assert from "node:assert/strict";
import test from "node:test";
import { createS3Host, plain, normalisePath } from "./host.mjs";

function response(body = null, status = 200, headers = {}) {
  return new Response(body, { status, headers });
}

const listXml = `<?xml version="1.0"?>
<ListBucketResult>
  <Contents><Key>root/docs/a.txt</Key><Size>3</Size><ETag>\"a-1\"</ETag><LastModified>2026-08-24T00:00:00.000Z</LastModified></Contents>
  <CommonPrefixes><Prefix>root/docs/images/</Prefix></CommonPrefixes>
  <IsTruncated>true</IsTruncated><NextContinuationToken>next-1</NextContinuationToken>
</ListBucketResult>`;

function makeFixture() {
  const requests = [];
  const fetch = async (url, init) => {
    const parsed = new URL(url);
    requests.push({ url: String(url), init });
    if (init.method === "HEAD" && parsed.pathname.endsWith("/root/docs/a.txt")) {
      return response(null, 200, {
        ETag: '"a-1"',
        "Content-Length": "3",
        "Last-Modified": "Mon, 24 Aug 2026 00:00:00 GMT"
      });
    }
    if (init.method === "HEAD" && parsed.pathname.endsWith("/root/docs/missing.txt")) return response(null, 404);
    if (init.method === "GET" && parsed.pathname.endsWith("/root/docs/a.txt")) {
      return response(Uint8Array.of(0, 255, 1), 200, { ETag: '"a-1"' });
    }
    if (init.method === "GET" && parsed.searchParams.get("list-type") === "2") {
      return response(listXml, 200, { "Content-Type": "application/xml" });
    }
    if (init.method === "PUT" && init.headers.get("x-amz-copy-source")) {
      return response("<CopyObjectResult><ETag>\"copy-1\"</ETag></CopyObjectResult>", 200);
    }
    if (init.method === "PUT") return response(null, 200, { ETag: '"write-1"' });
    if (init.method === "DELETE") return response(null, 204);
    return response(null, 404);
  };
  return { fetch, requests };
}

function context() {
  return { signal: new AbortController().signal, call: `call-${Math.random()}` };
}

function call(host, method, args, receiver = context()) {
  const handler = host.hostCalls[`filesystem.s3/${method}`];
  assert.equal(typeof handler, "function");
  return handler.call(receiver, ...args);
}

test("S3 host confines paths, keeps authority out of descriptors, and returns exact bytes", async () => {
  const fixture = makeFixture();
  const host = createS3Host({
    endpoint: "https://s3.example.test/",
    bucket: "fixture-bucket",
    prefix: "root",
    fetch: fixture.fetch,
    signRequest({ headers }) {
      headers.set("authorization", "Bearer secret");
      return { headers };
    },
    capabilities: ["read", "entries", "write", "delete", "copy", "move", "revision-check"]
  });
  const opened = plain(await call(host, "open", [{ display: "Fixture" }]));
  assert.equal(opened.descriptor.kind, "s3");
  assert.equal(JSON.stringify(opened).includes("secret"), false);
  assert.equal(JSON.stringify(opened).includes("s3.example.test"), false);
  assert.deepEqual([...await call(host, "request", [opened.mount, "read", ["/docs/a.txt"]])], [0, 255, 1]);
  const metadata = plain(await call(host, "request", [opened.mount, "stat", ["/docs/a.txt"]]));
  assert.deepEqual({ path: metadata.path, size: metadata.size, revision: metadata.revision }, {
    path: "/docs/a.txt", size: 3, revision: "etag:\"a-1\""
  });
  assert.equal(fixture.requests.at(-1).init.headers.get("authorization"), "Bearer secret");
  await call(host, "close", [opened.mount]);
});

test("S3 virtual-directory pages are opaque, revision-fenced, and copy/move is guarded", async () => {
  const fixture = makeFixture();
  const host = createS3Host({
    endpoint: "https://s3.example.test/",
    bucket: "fixture-bucket",
    prefix: "root",
    fetch: fixture.fetch,
    capabilities: ["read", "entries", "write", "delete", "copy", "move", "revision-check"]
  });
  const opened = plain(await call(host, "open", [{}]));
  const page = plain(await call(host, "request", [opened.mount, "entries-page", ["/docs", { limit: 1 }]]));
  assert.deepEqual(page.entries.map(value => value.name), ["a.txt"]);
  assert.match(page["next-token"], /^s3-page-/);
  await assert.rejects(
    call(host, "request", [opened.mount, "write", ["/docs/a.txt", Uint8Array.of(1), { mode: "replace" }, { "expected-revision": "etag:\"stale\"" }]]),
    /file\/conflict/
  );
  const copied = plain(await call(host, "request", [opened.mount, "copy", ["/docs/a.txt", "/docs/b.txt", { replace: true }, {}]]));
  assert.equal(copied.path, "/docs/b.txt");
  await assert.rejects(
    call(host, "request", [opened.mount, "mkdir", ["/docs/new", {}, {}]]),
    /file\/unsupported/
  );
  await call(host, "close", [opened.mount]);
});

test("S3 rejects host syntax and embedded credentials", () => {
  assert.throws(() => normalisePath("../escape"), /file\/outside-root/);
  assert.throws(() => createS3Host({ bucket: "bad bucket", fetch: async () => response() }), /file\/descriptor-invalid/);
  assert.throws(() => createS3Host({ endpoint: "https://user:secret@s3.example.test/", bucket: "valid-bucket", fetch: async () => response() }), /file\/descriptor-invalid/);
});
