# Hara development environment

The same bootstrap is used by the Ubuntu 24.04 devcontainer, GitHub Codespaces,
and Codex cloud. Codex uses its universal image rather than building this
Dockerfile, so setup and cached-environment maintenance deliberately share one
idempotent command.

## Setup and maintenance

```sh
bash .devcontainer/post-create.sh
```

Use these Codex environment values:

- **Setup script:** `bash .devcontainer/post-create.sh`
- **Maintenance script:** `bash .devcontainer/post-create.sh`
- **Agent internet access:** not required for the checks below after setup
- **Docker integration:** not required by Hara's documented checks

The script installs `hara` and `hara-test` in `$HOME/.local/bin`, persists that
path in `.bashrc`, prepares both locked Rust graphs and Maven dependencies,
installs HTA/browser and website packages, and verifies exact clean auxiliary
checkouts (including the website's visual-language package). A dirty or
mismatched cached checkout fails without being reset.

## Smoke test

```sh
hara --version
hara-test --help
hara --project core/lib check
```

## Representative offline checks

```sh
cargo +stable test --locked --manifest-path core/rust/Cargo.toml
cargo +stable test --locked --manifest-path core/rust/raw/Cargo.toml
mvn -o -B -f core/java/pom.xml -Ptruffle test
npm --prefix core/rust/web run test:hta
npm run build --prefix website/hara-www
```

Port `1311` is forwarded for RESP sessions and port `4321` for the Astro docs
preview. Setup does not start either service.
