#!/usr/bin/env python3
"""Build the canonical Foundation API manifest from Hara's registered inventory."""
from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import subprocess
import sys
from pathlib import Path
from typing import Any

SCHEMA_VERSION = 2
DEFAULT_EXTRA_NAMESPACES = ("std.lib.collection",)
ALLOWED_MIGRATION_STATUSES = {
    "moved", "retired", "compatibility-only", "planned-replacement",
}


def canonical_json(value: Any) -> bytes:
    return json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=False).encode()


def digest(value: Any) -> str:
    return f"sha256:{hashlib.sha256(canonical_json(value)).hexdigest()}"


def repository_path(path: Path, root: Path) -> str:
    """Return stable repository-relative provenance; reject paths outside root."""
    root = root.resolve()
    path = path.resolve()
    try:
        return path.relative_to(root).as_posix()
    except ValueError as error:
        raise ValueError(f"Manifest provenance path must be inside Hara root {root}: {path}") from error


def read_inventory(path: Path) -> list[str]:
    return sorted({
        line.strip() for line in path.read_text().splitlines()
        if line.strip() and not line.lstrip().startswith("#")
    })


# Development-only sources that are registered but not production-bootstrapped
# (see core/rust/bootstrap.namespaces and the closed inventory in the
# foundation spec: exactly six production std.foundation namespaces).
DEVELOPMENT_ONLY_NAMESPACES = ("std.foundation.bootstrap",)


def selected_namespaces(inventory: list[str], extras: tuple[str, ...]) -> list[str]:
    registered = set(inventory) - set(DEVELOPMENT_ONLY_NAMESPACES)
    missing = sorted(set(extras) - registered)
    if missing:
        raise ValueError("Configured API namespaces are not registered: " + ", ".join(missing))
    return sorted(name for name in registered if (
        name == "std.foundation" or name.startswith("std.foundation.") or name in extras
    ))


def parse_runtime_config(path: Path) -> tuple[list[dict[str, Any]], list[dict[str, Any]]]:
    source = path.read_text()
    libraries = re.search(
        r"const (?:LIBRARIES|FOUNDATION_LIBRARIES):.*?=\s*&\[(.*?)\];",
        source,
        re.S,
    )
    natives = re.search(r"const NATIVE_TYPES:.*?=\s*&\[(.*?)\];", source, re.S)
    native_names = re.findall(r'"([^"]+)"', natives.group(1)) if natives else []
    if not native_names:
        declarations = path.parent.parent / "core" / "native_declarations.rs"
        if declarations.is_file():
            native_names = re.findall(
                r"#\[hara_native\((?:(?!\)\]).)*?name\s*=\s*\"([^\"]+)\"",
                declarations.read_text(),
                re.S,
            )
    if not libraries or not native_names:
        raise ValueError(f"Unable to parse runtime aliases from {path}")
    aliases = [{
        "alias": alias,
        "target": namespace,
        "kind": "namespace-alias",
        "automatic": True,
    } for _, namespace, alias in re.findall(
        r'\("([^"]+)",\s*"([^"]+)",\s*"([^"]+)"\)', libraries.group(1)
    ) if namespace.startswith("std.foundation.")]
    native_objects = [{
        "name": name,
        "namespace": f"std.native.{name}",
        "automaticAlias": name,
        "kind": "static-object",
    } for name in native_names]
    return (
        sorted(aliases, key=lambda item: item["alias"]),
        sorted(native_objects, key=lambda item: item["name"]),
    )


def load_migrations(path: Path | None) -> tuple[int, list[dict[str, Any]]]:
    if path is None:
        return 0, []
    document = json.loads(path.read_text())
    version, migrations = document.get("schemaVersion"), document.get("migrations")
    if not isinstance(version, int) or not isinstance(migrations, list):
        raise ValueError("Foundation migration ledger requires schemaVersion and migrations")
    seen: set[str] = set()
    for migration in migrations:
        former, status = migration.get("formerName"), migration.get("status")
        if not isinstance(former, str) or not former.startswith("std.foundation."):
            raise ValueError(f"Invalid Foundation migration name: {former!r}")
        if status not in ALLOWED_MIGRATION_STATUSES:
            raise ValueError(f"Invalid migration status for {former}: {status!r}")
        if former in seen:
            raise ValueError(f"Duplicate Foundation migration: {former}")
        if not migration.get("replacement") and not migration.get("disposition"):
            raise ValueError(f"Migration requires replacement or disposition: {former}")
        for field in ("requireRewrite", "callRewrite", "evidence"):
            if not migration.get(field):
                raise ValueError(f"Migration requires {field}: {former}")
        seen.add(former)
    return version, sorted(migrations, key=lambda item: item["formerName"])


def raw_api(args: argparse.Namespace, root: Path) -> dict[str, Any]:
    if args.api_index:
        return json.loads(args.api_index.read_text())
    command = [
        "cargo", "run", "--quiet", "--manifest-path", str(root / "core/rust/Cargo.toml"),
        "--bin", "hara-api-doc", "--", str(root / "core/lib/src"), str(root / "core/lib/test"),
    ]
    return json.loads(subprocess.check_output(command, text=True))


