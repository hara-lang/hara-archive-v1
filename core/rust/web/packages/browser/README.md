# @hara-lang/browser

Embeddable Hara runtime for browsers and CDN scripts.

```js
import { start } from "@hara-lang/browser/vm";

const hara = await start();
console.log(hara.eval("(+ 19 23)"));
```

The package root remains an alias for `/vm`. Heavy-duty whole-function
WebAssembly compilation is available from `@hara-lang/browser/full`. The
compiler runs inside the browser runtime and the resulting module executes on
the browser's own WebAssembly engine:

```js
import { start } from "@hara-lang/browser/full";
const hara = await start();
const compiled = await hara.compileWholeWasm(
  "(loop [i 0 acc 0] (if (< i 5000) (recur (+ i 1) (+ acc i)) acc))"
);
console.log(compiled.call()); // 12497500n
```

The full package owns dynamic constants and persistent values in the outer Hara
runtime while generated scalar and specialized collection work runs directly
inside the browser's WebAssembly engine.

The release also provides an IIFE bundle for a plain script tag:

```html
<script src="https://unpkg.com/@hara-lang/browser@0.1.0/dist/hara-wasm-vm/hara.js"></script>
<script>
  Hara.start().then((hara) => console.log(hara.eval("(+ 19 23)")));
</script>
```

The default browser runtime is the small core evaluator. Foundation and other
semantic package families are opt-in, so a page can choose the exact lock and
load only the capabilities it needs. Host resources can still be registered
before requiring them:

```js
const hara = await Hara.start({
  resources: {
    "app.config": "(ns app.config) (def answer 42)"
  }
});
```

Locked Hara packages can be fetched from an immutable package host (including
`packages.*`) or a release asset and installed before application evaluation:

```js
import { installLockedPackages, start } from "@hara-lang/browser";

const hara = await start();
const lock = await fetch(projectLockUrl).then((response) => response.text());
await installLockedPackages(hara, lock);
hara.require("my.world");
```

The same selection can happen during isolated startup. `targets` accepts a
semantic package name, an exact lock coordinate, or a namespace:

```js
const hara = await start({
  lock,
  targets: ["lang.model.v1.postgres"],
  packageOptions: { origin: "https://packages.example" }
});
```

HARP archives may contain a verified `bytecode/package.hbx`. The loader checks
its manifest digest and installs the HBX0 index when the VM runtime exposes the
bundle seam; otherwise the verified HAL resources remain the source fallback.
`config/packages.edn` follows Foundation's `:include` (`:base`, `:complete`,
and `:exclude`) and dependency conventions when building semantic packages.

Memory-backed Wasm packages use the explicit `memory.v1` binding route. The
manifest, canonical interface, and canonical `bindings.edn` plan are verified
before the module is instantiated:

```js
hara.installMemoryWasmBinding(manifest, interfaceSource, bindingsSource, wasmBytes);
hara.require("my.wasm.package");
```

Only format-2 locks are accepted. The loader verifies the HARP archive digest,
optional archive size, every file declared by `package.edn`, safe archive paths,
and unique HAL namespaces. Resources are registered only after the complete
lock has passed verification. A lock entry may use `:distribution/url`,
`:packages/url`, `:release-url`, or `:url`; package distribution URLs take
precedence and the lock digest remains authoritative.

Verified `:hta` extensions select only their prebuilt `:browser` web-worker
target. Declared assets are loaded from the archive, and unsupported
capabilities fail during installation; no Cargo, Maven, or compiler step is
performed.
