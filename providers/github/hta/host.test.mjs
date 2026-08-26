import assert from "node:assert/strict";
import test from "node:test";
import { createGithubFetchHost } from "./host.mjs";

const COMMIT = "a".repeat(40);
const ROOT_TREE = "b".repeat(40);
const SOURCE_TREE = "c".repeat(40);
const README_BLOB = "d".repeat(40);
const SOURCE_BLOB = "e".repeat(40);
const LINK_BLOB = "f".repeat(40);
const CREATED_BLOB = "1".repeat(40);
const CREATED_TREE = "2".repeat(40);
const CREATED_COMMIT = "3".repeat(40);
const COMPETING_COMMIT = "4".repeat(40);

function mapEntries(value) {
  return value instanceof Map ? Object.fromEntries(value) : value;
}

function response(status, value, headers = {}) {
  const body = value instanceof Uint8Array || typeof value === "string"
    ? value
    : JSON.stringify(value);
  return new Response(body, { status, headers: { "content-type": "application/json", ...headers } });
}

function fixture(options = {}) {
  let head = COMMIT;
  let moved = false;
  let refReads = 0;
  let delayedRead = false;
  const requests = [];
  const fetch = async (input, init = {}) => {
    const url = new URL(input);
    const method = init.method ?? "GET";
    requests.push({ method, path: url.pathname + url.search, init });
    if (delayedRead && url.pathname.endsWith(`/git/blobs/${README_BLOB}`)) {
      return new Promise((resolve, reject) => {
        const abort = () => {
          const error = new Error("aborted");
          error.name = "AbortError";
          reject(error);
        };
        init.signal?.addEventListener("abort", abort, { once: true });
        setTimeout(() => {
          init.signal?.removeEventListener("abort", abort);
          resolve(response(200, new TextEncoder().encode("hello"), { "content-type": "application/octet-stream" }));
        }, 100);
      });
    }
    const prefix = "/repos/hara-lang/hara";
    const path = url.pathname;
    if (method === "GET" && path === `${prefix}/git/ref/heads/main`) {
      refReads += 1;
      if (moved && refReads > 2) head = COMPETING_COMMIT;
      return response(200, { object: { type: "commit", sha: head } });
    }
    if (method === "GET" && path === `${prefix}/git/commits/${COMMIT}`) {
      return response(200, { sha: COMMIT, tree: { sha: ROOT_TREE } });
    }
    if (method === "GET" && path === `${prefix}/git/commits/${COMPETING_COMMIT}`) {
      return response(200, { sha: COMPETING_COMMIT, tree: { sha: ROOT_TREE } });
    }
    if (method === "GET" && path === `${prefix}/git/trees/${ROOT_TREE}`) {
      return response(200, {
        sha: ROOT_TREE,
        truncated: false,
        tree: [
          { path: "README.md", mode: "100644", type: "blob", sha: README_BLOB, size: 5 },
          { path: "link", mode: "120000", type: "blob", sha: LINK_BLOB, size: 9 },
          { path: "src", mode: "040000", type: "tree", sha: SOURCE_TREE },
          { path: "src/main.hal", mode: "100644", type: "blob", sha: SOURCE_BLOB, size: 7 },
          { path: "vendor", mode: "160000", type: "commit", sha: COMPETING_COMMIT }
        ]
      });
    }
    if (method === "GET" && path === `${prefix}/git/blobs/${README_BLOB}`) {
      return response(200, new TextEncoder().encode("hello"), { "content-type": "application/octet-stream" });
    }
    if (method === "GET" && path === `${prefix}/git/blobs/${SOURCE_BLOB}`) {
      return response(200, new TextEncoder().encode("(+ 1 2)"), { "content-type": "application/octet-stream" });
    }
    if (method === "GET" && path === `${prefix}/git/blobs/${LINK_BLOB}`) {
      return response(200, new TextEncoder().encode("README.md"), { "content-type": "application/octet-stream" });
    }
    if (method === "POST" && path === `${prefix}/git/blobs`) {
      assert.equal(JSON.parse(init.body).encoding, "base64");
      return response(201, { sha: CREATED_BLOB });
    }
    if (method === "POST" && path === `${prefix}/git/trees`) {
      const body = JSON.parse(init.body);
      assert.equal(body.base_tree, ROOT_TREE);
      return response(201, { sha: CREATED_TREE });
    }
    if (method === "POST" && path === `${prefix}/git/commits`) {
      const body = JSON.parse(init.body);
      assert.deepEqual(body.parents, [COMMIT]);
      return response(201, { sha: CREATED_COMMIT });
    }
    if (method === "PATCH" && path === `${prefix}/git/refs/heads/main`) {
      if (moved) {
        head = COMPETING_COMMIT;
        return response(409, { message: "reference moved" });
      }
      const body = JSON.parse(init.body);
      assert.equal(body.force, false);
      head = body.sha;
      return response(200, { object: { sha: head } });
    }
    if (options.rateLimited) return response(403, { message: `rate limit ${options.token ?? "secret"}` });
    return response(404, { message: "not found" });
  };
  return {
    fetch,
    requests,
    setMoved(value) { moved = value; },
    setDelayedRead(value) { delayedRead = value; },
    head() { return head; }
  };
}

