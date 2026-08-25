import { decodeHta, encodeHta, HtaKeyword } from "./index.js";
import {
  createProviderLifecycle,
  HTA_PROVIDER_EVENT,
  providerError,
  providerErrorCode,
  toHta
} from "./provider-common.mjs";

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
 * Creates the provider side of the HTA transport.
 *
 * The provider receives a third argument with a cancellable signal and a
 * manifest-authorized host-call bridge. Ordinary providers that only accept
 * `(operation, args)` remain source compatible.
 */
export function createBrowserProvider(call, options = {}) {
  const scope = options.scope ?? self;
  const lifecycle = createProviderLifecycle({
    origin: options.origin ?? "browser",
    onEvent: options.onEvent
  });
  const cancelled = new Set();
  const calls = new Map();
  const hostCalls = new Map();
  const releases = new Set();
  let nextHostCall = 0;
  let closing = false;
  let closed = false;

  lifecycle.emit(HTA_PROVIDER_EVENT.START);

  function rejectHostCalls(error) {
    for (const pending of hostCalls.values()) pending.reject(error);
    hostCalls.clear();
  }

  function hostCall(service, method, args = [], metadata = {}) {
    if (closed) return Promise.reject(new Error("hta/provider-closed"));
    const id = ++nextHostCall;
    const eventFields = {
      ...(metadata.request === undefined ? {} : { request: Number(metadata.request) }),
      ...(metadata.task === undefined ? {} : { task: Number(metadata.task) }),
      call: id,
      service: String(service),
      method: String(method)
    };
    const signal = Object.hasOwn(metadata, "signal") ? metadata.signal : undefined;
    return new Promise((resolve, reject) => {
      let abort;
      const cleanup = () => signal?.removeEventListener?.("abort", abort);
      abort = () => {
        if (!hostCalls.delete(id)) return;
        cleanup();
        const error = new Error("hta/host-call-cancelled");
        error.code = "hta/host-call-cancelled";
        lifecycle.emit(HTA_PROVIDER_EVENT.HOST_CALL, {
          ...eventFields,
          status: "error",
          code: error.code
        });
        reject(error);
      };
      hostCalls.set(id, {
        eventFields,
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
      lifecycle.emit(HTA_PROVIDER_EVENT.HOST_CALL, { ...eventFields, status: "enter" });
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
      await Promise.all([...releases]);
    } catch (releaseError) {
      failure = releaseError;
    }
    try {
      await options.close?.();
    } catch (closeError) {
      failure ??= closeError;
    } finally {
      rejectHostCalls(error);
      closed = true;
      if (failure) {
        lifecycle.emit(HTA_PROVIDER_EVENT.FAILURE, {
          status: "error",
          code: providerErrorCode(failure, options.errorCode)
        });
      }
      lifecycle.shutdown({
        status: failure === null ? "ok" : "error",
        ...(failure === null
          ? {}
          : { code: providerErrorCode(failure, options.errorCode) })
      });
    }
    if (failure) {
      scope.postMessage({
        type: "fatal",
        error: { message: String(failure?.message ?? failure) }
      });
    }
  }

  async function handle(message) {
    try {
      if (message.type === "delivery") {
        const pending = hostCalls.get(message.call);
        if (!pending) return;
        hostCalls.delete(message.call);
        try {
          const value = decodeHta(message.frame);
          const error = message.ok ? null : errorFrom(value);
          lifecycle.emit(HTA_PROVIDER_EVENT.HOST_CALL, {
            ...pending.eventFields,
            status: message.ok ? "ok" : "error",
            ...(error ? { code: providerErrorCode(error, options.errorCode) } : {})
          });
          message.ok ? pending.resolve(value) : pending.reject(error);
        } catch (error) {
          pending.reject(error);
        }
      } else if (message.type === "release") {
        if (typeof options.release !== "function") {
          throw new Error("hta/handle-release-unsupported");
        }
        const release = Promise.resolve().then(() => options.release(decodeHta(message.frame)));
        releases.add(release);
        try {
          await release;
          lifecycle.emit(HTA_PROVIDER_EVENT.RELEASE, { status: "ok" });
        } catch (error) {
          lifecycle.emit(HTA_PROVIDER_EVENT.RELEASE, {
            status: "error",
            code: providerErrorCode(error, options.errorCode)
          });
          throw error;
        } finally {
          releases.delete(release);
        }
      } else if (message.type === "cancel") {
        const controller = calls.get(message.id);
        if (controller) {
          cancelled.add(message.id);
          controller.abort(new Error("cancelled"));
          lifecycle.emit(HTA_PROVIDER_EVENT.CANCEL, { request: Number(message.id) });
        }
      } else if (message.type === "close") {
        await closeProvider();
      } else if (message.type === "call") {
        if (closing) throw new Error("hta/provider-closed");
        const [operation, args] = decodeHta(message.frame);
        lifecycle.emit(HTA_PROVIDER_EVENT.CALL_ENTER, {
          request: Number(message.id),
          operation: String(operation)
        });
        const controller = new AbortController();
        calls.set(message.id, controller);
        const context = Object.freeze({
          signal: controller.signal,
          hostCall(service, method, values = [], metadata = {}) {
            return hostCall(service, method, values, {
            ...metadata,
            task: metadata.task ?? message.id,
            request: metadata.request ?? message.id,
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
            lifecycle.emit(HTA_PROVIDER_EVENT.CALL_RETURN, {
              request: Number(message.id),
              operation: String(operation),
              status: "ok"
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
            lifecycle.emit(HTA_PROVIDER_EVENT.CALL_ERROR, {
              request: Number(message.id),
              operation: String(operation),
              status: "error",
              code: providerErrorCode(error, options.errorCode)
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
  }

  return Object.freeze({ close: closeProvider, handle });
}
