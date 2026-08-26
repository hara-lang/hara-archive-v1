import {
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
} from "./common.mjs";

const DEFAULT_ENDPOINT = "https://api.github.com/";
const API_VERSION = "2026-03-10";
const CONTEXT_CLOSE_HOOKS = Symbol.for("hara.hta.close-hooks");

function installContextCloseHook(context, hook) {
  if (!context || typeof context.close !== "function") return;
  let state = context[CONTEXT_CLOSE_HOOKS];
  if (!state) {
    const originalClose = context.close.bind(context);
    const hooks = new Set();
    let closePromise = null;
    state = Object.freeze({ hooks });
    Object.defineProperty(context, CONTEXT_CLOSE_HOOKS, { value: state });
    Object.defineProperty(context, "close", {
      configurable: true,
      value(...args) {
        if (!closePromise) {
          closePromise = (async () => {
            const settled = await Promise.allSettled([...hooks].map(cleanup => cleanup()));
            const result = await originalClose(...args);
            const rejected = settled.find(item => item.status === "rejected");
            if (rejected) throw rejected.reason;
            return result;
          })();
        }
        return closePromise;
      }
    });
  }
  state.hooks.add(hook);
}

function repositoryName(value) {
  if (typeof value !== "string" || !/^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/.test(value)) {
    fail("file/descriptor-invalid", "GitHub repository must be owner/name");
  }
  return value;
}

function referenceName(value) {
  if (typeof value !== "string" || !value.trim()) {
    fail("file/descriptor-invalid", "GitHub ref is required");
  }
  let result = value.trim();
  if (result.startsWith("refs/")) result = result.slice("refs/".length);
  if (
    result.includes("\0")
    || result.includes("\\")
    || result.includes("..")
    || result.startsWith("/")
    || result.endsWith("/")
    || result.split("/").some(segment => !segment || segment === ".")
  ) {
    fail("file/descriptor-invalid", "GitHub ref is malformed");
  }
  if (/^[0-9a-fA-F]{7,64}$/.test(result)) return result.toLowerCase();
  if (!result.startsWith("heads/") && !result.startsWith("tags/")) {
    fail("file/descriptor-invalid", "GitHub ref must be heads/*, tags/*, or a commit SHA");
  }
  return result;
}

function sha(value, label = "GitHub object SHA") {
  if (typeof value !== "string" || !/^[0-9a-fA-F]{7,64}$/.test(value)) {
    fail("file/io", `${label} is malformed`);
  }
  return value.toLowerCase();
}

function canonicalEndpoint(value, allowInsecureLoopback) {
  let url;
  try {
    url = new URL(value ?? DEFAULT_ENDPOINT);
  } catch {
    fail("file/descriptor-invalid", "GitHub API endpoint must be an absolute URL");
  }
  const loopback = new Set(["127.0.0.1", "[::1]", "localhost"]).has(url.hostname);
  if (url.protocol !== "https:" && !(allowInsecureLoopback && loopback && url.protocol === "http:")) {
    fail("file/descriptor-invalid", "GitHub transport requires HTTPS outside explicit loopback fixtures");
  }
  if (url.username || url.password || url.search || url.hash) {
    fail("file/descriptor-invalid", "GitHub API endpoint cannot contain credentials, query, or fragment");
  }
  url.pathname = `${url.pathname.replace(/\/+$/, "")}/`;
  return url;
}

function encodePath(value) {
  return value.split("/").map(encodeURIComponent).join("/");
}

function repositoryPath(repository, suffix) {
  return `/repos/${encodePath(repository)}${suffix}`;
}

function mountRoot(value) {
  const result = normaliseLogicalPath(value ?? "/");
  if (result.includes("..")) fail("file/invalid-path", "GitHub mount root is unsafe");
  return result;
}

function safeTreePath(value) {
  if (
    typeof value !== "string"
    || !value.length
    || value.startsWith("/")
    || value.endsWith("/")
    || value.includes("\0")
    || value.includes("\\")
  ) {
    fail("file/io", "GitHub tree contains a malformed path");
  }
  const parts = value.split("/");
  if (parts.some(part => !part || part === "." || part === ".." || part.includes(":"))) {
    fail("file/io", "GitHub tree contains an unsafe path");
  }
  if (parts.length > MAX_TREE_DEPTH) fail("file/unsupported", "GitHub tree nesting exceeds the provider limit");
  return value;
}

function bytesBase64(bytes) {
  let binary = "";
  const chunkSize = 0x8000;
  for (let offset = 0; offset < bytes.length; offset += chunkSize) {
    binary += String.fromCharCode(...bytes.subarray(offset, offset + chunkSize));
  }
  return btoa(binary);
}

function optionMode(value, fallback) {
  const mode = value === undefined || value === null ? fallback : String(valueName(value));
  if (mode !== "read-only" && mode !== "commit") {
    fail("file/descriptor-invalid", "GitHub filesystem mode must be read-only or commit");
  }
  return mode;
}

function objectType(entry) {
  if (entry.type === "tree" || entry.mode === "040000") return "directory";
  if (entry.mode === "120000") return "symlink";
  if (entry.type === "commit" || entry.mode === "160000") return "other";
  if (entry.type === "blob") return "file";
  fail("file/io", "GitHub returned an unknown tree entry type");
}

