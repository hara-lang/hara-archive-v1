import {
  SERVICE,
  DEFAULT_MAX_TRANSFER_BYTES,
  DEFAULT_TIMEOUT_MS,
  MAX_DAV_ENTRIES,
  KNOWN_CAPABILITIES,
  SAFE_REQUEST_HEADERS,
  SAFE_RESPONSE_HEADERS,
  fail,
  plain,
  wire,
  positiveInteger,
  normaliseLogicalPath,
  capabilitySet,
  statusFailure
} from "./common.mjs";
import { createWebdavProvider } from "./provider.mjs";

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

function canonicalRootUrl(value, allowInsecureLoopback) {
  let url;
  try {
    url = new URL(value);
  } catch {
    fail("file/descriptor-invalid", "WebDAV rootUrl must be an absolute URL");
  }
  if (url.username || url.password || url.search || url.hash) {
    fail("file/descriptor-invalid", "WebDAV rootUrl cannot contain credentials, query, or fragment");
  }
  const loopback = new Set(["127.0.0.1", "[::1]", "localhost"]).has(url.hostname);
  if (url.protocol !== "https:" && !(allowInsecureLoopback && loopback && url.protocol === "http:")) {
    fail("file/descriptor-invalid", "WebDAV transport requires HTTPS outside explicit loopback fixtures");
  }
  const decodedSegments = url.pathname.split("/").filter(Boolean).map(segment => {
    let decoded;
    try {
      decoded = decodeURIComponent(segment);
    } catch {
      fail("file/descriptor-invalid", "WebDAV rootUrl contains invalid escaping");
    }
    if (!decoded || decoded === "." || decoded === ".." || decoded.includes("/") || decoded.includes("\\")) {
      fail("file/descriptor-invalid", "WebDAV rootUrl contains an unsafe path segment");
    }
    return decoded;
  });
  url.pathname = `/${decodedSegments.map(encodeURIComponent).join("/")}${decodedSegments.length ? "/" : ""}`;
  return url;
}

function logicalUrl(root, path) {
  const logical = normaliseLogicalPath(path);
  const result = new URL(root.toString());
  const suffix = logical === "/"
    ? ""
    : logical.slice(1).split("/").map(encodeURIComponent).join("/");
  result.pathname = `${root.pathname}${suffix}`;
  return result;
}

function hrefLogicalPath(root, href) {
  let url;
  try {
    url = new URL(href, root);
  } catch {
    fail("file/outside-root", "WebDAV response contains an invalid href");
  }
  if (url.origin !== root.origin || url.search || url.hash) {
    fail("file/outside-root", "WebDAV response href escapes the mounted origin");
  }
  const rootPath = root.pathname;
  const rootWithoutSlash = rootPath === "/" ? "/" : rootPath.slice(0, -1);
  if (url.pathname === rootWithoutSlash || url.pathname === rootPath) return "/";
  if (!url.pathname.startsWith(rootPath)) {
    fail("file/outside-root", "WebDAV response href escapes the mounted root");
  }
  const suffix = url.pathname.slice(rootPath.length);
  const segments = suffix.split("/").filter(Boolean).map(segment => {
    let decoded;
    try {
      decoded = decodeURIComponent(segment);
    } catch {
      fail("file/outside-root", "WebDAV response href has invalid escaping");
    }
    if (!decoded || decoded === "." || decoded === ".." || decoded.includes("/") || decoded.includes("\\") || decoded.includes("\0")) {
      fail("file/outside-root", "WebDAV response href contains an unsafe segment");
    }
    return decoded;
  });
  return normaliseLogicalPath(`/${segments.join("/")}`);
}

