import {
  DEFAULT_RUN_LIMIT,
  LIVE_SESSION_CAPABILITIES_SCHEMA,
  LIVE_SESSION_PROTOCOL,
  LIVE_SESSION_REPLY_SCHEMA,
  LIVE_SESSION_STATE_SCHEMA,
  LiveSessionError,
  MAX_SAFE_INTEGER,
  boundedInteger,
  canonicalStatus,
  capabilitiesDocument,
  disposeBackendSession,
  field,
  immutableProjection,
  increment,
  nonEmptyString,
  normalizeBackend,
  normalizeRequest,
  normalizeSettlement,
  normalizeSource,
  readBackendSequence,
  replacementPolicy,
  requireBackendSession,
  sourceSummary,
  supportsOperation,
  supportsReplacement,
  supportsSourceKind,
} from "./live-session-model.js";
import {
  createBytecodeLiveBackend,
  createInterpreterLiveBackend,
  createWholeWasmLiveBackend,
} from "./live-session-backends.js";

export {
  LIVE_SESSION_CAPABILITIES_SCHEMA,
  LIVE_SESSION_PROTOCOL,
  LIVE_SESSION_REPLY_SCHEMA,
  LIVE_SESSION_STATE_SCHEMA,
  LiveSessionError,
  createBytecodeLiveBackend,
  createInterpreterLiveBackend,
  createWholeWasmLiveBackend,
};

/**
 * Browser host for backend-neutral live sessions.
 *
 * This object owns only session identity, source revisions, lifecycle fencing,
 * replacement policy, and JSON projection. Interpreter and HBC sessions retain
 * their executable values, continuations, frames, promises, and evidence.
 */
export class LiveSessionRuntime {
  constructor({ backends = [], maxSessions = 256, diagnostics = () => {} } = {}) {
    this.backends = new Map();
    this.sessions = new Map();
    this.starting = new Set();
    this.maxSessions = boundedInteger(
      maxSessions,
      "live session limit",
      MAX_SAFE_INTEGER,
      1,
    );
    this.diagnostics = typeof diagnostics === "function" ? diagnostics : () => {};
    this.disposed = false;
    for (const backend of backends) this.registerBackend(backend);
  }

  registerBackend(backend) {
    this.assertActive();
    const descriptor = normalizeBackend(backend);
    if (this.backends.has(descriptor.id)) {
      throw new LiveSessionError(
        "live-session/backend-exists",
        `live session backend already registered: ${descriptor.id}`,
      );
    }
    this.backends.set(descriptor.id, descriptor);
    return this.backendCapabilities(descriptor.id);
  }

  backendCapabilities(backendId) {
    return capabilitiesDocument(this.requireBackend(backendId));
  }

  listBackends() {
    return Object.freeze(
      [...this.backends.keys()].sort().map((id) => this.backendCapabilities(id)),
    );
  }

  listSessions() {
    return Object.freeze([...this.sessions.values()].map((session) => session.state()));
  }

  dispatch(value) {
    this.assertActive();
    const request = normalizeRequest(value);
    if (request.op === "start") return this.start(request);
    return this.requireSession(request.sessionId).dispatch(request);
  }

  start(request) {
    const { backend, source } = this.prepareStart(request);
    let backendSession;
    try {
      backendSession = backend.start({
        sessionId: request.sessionId,
        source,
        payload: request.payload,
      });
      return this.finishStart(request, backend, source, backendSession);
    } catch (error) {
      this.starting.delete(request.sessionId);
      throw error;
    }
  }

  async dispatchAsync(value) {
    this.assertActive();
    const request = normalizeRequest(value);
    if (request.op === "start") return this.startAsync(request);
    return this.requireSession(request.sessionId).dispatch(request);
  }

  async startAsync(request) {
    const { backend, source } = this.prepareStart(request);
    try {
      const backendSession = await backend.start({
        sessionId: request.sessionId,
        source,
        payload: request.payload,
      });
      if (this.disposed) {
        disposeBackendSession(backendSession);
        throw new LiveSessionError(
          "live-session/runtime-disposed",
          "live session runtime was disposed during startup",
        );
      }
      return this.finishStart(request, backend, source, backendSession);
    } catch (error) {
      this.starting.delete(request.sessionId);
      throw error;
    }
  }

