import { parseEDNString } from "edn-data";
import { unzipSync } from "fflate";
import {
  HtaKeyword,
  HtaSymbol,
  loadHtaExtension,
  parseEdnData,
  parseHtaManifest
} from "../../hta/index.js";

const ednOptions = {
  mapAs: "object",
  setAs: "array",
  listAs: "array",
  keywordAs: "string",
  charAs: "string",
  objectKeysAs: "string"
};

const textDecoder = new TextDecoder();
const hostDispatchers = new WeakMap();
const packageCleanups = new WeakMap();

function parseEdn(source) {
  return parseEDNString(String(source), ednOptions);
}

function manifestField(map, name) {
  if (!(map instanceof Map)) return undefined;
  for (const [key, value] of map) {
    if (key instanceof HtaKeyword && key.name === name) return value;
  }
  return undefined;
}

function extensionName(value) {
  if (typeof value === "string") return value;
  if (value instanceof HtaKeyword || value instanceof HtaSymbol) return value.name;
  return undefined;
}

function archivePath(root, path) {
  const prefix = root ? `${root.replace(/\/$/, "")}/` : "";
  const result = `${prefix}${path}`;
  if (!safeArchivePath(result)) throw new Error(`package/extension-path-unsafe: ${result}`);
  return result;
}

function extensionDescriptor(namespace, declaration) {
  const fields = [...declaration];
  if (!manifestField(declaration, "namespace")) {
    fields.push([new HtaKeyword("namespace"), namespace]);
  }
  return `{${fields.map(([key, value]) => `${ednValue(key)} ${ednValue(value)}`).join(" ")}}`;
}

function ednValue(value) {
  if (value instanceof HtaKeyword) return `:${value.name}`;
  if (value instanceof HtaSymbol) return value.name;
  if (typeof value === "string") return JSON.stringify(value);
  if (value === null) return "nil";
  if (value === true) return "true";
  if (value === false) return "false";
  if (Array.isArray(value)) return `[${value.map(ednValue).join(" ")}]`;
  if (value instanceof Map) {
    return `{${[...value].map(([key, item]) => `${ednValue(key)} ${ednValue(item)}`).join(" ")}}`;
  }
  if (typeof value === "number" && Number.isFinite(value)) return String(value);
  throw new Error("package/extension-manifest-unsupported");
}

function sourceBridge(namespace, manifest) {
  const bindings = manifest.exports.map((name) => {
    if (!/^[A-Za-z][A-Za-z0-9_?!*+-]*$/.test(name)) {
      throw new Error(`package/extension-export-invalid: ${name}`);
    }
    const arity = manifest.exportArity[name] ?? 0;
    const arguments_ = Array.from({ length: arity }, (_, index) => `arg${index}`);
    const values = arguments_.length ? `[${arguments_.join(" ")}]` : "[]";
    return `(defn ${name} [${arguments_.join(" ")}] (Host/call ${JSON.stringify(namespace)} ${JSON.stringify(name)} ${values}))`;
  });
  return `(ns ${namespace}) ${bindings.join(" ")}`;
}

function toPlainHta(value) {
  if (Array.isArray(value)) return value.map(toPlainHta);
  if (value instanceof Uint8Array) return value;
  if (value instanceof HtaKeyword || value instanceof HtaSymbol) return value.name;
  if (value instanceof Map) {
    const result = Object.create(null);
    for (const [key, item] of value) result[String(key instanceof HtaKeyword || key instanceof HtaSymbol ? key.name : key)] = toPlainHta(item);
    return result;
  }
  return value;
}

function toHtaValue(value) {
  if (Array.isArray(value)) return value.map(toHtaValue);
  if (value instanceof Uint8Array) return value;
  if (value && typeof value === "object" && !(value instanceof Map)) {
    return new Map(Object.entries(value).map(([key, item]) => [new HtaKeyword(key), toHtaValue(item)]));
  }
  if (value instanceof Map) return new Map([...value].map(([key, item]) => [toHtaValue(key), toHtaValue(item)]));
  return value;
}

