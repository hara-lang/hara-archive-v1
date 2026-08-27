import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";
import { zipSync } from "fflate";
import {
  disposeBrowserPackageProviders,
  installLockedPackages,
  installPackageProvider,
  loadLockedPackageResources
} from "./src/packages.js";
import { decodeHta, encodeHta, HtaKeyword } from "@hara-lang/hta";

const encoder = new TextEncoder();

function hex(bytes) {
  return [...bytes].map((byte) => byte.toString(16).padStart(2, "0")).join("");
}

async function digest(bytes) {
  return `sha256:${hex(new Uint8Array(await crypto.subtle.digest("SHA-256", bytes)))}`;
}

async function fixture() {
  const source = encoder.encode("(ns demo.world) (def world {:title \"Demo\"})");
  const sourceDigest = await digest(source);
  const manifest = encoder.encode(
    `{:files {"src/demo/world.hal" {:size ${source.byteLength} :sha256 "${sourceDigest}"}} `
      + `:resources {"demo.world" "src/demo/world.hal"}}`
  );
  const archive = zipSync({
    "package.edn": manifest,
    "src/demo/world.hal": source
  });
  const archiveDigest = await digest(archive);
  const registryCommit = "a".repeat(40);
  const identityRevision = "b".repeat(40);
  const lock = `{:lock/format \"0.0.0-alpha\" :packages {"demo:world" `
    + `{:version "1.0.0" :tap "hara" :registry-commit "${registryCommit}" `
    + `:identity-revision "${identityRevision}" :archive-sha256 "${archiveDigest}" `
    + `:namespaces [demo.world]}}}`;
  const registry = `{:registry/packages {"demo:world" {"1.0.0" `
    + `{:archive-sha256 "${archiveDigest}" :identity-revision "${identityRevision}"}}}}`;
  return { archive, lock, registry, registryCommit, archiveDigest };
}

async function bytecodeFixture() {
  const source = encoder.encode("(ns postgres.core) (def answer 42)");
  const bundle = new Uint8Array([0x48, 0x42, 0x58, 0x30, 0x01]);
  const sourceDigest = await digest(source);
  const bundleDigest = await digest(bundle);
  const manifest = encoder.encode(
    `{:harp/format "0.0.0-alpha" `
      + `:package {:identity "hara:lang/model.v1.postgres" :name "lang.model.v1.postgres" :version "1.0.0"} `
      + `:files {"src/postgres/core.hal" {:size ${source.byteLength} :sha256 "${sourceDigest}"} `
      + `"bytecode/package.hbx" {:size ${bundle.byteLength} :sha256 "${bundleDigest}"}} `
      + `:resources {"postgres.core" "src/postgres/core.hal"} `
      + `:bytecode {:format "0.0.0-alpha" :path "bytecode/package.hbx" :sha256 "${bundleDigest}"}}`
  );
  const archive = zipSync({
    "package.edn": manifest,
    "src/postgres/core.hal": source,
    "bytecode/package.hbx": bundle
  });
  const archiveDigest = await digest(archive);
  const registryCommit = "1".repeat(40);
  const identityRevision = "2".repeat(40);
  const lock = `{:lock/format "0.0.0-alpha" :packages {"hara:lang/model.v1.postgres" `
    + `{:name "lang.model.v1.postgres" :version "1.0.0" :tap "hara" `
    + `:registry-commit "${registryCommit}" :identity-revision "${identityRevision}" `
    + `:archive-sha256 "${archiveDigest}" :namespaces [postgres.core]}}}`;
  const registry = `{:registry/packages {"hara:lang/model.v1.postgres" {"1.0.0" `
    + `{:archive-sha256 "${archiveDigest}" :identity-revision "${identityRevision}"}}}}`;
  return { archive, bundle, lock, registry, registryCommit, archiveDigest };
}

