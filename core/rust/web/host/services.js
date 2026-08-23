const DEFAULT_DATABASE = "hara-studio";
const STORE = "kv";

/**
 * Generic host services for studio kernels: an IndexedDB key/value store and
 * fetch-backed HTTP. Returns a handler map for the `hostCalls` option of
 * `HtaContext`, keyed "service/method"; handlers are async, take plain
 * decoded HTA arguments, and return encodeable values (null -> nil).
 */
export function createHostServices(options = {}) {
  const dbName = options.dbName ?? DEFAULT_DATABASE;
  const fetchImpl = options.fetch ?? globalThis.fetch.bind(globalThis);
  const scopeForContext = options.scopeForContext ?? null;
  const memoryFilesystems = options.memoryFilesystems ?? new Map();
  let opening = null;

  function open() {
    opening ??= new Promise((resolve, reject) => {
      const request = indexedDB.open(dbName, 1);
      request.onupgradeneeded = () => request.result.createObjectStore(STORE);
      request.onsuccess = () => resolve(request.result);
      request.onerror = () => reject(request.error);
    });
    const pending = opening;
    pending.catch(() => { if (opening === pending) opening = null; });
    return pending;
  }

  async function store(mode) {
    const db = await open();
    return db.transaction(STORE, mode).objectStore(STORE);
  }

  const filesystemHost = createFilesystemHost({
    store,
    memoryFilesystems: options.memoryFilesystems ?? new Map()
  });
  const hostDescription = createHostDescription({
    capabilities: options.capabilities,
    grantedCapabilities: options.grantedCapabilities,
    limits: options.limits,
    graphHost: options.graphHost,
    nodeRuntime: options.nodeRuntime,
    canvasRuntime: options.canvasRuntime ?? options.canvasRuntimeForSession,
    audioPipeline: options.supersonic ?? options.audioPipeline
  });

  function scopedKey(invocation, key, { keys = false } = {}) {
    if (!scopeForContext) return key;
    // Host calls originate from an HtaSession, while website scope ownership
    // is registered against its parent HtaContext when the kernel starts.
    // Prefer that kernel context, retaining the session context for callers
    // that scope sessions directly.
    const space = scopeForContext(invocation?.kernelContext ?? invocation?.context);
    if (!space) throw new Error("store/workspace-scope-unavailable");
    const prefix = `spaces/${space}/`;
    if (keys && (key === undefined || key === null)) return prefix;
    if (typeof key !== "string" || !key.startsWith(prefix)) {
      throw new Error(`store/workspace-scope-denied:${space}`);
    }
    return key;
  }

  const services = {
    "store/get": async function(key) {
      return request(await store("readonly"), "get", scopedKey(this, key));
    },
    "store/put": async function(key, value) {
      key = scopedKey(this, key);
      await request(await store("readwrite"), "put", value, key);
      return true;
    },
    "store/del": async function(key) {
      key = scopedKey(this, key);
      await request(await store("readwrite"), "delete", key);
      return true;
    },
    "store/keys": async function(prefix) {
      prefix = scopedKey(this, prefix, { keys: true });
      const keys = await request(await store("readonly"), "getAllKeys");
      return prefix === undefined || prefix === null
        ? keys
        : keys.filter((key) => key.startsWith(prefix));
    },
    "file/read": function(path) {
      return filesystemHost.invoke(this.kernelContext, this.mountId, "read", [path]);
    },
    "file/write": function(path, bytes) {
      return filesystemHost.invoke(this.kernelContext, this.mountId, "write", [path, bytes]);
    },
    "file/exists": function(path) {
      return filesystemHost.invoke(this.kernelContext, this.mountId, "exists", [path]);
    },
    "file/list": function(path) {
      return filesystemHost.invoke(this.kernelContext, this.mountId, "list", [path]);
    },
    "file/mkdir": function(path) {
      return filesystemHost.invoke(this.kernelContext, this.mountId, "mkdir", [path]);
    },
    "file/delete": function(path) {
      return filesystemHost.invoke(this.kernelContext, this.mountId, "delete", [path]);
    },
    "http/get": async (url) => {
      const response = await fetchImpl(url);
      if (!response.ok) throw new Error(`http/get failed with status ${response.status}`);
      return response.text();
    },
    "json/parse": async (text) => fromJson(parseJson(text))
  };
  if (options.nodeRuntime) Object.assign(services, createNodeHostServices(options.nodeRuntime));
  if (options.graphHost) Object.assign(services, createGraphHostServices(options.graphHost, options.graphHostOptions));
  if (options.canvasRuntime || options.canvasRuntimeForSession) {
    const canvasFor = (invocation) =>
      options.canvasRuntimeForSession?.(invocation.sessionId ?? "ROOT") ??
      options.canvasRuntime;
    services["studio.canvas/next-frame"] = function(nodeId, canvasId) {
      const runtime = canvasFor(this);
      if (!runtime) throw new Error(`canvas/session-unavailable:${this.sessionId ?? "ROOT"}`);
      return runtime.nextFrame(nodeId, canvasId);
    };
    services["studio.canvas/render"] = function(nodeId, canvasId, frame) {
      const runtime = canvasFor(this);
      if (!runtime) throw new Error(`canvas/session-unavailable:${this.sessionId ?? "ROOT"}`);
      return runtime.render(nodeId, canvasId, frame);
    };
    services["studio.canvas/publish"] = function(nodeId, canvasId, frame) {
      const runtime = canvasFor(this);
      if (!runtime) throw new Error(`canvas/session-unavailable:${this.sessionId ?? "ROOT"}`);
      return runtime.publish(nodeId, canvasId, frame);
    };
  }
  if (options.audioPipeline) {
    services["studio.audio/configure"] = async (spec) =>
      toHta(await options.audioPipeline.configure(toPlain(spec)));
    services["studio.audio/control"] = async (command, value) =>
      toHta(await options.audioPipeline.control(String(command), toPlain(value)));
  }
  if (options.supersonic) {
    services["gw.audio.supersonic/start"] = async (graph) =>
      toHta(await options.supersonic.start(toPlain(graph)));
    services["gw.audio.supersonic/update"] = async (graphId, nodeId, parameter, value) =>
      toHta(await options.supersonic.update(
        String(graphId), String(nodeId), String(parameter), toPlain(value)));
    services["gw.audio.supersonic/status"] = async (graphId) =>
      toHta(await options.supersonic.status(String(graphId)));
    services["gw.audio.supersonic/stop"] = async (graphId) =>
      toHta(await options.supersonic.stop(String(graphId)));
    // Compatibility for early Studio documents. Both names reach the same
    // provider; new code should use gw.audio.supersonic directly.
    services["studio.audio/configure"] = async (graph) =>
      toHta(await options.supersonic.start(toPlain(graph)));
    services["studio.audio/control"] = async (command, value) =>
      toHta(await options.supersonic.engine?.control?.(String(command), toPlain(value)) ?? false);
  }
  if (options.renderCanvas && !options.canvasRuntime) {
    services["studio.canvas/render"] = async (...args) => {
      const [canvas, scene] = args.length >= 3 ? args.slice(1) : args;
      await options.renderCanvas(canvas, scene);
      return true;
    };
  }
  if (options.grantsForSession || options.grantedCapabilities) {
    const capabilityForService = options.capabilityForService ?? defaultCapabilityForService;
    for (const [operation, handler] of Object.entries(services)) {
      const [service] = operation.split("/", 1);
      const capability = capabilityForService(service, operation);
      if (!capability) continue;
      services[operation] = async function(...args) {
        const granted = new Set(resolveGrantedCapabilities(
          hostDescription, options.grantsForSession, this?.sessionId ?? "ROOT"));
        if (!granted.has(capability)) throw new Error(`host/capability-denied:${capability}`);
        return handler.apply(this, args);
      };
    }
  }
  Object.assign(services, createHostIntrospection(hostDescription, {
    grantsForSession: options.grantsForSession
  }));
  Object.defineProperty(services, "filesystemHost", {
    value: filesystemHost,
    enumerable: false
  });
  return services;
}

