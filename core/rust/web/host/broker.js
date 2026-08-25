import { HtaContext } from "../packages/hta/index.js";

const ROOT = "ROOT";
const NAME_PATTERN = /^[A-Za-z0-9_.-]+$/;

/**
 * Owns studio kernel lifecycle: one kernel = one Web Worker running one raw
 * HTA wasm instance, and each kernel owns many isolated evaluator sessions.
 * Its per-kernel session lifecycle mirrors the JVM `SessionKernel`
 * (java/src/main/java/hara/truffle/SessionKernel.java).
 * normalization (reject, never lowercase), same error codes
 * (INVALID_SESSION_NAME, SESSION_EXISTS, NO_SESSION, ROOT_CANNOT_CLOSE), and
 * an always-present ROOT kernel.
 *
 * ROOT is created lazily: the first async access (`eval`/`require`) or sync
 * listing (`list`/`size`) triggers the spawn, and the in-flight promise is
 * cached so concurrent first access cannot double-spawn. A failed ROOT spawn
 * is forgotten, so later access retries. `require` is async (unlike the JVM)
 * because spawning a worker is async; `list`/`size` are sync snapshots that
 * report ROOT once its spawn has been triggered.
 */
export class KernelBroker {
  constructor({
    spawn, resources = {}, halcResources = [], hirResources, onKernelStarting = async () => {},
    onKernelCreated = async () => {}, onKernelClosed = async () => {}
  }) {
    this.spawn = spawn;
    this.resources = resources;
    this.halcResources = hirResources ?? halcResources;
    this.onKernelStarting = onKernelStarting;
    this.onKernelCreated = onKernelCreated;
    this.onKernelClosed = onKernelClosed;
    this.kernels = new Map(); // name -> { name, context, worker }
    this.pending = new Map(); // name -> in-flight create promise
    this.rootStart = null; // in-flight ROOT spawn promise, once triggered
    this.documents = new Map(); // kernel/document -> active private generation
    this.documentGenerations = new Map();
    this.previews = new Map(); // generation id -> disposable InstaREPL session + traces
    this.nextPreviewGeneration = 1;
  }

  static normalizeName(value) {
    if (typeof value !== "string" || value.length === 0 || !NAME_PATTERN.test(value)) {
      throw new Error("INVALID_SESSION_NAME");
    }
    return value;
  }

  async create(name, { bootstrap } = {}) {
    name = KernelBroker.normalizeName(name);
    if (name === ROOT) await this.rootKernel(); // settle any in-flight ROOT first
    if (this.kernels.has(name) || this.pending.has(name)) throw new Error(`SESSION_EXISTS ${name}`);
    const boot = this.boot(name, bootstrap);
    this.pending.set(name, boot);
    try {
      const kernel = await boot;
      this.kernels.set(name, kernel);
      await this.afterCreate(kernel);
      return kernel;
    } finally {
      this.pending.delete(name);
    }
  }

  async require(name) {
    if (name === ROOT) return this.rootKernel();
    const kernel = this.kernels.get(name);
    if (!kernel) throw new Error(`NO_SESSION ${name}`);
    return kernel;
  }

  async eval(name, source) {
    const kernel = await this.require(name);
    return kernel.context.call("eval", [source]);
  }

  async createSession(kernelName, sessionName, { filesystem = null, bootstrap } = {}) {
    const kernel = await this.require(kernelName);
    sessionName = KernelBroker.normalizeName(sessionName);
    if (sessionName === ROOT || kernel.sessions?.has(sessionName)) {
      throw new Error(`SESSION_EXISTS ${sessionName}`);
    }
    await kernel.context.call("session/create", [sessionName]);
    kernel.sessions ??= new Set([ROOT]);
    kernel.sessions.add(sessionName);
    try {
      if (this.halcResources.length) {
        await kernel.context.call("session/eval-halc-bundle", [sessionName, this.halcResources]);
      }
      if (filesystem !== null) {
        await kernel.context.session(sessionName).attachFilesystem(filesystem);
      }
      if (bootstrap !== undefined) {
        await kernel.context.call("session/eval", [sessionName, bootstrap]);
      }
      return { kernel: kernelName, name: sessionName, context: kernel.context };
    } catch (error) {
      await kernel.context.call("session/close", [sessionName]).catch(() => {});
      kernel.sessions.delete(sessionName);
      throw error;
    }
  }

