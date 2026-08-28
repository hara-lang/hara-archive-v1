# Rust runtime architecture

The Rust runtime follows the responsibility boundaries of the Java runtime
without copying its class hierarchy. The public Rust surface remains flat:
existing paths such as `hara_wasm::Runtime` and `hara_wasm::core::Value`
continue to work.

## Compatibility facades

`src/core.rs` remains the core facade. The target-neutral runtime source graph
is owned by the `hara-runtime` crate through `src/runtime_lib.rs`; `src/lib.rs`
is now the thin `hara-wasm` delivery facade. This keeps one implementation
graph while giving lower layers a dependency that does not point back through
the browser package.

The runtime declares the responsibility-focused implementation fragments:

- `src/core/` contains values, environment and protocol state, native
  operations and providers, asynchronous values, primitives, forms, namespace
  loading, and evaluation.
- `src/runtime/` contains the embedding model, sessions, runtime bootstrap,
  bytecode integration, native evaluation, WebAssembly bridges, and runtime
  tests.

The fragments use `include!` deliberately. They are compiled once in the
runtime owner's module rather than reassembled by delivery crates, preserving
visibility, symbol paths, and macro scope. The raw Wasm, observation, VM, and
compiler crates now consume that owner through explicit interfaces. This makes
the reorganization structural rather than behavioral.

Dependencies point inward: embedding and runtime code may use core facilities;
core facilities do not depend on the delivery facade. `hara-vm` and
`hara-compiler` consume `hara-runtime` directly; `hara-wasm` only re-exports
the runtime for browser/native delivery. The local `hara-runtime` package is
intentionally an internal workspace boundary while its source root remains
shared with the distribution package. Experimental bytecode and WebAssembly
adapters stay at the runtime boundary. Its library target is not independently
publishable: the source graph intentionally lives outside that manifest and
the root distribution package remains the publication facade. The runtime
package owns the implementation unit-test target so filtered Cargo commands
cannot pass while testing the empty delivery facade. `hara-wasm` owns
integration, raw-Wasm, and browser tests. Keep path-sensitive source ownership
in `runtime_lib.rs`; do not create a second included source graph.

## Live execution and compiler products

`LIVE_SESSION.md` defines the backend-neutral live-session contract, its
ownership beneath Sandbox-private Sessions, source replacement semantics, and
the distinction between HBC, whole-Wasm, runtime-host Wasm, and extension Wasm
products. New interactive execution and compiler-target work must preserve
those boundaries rather than adding evaluator behaviour to `Sandbox`.

## Layout policy

Most Rust files remain subject to the repository's line-count gate. The
`core` and `runtime` facade trees are exempt because their first constraint
is API and raw-crate compatibility. Their files are grouped by responsibility,
and can be converted into encapsulated modules later as those compatibility
constraints are retired.

New module trees should use `module.rs` with `module/*.rs`; do not add new
`mod.rs` files.