function registerHostService(runtime, service, handler) {
  const key = runtimeKey(runtime);
  let state = hostDispatchers.get(key);
  if (!state) {
    const routes = new Map();
    const dispatcher = (requestedService, operation, arguments_) => {
      const route = routes.get(requestedService);
      if (!route) throw new Error(`host/unsupported-service: ${requestedService}`);
      return route(operation, arguments_);
    };
    installHostHandler(runtime, dispatcher);
    state = { routes, dispatcher };
    hostDispatchers.set(key, state);
  }
  if (state.routes.has(service)) throw new Error(`host/service-already-installed: ${service}`);
  state.routes.set(service, handler);
  return () => {
    if (state.routes.get(service) === handler) state.routes.delete(service);
  };
}

function installHostHandler(runtime, handler) {
  const install = runtime.installHostHandler
    ?? runtime.raw?.install_host_handler
    ?? runtime.raw?.installHostHandler;
  if (typeof install !== "function") throw new Error("package/host-handler-unavailable");
  install.call(runtime.installHostHandler ? runtime : runtime.raw, handler);
}

function runtimeKey(runtime) {
  return runtime?.raw ?? runtime;
}

function hex(bytes) {
  return [...bytes]
    .map((byte) => byte.toString(16).padStart(2, "0"))
    .join("");
}

async function sha256(bytes) {
  const digest = await globalThis.crypto.subtle.digest("SHA-256", bytes);
  return `sha256:${hex(new Uint8Array(digest))}`;
}

const defaultPackagesOrigin = "https://packages.hara-lang.org";

function ednScalar(value) {
  if (typeof value === "string") return value;
  if (value && typeof value.sym === "string") return value.sym;
  if (value && typeof value.key === "string") return value.key;
  return String(value);
}

function packageCoordinate(lock, target) {
  if (Object.hasOwn(lock.packages ?? {}, target)) return target;
  for (const [coordinate, entry] of Object.entries(lock.packages ?? {})) {
    if ((entry.namespaces ?? []).some((namespace) => ednScalar(namespace) === target)) return coordinate;
  }
  throw new Error(`package/not-locked: ${target}`);
}

function lockedClosure(lock, targets) {
  const selected = new Set();
  const visit = (target) => {
    const coordinate = packageCoordinate(lock, target);
    if (selected.has(coordinate)) return;
    const entry = lock.packages[coordinate];
    selected.add(coordinate);
    for (const dependency of Object.keys(entry.dependencies ?? {})) visit(dependency);
  };
  for (const target of targets ?? Object.keys(lock.packages ?? {})) visit(target);
  return [...selected].sort();
}

function safeArchivePath(path) {
  return path
    && !path.startsWith("/")
    && !path.includes("\\")
    && path.split("/").every((part) => part && part !== "." && part !== "..");
}

/**
 * Downloads and verifies every HARP archive through the commit-pinned Packages
 * registry. Nothing is registered until all packages verify.
 */
export async function loadLockedPackageResources(
  lockSource,
  request = (...args) => globalThis.fetch(...args),
  origin = defaultPackagesOrigin,
  targets
) {
  const loaded = await loadLockedPackageArtifacts(lockSource, request, origin, targets);
  return loaded.resources;
}

