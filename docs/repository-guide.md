# Hara repository guide

This is the architecture, repository layout, and wider workspace guide that
previously lived in the top-level README. For the practical build, local
installation, and release workflow, start with the [main README](../README.md).


Hara is a programmable, runtime-neutral kernel for building, inspecting, and
changing live systems. Programs communicate with the kernel through HAL (Hara
Lisp), an EDN-compatible, host-neutral notation and data format. The supported native runtimes are Rust and Java/Truffle; their focused
conformance suites are kept in parity. They provide a compact core language, persistent data,
explicit mutable `array`/`object` markers, protocols, promises, bytes,
capability-gated I/O, and a JLine REPL.

HAL deliberately exposes two numeric runtime types: signed 64-bit longs and
IEEE-754 doubles. Use `long?` and `double?` for exact checks, or `number?` for
either. Internal interchange representations for big integers and decimal
values are not a public numeric tower and do not imply literal or arithmetic
support. HAL therefore does not expose `integer?` or `decimal?`.

```text
Hara source
    |
    v
Truffle parser / AST
    |
    +--> runtime-neutral core
    |
    +--> explicit libraries (bytes, promise, file, socket, string)
    |
    +--> host capability boundary
```

## Repository layout

This repository (`hara-lang/hara`) is the language runtime. It keeps the
actual code under `core/` and runtime tooling under `scripts/runtime/`. Sibling
repos in the workspace provide the website, editor/browser extensions, specs,
and archive:

- [`core/java/`](../core/java/) — the Java/Truffle runtime (Maven project, CLI, native-image).
- [`core/rust/`](../core/rust/) — the Rust/embedding runtime: native CLI, wasm builds, web
  loader, and the shared extension ABI. Concrete providers live in
  [`../../extensions/hara-runtime/`](../../../extensions/hara-runtime/).
- [`core/lib/`](../core/lib/) — hara-language source and workloads: the std foundation and
  Talo compiler port (`core/lib/src`, `core/lib/test`), demo projects
  ([`core/lib/examples/`](../core/lib/examples/)), and benchmark suites
  ([`../../website/hara-benchmarks/runtime/hara/`](../../../website/hara-benchmarks/runtime/hara/)).
- [`core/spec/`](../core/spec/) — parity specifications and substrate tests for core
  language targets.
