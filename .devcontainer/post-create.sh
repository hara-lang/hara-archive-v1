#!/usr/bin/env bash
set -Eeuo pipefail

repo_root="$(git rev-parse --show-toplevel 2>/dev/null || true)"
[[ -n "$repo_root" ]] || { echo "error: run from a Hara checkout" >&2; exit 1; }

SPECS_REPOSITORY="https://github.com/hara-lang/hara-specs-registry.git"
SPECS_REVISION="a40b7da53ed8e4ef241e36a9fd2802b3bc34ea8a"
WWW_REPOSITORY="https://github.com/hara-lang/hara-www.git"
WWW_REVISION="88179d06aeb0a233b21b63a5ddfd0625aa2352fa"

fail() {
  echo "error: $*" >&2
  exit 1
}

need() {
  command -v "$1" >/dev/null 2>&1 || fail "missing required command: $1"
}

persist_local_bin() {
  local line='export PATH="$HOME/.local/bin:$PATH"'
  mkdir -p "$HOME/.local/bin"
  touch "$HOME/.bashrc"
  grep -Fqx "$line" "$HOME/.bashrc" || printf '\n%s\n' "$line" >> "$HOME/.bashrc"
  export PATH="$HOME/.local/bin:$PATH"
}

select_node() {
  local major="$1"
  export NVM_DIR="${NVM_DIR:-$HOME/.nvm}"
  if [[ -s "$NVM_DIR/nvm.sh" ]]; then
    # shellcheck disable=SC1090
    source "$NVM_DIR/nvm.sh"
    nvm install "$major"
    nvm use "$major"
  fi
  need node
  need npm
  [[ "$(node -p 'process.versions.node.split(".")[0]')" == "$major" ]] \
    || fail "Node $major is required; found $(node --version)"
}

select_java() {
  local major="$1"
  local candidate="/usr/lib/jvm/java-${major}-openjdk-amd64"
  if [[ -x "$candidate/bin/java" ]]; then
    export JAVA_HOME="$candidate"
    export PATH="$JAVA_HOME/bin:$PATH"
  fi
  need java
  need mvn
  java -version 2>&1 | head -n 1 | grep -Eq "(version \"${major}[.]|openjdk ${major}[.])" \
    || fail "JDK $major is required"
}

ensure_rust() {
  need rustup
  rustup toolchain install stable --profile minimal
  rustup target add --toolchain stable wasm32-unknown-unknown wasm32-wasip1
  need cargo
}

ensure_checkout() {
  local repository="$1"
  local revision="$2"
  local checkout="$3"

  if [[ -e "$checkout" ]]; then
    [[ -d "$checkout/.git" ]] || fail "$checkout exists but is not a Git checkout"
    [[ -z "$(git -C "$checkout" status --porcelain --untracked-files=all)" ]] \
      || fail "dependency checkout is dirty: $checkout"
    local actual
    actual="$(git -C "$checkout" rev-parse HEAD)"
    [[ "$actual" == "$revision" ]] \
      || fail "dependency revision mismatch at $checkout (expected $revision, found $actual); refusing to reset it"
    return
  fi

  mkdir -p "$(dirname "$checkout")"
  local temporary="${checkout}.tmp.$$"
  rm -rf "$temporary"
  git clone --filter=blob:none --no-checkout "$repository" "$temporary"
  git -C "$temporary" fetch --depth 1 origin "$revision"
  git -C "$temporary" checkout --detach "$revision"
  mv "$temporary" "$checkout"
}

print_version() {
  local label="$1"
  shift
  printf '%-14s ' "$label:"
  "$@" --version 2>&1 | head -n 1 || true
}

persist_local_bin
select_node 22
select_java 21
ensure_rust
need git

cd "$repo_root"
git submodule update --init --recursive

ensure_checkout "$SPECS_REPOSITORY" "$SPECS_REVISION" "$repo_root/hara-specs-registry"
ensure_checkout "$WWW_REPOSITORY" "$WWW_REVISION" "$repo_root/website/hara-www"
git -C "$repo_root/website/hara-www" submodule update --init --recursive

cargo +stable fetch --locked --manifest-path "$repo_root/core/rust/Cargo.toml"
cargo +stable fetch --locked --manifest-path "$repo_root/core/rust/raw/Cargo.toml"
cargo +stable build --locked --release \
  --manifest-path "$repo_root/core/rust/Cargo.toml" \
  --bin hara --bin hara-test
install -m 0755 "$repo_root/core/rust/target/release/hara" "$HOME/.local/bin/hara"
install -m 0755 "$repo_root/core/rust/target/release/hara-test" "$HOME/.local/bin/hara-test"

npm ci --prefix "$repo_root/core/rust/web"
(
  cd "$repo_root/core/rust/web"
  npx playwright install chromium
)
npm ci --prefix "$repo_root/website/hara-www"

mvn -B -f "$repo_root/core/java/pom.xml" -Ptruffle -DskipTests dependency:go-offline

[[ -z "$(git -C "$repo_root" status --porcelain --untracked-files=all)" ]] \
  || fail "setup changed the Hara working tree"

printf '\nHara development environment ready.\n'
print_version "Java" java
print_version "Maven" mvn
print_version "Node" node
print_version "npm" npm
print_version "Rust" rustc +stable
print_version "Cargo" cargo +stable
print_version "Hara" hara
print_version "hara-test" hara-test
printf 'Specs registry: %s\n' "$(git -C "$repo_root/hara-specs-registry" rev-parse HEAD)"
printf 'Hara website:  %s\n' "$(git -C "$repo_root/website/hara-www" rev-parse HEAD)"
cat <<'CHECKS'

Available checks (dependencies are prepared for offline execution):
  hara --project core/lib check
  cargo +stable test --locked --manifest-path core/rust/Cargo.toml --workspace
  cargo +stable test --locked --manifest-path core/rust/raw/Cargo.toml --workspace
  mvn -o -B -f core/java/pom.xml -Ptruffle test
  npm --prefix core/rust/web run test:hta
  npm run build --prefix website/hara-www
CHECKS