  prepareStart(request) {
    if (request.op !== "start") {
      throw new LiveSessionError("live-session/operation", "start requires op=start");
    }
    if (this.sessions.has(request.sessionId) || this.starting.has(request.sessionId)) {
      throw new LiveSessionError(
        "live-session/already-exists",
        `live session identity cannot be reused: ${request.sessionId}`,
        this.sessions.get(request.sessionId).state(),
      );
    }
    const activeSessions = [...this.sessions.values()]
      .filter((session) => !session.disposed).length + this.starting.size;
    if (activeSessions >= this.maxSessions) {
      throw new LiveSessionError(
        "live-session/limit",
        `live session limit ${this.maxSessions} reached`,
      );
    }

    const backendId = nonEmptyString(
      field(request.payload, "backend") ?? field(request.raw, "backend"),
      "live session backend",
    );
    const backend = this.requireBackend(backendId);
    const source = normalizeSource(sourcePayload(request));
    if (!supportsSourceKind(backend, source.kind)) {
      throw new LiveSessionError(
        "live-session/source-kind",
        `${backend.id} backend does not support ${source.kind} input`,
        { backend: backend.id, kind: source.kind },
      );
    }
    this.starting.add(request.sessionId);
    return { backend, source };
  }

  finishStart(request, backend, source, backendSession) {
    let session;
    try {
      session = new BrowserLiveSession({
        backend,
        backendSession,
        sessionId: request.sessionId,
        source,
      });
    } catch (error) {
      disposeBackendSession(backendSession);
      this.starting.delete(request.sessionId);
      throw error;
    }
    this.starting.delete(request.sessionId);
    this.sessions.set(request.sessionId, session);
    const reply = session.reply(request.requestId, {
      started: true,
      source: sourceSummary(source),
      capabilities: session.capabilities(),
    });
    this.emitDiagnostic("live-session/started", reply.state);
    return reply;
  }

  info(sessionId) {
    return this.requireSession(nonEmptyString(sessionId, "live session id")).state();
  }

  dispose() {
    if (this.disposed) return false;
    this.disposed = true;
    for (const session of this.sessions.values()) session.disposeBackend();
    this.sessions.clear();
    this.starting.clear();
    return true;
  }

  assertActive() {
    if (this.disposed) {
      throw new LiveSessionError(
        "live-session/runtime-disposed",
        "live session runtime is disposed",
      );
    }
  }

  requireBackend(backendId) {
    const id = nonEmptyString(backendId, "live session backend id");
    const backend = this.backends.get(id);
    if (!backend) {
      throw new LiveSessionError(
        "live-session/backend-not-found",
        `unknown live session backend: ${id}`,
        { backend: id },
      );
    }
    return backend;
  }

  requireSession(sessionId) {
    const session = this.sessions.get(sessionId);
    if (!session) {
      throw new LiveSessionError(
        "live-session/not-found",
        `unknown live session: ${sessionId}`,
        { "session-id": sessionId },
      );
    }
    return session;
  }

  emitDiagnostic(kind, detail) {
    this.diagnostics(Object.freeze({ kind, detail: immutableProjection(detail) }));
  }
}

class BrowserLiveSession {
  constructor({ backend, backendSession, sessionId, source }) {
    this.backend = backend;
    this.backendSession = requireBackendSession(backendSession, backend.id);
    this.sessionId = sessionId;
    this.source = source;
    this.generation = 0;
    this.observedSequence = readBackendSequence(this.backendSession, this.backend.id);
    this.pendingSource = null;
    this.terminalStatus = null;
    this.disposed = false;
    const initialStatus = canonicalStatus(this.backendSession, this.backend.id);
    if (initialStatus === "cancelled" || initialStatus === "disposed") {
      throw new LiveSessionError(
        "live-session/backend-status",
        `backend ${this.backend.id} started in terminal status ${initialStatus}`,
      );
    }
  }

  capabilities() {
    return capabilitiesDocument(this.backend);
  }

