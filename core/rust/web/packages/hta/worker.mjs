import { decodeHta, encodeHta, HTA_MAX_FRAME_BYTES } from "./index.js";
import { createBrowserProvider } from "./provider-browser.mjs";
import { createProviderLifecycle, HTA_PROVIDER_EVENT, providerErrorCode } from "./provider-common.mjs";

let instance;
let abiVersion;
let backend;
let lifecycle;
const requests = new Map();
const tasks = new Map();
const hostTasks = new Map();
const externrefTable = [];
let activeOperation = null;

const wasmImports = {
  env: {
    hara_random_fill(pointer, length) {
      const memory = instance?.exports.memory;
      if (!memory) throw new Error("hta/memory-unavailable");
      try {
        globalThis.crypto.getRandomValues(new Uint8Array(memory.buffer, pointer, length));
        return 0;
      } catch {
        return 1;
      }
    },
    hara_time_ms() {
      return BigInt(Math.trunc(Date.now()));
    },
    hara_time_ns() {
      return BigInt(Math.trunc(performance.now() * 1_000_000));
    }
  },
  "__wbindgen_placeholder__": {
    __wbindgen_describe() {},
    __wbindgen_object_drop_ref(index) {
      externrefTable[index] = undefined;
    },
    __wbg_getRandomValues_eb590f34c5dc8fa0(pointer, length) {
      const memory = instance?.exports.memory;
      if (!memory) throw new Error("hta/memory-unavailable");
      crypto.getRandomValues(new Uint8Array(memory.buffer, pointer, length));
    },
    __wbg___wbindgen_throw_bb96b2010945f0bc(pointer, length) {
      const bytes = instance ? new Uint8Array(instance.exports.memory.buffer, pointer, length) : [];
      throw new Error(instance ? new TextDecoder().decode(bytes) : "wasm error");
    }
  },
  "__wbindgen_externref_xform__": {
    __wbindgen_externref_table_grow(size) {
      const previous = externrefTable.length;
      externrefTable.length += size;
      return previous;
    },
    __wbindgen_externref_table_set_null(index) {
      externrefTable[index] = undefined;
    }
  }
};

self.addEventListener("message", event => {
  return receive(event.data);
});

async function receive(message) {
  try {
    if (message.type === "init") {
      await initialize(message);
      return;
    }
    if (backend?.kind === "provider") {
      await backend.provider.handle(message);
      if (message.type === "close") {
        self.postMessage({type:"closed"});
        self.close();
      }
      return;
    }
    if (backend?.kind !== "wasm") throw new Error("hta/worker-not-initialized");
    if (message.type === "call") {
      const session = requestSession(message.frame);
      const [operation] = decodeHta(message.frame);
      activeOperation = String(operation);
      const task = Number(callFrame(instance.exports.hta_start, message.frame));
      if (task <= 0) throw new Error("hta/start-failed");
      requests.set(message.id, task);
      tasks.set(task, { id: message.id, session, operation: String(operation) });
      lifecycle?.emit(HTA_PROVIDER_EVENT.CALL_ENTER, {
        request: Number(message.id),
        operation: String(operation)
      });
      pump();
    } else if (message.type === "delivery") {
      const hostTask = hostTasks.get(message.call);
      const task = hostTask?.task;
      if (hostTask === undefined || !tasks.has(task)) {
        hostTasks.delete(message.call);
        pump();
        return;
      }
      hostTasks.delete(message.call);
      callFrame(
        instance.exports.hta_deliver,
        encodeHta([message.call, message.ok ? 0 : 1, decodeHta(message.frame)])
      );
      pump();
    } else if (message.type === "cancel") {
      const task = requests.get(message.id);
      if (task !== undefined) {
        const request = tasks.get(task);
        const calls = [...hostTasks.entries()]
          .filter(([, hostTask]) => hostTask.task === task)
          .map(([call, hostTask]) => ({
            call,
            task,
            session: hostTask.session,
            mount: hostTask.mount,
            service: hostTask.service,
            method: hostTask.method
          }));
        if (calls.length) self.postMessage({ type: "host-cancel", calls });
        const cancelStatus = instance.exports.hta_cancel(BigInt(task));
        const dropStatus = instance.exports.hta_drop_task(BigInt(task));
        requests.delete(message.id);
        tasks.delete(task);
        removeHostTasks(task);
        if (cancelStatus !== 0) throw new Error(`hta/cancel-failed: ${cancelStatus}`);
        if (dropStatus !== 0) throw new Error(`hta/drop-task-failed: ${dropStatus}`);
        lifecycle?.emit(HTA_PROVIDER_EVENT.CANCEL, {
          request: Number(message.id),
          operation: request?.operation
        });
        pump();
      }
    } else if (message.type === "release") {
      const status = callFrame(instance.exports.hta_release, message.frame);
      if (Number(status) !== 0) throw new Error(`hta/handle-release-failed: ${status}`);
      lifecycle?.emit(HTA_PROVIDER_EVENT.RELEASE, { status: "ok" });
      pump();
    } else if (message.type === "close") {
      closeWasm();
    }
  } catch (error) {
    lifecycle?.emit(HTA_PROVIDER_EVENT.FAILURE, {
      status: "error",
      code: providerErrorCode(error)
    });
    const operation = activeOperation ? ` (${activeOperation})` : "";
    self.postMessage({ type: "fatal", error: { message: `${String(error?.message ?? error)}${operation}` } });
  }
}