/**
 * Creates the serializable, versioned descriptor for a browser host. It is
 * deliberately independent of a Studio shell: products may expose it to
 * tooling, while providers remain private to the embedding host.
 */
export function createHostDescription({
  capabilities = [], grantedCapabilities = null, limits = {}, graphHost = null, nodeRuntime = null,
  canvasRuntime = null, audioPipeline = null
} = {}) {
  const available = new Set(capabilities);
  available.add("filesystem");
  available.add("store");
  available.add("network/http");
  if (nodeRuntime) available.add("transport/node");
  if (graphHost) {
    available.add("program");
    available.add("graph");
    for (const capability of graphHost.availableCapabilities?.() ?? []) available.add(capability);
  }
  if (canvasRuntime) available.add("surface/canvas-2d");
  if (audioPipeline) available.add("audio/playback");
  const availableCapabilities = [...available].sort();
  const granted = grantedCapabilities == null
    ? availableCapabilities
    : normalizeCapabilities(grantedCapabilities).filter((capability) => available.has(capability));
  return {
    "host/version": "hara.host.v1",
    "host/available": availableCapabilities,
    "host/granted": granted,
    // Compatibility field for hosts that consumed the first hara.host.v1
    // draft. Its meaning is now explicitly the session grant set.
    "host/capabilities": granted,
    "host/limits": { ...limits }
  };
}

