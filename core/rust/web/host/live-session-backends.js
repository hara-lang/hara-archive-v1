import {
  LiveSessionError,
  requireBackendSession,
} from "./live-session-model.js";

export function createInterpreterLiveBackend(runtime) {
  if (!runtime || typeof runtime.startNamed !== "function") {
    throw new TypeError("interpreter live backend requires InterpreterObservationRuntime");
  }
  return Object.freeze({
    id: "interpreter",
    operations: Object.freeze([
      "snapshot",
      "step",
      "run",
      "resume",
      "resolve",
      "reject",
      "update",
      "reset",
      "cancel",
      "dispose",
    ]),
    replacementPolicies: Object.freeze([
      "restart",
      "replace-on-next-start",
    ]),
    sourceKinds: Object.freeze(["source"]),
    start({ sessionId, source }) {
      if (source.kind !== "source") {
        throw new LiveSessionError(
          "live-session/source-kind",
          "interpreter backend starts from source only",
          { backend: "interpreter", kind: source.kind },
        );
      }
      return runtime.startNamed(sessionId, source.sourceId, source.value);
    },
  });
}

export function createBytecodeLiveBackend(runtime) {
  if (!runtime || typeof runtime.compileNamed !== "function" ||
      typeof runtime.fromNamedArtifact !== "function") {
    throw new TypeError("HBC live backend requires BytecodeObservationRuntime");
  }
  return Object.freeze({
    id: "hbc",
    operations: Object.freeze([
      "snapshot",
      "step",
      "run",
      "pause",
      "resume",
      "resolve",
      "reject",
      "update",
      "reset",
      "cancel",
      "dispose",
    ]),
    replacementPolicies: Object.freeze([
      "restart",
      "replace-on-next-start",
    ]),
    sourceKinds: Object.freeze(["source", "artifact"]),
    start({ sessionId, source }) {
      const session = source.kind === "artifact"
        ? runtime.fromNamedArtifact(sessionId, source.sourceId, source.value)
        : runtime.compileNamed(sessionId, source.sourceId, source.value);
      return new BytecodeLiveSessionAdapter(session);
    },
  });
}

/**
 * Creates the browser adapter for a prepared whole-Wasm artifact. Instantiation
 * is asynchronous, so callers must use LiveSessionRuntime.dispatchAsync().
 * Whole-Wasm deliberately advertises only the operations its prepared module
 * can execute; it does not borrow interpreter or HBC controls.
 */
export function createWholeWasmLiveBackend({ instantiate, Host, fallback } = {}) {
  if (typeof instantiate !== "function") {
    throw new TypeError("whole-Wasm live backend requires instantiate()");
  }
  if (typeof Host !== "function") {
    throw new TypeError("whole-Wasm live backend requires a Host constructor");
  }
  return Object.freeze({
    id: "whole-wasm",
    operations: Object.freeze(["run", "call", "dispose"]),
    replacementPolicies: Object.freeze([]),
    sourceKinds: Object.freeze(["artifact"]),
    async start({ source }) {
      if (source.kind !== "artifact") {
        throw new LiveSessionError(
          "live-session/source-kind",
          "whole-Wasm backend starts from an artifact only",
          { backend: "whole-wasm", kind: source.kind },
        );
      }
      const module = await instantiate(source.value, Host, fallback);
      return new WholeWasmLiveSessionAdapter(module);
    },
  });
}

class BytecodeLiveSessionAdapter {
  constructor(session) {
    this.session = requireBackendSession(session, "hbc");
  }

  get status() { return this.session.status; }
  get sequence() { return this.session.sequence; }

  snapshot() { return this.session.snapshot(); }
  step() { return this.session.step(); }
  run(limit) { return this.session.run(limit); }
  pause() { return this.session.pause(); }
  resume(settlement) { return this.session.resume(settlement); }
  resolveSuspension(value) { return this.session.resolveSuspension(value); }
  rejectSuspension(error) { return this.session.rejectSuspension(error); }
  reset() { return this.session.reset(); }

  cancel() {
    return Object.freeze({ cancelled: this.session.dispose() });
  }

  dispose() { return this.session.dispose(); }
}

class WholeWasmLiveSessionAdapter {
  constructor(module) {
    if (!module || typeof module !== "object" ||
        typeof module.call !== "function") {
      throw new LiveSessionError(
        "live-session/backend-session",
        "whole-Wasm instantiation returned an invalid module",
      );
    }
    this.module = module;
    this.status = "ready";
    this.sequence = 0;
    this.disposed = false;
  }

  run() {
    return this.execute("run", () => this.module.call());
  }

  call(functionId, arguments_ = []) {
    if (functionId !== 0 && functionId !== 0n) {
      throw new LiveSessionError(
        "live-session/unsupported-operation",
        "browser whole-Wasm calls currently target the prepared entrypoint",
        { function: functionId },
      );
    }
    if (!Array.isArray(arguments_)) {
      throw new TypeError("whole-Wasm call arguments must be an array");
    }
    return this.execute("call", () => typeof this.module.callFunction === "function"
      ? this.module.callFunction(
        typeof this.module.entryFunction === "function"
          ? this.module.entryFunction()
          : 0,
        ...arguments_,
      )
      : this.module.call(...arguments_));
  }

  dispose() {
    if (this.disposed) return false;
    this.disposed = true;
    this.status = "disposed";
    this.module = null;
    return true;
  }

  execute(operation, invoke) {
    if (this.disposed) {
      throw new LiveSessionError(
        "live-session/disposed",
        "whole-Wasm session is disposed",
      );
    }
    try {
      const result = invoke();
      this.sequence += 1;
      this.status = "returned";
      return Object.freeze({
        operation,
        status: this.status,
        result,
        sequence: this.sequence,
      });
    } catch (error) {
      this.sequence += 1;
      this.status = "failed";
      throw error;
    }
  }
}
