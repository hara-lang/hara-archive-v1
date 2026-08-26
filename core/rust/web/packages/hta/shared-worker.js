import { decodeHta, encodeHta, HTA_MAX_FRAME_BYTES } from "./index.js";
import { createBrowserProvider } from "./provider-browser.mjs";

// One raw HTA instance shared by every same-origin tab. Each MessagePort owns
// its request IDs and host calls, while the task table associates kernel work
// with the port that initiated it.
let instance;
let boot;
const clients = new Map();
const providers = new Map();
const tasks = new Map();
const hostCalls = new Map();

self.onconnect = (event) => {
  const port = event.ports[0];
  clients.set(port, new Map());
  port.addEventListener("message", (message) => receive(port, message.data));
  port.start();
};

async function receive(port, message) {
  try {
    if (message.type === "init") {
      if (message.backend === "provider") {
        await initializeProvider(port, message);
        port.postMessage({ type: "ready" });
        return;
      }
      boot ??= instantiate(message);
      await boot;
      port.postMessage({ type: "ready" });
    } else if (providers.has(port)) {
      const provider = providers.get(port);
      await provider.handle(message);
      if (message.type === "close") {
        providers.delete(port);
        clients.delete(port);
        port.postMessage({ type: "closed" });
      }
    } else if (message.type === "call") {
      await boot;
      const session = requestSession(message.frame);
      const task = Number(callFrame(instance.exports.hta_start, message.frame));
      clients.get(port)?.set(message.id, task);
      tasks.set(task, { port, id: message.id, session });
      pump();
    } else if (message.type === "delivery") {
      await boot;
      if (hostCalls.get(message.call)?.port !== port) return;
      hostCalls.delete(message.call);
      callFrame(instance.exports.hta_deliver, encodeHta([message.call, message.ok ? 0 : 1, decodeHta(message.frame)]));
      pump();
    } else if (message.type === "cancel") {
      const task = clients.get(port)?.get(message.id);
      if (task !== undefined) {
        const calls = [...hostCalls.entries()]
          .filter(([, owner]) => owner.port === port && owner.task === task)
          .map(([call, owner]) => ({ ...owner, call }));
        if (calls.length) port.postMessage({ type: "host-cancel", calls });
        clients.get(port).delete(message.id);
        tasks.delete(task);
        try { instance.exports.hta_cancel(BigInt(task)); }
        finally { instance.exports.hta_drop_task(BigInt(task)); }
        for (const [call, owner] of hostCalls) if (owner.port === port && owner.task === task) hostCalls.delete(call);
        pump();
      }
    } else if (message.type === "release") {
      await boot;
      const status = callFrame(instance.exports.hta_release, message.frame);
      if (Number(status) !== 0) throw new Error(`hta/handle-release-failed: ${status}`);
      pump();
    } else if (message.type === "close") {
      dropClientTasks(port);
      clients.delete(port);
      // HtaContext terminates the client port after the acknowledgement. The
      // acknowledgement must cross the port before the client closes it.
      port.postMessage({ type: "closed" });
    }
  } catch (error) {
    failPort(port, error);
  }
}

async function initializeProvider(port, message) {
  if (providers.has(port)) throw new Error("hta/worker-already-initialized");
  if (typeof message.providerUrl !== "string") throw new Error("hta/provider-missing");
  const providerModule = await import(message.providerUrl);
  const call = providerModule.default ?? providerModule.call ?? providerModule.provider;
  if (typeof call !== "function") throw new Error("hta/provider-invalid");
  const close = providerModule.close ?? providerModule.closeAll;
  if (close !== undefined && typeof close !== "function") {
    throw new Error("hta/provider-close-invalid");
  }
  if (providerModule.release !== undefined && typeof providerModule.release !== "function") {
    throw new Error("hta/provider-release-invalid");
  }
  providers.set(port, createBrowserProvider(call, {
    scope: { postMessage: value => port.postMessage(value) },
    origin: "shared-worker",
    errorCode: message.errorCode,
    close: close === undefined ? undefined : () => close(),
    release: providerModule.release,
    onEvent: message.instrumentation
      ? event => port.postMessage({ type: "provider-event", event })
      : undefined
  }));
}