  async evalSession(kernelName, sessionName, source) {
    const kernel = await this.require(kernelName);
    if (sessionName !== ROOT && !kernel.sessions?.has(sessionName)) {
      throw new Error(`NO_SESSION ${sessionName}`);
    }
    return kernel.context.call("session/eval", [sessionName, source]);
  }

  async traceEval(kernelName, sessionName, source) {
    const kernel = await this.require(KernelBroker.normalizeName(kernelName));
    sessionName = KernelBroker.normalizeName(sessionName);
    if (sessionName !== ROOT && !kernel.sessions?.has(sessionName)) {
      throw new Error(`NO_SESSION ${sessionName}`);
    }
    return kernel.context.call("session/trace-eval", [sessionName, source]);
  }

  /** Legacy form timing projection retained for older hosts. New Studio UI
   * uses traceEval/previewDocument and the structured evaluator trace. */
  async traceForms(kernelName, forms, {
    now = () => globalThis.performance?.now?.() ?? Date.now(),
    bootstrap
  } = {}) {
    if (!Array.isArray(forms)) throw new Error("TRACE_FORMS_MUST_BE_ARRAY");
    kernelName = KernelBroker.normalizeName(kernelName);
    const sessionName = `TRACE.${Date.now()}.${Math.random().toString(36).slice(2, 10)}`;
    await this.createSession(kernelName, sessionName, { bootstrap });
    const rows = [];
    try {
      for (const form of forms) {
        const startedAt = now();
        try {
          const value = await this.evalSession(kernelName, sessionName, form.source);
          rows.push({ ...form, status: "ok", value, duration: now() - startedAt });
        } catch (error) {
          rows.push({
            ...form, status: "error",
            error: String(error?.message ?? error),
            duration: now() - startedAt
          });
          break;
        }
      }
      return rows;
    } finally {
      await this.closeSession(kernelName, sessionName).catch(() => {});
    }
  }

  /** Evaluate complete forms in a clean disposable session and retain each
   * structured trace for result-gutter click-through. */
  async previewDocument(kernelName, documentId, forms, { bootstrap } = {}) {
    if (typeof documentId !== "string" || !documentId) throw new Error("INVALID_DOCUMENT_ID");
    if (!Array.isArray(forms)) throw new Error("PREVIEW_FORMS_MUST_BE_ARRAY");
    kernelName = KernelBroker.normalizeName(kernelName);
    const generationId = `preview-${this.nextPreviewGeneration++}`;
    const sessionName = `PREVIEW.${generationId.replace("-", ".")}`;
    await this.createSession(kernelName, sessionName, { bootstrap });
    const preview = {
      generationId, kernelName, documentId, sessionName,
      traces: new Map(), disposed: false
    };
    this.previews.set(generationId, preview);
    const rows = [];
    try {
      for (let index = 0; index < forms.length; index += 1) {
        const form = forms[index];
        const trace = await this.traceEval(kernelName, sessionName, form.source);
        const traceId = String(traceField(trace, "trace-id") ?? `${generationId}-${index + 1}`);
        preview.traces.set(traceId, trace);
        const status = traceName(traceField(trace, "status"));
        const result = traceField(trace, "result");
        rows.push({
          ...form,
          status: status === "error" ? "error" : "ok",
          value: previewDisplay(result),
          valueType: traceField(result, "type") ?? null,
          error: traceField(trace, "error") ?? null,
          traceId
        });
        if (status === "error") {
          for (const skipped of forms.slice(index + 1)) {
            rows.push({ ...skipped, status: "skipped", traceId: null });
          }
          break;
        }
      }
      return { generationId, sessionName, rows };
    } catch (error) {
      await this.disposePreview(generationId);
      throw error;
    }
  }

  getPreviewTrace(generationId, traceId) {
    const preview = this.previews.get(generationId);
    if (!preview || preview.disposed) throw new Error(`NO_PREVIEW ${generationId}`);
    const trace = preview.traces.get(traceId);
    if (!trace) throw new Error(`NO_PREVIEW_TRACE ${traceId}`);
    return trace;
  }

  async disposePreview(generationId) {
    const preview = this.previews.get(generationId);
    if (!preview) return false;
    this.previews.delete(generationId);
    preview.disposed = true;
    preview.traces.clear();
    await this.closeSession(preview.kernelName, preview.sessionName).catch(() => {});
    return true;
  }

