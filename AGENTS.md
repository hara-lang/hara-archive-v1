# AGENTS.md

Repo layout and per-component build/test commands. See `README.md` for the
component map.

## Native Hara source workflow

Use `$hara-development` for every `.hal` change. Evaluate the complete proposed
file with the native Hara runtime, run the focused test, write the edit, run the
written file in a fresh Hara process, and repeat the test. Hara execution is
fast and isolated, with no retained runtime state between validations. Correct
parser failures in the candidate before writing it.

Use `$hara-postgres` for native PostgreSQL DSL source, `$hara-xtalk` for XTalk
or emitter work, `$hara-xtalk-compatibility` for target parity, and
`$hara-dev-spec-writer` for language specifications.

For collection/core call shapes, read
`core/spec/std/variadic-functions.md` before editing. It records the
source-backed Foundation and runtime boundaries, including the distinction
between the multi-pair `std.foundation/assoc` wrapper and its fixed
three-argument protocol method; verify the actual resolved owner instead of
assuming a Clojure equivalent has the same arity.

Use intrinsic protocol method symbols such as `IAssoc/assoc` when that
intrinsic is available; do not spell the equivalent full
`std.protocol.*` path in new Hara source.

## Compact Hara syntax

Prefer the shortest valid Hara form. Use fully qualified symbols only when
absolutely necessary to resolve ambiguity, cross a namespace boundary, or
refer to a symbol that is not available through a local alias or intrinsic.

Prefer:

- `(apply ...)` over `(std.foundation/apply ...)` inside `std.foundation`.
- `(ILookup/lookup value key)` over
  `(std.protocol.ilookup.ILookup/lookup value key)`.
- Local symbols over qualified references to the current namespace.

Do not introduce qualification merely for clarity. Preserve the compact local
style of surrounding code. Before adding a qualified symbol, verify that the
short form is actually ambiguous or unavailable.

## Hara error handling

Hara uses structured guest exceptions, not Java or Rust exception syntax.
Construct errors with `ex`, raise them with `throw`, and inspect them with
`ex-data`, `ex-message`, `ex-cause`, `ex-class`, or `ex-provenance` as needed.
The `ex` form takes an error code, data, and optional keyword settings:

```hara
(ex :io
    {:path path}
    :ex/message "Unable to read path"
    :ex/class :ex.class/io
    :ex/cause cause)
```

Keep the error code and data meaningful; use keyword options for standard
exception metadata rather than burying messages or classes in arbitrary data.

Use the catch-only form for a catch-all guest exception handler:

```hara
(try
  (operation)
  (catch error
    (recover error)))
```

Do not use host-language selectors such as `Throwable`, or Java-shaped forms
such as `(catch Throwable error body)`, in portable Hara source. Use a
namespaced keyword or keyword vector when the handler intentionally selects
particular structured error codes:

```hara
(try
  (read-file path)
  (catch :file/not-found error
    (recover-missing path error))
  (catch [:file/permission-denied :file/closed] error
    (recover-file-error path error)))
```

These catch selectors match the error code supplied as the first argument to
`ex`; they are not host exception classes. Preserve the caught Hara exception
when wrapping or rethrowing it, and use `finally` for deterministic cleanup.

## Namespace roles and publication

Do not add top-level `defn-`, `defmacro-`, or private Vars to `.hal` source.
Implementation functions belong in an explicitly internal namespace and remain
ordinary, inspectable, directly testable definitions:

```hara
(ns example.codec.internal
  (:config {:role :internal}))
```

A supported porcelain namespace declares `:facade`, contains no definitions or
load-time effects, and publishes a coherent owner with `intern-all`:

```hara
(ns example.codec
  (:config {:role :facade})
  (:require [example.codec.internal]))

(intern-all example.codec.internal)
```

Use `intern-in` instead when only selected symbols are supported. Never convert
a private helper to public inside an `intern-all` owner without first deciding
that it belongs to the exported API; move unsupported helpers to another
`:internal` owner.

Mark every supported, recommended API Var at its owning definition:

```hara
^{:public true}
(defn encode [value] ...)
```

