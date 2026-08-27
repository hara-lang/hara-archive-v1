import init, * as wasmBindings from "./wasm/hara_wasm.js";
import { instantiateWholeWasm } from "./whole-wasm.js";
import { parseJson } from "./json.js";
import { disposeBrowserPackageProviders, installLockedPackages } from "./packages.js";
export {
  disposeBrowserPackageProviders,
  installLockedPackages,
  installPackageProvider,
  loadLockedPackageResources
} from "./packages.js";

const { Runtime } = wasmBindings;

function asResourceEntries(resources) {
  if (!resources) return [];
  if (resources instanceof Map) return [...resources.entries()];
  return Object.entries(resources);
}

function createApi(runtime) {
  const loadWholeWasm = (artifact) =>
    instantiateWholeWasm(
      artifact,
      wasmBindings.WholeWasmHost,
      (hbc) => runtime.evalBytecodeArtifact(hbc)
    );

  const api = {
    eval(source) {
      return runtime.eval(String(source));
    },
    require(namespace) {
      return runtime.require_resource(String(namespace));
    },
    registerResource(namespace, source) {
      runtime.register_resource(String(namespace), String(source));
    },
    installDirectWasmImport(logical, bytes) {
      runtime.installDirectWasmImport(String(logical), bytes);
    },
    installMemoryWasmBinding(manifest, interfaceSource, bindingsSource, bytes) {
      runtime.installMemoryWasmBinding(
        String(manifest),
        String(interfaceSource),
        String(bindingsSource),
        bytes
      );
    },
    installHostHandler(handler) {
      if (typeof handler !== "function" || typeof runtime.install_host_handler !== "function") {
        throw new Error("host-handler-unavailable");
      }
      runtime.install_host_handler(handler);
    },
    unregisterResource(namespace) {
      runtime.unregister_resource(String(namespace));
    },
    evalInNamespace(namespace, source) {
      return runtime.eval_in_namespace(String(namespace), String(source));
    },
    currentNamespace() {
      return runtime.current_namespace();
    },
    compileBytecode(source) {
      return runtime.compileBytecodeArtifact(String(source));
    },
    compileBytecodeProduct(source) {
      const value = String(source);
      return Object.freeze({
        artifact: runtime.compileBytecodeArtifact(value),
        manifest: parseJson(runtime.compileBytecodeManifest(value)),
      });
    },
    evalBytecode(artifact) {
      return runtime.evalBytecodeArtifact(artifact);
    },
    evalBytecodeBundle(artifact) {
      if (typeof runtime.evalBytecodeBundle !== "function") return false;
      return runtime.evalBytecodeBundle(artifact);
    },
    installPackages(lockSource, options = {}) {
      return installLockedPackages(api, lockSource, options);
    },
    instrumentationConformance(corpus) {
      if (typeof wasmBindings.instrumentation_conformance !== "function") {
        throw new Error("instrumentation conformance requires the full Wasm runtime");
      }
      return parseJson(wasmBindings.instrumentation_conformance(JSON.stringify(corpus)));
    },
    loadWholeWasm(artifact) {
      return loadWholeWasm(artifact);
    },
    async compileWholeWasm(source) {
      if (typeof runtime.compileWholeWasmArtifact !== "function") {
        throw new Error("whole-Wasm compilation requires @hara-lang/browser/full");
      }
      const artifact = runtime.compileWholeWasmArtifact(String(source));
      return loadWholeWasm(artifact);
    },
    compileWholeWasmProduct(source) {
      if (typeof runtime.compileWholeWasmManifest !== "function") {
        throw new Error("whole-Wasm compilation requires @hara-lang/browser/full");
      }
      const value = String(source);
      const artifact = runtime.compileWholeWasmArtifact(value);
      return Object.freeze({
        artifact,
        manifest: parseJson(runtime.compileWholeWasmManifest(value)),
      });
    },
    raw: runtime,
    dispose() {
      const cleanup = disposeBrowserPackageProviders(runtime);
      runtime.free();
      return cleanup;
    }
  };
  return Object.freeze(api);
}

function defaultWasmUrl() {
  // The release build inlines the Wasm payload into the generated
  // wasm-bindgen module, so the default path is self-contained for both the
  // ESM and IIFE/CDN builds. A caller can still provide wasmUrl explicitly.
  return undefined;
}

/**
 * Starts an isolated core runtime. Foundation and other semantic packages are
 * installed only when the caller supplies a lock/profile explicitly.
 */
export async function start({ wasmUrl, resources, lock, targets, packageOptions = {} } = {}) {
  await init(wasmUrl ?? defaultWasmUrl());
  const runtime = typeof Runtime.core === "function" ? Runtime.core() : new Runtime();
  const api = createApi(runtime);
  for (const [namespace, source] of asResourceEntries(resources)) {
    api.registerResource(namespace, source);
  }
  if (lock !== undefined && lock !== null) {
    const installOptions = { ...packageOptions };
    if (targets !== undefined) installOptions.targets = targets;
    await api.installPackages(lock, installOptions);
  }
  return api;
}

export const ready = start();
