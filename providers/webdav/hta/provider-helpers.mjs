import {
  SERVICE,
  fail,
  plain,
  normaliseLogicalPath,
  logicalParent,
  mutationRequired,
  requestHeaders,
  statusFailure,
  requireSuccess,
  normaliseHostError,
  entry,
  requireCapability
} from "./common.mjs";

function requestId(state) {
  state.requestSequence += 1;
  return `${state.id}:${state.requestSequence}`;
}

async function callHost(context, state, method, args, operation, path) {
  if (!context?.hostCall) fail("file/host-unavailable", "WebDAV host capability is unavailable");
  const id = method === "request" || method === "open" ? args[method === "open" ? 0 : 1] : null;
  let cancelled = false;
  const onAbort = () => {
    cancelled = true;
    context.hostCall(
      SERVICE,
      "cancel",
      [state?.hostMount ?? null, id],
      { signal: null }
    ).catch(() => {});
  };
  if (context.signal?.aborted) onAbort();
  else context.signal?.addEventListener?.("abort", onAbort, { once: true });
  try {
    if (cancelled) fail("file/cancelled", "WebDAV operation was cancelled", { operation, path });
    return plain(await context.hostCall(SERVICE, method, args, { signal: null }));
  } catch (error) {
    if (cancelled || context.signal?.aborted) {
      fail("file/cancelled", "WebDAV operation was cancelled", { operation, path });
    }
    normaliseHostError(error, operation, path);
  } finally {
    context.signal?.removeEventListener?.("abort", onAbort);
  }
}

async function remoteRequest(context, state, operation, path, request) {
  const id = requestId(state);
  return callHost(
    context,
    state,
    "request",
    [state.hostMount, id, { ...request, path }],
    operation,
    path
  );
}

async function statEntry(context, state, path) {
  requireCapability(state, "read", "stat", path);
  const logical = normaliseLogicalPath(path);
  const response = requireSuccess(
    await remoteRequest(context, state, "stat", logical, {
      method: "PROPFIND",
      headers: { Depth: "0" }
    }),
    "stat",
    logical,
    status => status === 207 || (status >= 200 && status < 300)
  );
  const entries = Array.isArray(response.entries) ? response.entries.map(entry) : [];
  const exact = entries.find(item => item.path === logical);
  if (!exact) fail("file/io", "WebDAV PROPFIND omitted the requested entry", { path: logical });
  return exact;
}

async function optionalStatEntry(context, state, path) {
  try {
    return await statEntry(context, state, path);
  } catch (error) {
    if (error?.code === "file/not-found") return null;
    throw error;
  }
}

function requireRegularEntry(value, operation, path) {
  if (value.type === "directory") {
    fail("file/is-directory", `cannot ${operation} a directory`, { operation, path });
  }
  if (value.type !== "file") {
    fail("file/unsupported", `cannot ${operation} a non-regular WebDAV entry`, {
      operation,
      path,
      type: value.type
    });
  }
}

function requireRevisionSupport(state, mutation, operation, path, target = null) {
  if (mutationRequired(mutation) && !state.capabilities.has("revision-check")) {
    fail("file/unsupported", "WebDAV revision checks are unavailable", {
      operation,
      path,
      target,
      reason: "revision-check-unavailable"
    });
  }
}

function mutationResult(path, response = null, revision = undefined) {
  const headers = requestHeaders(response);
  return Object.freeze({
    path: normaliseLogicalPath(path),
    revision: revision === undefined ? headers.etag ?? null : revision,
    "mount-revision": null,
    extensions: Object.freeze({})
  });
}

async function ensureParentDirectories(context, state, path) {
  const parent = logicalParent(path);
  if (!parent || parent === "/") return;
  const segments = parent.slice(1).split("/");
  let current = "";
  for (const segment of segments) {
    current += `/${segment}`;
    const existing = await optionalStatEntry(context, state, current);
    if (existing) {
      if (existing.type !== "directory") {
        fail("file/not-directory", "WebDAV parent path is not a directory", { path: current });
      }
      continue;
    }
    const response = await remoteRequest(context, state, "mkdir", current, {
      method: "MKCOL",
      headers: {}
    });
    const status = Number(response.status);
    if (status >= 200 && status < 300) continue;
    if (status === 405) {
      const existing = await statEntry(context, state, current);
      if (existing.type !== "directory") {
        fail("file/not-directory", "WebDAV parent path is not a directory", { path: current });
      }
      continue;
    }
    statusFailure(status, "mkdir", current);
  }
}

async function ensureParent(context, state, path, parents, operation) {
  const parent = logicalParent(path);
  if (!parent || parent === "/") return;
  if (parents) {
    await ensureParentDirectories(context, state, path);
    return;
  }
  const existing = await optionalStatEntry(context, state, parent);
  if (!existing) {
    fail("file/not-found", "WebDAV parent directory does not exist", {
      operation,
      path,
      parent
    });
  }
  if (existing.type !== "directory") {
    fail("file/not-directory", "WebDAV parent path is not a directory", {
      operation,
      path,
      parent
    });
  }
}

export {
  callHost,
  remoteRequest,
  statEntry,
  optionalStatEntry,
  requireRegularEntry,
  requireRevisionSupport,
  mutationResult,
  ensureParentDirectories,
  ensureParent
};