export function createHostIntrospection(description, { grantsForSession = null } = {}) {
  const describe = (invocation) => {
    const granted = resolveGrantedCapabilities(
      description, grantsForSession, invocation?.sessionId ?? "ROOT");
    return {
      ...description,
      "host/granted": granted,
      "host/capabilities": granted
    };
  };
  return {
    "host/describe": async function() { return toHta(describe(this)); },
    "host/capabilities": async function() { return toHta(describe(this)["host/granted"]); },
    "host/capability?": async function(capability) {
      return describe(this)["host/granted"]
        .includes(String(capability?.name ?? capability).replace(/^:/, ""));
    }
  };
}

function resolveGrantedCapabilities(description, grantsForSession, sessionId) {
  const available = new Set(description["host/available"] ?? []);
  const requested = grantsForSession
    ? grantsForSession(sessionId)
    : description["host/granted"] ?? [];
  return normalizeCapabilities(requested).filter((capability) => available.has(capability));
}

function normalizeCapabilities(values) {
  return [...new Set([...(values ?? [])]
    .map((value) => String(value?.name ?? value).replace(/^:/, ""))
    .filter(Boolean))].sort();
}

function defaultCapabilityForService(service) {
  return ({
    store: "store",
    file: "filesystem",
    http: "network/http",
    program: "program",
    graph: "graph",
    node: "transport/node",
    "studio.canvas": "surface/canvas-2d",
    "studio.audio": "audio/playback",
    "gw.audio.supersonic": "audio/playback"
  })[service] ?? null;
}

