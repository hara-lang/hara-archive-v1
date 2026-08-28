# Hara

Hara is a small Lisp runtime and toolchain for building, inspecting, and
changing live systems. This guide is for someone working from a source checkout:
build a runtime, install it locally, use it from Emacs, and understand how an
official release is published.

If you are looking for the architecture, repository map, package workspaces,
or the relationship between the Hara repositories, see the
[repository guide](docs/repository-guide.md).

## Choose a runtime

You do not need every Hara runtime for ordinary development.

| Runtime | Command after installation | Use it for |
| --- | --- | --- |
| Rust | `hara` | The normal native CLI and recommended local default |
| Rust lite | `hara-lite` | A smaller recovery/development CLI with evaluator and REPL commands |
| Truffle JVM | `hara-truffle` | The Java/Truffle runtime, JVM tests, and JVM-specific debugging |
| Truffle native image | `hara-truffle-native` | A standalone GraalVM-built Truffle executable |
| Browser Wasm | no terminal command | Browser, Studio, and embedding work |

Start with the Rust runtime. Build another variant only when the work you are
doing needs it.

## 1. Prepare the checkout


- [Hara website docs](../../website/hara-www/docs/) — user guides, reference, and published documentation.
- [HAL meta-spec](../hara-specs-registry/01-lang/000-metaspec/draft/README.md) — the self-describing contract for metaspec documents.
- [HAL language draft](../hara-specs-registry/01-lang/001-language/draft/README.md) — the small EDN-oriented data and reader contract.
- [Planning archive](../hara-specs-registry/99-archive/planning/README.md) — earlier runtime, extension, interop, and tooling designs.
- [Hara for Emacs](../../extensions/hara-emacs/README.md) — project-aware evaluation, sessions, completion, docs,
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
[developer guide](../../website/hara-www/docs/development/); native mode intentionally removes dynamic JVM services.

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

```hara
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

```hara
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
[`../hara-archive/legacy-docs/README.legacy.md`](../hara-archive/legacy-docs/README.legacy.md) for historical
reference only; it is not the current language guide.

## Cloning this workspace

This repository no longer uses Git submodules. Some build and test targets expect
sibling repositories next to this one:
=======
Hara itself lives in `hara-lang/hara`. The Truffle build and several
conformance suites also expect `hara-specs-registry` beside it.
>>>>>>> d48d55718af8942ef7db034dbea17ac72607fc7a

```shell
git clone https://github.com/hara-lang/hara.git
git clone https://github.com/hara-lang/hara-specs-registry.git
cd hara
```

In the Greenways workspace used by this repository, those paths are already:

```text
technology/hara
technology/hara-specs-registry
extensions/hara-emacs
```

### Toolchains

Install only the toolchains needed by the variants you intend to build:

| Variant | Required tools |
| --- | --- |
| Rust and Rust lite | stable Rust and Cargo |
| Truffle JVM | JDK 21 and Maven |
| Truffle native image | GraalVM 25 with `native-image`, plus Maven |
| Browser Wasm | Rust target `wasm32-unknown-unknown`, `wasm-bindgen`, Node.js 22, and npm |

Confirm the basics before starting:

```shell
cargo --version
java -version
mvn --version
```

For Wasm development, add the Rust target and install the `wasm-bindgen` CLI
whose version matches `core/rust/Cargo.lock`.

## 2. The quickest local build and install

From the repository root:

```shell
make install
```

This builds the optimised Rust CLI and installs:

```text
~/.local/bin/hara
~/.local/share/hara-lite/project.edn
~/.local/share/hara-lite/lib/
```

Make sure `~/.local/bin` is on `PATH`, then smoke-test the installed binary:

```shell
hara --version
hara eval '(+ 19 23)'
```

The expected evaluation result is `42`.

To install under a different prefix:

```shell
make install PREFIX=/opt/hara
```

To test the install layout without writing into the real prefix:

