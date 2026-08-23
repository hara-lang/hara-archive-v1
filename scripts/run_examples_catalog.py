#!/usr/bin/env python3
from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
from pathlib import Path

import check_examples_catalog


ROOT = Path(__file__).resolve().parents[1]
EXAMPLES = ROOT / "core" / "lib" / "examples"


def report(authority: str, path: str, runtime: str, phase: str, **values: str) -> None:
    fields = [
        f"example={path}",
        f"runtime={runtime}",
        f"phase={phase}",
        f"authority={authority}",
    ]
    fields.extend(
        f"{key}={json.dumps(value, separators=(',', ':'), sort_keys=True)}"
        for key, value in values.items()
    )
    print(" ".join(fields))


def run_native_smoke(
    entry: dict, native: Path, runtime: str, authority: str
) -> bool:
    path = entry["path"]
    command = [str(native), "run", str(EXAMPLES / path)]
    environment = {**os.environ, "HARA_RUNTIME": runtime}
    result = subprocess.run(
        command,
        cwd=ROOT,
        capture_output=True,
        text=True,
        env=environment,
    )
    if result.returncode != 0:
        report(
            authority,
            path,
            runtime,
            "execute",
            status="fail",
            diagnostic=result.stderr.strip() or f"exit {result.returncode}",
        )
        return False
    actual = result.stdout.removesuffix("\n")
    expected = entry["validation"]["expectedStdout"]
    if actual != expected:
        report(
            authority,
            path,
            runtime,
            "assert",
            status="fail",
            diagnostic=f"expected {expected!r}, got {actual!r}",
        )
        return False
    report(authority, path, runtime, "assert", status="pass", value=actual)
    return True


def main() -> int:
    parser = argparse.ArgumentParser(description="Run deterministic Hara examples.")
    parser.add_argument("--native", type=Path, required=True)
    parser.add_argument("--runtime", default="rust")
    args = parser.parse_args()

    document = check_examples_catalog.load_catalog()
    check_examples_catalog.validate_document(document)
    authority = (
        f"{document['authority']['repository']}@{document['authority']['commit']}"
    )
    passed = True
    for entry in document["entries"]:
        path = entry["path"]
        validation = entry["validation"]
        if validation["mode"] == "native-smoke":
            if args.runtime not in entry["supportedRuntimes"]:
                report(
                    authority,
                    path,
                    args.runtime,
                    "skip",
                    status="skip",
                    diagnostic=f"runtime {args.runtime!r} is not supported",
                )
                continue
            passed = run_native_smoke(entry, args.native, args.runtime, authority) and passed
            continue
        report(
            authority,
            path,
            args.runtime,
            "skip",
            status="skip",
            diagnostic=validation["reason"],
        )
    print(f"example catalog: {'pass' if passed else 'fail'}")
    return 0 if passed else 1


if __name__ == "__main__":
    sys.exit(main())
