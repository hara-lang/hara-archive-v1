const SERVICE = "filesystem.s3";
const DEFAULT_ENDPOINT = "https://s3.amazonaws.com/";
const DEFAULT_MAX_TRANSFER_BYTES = 64 * 1024 * 1024;
const DEFAULT_PAGE_LIMIT = 256;
const MAX_PAGE_LIMIT = 1_000;
const MAX_XML_BYTES = 2 * 1024 * 1024;
const MUTATING_CAPABILITIES = new Set(["write", "delete", "copy", "move"]);
const KNOWN_CAPABILITIES = new Set([
  "read", "write", "entries", "delete", "copy", "move", "revision-check"
]);

function fail(code, message, data = undefined, retryable = false) {
  const error = new Error(`${code}: ${message}`);
  error.code = code;
  error.data = data;
  error.retryable = retryable;
  throw error;
}

function valueName(value) {
  return value && typeof value === "object" && typeof value.name === "string"
    ? value.name
    : value;
}

function plain(value) {
  if (value instanceof Uint8Array || value === null || value === undefined) return value;
  if (Array.isArray(value)) return value.map(plain);
  if (value instanceof Set) return [...value].map(plain);
  if (value instanceof Map) {
    const output = Object.create(null);
    for (const [key, item] of value) output[String(valueName(key))] = plain(item);
    return output;
  }
  if (typeof value.name === "string" && Object.keys(value).every(key => key === "name")) {
    return value.name;
  }
  return value;
}

function wire(value) {
  if (value === null || value === undefined || value instanceof Uint8Array) return value ?? null;
  if (Array.isArray(value)) return value.map(wire);
  if (value instanceof Map) return new Map([...value].map(([key, item]) => [key, wire(item)]));
  if (value && typeof value === "object") {
    return new Map(Object.entries(value).map(([key, item]) => [key, wire(item)]));
  }
  return value;
}

function objectOptions(value, allowed, label) {
  const options = plain(value ?? new Map());
  if (!options || typeof options !== "object" || Array.isArray(options)) {
    fail("file/descriptor-invalid", `${label} must be a map`);
  }
  for (const key of Object.keys(options)) {
    if (!allowed.has(key)) fail("file/descriptor-invalid", `unknown ${label} option ${key}`);
  }
  return options;
}

function booleanOption(options, key, fallback = false) {
  const value = options[key];
  if (value === undefined) return fallback;
  if (typeof value !== "boolean") fail("file/descriptor-invalid", `${key} must be a boolean`);
  return value;
}

function positiveInteger(options, key, fallback, maximum = Number.MAX_SAFE_INTEGER) {
  const value = options[key];
  if (value === undefined) return fallback;
  if (!Number.isSafeInteger(value) || value <= 0 || value > maximum) {
    fail("file/descriptor-invalid", `${key} must be a positive bounded integer`);
  }
  return value;
}

function textOption(options, key, fallback) {
  const value = options[key];
  if (value === undefined) return fallback;
  if (typeof value !== "string" || !value.trim()) {
    fail("file/descriptor-invalid", `${key} must be a nonblank string`);
  }
  return value;
}

function normalisePath(input) {
  if (typeof input !== "string") fail("file/invalid-path", "logical path must be a string");
  if (input === "" || input === "/") return "/";
  if (input.includes("\0") || input.includes("\\") || input.includes(":")) {
    fail("file/invalid-path", "S3 logical paths contain no host syntax");
  }
  const value = input.startsWith("/") ? input.slice(1) : input;
  const segments = value.split("/");
  const output = [];
  for (const segment of segments) {
    if (!segment || segment === ".") fail("file/invalid-path", "S3 logical path contains an empty or dot segment");
    if (segment === "..") fail("file/outside-root", "S3 logical path escapes the mounted prefix");
    output.push(segment);
  }
  return `/${output.join("/")}`;
}

function logicalParent(path) {
  if (path === "/") return null;
  const index = path.lastIndexOf("/");
  return index === 0 ? "/" : path.slice(0, index);
}

function directChild(parent, path) {
  if (path === parent) return false;
  if (parent === "/") return path.startsWith("/") && !path.slice(1).includes("/");
  return path.startsWith(`${parent}/`) && !path.slice(parent.length + 1).includes("/");
}

