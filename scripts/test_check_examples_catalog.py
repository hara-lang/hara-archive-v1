import copy
import json
import tempfile
import unittest
from pathlib import Path

import check_examples_catalog


class ExampleCatalogTest(unittest.TestCase):
    def test_catalog_covers_recursive_inventory(self):
        document = check_examples_catalog.load_catalog()

        check_examples_catalog.validate_document(document)

        paths = {entry["path"] for entry in document["entries"]}
        self.assertIn("services/src/services/api.hal", paths)
        self.assertIn("extensions/demo/000-answer-42/project.edn", paths)
        self.assertEqual(["catalog.json"], list(document["excludedPaths"]))

    def test_catalog_rejects_undeclared_nested_path(self):
        document = copy.deepcopy(check_examples_catalog.load_catalog())
        document["entries"].pop()

        with self.assertRaisesRegex(SystemExit, "recursive inventory mismatch"):
            check_examples_catalog.validate_document(document)

    def test_validation_uses_supplied_example_root(self):
        document = {
            "schemaVersion": 1,
            "authority": {
                "repository": "hara-lang/hara-specs-registry",
                "commit": check_examples_catalog.EXPECTED_REGISTRY,
            },
            "core": {
                "repository": "hara-lang/hara",
                "baseCommit": "0" * 40,
            },
            "excludedPaths": {"catalog.json": "catalog"},
            "entries": [
                {
                    "path": "nested/example.hal",
                    "kind": "user-facing-example",
                    "status": "deterministic",
                    "purpose": "test",
                    "governingSpecs": [],
                    "capabilities": [],
                    "supportedRuntimes": ["rust"],
                    "validation": {
                        "mode": "native-smoke",
                        "expectedStdout": "42",
                    },
                }
            ],
        }
        with tempfile.TemporaryDirectory() as directory:
            examples = Path(directory)
            (examples / "nested").mkdir()
            (examples / "nested/example.hal").write_text("42")
            (examples / "catalog.json").write_text(json.dumps(document))

            check_examples_catalog.validate_document(document, examples)


if __name__ == "__main__":
    unittest.main()
