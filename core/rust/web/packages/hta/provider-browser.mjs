import { decodeHta, encodeHta, HtaKeyword } from "./index.js";
import { providerError, toHta } from "./provider-common.mjs";

function errorFrom(value) {
  if (value instanceof Error) return value;
  if (value instanceof Map) {
    let code = "host/error";
    let message = "HTA host call failed";
    let data = value;
    for (const [key, item] of value) {
      const name = key instanceof HtaKeyword ? key.name : String(key);
      if (name === "code") code = item instanceof HtaKeyword ? item.name : String(item);
      if (name === "message") message = String(item);
    }
    const error = new Error(message);
    error.code = code;
    error.data = data;
    return error;
  }
  return new Error(String(value));
}

/**
 * Serves one HTA provider inside a browser worker.
 *
 * The provider receives a third argument with a cancellable signal and a
 * manifest-authorized host-call bridge. Ordinary providers that only accept
 * `(operation, args)` remain source compatible.
 */
export function serveBrowserProvider(call, options = {}) {
  const scope = options.scope ?? self;
  const cancelled = new Set();
  const calls = new Map();
  const hostCalls = new Map();
  let nextHostCall = 0;
  let closing = false;
  let closed = false;

  function rejectHostCalls(error) {
    for (const pending of hostCalls.values()) pending.reject(error);
    hostCalls.clear();
  }

  function hostCall(service, method, args = [], metadata = {}) {
    if (closed) return Promise.reject(new Error("hta/provider-closed"));
    const id = ++nextHostCall;
    const signal = Object.hasOwn(metadata, "signal") ? metadata.signal : undefined;
    return new Promise((resolve, reject) => {
      let abort;
      const cleanup = () => signal?.removeEventListener?.("abort", abort);
      abort = () => {
        if (!hostCalls.delete(id)) return;
        cleanup();
        const error = new Error("hta/host-call-cancelled");
        error.code = "hta/host-call-cancelled";
        reject(error);
      };
      hostCalls.set(id, {
        resolve(value) {
          cleanup();
          resolve(value);
        },
        reject(error) {
          cleanup();
          reject(error);
        }
      });
      if (signal?.aborted) {
        abort();
        return;
      }
      signal?.addEventListener?.("abort", abort, { once: true });
      scope.postMessage({
        type: "host-call",
        call: id,
        service: String(service),
        method: String(method),
        session: metadata.session,
        mount: metadata.mount,
        task: metadata.task,
        frame: encodeHta(toHta(args))
      });
    });
  }

  async function closeProvider() {
    if (closing) return;
    closing = true;
    const error = new Error("hta/provider-closed");
    for (const controller of calls.values()) controller.abort(error);
    calls.clear();
    let failure = null;
    try {
      await options.close?.();
    } catch (closeError) {
      failure = closeError;
    } finally {
      rejectHostCalls(error);
      closed = true;
    }
    if (failure) {
      scope.postMessage({
        type: "fatal",
        error: { message: String(failure?.message ?? failure) }
      });
    }
    scope.close();
  }

  scope.addEventListener("message", async event => {
    const message = event.data;
    try {
      if (message.type === "init") {
        scope.postMessage({ type: "ready" });
      } else if (message.type === "delivery") {
        const pending = hostCalls.get(message.call);
        if (!pending) return;
        hostCalls.delete(message.call);
        try {
          const value = decodeHta(message.frame);
          message.ok ? pending.resolve(value) : pending.reject(errorFrom(value));
        } catch (error) {
          pending.reject(error);
        }
      } else if (message.type === "cancel") {
        const controller = calls.get(message.id);
        if (controller) {
          cancelled.add(message.id);
          controller.abort(new Error("cancelled"));
        }
      } else if (message.type === "close") {
        await closeProvider();
      } else if (message.type === "call") {
        if (closing) throw new Error("hta/provider-closed");
        const [operation, args] = decodeHta(message.frame);
        const controller = new AbortController();
        calls.set(message.id, controller);
        const context = Object.freeze({
          signal: controller.signal,
          hostCall(service, method, values = [], metadata = {}) {
            return hostCall(service, method, values, {
              ...metadata,
              task: metadata.task ?? message.id,
              signal: Object.hasOwn(metadata, "signal") ? metadata.signal : controller.signal
            });
          }
        });
        try {
          const value = await call(operation, args, context);
          if (!cancelled.has(message.id) && !closing) {
            scope.postMessage({
              type: "result",
              id: message.id,
              ok: true,
              frame: encodeHta(toHta(value))
            });
          }
        } catch (error) {
          if (!cancelled.has(message.id) && !closing) {
            scope.postMessage({
              type: "result",
              id: message.id,
              ok: false,
              frame: encodeHta(providerError(error, "browser", options.errorCode))
            });
          }
        } finally {
          calls.delete(message.id);
          cancelled.delete(message.id);
        }
      }
    } catch (error) {
      scope.postMessage({ type: "fatal", error: { message: String(error?.message ?? error) } });
    }
  });

  return Object.freeze({ close: closeProvider });
}
