/**
 * Host-side verified package cache. The WASM evaluator is intentionally never
 * given fetch authority; callers register returned bytes before evaluation.
 */
import { HtaKeyword, HtaSymbol, parseEdnData } from "./packages/hta/index.js";

const encoder = new TextEncoder();
const decoder = new TextDecoder("utf-8", { fatal: true });

export async function sha256(bytes) {
  const digest = await crypto.subtle.digest("SHA-256", bytes);
  return `sha256:${[...new Uint8Array(digest)].map((byte) => byte.toString(16).padStart(2, "0")).join("")}`;
}

export async function verifyEd25519({ publicKey, statement, signature }) {
  const key = await crypto.subtle.importKey("raw", publicKey, { name: "Ed25519" }, false, ["verify"]);
  return crypto.subtle.verify({ name: "Ed25519" }, key, signature, encoder.encode(statement));
}

export async function fetchVerifiedPackage({ url, digest, publisher, registry, fetchImpl = fetch, cache }) {
  const cacheKey = `hara-package/${digest}`;
  const cached = cache && await cache.match(cacheKey);
  const response = cached || await fetchImpl(url);
  if (!response?.ok) throw new Error(`package/fetch-failed: ${response?.status ?? "network"}`);
  const bytes = await response.arrayBuffer();
  if (await sha256(bytes) !== digest) throw new Error("package/digest-mismatch");
  for (const signature of [publisher, registry]) {
    if (!signature || !await verifyEd25519(signature)) throw new Error("package/signature-invalid");
  }
  if (!cached && cache) await cache.put(cacheKey, new Response(bytes, { headers: { "content-type": "application/vnd.hara.harp" } }));
  return bytes;
}

/**
 * Validates a HARP container and returns its immutable resource index.
 * Verification is deliberately complete before any caller can register a
 * namespace with a runtime.
 */
export async function inspectHarp(input) {
  const entries = await readZip(input);
  const manifestBytes = entries.get("package.edn");
  if (!manifestBytes) throw new Error("package/manifest-missing");
  const manifest = parseEdnData(decoder.decode(manifestBytes), "package/manifest-malformed");
  if (!(manifest instanceof Map) || field(manifest, "harp/format") !== "0.0.0-alpha") {
    throw new Error("package/manifest-malformed: unsupported :harp/format");
  }
  const declaredFiles = field(manifest, "files");
  const integrity = field(manifest, "integrity");
  const resourceIndex = field(manifest, "resources") ?? new Map();
  const declaredExtensions = field(manifest, "extensions");
  const extensionDeclarations = declaredExtensions === undefined
    || (Array.isArray(declaredExtensions) && declaredExtensions.length === 0)
    ? new Map()
    : declaredExtensions;
  if (!(declaredFiles instanceof Map) || !(integrity instanceof Map)
      || !(resourceIndex instanceof Map) || !(extensionDeclarations instanceof Map)) {
    throw new Error("package/manifest-malformed: invalid files, resources, or extensions");
  }
  const actualPaths = [...entries.keys()].filter((path) => path !== "package.edn").sort();
  const declaredPaths = [...declaredFiles.keys()];
  if (declaredPaths.some((path) => typeof path !== "string")
      || declaredPaths.slice().sort().join("\0") !== actualPaths.join("\0")) {
    throw new Error("package/file-set-mismatch");
  }
  const treeParts = [];
  for (const path of declaredPaths) {
    const bytes = entries.get(path);
    const spec = declaredFiles.get(path);
    if (!(spec instanceof Map) || !bytes) throw new Error(`package/file-invalid:${path}`);
    if (field(spec, "size") !== bytes.byteLength
        || field(spec, "sha256") !== await sha256(bytes)) {
      throw new Error(`package/file-integrity:${path}`);
    }
    treeParts.push(encoder.encode(path), Uint8Array.of(0), bytes);
  }
  if (field(integrity, "tree-sha256") !== await sha256(joinBytes(treeParts))) {
    throw new Error("package/tree-integrity");
  }
  const resources = new Map();
  for (const [namespace, path] of resourceIndex) {
    if (typeof namespace !== "string" || typeof path !== "string"
        || !entries.has(path)
        || (!path.endsWith(".hal") && !path.endsWith(".halc") && !path.endsWith(".hir"))) {
      throw new Error("package/resource-invalid");
    }
    if (resources.has(namespace)) throw new Error(`package/resource-duplicate:${namespace}`);
    resources.set(namespace, Object.freeze(path.endsWith(".halc") || path.endsWith(".hir")
      ? { format: "halc", bytes: entries.get(path).slice() }
      : { format: "hal", source: decoder.decode(entries.get(path)) }));
  }
  const extensions = [];
  for (const [namespace, declaration] of extensionDeclarations) {
    if (!(namespace instanceof HtaKeyword) && !(namespace instanceof HtaSymbol) && typeof namespace !== "string") {
      throw new Error("package/extension-invalid");
    }
    if (!(declaration instanceof Map)) throw new Error("package/extension-invalid");
    const root = field(declaration, "root") ?? "";
    if (typeof root !== "string") throw new Error("package/extension-invalid");
    const base = root === "" ? "" : `${root.replace(/\/$/, "")}/`;
    const targets = field(declaration, "targets");
    const targetProviders = targets instanceof Map
      ? [...targets.values()].map(target => target instanceof Map ? field(target, "provider") : undefined)
      : [];
    for (const asset of [
      ...([field(declaration, "module")].filter(Boolean)),
      ...targetProviders.filter(Boolean),
      ...(field(declaration, "assets") ?? [])
    ]) {
      if (typeof asset !== "string" || !entries.has(`${base}${asset}`)) {
        throw new Error(`package/extension-asset-missing:${namespaceName(namespace)}:${asset}`);
      }
    }
    extensions.push(Object.freeze({ namespace: namespaceName(namespace), declaration }));
  }
  return Object.freeze({
    manifest,
    files: entries,
    resources,
    extensions: Object.freeze(extensions)
  });
}