function receiver(task, signal = new AbortController().signal, kernelContext = null) {
  return { task, signal, kernelContext };
}

test("trusted GitHub host opens a root-scoped mount and preserves Git object semantics", async () => {
  const fixtureState = fixture();
  const host = createGithubFetchHost({
    repository: "hara-lang/hara",
    ref: "heads/main",
    token: "secret-token",
    fetch: fixtureState.fetch
  });
  const open = host.hostCalls[`${"filesystem.github"}/open`];
  const request = host.hostCalls[`${"filesystem.github"}/request`];
  const close = host.hostCalls[`${"filesystem.github"}/close`];
  const opened = await open.call(receiver(1), new Map([["root", "/src"], ["mode", "read-only"]]));
  const openedMap = mapEntries(opened);
  const descriptor = mapEntries(openedMap.descriptor);
  assert.equal(openedMap.id.startsWith("github-host-"), true);
  assert.equal(descriptor.kind, "github");
  assert.equal(descriptor.revision, COMMIT);
  assert.equal(JSON.stringify(opened).includes("secret-token"), false);

  const stat = mapEntries(await request.call(receiver(2), openedMap.id, "stat", ["/main.hal"]));
  assert.equal(stat.path, "/main.hal");
  assert.equal(stat.type, "file");
  assert.equal(stat.revision, SOURCE_BLOB);
  assert.deepEqual(await request.call(receiver(3), openedMap.id, "read", ["/main.hal"]), new TextEncoder().encode("(+ 1 2)"));
  await assert.rejects(
    request.call(receiver(4), openedMap.id, "read", ["/README.md"]),
    error => error.code === "file/not-found"
  );
  const rootOpened = mapEntries(await open.call(receiver(5), new Map()));
  await assert.rejects(
    request.call(receiver(6), rootOpened.id, "read", ["/link"]),
    error => error.code === "file/unsupported"
  );
  const page = mapEntries(await request.call(receiver(7), rootOpened.id, "entries-page", ["/", new Map([["limit", 1]])]));
  assert.equal(page.entries.length, 1);
  assert.equal(typeof page["next-token"], "string");
  const second = mapEntries(await request.call(receiver(8), rootOpened.id, "entries-page", ["/", new Map([["limit", 10], ["token", page["next-token"]]])]));
  assert.deepEqual(second.entries.map(value => mapEntries(value).path), ["/README.md", "/src", "/vendor"]);
  await close.call(receiver(9), openedMap.id);
  await close.call(receiver(10), rootOpened.id);
});