function entryMap(node) {
  return wire({
    path: node.path,
    name: node.name,
    type: node.type,
    size: node.type === "directory" ? null : node.size ?? null,
    "modified-at": null,
    id: node.sha,
    revision: node.sha,
    capabilities: null,
    extensions: { "provider/mode": node.mode }
  });
}

function mutationResult(path, revision, mountRevision) {
  return wire({
    path,
    revision: revision ?? null,
    "mount-revision": mountRevision ?? null,
    extensions: {}
  });
}

function buildIndex(tree, root) {
  const entries = new Map();
  for (const raw of tree.entries) {
    const path = safeTreePath(raw.path);
    if (entries.has(path)) fail("file/io", "GitHub tree contains duplicate paths");
    entries.set(path, {
      path,
      mode: String(raw.mode ?? "100644"),
      type: String(raw.type ?? ""),
      sha: sha(raw.sha, "Git tree object SHA"),
      size: raw.size === null || raw.size === undefined ? null : Number(raw.size)
    });
  }
  if (entries.size > MAX_TREE_ENTRIES) {
    fail("file/unsupported", "GitHub tree exceeds the provider entry limit");
  }

  const rootRepositoryPath = root === "/" ? "" : root.slice(1);
  const rootEntry = rootRepositoryPath ? entries.get(rootRepositoryPath) : null;
  const hasDescendant = !rootRepositoryPath
    || [...entries.keys()].some(path => path.startsWith(`${rootRepositoryPath}/`));
  if (rootRepositoryPath && !rootEntry && !hasDescendant) {
    fail("file/not-found", "GitHub mount root does not exist", { reason: "root-not-found" });
  }
  if (rootEntry && objectType(rootEntry) !== "directory") {
    fail("file/not-directory", "GitHub mount root is not a directory", { reason: "root-not-tree" });
  }

  const projected = new Map();
  projected.set("/", {
    path: "/",
    name: "/",
    type: "directory",
    mode: "040000",
    sha: rootEntry?.sha ?? tree.sha,
    size: null,
    repositoryPath: rootRepositoryPath
  });

  function relativePath(repository) {
    if (!rootRepositoryPath) return repository;
    if (repository === rootRepositoryPath) return "";
    return repository.startsWith(`${rootRepositoryPath}/`)
      ? repository.slice(rootRepositoryPath.length + 1)
      : null;
  }

  const sorted = [...entries.values()].sort((left, right) => left.path.localeCompare(right.path));
  for (const raw of sorted) {
    const relative = relativePath(raw.path);
    if (relative === null || relative === "") continue;
    const parts = relative.split("/");
    for (let index = 1; index < parts.length; index += 1) {
      const logical = `/${parts.slice(0, index).join("/")}`;
      if (projected.has(logical)) continue;
      const repository = rootRepositoryPath
        ? `${rootRepositoryPath}/${parts.slice(0, index).join("/")}`
        : parts.slice(0, index).join("/");
      const ancestor = entries.get(repository);
      if (ancestor && objectType(ancestor) !== "directory") {
        fail("file/not-directory", "GitHub tree has a non-directory ancestor");
      }
      projected.set(logical, {
        path: logical,
        name: parts[index - 1],
        type: "directory",
        mode: ancestor?.mode ?? "040000",
        sha: ancestor?.sha ?? null,
        size: null,
        repositoryPath: repository
      });
    }
    const logical = `/${relative}`;
    const type = objectType(raw);
    const node = {
      path: logical,
      name: parts.at(-1),
      type,
      mode: raw.mode,
      sha: raw.sha,
      size: Number.isSafeInteger(raw.size) && raw.size >= 0 ? raw.size : null,
      repositoryPath: raw.path
    };
    const previous = projected.get(logical);
    if (previous && (previous.type !== node.type || previous.sha !== node.sha)) {
      fail("file/io", "GitHub tree projection collides");
    }
    projected.set(logical, node);
  }

  return {
    root,
    repositoryRoot: rootRepositoryPath,
    find(path) {
      return projected.get(normaliseLogicalPath(path));
    },
    children(path) {
      const logical = normaliseLogicalPath(path);
      const parent = projected.get(logical);
      if (!parent) fail("file/not-found", "GitHub path does not exist", { path: logical });
      if (parent.type !== "directory") fail("file/not-directory", "path is not a directory", { path: logical });
      return [...projected.values()]
        .filter(node => node.path !== logical && directChild(logical, node.path))
        .sort((left, right) => left.path.localeCompare(right.path));
    },
    descendants(path) {
      const logical = normaliseLogicalPath(path);
      const prefix = logical === "/" ? "/" : `${logical}/`;
      return [...projected.values()]
        .filter(node => node.path !== logical && node.path.startsWith(prefix))
        .sort((left, right) => left.path.localeCompare(right.path));
    },
    repositoryPath(path) {
      const logical = normaliseLogicalPath(path);
      const relative = logical === "/" ? "" : logical.slice(1);
      return rootRepositoryPath ? (relative ? `${rootRepositoryPath}/${relative}` : rootRepositoryPath) : relative;
    },
    parentExists(path) {
      const parent = logicalParent(path);
      const node = parent === null ? null : projected.get(parent);
      return Boolean(node && node.type === "directory");
    }
  };
}

