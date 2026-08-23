import assert from "node:assert/strict";
import test from "node:test";
import {
  LIVE_SESSION_CAPABILITIES_SCHEMA,
  LIVE_SESSION_PROTOCOL,
  LIVE_SESSION_REPLY_SCHEMA,
  LIVE_SESSION_STATE_SCHEMA,
  LiveSessionError,
  LiveSessionRuntime,
  createBytecodeLiveBackend,
  createInterpreterLiveBackend,
  createWholeWasmLiveBackend,
} from "./host/live-session.js";

let nextRequestId = 0;
const send = (runtime, request) => runtime.dispatch({
  protocol: LIVE_SESSION_PROTOCOL,
  "request-id": `live-session-test/${++nextRequestId}`,
  ...request,
});

const startRequest = ({
  backend,
  sessionId,
  sourceId = `${sessionId}.hal`,
  revision,
  source = "(+ 20 22)",
  artifact = null,
}) => ({
  op: "start",
  "session-id": sessionId,
  payload: {
    backend,
    "source-id": sourceId,
    revision,
    ...(artifact == null ? { source } : { artifact }),
  },
});

class FakeObservationSession {
  constructor({ backend, sessionId, sourceId, input, cooperativeCancel = false }) {
    this.backend = backend;
    this.sessionId = sessionId;
    this.sourceId = sourceId;
    this.input = input;
    this.status = "ready";
    this.sequence = 0;
    this.disposed = false;
    if (cooperativeCancel) {
      this.cancel = () => {
        this.assertActive();
        this.status = "cancelled";
        return { status: "cancelled" };
      };
    }
  }

  snapshot() {
    this.assertActive();
    return {
      schema: `${this.backend}/snapshot`,
      sourceId: this.sourceId,
      sequence: this.sequence,
      status: this.status,
    };
  }

  step() {
    this.assertActive();
    this.status = "running";
    this.sequence += 1;
    return { schema: `${this.backend}/step`, sequence: this.sequence };
  }

  run(limit) {
    this.assertActive();
    this.status = "returned";
    this.sequence += Math.min(limit, 3);
    return { schema: `${this.backend}/run`, limit, sequence: this.sequence };
  }

  pause() {
    this.assertActive();
    this.status = "paused";
    return true;
  }

  resume(settlement = null) {
    this.assertActive();
    this.status = "ready";
    this.sequence += 1;
    return { schema: `${this.backend}/resume`, settlement };
  }

  resolveSuspension(value) {
    this.assertActive();
    return { accepted: true, value };
  }

  rejectSuspension(error) {
    this.assertActive();
    return { accepted: true, error };
  }

  reset() {
    this.assertActive();
    this.status = "ready";
    this.sequence = 0;
    return this.snapshot();
  }

  dispose() {
    if (this.disposed) return false;
    this.disposed = true;
    this.status = "disposed";
    return true;
  }

  assertActive() {
    if (this.disposed) throw new Error(`${this.backend} session is disposed`);
  }
}

class FakeInterpreterRuntime {
  constructor() {
    this.started = [];
  }

  startNamed(sessionId, sourceId, source) {
    if (source.includes("invalid")) throw new Error("invalid source");
    const session = new FakeObservationSession({
      backend: "interpreter",
      sessionId,
      sourceId,
      input: source,
      cooperativeCancel: true,
    });
    this.started.push(session);
    return session;
  }
}

class FakeBytecodeRuntime {
  constructor() {
    this.started = [];
  }

  compileNamed(sessionId, sourceId, source) {
    if (source.includes("invalid")) throw new Error("invalid source");
    return this.track(new FakeObservationSession({
      backend: "hbc",
      sessionId,
      sourceId,
      input: source,
    }));
  }

  fromNamedArtifact(sessionId, sourceId, artifact) {
    return this.track(new FakeObservationSession({
      backend: "hbc",
      sessionId,
      sourceId,
      input: new Uint8Array(artifact),
    }));
  }

  track(session) {
    this.started.push(session);
    return session;
  }
}

const runtimeFixture = (options = {}) => {
  const interpreter = new FakeInterpreterRuntime();
  const hbc = new FakeBytecodeRuntime();
  const runtime = new LiveSessionRuntime({
    backends: [
      createInterpreterLiveBackend(interpreter),
      createBytecodeLiveBackend(hbc),
    ],
    ...options,
  });
  return { runtime, interpreter, hbc };
};