```shell
make check-install
```

To remove files installed by these Make targets:

```shell
make uninstall
```

## 3. Build and install every local variant

### Rust CLI

Build without installing:

```shell
make build-rust
core/rust/target/release/hara eval '(+ 19 23)'
```

Build and install as `hara`:

```shell
make install-rust
```

The official release build enables the `native-jit` feature. To reproduce that
exact binary locally:

```shell
cargo build --release \
  --manifest-path core/rust/Cargo.toml \
  --features native-jit \
  --bin hara
```

### Rust lite CLI

Rust lite deliberately exposes only evaluator and REPL operations. It remains
usable when higher-level Hara tooling is being repaired.

```shell
make build-rust-lite
core/rust/target/release/hara-lite --project core eval '(+ 19 23)'
make install-rust-lite
hara-lite eval '(+ 19 23)'
```

The lite build uses only the direct-native runtime feature set; it does not
enable the browser and full-runtime default features.

The installed binary finds its portable Hara project under
`~/.local/share/hara-lite`. Set `HARA_LITE_PROJECT` only when you intentionally
want to use another copy.

`hara-lite` reads that project's `project.edn` and indexes the declared
namespaces under its native `:project/source-paths`. The `.hal` files remain
ordinary files in the installed `lib/` tree; their source bodies are read and
evaluated when a namespace is required. Lite does not need a prebuilt
whole-library bytecode bundle. With `--project PATH`, the selected project and
its verified installed Hara dependencies are added to the same source catalog,
so normal project `require` forms work from the source tree.

### Truffle on the JVM

Build the executable JAR:

```shell
make build-truffle
java -jar core/java/target/hara-truffle.jar eval '(+ 19 23)'
```

Build and install the JAR plus a launcher:

```shell
make install-truffle
hara-truffle eval '(+ 19 23)'
```

The installed layout is:

```text
~/.local/bin/hara-truffle
~/.local/share/hara/hara-truffle.jar
```

Use `HARA_JAVA` to select a Java executable and `HARA_RUNTIME_JAR` to point the
launcher at another JAR.

### Truffle native image

This build needs GraalVM's `native-image` command, not only a normal JDK.

```shell
make build-truffle-native
core/target/hara-truffle eval '(+ 19 23)'
make install-truffle-native
hara-truffle-native eval '(+ 19 23)'
```

The local Make target uses the name `hara-truffle-native` so it can coexist
with the JVM launcher. Official release archives use the shorter binary name
`hara-truffle` because they contain the native image, not the JVM launcher.

### Raw and browser Wasm

Build all raw Wasm variants:

```shell
scripts/runtime/build-hara-wasm-raw all
```

The variants are:

- `hara-wasm-core` — the smallest raw evaluator;
- `hara-wasm-vm` — the default bytecode-VM build;
- `hara-wasm-trace` — development tracing enabled.

Install browser workspace dependencies once, then build the browser packages:

```shell
npm ci --prefix core/rust/web
scripts/runtime/build-hara-browser all
```

Browser packages are written beneath
`core/rust/web/packages/browser/dist/`. They are build artifacts for embedding
and Studio development; they are not terminal executables and the repository
currently has no automatic npm publication workflow for `@hara-lang/browser`.

## 4. Test and validate a local build

Use the smallest slice that covers a change while iterating, then run the
appropriate aggregate before treating the change as release-ready. List all
available targets with `make -C core help`.

### What conformance means

Ordinary tests check one implementation: a Rust function, a JVM provider, a
CLI command, or a browser host. Conformance tests check an observable Hara
contract that more than one runtime or host is expected to implement. A
conformance failure means that an implementation disagrees with that shared
contract; it does not necessarily mean that the individual class or function
named in the failure is independently broken.

Some contracts and machine-readable corpora are owned by the sibling
[`hara-specs-registry`](../hara-specs-registry/). Other parity tests remain in
this repository while their contracts are being developed. Aggregate commands
include both kinds. The focused slices below distinguish local regressions,
registry-contract mismatches, and host-integration failures.