Autocomplete and documentation tools use `:public true` to prioritize these
Vars. The marker does not change visibility, namespace role, access, or
publication. Ordinary unmarked definitions remain resolvable and directly
testable but are not recommended API. Put the marker on the owner; `intern-all`
and `intern-in` must preserve it when a facade publishes the Var.

## Reversible state and transformations

Every state-owning component must expose a deterministic reset, teardown, or
snapshot/restore boundary. It must restore the documented baseline after normal
use, partial initialization, and failure, and repeated reset/teardown calls must
be safe. Stateful tests start from that baseline and restore it on every exit
path.

Every data-format transformation has a tested inverse: encode/decode,
parse/render, serialize/deserialize, or the domain-equivalent pair. Test both
round-trip directions where the representations are equivalent. If a boundary
is intentionally lossy or canonicalizing, document the lost information,
retain sufficient source or provenance to restore/reconstruct the prior form,
and test canonicalization for idempotence.

## Corresponding unit tests

Source and tests are a required pair. A source namespace under `core/lib/src`
or `core/lib/src-lang` has a test namespace at the same relative path under the
corresponding `core/lib/test` or `core/lib/test-lang` root, using the
`*_test.hal` filename. Every function or macro, including ordinary definitions
in `:internal` namespaces, must have a corresponding test block identified by
`^{:refer namespace/symbol}` and containing a real behavioral assertion.

The first post-implementation action is to run the native scaffold, before
hand-writing or reorganising tests:

```shell
hara --project core --offline manage scaffold namespace.name
hara --project core --offline manage scaffold namespace.name --write
```

The first command previews the paired test-file edit; the second applies it.
For bootstrap, `std.native.*`, and `std.protocol.*` seams that use `Test/run`
blocks, use `std.foundation.bootstrap/scaffold` and its coverage report.

A generated `fact` without an assertion or an empty `Test/run` block is a
pending test, not coverage. Replace every generated stub with meaningful
assertions, run the path-matched focused test, then use `code.manage`'s
`incomplete`, `unchecked`, or `pedantic` report as appropriate to confirm the
changed namespace has no missing or placeholder tests. A source change is not
complete while its corresponding tests are absent, pending, or failing.

Scaffolding inventories the tests that must be written; it does not author
them. Read each implementation, identify its semantic contract, and hand-write
the permanent test body with the same care as the source. Assert exact stable
results rather than merely a broad type, truthiness, non-nil, successful
delivery, or absence of an exception. Cover the function's meaningful branches
and boundaries, expected failures, observable state transitions, cleanup or
reset behavior, and inverse or round-trip properties where applicable. A broad
predicate is appropriate only when that predicate is itself the documented
contract.

Prove that each new or materially changed test can detect a regression. Run it
against the pre-change behavior or a deliberately incorrect candidate or
expectation and observe the focused test fail; then restore the intended code
and assertion and observe it pass. One corresponding test block per function
is the minimum index into the source, not a limit on the assertions needed to
describe its behavior.

## Layout

- `core/` — language runtime source code:
  - `core/java/` — Java/Truffle runtime (Maven, JDK 21)
  - `core/rust/` — Rust/embedding runtime (native CLI, wasm builds, web loader,
    and shared extension ABI). Concrete extensions live in
    `../../extensions/hara-runtime/`. The old `wasm/` tree was
    removed — never reference it; everything is `core/rust/`. `core/rust/web/`
    holds the browser loaders plus the shared studio environment
    (`core/rust/web/studio/`, mounted by the hara-www studio page and the
    greenways-os DevTools panel).
  - `core/lib/` — Hara-language sources. Compiler, runtime, and host libraries
    live under `core/lib/src` with tests in `core/lib/test`; portable `xt.*`
    libraries live under `core/lib/src/lang`; portable `xt.*` and `postgres.*`
    libraries remain under `core/lib/src-lang`, with tests in
    `core/lib/test-lang`. Examples are in `core/lib/examples/` and benchmarks
    in `../../website/hara-benchmarks/runtime/hara/`. Notable namespaces include `std.foundation`, the
    `lang.*` compiler port, and the `db.ledger.*` consensus-free
    executable-chain experiments.
  - `core/spec/` — parity specifications and substrate tests for core language
    targets.
