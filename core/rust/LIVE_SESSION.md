# Live execution and compiler targets

This document defines the ownership boundary for interactive execution and the
terminology for Hara compiler products. It complements `ARCHITECTURE.md`; it
does not change the security guarantees of `Kernel`, `Sandbox`, `Session`, or
`Runtime`.

## Ownership

```text
Kernel
  └── SandboxProvider / Sandbox
        └── private Session
              ├── Runtime
              │     └── interpreter, HBC, or whole-Wasm backend
              └── LiveSession lifecycle and serialization
```

A `Sandbox` is the supervised environment and authority envelope. It owns
provider selection, mounts, immutable transfer, limits, cancellation, and
closure. It is not an evaluator and does not own editor revisions.

A `LiveSession` is the backend-neutral control boundary for one interactive
execution. It owns:

- stable session, source, revision, generation, request, and sequence identity;
- stale-generation and stale-revision rejection;
- normalized lifecycle state and explicit backend capabilities;
- source replacement policy;
- backend dispatch and response normalization.

The backend continues to own frames, continuations, runtime values, promises,
compiled programs, and evidence documents. The common layer never serializes a
live runtime object merely to make two engines look identical.

### Sandbox-private Session hosting

A native sandbox provider may construct a zero-authority private `Session`
through `restricted_sandbox_session` or
`restricted_sandbox_session_with_host`. That Session, not the public Sandbox
facade, owns its live-session registry. It may:

- start authoritative interpreter or feature-gated HBC live sessions;
- retain backend objects privately while publishing only state and capabilities;
- dispatch the existing generation- and revision-fenced request envelope;
- dispose every nested live session before releasing its Runtime and mounts.

Live-session identities remain reserved for the lifetime of the owning Session,
even after cancellation or disposal. Closing the owner disposes and forgets all
nested live sessions exactly once.

This is a Rust provider embedding seam. It does not add live-session methods to
`SandboxInstance`, `SessionKernel::sandbox_*`, or the Hara Sandbox surface.
Sandbox therefore retains its coarse `eval`, `call`, `cancel`, `status`, and
`close` contract and never exposes evaluator frames, continuations, handles, or
backend-specific observations.

## Protocol

The initial protocol identifier is `hara.live-session/0-alpha`. State documents
use `hara.live-session.state/0-alpha` and include:

```text
session-id
source-id
generation
revision
sequence
backend      interpreter | hbc | whole-wasm
status       ready | running | paused | suspended | returned |
             failed | cancelled | disposed
```

Commands are checked against the current session identity before reaching a
backend. A supplied generation or revision must match the active state. This
prevents delayed UI messages and resumed tool calls from mutating a newer
source generation.

The instance command vocabulary is:

```text
snapshot  step  run  pause  resume  resolve  reject
update    reset cancel dispose
```

Support is capability-driven. The interpreter does not claim `pause`; a
whole-Wasm backend must not claim stepping until it can expose a real bounded
continuation. Hosts should render or route only the operations advertised by
the selected backend.

The Rust implementation lives in `src/live_session.rs` and
`src/live_session/*.rs`. The existing interpreter observation ABI and HBC
observation session remain intact and are adapted behind this boundary.

## Source replacement

Every update carries a new source identifier, revision, source text, and one of
three policies:

- `restart`: validate the replacement, terminate the current execution, and
  activate a new generation immediately;
- `replace-on-next-start`: retain the current execution and activate the queued
  revision at the next reset/start boundary;
- `preserve-runtime`: reload only when a backend can prove that active state is
  safe to retain.

The initial interpreter and HBC adapters implement `restart` and
`replace-on-next-start`. They reject `preserve-runtime`. There is no closure
migration, parked-promise migration, stack rewriting, or active-continuation
patching.

Replacement is transactional where possible: a replacement program/session is
created before the current backend instance is released. A failed replacement
therefore leaves the active generation unchanged.

## Compiler product terminology

“Wasm compilation” previously covered several unrelated products. The runtime
now uses these names:

| Product | Meaning |
| --- | --- |
| runtime-host-wasm | Rust runtime or transport adapter compiled for `wasm32` |
| hbc-module | validated Hara bytecode module |
| hbx-package | package/container operation over one or more modules |
| whole-wasm | standalone Wasm lowered from validated HBC |
| extension-wasm | extension module loaded through the runtime extension ABI |

Only `hbc-module` and `whole-wasm` are source compiler targets today. The
whole-Wasm product is the versioned HNW0 artifact described in
[`HNW0_HTA_BOUNDARY.md`](HNW0_HTA_BOUNDARY.md); HTA0 remains an external value
and lifecycle boundary rather than another source compiler target.
`hara-compiler` exposes them through `CompileTarget` and the single
`compile(source, target)` entry point. Target identity and ABI version are
defined by `CompileTarget::product_identity`, so callers do not need to infer
the product from an artifact filename or a legacy helper name.

HBX remains a packaging concern rather than a second source compiler. Runtime
host Wasm and extension Wasm remain build-system products rather than Hara
source targets.

## Target tree

```text
project sources
  → project analysis, dependency reachability, deterministic retention
  → retained canonical module set
  → HALC / HBC provider
       ├── hbc-module
       ├── hbx-package
       └── whole-wasm
             └── lower validated HBC to standalone Wasm
```

There must be one project-level source and dependency front end. Whole-Wasm
must not recreate parsing, namespace reachability, tree shaking, or package
selection. It consumes the validated HBC product produced by the shared front
end.

The explicit target API is the source/compiler seam. HNW0 format versioning and
build-product manifests remain separate, reviewable changes from the HTA0
transport contract.

## Migration order

1. Route native hosts and tests through the common live-session contract.
2. Add browser serialization for the same request, state, capability, and reply
   schemas; keep the legacy browser wrappers as compatibility facades.
3. Move Studio controls to capability-driven commands and revision guards.
4. Host live sessions beneath Sandbox-provider private Sessions without
   expanding the public Sandbox protocol.
5. Extract the target-neutral retained-module seam from project compilation.
6. Add HBX packaging and whole-Wasm lowering as consumers of retained HBC.
7. Emit one versioned build-product manifest for browser and deployment
   loaders, then remove hard-coded product guesses.