### Aggregate checks

| Command | Coverage |
| --- | --- |
| `make -C core java-test` | Fast JVM/Truffle tests without registry conformance |
| `make -C core java-specs` | Registry-backed JVM tests only |
| `make -C core java-conformance` | Complete JVM/Truffle suite, including registry conformance |
| `make -C core rust-test` | All ordinary native Rust slices and workspace crates |
| `make -C core wasm-test` | Raw-Wasm, Node, browser SDK, and Playwright slices |
| `make -C core lib-test` | Portable `.hal` libraries through the native runner |
| `make -C core check-all` | Repository gate: layout, Java, Rust, raw-Wasm, libraries, HTA, and Studio |

`java-test` is the fast implementation signal and excludes tests tagged
`hara.spec.RegistryConformance`. Use `java-specs` to run only registry-backed
tests, or `java-conformance` for the complete JVM suite. Registry lookup is
independent of the Maven working directory; set `HARA_SPECS_REGISTRY` or
`HARA_WORKSPACE_ROOT` when the checkout is not in the standard workspace
layout. Maven also accepts `-Dhara.specs.registry=/absolute/path`.

### Native Rust slices

Cargo's workspace default selects only the main `hara-wasm` member. The
`rust-test` aggregate is broader: it explicitly includes `hara-abi`,
`hara-hta`, `hara-vm`, and `hara-compiler` so those crate tests are not silently
omitted.

| Target | Coverage |
| --- | --- |
| `rust-test-library` | Unit tests in the main `hara-wasm` library |
| `rust-test-cli` | Unit tests embedded in every native CLI binary |
| `rust-test-integration` | Every `core/rust/tests/*.rs` integration target |
| `rust-test-examples` | Example compilation and test harnesses |
| `rust-test-doc` | Rust documentation tests |
| `rust-test-main` | The five main-crate slices above |
| `rust-test-abi` | Dependency-free `hara-abi` crate |
| `rust-test-hta` | Canonical `hara-hta` codec crate |
| `rust-test-vm` | VM-only `hara-vm` crate |
| `rust-test-compiler` | `hara-compiler` facade crate |
| `rust-test-crates` | All four supporting workspace crates |
| `rust-test-ignored` | Opt-in tests requiring external artifacts; excluded from `rust-test` |
| `rust-test-conformance-groups` | Issue #1047 runtime-owner groups for targeting Rust failures |

For example:

```shell
make -C core rust-test-library
make -C core rust-test-integration
make -C core rust-test-crates
make -C core rust-test-conformance-groups ARGS="vm whole-wasm"
```

### Wasm slices

Wasm testing crosses several host boundaries, so raw Rust tests, Node tests,
the publishable browser SDK, and real-browser tests are separate slices.

| Target | Coverage |
| --- | --- |
| `wasm-test-raw-unit` | Unit tests compiled inside `hara-wasm-raw` |
| `wasm-test-raw-integration` | Raw-crate integration targets |
| `wasm-test-raw` | Both raw-crate slices |
| `wasm-test-hta` | HTA codec, value transport, and sandbox tests in Node |
| `wasm-test-exceptions` | Raw-Wasm exception conformance in Node |
| `wasm-test-runtime` | Portable code through the wasm-bindgen runtime in Node |
| `wasm-test-studio` | Studio host, broker, filesystem, endpoint, and real-Wasm Node tests |
| `wasm-test-node` | All four Node-hosted slices |
| `wasm-test-browser-sdk` | Publishable browser SDK tests |
| `wasm-test-browser` | Full Playwright browser suite |

Install web dependencies once on a fresh checkout before running Node or
browser slices:

```shell
make -C core web-install
make -C core wasm-test-raw
make -C core wasm-test-node
make -C core wasm-test-browser
```

