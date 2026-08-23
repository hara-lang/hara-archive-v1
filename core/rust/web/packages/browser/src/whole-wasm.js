const ERRORS = new Map([
  [1, "integer overflow"],
  [2, "division by zero"],
  [3, "array index out of bounds"],
  [4, "object key not found"]
]);

function readU32(bytes, offset) {
  return new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength)
    .getUint32(offset, false);
}

/** Extracts the WebAssembly payload from an HNW0 artifact produced by Rust. */
export function decodeHnw0(input) {
  const bytes = input instanceof Uint8Array ? input : new Uint8Array(input);
  if (bytes.length < 40 || String.fromCharCode(...bytes.subarray(0, 4)) !== "HNW0") {
    throw new Error("native artifact has invalid magic");
  }
  const payloadLength = readU32(bytes, 4);
  const payloadEnd = 8 + payloadLength;
  if (payloadEnd + 32 !== bytes.length) {
    throw new Error("native artifact length mismatch");
  }
  let offset = 8;
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  const abiVersion = view.getUint16(offset, false);
  offset += 2;
  if (abiVersion !== 2) throw new Error(`unsupported HNW ABI version ${abiVersion}`);
  const functionCount = view.getUint16(offset, false);
  offset += 2;
  const functions = [];
  for (let index = 0; index < functionCount; index += 1) {
    if (offset + 4 > payloadEnd) throw new Error("native artifact is truncated");
    const id = view.getUint16(offset, false);
    const arity = view.getUint16(offset + 2, false);
    offset += 4;
    if (id !== index) throw new Error("native artifact function table is not canonical");
    functions.push({ id, arity });
  }
  if (offset + functionCount > payloadEnd) {
    throw new Error("native artifact is truncated");
  }
  const capabilities = Array.from(bytes.subarray(offset, offset + functionCount), (native) => {
    if (native !== 0 && native !== 1) {
      throw new Error("native artifact capability table is not canonical");
    }
    return native === 1;
  });
  offset += functionCount;
  if (offset + 4 > payloadEnd) {
    throw new Error("native artifact contains malformed sections");
  }
  const hbcLength = readU32(bytes, offset);
  offset += 4;
  if (hbcLength > payloadEnd - offset) {
    throw new Error("native artifact contains malformed sections");
  }
  const hbc = bytes.slice(offset, offset + hbcLength);
  offset += hbcLength;
  if (offset + 4 > payloadEnd) {
    throw new Error("native artifact contains malformed sections");
  }
  const wasmLength = readU32(bytes, offset);
  offset += 4;
  if (offset + wasmLength !== payloadEnd) {
    throw new Error("native artifact contains malformed sections");
  }
  const wasm = bytes.slice(offset, offset + wasmLength);
  if (String.fromCharCode(...wasm.subarray(0, 4)) !== "\0asm") {
    throw new Error("native artifact contains invalid Wasm");
  }
  return { abiVersion, functionCount, functions, capabilities, hbc, wasm };
}

function hostImports(host) {
  return {
    constant_handle: (index) => host.constantHandle(index),
    box_i64: (value) => host.boxI64(value),
    unbox_i64: (handle) => host.unboxI64(handle),
    vector_empty: () => host.vectorEmpty(),
    vector_push: (vector, item) => host.vectorPush(vector, item),
    map_empty: () => host.mapEmpty(),
    map_assoc: (map, key, value) => host.mapAssoc(map, key, value),
    get: (collection, key) => host.getValue(collection, key),
    is_number: (value) => host.isNumber(value),
    count: (collection) => host.count(collection),
    nth: (collection, index) => host.nth(collection, index),
    map_i64_pair: (key, value) => host.mapI64Pair(key, value),
    get_i64: (collection, key) => host.getI64(collection, key),
    get_path_i64_constants: (collection, first, second) =>
      host.getPathI64Constants(collection, first, second),
    assoc_map_i64_pair: (collection, outerKey, innerKey, value) =>
      host.assocMapI64Pair(collection, outerKey, innerKey, value)
  };
}

/** Instantiates and calls a whole-function Hara WebAssembly artifact. */
export async function instantiateWholeWasm(artifact, Host, fallback) {
  if (typeof Host !== "function") {
    throw new Error("whole-Wasm compilation requires @hara-lang/browser/full");
  }
  const decoded = decodeHnw0(artifact);
  const host = new Host(artifact);
  const { hbc, wasm, capabilities } = decoded;
  const { instance, module } = await WebAssembly.instantiate(wasm, {
    hara: hostImports(host)
  });
  if (typeof instance.exports.hara_entry !== "function") {
    throw new Error("whole-Wasm module has no hara_entry function");
  }
  return Object.freeze({
    host,
    module,
    instance,
    call(...arguments_) {
      host.beginCall();
      if (!capabilities[0]) {
        if (typeof fallback !== "function") {
          throw new Error("whole-Wasm entry requires its validated HBC fallback");
        }
        return fallback(hbc);
      }
      instance.exports.hara_error.value = 0;
      instance.exports.hara_heap.value = 0;
      try {
        return instance.exports.hara_entry(...arguments_.map(BigInt));
      } catch (error) {
        const message = ERRORS.get(instance.exports.hara_error.value);
        throw new Error(message ?? `whole-Wasm trap: ${error.message}`);
      }
    }
  });
}
