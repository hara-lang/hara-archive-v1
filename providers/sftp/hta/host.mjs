import { posix } from "node:path";

const SERVICE = "filesystem.sftp";
const DEFAULT_MAX_TRANSFER_BYTES = 64 * 1024 * 1024;
const DEFAULT_PAGE_LIMIT = 256;
const MAX_PAGE_LIMIT = 1_000;
const MAX_ENTRIES = 100_000;
const MUTATING_CAPABILITIES = new Set(["write", "mkdir", "delete", "copy", "move", "append", "atomic-move", "preserve-modified"]);
const KNOWN_CAPABILITIES = new Set([
  "read", "write", "entries", "mkdir", "delete", "copy", "move", "append", "atomic-move", "preserve-modified", "revision-check"
]);

function fail(code, message, data = undefined, retryable = false) {
  const error = new Error(`${code}: ${message}`);
  error.code = code;
  error.data = data;
  error.retryable = retryable;
  throw error;
}

function valueName(value) {
  return value && typeof value === "object" && typeof value.name === "string" ? value.name : value;
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
  if (value && typeof value === "object" && typeof value.name === "string" && Object.keys(value).every(key => key === "name")) return value.name;
  return value;
}

function wire(value) {
  if (value === null || value === undefined || value instanceof Uint8Array) return value ?? null;
  if (Array.isArray(value)) return value.map(wire);
  if (value instanceof Map) return new Map([...value].map(([key, item]) => [key, wire(item)]));
  if (value && typeof value === "object") return new Map(Object.entries(value).map(([key, item]) => [key, wire(item)]));
  return value;
}

function objectOptions(value, allowed, label) {
  const options = plain(value ?? new Map());
  if (!options || typeof options !== "object" || Array.isArray(options)) fail("file/descriptor-invalid", `${label} must be a map`);
  for (const key of Object.keys(options)) if (!allowed.has(key)) fail("file/descriptor-invalid", `unknown ${label} option ${key}`);
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
  if (!Number.isSafeInteger(value) || value <= 0 || value > maximum) fail("file/descriptor-invalid", `${key} must be a positive bounded integer`);
  return value;
}

function textOption(options, key, fallback) {
  const value = options[key];
  if (value === undefined) return fallback;
  if (typeof value !== "string" || !value.trim()) fail("file/descriptor-invalid", `${key} must be a nonblank string`);
  return value;
}

function normalisePath(input) {
  if (typeof input !== "string") fail("file/invalid-path", "logical path must be a string");
  if (input === "" || input === "/") return "/";
  if (input.includes("\0") || input.includes("\\") || input.includes(":")) fail("file/invalid-path", "SFTP logical paths contain no host syntax");
  const value = input.startsWith("/") ? input.slice(1) : input;
  const segments = value.split("/");
  if (segments.some(segment => !segment || segment === ".")) fail("file/invalid-path", "SFTP logical path contains an empty or dot segment");
  if (segments.some(segment => segment === "..")) fail("file/outside-root", "SFTP logical path escapes the mounted root");
  return `/${segments.join("/")}`;
}

function logicalParent(path) {
  if (path === "/") return null;
  const index = path.lastIndexOf("/");
  return index === 0 ? "/" : path.slice(0, index);
}

function leafName(path) {
  return path === "/" ? "/" : path.slice(path.lastIndexOf("/") + 1);
}

function remoteRoot(value) {
  if (typeof value !== "string" || !value.startsWith("/") || value.includes("\0") || value.includes("\\")) fail("file/descriptor-invalid", "SFTP root must be an absolute remote path");
  if (value.split("/").some(segment => segment === "..")) fail("file/descriptor-invalid", "SFTP root cannot contain parent traversal");
  const normalized = posix.normalize(value);
  if (normalized !== "/" && normalized.endsWith("/")) return normalized.slice(0, -1);
  return normalized;
}

function credentialReference(value) {
  if (typeof value !== "string" || !value.trim() || value.includes("\0") || value.includes("/")) fail("file/descriptor-invalid", "SFTP credential reference is malformed");
  return value;
}