Every top-level Node test under `core/rust/web` is assigned to an npm test
script, while Playwright's configuration owns the browser `*.spec.js`
inventory.

### Bytecode conformance and benchmarks

Portable bytecode uses HBC0 artifacts and deterministic HBX0 bundles across
the Rust, Truffle JVM, and Truffle native-image runtimes. Inspect or execute an
artifact with `hara bytecode disassemble FILE` and `hara bytecode run FILE`.
The following command executes the complete Rust-produced opcode corpus and
checks every result; the native-image CI gate runs the same corpus:

```shell
hara bytecode conformance core/rust/assets/bytecode-conformance.hcc
```

Performance entry points are `runtime-benchmark`, `vm-bb-benchmark`,
`truffle-benchmark`, and `parity-benchmark`. They accept extra arguments through
`ARGS` and measure performance; they are not substitutes for correctness tests.

For any saved `.hal` change, follow the repository's fresh-process workflow:
evaluate the complete candidate, run its focused test, write the edit, run the
written file in a new Hara process, and repeat the focused test.

## 5. Use the local build from Emacs

The workspace's `hara-emacs` checkout automatically prefers runtime artifacts
built in this repository. A simple configuration is:

```elisp
(use-package hara-mode
  :load-path "/path/to/hara-extensions/hara-emacs"
  :mode ("\\.hal\\'" . hara-mode))
```

Build the runtime you want, set `HARA_BACKEND` before Emacs starts, then open a
`.hal` file:

```shell
make build-rust-lite
export HARA_BACKEND=rust-lite
emacs
```

Supported backend names in the package launcher are `rust`, `rust-lite`,
`truffle`, and `native`. In Emacs, `M-x hara-jack-in` starts or reuses the
project server and `M-x hara-repl` opens that project's REPL.

