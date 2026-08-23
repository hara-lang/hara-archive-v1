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

The Hara HAL catalog is embedded in the Wasm runtime. Host resources can be
registered before requiring them:

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