  dispatch(request) {
    this.assertFence(request);
    this.assertTerminal(request.op);
    if (!supportsOperation(this.backend, request.op)) {
      throw new LiveSessionError(
        "live-session/unsupported-operation",
        `${this.backend.id} backend does not support ${request.op}`,
        { backend: this.backend.id, operation: request.op },
      );
    }

    let payload;
    switch (request.op) {
      case "snapshot":
        payload = this.callRequired("snapshot");
        break;
      case "step":
        payload = this.callRequired("step");
        break;
      case "run":
        payload = this.callRequired("run", [boundedInteger(
          field(request.payload, "boundary-limit", "boundaryLimit") ??
            field(request.payload, "limit") ?? DEFAULT_RUN_LIMIT,
          "live session run boundary limit",
          100_000,
        )]);
        break;
      case "call":
        payload = this.callRequired("call", [
          field(request.payload, "function"),
          field(request.payload, "arguments") ?? [],
        ]);
        break;
      case "pause":
        payload = this.callRequired("pause");
        break;
      case "resume":
        payload = this.callRequired("resume", [
          normalizeSettlement(field(request.payload, "settlement")),
        ]);
        break;
      case "resolve":
        payload = this.callRequired("resolveSuspension", [
          field(request.payload, "value") ?? null,
        ]);
        break;
      case "reject":
        payload = this.callRequired("rejectSuspension", [
          field(request.payload, "error") ?? null,
        ]);
        break;
      case "update":
        payload = this.update(request);
        break;
      case "reset":
        payload = this.reset();
        break;
      case "cancel":
        payload = this.cancel();
        break;
      case "dispose":
        payload = this.disposeBackend();
        break;
      default:
        throw new LiveSessionError(
          "live-session/unsupported-operation",
          `unsupported live session operation: ${request.op}`,
        );
    }
    return this.reply(request.requestId, payload);
  }

  update(request) {
    const policy = replacementPolicy(
      field(request.payload, "policy") ?? "restart",
    );
    if (!supportsReplacement(this.backend, policy)) {
      throw new LiveSessionError(
        "live-session/unsupported-replacement",
        `${this.backend.id} backend does not support ${policy} replacement`,
        { backend: this.backend.id, policy },
      );
    }
    const source = normalizeSource(request.payload);
    if (!supportsSourceKind(this.backend, source.kind)) {
      throw new LiveSessionError(
        "live-session/source-kind",
        `${this.backend.id} backend does not support ${source.kind} input`,
        { backend: this.backend.id, kind: source.kind },
      );
    }

    if (policy === "replace-on-next-start") {
      this.pendingSource = source;
      return Object.freeze({
        accepted: true,
        activation: "next-start",
        revision: source.revision,
      });
    }

    if (policy === "preserve-runtime") {
      return this.preserveRuntime(source, request.payload);
    }

    return this.restart(source, request.payload);
  }

  preserveRuntime(source, payload) {
    if (["running", "paused", "suspended"].includes(this.status())) {
      throw new LiveSessionError(
        "live-session/active-continuation",
        "preserve-runtime replacement requires an inactive continuation",
        this.state(),
      );
    }
    if (typeof this.backendSession.replaceRuntime !== "function") {
      throw new LiveSessionError(
        "live-session/unsupported-replacement",
        `${this.backend.id} backend does not implement preserve-runtime replacement`,
        { backend: this.backend.id, policy: "preserve-runtime" },
      );
    }
    const nextGeneration = increment(this.generation, "live session generation");
    const result = this.backendSession.replaceRuntime({ source, payload });
    this.source = source;
    this.generation = nextGeneration;
    this.pendingSource = null;
    this.terminalStatus = null;
    this.observedSequence = readBackendSequence(this.backendSession, this.backend.id);
    return result;
  }

  reset() {
    if (this.pendingSource) {
      const source = this.pendingSource;
      try {
        return this.restart(source, {});
      } catch (error) {
        this.pendingSource = source;
        throw error;
      }
    }
    const nextGeneration = increment(this.generation, "live session generation");
    const payload = this.callRequired("reset");
    this.generation = nextGeneration;
    this.terminalStatus = null;
    this.observedSequence = readBackendSequence(this.backendSession, this.backend.id);
    return payload;
  }

  restart(source, payload) {
    const nextGeneration = increment(this.generation, "live session generation");
    const candidate = requireBackendSession(this.backend.start({
      sessionId: this.sessionId,
      source,
      payload,
    }), this.backend.id);
    let candidateSequence;
    let candidateStatus;
    let candidatePayload;
    try {
      candidateSequence = readBackendSequence(candidate, this.backend.id);
      candidateStatus = canonicalStatus(candidate, this.backend.id);
      if (candidateStatus === "cancelled" || candidateStatus === "disposed") {
        throw new LiveSessionError(
          "live-session/backend-status",
          `replacement ${this.backend.id} backend started in ${candidateStatus}`,
        );
      }
      candidatePayload = typeof candidate.snapshot === "function"
        ? candidate.snapshot()
        : null;
      candidateSequence = readBackendSequence(candidate, this.backend.id);
    } catch (error) {
      disposeBackendSession(candidate);
      throw error;
    }

    const previous = this.backendSession;
    try {
      disposeBackendSession(previous);
    } catch (error) {
      disposeBackendSession(candidate);
      throw error;
    }

    this.backendSession = candidate;
    this.source = source;
    this.generation = nextGeneration;
    this.pendingSource = null;
    this.terminalStatus = null;
    this.observedSequence = candidateSequence;
    return candidatePayload;
  }