async function loadLockedPackageArtifacts(
  lockSource,
  request = (...args) => globalThis.fetch(...args),
  origin = defaultPackagesOrigin,
  targets
) {
  const lock = parseEdn(lockSource);
  if (lock["lock/format"] !== "0.0.0-alpha") {
    throw new Error("project.lock.edn requires :lock/format \"0.0.0-alpha\"");
  }

  const staged = {};
  const extensions = [];
  for (const coordinate of lockedClosure(lock, targets)) {
    const entry = lock.packages[coordinate];
    const registryCommit = entry["registry-commit"];
    const identityRevision = entry["identity-revision"];
    const digest = entry["archive-sha256"];
    const version = entry.version;
    if (!/^[0-9a-f]{40}$/.test(registryCommit ?? "")
        || !/^[0-9a-f]{40}$/.test(identityRevision ?? "")
        || !/^sha256:[0-9a-f]{64}$/.test(digest ?? "")
        || typeof version !== "string") {
      throw new Error(`Locked package ${coordinate} has an incomplete exact descriptor`);
    }
    const base = String(origin).replace(/\/$/, "");
    const registryResponse = await request(`${base}/v1/registry?ref=${registryCommit}`);
    if (!registryResponse.ok) {
      throw new Error(`Locked package ${coordinate} registry failed: ${registryResponse.status}`);
    }
    const registry = parseEdn(await registryResponse.text());
    const release = registry["registry/packages"]?.[coordinate]?.[version];
    if (release?.["archive-sha256"] !== digest
        || release?.["identity-revision"] !== identityRevision) {
      throw new Error(`Locked package ${coordinate} registry mismatch`);
    }
    const response = await request(`${base}/objects/sha256/${digest.slice(7)}`);
    if (!response.ok) {
      throw new Error(`Locked package ${coordinate} failed: ${response.status}`);
    }
    const archive = new Uint8Array(await response.arrayBuffer());
    if (entry.size !== undefined && archive.byteLength !== entry.size) {
      throw new Error(`Locked package ${coordinate} size mismatch`);
    }
    if (await sha256(archive) !== digest) {
      throw new Error(`Locked package ${coordinate} digest mismatch`);
    }

    const files = unzipSync(archive);
    if (!files["package.edn"]) {
      throw new Error(`Locked package ${coordinate} has no package.edn`);
    }
    for (const path of Object.keys(files)) {
      if (!safeArchivePath(path)) {
        throw new Error(`Locked package ${coordinate} contains an unsafe path`);
      }
    }

    const manifestSource = textDecoder.decode(files["package.edn"]);
    const manifest = parseEdn(manifestSource);
    for (const [path, file] of Object.entries(manifest.files ?? {})) {
      const bytes = files[path];
      if (!bytes) {
        throw new Error(`Locked package ${coordinate} is missing ${path}`);
      }
      if (file.size !== bytes.byteLength || await sha256(bytes) !== file.sha256) {
        throw new Error(`Locked package ${coordinate} failed file verification: ${path}`);
      }
    }
    for (const [namespace, path] of Object.entries(manifest.resources ?? {})) {
      if (Object.hasOwn(staged, namespace)) {
        throw new Error(`Duplicate locked HAL namespace: ${namespace}`);
      }
      const bytes = files[path];
      if (!bytes) {
        throw new Error(`Locked package ${coordinate} is missing resource ${path}`);
      }
      staged[namespace] = textDecoder.decode(bytes);
    }
    const manifestData = parseEdnData(manifestSource, "package/manifest-malformed");
    const declaredExtensions = manifestField(manifestData, "extensions");
    const declarations = Array.isArray(declaredExtensions) && declaredExtensions.length === 0
      ? undefined
      : declaredExtensions;
    if (declarations !== undefined && !(declarations instanceof Map)) {
      throw new Error(`Locked package ${coordinate} has invalid extensions`);
    }
    for (const [key, declaration] of declarations ?? []) {
      const namespace = extensionName(key);
      if (!namespace || !(declaration instanceof Map)) {
        throw new Error(`Locked package ${coordinate} has an invalid extension`);
      }
      const root = manifestField(declaration, "root") ?? "";
      if (typeof root !== "string") throw new Error(`Locked package ${coordinate} has an invalid extension root`);
      const descriptor = extensionDescriptor(namespace, declaration);
      const parsed = parseHtaManifest(descriptor);
      for (const asset of [
        parsed.provider === "wasm" ? parsed.module : parsed.browserTarget?.module,
        ...parsed.assets
      ]) {
        const path = archivePath(root, asset);
        if (!files[path]) throw new Error(`Locked package ${coordinate} is missing extension asset: ${path}`);
      }
      if (parsed.provider !== "hta") {
        throw new Error(`Locked package ${coordinate} has an unsupported extension provider: ${parsed.provider}`);
      }
      extensions.push(Object.freeze({
        coordinate,
        namespace,
        declaration,
        descriptor,
        manifest: parsed,
        files: new Map(Object.entries(files))
      }));
    }
  }
  return Object.freeze({ resources: staged, extensions: Object.freeze(extensions) });
}