function capabilitySet(values, readOnly) {
  if (!Array.isArray(values)) fail("file/descriptor-invalid", "S3 host capabilities must be a vector");
  const output = new Set();
  for (const raw of values) {
    const capability = String(valueName(raw));
    if (!KNOWN_CAPABILITIES.has(capability)) fail("file/descriptor-invalid", `unknown S3 capability ${capability}`);
    if (!readOnly || !MUTATING_CAPABILITIES.has(capability)) output.add(capability);
  }
  output.add("read");
  output.add("entries");
  return output;
}

function requireCapability(state, capability, operation, path) {
  if (state.closed) fail("file/provider-closed", "S3 filesystem is closed", { operation, path });
  if (!state.capabilities.has(capability)) {
    fail("file/unsupported", `S3 capability ${capability} is unavailable`, { operation, path });
  }
  if (state.readOnly && MUTATING_CAPABILITIES.has(capability)) {
    fail("file/permission-denied", "S3 filesystem is read-only", { operation, path });
  }
}

function mutationContext(value) {
  const mutation = plain(value ?? new Map());
  if (!mutation || typeof mutation !== "object" || Array.isArray(mutation)) {
    fail("file/descriptor-invalid", "mutation context must be a map");
  }
  return {
    expectedRevision: mutation["expected-revision"] ?? mutation.expected_revision ?? null,
    expectedTargetRevision:
      mutation["expected-target-revision"] ?? mutation.expected_target_revision ?? null
  };
}

function revisionMatches(actual, expected, path, target = false) {
  if (expected === null || expected === undefined) return;
  if (typeof expected !== "string" || !expected.length) {
    fail("file/descriptor-invalid", "expected revision must be a nonblank string");
  }
  if (actual !== expected) {
    fail("file/conflict", "S3 revision mismatch", {
      path, target, expected, actual: actual ?? null, reason: "revision-mismatch"
    }, true);
  }
}

function normaliseHostError(error, operation, path) {
  if (error?.code?.startsWith?.("file/")) throw error;
  if (error?.name === "AbortError" || /abort|cancel/i.test(String(error?.message ?? error))) {
    fail("file/cancelled", "S3 operation was cancelled", { operation, path });
  }
  fail("file/io", "S3 host transport failed", { operation, path }, true);
}

function decodeXml(value) {
  return String(value ?? "")
    .replaceAll("&lt;", "<")
    .replaceAll("&gt;", ">")
    .replaceAll("&quot;", '"')
    .replaceAll("&apos;", "'")
    .replaceAll("&amp;", "&");
}

function xmlValue(source, tag) {
  const match = new RegExp(`<${tag}(?:\\s[^>]*)?>([\\s\\S]*?)</${tag}>`).exec(source);
  return match ? decodeXml(match[1]) : null;
}

function xmlBlocks(source, tag) {
  const result = [];
  const expression = new RegExp(`<${tag}(?:\\s[^>]*)?>([\\s\\S]*?)</${tag}>`, "g");
  let match;
  while ((match = expression.exec(source))) result.push(match[1]);
  return result;
}

function parseRevision(headers, xml = "") {
  const etag = headers.get("etag") ?? xmlValue(xml, "ETag");
  const version = headers.get("x-amz-version-id") ?? xmlValue(xml, "VersionId");
  if (!etag && !version) return null;
  return version ? `version:${version}` : `etag:${etag}`;
}

function entry(path, type, options = {}) {
  const name = path === "/" ? "/" : path.slice(path.lastIndexOf("/") + 1);
  return {
    path,
    name,
    type,
    size: type === "directory" ? null : options.size ?? null,
    "modified-at": options.modifiedAt ?? null,
    id: options.id ?? null,
    revision: options.revision ?? null,
    capabilities: null,
    extensions: options.extensions ?? {}
  };
}

function statusFailure(status, operation, path) {
  const data = { operation, path, status };
  if (status === 401) fail("file/authentication-failed", "S3 authentication failed", data);
  if (status === 403) fail("file/permission-denied", "S3 operation is forbidden", data);
  if (status === 404) fail("file/not-found", "S3 object was not found", data);
  if (status === 409 || status === 412) fail("file/conflict", "S3 revision or object conflict", data);
  if (status === 429) fail("file/rate-limited", "S3 request was rate limited", data, true);
  if (status === 507) fail("file/quota-exceeded", "S3 storage quota was exceeded", data);
  if (status >= 500) fail("file/io", `S3 service failed with status ${status}`, data, true);
  fail("file/io", `unexpected S3 response status ${status}`, data);
}

