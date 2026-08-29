import { HtaContext, HtaHandle, HtaKeyword, HtaMapEntry, HtaStruct } from "./index.js";

export const BROWSER_WASM_SANDBOX_PROTOCOL = "hara.browser-wasm-sandbox/0-alpha";
export const MCP_PURE_PROFILE = "hara.mcp-pure/0-alpha";
export const SANDBOX_EVAL_TARGET = "sandbox/eval";

export const DEFAULT_BROWSER_SANDBOX_LIMITS = Object.freeze({
  sourceBytes: 65_536,
  outputBytes: 1_048_576,
  wallMs: 5_000,
});

export const MAX_BROWSER_SANDBOX_LIMITS = Object.freeze({
  sourceBytes: 1_048_576,
  outputBytes: 4_194_304,
  wallMs: 30_000,
});

const REQUEST_KEYS = new Set(["operation", "source", "limits"]);
const LIMIT_KEYS = new Set(["sourceBytes", "outputBytes", "wallMs"]);
const TEXT_ENCODER = new TextEncoder();
const MAX_TRANSFER_DEPTH = 32;
const MAX_TRANSFER_ITEMS = 65_536;

function fail(code, message) {
  const error = new Error(message);
  error.code = code;
  return error;
}

function exactObject(value, keys, label) {
  if (value === null || typeof value !== "object" || Array.isArray(value)) {
    throw fail("sandbox/request-invalid", `${label} must be an object`);
  }
  for (const key of Object.keys(value)) {
    if (!keys.has(key)) {
      throw fail("sandbox/request-invalid", `${label} contains unknown field ${key}`);
    }
  }
  return value;
}

function positiveInteger(value, label, maximum) {
  if (!Number.isSafeInteger(value) || value <= 0 || value > maximum) {
    throw fail(
      "sandbox/limit-invalid",
      `${label} must be a positive integer no greater than ${maximum}`,
    );
  }
  return value;
}

function byteLength(value) {
  return TEXT_ENCODER.encode(value).byteLength;
}

function validateSource(source, maximum) {
  if (typeof source !== "string" || source.length === 0) {
    throw fail("sandbox/request-invalid", "sandbox.eval requires non-empty source");
  }
  const bytes = byteLength(source);
  if (bytes > maximum) {
    throw fail(
      "sandbox/source-limit",
      `source is ${bytes} bytes, above the sandbox maximum ${maximum}`,
    );
  }
  return source;
}

function validateLimits(value) {
  if (value === undefined) return { ...DEFAULT_BROWSER_SANDBOX_LIMITS };
  const limits = exactObject(value, LIMIT_KEYS, "sandbox limits");
  return {
    sourceBytes:
      limits.sourceBytes === undefined
        ? DEFAULT_BROWSER_SANDBOX_LIMITS.sourceBytes
        : positiveInteger(
            limits.sourceBytes,
            "limits.sourceBytes",
            MAX_BROWSER_SANDBOX_LIMITS.sourceBytes,
          ),
    outputBytes:
      limits.outputBytes === undefined
        ? DEFAULT_BROWSER_SANDBOX_LIMITS.outputBytes
        : positiveInteger(
            limits.outputBytes,
            "limits.outputBytes",
            MAX_BROWSER_SANDBOX_LIMITS.outputBytes,
          ),
    wallMs:
      limits.wallMs === undefined
        ? DEFAULT_BROWSER_SANDBOX_LIMITS.wallMs
        : positiveInteger(limits.wallMs, "limits.wallMs", MAX_BROWSER_SANDBOX_LIMITS.wallMs),
  };
}

export function validateBrowserSandboxRequest(value) {
  const request = exactObject(value, REQUEST_KEYS, "sandbox request");
  if (request.operation !== "sandbox.eval") {
    throw fail(
      "sandbox/capability-unsupported",
      `browser sandbox operation is not available: ${String(request.operation)}`,
    );
  }
  const limits = validateLimits(request.limits);
  return Object.freeze({
    operation: request.operation,
    source: validateSource(request.source, limits.sourceBytes),
    limits: Object.freeze(limits),
  });
}