function hostKeyPolicy(value) {
  const policy = plain(value);
  if (!policy || typeof policy !== "object" || Array.isArray(policy)) fail("file/descriptor-invalid", "SFTP host-key policy is required");
  const type = String(policy.type ?? "");
  if (type === "pinned") {
    if (!Array.isArray(policy.fingerprints) || policy.fingerprints.length === 0 || policy.fingerprints.some(item => typeof item !== "string" || !item.trim())) fail("file/descriptor-invalid", "pinned SFTP host-key policy requires fingerprints");
    return { type, fingerprints: [...policy.fingerprints] };
  }
  if (type === "known-hosts") {
    if (typeof policy.reference !== "string" || !policy.reference.trim()) fail("file/descriptor-invalid", "known-hosts SFTP policy requires a trusted reference");
    return { type, reference: policy.reference };
  }
  fail("file/descriptor-invalid", "SFTP host-key policy must be pinned or known-hosts");
}

function capabilitySet(values, readOnly) {
  if (!Array.isArray(values)) fail("file/descriptor-invalid", "SFTP negotiated capabilities must be a vector");
  const output = new Set();
  for (const raw of values) {
    const capability = String(valueName(raw));
    if (!KNOWN_CAPABILITIES.has(capability)) fail("file/descriptor-invalid", `unknown SFTP capability ${capability}`);
    if (!readOnly || !MUTATING_CAPABILITIES.has(capability)) output.add(capability);
  }
  output.add("read");
  output.add("entries");
  return output;
}

function requireCapability(state, capability, operation, path) {
  if (state.closed) fail("file/provider-closed", "SFTP filesystem is closed", { operation, path });
  if (!state.capabilities.has(capability)) fail("file/unsupported", `SFTP capability ${capability} is unavailable`, { operation, path });
  if (state.readOnly && MUTATING_CAPABILITIES.has(capability)) fail("file/permission-denied", "SFTP filesystem is read-only", { operation, path });
}

function mutationContext(value) {
  const mutation = plain(value ?? new Map());
  if (!mutation || typeof mutation !== "object" || Array.isArray(mutation)) fail("file/descriptor-invalid", "mutation context must be a map");
  return {
    expectedRevision: mutation["expected-revision"] ?? mutation.expected_revision ?? null,
    expectedTargetRevision: mutation["expected-target-revision"] ?? mutation.expected_target_revision ?? null
  };
}

function revisionMatches(actual, expected, path, target = false) {
  if (expected === null || expected === undefined) return;
  if (typeof expected !== "string" || !expected.length) fail("file/descriptor-invalid", "expected revision must be a nonblank string");
  if (actual !== expected) fail("file/conflict", "SFTP revision mismatch", { path, target, expected, actual: actual ?? null, reason: "revision-mismatch" }, true);
}

function normaliseHostError(error, operation, path) {
  if (error?.code?.startsWith?.("file/")) throw error;
  if (error?.code === "ENOENT" || error?.code === "SSH_FX_NO_SUCH_FILE") fail("file/not-found", "SFTP path does not exist", { operation, path });
  if (error?.code === "EACCES" || error?.code === "SSH_FX_PERMISSION_DENIED") fail("file/permission-denied", "SFTP operation is forbidden", { operation, path });
  if (error?.name === "AbortError" || /abort|cancel/i.test(String(error?.message ?? error))) fail("file/cancelled", "SFTP operation was cancelled", { operation, path });
  fail("file/io", "SFTP host transport failed", { operation, path }, true);
}

function entry(path, metadata, state) {
  const type = metadata?.type === "symlink" || metadata?.isSymbolicLink?.() === true ? "symlink"
    : metadata?.type === "directory" || metadata?.isDirectory?.() === true ? "directory" : "file";
  const size = metadata?.size === undefined || metadata?.size === null ? null : Number(metadata.size);
  const modified = metadata?.modifiedAt ?? metadata?.mtimeMs ?? (metadata?.mtime instanceof Date ? metadata.mtime.getTime() : null);
  const revision = state.revisionSupported
    ? String(metadata?.revision ?? (Number.isFinite(modified) ? `${modified}:${Number.isFinite(size) ? size : ""}` : "")) || null
    : null;
  return {
    path,
    name: leafName(path),
    type,
    size: type === "directory" || !Number.isSafeInteger(size) || size < 0 ? null : size,
    "modified-at": Number.isSafeInteger(modified) && modified >= 0 ? modified : null,
    id: metadata?.id ?? path,
    revision,
    capabilities: null,
    extensions: { "provider/no-follow": type === "symlink" }
  };
}