async function htaFixture(capabilities = "[]", namespace = "db.sqlite.wasm.hta") {
  const source = encoder.encode("(ns demo.world) (def world {:title \"Demo\"})");
  const worker = encoder.encode('import "./assets/chunk.js"; export const sqlite = true;');
  const asset = encoder.encode("export const asset = true;");
  const sourceDigest = await digest(source);
  const workerDigest = await digest(worker);
  const assetDigest = await digest(asset);
  const manifest = encoder.encode(
    `{:files {"src/demo/world.hal" {:size ${source.byteLength} :sha256 "${sourceDigest}"} `
      + `"provider/browser/provider.mjs" {:size ${worker.byteLength} :sha256 "${workerDigest}"} `
      + `"provider/browser/assets/chunk.js" {:size ${asset.byteLength} :sha256 "${assetDigest}"}} `
      + `:resources {"demo.world" "src/demo/world.hal"} `
      + `:extensions {${namespace} {:root "provider" :provider :hta :abi :hta.v1 `
      + `:targets {:browser {:provider "browser/provider.mjs" :runtime :web-worker} `
      + `:node {:provider "node/provider.mjs" :runtime :process}} `
      + `:assets ["browser/assets/chunk.js"] :exports {"version" {:args [] :returns :value} "open" {:args [:value] :returns :value}} `
      + `:capabilities ${capabilities}}}}`
  );
  const archive = zipSync({
    "package.edn": manifest,
    "src/demo/world.hal": source,
    "provider/browser/provider.mjs": worker,
    "provider/browser/assets/chunk.js": asset
  });
  const archiveDigest = await digest(archive);
  const registryCommit = "c".repeat(40);
  const identityRevision = "d".repeat(40);
  const lock = `{:lock/format "0.0.0-alpha" :packages {"demo:world" `
    + `{:version "1.0.0" :tap "hara" :registry-commit "${registryCommit}" `
    + `:identity-revision "${identityRevision}" :archive-sha256 "${archiveDigest}" `
    + `:namespaces [demo.world]}}}`;
  const registry = `{:registry/packages {"demo:world" {"1.0.0" `
    + `{:archive-sha256 "${archiveDigest}" :identity-revision "${identityRevision}"}}}}`;
  return { archive, lock, registry, registryCommit, worker, asset };
}

async function wasmHtaFixture(namespace = "fs.github.wasm") {
  const source = encoder.encode("(ns demo.world) (def world {:title \"Demo\"})");
  const module = new Uint8Array(await readFile(
    new URL("../../test-fixtures/hta-adapter/adapter.wasm", import.meta.url)
  ));
  const library = new Uint8Array(await readFile(
    new URL("../../test-fixtures/hta-adapter/library.wasm", import.meta.url)
  ));
  const sourceDigest = await digest(source);
  const moduleDigest = await digest(module);
  const libraryDigest = await digest(library);
  const manifest = encoder.encode(
    `{:files {"src/demo/world.hal" {:size ${source.byteLength} :sha256 "${sourceDigest}"} `
      + `"provider/provider.wasm" {:size ${module.byteLength} :sha256 "${moduleDigest}"} `
      + `"provider/library.wasm" {:size ${library.byteLength} :sha256 "${libraryDigest}"}} `
      + `:resources {"demo.world" "src/demo/world.hal"} `
      + `:extensions {${namespace} {:root "provider" :provider :wasm :module "provider.wasm" `
      + `:abi :hta.v1 :assets ["library.wasm"] `
      + `:exports {"sum" {:args [:i64 :i64] :returns :i64 :async true}} `
      + `:capabilities [:filesystem :network]}}}`
  );
  const archive = zipSync({
    "package.edn": manifest,
    "src/demo/world.hal": source,
    "provider/provider.wasm": module,
    "provider/library.wasm": library
  });
  const archiveDigest = await digest(archive);
  const registryCommit = "e".repeat(40);
  const identityRevision = "f".repeat(40);
  const lock = `{:lock/format "0.0.0-alpha" :packages {"demo:github" `
    + `{:version "1.0.0" :tap "hara" :registry-commit "${registryCommit}" `
    + `:identity-revision "${identityRevision}" :archive-sha256 "${archiveDigest}" `
    + `:namespaces [demo.world]}}}`;
  const registry = `{:registry/packages {"demo:github" {"1.0.0" `
    + `{:archive-sha256 "${archiveDigest}" :identity-revision "${identityRevision}"}}}}`;
  return { archive, lock, registry, module, library };
}

test("exact locks use the pinned registry and digest object endpoint", async () => {
  const { archive, lock, registry, registryCommit, archiveDigest } = await fixture();
  const requested = [];
  const resources = await loadLockedPackageResources(lock, async (url) => {
    requested.push(url);
    return new Response(url.includes("/v1/registry") ? registry : archive);
  }, "https://packages.example");

  assert.deepEqual(requested, [
    `https://packages.example/v1/registry?ref=${registryCommit}`,
    `https://packages.example/objects/sha256/${archiveDigest.slice(7)}`
  ]);
  assert.equal(resources["demo.world"], "(ns demo.world) (def world {:title \"Demo\"})");
});

