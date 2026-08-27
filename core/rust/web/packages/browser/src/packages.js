import { parseEDNString } from "edn-data";
import { unzipSync } from "fflate";
import {
  HtaKeyword,
  HtaSymbol,
  HTA_BROWSER_WORKER_URL,
  loadHtaExtension,
  parseEdnData,
  parseHtaManifest
} from "@hara-lang/hta";

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

function extensionDescriptor(namespace, declaration, version) {
  const fields = [...declaration];
  if (!manifestField(declaration, "namespace")) {
    fields.push([new HtaKeyword("namespace"), namespace]);
  }
  if (!manifestField(declaration, "version")) {
    fields.push([new HtaKeyword("version"), version]);
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

function lockedPackages(lock) {
  if (!lock || lock.packages === null || typeof lock.packages !== "object" || Array.isArray(lock.packages)) {
    throw new Error("project.lock.edn requires :packages to be a map");
  }
  return lock.packages ?? {};
}

function packageCoordinate(lock, target) {
  const packages = lockedPackages(lock);
  if (typeof target !== "string" || target.length === 0) {
    throw new Error("package/target-invalid");
  }
  if (Object.hasOwn(packages, target)) return target;
  const matches = [];
  for (const [coordinate, entry] of Object.entries(packages)) {
    if (!entry || typeof entry !== "object" || Array.isArray(entry)) continue;
    const name = entry.name ?? entry["package/name"];
    if (name === target || (entry.namespaces ?? []).some((namespace) => ednScalar(namespace) === target)) {
      matches.push(coordinate);
    }
  }
  if (matches.length === 1) return matches[0];
  if (matches.length > 1) {
    throw new Error(`package/ambiguous-target: ${target} (${matches.sort().join(",")})`);
  }
  throw new Error(`package/not-locked: ${target}`);
}

function lockedClosure(lock, targets) {
  const packages = lockedPackages(lock);
  const selected = new Set();
  const visiting = new Set();
  const ordered = [];
  const visit = (target) => {
    const coordinate = packageCoordinate(lock, target);
    if (selected.has(coordinate)) return;
    if (visiting.has(coordinate)) throw new Error(`package/dependency-cycle: ${coordinate}`);
    const entry = packages[coordinate];
    if (!entry || typeof entry !== "object" || Array.isArray(entry)) {
      throw new Error(`package/descriptor-invalid: ${coordinate}`);
    }
    if (entry.dependencies !== undefined
        && (entry.dependencies === null
          || typeof entry.dependencies !== "object"
          || Array.isArray(entry.dependencies))) {
      throw new Error(`package/descriptor-invalid: ${coordinate} dependencies`);
    }
    visiting.add(coordinate);
    for (const dependency of Object.keys(entry.dependencies ?? {}).sort()) visit(dependency);
    visiting.delete(coordinate);
    selected.add(coordinate);
    ordered.push(coordinate);
  };
  for (const target of targets ?? Object.keys(packages).sort()) visit(target);
  return ordered;
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
  lockedPackages(lock);

  const staged = {};
  const extensions = [];
  const packages = [];
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
    if (!manifest || typeof manifest !== "object" || Array.isArray(manifest)) {
      throw new Error(`Locked package ${coordinate} has an invalid manifest`);
    }
    if (manifest.files === null
        || typeof manifest.files !== "object"
        || Array.isArray(manifest.files)) {
      throw new Error(`Locked package ${coordinate} has invalid :files`);
    }
    if (manifest.resources !== undefined
        && (manifest.resources === null
          || typeof manifest.resources !== "object"
          || Array.isArray(manifest.resources))) {
      throw new Error(`Locked package ${coordinate} has invalid :resources`);
    }
    if (manifest.package !== undefined
        && (manifest.package === null
          || typeof manifest.package !== "object"
          || Array.isArray(manifest.package))) {
      throw new Error(`Locked package ${coordinate} has invalid :package`);
    }
    const manifestIdentity = manifest.package?.identity;
    if (manifestIdentity !== undefined && manifestIdentity !== coordinate) {
      throw new Error(`Locked package ${coordinate} package identity mismatch`);
    }
    const manifestVersion = manifest.package?.version;
    if (manifestVersion !== undefined && manifestVersion !== entry.version) {
      throw new Error(`Locked package ${coordinate} package version mismatch`);
    }
    const declaredFiles = new Set(Object.keys(manifest.files));
    if (declaredFiles.has("package.edn")) {
      throw new Error(`Locked package ${coordinate} declares package.edn as an artifact`);
    }
    for (const path of declaredFiles) {
      if (!safeArchivePath(path)) {
        throw new Error(`Locked package ${coordinate} has an unsafe manifest path: ${path}`);
      }
    }
    for (const path of Object.keys(files)) {
      if (path !== "package.edn" && !declaredFiles.has(path)) {
        throw new Error(`Locked package ${coordinate} contains undeclared file: ${path}`);
      }
    }
    const manifestName = manifest.package?.name ?? manifest.package?.["package/name"];
    const lockedName = entry.name ?? entry["package/name"];
    if (lockedName !== undefined && typeof lockedName !== "string") {
      throw new Error(`Locked package ${coordinate} has an invalid semantic name`);
    }
    if (lockedName !== undefined && lockedName !== manifestName) {
      throw new Error(`Locked package ${coordinate} semantic name mismatch`);
    }
    for (const [path, file] of Object.entries(manifest.files ?? {})) {
      const bytes = files[path];
      if (!bytes || typeof file?.size !== "number" || typeof file?.sha256 !== "string") {
        throw new Error(`Locked package ${coordinate} is missing ${path}`);
      }
      if (file.size !== bytes.byteLength || await sha256(bytes) !== file.sha256) {
        throw new Error(`Locked package ${coordinate} failed file verification: ${path}`);
      }
    }
    let bytecode;
    if (manifest.bytecode !== undefined) {
      const descriptor = manifest.bytecode;
      if (!descriptor || typeof descriptor !== "object" || Array.isArray(descriptor)) {
        throw new Error(`Locked package ${coordinate} has invalid bytecode metadata`);
      }
      const path = descriptor.path;
      if (typeof path !== "string" || !safeArchivePath(path)) {
        throw new Error(`Locked package ${coordinate} has an unsafe bytecode path`);
      }
      const bytes = files[path];
      const file = manifest.files?.[path];
      if (!bytes || !file || !/^sha256:[0-9a-f]{64}$/.test(descriptor.sha256 ?? "")
          || descriptor.format !== "0.0.0-alpha"
          || file.size !== bytes.byteLength || file.sha256 !== descriptor.sha256
          || await sha256(bytes) !== descriptor.sha256) {
        throw new Error(`Locked package ${coordinate} failed bytecode verification: ${path}`);
      }
      bytecode = bytes;
    }
    const packageResources = {};
    for (const [namespace, path] of Object.entries(manifest.resources ?? {})) {
      if (Object.hasOwn(staged, namespace)) {
        throw new Error(`Duplicate locked HAL namespace: ${namespace}`);
      }
      if (typeof path !== "string" || !safeArchivePath(path) || !manifest.files?.[path]) {
        throw new Error(`Locked package ${coordinate} has an invalid resource path: ${path}`);
      }
      const bytes = files[path];
      if (!bytes) {
        throw new Error(`Locked package ${coordinate} is missing resource ${path}`);
      }
      const source = textDecoder.decode(bytes);
      staged[namespace] = source;
      packageResources[namespace] = source;
    }
    if (entry.namespaces !== undefined && !Array.isArray(entry.namespaces)) {
      throw new Error(`Locked package ${coordinate} has invalid namespace declarations`);
    }
    const lockedNamespaces = (entry.namespaces ?? [])
      .map(ednScalar)
      .sort();
    const manifestNamespaces = Object.keys(packageResources).sort();
    if (lockedNamespaces.length && JSON.stringify(lockedNamespaces) !== JSON.stringify(manifestNamespaces)) {
      throw new Error(`Locked package ${coordinate} namespace declaration mismatch`);
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
      const descriptor = extensionDescriptor(namespace, declaration, version);
      const parsed = parseHtaManifest(descriptor);
      for (const asset of [
        parsed.provider === "wasm" ? parsed.module : parsed.browserTarget?.provider,
        ...parsed.assets
      ]) {
        const path = archivePath(root, asset);
        if (!files[path]) throw new Error(`Locked package ${coordinate} is missing extension asset: ${path}`);
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
    packages.push(Object.freeze({
      coordinate,
      name: manifestName ?? lockedName,
      namespaces: Object.freeze(Object.keys(packageResources).sort()),
      resources: Object.freeze(packageResources),
      bytecode
    }));
  }
  return Object.freeze({
    resources: staged,
    extensions: Object.freeze(extensions),
    packages: Object.freeze(packages)
  });
}

function evalBytecodeBundle(runtime, bytes) {
  const evaluator = runtime.evalBytecodeBundle
    ?? runtime.raw?.evalBytecodeBundle
    ?? runtime.raw?.eval_bytecode_bundle;
  if (typeof evaluator !== "function") return false;
  const owner = runtime.evalBytecodeBundle ? runtime : runtime.raw;
  return evaluator.call(owner, bytes) !== false;
}

function unregisterResource(runtime, namespace) {
  const unregister = runtime.unregisterResource
    ?? runtime.raw?.unregisterResource
    ?? runtime.raw?.unregister_resource;
  if (typeof unregister === "function") {
    try {
      unregister.call(runtime.unregisterResource ? runtime : runtime.raw, namespace);
    } catch {
      // Rollback is best-effort for adapters that expose no unregister seam.
    }
  }
}

function installBytecodeBundles(runtime, packages) {
  for (const package_ of packages) {
    if (package_.bytecode) evalBytecodeBundle(runtime, package_.bytecode);
  }
}

/** Installs the on-demand Package capability used by std.native.Package. */
export function installPackageProvider(runtime, lockSource, options = {}) {
  const lock = parseEdn(lockSource);
  const active = new Set();
  runtime.raw?.registerPackageLock?.(lockSource);
  const handler = async (service, operation, arguments_) => {
    if (service !== "package") throw new Error(`host/unsupported-service: ${service}`);
    const descriptor = arguments_?.[0] ?? {};
    const requestedCoordinate = descriptor["package/coordinate"];
    if (typeof requestedCoordinate !== "string") throw new Error("package/descriptor-invalid");
    const coordinate = packageCoordinate(lock, requestedCoordinate);
    if (operation === "ensure") {
      const closure = lockedClosure(lock, [coordinate]);
      if (closure.every(item => active.has(item))) return descriptor;
      const loaded = await loadLockedPackageArtifacts(
        lockSource,
        options.fetch,
        options.origin ?? defaultPackagesOrigin,
        closure
      );
      const registered = [];
      const pending = new Set(closure.filter(item => !active.has(item)));
      try {
        for (const package_ of loaded.packages) {
          if (!pending.has(package_.coordinate)) continue;
          for (const [namespace, source] of Object.entries(package_.resources)) {
            runtime.registerResource(namespace, source);
            registered.push(namespace);
          }
        }
        installBytecodeBundles(runtime, loaded.packages.filter(package_ => pending.has(package_.coordinate)));
      } catch (error) {
        for (const namespace of registered.reverse()) unregisterResource(runtime, namespace);
        throw error;
      }
      closure.forEach((item) => active.add(item));
      return descriptor;
    }
    if (operation === "unload") {
      const cascade = arguments_?.[1]?.cascade === true;
      const selected = new Set([coordinate]);
      const packages = lockedPackages(lock);
      if (cascade) {
        let changed = true;
        while (changed) {
          changed = false;
          for (const [candidate, entry] of Object.entries(packages)) {
            if (active.has(candidate)
                && Object.keys(entry.dependencies ?? {}).some((dependency) => {
                  return selected.has(packageCoordinate(lock, dependency));
                })
                && !selected.has(candidate)) {
              selected.add(candidate);
              changed = true;
            }
          }
        }
      } else {
        const blockers = Object.entries(packages)
          .filter(([candidate, entry]) => active.has(candidate)
            && Object.keys(entry.dependencies ?? {}).some((dependency) => {
              return packageCoordinate(lock, dependency) === coordinate;
            }))
          .map(([candidate]) => candidate);
        if (blockers.length) throw new Error(`package/unload-blocked: ${blockers.join(",")}`);
      }
      const order = [...selected].reverse();
      for (const item of order) {
        for (const namespace of lock.packages[item]?.namespaces ?? []) {
          unregisterResource(runtime, ednScalar(namespace));
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
      const providerPath = archivePath(
        root,
        extension.manifest.provider === "wasm"
          ? extension.manifest.module
          : extension.manifest.browserTarget.provider
      );
      const providerBytes = extension.files.get(providerPath);
      if (!providerBytes) throw new Error(`package/extension-asset-missing:${extension.namespace}:${providerPath}`);
      const assetBytes = new Map();
      const assetUrls = new Map();
      assetBytes.set(providerPath, providerBytes);
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
      const providerUrl = extension.manifest.provider === "hta"
        ? assetUrl(providerPath)
        : undefined;
      const libraryPath = extension.manifest.provider === "wasm"
        ? extension.manifest.assets.find(asset => asset.endsWith(".wasm") && asset !== extension.manifest.module)
        : undefined;
      const libraryBytes = libraryPath
        ? extension.files.get(archivePath(root, libraryPath))
        : undefined;
      const worker = workerFactory(options.workerUrl ?? HTA_BROWSER_WORKER_URL, {
        type: "module",
        name: `hara-${extension.namespace}`
      });
      const record = { context: null, worker };
      records.push(record);
      const context = await loadHtaExtension({
        worker,
        providerUrl,
        descriptor: extension.descriptor,
        moduleBytes: extension.manifest.provider === "wasm" ? providerBytes : undefined,
        libraryBytes,
        hostCalls,
        capabilities: [...supportedCapabilities],
        instrumentation: options.instrumentation === true || typeof options.onProviderEvent === "function",
        onProviderEvent: typeof options.onProviderEvent === "function"
          ? event => options.onProviderEvent(extension.namespace, event)
          : undefined
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
    installBytecodeBundles(runtime, loaded.packages);
  } catch (error) {
    for (const [namespace] of registered.reverse()) unregisterResource(runtime, namespace);
    await extensionState.cleanup();
    throw error;
  }
  return [...resources, ...bridges].map(([namespace]) => namespace);
}