function projectMap(value, state, depth) {
  const projected = {};
  const entries = [...value.entries()];
  state.items += entries.length;
  if (state.items > MAX_TRANSFER_ITEMS) {
    throw fail("sandbox/result-limit", "sandbox result contains too many values");
  }
  for (const [key, item] of entries) {
    let name;
    if (typeof key === "string") name = key;
    else if (key instanceof HtaKeyword) name = `:${key.name}`;
    else {
      throw fail(
        "sandbox/result-non-transferable",
        "sandbox map keys must be strings or keywords",
      );
    }
    if (Object.prototype.hasOwnProperty.call(projected, name)) {
      throw fail("sandbox/result-non-transferable", `sandbox map key collision: ${name}`);
    }
    Object.defineProperty(projected, name, {
      value: projectValue(item, state, depth + 1),
      enumerable: true,
      configurable: true,
      writable: true,
    });
  }
  return projected;
}

function projectValue(value, state, depth) {
  if (depth > MAX_TRANSFER_DEPTH) {
    throw fail("sandbox/result-limit", "sandbox result exceeds maximum nesting depth");
  }
  if (value === null || typeof value === "string" || typeof value === "boolean") return value;
  if (typeof value === "number") {
    if (!Number.isFinite(value)) {
      throw fail("sandbox/result-non-transferable", "sandbox result contains a non-finite number");
    }
    return value;
  }
  if (typeof value === "bigint") {
    const number = Number(value);
    if (!Number.isSafeInteger(number)) {
      throw fail("sandbox/result-non-transferable", "sandbox integer exceeds transfer-safe range");
    }
    return number;
  }
  if (value instanceof HtaKeyword) return `:${value.name}`;
  if (value instanceof HtaMapEntry) {
    return [
      projectValue(value.key, state, depth + 1),
      projectValue(value.value, state, depth + 1),
    ];
  }
  if (value instanceof HtaHandle || value instanceof HtaStruct) {
    throw fail(
      "sandbox/result-non-transferable",
      "sandbox result contains a live runtime value",
    );
  }
  if (Array.isArray(value)) {
    state.items += value.length;
    if (state.items > MAX_TRANSFER_ITEMS) {
      throw fail("sandbox/result-limit", "sandbox result contains too many values");
    }
    return value.map((item) => projectValue(item, state, depth + 1));
  }
  if (value instanceof Map) return projectMap(value, state, depth);
  throw fail(
    "sandbox/result-non-transferable",
    `sandbox result type is not transfer-safe: ${Object.prototype.toString.call(value)}`,
  );
}

export function projectBrowserSandboxValue(value, outputBytes) {
  const json = projectValue(value, { items: 1 }, 0);
  const text = typeof json === "string" ? json : JSON.stringify(json);
  const bytes = byteLength(text);
  if (bytes > outputBytes) {
    throw fail(
      "sandbox/output-limit",
      `sandbox result is ${bytes} bytes, above the requested maximum ${outputBytes}`,
    );
  }
  return Object.freeze({ text, json });
}

function defaultWorkerFactory(workerUrl) {
  if (typeof Worker !== "function") {
    throw fail("sandbox/worker-unavailable", "Worker is unavailable in this environment");
  }
  return new Worker(workerUrl, { type: "module", name: "hara-browser-sandbox" });
}

function defaultContextFactory(options) {
  return new HtaContext(options);
}

export class BrowserWasmSandbox {
  constructor(options = {}) {
    const allowed = new Set([
      "workerUrl",
      "moduleUrl",
      "moduleBytes",
      "workerFactory",
      "contextFactory",
      "setTimer",
      "clearTimer",
    ]);
    exactObject(options, allowed, "BrowserWasmSandbox options");
    if (
      !(typeof options.workerUrl === "string" || options.workerUrl instanceof URL) ||
      String(options.workerUrl).length === 0
    ) {
      throw fail("sandbox/config-invalid", "workerUrl must be a non-empty URL or string");
    }
    if ((options.moduleUrl === undefined) === (options.moduleBytes === undefined)) {
      throw fail(
        "sandbox/config-invalid",
        "exactly one of moduleUrl or moduleBytes must be supplied",
      );
    }
    if (
      options.moduleUrl !== undefined &&
      !(typeof options.moduleUrl === "string" || options.moduleUrl instanceof URL)
    ) {
      throw fail("sandbox/config-invalid", "moduleUrl must be a URL or string");
    }
    if (
      options.moduleBytes !== undefined &&
      !(options.moduleBytes instanceof Uint8Array || options.moduleBytes instanceof ArrayBuffer)
    ) {
      throw fail("sandbox/config-invalid", "moduleBytes must be an ArrayBuffer or Uint8Array");
    }
    this.workerUrl = options.workerUrl;
    this.moduleUrl = options.moduleUrl;
    this.moduleBytes = options.moduleBytes;
    this.workerFactory = options.workerFactory ?? defaultWorkerFactory;
    this.contextFactory = options.contextFactory ?? defaultContextFactory;
    this.setTimer = options.setTimer ?? ((callback, milliseconds) => setTimeout(callback, milliseconds));
    this.clearTimer = options.clearTimer ?? ((timer) => clearTimeout(timer));
    this.state = "new";
    this.worker = null;
    this.context = null;
    this.activePromise = null;
  }