/** Installs the on-demand Package capability used by std.native.Package. */
export function installPackageProvider(runtime, lockSource, options = {}) {
  const lock = parseEdn(lockSource);
  const active = new Set();
  runtime.raw?.registerPackageLock?.(lockSource);
  const handler = async (service, operation, arguments_) => {
    if (service !== "package") throw new Error(`host/unsupported-service: ${service}`);
    const descriptor = arguments_?.[0] ?? {};
    const coordinate = descriptor["package/coordinate"];
    if (typeof coordinate !== "string") throw new Error("package/descriptor-invalid");
    if (operation === "ensure") {
      const closure = lockedClosure(lock, [coordinate]);
      const resources = await loadLockedPackageResources(
        lockSource,
        options.fetch,
        options.origin ?? defaultPackagesOrigin,
        closure
      );
      for (const [namespace, source] of Object.entries(resources)) {
        runtime.registerResource(namespace, source);
      }
      closure.forEach((item) => active.add(item));
      return descriptor;
    }
    if (operation === "unload") {
      const cascade = arguments_?.[1]?.cascade === true;
      const selected = new Set([coordinate]);
      if (cascade) {
        let changed = true;
        while (changed) {
          changed = false;
          for (const [candidate, entry] of Object.entries(lock.packages ?? {})) {
            if (active.has(candidate)
                && Object.keys(entry.dependencies ?? {}).some((dependency) => selected.has(dependency))
                && !selected.has(candidate)) {
              selected.add(candidate);
              changed = true;
            }
          }
        }
      } else {
        const blockers = Object.entries(lock.packages ?? {})
          .filter(([candidate, entry]) => active.has(candidate)
            && Object.keys(entry.dependencies ?? {}).includes(coordinate))
          .map(([candidate]) => candidate);
        if (blockers.length) throw new Error(`package/unload-blocked: ${blockers.join(",")}`);
      }
      const order = [...selected].reverse();
      for (const item of order) {
        for (const namespace of lock.packages[item]?.namespaces ?? []) {
          runtime.raw?.unregister_resource?.(ednScalar(namespace));
        }
        active.delete(item);
      }
      return order;
    }
    throw new Error(`package/unsupported-operation: ${operation}`);
  };
  registerHostService(runtime, "package", (operation, arguments_) => handler("package", operation, arguments_));
  return Object.freeze({ active, handler });
}