async function initialize(message) {
  if (backend) throw new Error("hta/worker-already-initialized");
  if (message.backend === "provider") {
    if (typeof message.providerUrl !== "string") throw new Error("hta/provider-missing");
    const providerModule = await import(message.providerUrl);
    const call = providerModule.default ?? providerModule.call ?? providerModule.provider;
    if (typeof call !== "function") throw new Error("hta/provider-invalid");
    const close = providerModule.close ?? providerModule.closeAll;
    if (close !== undefined && typeof close !== "function") throw new Error("hta/provider-close-invalid");
    if (providerModule.release !== undefined && typeof providerModule.release !== "function") {
      throw new Error("hta/provider-release-invalid");
    }
    backend = {
      kind: "provider",
      provider: createBrowserProvider(call, {
        scope: self,
        errorCode: message.errorCode,
        close: close === undefined ? undefined : () => close(),
        release: providerModule.release,
        onEvent: message.instrumentation
          ? event => self.postMessage({ type: "provider-event", event })
          : undefined
      })
    };
    self.postMessage({ type: "ready" });
    return;
  }

  const bytes = message.moduleBytes ?? await (await fetch(message.moduleUrl)).arrayBuffer();
  const libraryBytes = message.libraryBytes
    ?? (message.libraryUrl ? await (await fetch(message.libraryUrl)).arrayBuffer() : null);
  const imports = { ...wasmImports };
  if (libraryBytes) {
    const library = (await WebAssembly.instantiate(libraryBytes, imports)).instance;
    imports["hara/library"] = {};
    for (const entry of WebAssembly.Module.imports(await WebAssembly.compile(bytes))) {
      if (entry.module !== "hara/library") continue;
      const exported = library.exports[entry.name];
      if (exported === undefined) throw new Error(`hta/library-export-missing: ${entry.name}`);
      imports["hara/library"][entry.name] = exported;
    }
  }
  instance = (await WebAssembly.instantiate(bytes, imports)).instance;
  required();
  lifecycle = createProviderLifecycle({
    origin: "browser-wasm",
    onEvent: message.instrumentation
      ? event => self.postMessage({ type: "provider-event", event })
      : undefined
  });
  lifecycle.emit(HTA_PROVIDER_EVENT.START);
  backend = { kind: "wasm" };
  self.postMessage({ type: "ready" });
}

function required() {
  for (const name of [
    "memory", "hta_abi_version", "hta_alloc", "hta_dealloc", "hta_start",
    "hta_next_event", "hta_deliver", "hta_cancel", "hta_drop_task", "hta_release"
  ]) {
    if (!(name in instance.exports)) throw new Error(`hta/export-missing: ${name}`);
  }
  abiVersion = instance.exports.hta_abi_version();
  if (abiVersion !== 1 && abiVersion !== 2 && abiVersion !== 3 && abiVersion !== 4) {
    throw new Error("hta/version-unsupported");
  }
}

