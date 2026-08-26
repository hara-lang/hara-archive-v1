const SERVICE = "filesystem.google-drive";
const DEFAULT_API_BASE = "https://www.googleapis.com/drive/v3";
const DEFAULT_UPLOAD_BASE = "https://www.googleapis.com/upload/drive/v3";
const DEFAULT_MAX_TRANSFER_BYTES = 64 * 1024 * 1024;
const DEFAULT_PAGE_LIMIT = 256;
const MAX_PAGE_LIMIT = 1_000;
const MAX_JSON_BYTES = 2 * 1024 * 1024;
const FOLDER_MIME = "application/vnd.google-apps.folder";
const SHORTCUT_MIME = "application/vnd.google-apps.shortcut";
const MUTATING_CAPABILITIES = new Set(["write", "mkdir", "delete", "copy", "move"]);
const KNOWN_CAPABILITIES = new Set([
  "read", "write", "entries", "mkdir", "delete", "copy", "move", "revision-check"
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
  if (value && typeof value === "object" && typeof value.name === "string" && Object.keys(value).every(key => key === "name")) {
    return value.name;
  }
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
  if (input.includes("\0") || input.includes("\\") || input.includes(":")) fail("file/invalid-path", "Google Drive logical paths contain no host syntax");
  const value = input.startsWith("/") ? input.slice(1) : input;
  const segments = value.split("/");
  if (segments.some(segment => !segment || segment === ".")) fail("file/invalid-path", "Google Drive logical path contains an empty or dot segment");
  if (segments.some(segment => segment === "..")) fail("file/outside-root", "Google Drive logical path escapes the mounted root");
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

function capabilitySet(values, readOnly) {
  if (!Array.isArray(values)) fail("file/descriptor-invalid", "Google Drive host capabilities must be a vector");
  const output = new Set();
  for (const raw of values) {
    const capability = String(valueName(raw));
    if (!KNOWN_CAPABILITIES.has(capability)) fail("file/descriptor-invalid", `unknown Google Drive capability ${capability}`);
    if (!readOnly || !MUTATING_CAPABILITIES.has(capability)) output.add(capability);
  }
  output.add("read");
  output.add("entries");
  return output;
}

function requireCapability(state, capability, operation, path) {
  if (state.closed) fail("file/provider-closed", "Google Drive filesystem is closed", { operation, path });
  if (!state.capabilities.has(capability)) fail("file/unsupported", `Google Drive capability ${capability} is unavailable`, { operation, path });
  if (state.readOnly && MUTATING_CAPABILITIES.has(capability)) fail("file/permission-denied", "Google Drive filesystem is read-only", { operation, path });
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
  if (actual !== expected) fail("file/conflict", "Google Drive revision mismatch", { path, target, expected, actual: actual ?? null, reason: "revision-mismatch" }, true);
}

function normaliseHostError(error, operation, path) {
  if (error?.code?.startsWith?.("file/")) throw error;
  if (error?.name === "AbortError" || /abort|cancel/i.test(String(error?.message ?? error))) fail("file/cancelled", "Google Drive operation was cancelled", { operation, path });
  fail("file/io", "Google Drive host transport failed", { operation, path }, true);
}

function endpoint(value, label) {
  let url;
  try { url = new URL(value); } catch { fail("file/descriptor-invalid", `${label} must be an absolute URL`); }
  if (url.protocol !== "https:") fail("file/descriptor-invalid", `${label} requires HTTPS`);
  if (url.username || url.password || url.search || url.hash) fail("file/descriptor-invalid", `${label} cannot contain credentials, query, or fragment`);
  return url.toString().replace(/\/$/, "");
}

function requiredId(value, label) {
  if (typeof value !== "string" || !value.trim() || value.includes("\0") || /[\s/]/.test(value)) fail("file/descriptor-invalid", `${label} is malformed`);
  return value;
}

function entry(path, metadata) {
  const mime = String(metadata?.mimeType ?? "");
  const type = mime === FOLDER_MIME ? "directory" : mime === SHORTCUT_MIME ? "symlink" : mime.startsWith("application/vnd.google-apps.") ? "other" : "file";
  const size = metadata?.size === undefined || metadata?.size === null ? null : Number(metadata.size);
  return {
    path,
    name: path === "/" ? "/" : leafName(path),
    type,
    size: type === "directory" || !Number.isSafeInteger(size) || size < 0 ? null : size,
    "modified-at": metadata?.modifiedTime ? Date.parse(metadata.modifiedTime) || null : null,
    id: metadata?.id ?? null,
    revision: metadata?.headRevisionId ?? metadata?.md5Checksum ?? null,
    capabilities: null,
    extensions: {
      ...(type === "symlink" ? { "provider/shortcut-id": metadata?.shortcutDetails?.targetId ?? null } : {}),
      "provider/mime-type": mime
    }
  };
}

function jsonError(status, operation, path, value) {
  const reason = value?.error?.errors?.[0]?.reason;
  if (status === 401) fail("file/authentication-failed", "Google Drive authentication failed", { operation, path, reason });
  if (status === 403) fail("file/permission-denied", "Google Drive operation is forbidden", { operation, path, reason });
  if (status === 404) fail("file/not-found", "Google Drive item was not found", { operation, path, reason });
  if (status === 409 || status === 412) fail("file/conflict", "Google Drive item conflict", { operation, path, reason });
  if (status === 429) fail("file/rate-limited", "Google Drive request was rate limited", { operation, path, reason }, true);
  if (status >= 500) fail("file/io", `Google Drive service failed with status ${status}`, { operation, path, reason }, true);
  fail("file/io", `unexpected Google Drive response status ${status}`, { operation, path, reason });
}

async function responseJson(response, operation, path) {
  const length = Number(response.headers?.get?.("content-length"));
  if (Number.isSafeInteger(length) && length > MAX_JSON_BYTES) fail("file/quota-exceeded", "Google Drive metadata response is too large", { operation, path });
  const text = await response.text();
  if (text.length > MAX_JSON_BYTES) fail("file/quota-exceeded", "Google Drive metadata response is too large", { operation, path });
  try { return JSON.parse(text || "{}"); } catch { fail("file/io", "Google Drive returned malformed JSON", { operation, path }); }
}

async function responseBytes(response, maxBytes, operation, path) {
  const length = Number(response.headers?.get?.("content-length"));
  if (Number.isSafeInteger(length) && length > maxBytes) fail("file/quota-exceeded", "Google Drive response exceeds the configured transfer limit", { operation, path });
  const bytes = new Uint8Array(await response.arrayBuffer());
  if (bytes.byteLength > maxBytes) fail("file/quota-exceeded", "Google Drive response exceeds the configured transfer limit", { operation, path });
  return bytes;
}

function concatBytes(parts) {
  const total = parts.reduce((size, part) => size + part.byteLength, 0);
  const result = new Uint8Array(total);
  let offset = 0;
  for (const part of parts) { result.set(part, offset); offset += part.byteLength; }
  return result;
}

function multipartBody(metadata, bytes) {
  const boundary = `hara-${Math.random().toString(16).slice(2)}`;
  const prefix = new TextEncoder().encode(`--${boundary}\r\nContent-Type: application/json; charset=UTF-8\r\n\r\n${JSON.stringify(metadata)}\r\n--${boundary}\r\nContent-Type: application/octet-stream\r\n\r\n`);
  const suffix = new TextEncoder().encode(`\r\n--${boundary}--\r\n`);
  return { boundary, body: concatBytes([prefix, bytes, suffix]) };
}

function createGoogleDriveHost(configuration = {}) {
  const rootId = requiredId(configuration.rootId, "Google Drive root id");
  const driveId = configuration.driveId === undefined ? null : requiredId(configuration.driveId, "Google Drive shared drive id");
  const apiBase = endpoint(configuration.apiBase ?? DEFAULT_API_BASE, "Google Drive API base");
  const uploadBase = endpoint(configuration.uploadBase ?? DEFAULT_UPLOAD_BASE, "Google Drive upload base");
  const tokenProvider = configuration.tokenProvider;
  if (typeof tokenProvider !== "function") fail("file/descriptor-invalid", "Google Drive host requires a tokenProvider");
  const fetcher = configuration.fetch ?? globalThis.fetch?.bind(globalThis);
  if (typeof fetcher !== "function") fail("file/host-unavailable", "Google Drive host requires fetch");
  const configuredCapabilities = configuration.capabilities ?? ["read", "entries"];
  const exportMimeTypes = plain(configuration.exportMimeTypes ?? {});
  const mounts = new Map();
  let nextMount = 0;

  function mount(id) {
    const state = mounts.get(String(id));
    if (!state || state.closed) fail("file/provider-closed", "unknown or closed Google Drive filesystem", { id });
    return state;
  }

  function requestId(receiver) {
    return String(receiver?.call ?? receiver?.task ?? `drive-${Date.now()}-${Math.random()}`);
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

  async function http(state, operation, path, url, init, signal) {
    const token = await tokenProvider({ operation, path }, { signal });
    if (typeof token !== "string" || !token.trim()) fail("file/authentication-failed", "Google Drive token provider returned no token", { operation, path });
    const headers = new Headers(init.headers ?? {});
    headers.set("Authorization", `Bearer ${token}`);
    let response;
    try { response = await fetcher(url, { ...init, headers, signal }); }
    catch (error) { normaliseHostError(error, operation, path); }
    if (!response.ok) {
      let value = {};
      try { value = await responseJson(response, operation, path); } catch (error) { if (error?.code) throw error; }
      jsonError(Number(response.status), operation, path, value);
    }
    return response;
  }

  function metadataUrl(id, params = {}) {
    const url = new URL(`${apiBase}/files/${encodeURIComponent(id)}`);
    url.searchParams.set("fields", "id,name,mimeType,size,modifiedTime,headRevisionId,md5Checksum,parents,trashed,shortcutDetails,capabilities");
    if (driveId) url.searchParams.set("supportsAllDrives", "true");
    for (const [key, value] of Object.entries(params)) if (value !== undefined && value !== null) url.searchParams.set(key, value);
    return url;
  }

  function listUrl(state, parentId, name, pageToken, pageSize) {
    const url = new URL(`${apiBase}/files`);
    const escaped = name === null ? null : name.replaceAll("'", "\\'");
    const clauses = [`'${parentId}' in parents`, "trashed = false"];
    if (escaped !== null) clauses.push(`name = '${escaped}'`);
    url.searchParams.set("q", clauses.join(" and "));
    url.searchParams.set("pageSize", String(pageSize));
    url.searchParams.set("orderBy", "name,createdTime,id");
    url.searchParams.set("fields", "nextPageToken,files(id,name,mimeType,size,modifiedTime,headRevisionId,md5Checksum,parents,trashed,shortcutDetails,capabilities)");
    if (pageToken) url.searchParams.set("pageToken", pageToken);
    if (driveId) {
      url.searchParams.set("corpora", "drive");
      url.searchParams.set("driveId", driveId);
      url.searchParams.set("includeItemsFromAllDrives", "true");
      url.searchParams.set("supportsAllDrives", "true");
    }
    return url;
  }

  async function getMetadata(state, id, receiver, operation = "stat", path = "/") {
    return withPending(state, receiver, operation, path, async signal => {
      const response = await http(state, operation, path, metadataUrl(id), { method: "GET" }, signal);
      return responseJson(response, operation, path);
    });
  }

  async function listChildren(state, parentId, name, pageToken, pageSize, receiver, path) {
    return withPending(state, receiver, "entries-page", path, async signal => {
      const response = await http(state, "entries-page", path, listUrl(state, parentId, name, pageToken, pageSize), { method: "GET" }, signal);
      return responseJson(response, "entries-page", path);
    });
  }

  async function resolve(state, path, receiver) {
    const logical = normalisePath(path);
    if (logical === "/") return state.rootMetadata;
    if (state.pathCache.has(logical)) return state.pathCache.get(logical);
    let parentId = rootId;
    let current = "/";
    for (const segment of logical.slice(1).split("/")) {
      const next = current === "/" ? `/${segment}` : `${current}/${segment}`;
      const result = await listChildren(state, parentId, segment, null, 100, receiver, next);
      const files = Array.isArray(result?.files) ? result.files : [];
      if (files.length === 0) fail("file/not-found", "Google Drive path does not exist", { path: next });
      if (files.length > 1) fail("file/ambiguous-path", "Google Drive path has duplicate names", { path: next, count: files.length });
      const metadata = files[0];
      if (next !== logical && metadata.mimeType !== FOLDER_MIME) fail("file/not-directory", "Google Drive path ancestor is not a folder", { path: next });
      if (metadata.mimeType === SHORTCUT_MIME && next !== logical) fail("file/unsupported", "Google Drive shortcuts are not followed", { path: next, reason: "no-follow" });
      state.pathCache.set(next, metadata);
      current = next;
      parentId = metadata.id;
    }
    return state.pathCache.get(logical);
  }

function ensureBinary(metadata, path) {
    if (metadata.mimeType === FOLDER_MIME) fail("file/is-directory", "cannot read a Google Drive folder", { path });
    if (metadata.mimeType === SHORTCUT_MIME) fail("file/unsupported", "Google Drive shortcuts are not followed", { path, reason: "no-follow" });
    if (metadata.mimeType.startsWith("application/vnd.google-apps.")) {
      const exportMime = exportMimeTypes?.[metadata.mimeType];
      if (!exportMime) fail("file/unsupported", "Google Workspace document export is not enabled", { path, reason: "workspace-document" });
      return exportMime;
    }
    return null;
  }

  async function stat(state, path, receiver) {
    const logical = normalisePath(path);
    requireCapability(state, "read", "stat", logical);
    const metadata = await resolve(state, logical, receiver);
    return entry(logical, metadata);
  }

  async function read(state, path, receiver) {
    const logical = normalisePath(path);
    requireCapability(state, "read", "read", logical);
    const metadata = await resolve(state, logical, receiver);
    const exportMime = ensureBinary(metadata, logical);
    return withPending(state, receiver, "read", logical, async signal => {
      const url = exportMime
        ? metadataUrl(metadata.id, { alt: "media", mimeType: exportMime })
        : metadataUrl(metadata.id, { alt: "media" });
      if (exportMime) url.pathname = `${apiBase.replace(/\/$/, "")}/files/${encodeURIComponent(metadata.id)}/export`;
      const response = await http(state, "read", logical, url, { method: "GET" }, signal);
      return responseBytes(response, state.maxTransferBytes, "read", logical);
    });
  }

  async function parentMetadata(state, path, receiver) {
    const parent = logicalParent(path);
    if (parent === null) return null;
    const metadata = await resolve(state, parent, receiver);
    if (metadata.mimeType !== FOLDER_MIME) fail("file/not-directory", "Google Drive parent is not a folder", { path: parent });
    return metadata;
  }

  async function write(state, path, bytesValue, optionsValue, mutationValue, receiver) {
    const logical = normalisePath(path);
    requireCapability(state, "write", "write", logical);
    if (!(bytesValue instanceof Uint8Array)) fail("file/descriptor-invalid", "Google Drive write requires exact bytes", { path: logical });
    if (bytesValue.byteLength > state.maxTransferBytes) fail("file/quota-exceeded", "Google Drive write exceeds the configured transfer limit", { path: logical });
    const options = objectOptions(optionsValue, new Set(["mode", "parents"]), "write");
    const mode = String(valueName(options.mode ?? "create"));
    if (!["create", "replace", "append"].includes(mode)) fail("file/descriptor-invalid", `unknown Google Drive write mode ${mode}`);
    if (mode === "append") fail("file/unsupported", "Google Drive append is not advertised");
    const mutation = mutationContext(mutationValue);
    let existing = null;
    try { existing = await resolve(state, logical, receiver); } catch (error) { if (error?.code !== "file/not-found") throw error; }
    revisionMatches(existing ? entry(logical, existing).revision : null, mutation.expectedRevision, logical);
    if (mode === "create" && existing) fail("file/already-exists", "Google Drive path already exists", { path: logical });
    if (mode === "replace" && !existing) fail("file/not-found", "Google Drive path does not exist", { path: logical });
    if (existing) {
      ensureBinary(existing, logical);
      if (existing.mimeType.startsWith("application/vnd.google-apps.")) {
        fail("file/unsupported", "Google Workspace documents are not writable through the binary route", { path: logical });
      }
    }
    const parent = await parentMetadata(state, logical, receiver);
    if (!parent) fail("file/denied", "Google Drive root cannot be written");
    return withPending(state, receiver, "write", logical, async signal => {
      let response;
      if (existing) {
        const url = new URL(`${uploadBase}/files/${encodeURIComponent(existing.id)}`);
        url.searchParams.set("uploadType", "media");
        url.searchParams.set("supportsAllDrives", "true");
        response = await http(state, "write", logical, url, { method: "PATCH", headers: { "Content-Type": "application/octet-stream" }, body: bytesValue }, signal);
      } else {
        const url = new URL(`${uploadBase}/files`);
        url.searchParams.set("uploadType", "multipart");
        url.searchParams.set("supportsAllDrives", "true");
        const multipart = multipartBody({ name: leafName(logical), parents: [parent.id], mimeType: "application/octet-stream" }, bytesValue);
        response = await http(state, "write", logical, url, { method: "POST", headers: { "Content-Type": `multipart/related; boundary=${multipart.boundary}` }, body: multipart.body }, signal);
      }
      const metadata = await responseJson(response, "write", logical);
      state.pathCache.set(logical, metadata);
      return wire({ path: logical, revision: entry(logical, metadata).revision, "mount-revision": null, extensions: {} });
    });
  }

  async function entriesPage(state, path, requestValue, receiver) {
    const logical = normalisePath(path);
    requireCapability(state, "entries", "entries-page", logical);
    const metadata = await resolve(state, logical, receiver);
    if (metadata.mimeType !== FOLDER_MIME) fail("file/not-directory", "Google Drive entry is not a folder", { path: logical });
    const request = objectOptions(requestValue, new Set(["limit", "token"]), "entries page request");
    const limit = positiveInteger(request, "limit", DEFAULT_PAGE_LIMIT, MAX_PAGE_LIMIT);
    let upstreamToken = null;
    if (request.token !== undefined && request.token !== null) {
      const saved = state.pageTokens.get(String(request.token));
      if (!saved || saved.path !== logical || saved.parentId !== metadata.id) fail("file/invalid-page-token", "unknown Google Drive page token", { path: logical });
      upstreamToken = saved.upstreamToken;
      state.pageTokens.delete(String(request.token));
    }
    const result = await listChildren(state, metadata.id, null, upstreamToken, limit, receiver, logical);
    const values = (Array.isArray(result?.files) ? result.files : [])
      .sort((left, right) => `${left.name}\0${left.id}`.localeCompare(`${right.name}\0${right.id}`))
      .slice(0, limit)
      .map(item => entry(logical === "/" ? `/${item.name}` : `${logical}/${item.name}`, item));
    const next = result?.nextPageToken ?? null;
    let nextToken = null;
    if (next) {
      nextToken = `drive-page-${++state.pageSequence}`;
      state.pageTokens.set(nextToken, { path: logical, parentId: metadata.id, upstreamToken: next });
    }
    return wire({ entries: values, "next-token": nextToken });
  }

  async function mkdir(state, path, optionsValue, mutationValue, receiver) {
    const logical = normalisePath(path);
    requireCapability(state, "mkdir", "mkdir", logical);
    const options = objectOptions(optionsValue, new Set(["parents", "exists-ok"]), "mkdir");
    const mutation = mutationContext(mutationValue);
    let existing = null;
    try { existing = await resolve(state, logical, receiver); } catch (error) { if (error?.code !== "file/not-found") throw error; }
    revisionMatches(existing ? entry(logical, existing).revision : null, mutation.expectedRevision, logical);
    if (existing) {
      if (existing.mimeType === FOLDER_MIME && booleanOption(options, "exists-ok", true)) return wire({ path: logical, revision: entry(logical, existing).revision, "mount-revision": null, extensions: {} });
      fail("file/already-exists", "Google Drive path already exists", { path: logical });
    }
    const parent = await parentMetadata(state, logical, receiver);
    if (!parent) fail("file/denied", "Google Drive root cannot be created");
    return withPending(state, receiver, "mkdir", logical, async signal => {
      const url = new URL(`${apiBase}/files`);
      url.searchParams.set("supportsAllDrives", "true");
      const response = await http(state, "mkdir", logical, url, {
        method: "POST", headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ name: leafName(logical), parents: [parent.id], mimeType: FOLDER_MIME })
      }, signal);
      const metadata = await responseJson(response, "mkdir", logical);
      state.pathCache.set(logical, metadata);
      return wire({ path: logical, revision: entry(logical, metadata).revision, "mount-revision": null, extensions: {} });
    });
  }

  async function deleteEntry(state, path, optionsValue, mutationValue, receiver) {
    const logical = normalisePath(path);
    requireCapability(state, "delete", "delete", logical);
    if (logical === "/") fail("file/denied", "cannot delete the mounted Google Drive root");
    const options = objectOptions(optionsValue, new Set(["missing-ok"]), "delete");
    const mutation = mutationContext(mutationValue);
    let metadata = null;
    try { metadata = await resolve(state, logical, receiver); } catch (error) { if (error?.code !== "file/not-found") throw error; }
    if (!metadata) {
      if (mutation.expectedRevision) revisionMatches(null, mutation.expectedRevision, logical);
      if (booleanOption(options, "missing-ok", false)) return wire({ path: logical, revision: null, "mount-revision": null, extensions: {} });
      fail("file/not-found", "Google Drive path does not exist", { path: logical });
    }
    revisionMatches(entry(logical, metadata).revision, mutation.expectedRevision, logical);
    return withPending(state, receiver, "delete", logical, async signal => {
      const url = metadataUrl(metadata.id, { supportsAllDrives: "true" });
      const response = await http(state, "delete", logical, url, { method: "PATCH", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ trashed: true }) }, signal);
      await responseJson(response, "delete", logical);
      state.pathCache.delete(logical);
      return wire({ path: logical, revision: null, "mount-revision": null, extensions: {} });
    });
  }

  async function copyEntry(state, source, target, optionsValue, mutationValue, receiver) {
    const sourcePath = normalisePath(source);
    const targetPath = normalisePath(target);
    requireCapability(state, "copy", "copy", sourcePath);
    if (sourcePath === targetPath) fail("file/already-exists", "Google Drive copy source and target are identical");
    const options = objectOptions(optionsValue, new Set(["replace", "parents", "preserve-modified"]), "copy");
    if (booleanOption(options, "preserve-modified", false)) fail("file/unsupported", "Google Drive modified-time preservation is not advertised");
    const mutation = mutationContext(mutationValue);
    const sourceMetadata = await resolve(state, sourcePath, receiver);
    if (sourceMetadata.mimeType === SHORTCUT_MIME) fail("file/unsupported", "Google Drive shortcuts are not followed", { reason: "no-follow" });
    const targetParent = await parentMetadata(state, targetPath, receiver);
    if (!targetParent) fail("file/denied", "Google Drive root cannot be copied over");
    revisionMatches(entry(sourcePath, sourceMetadata).revision, mutation.expectedRevision, sourcePath);
    let targetMetadata = null;
    try { targetMetadata = await resolve(state, targetPath, receiver); } catch (error) { if (error?.code !== "file/not-found") throw error; }
    revisionMatches(targetMetadata ? entry(targetPath, targetMetadata).revision : null, mutation.expectedTargetRevision, targetPath, true);
    if (targetMetadata && !booleanOption(options, "replace", false)) fail("file/already-exists", "Google Drive copy target already exists");
    if (targetMetadata) await deleteEntry(state, targetPath, {}, {}, receiver);
    return withPending(state, receiver, "copy", sourcePath, async signal => {
      const url = new URL(`${apiBase}/files/${encodeURIComponent(sourceMetadata.id)}/copy`);
      url.searchParams.set("supportsAllDrives", "true");
      const response = await http(state, "copy", sourcePath, url, {
        method: "POST", headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ name: leafName(targetPath), parents: [targetParent.id] })
      }, signal);
      const metadata = await responseJson(response, "copy", sourcePath);
      state.pathCache.set(targetPath, metadata);
      return wire({ path: targetPath, revision: entry(targetPath, metadata).revision, "mount-revision": null, extensions: {} });
    });
  }

  async function moveEntry(state, source, target, optionsValue, mutationValue, receiver) {
    const sourcePath = normalisePath(source);
    const targetPath = normalisePath(target);
    requireCapability(state, "move", "move", sourcePath);
    if (sourcePath === "/" || targetPath === "/") fail("file/denied", "cannot move the mounted Google Drive root");
    const options = objectOptions(optionsValue, new Set(["replace", "parents", "atomic"]), "move");
    if (booleanOption(options, "atomic", false) === false) { /* Drive PATCH is the advertised move primitive. */ }
    const mutation = mutationContext(mutationValue);
    const sourceMetadata = await resolve(state, sourcePath, receiver);
    const targetParent = await parentMetadata(state, targetPath, receiver);
    if (!targetParent) fail("file/denied", "Google Drive root cannot be a move target");
    revisionMatches(entry(sourcePath, sourceMetadata).revision, mutation.expectedRevision, sourcePath);
    let targetMetadata = null;
    try { targetMetadata = await resolve(state, targetPath, receiver); } catch (error) { if (error?.code !== "file/not-found") throw error; }
    revisionMatches(targetMetadata ? entry(targetPath, targetMetadata).revision : null, mutation.expectedTargetRevision, targetPath, true);
    if (targetMetadata && !booleanOption(options, "replace", false)) fail("file/already-exists", "Google Drive move target already exists");
    if (targetMetadata) await deleteEntry(state, targetPath, {}, {}, receiver);
    const oldParent = Array.isArray(sourceMetadata.parents) ? sourceMetadata.parents[0] : null;
    return withPending(state, receiver, "move", sourcePath, async signal => {
      const url = metadataUrl(sourceMetadata.id, { addParents: targetParent.id, removeParents: oldParent, supportsAllDrives: "true" });
      const response = await http(state, "move", sourcePath, url, {
        method: "PATCH", headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ name: leafName(targetPath) })
      }, signal);
      const metadata = await responseJson(response, "move", sourcePath);
      state.pathCache.delete(sourcePath);
      state.pathCache.set(targetPath, metadata);
      return wire({ path: targetPath, revision: entry(targetPath, metadata).revision, "mount-revision": null, extensions: {} });
    });
  }

  async function open(optionsValue, receiver) {
    const options = objectOptions(optionsValue, new Set(["display", "read-only?", "operation-timeout-ms", "max-transfer-bytes"]), "Google Drive filesystem");
    const readOnly = booleanOption(options, "read-only?", false);
    const state = {
      id: `google-drive-host-${++nextMount}`,
      readOnly,
      capabilities: capabilitySet(configuredCapabilities, readOnly),
      maxTransferBytes: positiveInteger(options, "max-transfer-bytes", DEFAULT_MAX_TRANSFER_BYTES, 256 * 1024 * 1024),
      display: textOption(options, "display", "Google Drive filesystem"),
      pending: new Map(),
      pageTokens: new Map(),
      pageSequence: 0,
      pathCache: new Map(),
      closed: false,
      rootMetadata: null
    };
    mounts.set(state.id, state);
    try {
      state.rootMetadata = await getMetadata(state, rootId, receiver, "open", "/");
      if (state.rootMetadata.mimeType !== FOLDER_MIME) fail("file/not-directory", "Google Drive configured root is not a folder");
      state.pathCache.set("/", state.rootMetadata);
      const descriptor = {
        kind: "google-drive",
        display: state.display,
        "read-only": readOnly,
        capabilities: [...state.capabilities].sort(),
        revision: state.rootMetadata.headRevisionId ?? null,
        extensions: {
          "provider/root-scoped?": true,
          "provider/transport-verified?": true,
          "provider/route": "hta-wasm",
          "provider/shortcut-policy": "no-follow",
          "provider/workspace-policy": Object.keys(exportMimeTypes).length ? "explicit-export" : "unsupported"
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
      default: fail("file/unsupported", `unsupported Google Drive filesystem operation ${operation}`);
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
    state.pathCache.clear();
    state.pageTokens.clear();
    mounts.delete(String(mountId));
    return null;
  }

  async function closeAll() { await Promise.all([...mounts.keys()].map(id => close(id))); }

  async function describe() {
    return wire({ provider: "google-drive", identity: "hara/filesystem-google-drive", abi: "hta.v1", route: "hta-wasm", capabilities: [...KNOWN_CAPABILITIES].sort() });
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

export { createGoogleDriveHost, normalisePath, plain };