function statusFailure(status, operation, path, body = "") {
  const data = { operation, path, status };
  if (status === 401) fail("file/authentication-failed", "GitHub authentication failed", data);
  if (status === 403) {
    if (/rate|limit/i.test(body)) fail("file/rate-limited", "GitHub API rate limit reached", data, true);
    fail("file/permission-denied", "GitHub denied the operation", data);
  }
  if (status === 404) fail("file/not-found", "GitHub object was not found", data);
  if (status === 409 || status === 422) fail("file/conflict", "GitHub rejected the repository update", data, true);
  if (status === 429) fail("file/rate-limited", "GitHub API rate limit reached", data, true);
  if (status >= 500) fail("file/io", "GitHub service failed", data, true);
  fail("file/io", `unexpected GitHub response status ${status}`, data);
}

export function createGithubFetchHost(options = {}) {
  const repository = repositoryName(options.repository);
  const configuredRef = referenceName(options.ref ?? "heads/main");
  const endpoint = canonicalEndpoint(options.apiBaseUrl, options.allowInsecureLoopback === true);
  const fetchRequest = options.fetch ?? globalThis.fetch?.bind(globalThis);
  if (typeof fetchRequest !== "function") fail("file/host-unavailable", "fetch is unavailable");
  const timeoutMs = positiveInteger(options, "operationTimeoutMs", DEFAULT_TIMEOUT_MS, 300_000);
  const maxTransferBytes = positiveInteger(
    options,
    "maxTransferBytes",
    DEFAULT_MAX_TRANSFER_BYTES,
    256 * 1024 * 1024
  );
  const configuredReadOnly = options.readOnly === true;
  const configuredCapabilities = capabilitySet(
    options.capabilities ?? ["read", "write", "entries", "delete", "copy", "move", "revision-check"],
    configuredReadOnly
  );
  const configuredToken = options.token;
  const tokenProvider = options.tokenProvider;
  const mounts = new Map();
  const pending = new Map();
  const registeredContexts = new WeakSet();
  let nextMount = 0;
  let nextRequest = 0;

  function ownerFromCall(receiver) {
    const owner = receiver?.kernelContext ?? null;
    return owner && (typeof owner === "object" || typeof owner === "function") ? owner : null;
  }

  function registerOwner(owner) {
    if (!owner || registeredContexts.has(owner)) return;
    registeredContexts.add(owner);
    installContextCloseHook(owner, () => closeContext(owner));
  }

  function requireOwner(record, owner) {
    if (record?.owner && owner && record.owner !== owner) {
      fail("file/permission-denied", "GitHub host authority belongs to another HTA context");
    }
  }

  function mount(id) {
    const key = String(id);
    const value = mounts.get(key);
    if (!value || value.closed) fail("file/provider-closed", "unknown or closed GitHub filesystem", { id });
    return value;
  }

  function token() {
    let value = typeof tokenProvider === "function" ? tokenProvider() : configuredToken;
    if (value && typeof value.then === "function") fail("file/authentication-failed", "GitHub token provider must be synchronous");
    if (value !== undefined && value !== null && (typeof value !== "string" || !value.trim())) {
      fail("file/authentication-failed", "GitHub credential is unavailable");
    }
    return value?.trim() || null;
  }

  function requestIdentity(receiver, mountId, explicit = undefined) {
    const task = receiver?.task === undefined ? null : receiver.task;
    const id = explicit ?? (task === null ? `github-request-${++nextRequest}` : String(task));
    return { id: String(id), task: task === null ? String(id) : String(task), mountId: mountId === null ? null : String(mountId) };
  }

  async function withPending(receiver, mountId, explicit, operation, path, work) {
    const identity = requestIdentity(receiver, mountId, explicit);
    const key = `${identity.mountId ?? "open"}:${identity.id}`;
    if (pending.has(key)) fail("file/descriptor-invalid", "GitHub request identity is already active");
    const controller = new AbortController();
    let timedOut = false;
    const timer = setTimeout(() => {
      timedOut = true;
      controller.abort(new Error("timeout"));
    }, timeoutMs);
    const abort = () => controller.abort(new Error("cancelled"));
    const outerSignal = receiver?.signal;
    if (outerSignal?.aborted) abort();
    else outerSignal?.addEventListener?.("abort", abort, { once: true });
    pending.set(key, {
      key,
      id: identity.id,
      task: identity.task,
      mountId: identity.mountId,
      owner: ownerFromCall(receiver),
      controller
    });
    try {
      if (controller.signal.aborted) {
        fail(timedOut ? "file/timeout" : "file/cancelled", timedOut ? "GitHub operation timed out" : "GitHub operation was cancelled", { operation, path }, timedOut);
      }
      return await work(controller.signal);
    } catch (error) {
      if (controller.signal.aborted) {
        fail(timedOut ? "file/timeout" : "file/cancelled", timedOut ? "GitHub operation timed out" : "GitHub operation was cancelled", { operation, path }, timedOut);
      }
      normaliseHostError(error, operation, path);
    } finally {
      clearTimeout(timer);
      outerSignal?.removeEventListener?.("abort", abort);
      pending.delete(key);
    }
  }

  async function api(path, init = {}, signal, operation, logicalPath = null) {
    if (signal?.aborted) fail("file/cancelled", "GitHub operation was cancelled", { operation, path: logicalPath });
    const headers = {
      Accept: "application/vnd.github+json",
      "X-GitHub-Api-Version": API_VERSION,
      ...(init.headers ?? {})
    };
    const accessToken = token();
    if (accessToken) headers.Authorization = `Bearer ${accessToken}`;
    let response;
    try {
      const url = new URL(path.replace(/^\//, ""), endpoint);
      response = await fetchRequest(url, {
        ...init,
        headers,
        redirect: "manual",
        credentials: "omit",
        cache: "no-store",
        signal
      });
    } catch (error) {
      if (signal?.aborted || error?.name === "AbortError") {
        fail("file/cancelled", "GitHub operation was cancelled", { operation, path: logicalPath });
      }
      fail("file/io", "GitHub transport failed", { operation, path: logicalPath }, true);
    }
    if (response.redirected || (response.status >= 300 && response.status < 400)) {
      fail("file/io", "GitHub redirects are not permitted", { operation, path: logicalPath });
    }
    return response;
  }

  async function responseBytes(response) {
    const declared = response.headers?.get?.("content-length");
    if (declared !== null && declared !== undefined && (!/^\d+$/.test(declared) || Number(declared) > maxTransferBytes)) {
      fail("file/quota-exceeded", "GitHub response exceeds the configured transfer limit");
    }
    if (!response.body?.getReader) {
      const bytes = new Uint8Array(await response.arrayBuffer());
      if (bytes.byteLength > maxTransferBytes) fail("file/quota-exceeded", "GitHub response exceeds the configured transfer limit");
      return bytes;
    }
    const reader = response.body.getReader();
    const chunks = [];
    let total = 0;
    try {
      while (true) {
        const { done, value } = await reader.read();
        if (done) break;
        const bytes = value instanceof Uint8Array ? value : new Uint8Array(value);
        total += bytes.byteLength;
        if (total > maxTransferBytes) {
          await reader.cancel("GitHub response exceeds transfer limit").catch(() => {});
          fail("file/quota-exceeded", "GitHub response exceeds the configured transfer limit");
        }
        chunks.push(bytes);
      }
    } finally {
      reader.releaseLock?.();
    }
    const output = new Uint8Array(total);
    let offset = 0;
    for (const chunk of chunks) {
      output.set(chunk, offset);
      offset += chunk.byteLength;
    }
    return output;
  }

  async function json(response, operation, path) {
    const bytes = await responseBytes(response);
    if (!response.ok) statusFailure(response.status, operation, path, new TextDecoder().decode(bytes).slice(0, 256));
    try {
      return JSON.parse(new TextDecoder().decode(bytes));
    } catch {
      fail("file/io", "GitHub returned malformed JSON", { operation, path });
    }
  }

  async function resolveRevision(repositoryValue, reference, signal) {
    const referenceValue = referenceName(reference);
    let commit = referenceValue;
    if (!/^[0-9a-f]{7,64}$/.test(commit)) {
      const response = await api(
        repositoryPath(repositoryValue, `/git/ref/${encodePath(referenceValue)}`),
        {},
        signal,
        "open",
        "/"
      );
      const value = await json(response, "open", "/");
      commit = sha(value?.object?.sha, "GitHub reference SHA");
    }
    const response = await api(
      repositoryPath(repositoryValue, `/git/commits/${encodeURIComponent(commit)}`),
      {},
      signal,
      "open",
      "/"
    );
    const value = await json(response, "open", "/");
    return { commit: sha(value?.sha ?? commit, "GitHub commit SHA"), tree: sha(value?.tree?.sha, "GitHub tree SHA") };
  }

  async function readTree(repositoryValue, treeSha, signal, operation = "read", path = null) {
    const response = await api(
      repositoryPath(repositoryValue, `/git/trees/${encodeURIComponent(sha(treeSha, "GitHub tree SHA"))}?recursive=1`),
      {},
      signal,
      operation,
      path
    );
    const value = await json(response, operation, path);
    if (value?.truncated) fail("file/unsupported", "GitHub returned a truncated tree", { reason: "tree-truncated" }, true);
    if (!Array.isArray(value?.tree)) fail("file/io", "GitHub returned no tree entries", { operation, path });
    return { sha: sha(value.sha ?? treeSha, "GitHub tree SHA"), entries: value.tree };
  }

  async function loadSnapshot(state, signal, operation, path) {
    if (state.readOnly && state.snapshot) return state.snapshot;
    const revision = await resolveRevision(state.repository, state.reference, signal);
    const tree = await readTree(state.repository, revision.tree, signal, operation, path);
    const snapshot = { revision: revision.commit, treeSha: tree.sha, index: buildIndex(tree, state.root) };
    if (state.readOnly) state.snapshot = snapshot;
    return snapshot;
  }

  function requireNode(snapshot, path, operation, target = null) {
    const node = snapshot.index.find(path);
    if (!node) fail("file/not-found", "GitHub path does not exist", { operation, path, target });
    return node;
  }

  function requireParent(index, path, parents, operation, source = null) {
    let parent = logicalParent(path);
    while (parent !== null) {
      const node = index.find(parent);
      if (node) {
        if (node.type !== "directory") fail("file/not-directory", "path ancestor is not a directory", { operation, path: source ?? path, target: source ? path : null });
        return;
      }
      if (!parents) fail("file/not-found", "GitHub parent directory does not exist", { operation, path: source ?? path, target: source ? path : null });
      parent = logicalParent(parent);
    }
  }

  function checkExpected(node, expected, operation, path, target = null) {
    revisionMatches(node?.sha ?? null, expected, path, target !== null);
    if (expected !== null && expected !== undefined && !node) {
      fail("file/conflict", "GitHub entry revision does not match", { operation, path, target, reason: "revision-missing" }, true);
    }
  }

  function transferChanges(index, sourceNode, sourcePath, targetPath, removeSource) {
    const changes = [];
    if (sourceNode.type !== "directory") {
      changes.push({ path: index.repositoryPath(targetPath), mode: sourceNode.mode, type: sourceNode.type === "other" ? "commit" : "blob", sha: sourceNode.sha });
    } else if (sourceNode.sha) {
      changes.push({ path: index.repositoryPath(targetPath), mode: "040000", type: "tree", sha: sourceNode.sha });
    } else {
      const descendants = index.descendants(sourcePath).filter(node => node.type !== "directory");
      if (!descendants.length) fail("file/unsupported", "GitHub cannot copy an empty directory", { reason: "empty-directory-unavailable" });
      for (const node of descendants) {
        const suffix = node.path.slice(sourcePath.length);
        changes.push({ path: index.repositoryPath(`${targetPath}${suffix}`), mode: node.mode, type: node.type === "other" ? "commit" : "blob", sha: node.sha });
      }
    }
    if (removeSource) changes.push({ path: index.repositoryPath(sourcePath), sha: null });
    return changes;
  }

  async function createCommit(state, operation, path, target, snapshot, changes, resultRevision, signal) {
    const treeResponse = await api(
      repositoryPath(state.repository, "/git/trees"),
      {
        method: "POST",
        headers: { "Content-Type": "application/vnd.github+json" },
        body: JSON.stringify({
          base_tree: snapshot.treeSha,
          tree: changes.map(change => change.sha === null
            ? { path: change.path, sha: null }
            : { path: change.path, mode: change.mode, type: change.type, sha: change.sha })
        })
      },
      signal,
      operation,
      path
    );
    const tree = await json(treeResponse, operation, path);
    const treeSha = sha(tree?.sha, "GitHub created tree SHA");
    const message = `${state.commitMessagePrefix}: ${operation} ${target === null ? path : `${path} -> ${target}`}`;
    const commitResponse = await api(
      repositoryPath(state.repository, "/git/commits"),
      {
        method: "POST",
        headers: { "Content-Type": "application/vnd.github+json" },
        body: JSON.stringify({ message, tree: treeSha, parents: [snapshot.revision] })
      },
      signal,
      operation,
      path
    );
    const commit = await json(commitResponse, operation, path);
    const commitSha = sha(commit?.sha, "GitHub created commit SHA");
    const refResponse = await api(
      repositoryPath(state.repository, `/git/ref/${encodePath(state.reference)}`),
      {},
      signal,
      operation,
      path
    );
    const current = await json(refResponse, operation, path);
    const currentSha = sha(current?.object?.sha, "GitHub reference SHA");
    if (currentSha !== snapshot.revision) {
      fail("file/conflict", "GitHub reference moved before update", { reason: "reference-moved" }, true);
    }
    const updateResponse = await api(
      repositoryPath(state.repository, `/git/refs/${encodePath(state.reference)}`),
      {
        method: "PATCH",
        headers: { "Content-Type": "application/vnd.github+json" },
        body: JSON.stringify({ sha: commitSha, force: false })
      },
      signal,
      operation,
      path
    );
    await json(updateResponse, operation, path);
    state.revision = commitSha;
    state.descriptor.set("revision", commitSha);
    return mutationResult(target ?? path, resultRevision, commitSha);
  }

  async function open(optionsValue, receiver) {
    const optionsValuePlain = safeOptions(
      optionsValue,
      new Set(["display", "root", "mode", "commit-message-prefix", "operation-timeout-ms", "max-transfer-bytes", "repository", "ref"]),
      "GitHub filesystem"
    );
    const requestedRepository = optionsValuePlain.repository === undefined
      ? repository
      : repositoryName(optionsValuePlain.repository);
    if (requestedRepository !== repository) fail("file/permission-denied", "GitHub repository is outside the trusted host scope");
    const requestedReference = optionsValuePlain.ref === undefined
      ? configuredRef
      : referenceName(optionsValuePlain.ref);
    const root = mountRoot(optionsValuePlain.root ?? "/");
    const mode = optionMode(optionsValuePlain.mode, configuredReadOnly ? "read-only" : "read-only");
    if (mode === "commit" && configuredReadOnly) fail("file/permission-denied", "GitHub host is configured read-only");
    if (mode === "commit" && !requestedReference.startsWith("heads/")) {
      fail("file/permission-denied", "writable GitHub mounts require a heads/* ref");
    }
    const readOnly = mode === "read-only";
    const capabilities = capabilitySet([...configuredCapabilities], readOnly);
    const display = textOption(optionsValuePlain, "display", `${requestedRepository}@${requestedReference.replace(/^heads\//, "")}`);
    const prefix = textOption(optionsValuePlain, "commit-message-prefix", "hara filesystem");
    if (prefix.length > 120 || /[\r\n]/.test(prefix)) fail("file/descriptor-invalid", "invalid GitHub commit message prefix");
    registerOwner(ownerFromCall(receiver));
    return withPending(receiver, null, undefined, "open", "/", async signal => {
      const revision = await resolveRevision(requestedRepository, requestedReference, signal);
      const tree = await readTree(requestedRepository, revision.tree, signal, "open", "/");
      const snapshot = { revision: revision.commit, treeSha: tree.sha, index: buildIndex(tree, root) };
      if (mode === "commit" && !token()) fail("file/authentication-failed", "GitHub writes require an authenticated host");
      const id = `github-host-${++nextMount}`;
      const descriptor = wire({
        kind: "github",
        display,
        "read-only": readOnly,
        capabilities: [...capabilities].sort(),
        revision: revision.commit,
        extensions: {
          "provider/repository": requestedRepository,
          "provider/ref": requestedReference,
          "provider/root": root,
          "provider/route": "hta-wasm",
          "provider/root-scoped?": true,
          "provider/transport-verified?": true
        }
      });
      const state = {
        id,
        owner: ownerFromCall(receiver),
        repository: requestedRepository,
        reference: requestedReference,
        root,
        mode,
        readOnly,
        display,
        commitMessagePrefix: prefix,
        capabilities,
        descriptor,
        revision: revision.commit,
        snapshot: readOnly ? snapshot : null,
        closed: false
      };
      mounts.set(id, state);
      return wire({ id, descriptor });
    });
  }

  async function stat(id, path, receiver) {
    const state = mount(id);
    const logical = normaliseLogicalPath(path);
    requireCapability(state, "read", "stat", logical);
    return withPending(receiver, state.id, undefined, "stat", logical, async signal => {
      const snapshot = await loadSnapshot(state, signal, "stat", logical);
      return entryMap(requireNode(snapshot, logical, "stat"));
    });
  }

  async function read(id, path, receiver) {
    const state = mount(id);
    const logical = normaliseLogicalPath(path);
    requireCapability(state, "read", "read", logical);
    return withPending(receiver, state.id, undefined, "read", logical, async signal => {
      const snapshot = await loadSnapshot(state, signal, "read", logical);
      const node = requireNode(snapshot, logical, "read");
      if (node.type === "directory") fail("file/is-directory", "path is a directory", { path: logical });
      if (node.type !== "file") fail("file/unsupported", "GitHub links and gitlinks are not followed", { path: logical, reason: "no-follow" });
      const response = await api(
        repositoryPath(state.repository, `/git/blobs/${encodeURIComponent(node.sha)}`),
        { headers: { Accept: "application/vnd.github.raw+json" } },
        signal,
        "read",
        logical
      );
      if (!response.ok) {
        const bytes = await responseBytes(response);
        statusFailure(response.status, "read", logical, new TextDecoder().decode(bytes).slice(0, 256));
      }
      return responseBytes(response);
    });
  }

  async function write(id, path, bytesValue, optionsValue, mutationValue, receiver) {
    const state = mount(id);
    const logical = normaliseLogicalPath(path);
    requireCapability(state, "write", "write", logical);
    if (!(bytesValue instanceof Uint8Array)) fail("file/descriptor-invalid", "write requires exact bytes", { path: logical });
    if (bytesValue.byteLength > maxTransferBytes) fail("file/quota-exceeded", "write exceeds the configured transfer limit", { path: logical });
    const options = safeOptions(optionsValue, new Set(["mode", "parents"]), "write");
    const mode = String(options.mode ?? "create");
    if (!["create", "replace", "append"].includes(mode)) fail("file/descriptor-invalid", `unknown write mode ${mode}`);
    if (mode === "append") fail("file/unsupported", "GitHub append is not advertised");
    const parents = booleanOption(options, "parents", false);
    const mutation = mutationContext(mutationValue);
    return withPending(receiver, state.id, undefined, "write", logical, async signal => {
      const snapshot = await loadSnapshot(state, signal, "write", logical);
      const existing = snapshot.index.find(logical);
      checkExpected(existing, mutation.expectedRevision, "write", logical);
      if (mode === "create" && existing) fail("file/already-exists", "path already exists", { path: logical });
      if (mode === "replace" && !existing) {
        // GitHub's tree API can create a replacement when the path is absent;
        // preserve the JVM provider's replace semantics for compatibility.
      }
      if (existing?.type === "directory") fail("file/is-directory", "path is a directory", { path: logical });
      if (existing && existing.type !== "file") fail("file/unsupported", "GitHub links and gitlinks are not followed", { path: logical, reason: "no-follow" });
      requireParent(snapshot.index, logical, parents, "write");
      const blobResponse = await api(
        repositoryPath(state.repository, "/git/blobs"),
        {
          method: "POST",
          headers: { "Content-Type": "application/vnd.github+json" },
          body: JSON.stringify({ content: bytesBase64(bytesValue), encoding: "base64" })
        },
        signal,
        "write",
        logical
      );
      const blob = await json(blobResponse, "write", logical);
      const blobSha = sha(blob?.sha, "GitHub created blob SHA");
      return createCommit(state, "write", logical, null, snapshot, [{
        path: snapshot.index.repositoryPath(logical),
        mode: existing?.mode === "100755" ? "100755" : "100644",
        type: "blob",
        sha: blobSha
      }], blobSha, signal);
    });
  }

  async function entriesPage(id, path, requestValue, receiver) {
    const state = mount(id);
    const logical = normaliseLogicalPath(path);
    requireCapability(state, "entries", "entries", logical);
    const requestOptions = safeOptions(requestValue, new Set(["limit", "token"]), "entries page request");
    const limit = positiveInteger(requestOptions, "limit", DEFAULT_PAGE_LIMIT, MAX_PAGE_LIMIT);
    return withPending(receiver, state.id, undefined, "entries", logical, async signal => {
      const snapshot = await loadSnapshot(state, signal, "entries", logical);
      const directory = requireNode(snapshot, logical, "entries");
      if (directory.type !== "directory") fail("file/not-directory", "path is not a directory", { path: logical });
      const children = snapshot.index.children(logical);
      const offset = decodeCursor(requestOptions.token ?? null, snapshot.revision);
      if (offset > children.length) fail("file/invalid-page-token", "invalid GitHub filesystem page token");
      const entries = children.slice(offset, offset + limit).map(entryMap);
      const nextOffset = offset + entries.length;
      return wire({
        entries,
        "next-token": nextOffset < children.length ? encodeCursor(snapshot.revision, nextOffset) : null
      });
    });
  }

  async function mkdir(id, path, optionsValue, mutationValue, receiver) {
    const state = mount(id);
    const logical = normaliseLogicalPath(path);
    requireCapability(state, "write", "mkdir", logical);
    const options = safeOptions(optionsValue, new Set(["parents", "exists-ok"]), "mkdir");
    const mutation = mutationContext(mutationValue);
    return withPending(receiver, state.id, undefined, "mkdir", logical, async signal => {
      const snapshot = await loadSnapshot(state, signal, "mkdir", logical);
      const existing = snapshot.index.find(logical);
      checkExpected(existing, mutation.expectedRevision, "mkdir", logical);
      if (existing?.type === "directory" && booleanOption(options, "exists-ok", true)) {
        return mutationResult(logical, existing.sha, snapshot.revision);
      }
      if (existing) fail("file/already-exists", "path already exists", { path: logical });
      fail("file/unsupported", "GitHub cannot create empty directories", { reason: "empty-directory-unavailable" });
    });
  }

  async function deleteEntry(id, path, optionsValue, mutationValue, receiver) {
    const state = mount(id);
    const logical = normaliseLogicalPath(path);
    requireCapability(state, "delete", "delete", logical);
    if (logical === "/") fail("file/denied", "cannot delete the mounted GitHub root", { path: logical });
    const options = safeOptions(optionsValue, new Set(["missing-ok"]), "delete");
    const mutation = mutationContext(mutationValue);
    return withPending(receiver, state.id, undefined, "delete", logical, async signal => {
      const snapshot = await loadSnapshot(state, signal, "delete", logical);
      const existing = snapshot.index.find(logical);
      if (!existing) {
        if (mutation.expectedRevision !== null && mutation.expectedRevision !== undefined) checkExpected(null, mutation.expectedRevision, "delete", logical);
        if (booleanOption(options, "missing-ok", false)) return mutationResult(logical, null, snapshot.revision);
        fail("file/not-found", "GitHub path does not exist", { path: logical });
      }
      checkExpected(existing, mutation.expectedRevision, "delete", logical);
      if (existing.type === "directory" && snapshot.index.descendants(logical).length) {
        fail("file/directory-not-empty", "directory is not empty", { path: logical });
      }
      return createCommit(state, "delete", logical, null, snapshot, [{ path: snapshot.index.repositoryPath(logical), sha: null }], null, signal);
    });
  }

  async function copy(id, source, target, optionsValue, mutationValue, receiver) {
    return transfer(id, source, target, optionsValue, mutationValue, receiver, false);
  }

  async function move(id, source, target, optionsValue, mutationValue, receiver) {
    return transfer(id, source, target, optionsValue, mutationValue, receiver, true);
  }

  async function transfer(id, source, target, optionsValue, mutationValue, receiver, removeSource) {
    const operation = removeSource ? "move" : "copy";
    const state = mount(id);
    const logicalSource = normaliseLogicalPath(source);
    const logicalTarget = normaliseLogicalPath(target);
    requireCapability(state, removeSource ? "move" : "copy", operation, logicalSource);
    if (logicalSource === "/" || logicalTarget === "/") fail("file/denied", `cannot ${operation} the mounted GitHub root`);
    if (logicalSource === logicalTarget) {
      if (!removeSource) fail("file/already-exists", "source and target are the same path");
      return withPending(receiver, state.id, undefined, operation, logicalSource, async signal => {
        const snapshot = await loadSnapshot(state, signal, operation, logicalSource);
        const node = requireNode(snapshot, logicalSource, operation, logicalTarget);
        const mutation = mutationContext(mutationValue);
        checkExpected(node, mutation.expectedRevision, operation, logicalSource, logicalTarget);
        checkExpected(node, mutation.expectedTargetRevision, operation, logicalTarget, logicalTarget);
        return mutationResult(logicalTarget, node.sha, snapshot.revision);
      });
    }
    if (logicalTarget.startsWith(`${logicalSource}/`)) fail("file/invalid-path", `cannot ${operation} a directory beneath itself`);
    const options = safeOptions(optionsValue, new Set(["replace", "parents", "preserve-modified", "atomic"]), operation);
    if (removeSource && booleanOption(options, "atomic", false)) fail("file/unsupported", "GitHub cannot provide an atomic ref move");
    if (!removeSource && booleanOption(options, "preserve-modified", false)) fail("file/unsupported", "GitHub cannot preserve modified times");
    const mutation = mutationContext(mutationValue);
    return withPending(receiver, state.id, undefined, operation, logicalSource, async signal => {
      const snapshot = await loadSnapshot(state, signal, operation, logicalSource);
      const sourceNode = requireNode(snapshot, logicalSource, operation, logicalTarget);
      if (sourceNode.type === "symlink" || sourceNode.type === "other") fail("file/unsupported", "GitHub links and gitlinks are not followed", { reason: "no-follow" });
      checkExpected(sourceNode, mutation.expectedRevision, operation, logicalSource, logicalTarget);
      requireParent(snapshot.index, logicalTarget, booleanOption(options, "parents", false), operation, logicalSource);
      const targetNode = snapshot.index.find(logicalTarget);
      checkExpected(targetNode, mutation.expectedTargetRevision, operation, logicalTarget, logicalTarget);
      if (targetNode && !booleanOption(options, "replace", false)) fail("file/already-exists", "target already exists", { target: logicalTarget });
      if (sourceNode.type === "directory" && snapshot.index.descendants(logicalSource).length === 0 && !sourceNode.sha) {
        fail("file/unsupported", "GitHub cannot transfer an empty directory", { reason: "empty-directory-unavailable" });
      }
      const changes = transferChanges(snapshot.index, sourceNode, logicalSource, logicalTarget, removeSource);
      return createCommit(state, operation, logicalSource, logicalTarget, snapshot, changes, sourceNode.sha, signal);
    });
  }

  async function request(filesystemId, operationValue, argumentsValue, receiver) {
    const state = mount(filesystemId);
    const operation = String(operationValue?.name ?? operationValue);
    const args = Array.isArray(argumentsValue) ? argumentsValue : [];
    switch (operation) {
      case "stat": return stat(filesystemId, args[0], receiver);
      case "read": return read(filesystemId, args[0], receiver);
      case "write": return write(filesystemId, args[0], args[1], args[2], args[3], receiver);
      case "entries-page": return entriesPage(filesystemId, args[0], args[1], receiver);
      case "mkdir": return mkdir(filesystemId, args[0], args[1], args[2], receiver);
      case "delete": return deleteEntry(filesystemId, args[0], args[1], args[2], receiver);
      case "copy": return copy(filesystemId, args[0], args[1], args[2], args[3], receiver);
      case "move": return move(filesystemId, args[0], args[1], args[2], args[3], receiver);
      default: fail("file/unsupported", `unsupported GitHub filesystem operation ${operation}`);
    }
  }

  async function cancel(mountId, id, receiver) {
    const owner = ownerFromCall(receiver);
    const targetMount = mountId === null || mountId === undefined ? null : String(mountId);
    const target = String(id);
    for (const active of pending.values()) {
      if (targetMount !== null && active.mountId !== targetMount) continue;
      if (active.id !== target && active.task !== target) continue;
      requireOwner(active, owner);
      active.controller.abort(new Error("cancelled"));
      return true;
    }
    return false;
  }

  async function closeMount(id, receiver) {
    const value = mounts.get(String(id));
    if (!value) return null;
    requireOwner(value, ownerFromCall(receiver));
    mounts.delete(String(id));
    value.closed = true;
    for (const active of pending.values()) {
      if (active.mountId === String(id)) active.controller.abort(new Error("closed"));
    }
    return null;
  }

  async function close(id, receiver) {
    return closeMount(id, receiver);
  }

  async function closeContext(owner) {
    for (const active of pending.values()) {
      if (active.owner === owner) active.controller.abort(new Error("closed"));
    }
    await Promise.all([...mounts.values()]
      .filter(value => value.owner === owner)
      .map(value => closeMount(value.id, { kernelContext: owner })));
  }

  async function closeAll() {
    for (const active of pending.values()) active.controller.abort(new Error("closed"));
    await Promise.all([...mounts.keys()].map(id => closeMount(id, null)));
  }

  async function describe() {
    return wire({
      provider: "github",
      identity: "hara/filesystem-github",
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