test("browser serialization uses the native live-session schemas and statuses", () => {
  assert.equal(LIVE_SESSION_PROTOCOL, "hara.live-session/0-alpha");
  assert.equal(LIVE_SESSION_STATE_SCHEMA, "hara.live-session.state/0-alpha");
  assert.equal(LIVE_SESSION_REPLY_SCHEMA, "hara.live-session.reply/0-alpha");
  assert.equal(
    LIVE_SESSION_CAPABILITIES_SCHEMA,
    "hara.live-session.capabilities/0-alpha",
  );

  const { runtime } = runtimeFixture();
  const interpreterCapabilities = runtime.backendCapabilities("interpreter");
  assert.equal(interpreterCapabilities.schema, LIVE_SESSION_CAPABILITIES_SCHEMA);
  assert.equal(interpreterCapabilities.protocol, LIVE_SESSION_PROTOCOL);
  assert.deepEqual(interpreterCapabilities.operations, [
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
  ]);
  assert.deepEqual(interpreterCapabilities["replacement-policies"], [
    "restart",
    "replace-on-next-start",
  ]);

  for (const backend of ["interpreter", "hbc"]) {
    const sessionId = `lesson/${backend}`;
    const started = send(runtime, startRequest({
      backend,
      sessionId,
      revision: "sha256:revision-1",
    }));
    assert.equal(started.schema, LIVE_SESSION_REPLY_SCHEMA);
    assert.equal(started.protocol, LIVE_SESSION_PROTOCOL);
    assert.equal(started.state.schema, LIVE_SESSION_STATE_SCHEMA);
    assert.equal(started.state["session-id"], sessionId);
    assert.equal(started.state.backend, backend);
    assert.equal(started.state.status, "ready");
    assert.equal(started.state.generation, 0);
    assert.equal(started.state.sequence, 0);
    assert.equal(started.payload.capabilities.schema, LIVE_SESSION_CAPABILITIES_SCHEMA);

    const stepped = send(runtime, {
      op: "step",
      "session-id": sessionId,
      generation: 0,
      revision: "sha256:revision-1",
    });
    assert.equal(stepped.payload.schema, `${backend}/step`);
    assert.equal(stepped.state.status, "running");
    assert.equal(stepped.state.sequence, 1);

    const returned = send(runtime, {
      op: "run",
      "session-id": sessionId,
      generation: 0,
      revision: "sha256:revision-1",
      payload: { "boundary-limit": 10 },
    });
    assert.equal(returned.payload.schema, `${backend}/run`);
    assert.equal(returned.state.status, "returned");
    assert.equal(returned.state.sequence, 4);
  }
});

test("whole-Wasm live sessions use async instantiation and exact capabilities", async () => {
  let instantiations = 0;
  const backend = createWholeWasmLiveBackend({
    Host: class WholeWasmHost {},
    async instantiate(artifact, Host) {
      instantiations += 1;
      assert.ok(artifact instanceof Uint8Array);
      assert.equal(typeof Host, "function");
      await Promise.resolve();
      return { call: () => 42n };
    },
  });
  const runtime = new LiveSessionRuntime({ backends: [backend] });
  const sessionId = "lesson/whole-wasm";
  const revision = "sha256:artifact";
  const started = await runtime.dispatchAsync({
    protocol: LIVE_SESSION_PROTOCOL,
    "request-id": "whole-wasm/start",
    "session-id": sessionId,
    op: "start",
    payload: {
      "source-id": "lesson.hnw",
      revision,
      artifact: Uint8Array.from([72, 78, 87, 48]),
      backend: "whole-wasm",
    },
  });

  assert.equal(instantiations, 1);
  assert.equal(started.state.backend, "whole-wasm");
  assert.deepEqual(started.payload.capabilities.operations, ["run", "call", "dispose"]);
  assert.deepEqual(started.payload.capabilities["replacement-policies"], []);

  const result = await runtime.dispatchAsync({
    protocol: LIVE_SESSION_PROTOCOL,
    "request-id": "whole-wasm/run",
    "session-id": sessionId,
    generation: 0,
    revision,
    op: "run",
  });
  assert.equal(result.state.status, "returned");
  assert.equal(result.state.sequence, 1);
  assert.equal(result.payload.result, 42n);
});