- `scripts/runtime/` — runtime build, release, and conformance scripts.
- `../../website/hara-benchmarks/astro/` — runtime benchmark suites and generated site data.
- `../hara-archive/retired/registry-api/` — archived registry/publication API.

External sibling repos (checked out next to this repo in the workspace):

- `../../website/hara-www/` — landing page for `www.hara-lang.org` (`hara-lang/hara-www`),
  also checked out at `../../website/hara-www/` inside this repo by CI workflows.
- `../../extensions/` — `hara-vscode`, `hara-emacs`, `hara-lsp`
  (`hara-lang/hara-extensions`); the Chrome extension is in
  `../../application/greenways-os/extension/hara-chrome`.
- `../hara-specs-registry/` — normative specifications and conformance corpora
  (`hara-lang/hara-specs-registry`).
- `../hara-archive/` — legacy source repository (`hara-lang/hara-archive`).
- `../hara-archive/retired/platform-cloudflare/` — superseded reference implementation of the read-only
  edge for `id.hara-lang.org` / `packages.hara-lang.org`. Serving moved to
  Netlify sites driven by the `hara-lang/hara-identity` and
  `hara-lang/hara-packages` repos. The worker is no longer deployed; see its
  README.
- `notes/` — working documents, NOT published: design notes and
  `notes/superpowers/` (plans/specs written by the superpowers plugin).
  Put nothing here that belongs on the website.
- `../hara-specs-registry/00-unsorted/contrib/` — separately owned specifications
  and reference implementations.
- `../../website/hara-docs/docs/books/` — published Hara books.
- `LICENSES/` — license metadata for vendored or adapted components.

## Split-out components

These formerly in-tree components now live in their own repos (tracked under
the workspace super-repo):

- website — `hara-lang/hara-www` (Astro site for www.hara-lang.org, with the
  `vendor/hara-ui` snapshot), checked out at `../../website/hara-www/` (CI checks it
  out at `../../website/hara-www/` inside this repo).
- extensions — `hara-lang/hara-extensions` (`hara-vscode`, `hara-emacs`,
  `hara-lsp`), checked out at `../../extensions/`; the Chrome extension lives in
  `application/greenways-os/extension/hara-chrome`.
- specs — `hara-lang/hara-specs-registry` (normative specifications and conformance
  corpora), checked out at `../hara-specs-registry/`.
- archive — `hara-lang/hara-archive` (legacy source repository), checked out at
  `../hara-archive/`.
- benchmarks site — generated by `hara-lang/hara-benchmarks`
  (`python -m hara_bench build-site`)
- visual language package — `hara-lang/visual-language`
- package registry — `hara-lang/hara-packages`; signing identity —
  `hara-lang/hara-identity`

## Build and test

Java/Truffle runtime:

```shell
mvn -f core/java/pom.xml -Ptruffle package        # build + full test suite
mvn -f core/java/pom.xml -Ptruffle -Dtest=hara.truffle.HaraCoreLanguageConformanceTest test
./core/hara eval '(+ 19 23)'                      # CLI smoke test (shaded jar)
./scripts/runtime/run-lib-tests                 # library .hal test harness
bash scripts/runtime/build-truffle-native       # native-image build (core/target/hara-truffle)
core/target/hara-truffle eval '(+ 19 23)'         # native-image smoke test
./scripts/runtime/run-runtime-corpus-benchmark jvm interpreter-temurin core/target/runtime-corpus.csv
#   ^ shared benchmark corpus as CSV evidence (jvm mode uses the `java` on PATH;
#     bin mode appends a binary path, e.g. ... bin native-fallback out.csv 40 10 core/target/hara-truffle)
```

Rust runtime:

```shell
cargo test --manifest-path core/rust/Cargo.toml
cargo test --manifest-path core/rust/Cargo.toml --features bytecode-vm vm  # experimental VM (issues #195, #202)
cargo test --manifest-path core/rust/raw/Cargo.toml
bash core/rust/scripts/check-layout.sh
bash scripts/runtime/build-hara-wasm-raw             # raw wasm extension artifact
bash scripts/runtime/build-hara-wasm-web             # browser runtime (outputs to ../../website/hara-www/)
bash ../../extensions/hara-runtime/scripts/build-demo-synth-wasm
cd core/rust/web && npm ci && npm run test:hta       # browser loader tests
cd core/rust/web && npm run test:studio              # studio node tests (broker, hal, UI)
```

