import { decodeHta, encodeHta } from "./index.js";
import {
  createProviderLifecycle,
  HTA_PROVIDER_EVENT,
  providerError,
  providerErrorCode,
  toHta
} from "./provider-common.mjs";

/**
 * Serves a provider module over the Node process framing used by the Rust
 * loader. The framing is Node-specific; lifecycle, cancellation, release,
 * and provider context semantics are shared with the browser runner.
 */
export function serveNodeProvider(call, options = {}) {
  const input = options.input ?? process.stdin;
  const output = options.output ?? process.stdout;
  const exit = options.exit ?? (code => process.exit(code));
  const maxFrameSize = options.maxFrameSize ?? 64 * 1024 * 1024;
  const lifecycle = createProviderLifecycle({
    origin: options.origin ?? "node",
    onEvent: options.onEvent
  });
  const cancelled = new Set();
  const controllers = new Map();
  const inFlight = new Map();
  let nextHostCall = 0;
  let buffered = new Uint8Array();
  let expected = null;
  let closing = false;
  let closePromise = null;
  let exited = false;

  if (options.redirectConsole !== false) {
    console.log = (...values) => console.error(...values);
    console.info = (...values) => console.error(...values);
  }

  input.on("data", chunk => {
    try {
      const bytes = chunk instanceof Uint8Array ? chunk : new Uint8Array(chunk);
      const next = new Uint8Array(buffered.length + bytes.length);
      next.set(buffered);
      next.set(bytes, buffered.length);
      buffered = next;
      drain();
    } catch (error) {
      reportFatal(error);
    }
  });
  input.on("end", () => {
    void closeProvider().catch(() => {}).finally(finish);
  });

  function drain() {
    while (true) {
      if (expected === null) {
        if (buffered.length < 4) return;
        expected = new DataView(buffered.buffer, buffered.byteOffset, 4).getUint32(0, false);
        buffered = buffered.slice(4);
        if (expected === 0 || expected > maxFrameSize) {
          throw new Error("hta/process-frame-size");
        }
      }
      if (buffered.length < expected) return;
      const frame = buffered.slice(0, expected);
      buffered = buffered.slice(expected);
      expected = null;
      void dispatch(decodeHta(frame)).catch(reportFatal);
    }
  }

  async function dispatch(frame) {
    const [kind, id, operation, args] = frame;
    if (kind === "handshake") {
      lifecycle.emit(HTA_PROVIDER_EVENT.START, {
        namespace: typeof operation === "string" ? operation : undefined
      });
      write(["ready", 1]);
      return;
    }
    if (kind === "shutdown") {
      await closeProvider();
      finish();
      return;
    }
    if (kind === "cancel") {
      const requestId = Number(id);
      const controller = controllers.get(requestId);
      if (controller) {
        cancelled.add(requestId);
        controller.abort(new Error("cancelled"));
        lifecycle.emit(HTA_PROVIDER_EVENT.CANCEL, { request: requestId });
      }
      return;
    }
    if (kind === "release") {
      if (typeof options.release !== "function") {
        throw new Error("hta/handle-release-unsupported");
      }
      try {
        await options.release(id);
        lifecycle.emit(HTA_PROVIDER_EVENT.RELEASE, { status: "ok" });
      } catch (error) {
        lifecycle.emit(HTA_PROVIDER_EVENT.RELEASE, {
          status: "error",
          code: providerErrorCode(error, options.errorCode)
        });
        throw error;
      }
      return;
    }
    if (kind !== "call") throw new Error(`hta/process-event-unknown: ${kind}`);
    if (closing) throw new Error("hta/provider-closed");

    const requestId = Number(id);
    const controller = new AbortController();
    controllers.set(requestId, controller);
    lifecycle.emit(HTA_PROVIDER_EVENT.CALL_ENTER, {
      request: requestId,
      operation: String(operation)
    });
    const context = Object.freeze({
      signal: controller.signal,
      hostCall(service, method, values = [], metadata = {}) {
        if (typeof options.hostCall !== "function") {
          return Promise.reject(new Error("hta/host-call-unsupported: node provider has no host bridge"));
        }
        const call = ++nextHostCall;
        const fields = {
          request: requestId,
          call,
          service: String(service),
          method: String(method)
        };
        lifecycle.emit(HTA_PROVIDER_EVENT.HOST_CALL, { ...fields, status: "enter" });
        return Promise.resolve()
          .then(() => options.hostCall(service, method, values, {
            ...metadata,
            request: requestId,
            call,
            signal: controller.signal
          }))
          .then(value => {
            lifecycle.emit(HTA_PROVIDER_EVENT.HOST_CALL, { ...fields, status: "ok" });
            return value;
          }, error => {
            lifecycle.emit(HTA_PROVIDER_EVENT.HOST_CALL, {
              ...fields,
              status: "error",
              code: providerErrorCode(error, options.errorCode)
            });
            throw error;
          });
      }
    });
    const pending = invoke(requestId, operation, args, context, controller);
    inFlight.set(requestId, pending);
    try {
      await pending;
    } finally {
      inFlight.delete(requestId);
    }
  }

  async function invoke(requestId, operation, args, context, controller) {
    try {
      const value = await call(operation, args, context);
      if (!controller.signal.aborted && !cancelled.has(requestId) && !closing) {
        write(["result", requestId, toHta(value)]);
        lifecycle.emit(HTA_PROVIDER_EVENT.CALL_RETURN, {
          request: requestId,
          operation: String(operation),
          status: "ok"
        });
      }
    } catch (error) {
      if (!controller.signal.aborted && !cancelled.has(requestId) && !closing) {
        write(["error", requestId, providerError(error, "node", options.errorCode)]);
        lifecycle.emit(HTA_PROVIDER_EVENT.CALL_ERROR, {
          request: requestId,
          operation: String(operation),
          status: "error",
          code: providerErrorCode(error, options.errorCode)
        });
      }
    } finally {
      controllers.delete(requestId);
      cancelled.delete(requestId);
    }
  }

  async function closeProvider() {
    if (closePromise) return closePromise;
    closing = true;
    closePromise = (async () => {
      const error = new Error("hta/provider-closed");
      for (const controller of controllers.values()) controller.abort(error);
      await Promise.allSettled([...inFlight.values()]);
      let failure = null;
      try {
        await options.close?.();
      } catch (closeError) {
        failure = closeError;
      }
      lifecycle.shutdown({
        status: failure === null ? "ok" : "error",
        ...(failure === null
          ? {}
          : { code: providerErrorCode(failure, options.errorCode) })
      });
      if (failure) throw failure;
    })();
    return closePromise;
  }

  function finish() {
    if (exited) return;
    exited = true;
    exit(0);
  }

  function reportFatal(error) {
    console.error(String(error?.message ?? error));
    void closeProvider().catch(() => {}).finally(finish);
  }

  function write(value) {
    if (closing) return;
    const frame = encodeHta(value);
    const header = new Uint8Array(4);
    new DataView(header.buffer).setUint32(0, frame.length, false);
    output.write(header);
    output.write(frame);
  }
}
