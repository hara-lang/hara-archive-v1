#!/usr/bin/env python3
from __future__ import annotations

import json
import sys
from pathlib import Path
from pathlib import PurePosixPath

ROOT = Path(__file__).resolve().parents[1]
EXAMPLES = ROOT / "core" / "lib" / "examples"
CATALOG = EXAMPLES / "catalog.json"
EXPECTED_REGISTRY = "64d81ebe5fded2809c6fc4414796a3feddf98a33"


def fail(message: str) -> None:
    raise SystemExit(f"examples/catalog: {message}")


def load_catalog() -> dict:
    return json.loads(CATALOG.read_text())


def inventory(root: Path = EXAMPLES) -> list[str]:
    return sorted(
        path.relative_to(root).as_posix()
        for path in root.rglob("*")
        if path.is_file()
    )


def validate_document(document: dict, examples: Path = EXAMPLES) -> None:
    if document.get("schemaVersion") != 1:
        fail("schemaVersion must be 1")
    authority = document.get("authority") or {}
    if authority.get("repository") != "hara-lang/hara-specs-registry":
        fail("unexpected specification authority")
    if authority.get("commit") != EXPECTED_REGISTRY:
        fail("unexpected specification revision")
    core = document.get("core") or {}
    if core.get("repository") != "hara-lang/hara":
        fail("unexpected core repository")
    base_commit = core.get("baseCommit")
    if (
        not isinstance(base_commit, str)
        or len(base_commit) != 40
        or any(character not in "0123456789abcdef" for character in base_commit)
    ):
        fail("core.baseCommit must be a full lowercase commit SHA")

    entries = document.get("entries")
    if not isinstance(entries, list) or not entries:
        fail("entries must be a non-empty list")

    excluded = document.get("excludedPaths") or {}
    if not isinstance(excluded, dict) or not all(
        isinstance(path, str) and isinstance(reason, str) and reason.strip()
        for path, reason in excluded.items()
    ):
        fail("excludedPaths must map paths to reasons")

    actual = inventory(examples)
    declared = sorted(entry.get("path") for entry in entries if isinstance(entry, dict))
    covered = sorted(set(declared) | set(excluded))
    if actual != covered:
        fail(f"recursive inventory mismatch: actual={actual!r} declared={covered!r}")

    seen = set()
    allowed_modes = {"native-smoke", "deferred", "inventory-only"}
    for entry in entries:
        path = entry.get("path")
        parsed_path = PurePosixPath(path) if isinstance(path, str) else None
        if (
            not isinstance(path, str)
            or not path
            or "\\" in path
            or parsed_path is None
            or parsed_path.is_absolute()
            or parsed_path.as_posix() != path
            or any(part in {"", ".", ".."} for part in parsed_path.parts)
        ):
            fail(f"unsafe or invalid entry path: {path!r}")
        if path in seen:
            fail(f"duplicate entry: {path}")
        seen.add(path)
        target = examples / path
        if not target.is_file():
            fail(f"missing example path: {path}")
        if not entry.get("kind") or not entry.get("status") or not entry.get("purpose"):
            fail(f"{path}: kind, purpose, and status are required")
        specs = entry.get("governingSpecs")
        capabilities = entry.get("capabilities")
        if not isinstance(specs, list) or not all(isinstance(item, str) for item in specs):
            fail(f"{path}: governingSpecs must be a string list")
        if not isinstance(capabilities, list) or not all(isinstance(item, str) for item in capabilities):
            fail(f"{path}: capabilities must be a string list")
        runtimes = entry.get("supportedRuntimes")
        if not isinstance(runtimes, list) or not all(isinstance(item, str) for item in runtimes):
            fail(f"{path}: supportedRuntimes must be a string list")
        validation = entry.get("validation") or {}
        mode = validation.get("mode")
        if mode not in allowed_modes:
            fail(f"{path}: unsupported validation mode {mode!r}")
        if mode == "native-smoke" and not isinstance(validation.get("expectedStdout"), str):
            fail(f"{path}: native-smoke requires expectedStdout")
        if mode in {"deferred", "inventory-only"} and not validation.get("reason"):
            fail(f"{path}: {mode} requires a reason")


def main() -> int:
    document = load_catalog()
    validate_document(document)
    print(
        f"validated {len(document['entries'])} recursive example entries against "
        f"hara-specs-registry@{EXPECTED_REGISTRY}"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