function requireSuccess(response, operation, path, accepted = undefined) {
  const status = Number(response?.status);
  if (!Number.isInteger(status)) fail("file/io", "S3 host returned no HTTP status");
  const allowed = accepted ?? (value => value >= 200 && value < 300);
  if (!allowed(status)) statusFailure(status, operation, path);
  return response;
}

async function responseBytes(response, maxBytes, operation, path) {
  const length = Number(response.headers?.get?.("content-length"));
  if (Number.isSafeInteger(length) && length > maxBytes) {
    fail("file/quota-exceeded", "S3 response exceeds the configured transfer limit", { operation, path });
  }
  const value = new Uint8Array(await response.arrayBuffer());
  if (value.byteLength > maxBytes) {
    fail("file/quota-exceeded", "S3 response exceeds the configured transfer limit", { operation, path });
  }
  return value;
}

async function responseText(response, operation, path) {
  const bytes = await responseBytes(response, MAX_XML_BYTES, operation, path);
  return new TextDecoder("utf-8", { fatal: false }).decode(bytes);
}

function endpointUrl(value) {
  let url;
  try {
    url = new URL(value ?? DEFAULT_ENDPOINT);
  } catch {
    fail("file/descriptor-invalid", "S3 endpoint must be an absolute URL");
  }
  const loopback = new Set(["127.0.0.1", "localhost", "[::1]"]).has(url.hostname);
  if (url.protocol !== "https:" && !(loopback && url.protocol === "http:")) {
    fail("file/descriptor-invalid", "S3 transport requires HTTPS outside loopback fixtures");
  }
  if (url.username || url.password || url.search || url.hash) {
    fail("file/descriptor-invalid", "S3 endpoint cannot contain credentials, query, or fragment");
  }
  url.pathname = `${url.pathname.replace(/\/+$/, "")}/`;
  return url;
}

function bucketName(value) {
  if (typeof value !== "string" || !/^[a-z0-9][a-z0-9.-]{1,61}[a-z0-9]$/.test(value)) {
    fail("file/descriptor-invalid", "S3 bucket name is malformed");
  }
  return value;
}

function prefixName(value) {
  if (value === undefined || value === null || value === "") return "";
  if (typeof value !== "string" || value.includes("\0") || value.includes("\\") || value.startsWith("/")) {
    fail("file/descriptor-invalid", "S3 prefix is malformed");
  }
  const pieces = value.split("/");
  if (pieces.some(piece => !piece || piece === "." || piece === ".." || piece.includes(":"))) {
    fail("file/descriptor-invalid", "S3 prefix contains an unsafe segment");
  }
  return `${value.replace(/\/+$/, "")}${value ? "/" : ""}`;
}

function base64Url(value) {
  const bytes = new TextEncoder().encode(value);
  let binary = "";
  for (const byte of bytes) binary += String.fromCharCode(byte);
  return btoa(binary).replaceAll("+", "-").replaceAll("/", "_").replace(/=+$/, "");
}