async function instantiate(message) {
  const bytes = message.moduleBytes ?? await (await fetch(message.moduleUrl)).arrayBuffer();
  instance = (await WebAssembly.instantiate(bytes, {
    env: {
      hara_random_fill(pointer, length) {
      if (!validRange(pointer, length)) return 1;
      crypto.getRandomValues(new Uint8Array(instance.exports.memory.buffer, pointer, length));
      return 0;
      },
      hara_time_ms() { return BigInt(Math.trunc(Date.now())); },
      hara_time_ns() { return BigInt(Math.trunc(performance.now() * 1_000_000)); }
    }
  })).instance;
  for (const name of ["memory", "hta_abi_version", "hta_alloc", "hta_dealloc", "hta_start", "hta_next_event", "hta_deliver", "hta_cancel", "hta_drop_task", "hta_release"]) {
    if (!(name in instance.exports)) throw new Error(`hta/export-missing: ${name}`);
  }
  if (![1, 2, 3, 4].includes(instance.exports.hta_abi_version())) throw new Error("hta/version-unsupported");
}

function callFrame(fn, frame) {
  const bytes = frame instanceof Uint8Array ? frame : new Uint8Array(frame);
  if (bytes.length > HTA_MAX_FRAME_BYTES) throw new Error("hta/value-too-large: frame exceeds 64 MiB");
  const pointer = Number(instance.exports.hta_alloc(bytes.length));
  if (!Number.isSafeInteger(pointer) || !validRange(pointer, bytes.length)) throw new Error("hta/memory-unavailable");
  new Uint8Array(instance.exports.memory.buffer, pointer, bytes.length).set(bytes);
  try { return fn(pointer, bytes.length); } finally { instance.exports.hta_dealloc(pointer, bytes.length); }
}

function next() {
  const packed = instance.exports.hta_next_event();
  if (packed === 0n) return null;
  const pointer = Number(packed >> 32n);
  const size = Number(packed & 0xffff_ffffn);
  if (!Number.isSafeInteger(pointer) || !Number.isSafeInteger(size) || size === 0 ||
      size > HTA_MAX_FRAME_BYTES || !validRange(pointer, size)) {
    if (Number.isSafeInteger(pointer) && Number.isSafeInteger(size) && pointer >= 0 && size >= 0) {
      instance.exports.hta_dealloc(pointer, size);
    }
    throw new Error("hta/event-memory-invalid");
  }
  const frame = new Uint8Array(instance.exports.memory.buffer, pointer, size).slice();
  instance.exports.hta_dealloc(pointer, size);
  return decodeHta(frame);
}

function requestSession(frame) {
  const [target, args] = decodeHta(frame);
  return typeof target === "string" &&
    ["session/eval", "session/eval-vm", "session/prepare-vm", "session/invoke-vm",
      "session/eval-bound", "session/trace-eval", "session/complete"].includes(target) &&
    typeof args?.[0] === "string"
    ? args[0]
    : "ROOT";
}

function pump() {
  for (let event; (event = next()) !== null;) {
    const kind = Number(event[0]);
    if (kind === 0 || kind === 1) {
      const task = Number(event[1]);
      const request = tasks.get(task);
      if (!request) continue;
      tasks.delete(task);
      clients.get(request.port)?.delete(request.id);
      instance.exports.hta_drop_task(BigInt(task));
      request.port.postMessage({ type: "result", id: request.id, ok: kind === 0, frame: encodeHta(event[2]) });
    } else if (kind === 2) {
      const request = tasks.get(Number(event[2]));
      if (request) {
        const call = Number(event[1]);
        hostCalls.set(call, {
          port: request.port,
          task: Number(event[2]),
          session: event[3],
          mount: event[4] ?? null,
          service: event[5],
          method: event[6]
        });
        request.port.postMessage({ type: "host-call", call, task: Number(event[2]), session: event[3], mount: event[4] ?? null, service: event[5], method: event[6], frame: encodeHta(event[7]) });
      }
    } else throw new Error("hta/event-unknown");
  }
}

function validRange(pointer, size) {
  return Number.isSafeInteger(pointer) && Number.isSafeInteger(size) &&
    pointer >= 0 && size >= 0 && pointer + size <= instance.exports.memory.buffer.byteLength;
}

function dropClientTasks(port) {
  const requests = clients.get(port);
  if (!requests) return;
  for (const [id, task] of requests) {
    const calls = [...hostCalls.entries()]
      .filter(([, owner]) => owner.port === port && owner.task === task)
      .map(([call, owner]) => ({ ...owner, call }));
    if (calls.length) port.postMessage({ type: "host-cancel", calls });
    tasks.delete(task);
    try { instance.exports.hta_cancel(BigInt(task)); } finally { instance.exports.hta_drop_task(BigInt(task)); }
  }
  requests.clear();
  for (const [call, owner] of hostCalls) if (owner.port === port) hostCalls.delete(call);
}

function failPort(port, error) {
  dropClientTasks(port);
  port.postMessage({ type: "fatal", error: { message: String(error?.message ?? error) } });
}
