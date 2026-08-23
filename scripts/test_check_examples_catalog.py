import copy
import unittest

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


if __name__ == "__main__":
    unittest.main()
