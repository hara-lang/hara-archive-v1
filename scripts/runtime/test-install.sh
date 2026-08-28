#!/bin/sh
# Functional tests for the canonical Hara installer.
#
# Builds a fake "release" (tarball + SHA256SUMS) from the locally compiled
# release binary, serves it over file://, and exercises the installer.
#
# Prereq: cargo build --release --manifest-path core/rust/Cargo.toml --features native-jit --bin hara
#         cargo build --release --manifest-path core/rust/Cargo.toml --no-default-features --features direct-native --bin hara-lite
# Usage:  sh scripts/runtime/test-install.sh
set -eu

SCRIPT_DIR=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
HARA_ROOT=$(CDPATH='' cd -- "$SCRIPT_DIR/../.." && pwd)
INSTALL_SH="$SCRIPT_DIR/install.sh"
WORK="$HARA_ROOT/.tmp/install-test"
case "$(uname -s)/$(uname -m)" in
  Linux/x86_64) TRIPLE="x86_64-unknown-linux-gnu" ;;
  Darwin/arm64) TRIPLE="aarch64-apple-darwin" ;;
  Darwin/x86_64) TRIPLE="x86_64-apple-darwin" ;;
  *) echo "unsupported test platform: $(uname -s)/$(uname -m)" >&2; exit 1 ;;
esac
VERSION="v9.9.9"
TARBALL="hara-rust-$VERSION-$TRIPLE.tar.gz"
LITE_TARBALL="hara-rust-lite-$VERSION-$TRIPLE.tar.gz"
TRUFFLE_TARBALL="hara-truffle-$VERSION-$TRIPLE.tar.gz"
BINARY="$HARA_ROOT/core/rust/target/release/hara"
LITE_BINARY="$HARA_ROOT/core/rust/target/release/hara-lite"

pass=0
fail=0

ok() { pass=$((pass + 1)); printf 'ok %s - %s\n' "$pass" "$1"; }
not_ok() { fail=$((fail + 1)); printf 'not ok - %s\n' "$1"; }
check() { # check <description> <command...>
  desc=$1; shift
  if "$@" >/dev/null 2>&1; then ok "$desc"; else not_ok "$desc"; fi
}

if [ ! -x "$BINARY" ]; then
  echo "missing release binary at $BINARY" >&2
  echo "run: cargo build --release --manifest-path $HARA_ROOT/core/rust/Cargo.toml --bin hara" >&2
  exit 1
fi
if [ ! -x "$LITE_BINARY" ]; then
  echo "missing release binary at $LITE_BINARY" >&2
  echo "run: cargo build --release --manifest-path $HARA_ROOT/core/rust/Cargo.toml --no-default-features --features direct-native --bin hara-lite" >&2
  exit 1
fi

# --- fake release fixture ---------------------------------------------------
rm -rf "$WORK"
mkdir -p "$WORK/release"
mkdir -p "$WORK/full/share/hara-lite"
cp "$BINARY" "$WORK/full/hara"
cp "$HARA_ROOT/core/project.edn" "$WORK/full/share/hara-lite/project.edn"
cp -R "$HARA_ROOT/core/lib" "$WORK/full/share/hara-lite/lib"
tar -czf "$WORK/release/$TARBALL" -C "$WORK/full" hara share
mkdir -p "$WORK/lite/share/hara-lite"
cp "$LITE_BINARY" "$WORK/lite/hara-lite"
cp "$HARA_ROOT/core/project.edn" "$WORK/lite/share/hara-lite/project.edn"
cp -R "$HARA_ROOT/core/lib" "$WORK/lite/share/hara-lite/lib"
tar -czf "$WORK/release/$LITE_TARBALL" -C "$WORK/lite" hara-lite share
mkdir -p "$WORK/truffle"
cp "$BINARY" "$WORK/truffle/hara-truffle"
tar -czf "$WORK/release/$TRUFFLE_TARBALL" -C "$WORK/truffle" hara-truffle
(cd "$WORK/release" && sha256sum "$TARBALL" "$LITE_TARBALL" "$TRUFFLE_TARBALL" > SHA256SUMS)

run_installer() { # run_installer <extra-env...>; stdout+stderr captured
  env -i PATH="$TEST_PATH" HOME="$TEST_HOME" \
    HARA_VERSION="$VERSION" \
    HARA_TARGET_TRIPLE="$TRIPLE" \
    HARA_RELEASE_BASE_URL="file://$WORK/release" \
    "$@" sh "$INSTALL_SH" --rust 2>&1
}

run_truffle_installer() { # run_truffle_installer <extra-env...>; stdout+stderr captured
  env -i PATH="$TEST_PATH" HOME="$TEST_HOME" \
    HARA_VERSION="$VERSION" \
    HARA_TARGET_TRIPLE="$TRIPLE" \
    HARA_RELEASE_BASE_URL="file://$WORK/release" \
    "$@" sh "$INSTALL_SH" --truffle 2>&1
}

run_lite_installer() { # run_lite_installer <extra-env...>; stdout+stderr captured
  env -i PATH="$TEST_PATH" HOME="$TEST_HOME" \
    HARA_VERSION="$VERSION" \
    HARA_TARGET_TRIPLE="$TRIPLE" \
    HARA_RELEASE_BASE_URL="file://$WORK/release" \
    "$@" sh "$INSTALL_SH" --rust-lite 2>&1
}

TEST_HOME="$WORK/home"
mkdir -p "$TEST_HOME"
# Deliberately exclude any install dir from PATH so the hint is exercised.
TEST_PATH="/usr/bin:/bin"