export function createFilesystemHost({ store, memoryFilesystems = new Map() }) {
  const contexts = new WeakMap();

  function normalize(path) {
    if (typeof path !== "string" || path.includes("\0")) throw new Error("file/path-invalid");
    const parts = [];
    for (const part of path.split("/")) {
      if (part === "" || part === ".") continue;
      if (part === "..") {
        if (parts.length === 0) throw new Error("file/path-denied");
        parts.pop();
      } else parts.push(part);
    }
    return `/${parts.join("/")}`;
  }

  const parent = (path) => path === "/" ? null : path.slice(0, path.lastIndexOf("/")) || "/";
  const prefix = (mount) => `filesystems/${encodeURIComponent(mount.key)}/entries/`;
  const recordKey = (mount, path) => `${prefix(mount)}${encodeURIComponent(path)}`;

  async function records(mount) {
    if (mount.provider === "memory") return mount.entries;
    const keys = await request(await store("readonly"), "getAllKeys");
    const output = new Map();
    for (const key of keys.filter((key) => typeof key === "string" && key.startsWith(prefix(mount)))) {
      output.set(decodeURIComponent(key.slice(prefix(mount).length)),
        await request(await store("readonly"), "get", key));
    }
    return output;
  }

  async function getRecord(mount, path) {
    if (path === "/") return { kind: "directory" };
    if (mount.provider === "memory") return mount.entries.get(path) ?? null;
    return request(await store("readonly"), "get", recordKey(mount, path));
  }

  async function putRecord(mount, path, value) {
    if (mount.provider === "memory") {
      mount.entries.set(path, value);
      return;
    }
    await request(await store("readwrite"), "put", value, recordKey(mount, path));
  }

  async function deleteRecord(mount, path) {
    if (mount.provider === "memory") {
      mount.entries.delete(path);
      return;
    }
    await request(await store("readwrite"), "delete", recordKey(mount, path));
  }

  function mountsFor(context, create = false) {
    if (!context || (typeof context !== "object" && typeof context !== "function")) {
      throw new Error("filesystem/kernel-context-invalid");
    }
    let mounts = contexts.get(context);
    if (!mounts && create) contexts.set(context, mounts = new Map());
    return mounts;
  }

  async function requireMount(context, mountId) {
    if (!Number.isSafeInteger(mountId) || mountId <= 0) throw new Error("file/unattached");
    const mount = mountsFor(context)?.get(mountId);
    if (!mount) throw new Error(`file/mount-closed:${mountId}`);
    return mount;
  }

  return {
    async register(context, mountId, descriptor) {
      const mounts = mountsFor(context, true);
      if (!Number.isSafeInteger(mountId) || mountId <= 0 || mounts.has(mountId)) {
        throw new Error("filesystem/mount-id-invalid");
      }
      const provider = descriptor.provider;
      if (provider !== "memory" && provider !== "indexeddb") {
        throw new Error(`filesystem/provider-unsupported:${provider}`);
      }
      if (provider === "indexeddb" && (typeof descriptor.key !== "string" || descriptor.key.length === 0)) {
        throw new Error("filesystem/indexeddb-key-invalid");
      }
      let entries;
      if (provider === "memory") {
        let contextFilesystems = memoryFilesystems.get(context);
        if (!contextFilesystems) memoryFilesystems.set(context, contextFilesystems = new Map());
        entries = contextFilesystems.get(mountId) ?? new Map();
        contextFilesystems.set(mountId, entries);
      }
      mounts.set(mountId, { provider, key: descriptor.key ?? String(mountId), entries });
      return true;
    },
    async close(context, mountId) {
      const mounts = mountsFor(context);
      if (!mounts) throw new Error(`file/mount-closed:${mountId}`);
      if (!mounts.delete(mountId)) throw new Error(`file/mount-closed:${mountId}`);
      memoryFilesystems.get(context)?.delete(mountId);
      return true;
    },
    async invoke(context, mountId, method, args) {
      const mount = await requireMount(context, mountId);
      const path = normalize(args[0]);
      if (method === "read") {
        const entry = await getRecord(mount, path);
        if (!entry || entry.kind !== "file") throw new Error(`file/not-found:${path}`);
        return new Uint8Array(entry.bytes);
      }
      if (method === "write") {
        const bytes = args[1];
        if (!(bytes instanceof Uint8Array)) throw new Error("file/bytes-required");
        const directory = parent(path);
        const parentEntry = await getRecord(mount, directory);
        if (!parentEntry || parentEntry.kind !== "directory") throw new Error(`file/parent-missing:${directory}`);
        await putRecord(mount, path, { kind: "file", bytes: new Uint8Array(bytes) });
        return null;
      }
      if (method === "exists") return (await getRecord(mount, path)) !== null;
      if (method === "mkdir") {
        let current = "";
        for (const part of path.split("/").filter(Boolean)) {
          current += `/${part}`;
          const entry = await getRecord(mount, current);
          if (entry?.kind === "file") throw new Error(`file/not-directory:${current}`);
          if (!entry) await putRecord(mount, current, { kind: "directory" });
        }
        return null;
      }
      if (method === "list") {
        const directory = await getRecord(mount, path);
        if (!directory || directory.kind !== "directory") throw new Error(`file/not-directory:${path}`);
        const start = path === "/" ? "/" : `${path}/`;
        return [...(await records(mount)).keys()]
          .filter((candidate) => candidate.startsWith(start) &&
            !candidate.slice(start.length).includes("/"))
          .sort();
      }
      if (method === "delete") {
        if (path === "/") throw new Error("file/root-delete-denied");
        const entry = await getRecord(mount, path);
        if (!entry) throw new Error(`file/not-found:${path}`);
        if (entry.kind === "directory") {
          const start = `${path}/`;
          if ([...(await records(mount)).keys()].some((candidate) => candidate.startsWith(start))) {
            throw new Error(`file/directory-not-empty:${path}`);
          }
        }
        await deleteRecord(mount, path);
        return null;
      }
      throw new Error(`file/method-unsupported:${method}`);
    }
  };
}