test("generation and revision fences reject stale commands before mutation", () => {
  const { runtime, interpreter } = runtimeFixture();
  send(runtime, startRequest({
    backend: "interpreter",
    sessionId: "lesson/stale",
    revision: "sha256:first",
  }));
  const session = interpreter.started[0];

  assert.throws(
    () => send(runtime, {
      op: "step",
      "session-id": "lesson/stale",
      generation: 1,
      revision: "sha256:first",
    }),
    (error) => error instanceof LiveSessionError &&
      error.code === "live-session/stale-generation",
  );
  assert.throws(
    () => send(runtime, {
      op: "step",
      "session-id": "lesson/stale",
      generation: 0,
      revision: "sha256:other",
    }),
    (error) => error instanceof LiveSessionError &&
      error.code === "live-session/stale-revision",
  );
  assert.equal(session.sequence, 0);
  assert.equal(session.status, "ready");
});

test("restart replacement is transactional and advances one generation", () => {
  const { runtime, interpreter } = runtimeFixture();
  send(runtime, startRequest({
    backend: "interpreter",
    sessionId: "lesson/restart",
    revision: "sha256:first",
    source: "(+ 1 2)",
  }));
  const original = interpreter.started[0];
  const before = runtime.info("lesson/restart");

  assert.throws(
    () => send(runtime, {
      op: "update",
      "session-id": "lesson/restart",
      generation: 0,
      revision: "sha256:first",
      payload: {
        policy: "restart",
        "source-id": "replacement.hal",
        revision: "sha256:invalid",
        source: "invalid source",
      },
    }),
    /invalid source/,
  );
  assert.deepEqual(runtime.info("lesson/restart"), before);
  assert.equal(original.disposed, false);

  const replaced = send(runtime, {
    op: "update",
    "session-id": "lesson/restart",
    generation: 0,
    revision: "sha256:first",
    payload: {
      policy: "restart",
      "source-id": "replacement.hal",
      revision: "sha256:second",
      source: "(+ 40 2)",
    },
  });
  assert.equal(replaced.state.generation, 1);
  assert.equal(replaced.state.revision, "sha256:second");
  assert.equal(replaced.state["source-id"], "replacement.hal");
  assert.equal(original.disposed, true);
  assert.equal(interpreter.started.length, 2);
});

test("replace-on-next-start queues a revision until reset", () => {
  const { runtime, hbc } = runtimeFixture();
  send(runtime, startRequest({
    backend: "hbc",
    sessionId: "lesson/queued",
    revision: "sha256:first",
  }));
  const original = hbc.started[0];

  const queued = send(runtime, {
    op: "update",
    "session-id": "lesson/queued",
    generation: 0,
    revision: "sha256:first",
    payload: {
      policy: "replace-on-next-start",
      "source-id": "queued-v2.hal",
      revision: "sha256:second",
      source: "(+ 40 2)",
    },
  });
  assert.equal(queued.state.generation, 0);
  assert.equal(queued.state.revision, "sha256:first");
  assert.deepEqual(queued.payload, {
    accepted: true,
    activation: "next-start",
    revision: "sha256:second",
  });
  assert.equal(original.disposed, false);

  const activated = send(runtime, {
    op: "reset",
    "session-id": "lesson/queued",
    generation: 0,
    revision: "sha256:first",
  });
  assert.equal(activated.state.generation, 1);
  assert.equal(activated.state.revision, "sha256:second");
  assert.equal(activated.state["source-id"], "queued-v2.hal");
  assert.equal(original.disposed, true);
});

test("capabilities reject unsupported operations and replacement policies", () => {
  const { runtime } = runtimeFixture();
  send(runtime, startRequest({
    backend: "interpreter",
    sessionId: "lesson/capabilities",
    revision: "sha256:first",
  }));

  assert.throws(
    () => send(runtime, {
      op: "pause",
      "session-id": "lesson/capabilities",
      generation: 0,
      revision: "sha256:first",
    }),
    (error) => error instanceof LiveSessionError &&
      error.code === "live-session/unsupported-operation",
  );
  assert.throws(
    () => send(runtime, {
      op: "update",
      "session-id": "lesson/capabilities",
      generation: 0,
      revision: "sha256:first",
      payload: {
        policy: "preserve-runtime",
        "source-id": "next.hal",
        revision: "sha256:second",
        source: "(+ 40 2)",
      },
    }),
    (error) => error instanceof LiveSessionError &&
      error.code === "live-session/unsupported-replacement",
  );
});

