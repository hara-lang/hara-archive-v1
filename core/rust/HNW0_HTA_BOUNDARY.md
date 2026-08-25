# HNW0 and HTA0 boundary

This document records the ownership boundary between Hara's internal compiled
execution product and its external portable extension boundary. It complements
`ARCHITECTURE.md` and `LIVE_SESSION.md`.

## Product ownership

| Product | Boundary | Owner | Contents |
| --- | --- | --- | --- |
| HBC0 | internal runtime/compiler | `vm` and bytecode providers | canonical validated Hara program |
| HNW0 | internal compiled execution | `whole_wasm` | generated Wasm, canonical HBC0 fallback, function metadata, and a target declaration table |
| HTA0 | external portable value/lifecycle transport | `hta` and `web/packages/hta` | typed values, handles, requests, results, cancellation, and structured errors |
| extension Wasm | package/provider implementation | extension providers | a module selected by a declaration and invoked through the extension ABI |
| runtime-host Wasm | delivery/runtime host | `wasm-bindgen` runtime | the Hara runtime compiled for a host; it is not an HNW0 artifact or an extension |

HNW0 and HTA0 are complementary, not interchangeable. HNW0 is the compiled
execution product used inside a runtime. HTA0 is the portable boundary used by
an extension, worker, or host. A host may adapt HNW0 values to HTA0, but that
adapter is an explicit boundary and does not make HTA0 part of the generated
HNW0 module.

## HNW0 declaration and dispatch

The HNW0 compiler consumes canonical HBC0 and emits one generic bridge ABI.
Generated modules do not import a protocol- or native-keyed function name. The
fixed imports are:

```text
constant_handle(index) -> handle
box_i64(value) -> handle
unbox_i64(handle) -> i64
value_construct(target, slots, arity) -> handle
target_call(target, slots, arity, result-mode) -> value
```

`Target::ALL` in `whole_wasm/bridge.rs` is the declaration inventory. It owns
the target symbol, kind, arity, and artifact-local ID. `target_table()` writes
that declaration into HNW0; artifact decoding validates the table before Wasm
instantiation; both the Wasmtime and browser hosts dispatch from the decoded
descriptor. Code generation and representation inference refer to the same
declarations instead of maintaining independent numeric or suffix-based
registries.

The target-call sequence is:

```text
Hara source
  -> canonical HBC0
  -> HNW0 lowering
  -> generic target_call/value_construct import
  -> decoded declaration (ID, kind, arity)
  -> protocol/native intrinsic or collection constructor
  -> declared result mode
```

The Wasm module owns scalar arithmetic, control flow, and supported linear
collection kernels. The host owns dynamic Hara values, handles, protocol and
native dispatch, target validation, and fallback when a value cannot be
represented by the native path. The retained HBC0 bytes are the semantic
fallback; they are not a second compiler input or a second dispatch registry.

## HTA0 declaration and lifecycle

An extension declaration identifies its namespace, version, provider, ABI,
typed exports, capabilities, host calls, handles, assets, and (for a portable
`:hta` provider) node/browser targets. The declaration is validated before the
provider is started. A typed export declares:

```text
args    vector of wire type keywords
returns one wire type keyword or a vector of wire type keywords
async   optional boolean
```

The external lifecycle is explicit:

```text
load declaration
  -> verify package files, digest, target, ABI, and capabilities
  -> start provider once
  -> invoke declared export
  -> resolve/reject or cancel request
  -> release declared handles
  -> shutdown provider exactly once
```

HTA0 values cross this boundary as encoded frames. Handles are scoped to the
provider session and are released through the declared lifecycle; they must not
be treated as HNW0 linear-memory addresses. The browser package owns the
worker/transport implementation, while the Rust extension parser and provider
own native validation and execution. Both reject undeclared exports, invalid
arities, unsupported target runtimes, malformed assets, and capability drift.

## Instrumentation ownership

Instrumentation observes the execution boundary; it does not become a second
dispatch mechanism.

| Boundary | Event | Producer data |
| --- | --- | --- |
| HNW0 target bridge | `semantic/protocol-call` | target symbol, arity, result mode, `enter`/`return`/`error` status |
| HNW0/live session | `execution/terminal` | terminal status |
| HBC/interpreter/live session | same normalized event kinds | backend target and terminal status |
| HTA provider | lifecycle/request events | provider identity, request, cancellation, release, and structured result/error |

The HNW0 host emits bridge events around the generic call after declaration
validation. A failed validation or dispatch emits an error status when an
instrumented target is active. The live-session owner emits one terminal event
for the backend lifecycle. HTA transport events describe external requests and
must not be confused with internal protocol calls.

## Invariants and conformance evidence

The following invariants are required for every host:

1. HNW0 artifacts are deterministic and retain canonical HBC0 bytes.
2. The decoded HNW0 target table is canonical before instantiation.
3. Native and browser hosts accept the same generic imports and declaration
   table, with the same slot, handle, arity, result-mode, and error rules.
4. HTA manifests have the same typed export and target contract in Rust and
   browser loaders.
5. Package verification precedes provider startup, and shutdown is idempotent.
6. Instrumentation records the same normalized event kinds at HBC and HNW0
   execution boundaries without adding protocol-keyed imports.

Rust conformance lives in the HNW0 artifact/bridge tests, extension/provider
tests, package-loader tests, and whole-Wasm corpus. Browser conformance lives
in the HTA loader, package-loader, and whole-Wasm browser tests. Cross-host
fixtures should compare decoded declarations, lifecycle outcomes, and event
sequences rather than implementation-specific handles or object identities.
