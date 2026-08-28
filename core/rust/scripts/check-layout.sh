#!/usr/bin/env bash
set -euo pipefail

root="${HARA_LAYOUT_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)}"
baseline="${HARA_LAYOUT_BASELINE:-$root/scripts/layout-baseline.txt}"
maximum_lines="${HARA_LAYOUT_MAX_LINES:-700}"
failed=0

if [[ ! -f "$baseline" ]]; then
  echo "Missing Rust layout baseline: $baseline" >&2
  exit 1
fi
if [[ ! "$maximum_lines" =~ ^[0-9]+$ ]] || (( maximum_lines < 1 )); then
  echo "HARA_LAYOUT_MAX_LINES must be a positive integer" >&2
  exit 1
fi

baseline_limit() {
  local relative="$1"
  awk -v path="$relative" '
    $1 == "lines" && $2 == path { print $3; found = 1; exit }
    END { if (!found) print "" }
  ' "$baseline"
}

baseline_allows_modrs() {
  local relative="$1"
  grep -Fqx "modrs $relative" "$baseline"
}

# Validate the baseline itself and require stale or resolved debt entries to be removed.
while read -r kind relative limit extra; do
  [[ -z "${kind:-}" || "$kind" == \#* ]] && continue
  case "$kind" in
    lines)
      if [[ -z "${relative:-}" || ! "${limit:-}" =~ ^[0-9]+$ || -n "${extra:-}" ]]; then
        echo "Invalid line baseline entry: $kind ${relative:-} ${limit:-} ${extra:-}" >&2
        failed=1
        continue
      fi
      if (( limit <= maximum_lines )); then
        echo "Baseline entry for $relative is not legacy debt: $limit <= $maximum_lines" >&2
        failed=1
      fi
      if [[ ! -f "$root/$relative" ]]; then
        echo "Stale line baseline entry: $relative" >&2
        failed=1
        continue
      fi
      current_lines="$(wc -l < "$root/$relative" | tr -d '[:space:]')"
      if (( current_lines <= maximum_lines )); then
        echo "Resolved line baseline entry: $relative now has $current_lines lines; remove it from the baseline" >&2
        failed=1
      fi
      ;;
    modrs)
      if [[ -z "${relative:-}" || -n "${limit:-}" || -n "${extra:-}" ]]; then
        echo "Invalid mod.rs baseline entry: $kind ${relative:-} ${limit:-} ${extra:-}" >&2
        failed=1
        continue
      fi
      if [[ ! -f "$root/$relative" ]]; then
        echo "Stale mod.rs baseline entry: $relative" >&2
        failed=1
      fi
      ;;
    *)
      echo "Unknown Rust layout baseline entry: $kind" >&2
      failed=1
      ;;
  esac
done < "$baseline"

while IFS= read -r source; do
  relative="${source#"$root/"}"
  case "$relative" in
    src/core.rs|src/core/*|src/lib.rs|src/runtime/*|src/fiber.rs|src/bin/hara/repl.rs)
      # Runtime compatibility facades are responsibility-split without a
      # per-file size limit while their public surface remains flat.
      continue
      ;;
  esac

  lines="$(wc -l < "$source" | tr -d '[:space:]')"
  allowed="$(baseline_limit "$relative")"
  if [[ -z "$allowed" ]]; then
    allowed="$maximum_lines"
  fi
  if (( lines > allowed )); then
    if (( allowed > maximum_lines )); then
      echo "$relative grew to $lines lines; recorded legacy maximum is $allowed" >&2
    else
      echo "$relative has $lines lines; maximum is $maximum_lines" >&2
    fi
    failed=1
  fi
done < <(find "$root/src" -type f -name '*.rs' -print | sort)

while IFS= read -r source; do
  relative="${source#"$root/"}"
  if ! baseline_allows_modrs "$relative"; then
    echo "Use module.rs plus module/*.rs; unexpected legacy path: $relative" >&2
    failed=1
  fi
done < <(find "$root/src" -type f -name mod.rs -print | sort)

required=(
  src/lang/data/vector.rs
  src/lang/protocol/iassoc.rs
  src/lang/protocol/ilookup.rs
  src/lang/protocol/ifind.rs
)

for relative in "${required[@]}"; do
  if [[ ! -f "$root/$relative" ]]; then
    echo "Missing required dedicated module: $relative" >&2
    failed=1
  fi
done

exit "$failed"
