#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
runner="$script_dir/run-rust-conformance-groups"

list="$("$runner" --list)"
for group in core kernel lang runtime observation resp vm whole-wasm; do
    grep -Fq "  $group" <<<"$list"
done

dry_run="$("$runner" --dry-run --features bytecode-vm vm)"
grep -Fq -- "--features bytecode-vm" <<<"$dry_run"
grep -Fq -- "vm::" <<<"$dry_run"
grep -Fq -- "/core/rust/runtime/Cargo.toml" <<<"$dry_run"

if "$runner" --dry-run unknown >/dev/null 2>&1; then
    echo "unknown test group unexpectedly succeeded" >&2
    exit 1
fi

printf 'Rust conformance group runner tests passed.\n'