export function createNodeHostServices(runtime) {
  return {
    "node/in": async (nodeId, signal) => toHta(await runtime.in(nodeId, signal)),
    "node/in-frame": async (nodeId, signal) => toHta(await runtime.inFrame(nodeId, signal)),
    // Legacy value-oriented calls remain for existing Studio documents. New
    // documents use the frame forms below, which originate in
    // std.substrate.frame before reaching this browser adapter.
    "node/emit": async (nodeId, signal, value, meta) =>
      toHta(await runtime.emit(nodeId, signal, value, toPlain(meta))),
    "node/call": async (nodeId, target, action, args, opts) =>
      toHta((await runtime.call(nodeId, target, action, args, toPlain(opts))).data),
    "node/emit-frame": async (nodeId, frame) =>
      toHta(await runtime.emitFrame(nodeId, toPlain(frame))),
    "node/call-frame": async (nodeId, frame) =>
      toHta((await runtime.callFrame(nodeId, toPlain(frame))).data),
    "node/handle": function(nodeId, action, handlerId, meta) {
      const invocation = this;
      if (typeof handlerId !== "string" || handlerId.length === 0 || !invocation.context) {
        throw new Error("node/handle requires a kernel callback id");
      }
      const source = `(studio.node/invoke-handler ${JSON.stringify(handlerId)} __hta_arg_0 __hta_arg_1)`;
      runtime.stageKernelHandler(invocation.context, nodeId, action, (args, frame) => invocation.context.call(
        "eval-bound",
        [source, [toHta(args), toHta(frame)]]
      ), toPlain(meta));
      return handlerId;
    },
    "node/stop": (nodeId, task) => runtime.stop(nodeId, task),
    "node/info": (nodeId) => toHta(runtime.info(nodeId))
  };
}

/** Exact HTA host-call surface for generated programs and active graph nodes.
 * The compatibility session ingress methods are intentionally not registered
 * until SessionRouter owns their permission and lifecycle rules. */
export function createGraphHostServices(graph, options = {}) {
  const sessions = options.sessionRouter ?? graph.sessionRouter ?? null;
  const hostDescription = {
    "host/version": "hara.host.v1",
    "program/runtimes": options.programRuntimes ?? ["javascript/module", "javascript/audio-worklet"],
    capabilities: options.capabilities ?? graph.availableCapabilities?.() ?? [],
    limits: options.limits ?? {
      "program/max-source-bytes": 1048576,
      "graph/max-nodes": 1024,
      "graph/max-connections": 4096
    }
  };
  const services = {
    "program/install": async (descriptor, installOptions = {}) =>
      toHta(await graph.install(toPlain(descriptor), toPlain(installOptions))),
    "program/info": async (programId) => toHta(graph.programInfo(String(programId))),
    "program/release": async (programId) => graph.programs.release(String(programId)),
    "graph/spawn": async (descriptor, spawnOptions = {}) =>
      toHta(await graph.spawn(toPlain(descriptor), toPlain(spawnOptions))),
    "graph/release": async (nodeId) => graph.release(String(nodeId)),
    "graph/connect": async (descriptor) => graph.connect(toPlain(descriptor)),
    "graph/disconnect": async (connectionId) => graph.disconnect(String(connectionId)),
    "graph/send-frame": async (source, frame) => toHta(await graph.sendFrame(String(source), toPlain(frame))),
    "graph/call-frame": async (source, frame) => toHta(await graph.callFrame(String(source), toPlain(frame))),
    "graph/info": async (nodeId) => toHta(graph.info(String(nodeId))),
    "graph/list": async () => toHta(graph.list()),
    "host/describe": async () => toHta(hostDescription),
    "host/capabilities": async () => toHta(hostDescription.capabilities)
  };
  if (sessions) Object.assign(services, createSessionHostServices(graph, sessions));
  return services;
}