function createSftpHost(configuration = {}) {
  const root = remoteRoot(configuration.root);
  const credentialRef = credentialReference(configuration.credentialRef);
  const policy = hostKeyPolicy(configuration.hostKeyPolicy);
  const connectionFactory = configuration.connectionFactory;
  if (typeof connectionFactory !== "function") fail("file/descriptor-invalid", "SFTP host requires a connectionFactory");
  const configuredCapabilities = configuration.capabilities ?? null;
  const mounts = new Map();
  let nextMount = 0;

  function mount(id) {
    const state = mounts.get(String(id));
    if (!state || state.closed) fail("file/provider-closed", "unknown or closed SFTP filesystem", { id });
    return state;
  }

  function remote(state, path) {
    const logical = normalisePath(path);
    const value = logical === "/" ? state.root : posix.join(state.root, logical.slice(1));
    if (value !== state.root && !value.startsWith(`${state.root}/`)) fail("file/outside-root", "SFTP path escaped the configured root", { path: logical });
    return value;
  }

  function requestId(receiver) {
    return String(receiver?.call ?? receiver?.task ?? `sftp-${Date.now()}-${Math.random()}`);
  }

  async function withPending(state, receiver, operation, path, action) {
    const id = requestId(receiver);
    const controller = new AbortController();
    const abort = () => controller.abort(receiver?.signal?.reason ?? new Error("cancelled"));
    if (receiver?.signal?.aborted) abort();
    receiver?.signal?.addEventListener?.("abort", abort, { once: true });
    state.pending.set(id, { id, operation, path, controller });
    try { return await action(controller.signal, id); }
    catch (error) { normaliseHostError(error, operation, path); }
    finally { state.pending.delete(id); receiver?.signal?.removeEventListener?.("abort", abort); }
  }

  async function clientCall(state, method, args, signal, operation, path) {
    const functionValue = state.client?.[method];
    if (typeof functionValue !== "function") fail("file/unsupported", `SFTP client does not provide ${method}`, { operation, path });
    try {
      return await functionValue.call(state.client, ...args, { signal });
    } catch (error) {
      normaliseHostError(error, operation, path);
    }
  }

  async function lstat(state, path, receiver, operation = "stat") {
    const logical = normalisePath(path);
    return withPending(state, receiver, operation, logical, async signal => {
      const remotePath = remote(state, logical);
      const value = await clientCall(state, "lstat", [remotePath], signal, operation, logical);
      return { raw: value, value: entry(logical, value, state) };
    });
  }

  async function guardAncestors(state, path, receiver, operation) {
    const logical = normalisePath(path);
    const ancestors = [];
    let current = logicalParent(logical);
    while (current !== null && current !== "/") { ancestors.push(current); current = logicalParent(current); }
    for (const ancestor of ancestors.reverse()) {
      const value = await lstat(state, ancestor, receiver, operation);
      if (value.value.type === "symlink") fail("file/outside-root", "SFTP symlink ancestor is not followed", { path: ancestor, reason: "no-follow" });
      if (value.value.type !== "directory") fail("file/not-directory", "SFTP path ancestor is not a directory", { path: ancestor });
    }
  }

  async function stat(state, path, receiver) {
    const logical = normalisePath(path);
    requireCapability(state, "read", "stat", logical);
    await guardAncestors(state, logical, receiver, "stat");
    return wire((await lstat(state, logical, receiver, "stat")).value);
  }

  async function read(state, path, receiver) {
    const logical = normalisePath(path);
    requireCapability(state, "read", "read", logical);
    await guardAncestors(state, logical, receiver, "read");
    const metadata = await lstat(state, logical, receiver, "read");
    if (metadata.value.type === "directory") fail("file/is-directory", "cannot read an SFTP directory", { path: logical });
    if (metadata.value.type !== "file") fail("file/unsupported", "SFTP symlinks are not followed", { path: logical, reason: "no-follow" });
    if (metadata.value.size !== null && metadata.value.size > state.maxTransferBytes) fail("file/quota-exceeded", "SFTP file exceeds the configured transfer limit", { path: logical });
    return withPending(state, receiver, "read", logical, async signal => {
      const bytes = await clientCall(state, "readFile", [remote(state, logical)], signal, "read", logical);
      const output = bytes instanceof Uint8Array ? bytes : bytes instanceof ArrayBuffer ? new Uint8Array(bytes) : null;
      if (!output) fail("file/io", "SFTP client returned a non-byte read result", { path: logical });
      if (output.byteLength > state.maxTransferBytes) fail("file/quota-exceeded", "SFTP response exceeds the configured transfer limit", { path: logical });
      return output;
    });
  }

  async function optionalLstat(state, path, receiver, operation) {
    try { return await lstat(state, path, receiver, operation); }
    catch (error) { if (error?.code === "file/not-found") return null; throw error; }
  }

  async function ensureParent(state, path, parents, receiver, operation) {
    const parent = logicalParent(path);
    if (!parent) return;
    const existing = await optionalLstat(state, parent, receiver, operation);
    if (existing) {
      if (existing.value.type !== "directory") fail("file/not-directory", "SFTP parent is not a directory", { path: parent });
      return;
    }
    if (!parents) fail("file/not-found", "SFTP parent directory does not exist", { path: parent });
    requireCapability(state, "mkdir", operation, parent);
    await clientCall(state, "mkdir", [remote(state, parent), { recursive: true }], receiver?.signal, operation, parent);
  }

  async function write(state, path, bytesValue, optionsValue, mutationValue, receiver) {
    const logical = normalisePath(path);
    requireCapability(state, "write", "write", logical);
    if (!(bytesValue instanceof Uint8Array)) fail("file/descriptor-invalid", "SFTP write requires exact bytes", { path: logical });
    if (bytesValue.byteLength > state.maxTransferBytes) fail("file/quota-exceeded", "SFTP write exceeds the configured transfer limit", { path: logical });
    const options = objectOptions(optionsValue, new Set(["mode", "parents"]), "write");
    const mode = String(valueName(options.mode ?? "create"));
    if (!["create", "replace", "append"].includes(mode)) fail("file/descriptor-invalid", `unknown SFTP write mode ${mode}`);
    if (mode === "append") requireCapability(state, "append", "write", logical);
    const mutation = mutationContext(mutationValue);
    await guardAncestors(state, logical, receiver, "write");
    await ensureParent(state, logical, booleanOption(options, "parents", false), receiver, "write");
    const existing = await optionalLstat(state, logical, receiver, "write");
    revisionMatches(existing?.value.revision ?? null, mutation.expectedRevision, logical);
    if (mode === "create" && existing) fail("file/already-exists", "SFTP path already exists", { path: logical });
    if (mode === "replace" && !existing) fail("file/not-found", "SFTP path does not exist", { path: logical });
    if (existing && existing.value.type !== "file") fail("file/unsupported", "SFTP symlinks and directories are not written through", { path: logical, reason: "no-follow" });
    return withPending(state, receiver, "write", logical, async signal => {
      await clientCall(state, "writeFile", [remote(state, logical), bytesValue, { mode }], signal, "write", logical);
      const updated = await clientCall(state, "lstat", [remote(state, logical)], signal, "write", logical);
      const result = entry(logical, updated, state);
      return wire({ path: logical, revision: result.revision, "mount-revision": null, extensions: {} });
    });
  }

  async function entriesPage(state, path, requestValue, receiver) {
    const logical = normalisePath(path);
    requireCapability(state, "entries", "entries-page", logical);
    await guardAncestors(state, logical, receiver, "entries-page");
    const directory = await lstat(state, logical, receiver, "entries-page");
    if (directory.value.type !== "directory") fail("file/not-directory", "SFTP entry is not a directory", { path: logical });
    const request = objectOptions(requestValue, new Set(["limit", "token"]), "entries page request");
    const limit = positiveInteger(request, "limit", DEFAULT_PAGE_LIMIT, MAX_PAGE_LIMIT);
    let snapshot;
    let offset = 0;
    if (request.token !== undefined && request.token !== null) {
      const saved = state.pageTokens.get(String(request.token));
      if (!saved || saved.path !== logical) fail("file/invalid-page-token", "unknown SFTP page token", { path: logical });
      snapshot = saved.entries;
      offset = saved.offset;
      state.pageTokens.delete(String(request.token));
    } else {
      const values = await withPending(state, receiver, "entries-page", logical, async signal => clientCall(state, "readdir", [remote(state, logical)], signal, "entries-page", logical));
      if (!Array.isArray(values) || values.length > MAX_ENTRIES) fail("file/quota-exceeded", "SFTP directory exceeds the entry limit", { path: logical });
      snapshot = [];
      for (const value of values) {
        const name = typeof value === "string" ? value : value?.name;
        if (!name || name === "." || name === ".." || name.includes("/") || name.includes("\\")) fail("file/io", "SFTP client returned an invalid entry name", { path: logical });
        const childPath = logical === "/" ? `/${name}` : `${logical}/${name}`;
        const metadata = typeof value === "string" ? await lstat(state, childPath, receiver, "entries-page") : value;
        snapshot.push(entry(childPath, metadata.raw ?? metadata, state));
      }
      snapshot.sort((left, right) => left.path.localeCompare(right.path));
    }
    const values = snapshot.slice(offset, offset + limit);
    const nextOffset = offset + values.length;
    let nextToken = null;
    if (nextOffset < snapshot.length) {
      nextToken = `sftp-page-${++state.pageSequence}`;
      state.pageTokens.set(nextToken, { path: logical, entries: snapshot, offset: nextOffset });
    }
    return wire({ entries: values, "next-token": nextToken });
  }

  async function mkdir(state, path, optionsValue, mutationValue, receiver) {
    const logical = normalisePath(path);
    requireCapability(state, "mkdir", "mkdir", logical);
    const options = objectOptions(optionsValue, new Set(["parents", "exists-ok"]), "mkdir");
    const mutation = mutationContext(mutationValue);
    await guardAncestors(state, logical, receiver, "mkdir");
    const existing = await optionalLstat(state, logical, receiver, "mkdir");
    revisionMatches(existing?.value.revision ?? null, mutation.expectedRevision, logical);
    if (existing) {
      if (existing.value.type === "directory" && booleanOption(options, "exists-ok", true)) return wire({ path: logical, revision: existing.value.revision, "mount-revision": null, extensions: {} });
      fail("file/already-exists", "SFTP path already exists", { path: logical });
    }
    const parent = logicalParent(logical);
    if (!parent) fail("file/denied", "cannot create the mounted SFTP root");
    await ensureParent(state, logical, booleanOption(options, "parents", true), receiver, "mkdir");
    return withPending(state, receiver, "mkdir", logical, async signal => {
      await clientCall(state, "mkdir", [remote(state, logical), { recursive: booleanOption(options, "parents", true) }], signal, "mkdir", logical);
      const updated = await clientCall(state, "lstat", [remote(state, logical)], signal, "mkdir", logical);
      const result = entry(logical, updated, state);
      return wire({ path: logical, revision: result.revision, "mount-revision": null, extensions: {} });
    });
  }

  async function deleteEntry(state, path, optionsValue, mutationValue, receiver) {
    const logical = normalisePath(path);
    requireCapability(state, "delete", "delete", logical);
    if (logical === "/") fail("file/denied", "cannot delete the mounted SFTP root");
    const options = objectOptions(optionsValue, new Set(["missing-ok"]), "delete");
    const mutation = mutationContext(mutationValue);
    await guardAncestors(state, logical, receiver, "delete");
    const existing = await optionalLstat(state, logical, receiver, "delete");
    if (!existing) {
      if (mutation.expectedRevision) revisionMatches(null, mutation.expectedRevision, logical);
      if (booleanOption(options, "missing-ok", false)) return wire({ path: logical, revision: null, "mount-revision": null, extensions: {} });
      fail("file/not-found", "SFTP path does not exist", { path: logical });
    }
    revisionMatches(existing.value.revision, mutation.expectedRevision, logical);
    return withPending(state, receiver, "delete", logical, async signal => {
      const method = existing.value.type === "directory" ? "rmdir" : "unlink";
      await clientCall(state, method, [remote(state, logical)], signal, "delete", logical);
      return wire({ path: logical, revision: null, "mount-revision": null, extensions: {} });
    });
  }

  async function copyEntry(state, source, target, optionsValue, mutationValue, receiver) {
    const sourcePath = normalisePath(source);
    const targetPath = normalisePath(target);
    requireCapability(state, "copy", "copy", sourcePath);
    if (sourcePath === targetPath) fail("file/already-exists", "SFTP copy source and target are identical");
    const options = objectOptions(optionsValue, new Set(["replace", "parents", "preserve-modified"]), "copy");
    if (booleanOption(options, "preserve-modified", false)) requireCapability(state, "preserve-modified", "copy", sourcePath);
    const mutation = mutationContext(mutationValue);
    await guardAncestors(state, sourcePath, receiver, "copy");
    const sourceEntry = await lstat(state, sourcePath, receiver, "copy");
    if (sourceEntry.value.type !== "file") fail("file/unsupported", "SFTP copy only supports regular files");
    revisionMatches(sourceEntry.value.revision, mutation.expectedRevision, sourcePath);
    await ensureParent(state, targetPath, booleanOption(options, "parents", false), receiver, "copy");
    const targetEntry = await optionalLstat(state, targetPath, receiver, "copy");
    revisionMatches(targetEntry?.value.revision ?? null, mutation.expectedTargetRevision, targetPath, true);
    if (targetEntry && !booleanOption(options, "replace", false)) fail("file/already-exists", "SFTP copy target already exists");
    if (targetEntry && targetEntry.value.type !== "file") fail("file/unsupported", "SFTP copy target is not a regular file");
    return withPending(state, receiver, "copy", sourcePath, async signal => {
      await clientCall(state, "copyFile", [remote(state, sourcePath), remote(state, targetPath), { replace: booleanOption(options, "replace", false) }], signal, "copy", sourcePath);
      const updated = await clientCall(state, "lstat", [remote(state, targetPath)], signal, "copy", targetPath);
      const result = entry(targetPath, updated, state);
      return wire({ path: targetPath, revision: result.revision, "mount-revision": null, extensions: {} });
    });
  }

  async function moveEntry(state, source, target, optionsValue, mutationValue, receiver) {
    const sourcePath = normalisePath(source);
    const targetPath = normalisePath(target);
    requireCapability(state, "move", "move", sourcePath);
    if (sourcePath === "/" || targetPath === "/") fail("file/denied", "cannot move the mounted SFTP root");
    const options = objectOptions(optionsValue, new Set(["replace", "parents", "atomic"]), "move");
    if (booleanOption(options, "atomic", false)) requireCapability(state, "atomic-move", "move", sourcePath);
    const mutation = mutationContext(mutationValue);
    await guardAncestors(state, sourcePath, receiver, "move");
    const sourceEntry = await lstat(state, sourcePath, receiver, "move");
    revisionMatches(sourceEntry.value.revision, mutation.expectedRevision, sourcePath);
    await ensureParent(state, targetPath, booleanOption(options, "parents", false), receiver, "move");
    const targetEntry = await optionalLstat(state, targetPath, receiver, "move");
    revisionMatches(targetEntry?.value.revision ?? null, mutation.expectedTargetRevision, targetPath, true);
    if (targetEntry && !booleanOption(options, "replace", false)) fail("file/already-exists", "SFTP move target already exists");
    return withPending(state, receiver, "move", sourcePath, async signal => {
      await clientCall(state, "rename", [remote(state, sourcePath), remote(state, targetPath), { replace: booleanOption(options, "replace", false) }], signal, "move", sourcePath);
      const updated = await clientCall(state, "lstat", [remote(state, targetPath)], signal, "move", targetPath);
      const result = entry(targetPath, updated, state);
      return wire({ path: targetPath, revision: result.revision, "mount-revision": null, extensions: {} });
    });
  }

  async function open(optionsValue, receiver) {
    const options = objectOptions(optionsValue, new Set(["display", "read-only?", "operation-timeout-ms", "max-transfer-bytes"]), "SFTP filesystem");
    const readOnly = booleanOption(options, "read-only?", false);
    const state = {
      id: `sftp-host-${++nextMount}`,
      root,
      credentialRef,
      policy,
      client: null,
      readOnly,
      capabilities: new Set(["read", "entries"]),
      revisionSupported: false,
      maxTransferBytes: positiveInteger(options, "max-transfer-bytes", DEFAULT_MAX_TRANSFER_BYTES, 256 * 1024 * 1024),
      display: textOption(options, "display", "SFTP filesystem"),
      pending: new Map(),
      pageTokens: new Map(),
      pageSequence: 0,
      closed: false
    };
    mounts.set(state.id, state);
    try {
      await withPending(state, receiver, "open", "/", async signal => {
        state.client = await connectionFactory({ credentialRef, root, hostKeyPolicy: policy }, { signal });
        if (!state.client || state.client.authenticated !== true || state.client.hostKeyVerified !== true) fail("file/authentication-failed", "SFTP connection is not authenticated with a verified host key", { reason: "transport-unverified" });
        const negotiated = configuredCapabilities ?? state.client.capabilities;
        state.capabilities = capabilitySet(negotiated ?? ["read", "entries"], readOnly);
        state.revisionSupported = state.capabilities.has("revision-check");
        const rootValue = await clientCall(state, "lstat", [root], signal, "open", "/");
        const rootEntry = entry("/", rootValue, state);
        if (rootEntry.type === "symlink") fail("file/outside-root", "SFTP root cannot be a symbolic link", { reason: "root-symlink" });
        if (rootEntry.type !== "directory") fail("file/not-directory", "SFTP root is not a directory", { reason: "root-not-directory" });
      });
      const descriptor = {
        kind: "sftp",
        display: state.display,
        "read-only": readOnly,
        capabilities: [...state.capabilities].sort(),
        revision: null,
        extensions: {
          "provider/root-scoped?": true,
          "provider/transport-verified?": true,
          "provider/host-key-verified?": true,
          "provider/route": "hta-wasm",
          "provider/browser-policy": "external-host-only"
        }
      };
      return wire({ mount: state.id, descriptor });
    } catch (error) {
      await close(state.id);
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
      default: fail("file/unsupported", `unsupported SFTP filesystem operation ${operation}`);
    }
  }

  async function cancel(mountId, id) {
    const state = mounts.get(String(mountId));
    if (!state) return false;
    const pending = state.pending.get(String(id));
    if (!pending) return false;
    pending.controller.abort(new Error("cancelled"));
    state.client?.cancel?.(String(id));
    return true;
  }

  async function close(mountId) {
    const state = mounts.get(String(mountId));
    if (!state) return null;
    state.closed = true;
    for (const pending of state.pending.values()) pending.controller.abort(new Error("closed"));
    state.pending.clear();
    state.pageTokens.clear();
    try { await state.client?.close?.(); } finally { mounts.delete(String(mountId)); }
    return null;
  }

  async function closeAll() { await Promise.all([...mounts.keys()].map(id => close(id))); }

  async function describe() {
    return wire({
      provider: "sftp",
      identity: "hara/filesystem-sftp",
      abi: "hta.v1",
      route: "hta-wasm",
      capabilities: [...KNOWN_CAPABILITIES].sort(),
      browser: "external-host-only"
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

export { createSftpHost, normalisePath, remoteRoot, plain };