  async closeSession(kernelName, sessionName) {
    const kernel = await this.require(kernelName);
    sessionName = KernelBroker.normalizeName(sessionName);
    if (sessionName === ROOT) throw new Error("ROOT_CANNOT_CLOSE");
    if (!kernel.sessions?.has(sessionName)) throw new Error(`NO_SESSION ${sessionName}`);
    await kernel.context.call("session/close", [sessionName]);
    kernel.sessions.delete(sessionName);
    return true;
  }

  async listSessions(kernelName) {
    const kernel = await this.require(kernelName);
    return kernel.context.call("session/list", []);
  }

  /**
   * Evaluate an ns+ document in an isolated candidate session. Only a
   * successful candidate becomes active; the previous generation is then
   * closed. Document sessions share the selected kernel's worker and never
   * appear in the top-level kernel list.
   */
  async evalDocument(name, documentId, source, { nodeId = null } = {}) {
    const prepared = await this.prepareDocument(name, documentId, source, { nodeId });
    this.commitDocument(prepared);
    return documentResult(prepared);
  }

  /**
   * Evaluate a candidate anonymous generation without disturbing the active
   * document. Callers can run it until its first visible frame, then commit,
   * or discard it to preserve the previous generation.
   */
  async prepareDocument(name, documentId, source, { nodeId = null } = {}) {
    name = KernelBroker.normalizeName(name);
    if (typeof documentId !== "string" || documentId.length === 0) {
      throw new Error("INVALID_DOCUMENT_ID");
    }
    const compiled = compileAnonymousDocument(source, { documentId, nodeId });
    const key = `${name}\u0000${documentId}`;
    const generation = (this.documentGenerations.get(key) ?? 0) + 1;
    const sessionName = `DOC.${safeName(documentId)}.${generation}`;
    const kernel = await this.require(name);
    await this.createSession(name, sessionName);
    const context = kernel.context.session(sessionName);
    const candidate = {
      name: sessionName,
      sessionName,
      context,
      worker: kernel.worker,
      sharedWorker: true,
      kernelRecord: kernel
    };
    try {
      const value = await context.call("eval", [compiled.source]);
      return {
        ...candidate,
        key,
        kernel: name,
        documentId,
        nodeId,
        generation,
        moduleId: compiled.moduleId,
        value,
        prepared: true
      };
    } catch (error) {
      await context.close();
      kernel.sessions?.delete(sessionName);
      throw error;
    }
  }

  commitDocument(candidate) {
    if (!candidate?.prepared) throw new Error("INVALID_DOCUMENT_CANDIDATE");
    const previous = this.documents.get(candidate.key);
    candidate.prepared = false;
    this.documents.set(candidate.key, candidate);
    this.documentGenerations.set(candidate.key, candidate.generation);
    closeDocumentSession(previous);
    return documentResult(candidate);
  }

  discardDocument(candidate) {
    if (!candidate?.prepared) return false;
    candidate.prepared = false;
    closeDocumentSession(candidate);
    return true;
  }

  async evalForm(name, documentId, source) {
    const document = this.requireDocument(name, documentId);
    return document.context.call("eval", [expandBuiltinAliases(source)]);
  }

  async evalPreparedDocument(candidate, source) {
    if (!candidate?.prepared) throw new Error("INVALID_DOCUMENT_CANDIDATE");
    return candidate.context.call("eval", [expandBuiltinAliases(source)]);
  }

  releaseDocument(name, documentId) {
    const key = `${name}\u0000${documentId}`;
    const document = this.documents.get(key);
    if (!document) return false;
    this.documents.delete(key);
    closeDocumentSession(document);
    return true;
  }

  requireDocument(name, documentId) {
    const document = this.documents.get(`${name}\u0000${documentId}`);
    if (!document) throw new Error(`NO_DOCUMENT ${documentId}`);
    return document;
  }

  hasDocument(name, documentId) {
    return this.documents.has(`${name}\u0000${documentId}`);
  }

  async close(name) {
    name = KernelBroker.normalizeName(name);
    if (name === ROOT) throw new Error("ROOT_CANNOT_CLOSE");
    const kernel = this.kernels.get(name);
    if (!kernel) throw new Error(`NO_SESSION ${name}`);
    this.kernels.delete(name);
    for (const document of [...this.documents.values()]) {
      if (document.kernel !== name) continue;
      this.releaseDocument(name, document.documentId);
    }
    for (const preview of [...this.previews.values()]) {
      if (preview.kernelName === name) await this.disposePreview(preview.generationId);
    }
    try {
      await this.onKernelClosed(kernel);
    } finally {
      kernel.context?.close?.();
      kernel.worker?.terminate?.();
    }
  }