/** Compatibility ingress registration is intentionally a separate surface:
 * graph traffic never enters a Hara session unless that session explicitly
 * subscribes. The handler receives the owning HtaContext from HtaContext's
 * host-call invocation binding. */
export function createSessionHostServices(graph, sessions) {
  return {
    "session/register-ingress": function(sessionId, capabilities = []) {
      graph.capabilities?.grant(String(sessionId), toPlain(capabilities));
      return toHta(sessions.register(String(sessionId), this.context, {
        capabilities: toPlain(capabilities),
        onRelease: async (released) => graph.releaseSession(released)
      }));
    },
    "session/unregister-ingress": async (sessionId) => sessions.unregister(String(sessionId)),
    "session/subscribe": async (sessionId, signal, callbackId) =>
      sessions.subscribe(String(sessionId), String(signal), String(callbackId)),
    "session/unsubscribe": async (subscriptionId) => sessions.unsubscribe(String(subscriptionId))
  };
}

// Decoded shape: objects -> Maps with string keys, arrays -> arrays, scalars
// pass through (null -> nil on the hara side). String keys keep host-call
// arguments and store keys free of opaque keyword objects.
function fromJson(value) {
  if (Array.isArray(value)) return value.map(fromJson);
  if (value !== null && typeof value === "object") {
    return new Map(Object.entries(value).map(([key, item]) => [key, fromJson(item)]));
  }
  return value;
}

export function parseJson(source) {
  if (typeof source !== "string") throw new TypeError("json/parse expects text");
  let offset = 0;
  const whitespace = () => {
    while (/[ \t\n\r]/.test(source[offset] ?? "")) offset++;
  };
  const error = (message) => new Error(`json/parse: ${message} at character ${offset}`);
  const expect = (character) => {
    if (source[offset++] !== character) throw error(`expected '${character}'`);
  };
  const string = () => {
    expect("\"");
    let value = "";
    while (offset < source.length) {
      const character = source[offset++];
      if (character === "\"") return value;
      if (character < " ") throw error("invalid control character in string");
      if (character !== "\\") {
        value += character;
        continue;
      }
      const escaped = source[offset++];
      const escapes = { "\"": "\"", "\\": "\\", "/": "/", b: "\b", f: "\f", n: "\n", r: "\r", t: "\t" };
      if (escaped === "u") {
        const code = source.slice(offset, offset + 4);
        if (!/^[0-9a-fA-F]{4}$/.test(code)) throw error("invalid unicode escape");
        value += String.fromCharCode(Number.parseInt(code, 16));
        offset += 4;
      } else if (escaped in escapes) {
        value += escapes[escaped];
      } else {
        throw error("invalid escape sequence");
      }
    }
    throw error("unterminated string");
  };
  const number = () => {
    const start = offset;
    if (source[offset] === "-") offset++;
    if (source[offset] === "0") {
      offset++;
      if (/[0-9]/.test(source[offset] ?? "")) throw error("leading zero in JSON number");
    } else if (/[1-9]/.test(source[offset] ?? "")) {
      while (/[0-9]/.test(source[offset] ?? "")) offset++;
    } else {
      throw error("invalid JSON number");
    }
    let integer = true;
    if (source[offset] === ".") {
      integer = false;
      offset++;
      if (!/[0-9]/.test(source[offset] ?? "")) throw error("fraction requires digits");
      while (/[0-9]/.test(source[offset] ?? "")) offset++;
    }
    if (source[offset] === "e" || source[offset] === "E") {
      integer = false;
      offset++;
      if (source[offset] === "+" || source[offset] === "-") offset++;
      if (!/[0-9]/.test(source[offset] ?? "")) throw error("exponent requires digits");
      while (/[0-9]/.test(source[offset] ?? "")) offset++;
    }
    const text = source.slice(start, offset);
    if (integer) {
      try {
        const value = BigInt(text);
        return value >= BigInt(Number.MIN_SAFE_INTEGER) && value <= BigInt(Number.MAX_SAFE_INTEGER)
          ? Number(value) : value;
      } catch {
        throw error("invalid JSON integer");
      }
    }
    const value = Number(text);
    if (!Number.isFinite(value)) throw error("JSON number is outside the floating-point range");
    return value;
  };
  const value = (depth = 0) => {
    if (depth > 256) throw error("JSON nesting exceeds 256");
    whitespace();
    switch (source[offset]) {
      case "n": if (source.slice(offset, offset + 4) !== "null") throw error("invalid JSON token"); offset += 4; return null;
      case "t": if (source.slice(offset, offset + 4) !== "true") throw error("invalid JSON token"); offset += 4; return true;
      case "f": if (source.slice(offset, offset + 5) !== "false") throw error("invalid JSON token"); offset += 5; return false;
      case "\"": return string();
      case "[": {
        offset++;
        whitespace();
        const result = [];
        if (source[offset] === "]") { offset++; return result; }
        for (;;) {
          result.push(value(depth + 1));
          whitespace();
          if (source[offset] === "]") { offset++; return result; }
          expect(",");
          whitespace();
          if (source[offset] === "]") throw error("trailing comma");
        }
      }
      case "{": {
        offset++;
        whitespace();
        const result = Object.create(null);
        if (source[offset] === "}") { offset++; return result; }
        for (;;) {
          if (source[offset] !== "\"") throw error("object keys must be strings");
          const key = string();
          whitespace();
          expect(":");
          result[key] = value(depth + 1);
          whitespace();
          if (source[offset] === "}") { offset++; return result; }
          expect(",");
          whitespace();
          if (source[offset] === "}") throw error("trailing comma");
        }
      }
      default: return number();
    }
  };
  const result = value();
  whitespace();
  if (offset !== source.length) throw error("trailing content");
  return result;
}