function decodeXml(value) {
  return value.replace(/&(#x[0-9a-f]+|#[0-9]+|amp|lt|gt|quot|apos);/gi, match => {
    const token = match.slice(1, -1).toLowerCase();
    if (token === "amp") return "&";
    if (token === "lt") return "<";
    if (token === "gt") return ">";
    if (token === "quot") return '"';
    if (token === "apos") return "'";
    const code = token.startsWith("#x")
      ? Number.parseInt(token.slice(2), 16)
      : Number.parseInt(token.slice(1), 10);
    if (!Number.isSafeInteger(code) || code < 0 || code > 0x10ffff) {
      fail("file/io", "WebDAV XML contains an invalid character reference");
    }
    return String.fromCodePoint(code);
  });
}

function localElement(source, name) {
  const expression = new RegExp(
    `<(?:[A-Za-z_][\\w.-]*:)?${name}\\b[^>]*>([\\s\\S]*?)<\\/(?:[A-Za-z_][\\w.-]*:)?${name}\\s*>`,
    "i"
  );
  return expression.exec(source)?.[1] ?? null;
}

function hasLocalElement(source, name) {
  return new RegExp(`<(?:[A-Za-z_][\\w.-]*:)?${name}\\b`, "i").test(source);
}

function parseDavEntries(root, bytes) {
  const source = new TextDecoder("utf-8", { fatal: true }).decode(bytes);
  if (/<!DOCTYPE|<!ENTITY/i.test(source)) fail("file/io", "WebDAV XML declarations are not allowed");
  const responseExpression = /<(?:[A-Za-z_][\w.-]*:)?response\b[^>]*>([\s\S]*?)<\/(?:[A-Za-z_][\w.-]*:)?response\s*>/gi;
  const entries = [];
  const seen = new Set();
  for (const match of source.matchAll(responseExpression)) {
    if (entries.length >= MAX_DAV_ENTRIES) fail("file/quota-exceeded", "WebDAV response contains too many entries");
    const response = match[1];
    const hrefValue = localElement(response, "href");
    if (hrefValue === null) fail("file/io", "WebDAV response entry has no href");
    const path = hrefLogicalPath(root, decodeXml(hrefValue.trim()));
    if (seen.has(path)) fail("file/io", "WebDAV response contains duplicate canonical entries", { path });
    seen.add(path);
    const propstats = [...response.matchAll(/<(?:[A-Za-z_][\w.-]*:)?propstat\b[^>]*>([\s\S]*?)<\/(?:[A-Za-z_][\w.-]*:)?propstat\s*>/gi)];
    let properties = response;
    if (propstats.length) {
      const successful = propstats.find(item => /\b2\d\d\b/.test(localElement(item[1], "status") ?? ""));
      if (!successful) continue;
      properties = localElement(successful[1], "prop") ?? successful[1];
    }
    const type = hasLocalElement(localElement(properties, "resourcetype") ?? "", "collection")
      ? "directory"
      : "file";
    const lengthValue = localElement(properties, "getcontentlength");
    const size = type === "directory" || lengthValue === null ? null : Number(lengthValue.trim());
    if (size !== null && (!Number.isSafeInteger(size) || size < 0)) {
      fail("file/io", "WebDAV response contains an invalid content length", { path });
    }
    const modifiedValue = localElement(properties, "getlastmodified");
    const modified = modifiedValue === null ? null : Date.parse(decodeXml(modifiedValue.trim()));
    if (modified !== null && !Number.isSafeInteger(modified)) {
      fail("file/io", "WebDAV response contains an invalid modified time", { path });
    }
    const revisionValue = localElement(properties, "getetag");
    entries.push({
      path,
      type,
      size,
      "modified-at": modified,
      id: null,
      revision: revisionValue === null ? null : decodeXml(revisionValue.trim()),
      capabilities: null,
      extensions: {}
    });
  }
  if (!entries.length) fail("file/io", "WebDAV multistatus contains no entries");
  return entries;
}

function trustedHeaders(value) {
  const source = typeof value === "function" ? value() : value;
  if (source === undefined) return {};
  if (!source || typeof source !== "object" || Array.isArray(source)) {
    fail("file/descriptor-invalid", "WebDAV trusted headers must be an object or supplier");
  }
  const result = {};
  for (const [name, item] of Object.entries(source)) {
    if (typeof item !== "string" || /[\r\n]/.test(name) || /[\r\n]/.test(item)) {
      fail("file/descriptor-invalid", "WebDAV trusted headers contain invalid data");
    }
    result[name] = item;
  }
  return result;
}

