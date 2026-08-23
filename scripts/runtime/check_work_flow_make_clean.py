#!/usr/bin/env python3
"""Reject committed or leftover Work make scratch paths."""

from __future__ import annotations

import argparse
import re
import subprocess
import sys
from pathlib import Path
from typing import Iterable


PATTERNS = (
    re.compile(r"^core/private/tmp(?:/.*)?$"),
    re.compile(r"^core/project-heal--[0-9]+$"),
    re.compile(r"^core/test-[0-9]+\.clj$"),
)


def is_scratch(path: str) -> bool:
    return any(pattern.fullmatch(path) for pattern in PATTERNS)


def find_matches(root: Path, tracked: Iterable[str]) -> set[str]:
    matches = {path for path in tracked if path and is_scratch(path)}
    matches.update(
        str(path.relative_to(root))
        for path in root.rglob("*")
        if is_scratch(str(path.relative_to(root)))
    )
    return matches


def main(root: Path) -> None:
    tracked = subprocess.run(
        ["git", "ls-files", "-z"],
        check=True,
        cwd=root,
        stdout=subprocess.PIPE,
    ).stdout.decode().split("\0")
    matches = find_matches(root, tracked)
    if matches:
        print("work.flow.make scratch paths detected:", file=sys.stderr)
        print("\n".join(sorted(matches)), file=sys.stderr)
        raise SystemExit(1)


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", type=Path, required=True)
    main(parser.parse_args().root.resolve())