See the
[hara-emacs guide](https://github.com/hara-lang/hara-extensions/tree/main/hara-emacs)
for the daily evaluation, testing, source/test navigation, and `code.manage`
commands.

## 6. Understand official publication

There is an important distinction:

- your local computer prepares, validates, reviews, and authorises a release;
- GitHub Actions builds platform artifacts and publishes them from an immutable
  commit or tag.

Do not upload hand-built local binaries as official release artifacts. The CI
matrix is the authority for platform builds, checksums, smoke installs, and
downstream package updates.

### What the main release publishes

A normal `vX.Y.Z` release runs `.github/workflows/release.yml` and produces:

- Rust CLI archives for Linux x86-64, macOS x86-64, and macOS arm64;
- Truffle native-image archives for Linux x86-64, macOS x86-64, and macOS arm64;
- the versioned Studio runtime archive and checksum;
- a combined `SHA256SUMS` file;
- a GitHub prerelease while the version is `0.x`;
- binary Homebrew formulas in `hara-lang/homebrew-tap`;
- the source-built formula used by the Greenways tap when its token is present.

The workflow then installs the public artifacts on clean Linux and macOS
runners and verifies `(+ 19 23)` returns `42`.

### Prepare a normal reviewed release

1. Choose `X.Y.Z` and update every versioned surface. The authoritative check
   lists those surfaces and fails if they disagree:

   ```shell
   node scripts/runtime/check-release-version.mjs X.Y.Z
   ```

2. Run the relevant focused suites and `make -C core check-all`.

3. Merge the version change to `main`. Record the exact merged commit SHA.

4. Add one reviewed manifest at `.github/releases/vX.Y.Z.json`:

   ```json
   {
     "schema": "hara-release-cut/0-alpha",
     "version": "X.Y.Z",
     "tag": "vX.Y.Z",
     "commit": "FULL_40_CHARACTER_COMMIT_SHA",
     "workflow": "release.yml"
   }
   ```

5. Open and merge the manifest pull request. The
   `cut-reviewed-release.yml` workflow verifies that the commit is on `main`,
   creates the immutable annotated tag, starts `release.yml`, waits for it, and
   verifies the complete public asset set.

6. Read back the GitHub release, the release workflow result, and the Homebrew
   updates. Homebrew publication is intentionally `continue-on-error`, so a
   green runtime release does not by itself prove both taps were updated.

This reviewed-manifest route is the preferred full-release path. Although
`release.yml` can react to a pushed `v*` tag, manually cutting the full release
tag bypasses the repository's review gate.

### Publish the Rust lite prerelease

Rust lite has a separate tag and workflow. Its tag describes a lite release of
the current Cargo version, for example `v0.1.6-lite.1`:

```shell
git tag --annotate v0.1.6-lite.1 --message "hara-lite v0.1.6-lite.1"
git push origin v0.1.6-lite.1
```

`.github/workflows/release-lite.yml` builds all three supported Rust platform
archives and creates a GitHub prerelease. Confirm that `SHA256SUMS` and every
expected `hara-rust-lite-*` archive are present before announcing it.

### Publish the Rust crates

Crates.io publication is separate from the CLI release because crates are
immutable. After the versioned commit or tag is reviewed, dispatch:

```shell
gh workflow run publish-rust-crates.yml --ref main --field ref=vX.Y.Z
```

The workflow materialises the HAL source archive and publishes, in dependency
order:

1. `hara-abi`
2. `hara-hta`
3. `hara-wasm`
4. `hara-vm`
5. `hara-compiler`

It waits for each dependency to appear in the crates.io index before publishing
the next one, then retains the exact `.crate` archives and checksums as a
workflow artifact.

### Publish the Maven snapshot

The Java artifact is currently a snapshot, not a Maven Central release. Its
version comes from `core/java/pom.xml` and must end in `-SNAPSHOT`.

```shell
gh workflow run publish-maven-snapshot.yml --ref main
```

The workflow builds the Wasm fixtures, runs the Maven tests, and deploys
`org.hara-lang:hara.lang` to the configured Central Portal snapshot repository.

### Publish `@hara-lang/hta`

The HTA npm package is released independently. Update and validate
`core/rust/web/packages/hta/package.json`, merge that commit, then push an
annotated tag matching the package version exactly:

```shell
git tag --annotate hta-vX.Y.Z --message "@hara-lang/hta X.Y.Z"
git push origin hta-vX.Y.Z
```

The `publish-hta.yml` workflow runs the source and packed-consumer tests. It
publishes only when that exact npm version does not already exist; otherwise it
requires the registry integrity to match the locally packed artifact.

## Publication map

| Deliverable | Local build | Official publication trigger |
| --- | --- | --- |
| Rust CLI | `make build-rust` | reviewed `vX.Y.Z` release manifest |
| Truffle native image | `make build-truffle-native` | same reviewed `vX.Y.Z` release |
| Studio runtime | website assembly script in release CI | same reviewed `vX.Y.Z` release |
| Rust lite CLI | `make build-rust-lite` | pushed `vX.Y.Z-lite.N` tag |
| Rust crates | `cargo package` | manual `publish-rust-crates.yml` dispatch |
| Truffle JVM snapshot | `make build-truffle` | manual `publish-maven-snapshot.yml` dispatch |
| `@hara-lang/hta` | npm workspace build/test | pushed `hta-vX.Y.Z` tag |
| Browser Wasm package | `scripts/runtime/build-hara-browser all` | no npm publication workflow currently |

## More documentation

- [Repository structure and architecture](docs/repository-guide.md)
- [Getting started](GETTING_STARTED.md)
- [Contributing](CONTRIBUTING.md)
- [Architecture](ARCHITECTURE.md)
- [Hara for Emacs](https://github.com/hara-lang/hara-extensions/tree/main/hara-emacs)
- [License inventory](LICENSES/README.md)

Hara-owned source is licensed under the [Apache License 2.0](LICENSE).
