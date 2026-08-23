# Hara execution host `0-alpha`

Status: normative Hara-owned contract for provider adapters and conformance
fixtures. This contract is intentionally independent of MCP, OAuth, browser
extension, and application records.

## Boundary

An execution host is the single boundary between a client and one restricted
Hara runtime:

```text
client -> hara.execution-host/0-alpha -> Sandbox -> Session -> Runtime
```

The host does not expose evaluator frames, live Hara values, Vars, closures,
promises, continuations, native handles, provider credentials, browser objects,
or filesystem handles. Persistent interactive execution remains owned by
`LiveSession`; it is not an execution-host session.

## Closed records

Every record has an exact `:protocol` field and rejects unknown fields. IDs are
opaque non-empty strings; digests use `sha256:<lowercase-hex>`. Timestamps are
UTC RFC 3339 strings and limits are positive integers.

```clojure
{:protocol "hara.execution-host/0-alpha"
 :host/id "...opaque..."
 :host/generation 1
 :runtime {:backend :browser-wasm | :native-rust | :jvm-truffle | :fixture
           :build/id "..."
           :build/digest "sha256:..."
           :version "..."}
 :capabilities {:operations #{:runtime.get :sandbox.eval :sandbox.call
                              :sandbox.check :sandbox.cancel}
                :profiles ["hara.mcp-pure/0-alpha"]
                :limits {:source-bytes 65536
                         :result-bytes 1048576
                         :output-bytes 1048576
                         :evaluation-ms 5000}}}
```

The `:capabilities` set is a closed declaration. An operation not declared by
the host fails with `:execution-host/capability-unsupported`; it never falls
back to another evaluator, backend, or session.

The pure profile is an exact value, not a host hint:

```clojure
{:profile/id "hara.mcp-pure/0-alpha"
 :network :none
 :browser :none
 :filesystem :ephemeral-read-only
 :persistence :none
 :ambient-host-authority :none
 :sandbox-reuse false
 :external-effects false}
```

A host may advertise this profile only when it can prove that browser APIs,
DOM and site adapters, downloads, DevTools, sockets, processes, package
installation, external resource discovery, host filesystem roots, IndexedDB,
provider credentials, mounts, application resources, and parent
Kernel/Session/Runtime identities are unavailable. The profile may resolve
only the foundation needed by the requested computation. The restricted
runtime must remove the `Runtime`, `Kernel`, `Sandbox`, `Package`, `Crypto`,
`OS`, `Process`, `File`, `Socket`, and `Host` native surfaces at namespace
resolution, not merely reject their host calls.

### Source bundles

```clojure
{:protocol "hara.source-bundle/0-alpha"
 :bundle/digest "sha256:..."
 :bundle/files [{:path "src/example.hal"
                 :media-type "text/x-hara"
                 :bytes 42
                 :digest "sha256:..."
                 :namespace "example"}]
 :bundle/file-count 1
 :bundle/byte-count 42}
```

Paths are canonical project-relative UTF-8 paths. They must not be absolute,
empty, traversing, duplicated, URL-like, credential-bearing, or symlinks.
Files are bounded before mounting, and the bundle digest covers the canonical
ordered file descriptors and bytes. Adapters mount a validated bundle
read-only in ephemeral memory and discard it when the sandbox closes. The
initial profile supports flat `.hal` resources and one explicitly bounded
archive expansion; archives may not introduce a second root or links.

### Lease and request

```clojure
{:protocol "hara.execution-lease/0-alpha"
 :lease/id "..."
 :host/id "..."
 :host/generation 1
 :audience "..."
 :operation :sandbox.eval
 :profile "hara.mcp-pure/0-alpha"
 :request/digest "sha256:..."
 :capabilities/digest "sha256:..."
 :runtime/digest "sha256:..."
 :limits {:source-bytes 65536 :result-bytes 1048576
          :output-bytes 1048576 :evaluation-ms 5000}
 :issued-at "..."
 :expires-at "..."}
```

The lease is a request-bound authorization projection, not a bearer credential.
It contains no token-passthrough or provider credential field. A changed
operation, source, arguments, profile, capability manifest, runtime build,
audience, or limit requires a new lease. A stale host generation or expired
lease fails closed.

