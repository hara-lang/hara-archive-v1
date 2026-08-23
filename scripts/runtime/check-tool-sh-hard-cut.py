#!/usr/bin/env python3
"""Enforce the canonical tool.sh process boundary and namespace hard cuts."""

from __future__ import annotations

from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[2]

REQUIRED = {
    "tool.sh": ROOT / "core/lib/src/tool/sh.hal",
    "tool.sh.desktop": ROOT / "core/lib/src/tool/sh/desktop.hal",
    "tool.sh.docker": ROOT / "core/lib/src/tool/sh/docker.hal",
    "tool.sh.git": ROOT / "core/lib/src/tool/sh/git.hal",
    "tool.sh.net": ROOT / "core/lib/src/tool/sh/net.hal",
    "tool.sh.tmux": ROOT / "core/lib/src/tool/sh/tmux.hal",
}

FORBIDDEN_PATHS = (
    ROOT / "core/lib/src/lib/docker.hal",
    ROOT / "core/lib/src/tool/sh/misc.hal",
    ROOT / "core/lib/src/tool/sh/network.hal",
    ROOT / "core/lib/test/tool/sh/domain_test.hal",
)

DOMAIN_RUNNER_OWNERS = {
    ROOT / "core/lib/src/tool/sh/desktop.hal",
    ROOT / "core/lib/src/tool/sh/docker.hal",
    ROOT / "core/lib/src/tool/sh/git.hal",
    ROOT / "core/lib/src/tool/sh/net.hal",
    ROOT / "core/lib/src/tool/sh/tmux.hal",
}

ONE_SHOT_OWNERS = (
    ROOT / "core/lib/src/code/project/deploy/runtime.hal",
    ROOT / "core/lib/src/lang/runtime/basic/type_oneshot.hal",
    ROOT / "core/lib/src/lang/runtime/basic/type_verify.hal",
    ROOT / "core/lib/src/tool/cli/identity.hal",
    ROOT / "core/lib/src/tool/runtime.hal",
)

ACTIVE_ROOTS = (
    ROOT / "core/lib/src",
    ROOT / "core/lib/src-lang",
    ROOT / "core/lib/test",
    ROOT / "core/lib/test-lang",
    ROOT / "core/lib/integration",
)


def fail(message: str) -> None:
    print(message, file=sys.stderr)
    raise SystemExit(1)


def hal_files() -> list[Path]:
    files: list[Path] = []
    for root in ACTIVE_ROOTS:
        if root.exists():
            files.extend(root.rglob("*.hal"))
    return sorted(files)


def main() -> None:
    for namespace, path in REQUIRED.items():
        if not path.is_file():
            fail(f"missing canonical {namespace}: {path.relative_to(ROOT)}")
        if f"(ns {namespace}" not in path.read_text(encoding="utf-8"):
            fail(f"{path.relative_to(ROOT)} does not declare {namespace}")

    for path in FORBIDDEN_PATHS:
        if path.exists():
            fail(f"retired tool path remains: {path.relative_to(ROOT)}")

    for path in hal_files():
        text = path.read_text(encoding="utf-8")
        for token in ("tool.sh.misc", "tool.sh.network"):
            if token in text:
                fail(f"retired namespace {token} remains in {path.relative_to(ROOT)}")
        if ":tool.sh/runner" in text and path not in DOMAIN_RUNNER_OWNERS and "/test" not in path.as_posix():
            fail(f"runner injection escaped a command domain: {path.relative_to(ROOT)}")

    core = REQUIRED["tool.sh"].read_text(encoding="utf-8")
    for token in ("(defn run!", "(def run!", "execute-checked", ":trim", ":tool.sh/runner"):
        if token in core:
            fail(f"retired core tool.sh token remains: {token}")
    if "#{:cwd :env :stdin :timeout}" not in core:
        fail("tool.sh does not declare the closed process option set")
    for delegate in ("stdout", "stderr", "wait", "write", "close-input", "alive?", "kill"):
        if f"(defn {delegate}" not in core:
            fail(f"tool.sh is missing process delegate {delegate}")
    if "(defn run-checked" not in core:
        fail("tool.sh is missing run-checked")

    for path in ONE_SHOT_OWNERS:
        text = path.read_text(encoding="utf-8")
        if "Process/" in text:
            fail(f"one-shot caller still owns Process lifecycle: {path.relative_to(ROOT)}")

    inventory = set(
        (ROOT / "core/rust/standard-library.namespaces")
        .read_text(encoding="utf-8")
        .splitlines()
    )
    required_inventory = set(REQUIRED) | {"tool.runtime"}
    missing = sorted(required_inventory - inventory)
    if missing:
        fail(f"Rust standard-library inventory misses: {missing}")

    print("tool-sh-hard-cut: canonical process and command-domain ownership is green")


if __name__ == "__main__":
    main()