  list() {
    this.rootKernel().catch(() => {}); // trigger; failure surfaces on await
    const names = [...this.kernels.keys()];
    return names.includes(ROOT) ? names : [ROOT, ...names];
  }

  size() {
    return this.list().length;
  }

  rootKernel() {
    const existing = this.kernels.get(ROOT);
    if (existing) return Promise.resolve(existing);
    this.rootStart ??= this.boot(ROOT).then(
      (kernel) => {
        this.kernels.set(ROOT, kernel);
        return this.afterCreate(kernel).then(() => kernel);
      },
      (error) => {
        this.rootStart = null; // allow a later access to retry
        throw error;
      }
    );
    return this.rootStart;
  }

  // Spawns, registers resources, then evals the bootstrap source. A boot
  // failure terminates the half-started kernel and leaves nothing stored.
  async boot(name, bootstrap) {
    const { context, worker } = await this.spawn(name);
    try {
      await this.onKernelStarting({ name, context, worker });
      const resources = Object.entries(this.resources);
      if (resources.length > 0) {
        await context.call("register-resources", [resources]);
      }
      if (bootstrap !== undefined) await context.call("eval", [bootstrap]);
    } catch (error) {
      context?.close?.();
      worker?.terminate?.();
      throw error;
    }
    return { name, context, worker, sessions: new Set([ROOT]) };
  }

  async afterCreate(kernel) {
    try {
      await this.onKernelCreated(kernel);
    } catch (error) {
      this.kernels.delete(kernel.name);
      kernel.context?.close?.();
      kernel.worker?.terminate?.();
      throw error;
    }
  }
}

function traceField(value, name) {
  if (!(value instanceof Map)) return undefined;
  for (const [key, item] of value) {
    if ((key?.name ?? key) === name) return item;
  }
  return undefined;
}

function traceName(value) {
  return String(value?.name ?? value ?? "");
}

function previewDisplay(value) {
  return traceField(value, "display") ?? null;
}

function documentResult(document) {
  return {
    value: document.value,
    generation: document.generation,
    moduleId: document.moduleId,
    private: true
  };
}

export function compileAnonymousDocument(source, { documentId, nodeId = null } = {}) {
  if (typeof source !== "string") throw new Error("DOCUMENT_SOURCE_MUST_BE_STRING");
  const form = firstEffectiveForm(source);
  if (!form || form.head !== "ns+") throw new Error("NS_PLUS_MUST_BE_FIRST_EFFECTIVE_FORM");
  const generationToken = `${safeName(documentId)}.${Math.random().toString(36).slice(2)}`;
  const moduleId = `anonymous:${generationToken}`;
  const nsSource = `${source.slice(form.start, form.headStart)}ns ${moduleId.replaceAll(":", ".")}${source.slice(form.headEnd, form.end)}`;
  const body = source.slice(form.end);
  const binding = bindNodeSource(`(do ${body})`, nodeId);
  return {
    moduleId,
    source: `${nsSource}\n(require [studio.node :as node])\n${binding}`
  };
}

function bindNodeSource(source, nodeId) {
  return `(do (set! node/*node-id* ${nodeId == null ? "nil" : JSON.stringify(nodeId)}) ${expandBuiltinAliases(source)})`;
}

// Raw HTA kernels implement the canonical coroutine special form directly.
// `co/` is nevertheless a language builtin, so document compilation resolves
// it without requiring or publicly registering a namespace.
function expandBuiltinAliases(source) {
  let output = "";
  let cursor = 0;
  let string = false;
  let comment = false;
  while (cursor < source.length) {
    const character = source[cursor];
    if (comment) {
      output += character;
      cursor += 1;
      if (character === "\n") comment = false;
      continue;
    }
    if (string) {
      output += character;
      cursor += 1;
      if (character === "\\") {
        output += source[cursor] ?? "";
        cursor += 1;
      } else if (character === '"') {
        string = false;
      }
      continue;
    }
    if (character === ";") {
      comment = true;
      output += character;
      cursor += 1;
      continue;
    }
    if (character === '"') {
      string = true;
      output += character;
      cursor += 1;
      continue;
    }
    if (source.startsWith("co/await", cursor) &&
        !/[A-Za-z0-9*+!?._/-]/.test(source[cursor - 1] ?? "") &&
        !/[A-Za-z0-9*+!?._/-]/.test(source[cursor + 8] ?? "")) {
      // co/ is a builtin alias. Lower it to the raw runtime's fiber-aware
      // special form without registering or requiring a public namespace.
      output += "std.foundation.coroutine/await";
      cursor += 8;
      continue;
    }
    output += character;
    cursor += 1;
  }
  return output;
}

