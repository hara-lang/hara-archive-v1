#!/usr/bin/env python3
from __future__ import annotations

import tempfile
import unittest
from pathlib import Path

import check_work_flow_make_clean as audit


class WorkFlowMakeScratchAuditTest(unittest.TestCase):
    def setUp(self) -> None:
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name)

    def tearDown(self) -> None:
        self.temp.cleanup()

    def touch(self, relative: str) -> None:
        path = self.root / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.touch()

    def test_rejects_present_and_tracked_root_scratch_paths(self) -> None:
        present = [
            "core/private/tmp/work-flow-make-probe/hello.md",
            "core/project-heal--0000000000000001",
            "core/test-0000000000000001.clj",
        ]
        for path in present:
            self.touch(path)

        matches = audit.find_matches(
            self.root,
            present + ["core/project-heal--0000000000000002"],
        )

        self.assertEqual(
            {
                "core/private/tmp",
                "core/private/tmp/work-flow-make-probe",
                "core/private/tmp/work-flow-make-probe/hello.md",
                "core/project-heal--0000000000000001",
                "core/project-heal--0000000000000002",
                "core/test-0000000000000001.clj",
            },
            matches,
        )

    def test_does_not_match_nested_or_non_numbered_fixture_names(self) -> None:
        fixture_paths = [
            "core/lib/test-fixtures/project-heal--0000000000000001",
            "core/lib/test-fixtures/test-0000000000000001.clj",
            "core/project-heal--fixture",
            "core/test-fixture.clj",
        ]
        for path in fixture_paths:
            self.touch(path)

        self.assertEqual(set(), audit.find_matches(self.root, fixture_paths))

    def test_path_matching_is_root_anchored_and_syntax_aware(self) -> None:
        self.assertTrue(audit.is_scratch("core/test-42.clj"))
        self.assertTrue(audit.is_scratch("core/private/tmp/probe"))
        self.assertFalse(audit.is_scratch("notes/core/test-42.clj"))
        self.assertFalse(audit.is_scratch("core/test-42.clj.bak"))
        self.assertFalse(audit.is_scratch("core/project-heal--not-numbered"))


if __name__ == "__main__":
    unittest.main()
