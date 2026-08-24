const SERVICE = "filesystem.webdav";
const DEFAULT_MAX_TRANSFER_BYTES = 16 * 1024 * 1024;
const DEFAULT_TIMEOUT_MS = 30_000;
const DEFAULT_PAGE_LIMIT = 256;
const MAX_PAGE_LIMIT = 1_000;
const MAX_DAV_ENTRIES = 10_000;
const MUTATING_CAPABILITIES = new Set(["write", "mkdir", "delete", "copy", "move"]);
const KNOWN_CAPABILITIES = new Set([
  "read",
  "write",
  "entries",
  "mkdir",
  "delete",
  "copy",
  "move",
  "revision-check"
]);
const SAFE_REQUEST_HEADERS = new Set([
  "content-type",
  "depth",
  "if-match",
  "if-none-match",
  "overwrite"
]);
const SAFE_RESPONSE_HEADERS = [
  "allow",
  "content-length",
  "content-type",
  "dav",
  "etag",
  "last-modified"
];

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
    const result = Object.create(null);
    for (const [key, item] of value) result[String(valueName(key))] = plain(item);
    return result;
  }
  const named = valueName(value);
  return named === value ? value : named;
}

function wire(value) {
  if (value instanceof Uint8Array || value === null || value === undefined) return value ?? null;
  if (Array.isArray(value)) return value.map(wire);
  if (value instanceof Map) return new Map([...value].map(([key, item]) => [key, wire(item)]));
  if (value && typeof value === "object") {
    return new Map(Object.entries(value).map(([key, item]) => [key, wire(item)]));
  }
  return value;
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

function normaliseLogicalPath(input) {
  if (typeof input !== "string") fail("file/invalid-path", "logical path must be a string");
  if (input.includes("\0")) fail("file/invalid-path", "logical path contains NUL");
  if (input.includes("\\")) fail("file/invalid-path", "logical paths use '/' separators");
  const segments = [];
  for (const segment of input.split("/")) {
    if (!segment || segment === ".") continue;
    if (segment === "..") {
      if (!segments.length) fail("file/outside-root", "logical path escapes the mounted root");
      segments.pop();
      continue;
    }
    if (/^[A-Za-z]:/.test(segment)) {
      fail("file/invalid-path", "logical paths do not accept host drive prefixes");
    }
    segments.push(segment);
  }
  return segments.length ? `/${segments.join("/")}` : "/";
}

function logicalParent(path) {
  const value = normaliseLogicalPath(path);
  if (value === "/") return null;
  const index = value.lastIndexOf("/");
  return index === 0 ? "/" : value.slice(0, index);
}

function directChild(parent, path) {
  if (path === parent) return false;
  if (parent === "/") return path.startsWith("/") && !path.slice(1).includes("/");
  return path.startsWith(`${parent}/`) && !path.slice(parent.length + 1).includes("/");
}

function safeOptions(value, allowed, label) {
  const options = plain(value ?? new Map());
  if (!options || typeof options !== "object" || Array.isArray(options)) {
    fail("file/descriptor-invalid", `${label} must be a map`);
  }
  for (const key of Object.keys(options)) {
    if (!allowed.has(key)) fail("file/descriptor-invalid", `unknown ${label} option ${key}`);
  }
  return options;
}

function mutationContext(value) {
  const mutation = plain(value ?? new Map());
  return {
    expectedRevision: mutation?.["expected-revision"] ?? mutation?.expected_revision ?? null,
    expectedTargetRevision:
      mutation?.["expected-target-revision"] ?? mutation?.expected_target_revision ?? null
  };
}

function mutationRequired(mutation) {
  return (
    mutation.expectedRevision !== null
    && mutation.expectedRevision !== undefined
  ) || (
    mutation.expectedTargetRevision !== null
    && mutation.expectedTargetRevision !== undefined
  );
}

function requestHeaders(response) {
  return plain(response?.headers ?? new Map());
}

function statusFailure(status, operation, path) {
  const data = { operation, path, status };
  if (status === 401) fail("file/authentication-failed", "WebDAV authentication failed", data);
  if (status === 403) fail("file/permission-denied", "WebDAV operation is forbidden", data);
  if (status === 404) fail("file/not-found", "WebDAV entry was not found", data);
  if (status === 405 || status === 501) fail("file/unsupported", "WebDAV operation is unsupported", data);
  if (status === 409 || status === 412) fail("file/conflict", "WebDAV revision or hierarchy conflict", data);
  if (status === 413 || status === 507) fail("file/quota-exceeded", "WebDAV transfer or storage quota exceeded", data);
  if (status === 423) fail("file/locked", "WebDAV entry is locked", data);
  if (status === 429) fail("file/rate-limited", "WebDAV request was rate limited", data, true);
  if (status >= 500) fail("file/io", `WebDAV server failed with status ${status}`, data, true);
  fail("file/io", `unexpected WebDAV response status ${status}`, data);
}

function requireSuccess(response, operation, path, accepted = undefined) {
  const status = Number(response?.status);
  if (!Number.isInteger(status)) fail("file/io", "WebDAV host returned no HTTP status");
  const allowed = accepted ?? (value => value >= 200 && value < 300);
  if (!allowed(status)) statusFailure(status, operation, path);
  return response;
}

function normaliseHostError(error, operation, path) {
  if (error?.code?.startsWith?.("file/")) throw error;
  const message = String(error?.message ?? error);
  const match = /\b(file\/[a-z0-9-]+):/.exec(message);
  if (match) fail(match[1], message.slice(match.index + match[0].length).trim(), { operation, path });
  if (error?.name === "AbortError" || /abort|cancel/i.test(message)) {
    fail("file/cancelled", "WebDAV operation was cancelled", { operation, path });
  }
  fail("file/io", "WebDAV host transport failed", { operation, path }, true);
}

function entry(value) {
  const source = plain(value);
  const path = normaliseLogicalPath(source?.path);
  const type = String(valueName(source?.type));
  if (!new Set(["file", "directory", "symlink", "other"]).has(type)) {
    fail("file/io", "WebDAV host returned an invalid entry type", { path, type });
  }
  const name = path === "/" ? "/" : path.slice(path.lastIndexOf("/") + 1);
  const size = source?.size === null || source?.size === undefined ? null : Number(source.size);
  if (size !== null && (!Number.isSafeInteger(size) || size < 0)) {
    fail("file/io", "WebDAV host returned an invalid entry size", { path });
  }
  const modifiedAt = source?.["modified-at"] ?? source?.modified_at ?? null;
  if (modifiedAt !== null && (!Number.isSafeInteger(modifiedAt) || modifiedAt < 0)) {
    fail("file/io", "WebDAV host returned an invalid modified time", { path });
  }
  const revision = source?.revision ?? null;
  if (revision !== null && (typeof revision !== "string" || !revision.length)) {
    fail("file/io", "WebDAV host returned an invalid revision", { path });
  }
  return Object.freeze({
    path,
    name,
    type,
    size: type === "directory" ? null : size,
    "modified-at": modifiedAt,
    id: source?.id ?? null,
    revision,
    capabilities: source?.capabilities ?? null,
    extensions: Object.freeze({ ...(source?.extensions ?? {}) })
  });
}

function revisionMatches(actual, expected, path, target = false) {
  if (expected === null || expected === undefined) return;
  if (typeof expected !== "string" || !expected.length) {
    fail("file/descriptor-invalid", "expected revision must be a nonblank string");
  }
  if (actual !== expected) {
    fail("file/conflict", "WebDAV revision mismatch", {
      path,
      target,
      expected,
      actual: actual ?? null,
      reason: "revision-mismatch"
    });
  }
}

function capabilitySet(values, readOnly) {
  if (!Array.isArray(values)) fail("file/descriptor-invalid", "host capabilities must be a vector");
  const result = new Set();
  for (const raw of values) {
    const value = String(valueName(raw));
    if (!KNOWN_CAPABILITIES.has(value)) {
      fail("file/descriptor-invalid", `unknown WebDAV capability ${value}`);
    }
    if (!readOnly || !MUTATING_CAPABILITIES.has(value)) result.add(value);
  }
  result.add("read");
  result.add("entries");
  return result;
}

function requireCapability(state, capability, operation, path) {
  if (state.closed) fail("file/provider-closed", "WebDAV filesystem is closed", { operation, path });
  if (!state.capabilities.has(capability)) {
    fail("file/unsupported", `WebDAV capability ${capability} is unavailable`, { operation, path });
  }
  if (state.readOnly && MUTATING_CAPABILITIES.has(capability)) {
    fail("file/permission-denied", "WebDAV filesystem is read-only", { operation, path });
  }
}

export {
  SERVICE,
  DEFAULT_MAX_TRANSFER_BYTES,
  DEFAULT_TIMEOUT_MS,
  DEFAULT_PAGE_LIMIT,
  MAX_PAGE_LIMIT,
  MAX_DAV_ENTRIES,
  KNOWN_CAPABILITIES,
  SAFE_REQUEST_HEADERS,
  SAFE_RESPONSE_HEADERS,
  fail,
  valueName,
  plain,
  wire,
  booleanOption,
  positiveInteger,
  textOption,
  normaliseLogicalPath,
  logicalParent,
  directChild,
  safeOptions,
  mutationContext,
  mutationRequired,
  requestHeaders,
  statusFailure,
  requireSuccess,
  normaliseHostError,
  entry,
  revisionMatches,
  capabilitySet,
  requireCapability
};
