#!/usr/bin/env python3
from __future__ import annotations

import tempfile
import unittest
from pathlib import Path

import generate_foundation_api_manifest as module


class FoundationManifestTest(unittest.TestCase):
    def raw_api(self):
        return {
            "schemaVersion": 1,
            "namespaces": [
                {
                    "name": "std.foundation",
                    "source": "std/foundation.hal",
                    "definitions": [],
                    "examples": [],
                },
                {
                    "name": "std.foundation.bytes",
                    "source": "std/foundation/bytes.hal",
                    "definitions": [],
                    "examples": [],
                },
                {
                    "name": "std.lib.collection",
                    "source": "std/lib/collection.hal",
                    "definitions": [],
                    "examples": [],
                },
            ],
        }

    def build(self, migrations=()):
        return module.build_manifest(
            self.raw_api(),
            ["std.foundation", "std.foundation.bytes", "std.lib.collection"],
            list(migrations),
            1,
            "core/spec/std/foundation-migrations.json",
            [
                {
                    "alias": "bytes",
                    "target": "std.foundation.bytes",
                    "kind": "namespace-alias",
                    "automatic": True,
                }
            ],
            [
                {
                    "name": "File",
                    "namespace": "std.native.File",
                    "automaticAlias": "File",
                    "kind": "static-object",
                }
            ],
            repository="https://github.com/hara-lang/hara",
            source_ref="main",
            commit="a" * 40,
            profiles=["jvm", "rust", "wasm"],
            inventory_path="core/rust/standard-library.namespaces",
        )

    def test_inventory_is_authoritative(self):
        manifest = self.build()
        self.assertEqual(
            [namespace["name"] for namespace in manifest["namespaces"]],
            [
                "std.foundation",
                "std.foundation.bytes",
                "std.lib.collection",
            ],
        )
        self.assertRegex(
            manifest["surfaceDigest"],
            r"^sha256:[0-9a-f]{64}$",
        )

    def test_registered_namespace_missing_from_source_is_rejected(self):
        with self.assertRaisesRegex(ValueError, "Registered/source API mismatch"):
            module.build_manifest(
                self.raw_api(),
                [
                    "std.foundation",
                    "std.foundation.pretty",
                    "std.lib.collection",
                ],
                [],
                0,
                None,
                [],
                [],
                repository="repo",
                source_ref="main",
                commit="a" * 40,
                profiles=["rust"],
                inventory_path="core/rust/standard-library.namespaces",
            )

    def test_migration_cannot_still_be_current(self):
        migration = {
            "formerName": "std.foundation.bytes",
            "status": "moved",
            "replacement": {
                "kind": "native-static-object",
                "name": "Bytes",
            },
            "requireRewrite": "remove",
            "callRewrite": "use Bytes",
            "evidence": ["path"],
        }
        with self.assertRaisesRegex(ValueError, "still current API"):
            self.build([migration])

    def test_runtime_config_separates_aliases_and_native_objects(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "generated.rs"
            path.write_text(
                '''
const LIBRARIES: &[(&str, &str, &str)] = &[
    ("string", "std.foundation.string", "str"),
];
const NATIVE_TYPES: &[&str] = &["File", "Json"];
'''
            )
            aliases, native = module.parse_runtime_config(path)
            self.assertEqual(aliases[0]["alias"], "str")
            self.assertEqual(
                [item["name"] for item in native],
                ["File", "Json"],
            )

    def test_runtime_config_reads_annotated_native_declarations(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            path = root / "core/rust/src/kernel/generated.rs"
            path.parent.mkdir(parents=True)
            path.write_text(
                '''
const FOUNDATION_LIBRARIES: &[(&str, &str, &str)] = &[
    ("string", "std.foundation.string", "str"),
];
'''
            )
            declarations = root / "core/rust/src/core/native_declarations.rs"
            declarations.parent.mkdir(parents=True)
            declarations.write_text(
                '''
#[hara_native(namespace = "std.native", name = "File", methods = ["read"], provider = native_file_provider)]
struct File;
#[hara_native(
    namespace = "std.native",
    name = "Json",
    methods = ["read", "write"],
    provider = native_json_provider
)]
struct Json;
'''
            )
            aliases, native = module.parse_runtime_config(path)
            self.assertEqual(aliases[0]["alias"], "str")
            self.assertEqual(
                [item["name"] for item in native],
                ["File", "Json"],
            )

    def test_digest_is_deterministic(self):
        self.assertEqual(
            module.digest({"b": 2, "a": 1}),
            module.digest({"a": 1, "b": 2}),
        )

    def test_provenance_paths_are_repository_relative(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory) / "hara"
            path = root / "core/spec/std/foundation-migrations.json"
            path.parent.mkdir(parents=True)
            path.write_text("{}")
            self.assertEqual(
                module.repository_path(path, root),
                "core/spec/std/foundation-migrations.json",
            )

        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory) / "hara"
            root.mkdir()
            outside = Path(directory) / "outside.json"
            outside.write_text("{}")
            with self.assertRaisesRegex(ValueError, "inside Hara root"):
                module.repository_path(outside, root)


if __name__ == "__main__":
    unittest.main()