test("writable GitHub mounts use non-forced expected-head updates", async () => {
  const fixtureState = fixture();
  const host = createGithubFetchHost({
    repository: "hara-lang/hara",
    ref: "heads/main",
    token: "secret-token",
    fetch: fixtureState.fetch
  });
  const open = host.hostCalls["filesystem.github/open"];
  const request = host.hostCalls["filesystem.github/request"];
  const opened = mapEntries(await open.call(receiver(10), new Map([["mode", "commit"]])));
  const result = mapEntries(await request.call(receiver(11), opened.id, "write", [
    "/README.md",
    new TextEncoder().encode("updated"),
    new Map([["mode", "replace"]]),
    new Map([["expected-revision", README_BLOB]])
  ]));
  assert.equal(result.revision, CREATED_BLOB);
  assert.equal(result["mount-revision"], CREATED_COMMIT);
  assert.equal(fixtureState.head(), CREATED_COMMIT);
  const auth = fixtureState.requests.find(item => item.method === "POST" && item.path.endsWith("/git/blobs"));
  assert.equal(auth.init.headers.Authorization, "Bearer secret-token");

  const conflictFixture = fixture();
  conflictFixture.setMoved(true);
  const conflictHost = createGithubFetchHost({
    repository: "hara-lang/hara",
    ref: "heads/main",
    token: "secret-token",
    fetch: conflictFixture.fetch
  });
  const conflictOpen = conflictHost.hostCalls["filesystem.github/open"];
  const conflictRequest = conflictHost.hostCalls["filesystem.github/request"];
  const conflictMount = mapEntries(await conflictOpen.call(receiver(12), new Map([["mode", "commit"]])));
  await assert.rejects(
    conflictRequest.call(receiver(13), conflictMount.id, "write", [
      "/README.md", new TextEncoder().encode("conflict"), new Map([["mode", "replace"]]), new Map()
    ]),
    error => error.code === "file/conflict"
  );
  assert.equal(conflictFixture.requests.some(item => item.method === "PATCH"), false);
});

test("GitHub transport cancellation and context teardown abort pending work", async () => {
  const fixtureState = fixture();
  fixtureState.setDelayedRead(true);
  const host = createGithubFetchHost({
    repository: "hara-lang/hara",
    ref: "heads/main",
    fetch: fixtureState.fetch,
    operationTimeoutMs: 1_000
  });
  const kernelContext = { async close() {} };
  const opened = mapEntries(await host.hostCalls["filesystem.github/open"].call(receiver(20, undefined, kernelContext), new Map()));
  const controller = new AbortController();
  const pending = host.hostCalls["filesystem.github/request"].call(
    receiver(21, controller.signal, kernelContext),
    opened.id,
    "read",
    ["/README.md"]
  );
  await new Promise(resolve => setTimeout(resolve, 0));
  controller.abort();
  await assert.rejects(pending, error => error.code === "file/cancelled");

  const openedAgain = mapEntries(await host.hostCalls["filesystem.github/open"].call(receiver(22, undefined, kernelContext), new Map()));
  const pendingClose = host.hostCalls["filesystem.github/request"].call(
    receiver(23, new AbortController().signal, kernelContext),
    openedAgain.id,
    "read",
    ["/README.md"]
  );
  const closePromise = kernelContext.close();
  await assert.rejects(pendingClose, error => error.code === "file/cancelled" || error.code === "file/provider-closed");
  await closePromise;
});

test("host errors are stable and do not expose the configured credential", async () => {
  const state = fixture({ rateLimited: true, token: "secret-token" });
  const host = createGithubFetchHost({
    repository: "hara-lang/hara",
    ref: "heads/main",
    token: "secret-token",
    fetch: async (input, init) => {
      const url = new URL(input);
      if (url.pathname.endsWith("/git/ref/heads/main")) return response(403, { message: "rate limited secret-token" });
      return state.fetch(input, init);
    }
  });
  await assert.rejects(
    host.hostCalls["filesystem.github/open"].call(receiver(30), new Map()),
    error => error.code === "file/rate-limited" && !error.message.includes("secret-token")
  );
});