The `studio-hal` and `studio-broker` real-wasm integration tests need the
raw wasm artifact from `bash scripts/runtime/build-hara-wasm-raw` and
self-skip without it.

Apps:

```shell
cd ../../application/greenways-os/extension/hara-chrome && npm ci && npm run build && npm test
cd ../../application/greenways-os/extension/hara-chrome && npm run test:browser  # playwright (needs xvfb)
```

## Releasing the hara CLI

`scripts/runtime/install.sh` is the user-facing installer (`curl | sh`); it
downloads prebuilt binaries from GitHub releases. Test it with
`sh scripts/runtime/test-install.sh` (needs
`cargo build --release --manifest-path core/rust/Cargo.toml --bin hara` first).

To cut a release:

1. Bump `version` in `core/rust/Cargo.toml` and commit.
2. `git tag vX.Y.Z && git push origin vX.Y.Z` (tag version must match
   `core/rust/Cargo.toml`).
3. `.github/workflows/release.yml` builds Linux x86_64 + macOS arm64/x86_64
   binaries, publishes the GitHub release (prerelease while 0.x), and
   smoke-tests `scripts/runtime/install.sh` against it.

## Publishing the Rust crates

The `Publish Hara Rust crates` workflow is manually dispatched for an exact
tag or commit. Configure its `crates-io` environment with a
`CARGO_REGISTRY_TOKEN` secret. It publishes in dependency order:
`hara-abi`, `hara-wasm`, `hara-vm`, then `hara-compiler`. The workflow waits
for each package to appear in the crates.io index and uploads the resulting
`.crate` files and `SHA256SUMS` as the `hara-rust-crates` workflow artifact.

## Conventions

- Maven runs from the repo root via `-f core/java/pom.xml`; Surefire's working
  directory is the repo root, so tests use repo-relative paths. Conformance
  corpora formerly under `specs/` now live in the `hara-lang/hara-specs-registry` repo;
  check it out at `../hara-specs-registry/` next to this repo.
- Website content lives in `hara-lang/hara-www`; check it out at
  `../../website/hara-www/` next to this repo (CI workflows check it out at
  `../../website/hara-www/` inside this repo).
- The JVM runtime embeds `core/lib/src/**/*.hal` and
  `core/lib/src-lang/**/*.hal` as classpath resources via `core/java/pom.xml`.
  Repository Rust builds embed those canonical roots directly. Cargo
  publication materializes an ignored `core/rust/hal-src` snapshot because a
  `.crate` archive cannot include files above its crate root.
- `core/target/` is CI scratch/build artifacts; Maven output is
  `core/java/target/`. Both are gitignored.
- Website deployment is owned by `../../website/hara-www/.github/workflows/`.
- IDE state (`.idea/`, `.settings/`, `.classpath`, `.project`) is user-local
  and untracked.

## ChatGPT webapp connector workflow

When implementation is driven from the ChatGPT webapp, read and follow
`.github/CHATGPT_PROJECT_WORKFLOW.md`. All repository authoring must go through
the GitHub connector. Rust and Java changes must be committed on a connector
branch, opened as a draft pull request, and executed by
`.github/workflows/connector-code-execution.yml` plus every applicable normal
or focused workflow.

The connector execution workflow is read-only. It validates exact commits and
never materialises patches or writes product source. MCP, `hara-mcp`, and
`mcp.hara-lang.org` are explicitly out of scope for this workflow.

## Connector-first delivery

GitHub issues, pull requests, native relationships, checks, and repository
documents are authoritative. GitHub Projects are visual projections of that
state, not a separate source of truth.

Use the organisation workflow in
[hara-lang/.github](https://github.com/hara-lang/.github/blob/main/docs/connector-first-delivery.md).
Before implementing an issue, read its relationships and linked pull requests,
then follow this repository's local documentation and validation instructions.

Every executable issue must define Outcome, Scope, Acceptance criteria,
Validation, Relationships, Readiness, and Delivery. Keep durable decisions and
progress in the issue or pull request so that they remain visible through the
GitHub connector; do not rely on chat history as the only record.
