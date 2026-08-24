import {
  SERVICE,
  DEFAULT_MAX_TRANSFER_BYTES,
  DEFAULT_TIMEOUT_MS,
  DEFAULT_PAGE_LIMIT,
  MAX_PAGE_LIMIT,
  MAX_DAV_ENTRIES,
  KNOWN_CAPABILITIES,
  fail,
  valueName,
  plain,
  booleanOption,
  positiveInteger,
  textOption,
  normaliseLogicalPath,
  directChild,
  safeOptions,
  mutationContext,
  statusFailure,
  requireSuccess,
  entry,
  revisionMatches,
  capabilitySet,
  requireCapability
} from "./common.mjs";
import {
  callHost,
  remoteRequest,
  statEntry,
  optionalStatEntry,
  requireRegularEntry,
  requireRevisionSupport,
  mutationResult,
  ensureParent
} from "./provider-helpers.mjs";

export function createWebdavProvider() {
  const filesystems = new Map();
  let nextFilesystem = 0;

  function state(id) {
    const key = Number(id);
    const value = filesystems.get(key);
    if (!value) fail("file/provider-closed", "unknown or closed WebDAV filesystem", { id });
    return value;
  }

  async function open(optionsValue, context) {
    const options = safeOptions(
      optionsValue,
      new Set(["display", "read-only?", "operation-timeout-ms", "max-transfer-bytes"]),
      "WebDAV filesystem"
    );
    const id = ++nextFilesystem;
    const request = `${id}:open`;
    const display = textOption(options, "display", "WebDAV filesystem");
    const requestedReadOnly = booleanOption(options, "read-only?", false);
    const operationTimeoutMs = positiveInteger(
      options,
      "operation-timeout-ms",
      DEFAULT_TIMEOUT_MS,
      300_000
    );
    const maxTransferBytes = positiveInteger(
      options,
      "max-transfer-bytes",
      DEFAULT_MAX_TRANSFER_BYTES,
      64 * 1024 * 1024
    );
    const opened = await callHost(
      context,
      { hostMount: null },
      "open",
      [request, {
        display,
        "read-only?": requestedReadOnly,
        "operation-timeout-ms": operationTimeoutMs,
        "max-transfer-bytes": maxTransferBytes
      }],
      "open",
      "/"
    );
    const hostMount = opened?.mount;
    let activated = false;
    try {
      if ((typeof hostMount !== "string" && !Number.isSafeInteger(hostMount)) || hostMount === "") {
        fail("file/io", "WebDAV host returned an invalid mount identity");
      }
      const readOnly = requestedReadOnly || opened?.["read-only"] === true;
      const capabilities = capabilitySet(opened?.capabilities ?? ["read", "entries"], readOnly);
      const root = entry(opened?.["root-entry"]);
      if (root.path !== "/" || root.type !== "directory") {
        fail("file/not-directory", "WebDAV mounted root is not a directory");
      }
      const descriptor = Object.freeze({
        kind: "webdav",
        display,
        "read-only": readOnly,
        capabilities: Object.freeze([...capabilities].sort()),
        revision: root.revision,
        extensions: Object.freeze({
          "provider/root-scoped?": true,
          "provider/transport-verified?": true,
          "provider/route": "hta-wasm"
        })
      });
      filesystems.set(id, {
        id,
        hostMount,
        descriptor,
        capabilities,
        readOnly,
        operationTimeoutMs,
        maxTransferBytes,
        requestSequence: 0,
        pageSequence: 0,
        pageTokens: new Map(),
        closed: false,
        context
      });
      activated = true;
      return Object.freeze({ id, descriptor });
    } finally {
      if (!activated && (typeof hostMount === "string" || Number.isSafeInteger(hostMount))) {
        await context.hostCall(SERVICE, "close", [hostMount], { signal: null }).catch(() => {});
      }
    }
  }

  async function stat(id, path, context) {
    return statEntry(context, state(id), path);
  }

  async function read(id, path, context) {
    const filesystem = state(id);
    const logical = normaliseLogicalPath(path);
    requireCapability(filesystem, "read", "read", logical);
    const metadata = await statEntry(context, filesystem, logical);
    if (metadata.type === "directory") fail("file/is-directory", "cannot read a directory", { path: logical });
    if (metadata.type !== "file") fail("file/unsupported", "WebDAV entry is not a regular file", { path: logical });
    if (metadata.size !== null && metadata.size > filesystem.maxTransferBytes) {
      fail("file/quota-exceeded", "WebDAV file exceeds the configured transfer limit", { path: logical });
    }
    const response = requireSuccess(
      await remoteRequest(context, filesystem, "read", logical, { method: "GET", headers: {} }),
      "read",
      logical
    );
    if (!(response.body instanceof Uint8Array)) fail("file/io", "WebDAV host returned a non-byte body");
    if (response.body.byteLength > filesystem.maxTransferBytes) {
      fail("file/quota-exceeded", "WebDAV response exceeds the configured transfer limit", { path: logical });
    }
    return response.body;
  }

  async function write(id, path, bytes, optionsValue, mutationValue, context) {
    const filesystem = state(id);
    const logical = normaliseLogicalPath(path);
    requireCapability(filesystem, "write", "write", logical);
    if (!(bytes instanceof Uint8Array)) fail("file/descriptor-invalid", "write requires exact bytes");
    if (bytes.byteLength > filesystem.maxTransferBytes) {
      fail("file/quota-exceeded", "write exceeds the configured transfer limit", { path: logical });
    }
    const options = safeOptions(optionsValue, new Set(["mode", "parents"]), "write");
    const mode = String(valueName(options.mode ?? "create"));
    if (!new Set(["create", "replace", "append"]).has(mode)) {
      fail("file/descriptor-invalid", `unknown write mode ${mode}`);
    }
    if (mode === "append") fail("file/unsupported", "WebDAV append is not advertised");
    const parents = booleanOption(options, "parents", false);
    const mutation = mutationContext(mutationValue);
    requireRevisionSupport(filesystem, mutation, "write", logical);
    await ensureParent(context, filesystem, logical, parents, "write");
    const existing = await optionalStatEntry(context, filesystem, logical);
    if (mutation.expectedRevision !== null && mutation.expectedRevision !== undefined) {
      revisionMatches(existing?.revision ?? null, mutation.expectedRevision, logical, false);
    }
    if (existing) requireRegularEntry(existing, "write", logical);
    if (mode === "create" && existing) {
      fail("file/already-exists", "WebDAV entry already exists", { operation: "write", path: logical });
    }
    if (mode === "replace" && !existing) {
      fail("file/not-found", "WebDAV entry does not exist", { operation: "write", path: logical });
    }
    const headers = { "Content-Type": "application/octet-stream" };
    if (mutation.expectedRevision) headers["If-Match"] = mutation.expectedRevision;
    else if (mode === "create") headers["If-None-Match"] = "*";
    else headers["If-Match"] = "*";
    const response = requireSuccess(
      await remoteRequest(context, filesystem, "write", logical, {
        method: "PUT",
        headers,
        body: bytes
      }),
      "write",
      logical
    );
    return mutationResult(logical, response);
  }

  async function entriesPage(id, path, requestValue, context) {
    const filesystem = state(id);
    const logical = normaliseLogicalPath(path);
    requireCapability(filesystem, "entries", "entries-page", logical);
    const request = plain(requestValue ?? new Map());
    const limit = positiveInteger(request ?? {}, "limit", DEFAULT_PAGE_LIMIT, MAX_PAGE_LIMIT);
    const token = request?.token ?? null;
    let snapshot;
    let offset;
    if (token !== null) {
      if (typeof token !== "string" || !filesystem.pageTokens.has(token)) {
        fail("file/invalid-page-token", "unknown or expired WebDAV page token", { path: logical });
      }
      const saved = filesystem.pageTokens.get(token);
      filesystem.pageTokens.delete(token);
      if (saved.path !== logical) fail("file/invalid-page-token", "WebDAV page token belongs to another path");
      snapshot = saved.entries;
      offset = saved.offset;
    } else {
      const response = requireSuccess(
        await remoteRequest(context, filesystem, "entries-page", logical, {
          method: "PROPFIND",
          headers: { Depth: "1" }
        }),
        "entries-page",
        logical,
        status => status === 207 || (status >= 200 && status < 300)
      );
      const values = Array.isArray(response.entries) ? response.entries.map(entry) : [];
      const root = values.find(item => item.path === logical);
      if (!root) fail("file/io", "WebDAV PROPFIND omitted the requested directory", { path: logical });
      if (root.type !== "directory") fail("file/not-directory", "WebDAV entry is not a directory", { path: logical });
      snapshot = values
        .filter(item => directChild(logical, item.path))
        .sort((left, right) => left.path < right.path ? -1 : left.path > right.path ? 1 : 0);
      if (snapshot.length > MAX_DAV_ENTRIES) {
        fail("file/quota-exceeded", "WebDAV directory exceeds the entry limit", { path: logical });
      }
      offset = 0;
    }
    const entries = snapshot.slice(offset, offset + limit);
    const nextOffset = offset + entries.length;
    let nextToken = null;
    if (nextOffset < snapshot.length) {
      nextToken = `webdav-page-${filesystem.id}-${++filesystem.pageSequence}`;
      filesystem.pageTokens.set(nextToken, { path: logical, entries: snapshot, offset: nextOffset });
    }
    return Object.freeze({ entries: Object.freeze(entries), "next-token": nextToken });
  }

  async function mkdir(id, path, optionsValue, mutationValue, context) {
    const filesystem = state(id);
    const logical = normaliseLogicalPath(path);
    requireCapability(filesystem, "mkdir", "mkdir", logical);
    const options = safeOptions(optionsValue, new Set(["parents", "exists-ok"]), "mkdir");
    const parents = booleanOption(options, "parents", true);
    const existsOk = booleanOption(options, "exists-ok", true);
    const mutation = mutationContext(mutationValue);
    requireRevisionSupport(filesystem, mutation, "mkdir", logical);
    const existing = await optionalStatEntry(context, filesystem, logical);
    if (mutation.expectedRevision !== null && mutation.expectedRevision !== undefined) {
      revisionMatches(existing?.revision ?? null, mutation.expectedRevision, logical, false);
    }
    if (existing) {
      if (existing.type === "directory" && existsOk) {
        return mutationResult(logical, null, existing.revision);
      }
      fail("file/already-exists", "WebDAV entry already exists", { operation: "mkdir", path: logical });
    }
    await ensureParent(context, filesystem, logical, parents, "mkdir");
    const response = await remoteRequest(context, filesystem, "mkdir", logical, {
      method: "MKCOL",
      headers: mutation.expectedRevision ? { "If-Match": mutation.expectedRevision } : {}
    });
    const status = Number(response.status);
    if (status >= 200 && status < 300) return mutationResult(logical, response);
    if (status === 405 && existsOk) {
      const existing = await statEntry(context, filesystem, logical);
      if (existing.type !== "directory") fail("file/not-directory", "existing WebDAV entry is not a directory", { path: logical });
      revisionMatches(existing.revision, mutation.expectedRevision, logical, false);
      return mutationResult(logical, null, existing.revision);
    }
    if (status === 405) fail("file/already-exists", "WebDAV entry already exists", { path: logical });
    statusFailure(status, "mkdir", logical);
  }

  async function deleteEntry(id, path, optionsValue, mutationValue, context) {
    const filesystem = state(id);
    const logical = normaliseLogicalPath(path);
    requireCapability(filesystem, "delete", "delete", logical);
    if (logical === "/") fail("file/denied", "cannot delete the mounted WebDAV root", { path: logical });
    const options = safeOptions(optionsValue, new Set(["missing-ok"]), "delete");
    const missingOk = booleanOption(options, "missing-ok", false);
    const mutation = mutationContext(mutationValue);
    requireRevisionSupport(filesystem, mutation, "delete", logical);
    const existing = await optionalStatEntry(context, filesystem, logical);
    if (!existing) {
      if (mutation.expectedRevision !== null && mutation.expectedRevision !== undefined) {
        revisionMatches(null, mutation.expectedRevision, logical, false);
      }
      if (missingOk) return mutationResult(logical, null, null);
      fail("file/not-found", "WebDAV entry was not found", { operation: "delete", path: logical });
    }
    revisionMatches(existing.revision, mutation.expectedRevision, logical, false);
    const response = await remoteRequest(context, filesystem, "delete", logical, {
      method: "DELETE",
      headers: mutation.expectedRevision ? { "If-Match": mutation.expectedRevision } : {}
    });
    const status = Number(response.status);
    if (status >= 200 && status < 300) return mutationResult(logical, response, null);
    if (status === 404 && missingOk && !mutation.expectedRevision) return mutationResult(logical, null, null);
    statusFailure(status, "delete", logical);
  }

  async function copyEntry(id, source, target, optionsValue, mutationValue, context) {
    const filesystem = state(id);
    const sourcePath = normaliseLogicalPath(source);
    const targetPath = normaliseLogicalPath(target);
    requireCapability(filesystem, "copy", "copy", sourcePath);
    if (sourcePath === targetPath) {
      fail("file/already-exists", "WebDAV copy source and target are the same path", {
        source: sourcePath,
        target: targetPath
      });
    }
    const options = safeOptions(optionsValue, new Set(["replace", "parents", "preserve-modified"]), "copy");
    if (booleanOption(options, "preserve-modified", false)) {
      fail("file/unsupported", "WebDAV copy does not advertise modified-time preservation");
    }
    const mutation = mutationContext(mutationValue);
    requireRevisionSupport(filesystem, mutation, "copy", sourcePath, targetPath);
    const sourceEntry = await statEntry(context, filesystem, sourcePath);
    requireRegularEntry(sourceEntry, "copy", sourcePath);
    revisionMatches(sourceEntry.revision, mutation.expectedRevision, sourcePath, false);
    await ensureParent(
      context,
      filesystem,
      targetPath,
      booleanOption(options, "parents", false),
      "copy"
    );
    const targetEntry = await optionalStatEntry(context, filesystem, targetPath);
    revisionMatches(targetEntry?.revision ?? null, mutation.expectedTargetRevision, targetPath, true);
    if (targetEntry && !booleanOption(options, "replace", false)) {
      fail("file/already-exists", "WebDAV copy target already exists", {
        source: sourcePath,
        target: targetPath
      });
    }
    if (targetEntry) requireRegularEntry(targetEntry, "replace", targetPath);
    const headers = {
      Overwrite: booleanOption(options, "replace", false) ? "T" : "F"
    };
    if (mutation.expectedRevision) headers["If-Match"] = mutation.expectedRevision;
    const response = requireSuccess(
      await remoteRequest(context, filesystem, "copy", sourcePath, {
        method: "COPY",
        target: targetPath,
        headers
      }),
      "copy",
      sourcePath
    );
    return mutationResult(targetPath, response);
  }

  async function moveEntry(id, source, target, optionsValue, mutationValue, context) {
    const filesystem = state(id);
    const sourcePath = normaliseLogicalPath(source);
    const targetPath = normaliseLogicalPath(target);
    requireCapability(filesystem, "move", "move", sourcePath);
    if (sourcePath === "/" || targetPath === "/") {
      fail("file/denied", "cannot move the mounted WebDAV root", {
        source: sourcePath,
        target: targetPath
      });
    }
    if (targetPath.startsWith(`${sourcePath}/`)) {
      fail("file/invalid-path", "cannot move a WebDAV directory beneath itself", {
        source: sourcePath,
        target: targetPath
      });
    }
    const options = safeOptions(optionsValue, new Set(["replace", "parents", "atomic"]), "move");
    if (booleanOption(options, "atomic", false)) {
      fail("file/unsupported", "WebDAV move does not advertise atomic replacement");
    }
    const mutation = mutationContext(mutationValue);
    requireRevisionSupport(filesystem, mutation, "move", sourcePath, targetPath);
    if (sourcePath === targetPath) {
      const existing = await statEntry(context, filesystem, sourcePath);
      revisionMatches(existing.revision, mutation.expectedRevision, sourcePath, false);
      revisionMatches(existing.revision, mutation.expectedTargetRevision, targetPath, true);
      return mutationResult(targetPath, null, existing.revision);
    }
    const sourceEntry = await statEntry(context, filesystem, sourcePath);
    revisionMatches(sourceEntry.revision, mutation.expectedRevision, sourcePath, false);
    await ensureParent(
      context,
      filesystem,
      targetPath,
      booleanOption(options, "parents", false),
      "move"
    );
    const targetEntry = await optionalStatEntry(context, filesystem, targetPath);
    revisionMatches(targetEntry?.revision ?? null, mutation.expectedTargetRevision, targetPath, true);
    if (targetEntry && !booleanOption(options, "replace", false)) {
      fail("file/already-exists", "WebDAV move target already exists", {
        source: sourcePath,
        target: targetPath
      });
    }
    const headers = {
      Overwrite: booleanOption(options, "replace", false) ? "T" : "F"
    };
    if (mutation.expectedRevision) headers["If-Match"] = mutation.expectedRevision;
    const response = requireSuccess(
      await remoteRequest(context, filesystem, "move", sourcePath, {
        method: "MOVE",
        target: targetPath,
        headers
      }),
      "move",
      sourcePath
    );
    return mutationResult(targetPath, response);
  }

  async function close(id, context) {
    const filesystem = filesystems.get(Number(id));
    if (!filesystem) return null;
    filesystems.delete(Number(id));
    filesystem.closed = true;
    filesystem.pageTokens.clear();
    await (context ?? filesystem.context).hostCall(
      SERVICE,
      "close",
      [filesystem.hostMount],
      { signal: null }
    );
    return null;
  }

  async function closeAll() {
    const values = [...filesystems.keys()];
    await Promise.all(values.map(id => close(id).catch(() => {})));
  }

  async function call(_environment, operationValue, argsValue = [], context = {}) {
    const operation = String(valueName(operationValue));
    const args = Array.isArray(argsValue) ? argsValue : [];
    switch (operation) {
      case "describe":
        return Object.freeze({
          provider: "webdav",
          identity: "hara/filesystem-webdav",
          abi: "hta.v1",
          route: "hta-wasm",
          capabilities: Object.freeze([...KNOWN_CAPABILITIES].sort())
        });
      case "open": return open(args[0], context);
      case "descriptor": return state(args[0]).descriptor;
      case "stat": return stat(args[0], args[1], context);
      case "read": return read(args[0], args[1], context);
      case "write": return write(args[0], args[1], args[2], args[3], args[4], context);
      case "entries-page": return entriesPage(args[0], args[1], args[2], context);
      case "mkdir": return mkdir(args[0], args[1], args[2], args[3], context);
      case "delete": return deleteEntry(args[0], args[1], args[2], args[3], context);
      case "copy": return copyEntry(args[0], args[1], args[2], args[3], args[4], context);
      case "move": return moveEntry(args[0], args[1], args[2], args[3], args[4], context);
      case "close": return close(args[0], context);
      default: fail("file/unsupported", `unsupported WebDAV operation ${operation}`);
    }
  }

  return Object.freeze({ call, closeAll });
}