test("HBC sessions may start from an artifact without leaking mutable bytes", () => {
  const { runtime, hbc } = runtimeFixture();
  const artifact = Uint8Array.from([72, 66, 67, 49]);
  const started = send(runtime, startRequest({
    backend: "hbc",
    sessionId: "lesson/artifact",
    sourceId: "lesson/artifact.hbc",
    revision: "sha256:artifact",
    artifact,
  }));
  artifact[0] = 0;
  assert.deepEqual([...hbc.started[0].input], [72, 66, 67, 49]);
  assert.deepEqual(started.payload.source, { kind: "artifact", bytes: 4 });
  assert.equal(started.state.backend, "hbc");
});

test("cancel is terminal while dispose remains idempotent", () => {
  const { runtime } = runtimeFixture();
  send(runtime, startRequest({
    backend: "interpreter",
    sessionId: "lesson/cancel",
    revision: "sha256:first",
  }));

  const cancelled = send(runtime, {
    op: "cancel",
    "session-id": "lesson/cancel",
    generation: 0,
    revision: "sha256:first",
  });
  assert.equal(cancelled.state.status, "cancelled");
  assert.deepEqual(cancelled.payload, { status: "cancelled" });

  for (const op of ["cancel", "step", "reset", "update"]) {
    assert.throws(
      () => send(runtime, {
        op,
        "session-id": "lesson/cancel",
        generation: 0,
        revision: "sha256:first",
        payload: op === "update" ? {
          policy: "restart",
          "source-id": "next.hal",
          revision: "sha256:second",
          source: "(+ 40 2)",
        } : undefined,
      }),
      (error) => error instanceof LiveSessionError &&
        error.code === "live-session/cancelled",
    );
  }

  const disposed = send(runtime, {
    op: "dispose",
    "session-id": "lesson/cancel",
    generation: 0,
    revision: "sha256:first",
  });
  assert.equal(disposed.state.status, "disposed");
  assert.equal(disposed.payload, true);

  const repeated = send(runtime, {
    op: "dispose",
    "session-id": "lesson/cancel",
    generation: 0,
    revision: "sha256:first",
  });
  assert.equal(repeated.state.status, "disposed");
  assert.equal(repeated.payload, false);
});

test("backend sequences may reset only when the generation advances", () => {
  const runtime = new LiveSessionRuntime({
    backends: [{
      id: "interpreter",
      operations: ["snapshot", "step", "run", "resume", "resolve", "reject", "update", "reset", "cancel", "dispose"],
      replacementPolicies: ["restart", "replace-on-next-start"],
      sourceKinds: ["source"],
      start({ sessionId, source }) {
        const session = new FakeObservationSession({
          backend: "interpreter",
          sessionId,
          sourceId: source.sourceId,
          input: source.value,
          cooperativeCancel: true,
        });
        session.sequence = 3;
        session.step = () => {
          session.sequence = 2;
          return { sequence: 2 };
        };
        return session;
      },
    }],
  });
  send(runtime, startRequest({
    backend: "interpreter",
    sessionId: "lesson/sequence",
    revision: "sha256:first",
  }));
  assert.throws(
    () => send(runtime, {
      op: "step",
      "session-id": "lesson/sequence",
      generation: 0,
      revision: "sha256:first",
    }),
    (error) => error instanceof LiveSessionError &&
      error.code === "live-session/non-monotonic-sequence",
  );
});

test("protocol, session limits, and disposed identities are enforced", () => {
  const { runtime } = runtimeFixture({ maxSessions: 1 });
  assert.throws(
    () => runtime.dispatch({
      protocol: "hara.live-session/old",
      "request-id": "bad-protocol",
      ...startRequest({
        backend: "interpreter",
        sessionId: "lesson/one",
        revision: "sha256:first",
      }),
    }),
    (error) => error instanceof LiveSessionError &&
      error.code === "live-session/protocol",
  );

  send(runtime, startRequest({
    backend: "interpreter",
    sessionId: "lesson/one",
    revision: "sha256:first",
  }));
  assert.throws(
    () => send(runtime, startRequest({
      backend: "hbc",
      sessionId: "lesson/two",
      revision: "sha256:first",
    })),
    (error) => error instanceof LiveSessionError &&
      error.code === "live-session/limit",
  );

  send(runtime, {
    op: "dispose",
    "session-id": "lesson/one",
    generation: 0,
    revision: "sha256:first",
  });
  assert.throws(
    () => send(runtime, startRequest({
      backend: "interpreter",
      sessionId: "lesson/one",
      revision: "sha256:replacement",
    })),
    (error) => error instanceof LiveSessionError &&
      error.code === "live-session/already-exists",
  );
});