- [`scripts/runtime/`](../scripts/runtime/) — repo-level build/benchmark scripts.
- `../../website/hara-www/` — the landing page for `www.hara-lang.org`
  ([`hara-lang/hara-www`](https://github.com/hara-lang/hara-www)), checked out next to this repo for builds.
- `../../extensions/` — editor and browser apps (`hara-emacs`, `hara-lsp`, `hara-vscode`)
  ([`hara-lang/hara-extensions`](https://github.com/hara-lang/hara-extensions)); the Chrome extension lives in
  [`../../application/greenways-os/extension/hara-chrome`](../../../application/greenways-os/extension/hara-chrome).
- `../hara-specs-registry/` — normative specs: prose (`.md`), machine-checked corpora,
  and spec-shaped data ([`hara-lang/hara-specs-registry`](https://github.com/hara-lang/hara-specs-registry)), checked out next to this repo.
- `../hara-archive/` — legacy material kept for history
  ([`hara-lang/hara-archive`](https://github.com/hara-lang/hara-archive)).
- [`../hara-specs-registry/00-unsorted/contrib/`](../../hara-specs-registry/00-unsorted/contrib/)
  — independently owned contribution formats and their conformance material.

The canonical registry checkout in the Greenways workspace is
`technology/hara-specs-registry`. Java and Rust registry consumers resolve it
in this order: explicit configuration (`-Dhara.specs.registry=...` for Java
or `HARA_SPECS_REGISTRY`), `HARA_WORKSPACE_ROOT/technology/hara-specs-registry`,
then ancestor discovery. The Maven or Cargo working directory therefore does
not affect registry lookup. Set these variables in CI or for a non-standard
checkout:

```shell
export HARA_WORKSPACE_ROOT=/absolute/path/to/workspace
export HARA_SPECS_REGISTRY="$HARA_WORKSPACE_ROOT/technology/hara-specs-registry"
```

- [`notes/`](../notes/) — working documents (not published): design notes and
  `notes/superpowers/` plans/specs.
- [`../../website/hara-docs/docs/books/`](../../../website/hara-docs/docs/books/) — published books.
- [`../hara-archive/retired/`](../../hara-archive/retired/) — retired registry and platform services.
- [`../../website/hara-benchmarks/astro/`](../../../website/hara-benchmarks/astro/) — runtime benchmark suites and generated site data.

## Start here

- [Hara website docs](../../../website/hara-www/docs/) — user guides, reference, and published documentation.
- [HAL meta-spec](../../hara-specs-registry/01-lang/000-metaspec/draft/README.md) — the self-describing contract for metaspec documents.
- [HAL language draft](../../hara-specs-registry/01-lang/001-language/draft/README.md) — the small EDN-oriented data and reader contract.
- [Planning archive](../../hara-specs-registry/99-archive/planning/README.md) — earlier runtime, extension, interop, and tooling designs.
- [Hara for Emacs](../../../extensions/hara-emacs/README.md) — project-aware evaluation, sessions, completion, docs,
  and a RESP-backed REPL.

## Quick start

Install the native Rust CLI on macOS or Linux from the Greenways ecosystem tap:

```shell
brew install greenways-ai/tap/hara
hara eval '(+ 19 23)'
```

Hara also maintains its dedicated binary tap:

```shell
brew install hara-lang/tap/hara
```

The separately packaged Truffle native image is available as
`brew install hara-lang/tap/hara-truffle`. Neither binary formula requires a
JVM at runtime. The Greenways formula builds the Rust CLI from the exact tagged
Hara source so the same formula works across macOS and Linux architectures.

To build the Truffle runtime from source, install JDK 21 and Maven:

```shell
mvn -f core/java/pom.xml -Ptruffle package
./core/hara eval '(+ 19 23)'
./core/hara
```

The `hara` command starts the JLine REPL in the shared `ROOT` session and exposes that same
session through RESP on `127.0.0.1:1311`. Use `--offline` to start without the listener,
`headless` for a listener without terminal UI, and `remote HOST:PORT` for a client connection. The CLI also supports `run <file>`, `stdin`, and `help`. For a native-image build, see the
[developer guide](../../../website/hara-www/docs/development/); native mode intentionally removes dynamic JVM services.

### Agent in-REPL prototype

For agent-assisted exploration, Hara includes a native HAL RESP client in
`tool.inrepl`. This deliberately is not an MCP server and does not start a
daemon: start a loopback server yourself, keep its endpoint in
`HARA_INREPL_ENDPOINT`, and let the agent attach to its dedicated `AGENT`
session.

```shell
hara --host 127.0.0.1 --port 1311 server
export HARA_INREPL_ENDPOINT=127.0.0.1:1311
```

Opt a project in with `:project/inrepl-capabilities #{:inrepl/loopback}`, then
evaluate through a one-shot local client that preserves the server-side session:

```shell
hara --project . --allow-net eval \
  '(do (require [tool.inrepl :as inrepl])
       (inrepl/eval-project "." "127.0.0.1:1311" "(+ 19 23)"))'
```

Use it for experiments, documentation, completion, and checking live state.
It accepts only `localhost` or `127.0.0.1`, never uses `ROOT`, and can reset
its own `AGENT` session. It does not replace the required fresh-process
validation of saved `.hal` files.

The Makefile also mirrors the main repository and CI workflows:

```shell
make -C core java-test java-specs java-conformance
make -C core rust-test wasm-test-raw rust-layout
make -C core lib-test

make -C core wasm-web
make -C core wasm-test-hta
make -C core wasm-test-studio

make -C core chrome-build chrome-test
make -C core docs-build
make -C core www-build
```

`java-test` excludes tests tagged `hara.spec.RegistryConformance`, so it is a
fast implementation signal and does not require the external registry. Use
`java-specs` for only registry-backed Java tests, or `java-conformance` for the
complete JVM suite. The underlying Maven profile is explicit:

```shell
mvn -f core/java/pom.xml -Ptruffle test
mvn -f core/java/pom.xml -Ptruffle -Pconformance \
  -Dhara.specs.registry="$HARA_SPECS_REGISTRY" test
```

Run `make web-install` or `make chrome-install` before the corresponding Node
workflows on a fresh checkout. `make check-all` runs the core Java, Rust, raw
WASM, portable library, HTA, and Studio checks. Runtime performance entry
points are available as `runtime-benchmark`, `truffle-benchmark`,
and `parity-benchmark`; each accepts additional arguments
through `ARGS`.

Portable bytecode uses HBC0 artifacts and deterministic HBX0 bundles across
the Rust, Truffle JVM, and Truffle native-image runtimes. Inspect or execute an
artifact with `hara bytecode disassemble FILE` and `hara bytecode run FILE`.
`hara bytecode conformance core/rust/assets/bytecode-conformance.hcc` executes
the complete Rust-produced opcode corpus and checks every result; the
native-image CI gate runs this same command before accepting the image.

Per-component builds:

```shell
cargo test --manifest-path core/rust/Cargo.toml                 # Rust runtime
cd ../../application/greenways-os/extension/hara-chrome && npm ci && npm run build  # Chrome extension
```

## Package workspaces

The native CLI is authoritative for deterministic HARP package operations:

```shell
hara package check packages/core
hara package build packages/core
hara package install packages/core
hara package publish --tap hara --dry-run packages/core
```

A multi-package project can keep its Foundation-compatible namespace catalog in
`config/packages.edn` and build one semantic package at a time:

```shell
hara package profile core/config/packages.edn
hara package build core --package code.test --profile core/config/packages.edn
```

The archive contains only the selected namespaces plus declared bundles. Its
optional `bytecode/package.hbx` is verified by browser loaders, with the HAL
resources retained as the portable source fallback.

Repositories containing several packages can use `code.deploy`. It exposes the
same CLI operations as native `std.work` values, accepts a data catalog with
`:path`, `:depends`, and per-package `:options`, and processes packages in
stable dependency order. Package coordinates become stable work item
identities, so durable runtimes can checkpoint each package boundary. The
process capability is supplied through runtime context and argv is never
interpreted by a shell.

```clojure
(require '[code.deploy :as deploy])
(require '[std.work :as work])

(def runtime (work/local-runtime))

(def packages
  {'example/core {:path "packages/core"}
   'example/addon {:path "packages/addon"
                   :depends ['example/core]}})

(work/run runtime
          deploy/check
          {:selector :all
           :packages packages})

(work/run runtime
          deploy/package
          {:selector :all
           :packages packages})

(work/run runtime
          deploy/publish
          {:selector :all
           :packages packages
           :tap :hara
           :dry-run true})
```

Tests and remote hosts can replace process authority without changing the work
definition:

```clojure
(work/run runtime
          deploy/package
          {:selector :all
           :packages packages}
          {:context {:process/run runner}})
```

## Cloud development environment

The repository includes a Dev Container definition for GitHub Codespaces and
VS Code's **Dev Containers: Reopen in Container** command. On first creation it
installs JDK 21 with Maven, Node.js 22, stable Rust with the browser and WASI
targets, Python documentation tooling, and the web test dependencies. No host
toolchain setup is required.

The setup is reproducible from the repository root:

```shell
bash .devcontainer/post-create.sh
```

The container forwards the Hara RESP port (`1311`) and the MkDocs preview port
(`8000`). See the commands printed by the setup script for the main checks.

## Current runtime boundary

The language does not expose ambient JVM host interop. JVM reflection, compilation, mutable
classpath access, files, and sockets are explicit capabilities or provider services. This keeps the
core portable to future runtimes such as WASM hosts.

The old interpreter/Foundation/TCP architecture is retained as
[`../hara-archive/legacy-docs/README.legacy.md`](../../hara-archive/legacy-docs/README.legacy.md) for historical
reference only; it is not the current language guide.

## Cloning this workspace

This repository no longer uses Git submodules. Some build and test targets expect
sibling repositories next to this one:

```shell
git clone https://github.com/hara-lang/hara.git
git clone --depth 1 https://github.com/hara-lang/hara-specs-registry.git hara-specs-registry
git clone https://github.com/hara-lang/hara-www.git
```

CI workflows check out `hara-lang/hara-www` automatically when they need it.

## Status

Hara is an active experimental runtime. The core-language slice and focused conformance suites are the source
of truth. Provider discovery and WASM execution are documented contracts with
implementation work still in progress.

## License

Hara-owned source code is available under the [Apache License 2.0](../LICENSE).
Some directories contain separately licensed or provenance-sensitive material; see
[the license inventory](../LICENSES/README.md). Run `bash scripts/runtime/check-licenses` to
validate the repository's license metadata and documented exceptions.