```clojure
{:protocol "hara.execution-request/0-alpha"
 :run/id "..."
 :operation :sandbox.eval | :sandbox.call | :sandbox.check
 :profile "hara.mcp-pure/0-alpha"
 :source {:text "..." :digest "sha256:..."}
 :target {:namespace "example" :var "answer"} ; call only
 :arguments {...}                              ; transfer-safe only
 :source-bundle "sha256:..."
 :limits {...}
 :lease/id "..."}
```

`sandbox.call` resolves an already-loaded qualified Var and invokes it through
the canonical direct-call ABI. It never concatenates argument source.
Unsupported call or check capabilities are explicit failures.

## Events, results, and lifecycle

The host emits one monotonic sequence per run. The terminal event and result are
immutable and are emitted at most once.

```clojure
{:protocol "hara.execution-event/0-alpha"
 :host/id "..."
 :host/generation 1
 :run/id "..."
 :sequence 1
 :state :accepted | :running | :cancelling | :completed | :failed
          | :cancelled | :timed-out | :closed
 :payload {...bounded transfer-safe projection...}}
```

```clojure
{:protocol "hara.execution-result/0-alpha"
 :run/id "..."
 :status :completed | :failed | :cancelled | :timed-out
 :value {...transfer-safe projection...}
 :stdout "...bounded..."
 :stderr "...bounded..."
 :diagnostics [{:code :... :message "..."}]
 :artifacts [{:id "..." :media-type "..." :bytes 0 :digest "sha256:..."}]
 :runtime {...exact host runtime identity...}
 :evidence {:source-bundle "sha256:..."
            :sandbox-profile "hara.mcp-pure/0-alpha"
            :started-at "..."
            :completed-at "..."
            :elapsed-ms 0
            :cleanup :completed | :uncertain}}
```

Only nil, booleans, finite numbers in the agreed range, strings, keywords
projected as strings, and bounded arrays/maps with transfer-safe keys may cross
the result boundary. Closures, Vars, promises, continuations, browser objects,
provider handles, and mutable native state are rejected. Output, diagnostics,
artifacts, nesting, and item counts are bounded before the terminal result is
returned.

The lifecycle is:

```text
open -> eval/call/check -> status/cancel -> close
```

`cancel` and `close` are idempotent. Cancellation, timeout, disconnect, and
worker failure settle a run exactly once. A pure run always creates a fresh
restricted Sandbox and discards its Session, Runtime, mounts, source bundle,
and event state after settlement. A terminal result does not imply persistence,
publication, external delivery, or verified external state.

## Stable errors

Errors are bounded records with `:protocol`,
`:code`, `:message`, and `:retryable`. The stable codes include:

```text
execution-host/invalid-record
execution-host/unsupported-version
execution-host/capability-unsupported
execution-host/stale-generation
execution-host/lease-invalid
execution-host/lease-expired
execution-host/request-mismatch
execution-host/source-bundle-invalid
execution-host/limit-exceeded
execution-host/cancelled
execution-host/timed-out
execution-host/result-non-transferable
execution-host/cleanup-uncertain
```

## Conformance corpus

Provider-neutral fixtures must consume these records unchanged and cover:

- exact closed-record and version rejection;
- capability closure and explicit unsupported operations;
- source path canonicalization, traversal, collision, size, archive, and digest
  failures;
- lease audience, host-generation, expiry, and request-content binding;
- eval, direct qualified call, check, cancellation, timeout, output limits, and
  close;
- monotonic event sequence and terminal-state immutability;
- transfer-safe values and rejection of live values;
- no-network, no-browser, no-persistent-storage, no-host-filesystem, and
  no-parent-runtime negative proofs; and
- browser Wasm, native Rust, JVM/Truffle, and hermetic fixture parity.

Capability skips are explicit records, never successful no-ops. The raw
browser-Wasm adapter therefore uses a fresh transient restricted session and
does not reuse the ordinary `ROOT` session; an empty host-call map alone is not
evidence for `hara.mcp-pure/0-alpha`.