async function activateBrowserHtaExtensions(runtime, extensions, options = {}) {
  const supportedCapabilities = new Set(options.capabilities ?? []);
  const records = [];
  const removeRoutes = [];
  const objectUrls = [];
  const workerFactory = options.workerFactory
    ?? ((url, workerOptions) => {
      if (typeof Worker !== "function") throw new Error("package/hta-worker-unavailable");
      return new Worker(url, workerOptions);
    });
  const createObjectURL = options.createObjectURL
    ?? globalThis.URL?.createObjectURL?.bind(globalThis.URL);
  const revokeObjectURL = options.revokeObjectURL
    ?? globalThis.URL?.revokeObjectURL?.bind(globalThis.URL);
  const BlobConstructor = options.Blob ?? globalThis.Blob;

  const createUrl = (bytes, path) => {
    if (typeof createObjectURL !== "function" || typeof BlobConstructor !== "function") {
      throw new Error("package/hta-object-url-unavailable");
    }
    const url = createObjectURL(new BlobConstructor([bytes], { type: mimeType(path) }));
    objectUrls.push(url);
    return url;
  };

  try {
    for (const extension of extensions) {
      const missing = extension.manifest.capabilities.filter(capability => !supportedCapabilities.has(capability));
      if (missing.length) {
        throw new Error(`package/extension-capability-unsupported: ${extension.namespace}:${missing.join(",")}`);
      }
      const hostCalls = extensionHostCalls(extension.manifest, options.hostCalls);
      const root = manifestField(extension.declaration, "root") ?? "";
      const modulePath = archivePath(root, extension.manifest.browserTarget.module);
      const moduleBytes = extension.files.get(modulePath);
      if (!moduleBytes) throw new Error(`package/extension-asset-missing:${extension.namespace}:${modulePath}`);
      const assetBytes = new Map();
      const assetUrls = new Map();
      for (const asset of extension.manifest.assets) {
        const path = archivePath(root, asset);
        const bytes = extension.files.get(path);
        if (!bytes) throw new Error(`package/extension-asset-missing:${extension.namespace}:${path}`);
        assetBytes.set(path, bytes);
      }
      const building = new Set();
      const assetUrl = (path) => {
        if (assetUrls.has(path)) return assetUrls.get(path);
        if (building.has(path)) throw new Error(`package/extension-asset-cycle: ${path}`);
        building.add(path);
        const bytes = assetBytes.get(path);
        if (!isJavaScript(path)) {
          const url = createUrl(bytes, path);
          assetUrls.set(path, url);
          building.delete(path);
          return url;
        }
        let source = textDecoder.decode(bytes);
        for (const dependency of assetBytes.keys()) {
          if (dependency !== path && sourceReferences(source, dependency, path)) assetUrl(dependency);
        }
        source = rewriteAssetReferences(source, path, assetUrls);
        const url = createUrl(new TextEncoder().encode(source), path);
        assetUrls.set(path, url);
        building.delete(path);
        return url;
      };
      for (const path of assetBytes.keys()) assetUrl(path);
      const moduleSource = rewriteAssetReferences(
        textDecoder.decode(moduleBytes),
        modulePath,
        assetUrls
      );
      const workerUrl = createUrl(new TextEncoder().encode(moduleSource), modulePath);
      const worker = workerFactory(workerUrl, {
        type: "module",
        name: `hara-${extension.namespace}`
      });
      const record = { context: null, worker };
      records.push(record);
      const context = await loadHtaExtension({
        worker,
        descriptor: extension.descriptor,
        hostCalls
      });
      record.context = context;
      const route = (operation, arguments_) => {
        const request = context.call(operation, toHtaValue(arguments_ ?? []));
        return context.promiseProvider.create((resolve, reject, onCancel) => {
          onCancel(() => request.cancel?.());
          request.then(
            value => {
              try {
                resolve(toPlainHta(value));
              } catch (error) {
                reject(error);
              }
            },
            error => reject(stableExtensionError(error))
          );
        });
      };
      removeRoutes.push(registerHostService(runtime, extension.namespace, route));
    }
  } catch (error) {
    for (const remove of removeRoutes.reverse()) remove();
    for (const record of records.reverse()) {
      if (record.context) await record.context.close().catch(() => {});
      else record.worker.terminate?.();
    }
    for (const url of objectUrls.reverse()) revokeObjectURL?.(url);
    throw error;
  }

  let cleaned = false;
  const cleanup = async () => {
    if (cleaned) return;
    cleaned = true;
    for (const remove of removeRoutes.slice().reverse()) remove();
    for (const record of records.slice().reverse()) {
      if (record.context) await record.context.close().catch(() => {});
      else record.worker.terminate?.();
    }
    for (const url of objectUrls.slice().reverse()) revokeObjectURL?.(url);
  };
  const key = runtimeKey(runtime);
  const previous = packageCleanups.get(key);
  packageCleanups.set(key, async () => {
    await cleanup();
    if (previous) await previous();
  });
  return Object.freeze({
    namespaces: Object.freeze(extensions.map(extension => extension.namespace)),
    cleanup
  });
}

