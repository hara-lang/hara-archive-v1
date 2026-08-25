#!/usr/bin/env python3
"""Enforce built-in native static objects in canonical HAL sources."""

from __future__ import annotations

import argparse
import re
import sys
from dataclasses import dataclass
from pathlib import Path
from typing import Iterable, Sequence

ROOT = Path(__file__).resolve().parents[2]
DEFAULT_ROOTS = (
    ROOT / "core/lib/src",
    ROOT / "core/lib/src-lang",
    ROOT / "core/lib/integration",
)

LEGACY_CALLS = {
    "os/spawn": "Process/spawn",
    "os/process-alive?": "Process/alive?",
    "os/process-write": "Process/write",
    "os/process-close-input": "Process/close-input",
    "os/process-stdout": "Process/stdout",
    "os/process-stderr": "Process/stderr",
    "os/process-wait": "Process/wait",
    "os/process-kill": "Process/kill",
}

NATIVE_STATIC = re.compile(r"^std\.native\.([A-Z][A-Za-z0-9]*)(/[^\s\[\](){}\";,]+)?$")
TOKEN_DELIMITERS = frozenset(" \t\r\n[](){}\";,")


@dataclass(frozen=True)
class Finding:
    path: Path
    line: int
    column: int
    legacy: str
    replacement: str


def code_tokens(source: str) -> Iterable[tuple[int, int, str]]:
    """Yield one-based locations for tokens outside strings and comments."""
    line = column = 1
    index = 0
    while index < len(source):
        char = source[index]
        if char == ";":
            while index < len(source) and source[index] != "\n":
                index += 1
                column += 1
            continue
        if char == '"':
            index += 1
            column += 1
            escaped = False
            while index < len(source):
                char = source[index]
                index += 1
                if char == "\n":
                    line += 1
                    column = 1
                    escaped = False
                else:
                    column += 1
                    if escaped:
                        escaped = False
                    elif char == "\\":
                        escaped = True
                    elif char == '"':
                        break
            continue
        if char in TOKEN_DELIMITERS:
            index += 1
            if char == "\n":
                line += 1
                column = 1
            else:
                column += 1
            continue
        start = index
        token_line = line
        token_column = column
        while index < len(source) and source[index] not in TOKEN_DELIMITERS:
            index += 1
            column += 1
        yield token_line, token_column, source[start:index]


def hal_files(roots: Sequence[Path]) -> Iterable[Path]:
    for root in roots:
        if not root.exists():
            continue
        if root.is_file():
            if root.suffix == ".hal":
                yield root
            continue
        yield from sorted(root.rglob("*.hal"))


def audit_file(path: Path) -> list[Finding]:
    findings: list[Finding] = []
    source = path.read_text(encoding="utf-8")
    for line, column, token in code_tokens(source):
        replacement = LEGACY_CALLS.get(token)
        if replacement:
            findings.append(Finding(path, line, column, token, replacement))
            continue
        native = NATIVE_STATIC.fullmatch(token)
        if native:
            suffix = native.group(2) or ""
            findings.append(
                Finding(path, line, column, token, native.group(1) + suffix)
            )
    return findings


def audit(roots: Sequence[Path]) -> list[Finding]:
    findings: list[Finding] = []
    for path in hal_files(roots):
        findings.extend(audit_file(path))
    return findings


def display_path(path: Path) -> str:
    try:
        return path.resolve().relative_to(ROOT.resolve()).as_posix()
    except ValueError:
        return path.as_posix()


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "roots",
        nargs="*",
        type=Path,
        default=list(DEFAULT_ROOTS),
        help="canonical HAL files or directories to scan",
    )
    args = parser.parse_args(argv)
    findings = audit(args.roots)
    if findings:
        for finding in findings:
            print(
                f"{display_path(finding.path)}:{finding.line}:{finding.column}: "
                f"legacy {finding.legacy}; use {finding.replacement}",
                file=sys.stderr,
            )
        print(
            f"native-static-object-audit: {len(findings)} noncanonical call(s)",
            file=sys.stderr,
        )
        return 2
    print("native-static-object-audit: built-in static-object surface only")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