function firstEffectiveForm(source) {
  let cursor = 0;
  while (cursor < source.length) {
    while (/\s|,/.test(source[cursor] ?? "")) cursor += 1;
    if (source[cursor] === ";") {
      const newline = source.indexOf("\n", cursor);
      cursor = newline < 0 ? source.length : newline + 1;
      continue;
    }
    if (source.startsWith("#_", cursor)) {
      cursor += 2;
      while (/\s|,/.test(source[cursor] ?? "")) cursor += 1;
      cursor = scanForm(source, cursor);
      continue;
    }
    break;
  }
  if (source[cursor] !== "(") return null;
  const start = cursor;
  cursor += 1;
  while (/\s|,/.test(source[cursor] ?? "")) cursor += 1;
  const headStart = cursor;
  while (cursor < source.length && !/[\s,()[\]{}";]/.test(source[cursor])) cursor += 1;
  const headEnd = cursor;
  return { start, headStart, headEnd, head: source.slice(headStart, headEnd), end: scanForm(source, start) };
}

function scanForm(source, start) {
  let cursor = start;
  const opening = source[cursor];
  if ('"'.includes(opening)) {
    cursor += 1;
    while (cursor < source.length) {
      if (source[cursor] === "\\") cursor += 2;
      else if (source[cursor++] === '"') return cursor;
    }
    throw new Error("UNTERMINATED_DOCUMENT_FORM");
  }
  const pairs = { "(": ")", "[": "]", "{": "}" };
  if (!pairs[opening]) {
    while (cursor < source.length && !/[\s,()[\]{}";]/.test(source[cursor])) cursor += 1;
    return cursor;
  }
  const stack = [pairs[opening]];
  cursor += 1;
  while (cursor < source.length && stack.length) {
    const character = source[cursor];
    if (character === ";") {
      const newline = source.indexOf("\n", cursor);
      cursor = newline < 0 ? source.length : newline + 1;
    } else if (character === '"') {
      cursor = scanForm(source, cursor);
    } else if (pairs[character]) {
      stack.push(pairs[character]);
      cursor += 1;
    } else if (character === stack.at(-1)) {
      stack.pop();
      cursor += 1;
    } else {
      cursor += 1;
    }
  }
  if (stack.length) throw new Error("UNTERMINATED_DOCUMENT_FORM");
  return cursor;
}

function safeName(value) {
  return String(value).replace(/[^A-Za-z0-9_.-]/g, ".");
}

function closeDocumentSession(document) {
  if (!document?.context || document.sessionClosed) return false;
  document.sessionClosed = true;
  document.kernelRecord?.sessions?.delete(document.sessionName);
  document.context.close().catch(() => {});
  return true;
}

/**
 * Production wiring for the website and hara-chrome: a broker whose spawn
 * creates a module Worker plus an `HtaContext`. `hostCalls` is passed through
 * as-is (a shared map, e.g. `createHostServices()` output, possibly merged
 * with extra calls by the caller); `resources` registers into every kernel.
 */
function sharedWorkerPort(url) {
  const shared = new SharedWorker(url, { type: "module", name: "hara-runtime" });
  const port = shared.port;
  port.start();
  return {
    postMessage(message) { port.postMessage(message); },
    addEventListener(type, listener) { port.addEventListener(type, listener); },
    terminate() { port.close(); }
  };
}

export function createBrowserBroker({
  workerUrl, sharedWorkerUrl, moduleBytes, hostCalls = {}, resources, halcResources, hirResources,
  onKernelStarting, onKernelCreated, onKernelClosed
}) {
  return new KernelBroker({
    resources,
    halcResources,
    hirResources,
    onKernelStarting,
    onKernelCreated,
    onKernelClosed,
    spawn: async (name) => {
      const worker = sharedWorkerUrl && typeof SharedWorker !== "undefined"
        ? sharedWorkerPort(sharedWorkerUrl)
        : new Worker(workerUrl, { type: "module", name: `hara-kernel-${name}` });
      const context = new HtaContext({ worker, moduleBytes, hostCalls, kernelId: name });
      return { context, worker };
    }
  });
}
