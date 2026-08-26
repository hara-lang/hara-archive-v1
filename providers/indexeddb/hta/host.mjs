import {
  createIndexedDbFilesystemHost
} from "../../../core/rust/web/host/indexeddb-filesystem-host.js";

const SERVICE = "filesystem.indexeddb";
const KNOWN_CAPABILITIES = Object.freeze([
  "read",
  "write",
  "entries",
  "mkdir",
  "delete",
  "copy",
  "move",
  "append",
  "revision-check",
  "transactions"
]);

function fail(code, message) {
  const error = new Error(`${code}: ${message}`);
  error.code = code;
  throw error;
}

function valueName(value) {
  return value && typeof value === "object" && typeof value.name === "string"
    ? value.name
    : value;
}

function plain(value) {
  if (value instanceof Uint8Array || value === null || value === undefined) return value;
  if (value && (value.constructor?.name === "HtaKeyword" || value.constructor?.name === "HtaSymbol")) {
    return value.name;
  }
  if (Array.isArray(value)) return value.map(plain);
  if (value instanceof Map) {
    const result = {};
    for (const [key, item] of value) result[String(valueName(key))] = plain(item);
    return result;
  }
  if (typeof value === "object") {
    const result = {};
    for (const [key, item] of Object.entries(value)) result[key] = plain(item);
    return result;
  }
  return value;
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

function option(value, ...names) {
  const options = plain(value ?? {});
  for (const name of names) {
    if (options[name] !== undefined) return options[name];
  }
  return undefined;
}

function openConfiguration(value) {
  const options = plain(value ?? {});
  if (option(options, "read-only?", "readOnly") === true) {
    fail("file/unsupported", "IndexedDB does not expose a read-only mount");
  }
  const namespace = option(options, "namespace", "key") ?? "workspace";
  if (typeof namespace !== "string" || !namespace.trim()) {
    fail("file/descriptor-invalid", "IndexedDB namespace must be a nonblank string");
  }
  const configuration = {
    provider: "indexeddb",
    namespace,
    database: option(options, "database"),
    version: option(options, "version"),
    quotaBytes: option(options, "quotaBytes", "quota-bytes"),
    maxFileBytes: option(options, "maxFileBytes", "max-file-bytes")
  };
  for (const key of Object.keys(configuration)) {
    if (configuration[key] === undefined) delete configuration[key];
  }
  return configuration;
}

function options(value) {
  const input = plain(value ?? {});
  return {
    ...input,
    parents: input.parents ?? input["parents?"] ?? false,
    replace: input.replace ?? false,
    existsOk: input.existsOk ?? input["exists-ok"] ?? input["exists-ok?"] ?? true,
    missingOk: input.missingOk ?? input["missing-ok"] ?? input["missing-ok?"] ?? false,
    preserveModified: input.preserveModified ?? input["preserve-modified"] ?? false,
    atomic: input.atomic ?? false
  };
}

function mutation(value) {
  const input = plain(value ?? {});
  return {
    expectedRevision: input.expectedRevision ?? input["expected-revision"] ?? null,
    expectedTargetRevision:
      input.expectedTargetRevision ?? input["expected-target-revision"] ?? null
  };
}

function requestArguments(operation, values) {
  const args = Array.isArray(values) ? values.map(plain) : [];
  switch (operation) {
    case "write":
      return [args[0], args[1], options(args[2]), mutation(args[3])];
    case "entries-page":
      return [args[0], options(args[1])];
    case "mkdir":
    case "delete":
      return [args[0], options(args[1]), mutation(args[2])];
    case "copy":
    case "move":
      return [args[0], args[1], options(args[2]), mutation(args[3])];
    default:
      return args;
  }
}

function contextFor(receiver, fallback) {
  const context = receiver?.kernelContext ?? receiver?.context ?? receiver ?? fallback;
  if (!context || (typeof context !== "object" && typeof context !== "function")) {
    fail("file/host-unavailable", "IndexedDB host requires a kernel context");
  }
  return context;
}

function signalFor(receiver) {
  return receiver?.signal ?? new AbortController().signal;
}

export function createIndexedDbWasmHost(optionsValue = {}) {
  const host = createIndexedDbFilesystemHost(optionsValue);
  const fallbackContext = {};
  const mounts = new Map();
  const contexts = new Set();
  const pending = new Map();
  let nextMount = 0;
  let nextRequest = 0;

  function mount(value) {
    const record = mounts.get(String(value));
    if (!record) fail("file/provider-closed", "unknown or closed IndexedDB filesystem mount");
    return record;
  }

  async function describe() {
    return wire({
      provider: "indexeddb",
      identity: "hara/filesystem-indexeddb",
      abi: "hta.v1",
      route: "hta-wasm",
      capabilities: [...KNOWN_CAPABILITIES]
    });
  }

  async function open(options, receiver) {
    const context = contextFor(receiver, fallbackContext);
    const mountId = ++nextMount;
    if (!Number.isSafeInteger(mountId)) fail("file/io", "IndexedDB mount ids are exhausted");
    const descriptor = await host.register(context, mountId, openConfiguration(options));
    contexts.add(context);
    mounts.set(String(mountId), { context, mountId });
    return wire({ mount: mountId, descriptor });
  }

  async function request(mountValue, operationValue, values, receiver) {
    const record = mount(mountValue);
    const operation = String(valueName(operationValue));
    const externalSignal = signalFor(receiver);
    const controller = new AbortController();
    const abort = () => controller.abort(externalSignal.reason ?? new Error("cancelled"));
    if (externalSignal.aborted) abort();
    else externalSignal.addEventListener?.("abort", abort, { once: true });
    const requestId = String(receiver?.call ?? receiver?.task ?? ++nextRequest);
    const key = `${String(mountValue)}:${requestId}`;
    pending.set(key, { controller, record });
    try {
      return wire(await host.invoke(
        record.context,
        record.mountId,
        operation,
        requestArguments(operation, values),
        { signal: controller.signal }
      ));
    } finally {
      externalSignal.removeEventListener?.("abort", abort);
      pending.delete(key);
    }
  }

  async function cancel(mountValue, id) {
    const key = `${String(mountValue)}:${String(id)}`;
    const request = pending.get(key);
    if (!request) return false;
    request.controller.abort(new Error("cancelled"));
    return true;
  }

  async function close(mountValue) {
    const record = mounts.get(String(mountValue));
    if (!record) return null;
    for (const request of pending.values()) {
      if (request.record === record) request.controller.abort(new Error("closed"));
    }
    mounts.delete(String(mountValue));
    await host.close(record.context, record.mountId);
    return null;
  }

  async function closeAll() {
    for (const request of pending.values()) request.controller.abort(new Error("closed"));
    await Promise.allSettled([...contexts].map(context => host.closeContext(context)));
    pending.clear();
    mounts.clear();
    contexts.clear();
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

export { plain, normalisePath };

function normalisePath(value) {
  if (typeof value !== "string" || value.includes("\0") || value.includes("\\")) {
    fail("file/invalid-path", "IndexedDB logical paths must use '/' separators");
  }
  const segments = [];
  for (const segment of value.split("/")) {
    if (!segment || segment === ".") continue;
    if (segment === "..") {
      if (!segments.length) fail("file/outside-root", "path escapes the mounted root");
      segments.pop();
    } else segments.push(segment);
  }
  return segments.length ? `/${segments.join("/")}` : "/";
}