  cancel() {
    const payload = typeof this.backendSession.cancel === "function"
      ? this.backendSession.cancel()
      : Object.freeze({ cancelled: disposeBackendSession(this.backendSession) });
    this.observedSequence = safeBackendSequence(
      this.backendSession,
      this.backend.id,
      this.observedSequence,
    );
    this.pendingSource = null;
    this.terminalStatus = "cancelled";
    return payload;
  }

  disposeBackend() {
    if (this.disposed) return false;
    this.observedSequence = safeBackendSequence(
      this.backendSession,
      this.backend.id,
      this.observedSequence,
    );
    const disposed = disposeBackendSession(this.backendSession);
    this.pendingSource = null;
    this.terminalStatus = "disposed";
    this.disposed = true;
    return disposed;
  }

  callRequired(method, args = []) {
    const operation = this.backendSession[method];
    if (typeof operation !== "function") {
      throw new LiveSessionError(
        "live-session/unsupported-operation",
        `${this.backend.id} backend does not implement ${method}`,
        { backend: this.backend.id, operation: method },
      );
    }
    return operation.apply(this.backendSession, args);
  }

  assertFence(request) {
    if (request.sessionId !== this.sessionId) {
      throw new LiveSessionError(
        "live-session/session-mismatch",
        `request targets session ${request.sessionId} but adapter owns ${this.sessionId}`,
      );
    }
    if (request.generation != null && request.generation !== this.generation) {
      throw new LiveSessionError(
        "live-session/stale-generation",
        `request generation ${request.generation} does not match current generation ${this.generation}`,
        this.state(),
      );
    }
    if (request.revision != null && request.revision !== this.source.revision) {
      throw new LiveSessionError(
        "live-session/stale-revision",
        `request revision ${request.revision} does not match current revision ${this.source.revision}`,
        this.state(),
      );
    }
  }

  assertTerminal(operation) {
    if (this.disposed && operation !== "dispose") {
      throw new LiveSessionError(
        "live-session/disposed",
        "disposed live session accepts only dispose",
        this.state(),
      );
    }
    if (this.terminalStatus === "cancelled" && operation !== "dispose") {
      throw new LiveSessionError(
        "live-session/cancelled",
        "cancelled live session accepts only dispose",
        this.state(),
      );
    }
  }

  status() {
    return canonicalStatus(this.backendSession, this.backend.id, this.terminalStatus);
  }

  sequence() {
    if (this.disposed || this.terminalStatus === "cancelled") {
      return this.observedSequence;
    }
    const sequence = readBackendSequence(this.backendSession, this.backend.id);
    if (sequence < this.observedSequence) {
      throw new LiveSessionError(
        "live-session/non-monotonic-sequence",
        `backend ${this.backend.id} sequence moved backwards from ${this.observedSequence} to ${sequence}`,
        {
          backend: this.backend.id,
          previous: this.observedSequence,
          sequence,
        },
      );
    }
    this.observedSequence = sequence;
    return sequence;
  }

  state() {
    return Object.freeze({
      schema: LIVE_SESSION_STATE_SCHEMA,
      protocol: LIVE_SESSION_PROTOCOL,
      "session-id": this.sessionId,
      "source-id": this.source.sourceId,
      generation: this.generation,
      revision: this.source.revision,
      sequence: this.sequence(),
      backend: this.backend.id,
      status: this.status(),
    });
  }

  reply(requestId, payload) {
    return Object.freeze({
      schema: LIVE_SESSION_REPLY_SCHEMA,
      protocol: LIVE_SESSION_PROTOCOL,
      "request-id": requestId,
      state: this.state(),
      payload: immutableProjection(payload),
    });
  }
}

function sourcePayload(request) {
  const payload = request.payload;
  const raw = request.raw;
  return {
    "source-id": field(payload, "source-id", "sourceId") ??
      field(raw, "source-id", "sourceId"),
    revision: field(payload, "revision") ?? field(raw, "revision"),
    source: field(payload, "source") ?? field(raw, "source"),
    artifact: field(payload, "artifact") ?? field(raw, "artifact"),
  };
}

function safeBackendSequence(session, backendId, fallback) {
  try {
    return readBackendSequence(session, backendId);
  } catch {
    return fallback;
  }
}
