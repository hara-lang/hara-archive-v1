# GitHub rich HTA provider

This directory is the Wasm-only `:require` route for
`hara/filesystem-github`. `project.edn` is the source package declaration;
the package builder generates the archive `package.edn` extension map from it.
`provider.hal` is embedded into the raw Hara runtime and compiled as an actual
HTA Wasm module; no JavaScript provider module is part of the package.

The rich build has one explicit source contract: the embedded source declares
`hara.hta.provider/dispatch`. The raw runtime dispatches the manifest export
name and decoded arguments to that function, so maps, bytes, structured
errors, and Promise-returning host calls cross the normal HTA value boundary.

Build the artifact from the repository root:

```text
HARA_HTA_PROVIDER_SOURCE="$PWD/providers/github/hta/provider.hal" \
cargo build --manifest-path core/rust/raw/Cargo.toml \
  --target wasm32-unknown-unknown --profile browser-release \
  --features rich-hta
```

The resulting `hara_wasm_raw.wasm` is copied to `provider/provider.wasm` for release
packaging. The
trusted host implements `filesystem.github/describe`, `open`, `request`,
`cancel`, and `close`; credentials, GitHub HTTP, Git object creation, and
expected-head updates remain outside the Wasm package. Construct it with a
fixed `repository`, `ref`, and host-owned token provider. `open` may narrow the
root and choose read-only or optimistic `commit` mode, but cannot broaden the
repository authority.

The host adapter mirrors the JVM contract: commit mounts are immutable,
writable mounts require `heads/*`, updates compare the observed head and send
`force: false`, Git symlinks and gitlinks are not followed, page tokens are
revision-bound, and close/cancellation abort pending HTTP work. Its focused
fixture is run with:

```text
npm run test:github-host
```

For this source/toolchain build the artifact is 3,318,884 bytes with
SHA-256
`3be8f634190c805f08df8c42c077a252325dc14295c683ddb436a5f1757ab22e`.
Release packaging verifies `provider.sha256` and then builds the archive
without invoking a compiler during consumer installation:

```text
mkdir -p providers/github/hta/provider
cp core/rust/target/wasm32-unknown-unknown/browser-release/hara_wasm_raw.wasm \
  providers/github/hta/provider/provider.wasm
(cd providers/github/hta && sha256sum -c provider.sha256)
./core/hara package check providers/github/hta
./core/hara package build providers/github/hta \
  --output /tmp/hara-filesystem-github-0.1.0.harp
./core/hara package inspect /tmp/hara-filesystem-github-0.1.0.harp
```