test("semantic targets reject ambiguous lock names", async () => {
  const lock = `{:lock/format "0.0.0-alpha" :packages {"demo:one" `
    + `{:name "shared"} "demo:two" {:name "shared"}}}`;
  await assert.rejects(
    loadLockedPackageResources(lock, async () => new Response(""), "https://packages.example", ["shared"]),
    /package\/ambiguous-target: shared/
  );
});

test("semantic package names select verified bytecode bundles before source fallback", async () => {
  const { archive, bundle, lock, registry, archiveDigest } = await bytecodeFixture();
  const events = [];
  const runtime = {
    registerResource(namespace, source) {
      events.push(["source", namespace, source]);
    },
    raw: {
      registerPackageLock() {},
      evalBytecodeBundle(bytes) {
        events.push(["bytecode", bytes]);
      }
    }
  };
  const names = await installLockedPackages(runtime, lock, {
    targets: ["lang.model.v1.postgres"],
    origin: "https://packages.example",
    fetch: async (url) => new Response(url.includes("/v1/registry") ? registry : archive)
  });
  assert.deepEqual(names, ["postgres.core"]);
  assert.equal(events[0][0], "source");
  assert.equal(events[1][0], "bytecode");
  assert.deepEqual(events[1][1], bundle);
  assert.equal(archiveDigest.length, 71);
});

test("verified packages keep the source path when bytecode evaluation is unavailable", async () => {
  const { archive, lock, registry } = await bytecodeFixture();
  const events = [];
  const runtime = {
    registerResource(namespace, source) {
      events.push([namespace, source]);
    },
    raw: {
      registerPackageLock() {}
    }
  };
  const names = await installLockedPackages(runtime, lock, {
    targets: ["lang.model.v1.postgres"],
    origin: "https://packages.example",
    fetch: async (url) => new Response(url.includes("/v1/registry") ? registry : archive)
  });
  assert.deepEqual(names, ["postgres.core"]);
  assert.deepEqual(events, [["postgres.core", "(ns postgres.core) (def answer 42)"]]);
});

test("installation is atomic when a locked archive fails verification", async () => {
  const { archive, lock, registry } = await fixture();
  const registered = [];
  const runtime = {
    registerResource(namespace, source) {
      registered.push([namespace, source]);
    }
  };
  const corrupt = archive.slice();
  corrupt[corrupt.length - 1] ^= 1;

  await assert.rejects(
    installLockedPackages(runtime, lock, {
      origin: "https://packages.example",
      fetch: async (url) => new Response(url.includes("/v1/registry") ? registry : corrupt)
    }),
    /digest mismatch/
  );
  assert.deepEqual(registered, []);
});

test("the package provider activates and unloads an exact target", async () => {
  const { archive, lock, registry } = await fixture();
  const registered = [];
  const removed = [];
  let handler;
  const runtime = {
    registerResource(namespace, source) {
      registered.push([namespace, source]);
    },
    raw: {
      registerPackageLock() {},
      install_host_handler(value) { handler = value; },
      unregister_resource(namespace) { removed.push(namespace); }
    }
  };
  const provider = installPackageProvider(runtime, lock, {
    origin: "https://packages.example",
    fetch: async (url) => new Response(url.includes("/v1/registry") ? registry : archive)
  });

  await handler("package", "ensure", [{ "package/coordinate": "demo:world" }]);
  await handler("package", "ensure", [{ "package/coordinate": "demo:world" }]);
  assert.equal(provider.active.has("demo:world"), true);
  assert.equal(registered.length, 1);
  assert.equal(registered[0][0], "demo.world");
  assert.deepEqual(
    await handler("package", "unload", [{ "package/coordinate": "demo:world" }, {}]),
    ["demo:world"]
  );
  assert.deepEqual(removed, ["demo.world"]);
});