function responseHeaderMap(headers) {
  const result = {};
  for (const name of SAFE_RESPONSE_HEADERS) {
    const value = headers.get(name);
    if (value !== null) result[name] = value;
  }
  return result;
}

function safeProviderRequest(value) {
  const request = plain(value);
  const method = String(request?.method ?? "").toUpperCase();
  if (!new Set(["PROPFIND", "GET", "PUT", "MKCOL", "DELETE", "COPY", "MOVE"]).has(method)) {
    fail("file/unsupported", `unsupported WebDAV HTTP method ${method}`);
  }
  const headers = plain(request?.headers ?? new Map());
  const safe = {};
  for (const [name, item] of Object.entries(headers)) {
    const lower = name.toLowerCase();
    if (!SAFE_REQUEST_HEADERS.has(lower) || typeof item !== "string" || /[\r\n]/.test(item)) {
      fail("file/descriptor-invalid", `unsafe WebDAV request header ${name}`);
    }
    safe[name] = item;
  }
  const body = request?.body ?? null;
  if (body !== null && !(body instanceof Uint8Array)) {
    fail("file/descriptor-invalid", "WebDAV request body must be bytes");
  }
  return {
    method,
    headers: safe,
    body,
    target: request?.target === undefined ? null : normaliseLogicalPath(request.target),
    path: normaliseLogicalPath(request?.path)
  };
}

