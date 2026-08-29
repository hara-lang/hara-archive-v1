import { HtaKeyword, HtaMapEntry } from "./index.js";

export const HTA_PROVIDER_EVENT_SCHEMA = "hara.hta.provider.event/0-alpha";

export const HTA_PROVIDER_EVENT = Object.freeze({
  START: "start",
  CALL_ENTER: "call-enter",
  CALL_RETURN: "call-return",
  CALL_ERROR: "call-error",
  HOST_CALL: "host-call",
  CALLBACK: "callback",
  CANCEL: "cancel",
  RELEASE: "release",
  FAILURE: "failure",
  TERMINAL: "terminal",
  SHUTDOWN: "shutdown"
});

/**
 * Creates the host-neutral lifecycle trace used by every HTA provider
 * runner. The trace deliberately contains request/operation identity and
 * terminal status, but never provider values or opaque handle identities.
 */
export function createProviderLifecycle(options = {}) {
  const origin = options.origin === undefined ? "provider" : String(options.origin);
  let sequence = 0;
  let closed = false;

  function emit(event, fields = {}) {
    const record = Object.freeze({
      schema: HTA_PROVIDER_EVENT_SCHEMA,
      sequence: ++sequence,
      origin,
      event,
      ...fields
    });
    try {
      options.onEvent?.(record);
    } catch {
      // Instrumentation must not change provider semantics.
    }
    return record;
  }

  return Object.freeze({
    emit,
    isClosed() {
      return closed;
    },
    shutdown(fields = {}) {
      if (closed) return null;
      closed = true;
      return emit(HTA_PROVIDER_EVENT.SHUTDOWN, fields);
    }
  });
}

export function providerErrorCode(error, fallback = "provider/error") {
  const message = String(error?.message ?? error);
  const separator = message.indexOf(":");
  return separator > 0 ? message.slice(0, separator) : fallback;
}

export function toHta(value) {
  if (value === null || value === undefined || typeof value !== "object") {
    return value ?? null;
  }
  if (value instanceof HtaKeyword || value instanceof HtaMapEntry || value instanceof Uint8Array) return value;
  if (Array.isArray(value)) return value.map(toHta);
  if (value instanceof Map) {
    const result = new Map();
    for (const [key, item] of value) result.set(key, toHta(item));
    return result;
  }
  const result = new Map();
  for (const [key, item] of Object.entries(value)) {
    result.set(new HtaKeyword(key), toHta(item));
  }
  return result;
}

export function providerError(error, origin, fallbackCode = "provider/error") {
  const message = String(error?.message ?? error);
  const code = providerErrorCode(error, fallbackCode);
  return new Map([
    [new HtaKeyword("code"), new HtaKeyword(code)],
    [new HtaKeyword("message"), message],
    [new HtaKeyword("origin"), new HtaKeyword(origin)],
    [new HtaKeyword("retryable"), false]
  ]);
}