test("installation activates only the browser HTA target and publishes a Hara bridge", async () => {
  const { archive, lock, registry } = await htaFixture();
  const registered = [];
  const workers = [];
  const blobs = [];
  const revoked = [];
  let handler;
  const runtime = {
    registerResource(namespace, source) {
      registered.push([namespace, source]);
    },
    raw: {
      registerPackageLock() {},
      install_host_handler(value) { handler = value; }
    }
  };
  const packageProvider = installPackageProvider(runtime, lock, {
    origin: "https://packages.example",
    fetch: async (url) => new Response(url.includes("/v1/registry") ? registry : archive)
  });
  await packageProvider.handler("package", "ensure", [{ "package/coordinate": "demo:world" }]);
  const names = await installLockedPackages(runtime, lock, {
    origin: "https://packages.example",
    fetch: async (url) => new Response(url.includes("/v1/registry") ? registry : archive),
    workerFactory(url, options) {
      const value = new FakeWorker();
      workers.push({ value, url, options });
      return value;
    },
    createObjectURL(blob) {
      blobs.push(blob);
      return `blob:sqlite-${blobs.length}`;
    },
    revokeObjectURL(url) {
      revoked.push(url);
    }
  });

  assert.deepEqual(names, ["demo.world", "db.sqlite.wasm.hta"]);
  assert.equal(workers.length, 1);
  assert.equal(workers[0].options.type, "module");
  const bridge = registered.find(([namespace]) => namespace === "db.sqlite.wasm.hta")[1];
  assert.match(bridge, /\(ns db\.sqlite\.wasm\.hta\)/);
  assert.match(bridge, /Host\/call "db\.sqlite\.wasm\.hta" "version"/);
  assert.match(new TextDecoder().decode(await blobs[1].arrayBuffer()), /blob:sqlite-1/);

  const worker = workers[0].value;
  worker.emit({ type: "ready" });
  const result = handler("db.sqlite.wasm.hta", "version", []);
  await Promise.resolve();
  const call = worker.sent.find(message => message.type === "call");
  assert.deepEqual(decodeHta(call.frame), ["version", []]);
  worker.emit({
    type: "result",
    id: call.id,
    ok: true,
    frame: encodeHta(new Map([
      [new HtaKeyword("engine"), "sqlite"],
      [new HtaKeyword("version"), "3.50"]
    ]))
  });
  assert.deepEqual({ ...await result }, { engine: "sqlite", version: "3.50" });

  const failure = handler("db.sqlite.wasm.hta", "version", []);
  await Promise.resolve();
  const failedCall = worker.sent.at(-1);
  worker.emit({
    type: "result",
    id: failedCall.id,
    ok: false,
    frame: encodeHta(new Map([
      [new HtaKeyword("code"), new HtaKeyword("db/sqlite-error")],
      [new HtaKeyword("message"), "locked"]
    ]))
  });
  await assert.rejects(failure, error => error.code === "db/sqlite-error"
    && error.message === "db/sqlite-error: locked");

  const pending = handler("db.sqlite.wasm.hta", "version", []);
  const pendingRejection = assert.rejects(pending, /cancelled/);
  await Promise.resolve();
  pending.cancel();
  assert.equal(worker.sent.at(-1).type, "cancel");
  await pendingRejection;

  await disposeBrowserPackageProviders(runtime);
  assert.equal(worker.terminated, true);
  assert.deepEqual(revoked, ["blob:sqlite-2", "blob:sqlite-1"]);
});

test("installation activates a Wasm HTA package with inline module and library bytes", async () => {
  const { archive, lock, registry, module, library } = await wasmHtaFixture();
  const registered = [];
  const workers = [];
  const blobs = [];
  const runtime = {
    registerResource(namespace, source) {
      registered.push([namespace, source]);
    },
    raw: {
      registerPackageLock() {},
      install_host_handler(value) {
        runtime.handler = value;
      }
    }
  };
  const names = await installLockedPackages(runtime, lock, {
    origin: "https://packages.example",
    capabilities: ["filesystem", "network"],
    fetch: async (url) => new Response(url.includes("/v1/registry") ? registry : archive),
    workerFactory(url, options) {
      const worker = new FakeWorker();
      workers.push({ worker, url, options });
      return worker;
    },
    createObjectURL(blob) {
      blobs.push(blob);
      return `blob:wasm-${blobs.length}`;
    },
    revokeObjectURL() {}
  });

  assert.deepEqual(names, ["demo.world", "fs.github.wasm"]);
  assert.equal(workers.length, 1);
  const worker = workers[0].worker;
  const init = worker.sent.find(message => message.type === "init");
  assert.equal(init.backend, "wasm");
  assert.equal(init.providerUrl, undefined);
  assert.deepEqual(init.moduleBytes, module);
  assert.deepEqual(init.libraryBytes, library);
  assert.equal(blobs.length, 2);

  worker.emit({ type: "ready" });
  const result = runtime.handler("fs.github.wasm", "sum", [19, 23]);
  await Promise.resolve();
  const call = worker.sent.find(message => message.type === "call");
  assert.deepEqual(decodeHta(call.frame), ["sum", [19, 23]]);
  worker.emit({ type: "result", id: call.id, ok: true, frame: encodeHta(42) });
  assert.equal(await result, 42);
  assert.match(registered.find(([namespace]) => namespace === "fs.github.wasm")[1], /Host\/call/);
  await disposeBrowserPackageProviders(runtime);
  assert.equal(worker.terminated, true);
});

