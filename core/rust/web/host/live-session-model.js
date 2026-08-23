export const LIVE_SESSION_PROTOCOL = "hara.live-session/0-alpha";
export const LIVE_SESSION_STATE_SCHEMA = "hara.live-session.state/0-alpha";
export const LIVE_SESSION_REPLY_SCHEMA = "hara.live-session.reply/0-alpha";
export const LIVE_SESSION_CAPABILITIES_SCHEMA =
  "hara.live-session.capabilities/0-alpha";

export const MAX_SAFE_INTEGER = 9_007_199_254_740_991;
export const DEFAULT_RUN_LIMIT = 100_000;

const LIVE_OPERATIONS = new Set([
  "snapshot",
  "step",
  "run",
  "call",
  "pause",
  "resume",
  "resolve",
  "reject",
  "update",
  "reset",
  "cancel",
  "dispose",
]);

const REPLACEMENT_POLICIES = new Set([
  "restart",
  "replace-on-next-start",
  "preserve-runtime",
]);

const SOURCE_KINDS = new Set(["source", "artifact"]);
const LIVE_STATUSES = new Set([
  "ready",
  "running",
  "paused",
  "suspended",
  "returned",
  "failed",
  "cancelled",
  "disposed",
]);

export class LiveSessionError extends Error {
  constructor(code, message, detail = null) {
    super(message);
    this.name = "LiveSessionError";
    this.code = code;
    this.detail = detail;
  }
}

export function normalizeBackend(backend) {
  if (!backend || typeof backend !== "object" || Array.isArray(backend)) {
    throw new TypeError("live session backend must be an object");
  }
  const id = nonEmptyString(backend.id, "live session backend id");
  if (typeof backend.start !== "function") {
    throw new TypeError(`live session backend ${id} requires start()`);
  }
  const operations = normalizeEnumValues(
    backend.operations,
    LIVE_OPERATIONS,
    `live session backend ${id} operation`,
  );
  const replacementPolicies = normalizeEnumValues(
    backend.replacementPolicies ?? backend["replacement-policies"] ?? [],
    REPLACEMENT_POLICIES,
    `live session backend ${id} replacement policy`,
  );
  const sourceKinds = normalizeEnumValues(
    backend.sourceKinds ?? backend["source-kinds"] ?? ["source"],
    SOURCE_KINDS,
    `live session backend ${id} source kind`,
  );
  return Object.freeze({
    id,
    start: backend.start.bind(backend),
    operations,
    replacementPolicies,
    sourceKinds,
  });
}

export function normalizeRequest(value) {
  const request = objectValue(value, "live session request");
  const protocol = field(request, "protocol");
  if (protocol !== LIVE_SESSION_PROTOCOL) {
    throw new LiveSessionError(
      "live-session/protocol",
      `unsupported live-session protocol: ${protocol ?? "missing"}`,
    );
  }
  const requestId = nonEmptyString(
    field(request, "request-id", "requestId"),
    "live session request id",
  );
  const sessionId = nonEmptyString(
    field(request, "session-id", "sessionId"),
    "live session id",
  );
  const op = nonEmptyString(field(request, "op"), "live session operation");
  if (op !== "start" && !LIVE_OPERATIONS.has(op)) {
    throw new LiveSessionError(
      "live-session/unsupported-operation",
      `unsupported live session operation: ${op}`,
      { operation: op },
    );
  }
  const generationValue = field(request, "generation");
  const generation = generationValue == null
    ? null
    : boundedInteger(generationValue, "live session generation", MAX_SAFE_INTEGER);
  if (op === "start" && generation != null && generation !== 0) {
    throw new LiveSessionError(
      "live-session/start-generation",
      "new live sessions must start at generation 0",
      { generation },
    );
  }
  const revisionValue = field(request, "revision");
  const revision = revisionValue == null
    ? null
    : nonEmptyString(revisionValue, "live session revision fence");
  const payloadValue = field(request, "payload");
  const payload = payloadValue == null
    ? Object.freeze({})
    : objectValue(payloadValue, "live session payload");
  return Object.freeze({
    raw: request,
    protocol,
    requestId,
    sessionId,
    op,
    generation,
    revision,
    payload,
  });
}

export function normalizeSource(value) {
  const input = objectValue(value, "live session source payload");
  const sourceId = nonEmptyString(
    field(input, "source-id", "sourceId"),
    "live session source id",
  );
  const revision = nonEmptyString(
    field(input, "revision"),
    "live session source revision",
  );
  const source = field(input, "source");
  const artifact = field(input, "artifact");
  if (source != null && artifact != null) {
    throw new LiveSessionError(
      "live-session/source",
      "live session source payload must provide source or artifact, not both",
    );
  }
  if (source != null) {
    return Object.freeze({
      kind: "source",
      sourceId,
      revision,
      value: nonEmptyString(source, "live session source"),
    });
  }
  if (artifact != null) {
    return Object.freeze({
      kind: "artifact",
      sourceId,
      revision,
      value: copyBytes(artifact, "live session artifact"),
    });
  }
  throw new LiveSessionError(
    "live-session/source",
    "live session source payload requires source or artifact",
  );
}

export function normalizeSettlement(value) {
  if (value == null) return null;
  const settlement = objectValue(value, "live session settlement");
  const status = nonEmptyString(
    field(settlement, "status"),
    "live session settlement status",
  );
  if (status === "fulfilled") {
    return Object.freeze({ status, value: field(settlement, "value") ?? null });
  }
  if (status === "rejected") {
    return Object.freeze({ status, error: field(settlement, "error") ?? null });
  }
  throw new TypeError(`unsupported live session settlement status: ${status}`);
}