  snapshot() {
    return Object.freeze({
      protocol: BROWSER_WASM_SANDBOX_PROTOCOL,
      profile: MCP_PURE_PROFILE,
      state: this.state,
      reusable: false,
      hostCalls: false,
      filesystem: false,
    });
  }

  cancel() {
    if (this.state !== "running" || !this.activePromise) return false;
    const active = this.activePromise;
    this.activePromise = null;
    const cancelled = active.cancel?.() === true;
    this.state = "cancelling";
    if (!cancelled) this.close();
    return cancelled;
  }

  close() {
    if (this.state === "closed") return;
    this.activePromise?.cancel?.();
    this.activePromise = null;
    const context = this.context;
    const worker = this.worker;
    this.context = null;
    this.worker = null;
    try {
      context?.close?.();
    } finally {
      worker?.terminate?.();
      this.state = "closed";
    }
  }

  async run(value, options = {}) {
    exactObject(options, new Set(["signal"]), "sandbox run options");
    if (this.state !== "new") {
      throw fail("sandbox/not-reusable", "a browser sandbox instance can execute exactly once");
    }
    const request = validateBrowserSandboxRequest(value);
    const signal = options.signal;
    if (
      signal !== undefined &&
      !(
        typeof signal === "object" &&
        typeof signal.aborted === "boolean" &&
        typeof signal.addEventListener === "function" &&
        typeof signal.removeEventListener === "function"
      )
    ) {
      throw fail("sandbox/request-invalid", "signal must be an AbortSignal-compatible object");
    }
    if (signal?.aborted) {
      this.state = "closed";
      throw fail("sandbox/cancelled", "sandbox request was cancelled before start");
    }

    let timer;
    let timedOut = false;
    const onAbort = () => this.cancel();
    this.state = "starting";
    try {
      this.worker = this.workerFactory(this.workerUrl);
      this.context = this.contextFactory({
        worker: this.worker,
        moduleUrl: this.moduleUrl,
        moduleBytes: this.moduleBytes,
        hostCalls: Object.freeze({}),
        filesystemHost: null,
        kernelId: null,
      });
      signal?.addEventListener("abort", onAbort, { once: true });
      timer = this.setTimer(() => {
        timedOut = true;
        if (!this.cancel()) this.close();
      }, request.limits.wallMs);
      if (signal?.aborted) throw fail("sandbox/cancelled", "sandbox request was cancelled");
      await this.context.ready;
      if (timedOut) throw fail("sandbox/timed-out", "sandbox wall-time limit expired");
      this.state = "running";
      const call = this.context.call(SANDBOX_EVAL_TARGET, [request.source]);
      this.activePromise = call;
      const result = await call;
      if (timedOut) throw fail("sandbox/timed-out", "sandbox wall-time limit expired");
      if (signal?.aborted) throw fail("sandbox/cancelled", "sandbox request was cancelled");
      const projection = projectBrowserSandboxValue(result, request.limits.outputBytes);
      this.state = "completed";
      return Object.freeze({
        protocol: BROWSER_WASM_SANDBOX_PROTOCOL,
        profile: MCP_PURE_PROFILE,
        status: "completed",
        value: projection,
        cleanup: "completed",
      });
    } catch (error) {
      if (timedOut) throw fail("sandbox/timed-out", "sandbox wall-time limit expired");
      if (signal?.aborted || error?.message === "cancelled") {
        throw fail("sandbox/cancelled", "sandbox request was cancelled");
      }
      throw error;
    } finally {
      if (timer !== undefined) this.clearTimer(timer);
      signal?.removeEventListener("abort", onAbort);
      this.close();
    }
  }
}