function extensionHostCalls(manifest, configured = {}) {
  const hostCalls = {};
  for (const [service, methods] of Object.entries(manifest.hostCalls)) {
    for (const method of methods) {
      const key = `${service}/${method}`;
      const handler = configured?.[key] ?? configured?.[service]?.[method];
      if (typeof handler !== "function") {
        throw new Error(`package/extension-host-call-unsupported: ${key}`);
      }
      hostCalls[key] = handler;
    }
  }
  return hostCalls;
}

function stableExtensionError(error) {
  if (!error?.code || String(error.message).startsWith(`${error.code}:`)) return error;
  const wrapped = new Error(`${error.code}: ${error.message}`);
  wrapped.code = error.code;
  wrapped.data = error.data;
  return wrapped;
}

function mimeType(path) {
  if (path.endsWith(".mjs") || path.endsWith(".js")) return "text/javascript";
  if (path.endsWith(".wasm")) return "application/wasm";
  return "application/octet-stream";
}

function isJavaScript(path) {
  return path.endsWith(".mjs") || path.endsWith(".js");
}

function sourceReferences(source, assetPath, fromPath) {
  return assetReferences(assetPath, fromPath).some(reference =>
    ["\"", "'", "`"].some(quote => source.includes(`${quote}${reference}${quote}`)));
}

function rewriteAssetReferences(source, fromPath, urls) {
  let rewritten = source;
  for (const [assetPath, url] of urls) {
    for (const reference of assetReferences(assetPath, fromPath)) {
      for (const quote of ["\"", "'", "`"]) {
        rewritten = rewritten.replaceAll(`${quote}${reference}${quote}`, `${quote}${url}${quote}`);
      }
    }
  }
  return rewritten;
}

function assetReferences(assetPath, fromPath) {
  const directory = fromPath.includes("/")
    ? fromPath.slice(0, fromPath.lastIndexOf("/") + 1)
    : "";
  const relative = assetPath.startsWith(directory)
    ? assetPath.slice(directory.length)
    : assetPath;
  const suffix = assetPath.match(/(?:^|\/)assets\/(.+)$/)?.[1];
  return [...new Set([
    relative,
    `./${relative}`,
    suffix && `/assets/${suffix}`
  ].filter(Boolean))];
}

export async function disposeBrowserPackageProviders(runtime) {
  const key = runtimeKey(runtime);
  const cleanup = packageCleanups.get(key);
  packageCleanups.delete(key);
  await cleanup?.();
}

/** Verifies a lock completely, then atomically exposes its HAL resources. */
export async function installLockedPackages(runtime, lockSource, options = {}) {
  runtime.raw?.registerPackageLock?.(lockSource);
  const loaded = await loadLockedPackageArtifacts(
    lockSource,
    options.fetch,
    options.origin ?? defaultPackagesOrigin,
    options.targets
  );
  const resources = Object.entries(loaded.resources);
  const bridges = loaded.extensions.map(extension => [
    extension.namespace,
    sourceBridge(extension.namespace, extension.manifest)
  ]);
  const names = new Set();
  for (const [namespace] of [...resources, ...bridges]) {
    if (names.has(namespace)) throw new Error(`package/namespace-collision: ${namespace}`);
    names.add(namespace);
  }
  const extensionState = await activateBrowserHtaExtensions(runtime, loaded.extensions, options);
  const registered = [];
  try {
    for (const [namespace, source] of [...resources, ...bridges]) {
      runtime.registerResource(namespace, source);
      registered.push([namespace, source]);
    }
  } catch (error) {
    for (const [namespace] of registered.reverse()) runtime.unregisterResource?.(namespace);
    await extensionState.cleanup();
    throw error;
  }
  return [...resources, ...bridges].map(([namespace]) => namespace);
}