# --- 1. happy path ----------------------------------------------------------
OUT=$({ run_installer HARA_INSTALL_DIR="$WORK/bin"; } || echo "EXIT:$?")
check "installer exits 0 on happy path" test -x "$WORK/bin/hara"
RESULT=$("$WORK/bin/hara" eval '(+ 19 23)' 2>/dev/null || true)
check "installed binary evaluates (+ 19 23) => 42" test "$RESULT" = "42"
RESULT=$("$WORK/bin/hara" eval '(do (require [work.base :as work]) (:op (work/work-spec (work/pure :identity identity))))' 2>/dev/null || true)
check "installed binary resolves the Hara library project" test "$RESULT" = ":pure"
case "$OUT" in
  *"export PATH="*) ok "prints PATH hint when install dir not on PATH" ;;
  *) not_ok "prints PATH hint when install dir not on PATH" ;;
esac
case "$OUT" in
  *EXIT:*) not_ok "installer exit status was 0" ;;
  *) ok "installer exit status was 0" ;;
esac

# --- 2. dependency-light Rust install --------------------------------------
OUT=$({ run_lite_installer HARA_INSTALL_DIR="$WORK/lite-bin"; } || echo "EXIT:$?")
check "lite installer installs hara-lite" test -x "$WORK/lite-bin/hara-lite"
RESULT=$("$WORK/lite-bin/hara-lite" eval '(+ 19 23)' 2>/dev/null || true)
check "installed hara-lite evaluates (+ 19 23) => 42" test "$RESULT" = "42"
RESULT=$("$WORK/lite-bin/hara-lite" eval '(do (require [work.base :as work]) (:op (work/work-spec (work/pure :identity identity))))' 2>/dev/null || true)
check "installed hara-lite loads pure work primitives" test "$RESULT" = ":pure"
RESULT=$("$WORK/lite-bin/hara-lite" eval '(let [run (IWorkHost/work-submit (std.native.Work/default-host) (fn [work input options id] input) 42 {:id "installed-lite-work"})] (deref (IWorkRun/work-result run)))' 2>/dev/null || true)
check "installed hara-lite submits native work" test "$RESULT" = "42"
case "$OUT" in
  *EXIT:*) not_ok "lite installer exit status was 0" ;;
  *) ok "lite installer exit status was 0" ;;
esac

# --- 3. native-image install ------------------------------------------------
OUT=$({ run_truffle_installer HARA_INSTALL_DIR="$WORK/truffle-bin"; } || echo "EXIT:$?")
check "Truffle installer installs hara-truffle" test -x "$WORK/truffle-bin/hara-truffle"
RESULT=$("$WORK/truffle-bin/hara-truffle" eval '(+ 19 23)' 2>/dev/null || true)
check "installed hara-truffle evaluates (+ 19 23) => 42" test "$RESULT" = "42"
case "$OUT" in
  *EXIT:*) not_ok "Truffle installer exit status was 0" ;;
  *) ok "Truffle installer exit status was 0" ;;
esac

# --- 4. default install dir under HOME --------------------------------------
OUT=$({ run_installer; } || echo "EXIT:$?")
check "default install dir is \$HOME/.local/bin" test -x "$TEST_HOME/.local/bin/hara"

# --- 5. checksum mismatch aborts --------------------------------------------
cp "$WORK/release/SHA256SUMS" "$WORK/release/SHA256SUMS.good"
sed 's/^./X/' "$WORK/release/SHA256SUMS" > "$WORK/release/SHA256SUMS.bad"
mv "$WORK/release/SHA256SUMS.bad" "$WORK/release/SHA256SUMS"
OUT=$({ run_installer HARA_INSTALL_DIR="$WORK/bin-badsum"; } && echo "EXIT:0" || echo "EXIT:$?")
mv "$WORK/release/SHA256SUMS.good" "$WORK/release/SHA256SUMS"
case "$OUT" in
  *checksum*|*Checksum*|*CHECKSUM*) ok "checksum mismatch reported" ;;
  *) not_ok "checksum mismatch reported" ;;
esac
case "$OUT" in
  *EXIT:0*) not_ok "checksum mismatch aborts with nonzero exit" ;;
  *) ok "checksum mismatch aborts with nonzero exit" ;;
esac
check "checksum mismatch leaves no binary behind" test ! -e "$WORK/bin-badsum/hara"

# --- 6. unsupported platform ------------------------------------------------
OUT=$({ run_installer HARA_INSTALL_DIR="$WORK/bin-win" HARA_TARGET_TRIPLE="x86_64-pc-windows-msvc"; } \
      && echo "EXIT:0" || echo "EXIT:$?")
case "$OUT" in
  *"not supported"*) ok "unsupported platform reported" ;;
  *) not_ok "unsupported platform reported" ;;
esac
case "$OUT" in
  *EXIT:0*) not_ok "unsupported platform exits nonzero" ;;
  *) ok "unsupported platform exits nonzero" ;;
esac
check "unsupported platform installs nothing" test ! -e "$WORK/bin-win/hara"

# --- 7. overwrite existing install ------------------------------------------
OUT=$({ run_installer HARA_INSTALL_DIR="$WORK/bin"; } || echo "EXIT:$?")
case "$OUT" in
  *[Ee]xisting*|*verwrit*) ok "reinstall over existing binary is reported" ;;
  *) not_ok "reinstall over existing binary is reported" ;;
esac
check "reinstall keeps a working binary" test -x "$WORK/bin/hara"

# --- summary ----------------------------------------------------------------
echo "---"
echo "$((pass + fail)) tests, $pass passed, $fail failed"
[ "$fail" -eq 0 ]