function createS3Host(configuration = {}) {
  const endpoint = endpointUrl(configuration.endpoint);
  const bucket = bucketName(configuration.bucket);
  const prefix = prefixName(configuration.prefix);
  const fetcher = configuration.fetch ?? globalThis.fetch?.bind(globalThis);
  if (typeof fetcher !== "function") fail("file/host-unavailable", "S3 host requires fetch");
  const signRequest = configuration.signRequest;
  if (signRequest !== undefined && typeof signRequest !== "function") {
    fail("file/descriptor-invalid", "S3 signRequest must be a function");
  }
  const configuredCapabilities = configuration.capabilities ?? ["read", "entries"];
  const mounts = new Map();
  let nextMount = 0;

  function mount(id) {
    const state = mounts.get(String(id));
    if (!state || state.closed) fail("file/provider-closed", "unknown or closed S3 filesystem", { id });
    return state;
  }

  function objectKey(state, path) {
    const logical = normalisePath(path);
    return `${state.prefix}${logical === "/" ? "" : logical.slice(1)}`;
  }

  function objectUrl(state, path, query = undefined) {
    const url = new URL(endpoint);
    url.pathname = `${url.pathname.replace(/\/+$/, "")}/${encodeURIComponent(state.bucket)}/${objectKey(state, path)
      .split("/").map(encodeURIComponent).join("/")}`;
    if (query) for (const [key, value] of Object.entries(query)) if (value !== null && value !== undefined) url.searchParams.set(key, value);
    return url;
  }

  function pageUrl(state, path, request) {
    const url = objectUrl(state, "/");
    url.search = "";
    url.searchParams.set("list-type", "2");
    url.searchParams.set("delimiter", "/");
    url.searchParams.set("prefix", objectKey(state, path) + (path === "/" ? "" : "/"));
    url.searchParams.set("max-keys", String(request.limit));
    if (request.upstreamToken) url.searchParams.set("continuation-token", request.upstreamToken);
    return url;
  }

  function requestId(receiver) {
    return String(receiver?.call ?? receiver?.task ?? `s3-${Date.now()}-${Math.random()}`);
  }

  async function withPending(state, receiver, operation, path, action) {
    const id = requestId(receiver);
    const controller = new AbortController();
    const signal = receiver?.signal;
    const abort = () => controller.abort(signal?.reason ?? new Error("cancelled"));
    if (signal?.aborted) abort();
    signal?.addEventListener?.("abort", abort, { once: true });
    state.pending.set(id, { id, operation, path, controller });
    try {
      return await action(controller.signal, id);
    } catch (error) {
      normaliseHostError(error, operation, path);
    } finally {
      state.pending.delete(id);
      signal?.removeEventListener?.("abort", abort);
    }
  }

  async function http(state, operation, path, init, signal) {
    const url = init.url ?? objectUrl(state, path);
    const headers = new Headers(init.headers ?? {});
    headers.set("Accept", headers.get("Accept") ?? "application/octet-stream");
    let request = { ...init, headers, signal };
    delete request.url;
    if (signRequest) {
      const signed = await signRequest({ url: String(url), method: request.method ?? "GET", headers, body: request.body }, { signal });
      if (signed && typeof signed === "object") request = { ...request, ...signed, headers: new Headers(signed.headers ?? headers), signal };
    }
    try {
      return await fetcher(url, request);
    } catch (error) {
      normaliseHostError(error, operation, path);
    }
  }

  async function head(state, path, receiver) {
    return withPending(state, receiver, "stat", path, async (signal) => {
      const response = await http(state, "stat", path, { method: "HEAD" }, signal);
      if (Number(response.status) === 404) return null;
      requireSuccess(response, "stat", path);
      const size = Number(response.headers.get("content-length"));
      const modified = response.headers.get("last-modified");
      return entry(path, "file", {
        size: Number.isSafeInteger(size) && size >= 0 ? size : null,
        modifiedAt: modified ? Date.parse(modified) : null,
        id: objectKey(state, path),
        revision: parseRevision(response.headers)
      });
    });
  }

  async function list(state, path, request, receiver) {
    return withPending(state, receiver, "entries-page", path, async (signal) => {
      const response = await http(state, "entries-page", path, { method: "GET", url: pageUrl(state, path, request) }, signal);
      requireSuccess(response, "entries-page", path);
      const xml = await responseText(response, "entries-page", path);
      const values = [];
      for (const block of xmlBlocks(xml, "Contents")) {
        const key = xmlValue(block, "Key");
        if (!key) continue;
        const logical = key === state.prefix ? "/" : `/${key.slice(state.prefix.length).replace(/\/+$/, "")}`;
        if (!logical || logical === path) continue;
        if (!directChild(path, logical)) continue;
        const size = Number(xmlValue(block, "Size"));
        values.push(entry(logical, key.endsWith("/") ? "directory" : "file", {
          size: Number.isSafeInteger(size) && size >= 0 ? size : null,
          modifiedAt: Date.parse(xmlValue(block, "LastModified") ?? "") || null,
          id: key,
          revision: xmlValue(block, "VersionId")
            ? `version:${xmlValue(block, "VersionId")}`
            : xmlValue(block, "ETag") ? `etag:${xmlValue(block, "ETag")}` : null
        }));
      }
      for (const block of xmlBlocks(xml, "CommonPrefixes")) {
        const key = xmlValue(block, "Prefix");
        if (!key) continue;
        const relative = key.slice(state.prefix.length).replace(/\/+$/, "");
        if (!relative) continue;
        const logical = `/${relative}`;
        if (directChild(path, logical)) values.push(entry(logical, "directory", { id: key }));
      }
      const unique = new Map(values.map(value => [value.path, value]));
      const sorted = [...unique.values()].sort((left, right) => left.path.localeCompare(right.path));
      const next = xmlValue(xml, "NextContinuationToken");
      return { values: sorted, next };
    });
  }

  async function stat(state, path, receiver) {
    const logical = normalisePath(path);
    requireCapability(state, "read", "stat", logical);
    if (logical === "/") return entry("/", "directory", { id: `${state.bucket}/${state.prefix}` });
    const object = await head(state, logical, receiver);
    if (object) return object;
    const listed = await list(state, logical, { limit: 1, upstreamToken: null }, receiver);
    if (listed.values.length) return entry(logical, "directory", { id: objectKey(state, logical) });
    fail("file/not-found", "S3 path does not exist", { path: logical });
  }

  async function read(state, path, receiver) {
    const logical = normalisePath(path);
    requireCapability(state, "read", "read", logical);
    const metadata = await stat(state, logical, receiver);
    if (metadata.type === "directory") fail("file/is-directory", "cannot read an S3 directory", { path: logical });
    return withPending(state, receiver, "read", logical, async (signal) => {
      const response = await http(state, "read", logical, { method: "GET" }, signal);
      requireSuccess(response, "read", logical);
      return responseBytes(response, state.maxTransferBytes, "read", logical);
    });
  }

  async function write(state, path, bytesValue, optionsValue, mutationValue, receiver) {
    const logical = normalisePath(path);
    requireCapability(state, "write", "write", logical);
    const bytes = bytesValue instanceof Uint8Array ? bytesValue : null;
    if (!bytes) fail("file/descriptor-invalid", "S3 write requires exact bytes", { path: logical });
    if (bytes.byteLength > state.maxTransferBytes) fail("file/quota-exceeded", "S3 write exceeds the configured transfer limit", { path: logical });
    const options = objectOptions(optionsValue, new Set(["mode", "parents"]), "write");
    const mode = String(valueName(options.mode ?? "create"));
    if (!["create", "replace", "append"].includes(mode)) fail("file/descriptor-invalid", `unknown S3 write mode ${mode}`);
    if (mode === "append") fail("file/unsupported", "S3 append is not advertised");
    const mutation = mutationContext(mutationValue);
    const existing = await head(state, logical, receiver);
    revisionMatches(existing?.revision ?? null, mutation.expectedRevision, logical);
    if (mode === "create" && existing) fail("file/already-exists", "S3 object already exists", { path: logical });
    if (mode === "replace" && !existing) fail("file/not-found", "S3 object does not exist", { path: logical });
    return withPending(state, receiver, "write", logical, async (signal) => {
      const headers = new Headers({ "Content-Type": "application/octet-stream" });
      if (mutation.expectedRevision?.startsWith("etag:")) headers.set("If-Match", mutation.expectedRevision.slice(5));
      if (mode === "create") headers.set("If-None-Match", "*");
      const response = await http(state, "write", logical, { method: "PUT", headers, body: bytes }, signal);
      requireSuccess(response, "write", logical);
      return wire({ path: logical, revision: parseRevision(response.headers), "mount-revision": null, extensions: {} });
    });
  }

  async function entriesPage(state, path, requestValue, receiver) {
    const logical = normalisePath(path);
    requireCapability(state, "entries", "entries-page", logical);
    const request = objectOptions(requestValue, new Set(["limit", "token"]), "entries page request");
    const limit = positiveInteger(request, "limit", DEFAULT_PAGE_LIMIT, MAX_PAGE_LIMIT);
    let upstreamToken = null;
    if (request.token !== undefined && request.token !== null) {
      const saved = state.pageTokens.get(String(request.token));
      if (!saved || saved.path !== logical) fail("file/invalid-page-token", "unknown S3 page token", { path: logical });
      upstreamToken = saved.upstreamToken;
      state.pageTokens.delete(String(request.token));
    }
    const page = await list(state, logical, { limit, upstreamToken }, receiver);
    const values = page.values.slice(0, limit);
    let nextToken = null;
    if (page.next) {
      nextToken = `s3-page-${++state.pageSequence}-${base64Url(page.next).slice(0, 20)}`;
      state.pageTokens.set(nextToken, { path: logical, upstreamToken: page.next });
    }
    return wire({ entries: values, "next-token": nextToken });
  }

  async function mkdir(state, path) {
    normalisePath(path);
    requireCapability(state, "write", "mkdir", path);
    fail("file/unsupported", "S3 has no material directory operation");
  }

  async function deleteEntry(state, path, optionsValue, mutationValue, receiver) {
    const logical = normalisePath(path);
    requireCapability(state, "delete", "delete", logical);
    if (logical === "/") fail("file/denied", "cannot delete the mounted S3 root");
    const options = objectOptions(optionsValue, new Set(["missing-ok"]), "delete");
    const mutation = mutationContext(mutationValue);
    const existing = await head(state, logical, receiver);
    if (!existing) {
      const directory = await list(state, logical, { limit: 1, upstreamToken: null }, receiver);
      if (directory.values.length) fail("file/unsupported", "recursive S3 directory deletion is not advertised", { path: logical });
      if (mutation.expectedRevision) revisionMatches(null, mutation.expectedRevision, logical);
      if (booleanOption(options, "missing-ok", false)) return wire({ path: logical, revision: null, "mount-revision": null, extensions: {} });
      fail("file/not-found", "S3 object was not found", { path: logical });
    }
    revisionMatches(existing.revision, mutation.expectedRevision, logical);
    return withPending(state, receiver, "delete", logical, async (signal) => {
      const headers = new Headers();
      if (mutation.expectedRevision?.startsWith("etag:")) headers.set("If-Match", mutation.expectedRevision.slice(5));
      const response = await http(state, "delete", logical, { method: "DELETE", headers }, signal);
      if (Number(response.status) === 404 && booleanOption(options, "missing-ok", false)) return wire({ path: logical, revision: null, "mount-revision": null, extensions: {} });
      requireSuccess(response, "delete", logical);
      return wire({ path: logical, revision: null, "mount-revision": null, extensions: {} });
    });
  }

  async function copyEntry(state, source, target, optionsValue, mutationValue, receiver) {
    const sourcePath = normalisePath(source);
    const targetPath = normalisePath(target);
    requireCapability(state, "copy", "copy", sourcePath);
    if (sourcePath === targetPath) fail("file/already-exists", "S3 copy source and target are identical");
    const options = objectOptions(optionsValue, new Set(["replace", "parents", "preserve-modified"]), "copy");
    if (booleanOption(options, "preserve-modified", false)) fail("file/unsupported", "S3 modified-time preservation is not advertised");
    const mutation = mutationContext(mutationValue);
    const sourceEntry = await stat(state, sourcePath, receiver);
    if (sourceEntry.type !== "file") fail("file/unsupported", "S3 server-side copy only supports regular objects");
    revisionMatches(sourceEntry.revision, mutation.expectedRevision, sourcePath);
    const targetEntry = await head(state, targetPath, receiver);
    revisionMatches(targetEntry?.revision ?? null, mutation.expectedTargetRevision, targetPath, true);
    if (targetEntry && !booleanOption(options, "replace", false)) fail("file/already-exists", "S3 copy target already exists");
    return withPending(state, receiver, "copy", sourcePath, async (signal) => {
      const headers = new Headers({
        "x-amz-copy-source": `/${state.bucket}/${objectKey(state, sourcePath)}`,
        "x-amz-metadata-directive": "COPY"
      });
      const response = await http(state, "copy", targetPath, { method: "PUT", headers }, signal);
      requireSuccess(response, "copy", sourcePath);
      const revision = parseRevision(response.headers);
      return wire({ path: targetPath, revision, "mount-revision": null, extensions: {} });
    });
  }

  async function moveEntry(state, source, target, optionsValue, mutationValue, receiver) {
    const sourcePath = normalisePath(source);
    const targetPath = normalisePath(target);
    requireCapability(state, "move", "move", sourcePath);
    if (sourcePath === "/" || targetPath === "/") fail("file/denied", "cannot move the mounted S3 root");
    const options = objectOptions(optionsValue, new Set(["replace", "parents", "atomic"]), "move");
    if (booleanOption(options, "atomic", false)) fail("file/unsupported", "S3 move is not atomic");
    if (sourcePath === targetPath) {
      const current = await stat(state, sourcePath, receiver);
      const mutation = mutationContext(mutationValue);
      revisionMatches(current.revision, mutation.expectedRevision, sourcePath);
      revisionMatches(current.revision, mutation.expectedTargetRevision, targetPath, true);
      return wire({ path: targetPath, revision: current.revision, "mount-revision": null, extensions: {} });
    }
    const mutation = mutationContext(mutationValue);
    const copied = await copyEntry(
      state,
      sourcePath,
      targetPath,
      { replace: options.replace, parents: options.parents },
      mutation,
      receiver
    );
    try {
      await deleteEntry(state, sourcePath, {}, { "expected-revision": mutation.expectedRevision }, receiver);
    } catch (error) {
      if (error?.code?.startsWith?.("file/")) {
        error.retryable = true;
        error.data = { ...(error.data ?? {}), reason: "copy-completed-delete-pending", source: sourcePath, target: targetPath };
      }
      throw error;
    }
    return copied;
  }

  async function open(optionsValue, receiver) {
    const options = objectOptions(optionsValue, new Set(["display", "read-only?", "operation-timeout-ms", "max-transfer-bytes"]), "S3 filesystem");
    const readOnly = booleanOption(options, "read-only?", false);
    const state = {
      id: `s3-host-${++nextMount}`,
      bucket,
      prefix,
      readOnly,
      capabilities: capabilitySet(configuredCapabilities, readOnly),
      maxTransferBytes: positiveInteger(options, "max-transfer-bytes", DEFAULT_MAX_TRANSFER_BYTES, 256 * 1024 * 1024),
      display: textOption(options, "display", `S3 ${bucket}`),
      pending: new Map(),
      pageTokens: new Map(),
      pageSequence: 0,
      closed: false
    };
    mounts.set(state.id, state);
    try {
      const descriptor = {
        kind: "s3",
        display: state.display,
        "read-only": readOnly,
        capabilities: [...state.capabilities].sort(),
        revision: null,
        extensions: {
          "provider/root-scoped?": true,
          "provider/transport-verified?": true,
          "provider/route": "hta-wasm",
          "provider/object-model": "bucket-prefix"
        }
      };
      return wire({ mount: state.id, descriptor });
    } catch (error) {
      mounts.delete(state.id);
      throw error;
    }
  }

  async function request(mountId, operationValue, argsValue, receiver) {
    const state = mount(mountId);
    const operation = String(valueName(operationValue));
    const args = Array.isArray(argsValue) ? argsValue : [];
    switch (operation) {
      case "stat": return stat(state, args[0], receiver);
      case "read": return read(state, args[0], receiver);
      case "write": return write(state, args[0], args[1], args[2], args[3], receiver);
      case "entries-page": return entriesPage(state, args[0], args[1], receiver);
      case "mkdir": return mkdir(state, args[0], args[1], args[2], receiver);
      case "delete": return deleteEntry(state, args[0], args[1], args[2], receiver);
      case "copy": return copyEntry(state, args[0], args[1], args[2], args[3], receiver);
      case "move": return moveEntry(state, args[0], args[1], args[2], args[3], receiver);
      default: fail("file/unsupported", `unsupported S3 filesystem operation ${operation}`);
    }
  }

  async function cancel(mountId, id) {
    const state = mounts.get(String(mountId));
    if (!state) return false;
    const pending = state.pending.get(String(id));
    if (!pending) return false;
    pending.controller.abort(new Error("cancelled"));
    return true;
  }

  async function close(mountId) {
    const state = mounts.get(String(mountId));
    if (!state) return null;
    state.closed = true;
    for (const pending of state.pending.values()) pending.controller.abort(new Error("closed"));
    state.pending.clear();
    state.pageTokens.clear();
    mounts.delete(String(mountId));
    return null;
  }

  async function closeAll() {
    await Promise.all([...mounts.keys()].map(id => close(id)));
  }

  async function describe() {
    return wire({
      provider: "s3",
      identity: "hara/filesystem-s3",
      abi: "hta.v1",
      route: "hta-wasm",
      capabilities: [...KNOWN_CAPABILITIES].sort()
    });
  }

  return Object.freeze({
    hostCalls: Object.freeze({
      [`${SERVICE}/describe`]: describe,
      [`${SERVICE}/open`]: function (...args) { return open(...args, this); },
      [`${SERVICE}/request`]: function (...args) { return request(...args, this); },
      [`${SERVICE}/cancel`]: function (...args) { return cancel(...args, this); },
      [`${SERVICE}/close`]: function (...args) { return close(...args, this); }
    }),
    closeAll
  });
}

export { createS3Host, normalisePath, plain };