function callFrame(fn, frame) {
  const bytes = frame instanceof Uint8Array ? frame : new Uint8Array(frame);
  if (bytes.length > HTA_MAX_FRAME_BYTES) throw new Error("hta/value-too-large: frame exceeds 64 MiB");
  const pointer = Number(instance.exports.hta_alloc(bytes.length));
  if (!Number.isSafeInteger(pointer) || pointer < 0 || pointer + bytes.length > instance.exports.memory.buffer.byteLength) {
    throw new Error("hta/memory-unavailable");
  }
  new Uint8Array(instance.exports.memory.buffer, pointer, bytes.length).set(bytes);
  try {
    return fn(pointer, bytes.length);
  } finally {
    instance.exports.hta_dealloc(pointer, bytes.length);
  }
}

function next() {
  const packed = instance.exports.hta_next_event();
  if (packed === 0n) return null;
  const pointer = Number(packed >> 32n);
  const size = Number(packed & 0xffff_ffffn);
  if (!Number.isSafeInteger(pointer) || !Number.isSafeInteger(size) || size === 0
      || size > HTA_MAX_FRAME_BYTES || pointer + size > instance.exports.memory.buffer.byteLength) {
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
  return typeof target === "string"
      && ["session/eval", "session/eval-vm", "session/prepare-vm", "session/invoke-vm",
        "session/eval-bound", "session/trace-eval", "session/complete"].includes(target)
      && typeof args?.[0] === "string"
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
      requests.delete(request.id);
      removeHostTasks(task);
      instance.exports.hta_drop_task(BigInt(task));
      self.postMessage({ type: "result", id: request.id, ok: kind === 0, frame: encodeHta(event[2]) });
      lifecycle?.emit(kind === 0 ? HTA_PROVIDER_EVENT.CALL_RETURN : HTA_PROVIDER_EVENT.CALL_ERROR, {
        request: request.id,
        operation: request.operation,
        status: kind === 0 ? "ok" : "error"
      });
    } else if (kind === 2) {
      const call = Number(event[1]);
      const task = Number(event[2]);
      hostTasks.set(call, {
        task,
        session: event[3],
        mount: event[4] ?? null,
        service: String(abiVersion >= 2 ? event[5] : event[3]),
        method: String(abiVersion >= 2 ? event[6] : event[4])
      });
      const request = tasks.get(task);
      lifecycle?.emit(HTA_PROVIDER_EVENT.HOST_CALL, {
        request: request?.id,
        task,
        call,
        service: String(abiVersion >= 2 ? event[5] : event[3]),
        method: String(abiVersion >= 2 ? event[6] : event[4]),
        status: "enter"
      });
      if (abiVersion >= 2) {
        self.postMessage({
          type: "host-call", call, task, session: event[3], mount: event[4] ?? null,
          service: event[5], method: event[6], frame: encodeHta(event[7])
        });
      } else {
        const request = tasks.get(task);
        self.postMessage({
          type: "host-call", call, task, session: request?.session ?? "ROOT", mount: null,
          service: event[3], method: event[4], frame: encodeHta(event[5])
        });
      }
    } else {
      throw new Error(`hta/event-unknown: ${kind}`);
    }
  }
}

function removeHostTasks(task) {
  for (const [call, pendingTask] of hostTasks) {
    if (pendingTask.task === task) hostTasks.delete(call);
  }
}

function closeWasm() {
  const calls = [...hostTasks.entries()].map(([call, hostTask]) => ({
    call,
    task: hostTask.task,
    session: hostTask.session,
    mount: hostTask.mount,
    service: hostTask.service,
    method: hostTask.method
  }));
  if (calls.length) self.postMessage({ type: "host-cancel", calls });
  for (const [task] of tasks) {
    instance.exports.hta_cancel(BigInt(task));
    instance.exports.hta_drop_task(BigInt(task));
  }
  requests.clear();
  tasks.clear();
  hostTasks.clear();
  lifecycle?.emit(HTA_PROVIDER_EVENT.TERMINAL, { status: "ok" });
  lifecycle?.shutdown({ status: "ok" });
  self.postMessage({type:"closed"});
  self.close();
}
