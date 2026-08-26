const SERVICE = "filesystem.github";
const DEFAULT_TIMEOUT_MS = 30_000;
const DEFAULT_MAX_TRANSFER_BYTES = 64 * 1024 * 1024;
const DEFAULT_PAGE_LIMIT = 256;
const MAX_PAGE_LIMIT = 1_000;
const MAX_TREE_ENTRIES = 100_000;
const MAX_TREE_DEPTH = 256;
const MUTATING_CAPABILITIES = new Set(["write", "delete", "copy", "move"]);
const KNOWN_CAPABILITIES = new Set([
  "read",
  "write",
  "entries",
  "delete",
  "copy",
  "move",
  "revision-check"
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

function positiveInteger(options, key, fallback, maximum = Number.MAX_SAFE_INTEGER) {
  const value = options[key];
  if (value === undefined) return fallback;
  if (!Number.isSafeInteger(value) || value <= 0 || value > maximum) {
    fail("file/descriptor-invalid", `${key} must be a positive bounded integer`);
  }
  return value;
}

function booleanOption(options, key, fallback = false) {
  const value = options[key];
  if (value === undefined) return fallback;
  if (typeof value !== "boolean") fail("file/descriptor-invalid", `${key} must be a boolean`);
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
  if (input === "") return "/";
  if (input.includes("\0") || input.includes("\\") || input.includes(":")) {
    fail("file/invalid-path", "logical path contains an unsafe character");
  }
  if (input.includes("//") || input === "." || input.startsWith("./") || input.endsWith("/.")) {
    fail("file/invalid-path", "logical path contains an unsafe segment");
  }
  if (
    input === ".."
    || input.startsWith("../")
    || input.endsWith("/..")
    || input.includes("/../")
  ) {
    fail("file/outside-root", "logical path escapes the mounted root");
  }
  const value = input.startsWith("/") ? input : `/${input}`;
  if (value.length > 1 && value.endsWith("/")) {
    fail("file/invalid-path", "logical paths cannot end with a separator");
  }
  return value;
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
    fail("file/conflict", "GitHub entry revision does not match", {
      path,
      target,
      expected,
      actual: actual ?? null,
      reason: "revision-mismatch"
    }, true);
  }
}

function capabilitySet(values, readOnly) {
  if (!Array.isArray(values)) fail("file/descriptor-invalid", "host capabilities must be a vector");
  const result = new Set();
  for (const raw of values) {
    const value = String(valueName(raw));
    if (!KNOWN_CAPABILITIES.has(value)) fail("file/descriptor-invalid", `unknown GitHub capability ${value}`);
    if (!readOnly || !MUTATING_CAPABILITIES.has(value)) result.add(value);
  }
  result.add("read");
  result.add("entries");
  return result;
}

function requireCapability(state, capability, operation, path) {
  if (state.closed) fail("file/provider-closed", "GitHub filesystem is closed", { operation, path });
  if (!state.capabilities.has(capability)) {
    fail("file/unsupported", `GitHub capability ${capability} is unavailable`, { operation, path });
  }
  if (state.readOnly && MUTATING_CAPABILITIES.has(capability)) {
    fail("file/permission-denied", "GitHub revision mount is read-only", { operation, path });
  }
}

function normaliseHostError(error, operation, path) {
  if (error?.code?.startsWith?.("file/")) throw error;
  const message = String(error?.message ?? error);
  const match = /\b(file\/[a-z0-9-]+):/.exec(message);
  if (match) fail(match[1], message.slice(match.index + match[0].length).trim(), { operation, path });
  if (error?.name === "AbortError" || /abort|cancel/i.test(message)) {
    fail("file/cancelled", "GitHub operation was cancelled", { operation, path });
  }
  fail("file/io", "GitHub host transport failed", { operation, path }, true);
}

function encodeCursor(revision, offset) {
  const bytes = new TextEncoder().encode(`${revision}:${offset}`);
  let binary = "";
  for (const byte of bytes) binary += String.fromCharCode(byte);
  return btoa(binary).replaceAll("+", "-").replaceAll("/", "_").replace(/=+$/, "");
}

function decodeCursor(token, revision) {
  if (token === null || token === undefined) return 0;
  if (typeof token !== "string" || !token.length) {
    fail("file/invalid-page-token", "invalid GitHub filesystem page token");
  }
  try {
    const normalized = token.replaceAll("-", "+").replaceAll("_", "/")
      + "=".repeat((4 - token.length % 4) % 4);
    const binary = atob(normalized);
    const bytes = Uint8Array.from(binary, character => character.charCodeAt(0));
    const decoded = new TextDecoder("utf-8", { fatal: true }).decode(bytes);
    const separator = decoded.lastIndexOf(":");
    if (separator <= 0) throw new Error("malformed");
    if (decoded.slice(0, separator) !== revision) {
      fail("file/conflict", "GitHub page token belongs to another revision", {
        reason: "stale-page-token"
      }, true);
    }
    const offset = Number(decoded.slice(separator + 1));
    if (!Number.isSafeInteger(offset) || offset < 0) throw new Error("malformed");
    return offset;
  } catch (error) {
    if (error?.code) throw error;
    fail("file/invalid-page-token", "invalid GitHub filesystem page token");
  }
}

export {
  SERVICE,
  DEFAULT_TIMEOUT_MS,
  DEFAULT_MAX_TRANSFER_BYTES,
  DEFAULT_PAGE_LIMIT,
  MAX_PAGE_LIMIT,
  MAX_TREE_ENTRIES,
  MAX_TREE_DEPTH,
  MUTATING_CAPABILITIES,
  KNOWN_CAPABILITIES,
  fail,
  valueName,
  plain,
  wire,
  positiveInteger,
  booleanOption,
  textOption,
  normaliseLogicalPath,
  logicalParent,
  directChild,
  safeOptions,
  mutationContext,
  revisionMatches,
  capabilitySet,
  requireCapability,
  normaliseHostError,
  encodeCursor,
  decodeCursor
};
