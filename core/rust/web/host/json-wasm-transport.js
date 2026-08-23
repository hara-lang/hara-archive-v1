import { parseJson, stringifyJson } from "./services.js";

const encoder = new TextEncoder();
const decoder = new TextDecoder("utf-8", { fatal: true });

export const objectValue = (value, label) => {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    throw new TypeError(`${label} must be an object`);
  }
  return value;
};

export const byteArray = (value, label = "Wasm module") => {
  if (value instanceof Uint8Array) return value;
  if (value instanceof ArrayBuffer) return new Uint8Array(value);
  if (ArrayBuffer.isView(value)) {
    return new Uint8Array(value.buffer, value.byteOffset, value.byteLength);
  }
  throw new TypeError(`${label} must be bytes`);
};

/**
 * Shared plain-C JSON transport for runtime-host Wasm adapters.
 *
 * The Wasm module owns every runtime value and handle. The host copies UTF-8
 * request/response bytes around one packed-pointer invoke function and releases
 * both allocations before returning a plain JavaScript projection.
 */
export class JsonWasmTransport {
  constructor(exports, {
    label,
    requestLabel = `${label} request`,
    abiVersion = 1,
    errorCode = "runtime-host-wasm/error",
    names,
  }) {
    this.label = nonEmptyString(label, "JSON Wasm transport label");
    this.requestLabel = nonEmptyString(requestLabel, "JSON Wasm request label");
    this.errorCode = nonEmptyString(errorCode, "JSON Wasm default error code");
    this.names = normalizeNames(names);
    if (!exports || typeof exports !== "object") {
      throw new TypeError(`${this.label} exports are required`);
    }
    for (const name of Object.values(this.names)) {
      if (typeof exports[name] !== "function") {
        throw new Error(`${this.label} is missing ${name}`);
      }
    }
    if (!(exports.memory instanceof WebAssembly.Memory)) {
      throw new Error(`${this.label} is missing exported memory`);
    }
    if (exports[this.names.abiVersion]() !== abiVersion) {
      throw new Error(`unsupported ${this.label.replace(/ Wasm$/, "")} ABI`);
    }
    this.exports = exports;
  }

  invoke(request) {
    const source = stringifyJson(objectValue(request, this.requestLabel));
    const input = encoder.encode(source);
    const pointer = this.exports[this.names.alloc](input.byteLength);
    if (!Number.isInteger(pointer) || pointer <= 0) {
      throw new Error(`${this.label} failed to allocate request memory`);
    }

    let packed;
    try {
      new Uint8Array(this.exports.memory.buffer, pointer, input.byteLength).set(input);
      packed = this.exports[this.names.invoke](pointer, input.byteLength);
    } finally {
      this.exports[this.names.dealloc](pointer, input.byteLength);
    }

    if (typeof packed !== "bigint") {
      throw new Error(`${this.label} returned an invalid response pointer`);
    }
    const responsePointer = Number(packed >> 32n);
    const responseLength = Number(packed & 0xffff_ffffn);
    if (!Number.isSafeInteger(responsePointer) || responsePointer <= 0 ||
        !Number.isSafeInteger(responseLength) || responseLength <= 0) {
      throw new Error(`${this.label} returned an empty response`);
    }

    let responseBytes;
    try {
      responseBytes = new Uint8Array(
        new Uint8Array(this.exports.memory.buffer, responsePointer, responseLength),
      );
    } finally {
      this.exports[this.names.dealloc](responsePointer, responseLength);
    }

    const response = parseJson(decoder.decode(responseBytes));
    if (!response || response.ok !== true) {
      const message = response?.error?.message ?? `${this.requestLabel} failed`;
      const error = new Error(String(message));
      error.code = response?.error?.code ?? this.errorCode;
      throw error;
    }
    return response.value;
  }
}

export async function instantiateJsonWasm({
  wasmUrl,
  wasmBytes = null,
  fetchImpl = globalThis.fetch,
  imports = {},
  label,
  fetchDescription = label,
  loadDescription = label,
}) {
  let bytes = wasmBytes;
  if (bytes == null) {
    if (typeof fetchImpl !== "function") {
      throw new Error(`fetch is required to load ${fetchDescription}`);
    }
    const response = await fetchImpl(wasmUrl);
    if (!response?.ok) {
      throw new Error(`unable to load ${loadDescription}: ${response?.status ?? "network"}`);
    }
    bytes = await response.arrayBuffer();
  }
  const result = await WebAssembly.instantiate(byteArray(bytes, label), imports);
  return result instanceof WebAssembly.Instance ? result : result.instance;
}

function normalizeNames(names) {
  const value = objectValue(names, "JSON Wasm export names");
  return Object.freeze({
    abiVersion: nonEmptyString(value.abiVersion, "JSON Wasm ABI export"),
    alloc: nonEmptyString(value.alloc, "JSON Wasm allocation export"),
    dealloc: nonEmptyString(value.dealloc, "JSON Wasm deallocation export"),
    invoke: nonEmptyString(value.invoke, "JSON Wasm invoke export"),
  });
}

function nonEmptyString(value, label) {
  if (typeof value !== "string" || value.trim().length === 0) {
    throw new TypeError(`${label} must be a non-empty string`);
  }
  return value;
}