test("PostgreSQL :require activates only its generated browser HTA provider", async () => {
  const { archive, lock, registry } = await htaFixture("[]", "db.postgres.wasm.hta");
  const registered = [];
  const workers = [];
  const runtime = {
    registerResource(namespace, source) {
      registered.push([namespace, source]);
    },
    raw: {
      registerPackageLock() {},
      install_host_handler() {}
    }
  };
  const names = await installLockedPackages(runtime, lock, {
    origin: "https://packages.example",
    fetch: async (url) => new Response(url.includes("/v1/registry") ? registry : archive),
    workerFactory(url, options) {
      const worker = new FakeWorker();
      workers.push({ worker, url, options });
      return worker;
    },
    createObjectURL: (_blob) => `blob:postgres-${workers.length}`,
    revokeObjectURL() {}
  });

  assert.deepEqual(names, ["demo.world", "db.postgres.wasm.hta"]);
  assert.equal(workers.length, 1);
  assert.equal(workers[0].options.type, "module");
  assert.match(String(workers[0].url), /worker\.mjs$/);
  const bridge = registered.find(([namespace]) => namespace === "db.postgres.wasm.hta")[1];
  assert.match(bridge, /\(ns db\.postgres\.wasm\.hta\)/);
  assert.match(bridge, /Host\/call "db\.postgres\.wasm\.hta" "version"/);
  await disposeBrowserPackageProviders(runtime);
  assert.equal(workers[0].worker.terminated, true);
});

test("unsupported HTA capabilities fail before a browser worker is created", async () => {
  const { archive, lock, registry } = await htaFixture("[:filesystem]");
  let created = false;
  const runtime = {
    registerResource() {},
    raw: {
      registerPackageLock() {},
      install_host_handler() {}
    }
  };
  await assert.rejects(
    installLockedPackages(runtime, lock, {
      origin: "https://packages.example",
      fetch: async (url) => new Response(url.includes("/v1/registry") ? registry : archive),
      workerFactory() {
        created = true;
        return new FakeWorker();
      }
    }),
    /extension-capability-unsupported/
  );
  assert.equal(created, false);
});

test("failed bridge registration closes workers and revokes package object URLs", async () => {
  const { archive, lock, registry } = await htaFixture();
  const workers = [];
  const revoked = [];
  const runtime = {
    registerResource(namespace) {
      if (namespace === "db.sqlite.wasm.hta") throw new Error("resource registration failed");
    },
    unregisterResource() {},
    raw: {
      registerPackageLock() {},
      install_host_handler() {}
    }
  };
  await assert.rejects(
    installLockedPackages(runtime, lock, {
      origin: "https://packages.example",
      fetch: async (url) => new Response(url.includes("/v1/registry") ? registry : archive),
      workerFactory() {
        const worker = new FakeWorker();
        workers.push(worker);
        return worker;
      },
      createObjectURL: (_blob) => `blob:failed-${workers.length}`,
      revokeObjectURL: (url) => revoked.push(url)
    }),
    /resource registration failed/
  );
  assert.equal(workers[0].terminated, true);
  assert.deepEqual(revoked, ["blob:failed-0", "blob:failed-0"]);
});

class FakeWorker {
  constructor() {
    this.listeners = {};
    this.sent = [];
  }
  addEventListener(type, handler) {
    this.listeners[type] = handler;
  }
  postMessage(message) {
    this.sent.push(message);
    if (message.type === "close") queueMicrotask(() => this.emit({ type: "closed" }));
  }
  emit(data) {
    this.listeners.message({ data });
  }
  terminate() {
    this.terminated = true;
  }
}