async function readBoundedBody(response, maximum) {
  if (!response.body?.getReader) {
    const bytes = new Uint8Array(await response.arrayBuffer());
    if (bytes.byteLength > maximum) {
      fail("file/quota-exceeded", "WebDAV response exceeds the configured transfer limit");
    }
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
      if (total > maximum) {
        await reader.cancel("WebDAV response exceeds transfer limit").catch(() => {});
        fail("file/quota-exceeded", "WebDAV response exceeds the configured transfer limit");
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

export function createWebdavFetchHost(options = {}) {
  const root = canonicalRootUrl(options.rootUrl, options.allowInsecureLoopback === true);
  const request = options.fetch ?? globalThis.fetch?.bind(globalThis);
  if (typeof request !== "function") fail("file/host-unavailable", "fetch is unavailable");
  const timeoutMs = positiveInteger(options, "operationTimeoutMs", DEFAULT_TIMEOUT_MS, 300_000);
  const maxResponseBytes = positiveInteger(
    options,
    "maxResponseBytes",
    DEFAULT_MAX_TRANSFER_BYTES,
    64 * 1024 * 1024
  );
  const configuredReadOnly = options.readOnly === true;
  const configuredCapabilities = capabilitySet(
    options.capabilities ?? ["read", "entries"],
    configuredReadOnly
  );
  const mounts = new Map();
  const pending = new Map();
  const registeredContexts = new WeakSet();
  let nextMount = 0;

  function ownerFromCall(receiver) {
    const owner = receiver?.kernelContext ?? null;
    return owner && (typeof owner === "object" || typeof owner === "function") ? owner : null;
  }

  function requireOwner(record, owner) {
    if (record?.owner && owner && record.owner !== owner) {
      fail("file/permission-denied", "WebDAV host authority belongs to another HTA context");
    }
  }

  function registerOwner(owner) {
    if (!owner || registeredContexts.has(owner)) return;
    registeredContexts.add(owner);
    installContextCloseHook(owner, () => closeContext(owner));
  }

  function mount(id) {
    const value = mounts.get(id);
    if (!value || value.closed) fail("file/provider-closed", "unknown or closed WebDAV host mount");
    return value;
  }

  async function perform(mountId, id, providerRequest, owner = null) {
    if (typeof id !== "string" || !id.length || pending.has(id)) {
      fail("file/descriptor-invalid", "WebDAV request identity must be unique");
    }
    const value = safeProviderRequest(providerRequest);
    if (value.body?.byteLength > maxResponseBytes) {
      fail("file/quota-exceeded", "WebDAV request exceeds the configured transfer limit");
    }
    if (mountId !== null) {
      const mounted = mount(mountId);
      requireOwner(mounted, owner);
      owner = mounted.owner ?? owner;
    }
    registerOwner(owner);
    const controller = new AbortController();
    const timer = setTimeout(() => controller.abort(new Error("timeout")), timeoutMs);
    pending.set(id, { controller, mountId, owner });
    try {
      const url = logicalUrl(root, value.path);
      const headers = { ...trustedHeaders(options.headers), ...value.headers };
      if (value.target !== null) headers.Destination = logicalUrl(root, value.target).toString();
      const response = await request(url, {
        method: value.method,
        headers,
        body: value.body,
        redirect: "manual",
        credentials: "omit",
        cache: "no-store",
        signal: controller.signal
      });
      if (response.redirected || (response.status >= 300 && response.status < 400)) {
        fail("file/outside-root", "WebDAV redirects are not permitted", { status: response.status });
      }
      const declared = response.headers.get("content-length");
      if (declared !== null && (!/^\d+$/.test(declared) || Number(declared) > maxResponseBytes)) {
        fail("file/quota-exceeded", "WebDAV response exceeds the configured transfer limit");
      }
      let body = new Uint8Array();
      if (value.method === "GET" || value.method === "PROPFIND") {
        body = await readBoundedBody(response, maxResponseBytes);
      }
      const result = {
        status: response.status,
        headers: responseHeaderMap(response.headers),
        body
      };
      if (value.method === "PROPFIND" && (response.status === 207 || response.ok)) {
        result.entries = parseDavEntries(root, body);
      }
      return result;
    } catch (error) {
      if (controller.signal.aborted) {
        const timeout = controller.signal.reason?.message === "timeout";
        fail(timeout ? "file/timeout" : "file/cancelled", timeout ? "WebDAV request timed out" : "WebDAV request was cancelled");
      }
      if (error?.code?.startsWith?.("file/")) throw error;
      fail("file/io", "WebDAV host transport failed", undefined, true);
    } finally {
      clearTimeout(timer);
      pending.delete(id);
    }
  }

  async function open(id, openOptionsValue) {
    const owner = ownerFromCall(this);
    registerOwner(owner);
    const openOptions = plain(openOptionsValue ?? new Map());
    const readOnly = configuredReadOnly || openOptions?.["read-only?"] === true;
    const response = await perform(null, id, {
      method: "PROPFIND",
      path: "/",
      headers: { Depth: "0" }
    }, owner);
    if (response.status !== 207 && !response.ok && !(response.status >= 200 && response.status < 300)) {
      statusFailure(response.status, "open", "/");
    }
    const rootEntry = response.entries?.find(item => item.path === "/");
    if (!rootEntry) fail("file/io", "WebDAV root PROPFIND omitted the root entry");
    if (rootEntry.type !== "directory") fail("file/not-directory", "WebDAV root is not a directory");
    const mountId = `webdav-host-${++nextMount}`;
    const capabilities = capabilitySet([...configuredCapabilities], readOnly);
    mounts.set(mountId, { id: mountId, readOnly, capabilities, closed: false, owner });
    return wire({
      mount: mountId,
      "read-only": readOnly,
      capabilities: [...capabilities].sort(),
      "root-entry": rootEntry
    });
  }

  async function invoke(mountId, id, requestValue) {
    return wire(await perform(mountId, id, requestValue, ownerFromCall(this)));
  }

  async function cancel(mountId, id) {
    const owner = ownerFromCall(this);
    if (mountId !== null && mounts.has(mountId)) {
      const mounted = mount(mountId);
      requireOwner(mounted, owner);
    }
    const active = pending.get(id);
    if (!active) return false;
    requireOwner(active, owner);
    if (mountId !== null && active.mountId !== mountId) {
      fail("file/permission-denied", "WebDAV cancellation belongs to another mount");
    }
    active.controller.abort(new Error("cancelled"));
    return true;
  }

  async function closeMount(mountId) {
    const value = mounts.get(mountId);
    if (!value) return null;
    mounts.delete(mountId);
    value.closed = true;
    for (const active of pending.values()) {
      if (active.mountId === mountId) active.controller.abort(new Error("closed"));
    }
    return null;
  }

  async function close(mountId) {
    const value = mounts.get(mountId);
    if (!value) return null;
    requireOwner(value, ownerFromCall(this));
    return closeMount(mountId);
  }

  async function closeContext(owner) {
    for (const active of pending.values()) {
      if (active.owner === owner) active.controller.abort(new Error("closed"));
    }
    await Promise.all(
      [...mounts.values()]
        .filter(value => value.owner === owner)
        .map(value => closeMount(value.id))
    );
  }

  async function closeAll() {
    for (const active of pending.values()) active.controller.abort(new Error("closed"));
    await Promise.all([...mounts.keys()].map(closeMount));
  }

  return Object.freeze({
    hostCalls: Object.freeze({
      [`${SERVICE}/open`]: open,
      [`${SERVICE}/request`]: invoke,
      [`${SERVICE}/cancel`]: cancel,
      [`${SERVICE}/close`]: close
    }),
    closeAll
  });
}

/**
 * Bridges the compatibility WebDAV transport host to the rich HTA provider
 * contract. The semantic provider remains the single implementation of the
 * filesystem operations; this adapter only translates the rich host boundary
 * into the raw DAV requests owned by the trusted host.
 */
export function createWebdavWasmHost(options = {}) {
  const transport = createWebdavFetchHost(options);
  const provider = createWebdavProvider();
  const mounts = new Map();
  const idleSignal = new AbortController().signal;

  function receiverWithSignal(receiver, signal) {
    return { ...(receiver ?? {}), signal };
  }

  function providerContext(receiver) {
    const signal = receiver?.signal ?? idleSignal;
    return {
      signal,
      hostCall(service, method, args = []) {
        const key = `${service}/${method}`;
        const handler = transport.hostCalls[key];
        if (typeof handler !== "function") {
          return Promise.reject(new Error(`file/host-unavailable: ${key}`));
        }
        return Promise.resolve(handler.call(receiverWithSignal(receiver, signal), ...args));
      }
    };
  }

  async function describe() {
    return wire({
      provider: "webdav",
      identity: "hara/filesystem-webdav",
      abi: "hta.v1",
      route: "hta-wasm",
      capabilities: [...KNOWN_CAPABILITIES].sort()
    });
  }

  async function open(optionsValue, receiver) {
    const opened = await provider.call(
      "browser",
      "open",
      [plain(optionsValue ?? new Map())],
      providerContext(receiver)
    );
    mounts.set(String(opened.id), opened.id);
    return wire({ mount: opened.id, descriptor: opened.descriptor });
  }

  async function request(mount, operationValue, argsValue = [], receiver) {
    const result = await provider.call(
      "browser",
      String(operationValue),
      [mount, ...(Array.isArray(argsValue) ? argsValue : [])],
      providerContext(receiver)
    );
    return wire(result);
  }

  async function cancel(mount, id, receiver) {
    const hostMount = mounts.get(String(mount));
    if (hostMount === undefined) return false;
    const handler = transport.hostCalls[`${SERVICE}/cancel`];
    if (typeof handler !== "function") return false;
    return handler.call(receiverWithSignal(receiver, receiver?.signal ?? idleSignal), hostMount, id);
  }

  async function close(mount, receiver) {
    const result = await provider.call(
      "browser",
      "close",
      [mount],
      providerContext(receiver)
    );
    mounts.delete(String(mount));
    return result;
  }

  async function closeAll() {
    await provider.closeAll();
    mounts.clear();
    await transport.closeAll();
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
