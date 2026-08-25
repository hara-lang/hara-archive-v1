const ERRORS = new Map([
  [1, "integer overflow"],
  [2, "division by zero"],
  [3, "array index out of bounds"],
  [4, "object key not found"]
]);
const HNW0_OPERATION_REGISTRY_DIGEST =
  "d8b2cd6097d17600d5a534186d27ea2744f4c8057b779b2c6d0b7f9727623e2a";
const HNW0_ABI_VERSION = 0;

function readU32(bytes, offset) {
  return new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength)
    .getUint32(offset, false);
}

function readU16(view, offset) {
  return view.getUint16(offset, false);
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
  if (abiVersion !== HNW0_ABI_VERSION) {
    throw new Error(`unsupported HNW ABI version ${abiVersion}`);
  }
  const functionCount = readU16(view, offset);
  offset += 2;
  const functions = [];
  for (let index = 0; index < functionCount; index += 1) {
    if (offset + 4 > payloadEnd) throw new Error("native artifact is truncated");
    const id = readU16(view, offset);
    const arity = readU16(view, offset + 2);
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
  if (offset + 2 > payloadEnd) throw new Error("native artifact is truncated");
  const targetCount = readU16(view, offset);
  offset += 2;
  const targets = [];
  const decoder = new TextDecoder();
  for (let index = 0; index < targetCount; index += 1) {
    if (offset + 7 > payloadEnd) throw new Error("native artifact target table is truncated");
    const id = readU16(view, offset);
    offset += 2;
    const kind = view.getUint8(offset);
    offset += 1;
    if (kind < 0 || kind > 3) throw new Error("native artifact target kind is invalid");
    const encodedArity = readU16(view, offset);
    offset += 2;
    const symbolLength = readU16(view, offset);
    offset += 2;
    if (offset + symbolLength > payloadEnd) {
      throw new Error("native artifact target symbol is truncated");
    }
    const symbol = decoder.decode(bytes.subarray(offset, offset + symbolLength));
    offset += symbolLength;
    if (id !== index || symbol.length === 0) {
      throw new Error("native artifact target table is not canonical");
    }
    targets.push({
      id,
      kind,
      arity: encodedArity === 0xffff ? null : encodedArity,
      symbol
    });
  }
  if (offset + 32 > payloadEnd) {
    throw new Error("native artifact operation registry digest is truncated");
  }
  const operationRegistryDigest = bytes.slice(offset, offset + 32);
  offset += 32;
  const operationRegistryHex = [...operationRegistryDigest]
    .map((byte) => byte.toString(16).padStart(2, "0"))
    .join("");
  if (operationRegistryHex !== HNW0_OPERATION_REGISTRY_DIGEST) {
    throw new Error("native artifact operation registry digest mismatch");
  }
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
  return {
    abiVersion,
    functionCount,
    functions,
    capabilities,
    targets,
    operationRegistryDigest,
    hbc,
    wasm
  };
}

function hostImports(host, getMemory) {
  const readSlots = (pointer, argc) => {
    const memory = getMemory();
    if (!memory) throw new Error("whole-Wasm bridge memory is unavailable");
    const view = new DataView(memory.buffer);
    const start = Number(pointer);
    const count = Number(argc);
    if (!Number.isSafeInteger(start) || !Number.isSafeInteger(count) || count < 0 || count > 64 ||
        start < 0 || start + count * 16 > view.byteLength) {
      throw new Error("whole-Wasm bridge memory range is invalid");
    }
    const slots = [];
    for (let index = 0; index < count; index += 1) {
      const offset = start + index * 16;
      slots.push([
        view.getUint32(offset, true),
        view.getBigInt64(offset + 8, true),
      ]);
    }
    return slots;
  };
  return {
    constant_handle: (index) => host.constantHandle(index),
    box_i64: (value) => host.boxI64(value),
    unbox_i64: (handle) => host.unboxI64(handle),
    value_construct: (target, pointer, argc) =>
      host.valueConstruct(target, readSlots(pointer, argc)),
    target_call: (target, pointer, argc, resultMode) =>
      host.targetCall(target, readSlots(pointer, argc), resultMode)
  };
}

/** Instantiates and calls a whole-function Hara WebAssembly artifact. */
export async function instantiateWholeWasm(product, Host, fallback) {
  if (typeof Host !== "function") {
    throw new Error("whole-Wasm compilation requires @hara-lang/browser/full");
  }
  const { artifact: inputArtifact, manifest } = wholeWasmProduct(product);
  const artifact = normalizeArtifactBytes(inputArtifact);
  const decoded = decodeHnw0(artifact);
  await validateManifest(manifest, decoded, artifact);
  const host = new Host(artifact);
  const { hbc, wasm, capabilities } = decoded;
  const names = manifestNames(manifest);
  let instance;
  const instantiated = await WebAssembly.instantiate(wasm, {
    [names.importModule]: hostImports(host, () => instance?.exports.hara_memory)
  });
  instance = instantiated.instance;
  const { module } = instantiated;
  const entryFunction = host.entryFunction();
  const nativeEntry = capabilities[entryFunction] === true;
  let heapBase = 0;
  if (nativeEntry) {
    if (typeof instance.exports[names.entrypoint] !== "function") {
      throw new Error(`whole-Wasm module has no ${names.entrypoint} function`);
    }
    if (!instance.exports[names.heapGlobal] ||
        typeof instance.exports[names.heapGlobal].value !== "number") {
      throw new Error(`whole-Wasm module has no ${names.heapGlobal} global`);
    }
    heapBase = instance.exports[names.heapGlobal].value;
  }
  return Object.freeze({
    host,
    module,
    instance,
    manifest,
    entryFunction() {
      return typeof host.entryFunction === "function"
        ? host.entryFunction()
        : 0;
    },
    call(...arguments_) {
      host.beginCall();
      if (!capabilities[this.entryFunction()]) {
        if (typeof fallback !== "function") {
          throw new Error("whole-Wasm entry requires its validated HBC fallback");
        }
        return fallback(hbc);
      }
      instance.exports[names.errorGlobal].value = 0;
      instance.exports[names.heapGlobal].value = heapBase;
      try {
        return instance.exports[names.entrypoint](...arguments_.map(BigInt));
      } catch (error) {
        const message = ERRORS.get(instance.exports[names.errorGlobal].value);
        throw new Error(message ?? `whole-Wasm trap: ${error.message}`);
      }
    },
    callFunction(functionId, ...arguments_) {
      const id = Number(functionId);
      if (!Number.isSafeInteger(id) || id < 0) {
        throw new TypeError("whole-Wasm function id must be a non-negative integer");
      }
      if (id !== this.entryFunction()) {
        throw new Error(`whole-Wasm function ${id} has no prepared export`);
      }
      return this.call(...arguments_);
    }
  });
}

function wholeWasmProduct(value) {
  if (value instanceof Uint8Array || value instanceof ArrayBuffer ||
      ArrayBuffer.isView(value)) {
    return { artifact: value, manifest: null };
  }
  if (!value || typeof value !== "object" || Array.isArray(value) ||
      value.artifact == null) {
    throw new TypeError("whole-Wasm product requires artifact bytes");
  }
  return { artifact: value.artifact, manifest: value.manifest ?? null };
}

async function validateManifest(manifest, decoded, artifact) {
  if (manifest == null) return;
  if (!manifest || typeof manifest !== "object" || Array.isArray(manifest)) {
    throw new TypeError("whole-Wasm product manifest must be an object");
  }
  if (manifest.product !== "whole-wasm" || manifest.format !== "HNW0") {
    throw new Error("whole-Wasm product manifest does not describe HNW0");
  }
  if (manifest["abi-version"] !== `hnw0/${decoded.abiVersion}`) {
    throw new Error(
      `whole-Wasm product manifest ABI does not match HNW0/${decoded.abiVersion}`,
    );
  }
  if (manifest["artifact-bytes"] != null &&
      manifest["artifact-bytes"] !== artifact.byteLength) {
    throw new Error("whole-Wasm product manifest byte length does not match HNW0");
  }
  if (manifest["artifact-digest"] != null) {
    const subtle = globalThis.crypto?.subtle;
    if (!subtle) {
      throw new Error("whole-Wasm product manifest digest cannot be verified");
    }
    const bytes = new Uint8Array(
      await subtle.digest("SHA-256", artifact),
    );
    const digest = [...bytes].map((byte) => byte.toString(16).padStart(2, "0")).join("");
    if (digest !== manifest["artifact-digest"]) {
      throw new Error("whole-Wasm product manifest digest does not match artifact");
    }
  }
}

function normalizeArtifactBytes(value) {
  if (value instanceof Uint8Array) return value;
  if (value instanceof ArrayBuffer) return new Uint8Array(value);
  if (ArrayBuffer.isView(value)) {
    return new Uint8Array(value.buffer, value.byteOffset, value.byteLength);
  }
  throw new TypeError("whole-Wasm artifact must be binary data");
}

function manifestNames(manifest) {
  if (manifest == null) {
    return {
      entrypoint: "hara_entry",
      errorGlobal: "hara_error",
      heapGlobal: "hara_heap",
      importModule: "hara",
    };
  }
  const name = (key) => {
    if (typeof manifest[key] !== "string" || manifest[key].length === 0) {
      throw new Error(`whole-Wasm product manifest is missing ${key}`);
    }
    return manifest[key];
  };
  return {
    entrypoint: name("entrypoint"),
    errorGlobal: name("error-global"),
    heapGlobal: name("heap-global"),
    importModule: name("import-module"),
  };
}