def build_manifest(
    api: dict[str, Any], inventory: list[str], migrations: list[dict[str, Any]],
    migration_schema: int, migration_path: str | None,
    aliases: list[dict[str, Any]], native_objects: list[dict[str, Any]], *,
    repository: str, source_ref: str, commit: str, profiles: list[str],
    inventory_path: str, extras: tuple[str, ...] = DEFAULT_EXTRA_NAMESPACES,
) -> dict[str, Any]:
    selected = selected_namespaces(inventory, extras)
    raw = {namespace["name"]: namespace for namespace in api.get("namespaces", [])}
    missing = sorted(set(selected) - raw.keys())
    unexpected = sorted(name for name in raw if (
        name == "std.foundation" or name.startswith("std.foundation.") or name in extras
    ) and name not in selected and name not in DEVELOPMENT_ONLY_NAMESPACES)
    if missing or unexpected:
        raise ValueError(
            f"Registered/source API mismatch: missing={missing or 'none'} unexpected={unexpected or 'none'}"
        )
    current = set(selected)
    conflicting = sorted(m["formerName"] for m in migrations if m["formerName"] in current)
    if conflicting:
        raise ValueError("Migration names are still current API: " + ", ".join(conflicting))

    namespaces = []
    for name in selected:
        namespace = dict(raw[name])
        namespace.update({
            "group": "foundation" if name == "std.foundation" or name.startswith("std.foundation.") else "library",
            "status": "implemented",
            "profiles": profiles,
        })
        namespaces.append(namespace)

    surface = {
        "schemaVersion": SCHEMA_VERSION,
        "profiles": profiles,
        "namespaces": [{
            "name": namespace["name"],
            "group": namespace["group"],
            "status": namespace["status"],
            "definitions": [{
                "name": definition["name"],
                "kind": definition["kind"],
                "signature": definition.get("signature", ""),
            } for definition in namespace.get("definitions", [])],
        } for namespace in namespaces],
        "aliases": aliases,
        "nativeObjects": native_objects,
    }
    migration_document = {"schemaVersion": migration_schema, "migrations": migrations}
    return {
        "schemaVersion": SCHEMA_VERSION,
        "source": {"repository": repository, "ref": source_ref, "commit": commit},
        "generator": {"name": "generate-foundation-api-manifest", "version": "2"},
        "inventory": {"path": inventory_path, "authority": "registered-standard-library-namespaces"},
        "profiles": profiles,
        "surfaceDigest": digest(surface),
        "migrationLedger": {
            "schemaVersion": migration_schema,
            "path": migration_path,
            "digest": digest(migration_document),
        },
        "namespaces": namespaces,
        "aliases": aliases,
        "nativeObjects": native_objects,
        "migrations": migrations,
    }


def git_value(root: Path, *arguments: str, fallback: str) -> str:
    try:
        return subprocess.check_output(["git", "-C", str(root), *arguments], text=True).strip() or fallback
    except (OSError, subprocess.CalledProcessError):
        return fallback


def parser() -> argparse.ArgumentParser:
    result = argparse.ArgumentParser()
    result.add_argument("--root", type=Path, default=Path(__file__).resolve().parents[1])
    result.add_argument("--api-index", type=Path)
    result.add_argument("--inventory", type=Path)
    result.add_argument("--migrations", type=Path)
    result.add_argument("--runtime-config", type=Path)
    result.add_argument("--repository", default="https://github.com/hara-lang/hara")
    result.add_argument("--ref", dest="source_ref")
    result.add_argument("--commit")
    result.add_argument("--profiles", default="rust,jvm,wasm")
    result.add_argument("--output", type=Path)
    return result


def main(argv: list[str] | None = None) -> int:
    args = parser().parse_args(argv)
    root = args.root.resolve()
    inventory = (args.inventory or root / "core/rust/standard-library.namespaces").resolve()
    migrations = (args.migrations or root / "core/spec/std/foundation-migrations.json").resolve()
    runtime = (args.runtime_config or root / "core/rust/src/kernel/generated.rs").resolve()
    source_ref = args.source_ref or os.environ.get("HARA_API_REF") or git_value(
        root, "branch", "--show-current", fallback="detached"
    )
    commit = args.commit or os.environ.get("HARA_API_COMMIT") or os.environ.get("GITHUB_SHA") or git_value(
        root, "rev-parse", "HEAD", fallback="unknown"
    )
    profiles = sorted({item.strip() for item in args.profiles.split(",") if item.strip()})
    if not profiles:
        raise ValueError("At least one runtime profile is required")
    aliases, native_objects = parse_runtime_config(runtime)
    migration_schema, migration_rows = load_migrations(migrations)
    manifest = build_manifest(
        raw_api(args, root), read_inventory(inventory), migration_rows, migration_schema,
        repository_path(migrations, root), aliases, native_objects,
        repository=args.repository, source_ref=source_ref, commit=commit, profiles=profiles,
        inventory_path=repository_path(inventory, root),
    )
    output = json.dumps(manifest, indent=2, ensure_ascii=False) + "\n"
    if args.output:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(output)
    else:
        sys.stdout.write(output)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