/**
 * Stages an exact package set and makes its namespaces visible with one raw
 * runtime operation. Any verification or namespace collision leaves the
 * runtime unchanged.
 */
export async function activateLockedPackages({ packages, context }) {
  if (!Array.isArray(packages) || !context?.call) throw new Error("package/activation-invalid");
  const staged = new Map();
  const inspected = [];
  for (const record of packages) {
    const bytes = record?.bytes;
    if (!bytes) throw new Error("package/bytes-missing");
    if (record.digest && await sha256(bytes) !== record.digest) {
      throw new Error("package/digest-mismatch");
    }
    const archive = await inspectHarp(bytes);
    for (const [namespace, resource] of archive.resources) {
      if (staged.has(namespace)) throw new Error(`package/namespace-collision:${namespace}`);
      staged.set(namespace, resource);
    }
    inspected.push(archive);
  }
  const sourceResources = [...staged]
    .filter(([, resource]) => resource.format === "hal")
    .map(([namespace, resource]) => [namespace, resource.source]);
  if (sourceResources.length) await context.call("register-resources", [sourceResources]);
  for (const resource of staged.values()) {
    if (resource.format === "halc") await context.call("eval-halc", [resource.bytes]);
  }
  return Object.freeze({ packages: Object.freeze(inspected), resources: staged });
}

function field(map, name) {
  for (const [key, value] of map) {
    if (key instanceof HtaKeyword && key.name === name) return value;
  }
  return undefined;
}

function namespaceName(value) {
  return value instanceof HtaKeyword || value instanceof HtaSymbol ? value.name : value;
}

function joinBytes(parts) {
  const output = new Uint8Array(parts.reduce((size, part) => size + part.byteLength, 0));
  let cursor = 0;
  for (const part of parts) {
    output.set(part, cursor);
    cursor += part.byteLength;
  }
  return output;
}

async function readZip(input) {
  const bytes = input instanceof Uint8Array ? input : new Uint8Array(input);
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  let eocd = -1;
  for (let cursor = bytes.length - 22; cursor >= Math.max(0, bytes.length - 65557); cursor--) {
    if (u32(view, cursor) === 0x06054b50) { eocd = cursor; break; }
  }
  if (eocd < 0) throw new Error("package/zip-malformed");
  const count = view.getUint16(eocd + 10, true);
  let cursor = view.getUint32(eocd + 16, true);
  const entries = new Map();
  for (let index = 0; index < count; index++) {
    if (u32(view, cursor) !== 0x02014b50) throw new Error("package/zip-malformed");
    const flags = view.getUint16(cursor + 8, true);
    const method = view.getUint16(cursor + 10, true);
    const compressedSize = view.getUint32(cursor + 20, true);
    const size = view.getUint32(cursor + 24, true);
    const nameLength = view.getUint16(cursor + 28, true);
    const extraLength = view.getUint16(cursor + 30, true);
    const commentLength = view.getUint16(cursor + 32, true);
    const localOffset = view.getUint32(cursor + 42, true);
    if ((flags & 1) !== 0 || ![0, 8].includes(method)) throw new Error("package/zip-unsupported");
    const path = decoder.decode(bytes.subarray(cursor + 46, cursor + 46 + nameLength));
    validatePackagePath(path);
    if (entries.has(path)) throw new Error(`package/zip-duplicate:${path}`);
    if (u32(view, localOffset) !== 0x04034b50) throw new Error("package/zip-malformed");
    const localNameLength = view.getUint16(localOffset + 26, true);
    const localExtraLength = view.getUint16(localOffset + 28, true);
    const dataOffset = localOffset + 30 + localNameLength + localExtraLength;
    const compressed = bytes.subarray(dataOffset, dataOffset + compressedSize);
    let data;
    if (method === 0) {
      data = compressed.slice();
    } else {
      if (typeof DecompressionStream !== "function") throw new Error("package/deflate-unavailable");
      const stream = new Blob([compressed]).stream().pipeThrough(new DecompressionStream("deflate-raw"));
      data = new Uint8Array(await new Response(stream).arrayBuffer());
    }
    if (data.byteLength !== size) throw new Error(`package/zip-size:${path}`);
    entries.set(path, data);
    cursor += 46 + nameLength + extraLength + commentLength;
  }
  return entries;
}

function u32(view, offset) {
  if (offset < 0 || offset + 4 > view.byteLength) return -1;
  return view.getUint32(offset, true);
}

function validatePackagePath(path) {
  if (!path || path.startsWith("/") || path.includes("\\") || path.includes("\0")
      || path.split("/").some((part) => !part || part === "." || part === "..")
      || /^[A-Za-z]:/.test(path)) {
    throw new Error(`package/path-unsafe:${path}`);
  }
}