function toPlain(value) {
  if (value instanceof Map) {
    return Object.fromEntries([...value].map(([key, entry]) => [
      key?.constructor?.name === "HtaKeyword" ? key.name : String(key),
      toPlain(entry)
    ]));
  }
  if (Array.isArray(value)) return value.map(toPlain);
  return value;
}

function toHta(value) {
  if (Array.isArray(value)) return value.map(toHta);
  // Maps have already crossed an HTA boundary (or were deliberately built for
  // one), so preserve them as-is. Walking them again can recurse through host
  // runtime state, while encodeHta already knows how to serialize their values.
  if (value instanceof Map) return value;
  if (value !== null && typeof value === "object" &&
      !(value instanceof Uint8Array) && !(value instanceof ArrayBuffer) && !ArrayBuffer.isView(value)) {
    return new Map(Object.entries(value).map(([key, entry]) => [key, toHta(entry)]));
  }
  return value;
}

export function stringifyJson(value) {
  const encode = (value, inArray = false) => {
    if (value === null || value === undefined) return "null";
    if (typeof value === "boolean") return value ? "true" : "false";
    if (typeof value === "bigint") return value.toString();
    if (typeof value === "number") {
      if (!Number.isFinite(value)) throw new TypeError("json/write cannot encode non-finite numbers");
      return Object.is(value, -0) ? "0" : String(value);
    }
    if (typeof value === "string") return JSON.stringify(value);
    if (Array.isArray(value)) return `[${value.map((item) => encode(item, true)).join(",")}]`;
    if (typeof value === "object") {
      const entries = Object.entries(value)
        .filter(([, item]) => item !== undefined && typeof item !== "function" && typeof item !== "symbol")
        .map(([key, item]) => `${JSON.stringify(key)}:${encode(item)}`);
      return `{${entries.join(",")}}`;
    }
    if (inArray) return "null";
    throw new TypeError("json/write cannot encode this value");
  };
  return encode(value);
}

async function request(store, method, ...arguments_) {
  return new Promise((resolve, reject) => {
    const operation = store[method](...arguments_);
    operation.onsuccess = () => resolve(operation.result ?? null);
    operation.onerror = () => reject(operation.error);
  });
}