export function replacementPolicy(value) {
  const policy = nonEmptyString(value, "live session replacement policy");
  if (!REPLACEMENT_POLICIES.has(policy)) {
    throw new LiveSessionError(
      "live-session/unsupported-replacement",
      `unsupported live session replacement policy: ${policy}`,
      { policy },
    );
  }
  return policy;
}

export function capabilitiesDocument(backend) {
  return Object.freeze({
    schema: LIVE_SESSION_CAPABILITIES_SCHEMA,
    protocol: LIVE_SESSION_PROTOCOL,
    backend: backend.id,
    operations: Object.freeze([...backend.operations]),
    "replacement-policies": Object.freeze([...backend.replacementPolicies]),
  });
}

export function sourceSummary(source) {
  return Object.freeze({
    kind: source.kind,
    bytes: source.kind === "artifact"
      ? source.value.byteLength
      : new TextEncoder().encode(source.value).byteLength,
  });
}

export function sourcesEqual(left, right) {
  if (left.kind !== right.kind ||
      left.sourceId !== right.sourceId ||
      left.revision !== right.revision) {
    return false;
  }
  if (left.kind === "source") return left.value === right.value;
  if (left.value.byteLength !== right.value.byteLength) return false;
  return left.value.every((entry, index) => entry === right.value[index]);
}

export function requireBackendSession(session, backendId) {
  if (!session || typeof session !== "object" || Array.isArray(session)) {
    throw new LiveSessionError(
      "live-session/backend-session",
      `backend ${backendId} returned an invalid session`,
    );
  }
  if (typeof session.dispose !== "function") {
    throw new LiveSessionError(
      "live-session/backend-session",
      `backend ${backendId} session must implement dispose()`,
    );
  }
  return session;
}

export function disposeBackendSession(session) {
  try {
    return session?.dispose?.() ?? false;
  } catch (error) {
    if (/disposed/i.test(String(error?.message ?? error))) return false;
    throw error;
  }
}

export function readBackendSequence(session, backendId) {
  const sequence = Number(session?.sequence ?? 0);
  if (!Number.isSafeInteger(sequence) || sequence < 0) {
    throw new LiveSessionError(
      "live-session/backend-sequence",
      `backend ${backendId} exposed an invalid sequence`,
      { backend: backendId, sequence: session?.sequence ?? null },
    );
  }
  return sequence;
}

export function canonicalStatus(session, backendId, terminalStatus = null) {
  const status = terminalStatus ?? String(session?.status ?? "ready");
  if (!LIVE_STATUSES.has(status)) {
    throw new LiveSessionError(
      "live-session/backend-status",
      `backend ${backendId} exposed unsupported status ${status}`,
      { backend: backendId, status },
    );
  }
  return status;
}

export function supportsOperation(backend, operation) {
  return backend.operations.includes(operation);
}

export function supportsReplacement(backend, policy) {
  return backend.replacementPolicies.includes(policy);
}

export function supportsSourceKind(backend, kind) {
  return backend.sourceKinds.includes(kind);
}

export function nonEmptyString(value, label) {
  if (typeof value !== "string" || value.trim().length === 0) {
    throw new TypeError(`${label} must be a non-empty string`);
  }
  return value;
}

export function boundedInteger(value, label, maximum = MAX_SAFE_INTEGER, minimum = 0) {
  if (!Number.isSafeInteger(value) || value < minimum || value > maximum) {
    throw new RangeError(`${label} must be an integer between ${minimum} and ${maximum}`);
  }
  return value;
}

export function increment(value, label) {
  if (value >= MAX_SAFE_INTEGER) {
    throw new LiveSessionError("live-session/generation-exhausted", `${label} exhausted`);
  }
  return value + 1;
}

export function immutableProjection(value) {
  if (value == null || typeof value !== "object") return value ?? null;
  if (value instanceof Uint8Array) return Object.freeze([...value]);
  if (value instanceof ArrayBuffer) return Object.freeze([...new Uint8Array(value)]);
  if (ArrayBuffer.isView(value)) {
    return Object.freeze([
      ...new Uint8Array(value.buffer, value.byteOffset, value.byteLength),
    ]);
  }
  if (Array.isArray(value)) return Object.freeze(value.map(immutableProjection));
  const projection = {};
  for (const [key, entry] of Object.entries(value)) {
    projection[key] = immutableProjection(entry);
  }
  return Object.freeze(projection);
}

export function objectValue(value, label) {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    throw new TypeError(`${label} must be an object`);
  }
  return value;
}

export function field(value, canonical, camel = null) {
  if (Object.prototype.hasOwnProperty.call(value, canonical)) return value[canonical];
  if (camel && Object.prototype.hasOwnProperty.call(value, camel)) return value[camel];
  return undefined;
}

function normalizeEnumValues(values, allowed, label) {
  if (!Array.isArray(values)) throw new TypeError(`${label}s must be an array`);
  const normalized = [...new Set(values.map((value) => nonEmptyString(value, label)))];
  for (const value of normalized) {
    if (!allowed.has(value)) throw new TypeError(`unsupported ${label}: ${value}`);
  }
  return Object.freeze(normalized);
}

function copyBytes(value, label) {
  if (value instanceof Uint8Array) return new Uint8Array(value);
  if (value instanceof ArrayBuffer) return new Uint8Array(value.slice(0));
  if (ArrayBuffer.isView(value)) {
    return new Uint8Array(
      value.buffer.slice(value.byteOffset, value.byteOffset + value.byteLength),
    );
  }
  if (Array.isArray(value) && value.every((entry) =>
    Number.isInteger(entry) && entry >= 0 && entry <= 255)) {
    return Uint8Array.from(value);
  }
  throw new TypeError(`${label} must be bytes`);
}
