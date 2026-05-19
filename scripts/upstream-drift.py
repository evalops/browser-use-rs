#!/usr/bin/env python3
"""Detect browser-use upstream drift for browser-use-rs."""

from __future__ import annotations

import argparse
import dataclasses
import os
import re
import subprocess
import sys
import tempfile
import textwrap
import unittest
from pathlib import Path


UPSTREAM_REPO = "browser-use/browser-use"
UPSTREAM_URL = f"https://github.com/{UPSTREAM_REPO}.git"
CURRENT_TARGET_RE = re.compile(
    r'INITIAL_UPSTREAM_COMMIT:\s*&str\s*=\s*"(?P<sha>[0-9a-f]{40})"'
)
SHA_RE = re.compile(r"^[0-9a-f]{40}$")


@dataclasses.dataclass(frozen=True)
class DriftPlan:
    current_sha: str
    latest_sha: str

    @property
    def drifted(self) -> bool:
        return self.current_sha != self.latest_sha

    @property
    def current_short(self) -> str:
        return self.current_sha[:7]

    @property
    def latest_short(self) -> str:
        return self.latest_sha[:7]

    @property
    def compare_url(self) -> str:
        return (
            f"https://github.com/{UPSTREAM_REPO}/compare/"
            f"{self.current_sha}...{self.latest_sha}"
        )

    @property
    def issue_title(self) -> str:
        return f"Refresh upstream target to {self.latest_short}"


def repo_root() -> Path:
    return Path(__file__).resolve().parents[1]


def normalize_sha(raw: str, source: str) -> str:
    sha = raw.strip().lower()
    if not SHA_RE.match(sha):
        raise ValueError(f"{source} must be a full 40-character lowercase hex commit SHA")
    return sha


def read_current_target(root: Path) -> str:
    source = root / "crates/browser-use-core/src/lib.rs"
    match = CURRENT_TARGET_RE.search(source.read_text(encoding="utf-8"))
    if not match:
        raise RuntimeError(f"could not find INITIAL_UPSTREAM_COMMIT in {source}")
    return normalize_sha(match.group("sha"), "current upstream target")


def latest_upstream_commit(root: Path) -> str:
    result = subprocess.run(
        ["git", "ls-remote", UPSTREAM_URL, "HEAD"],
        cwd=root,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        check=False,
    )
    if result.returncode != 0:
        raise RuntimeError(f"git ls-remote failed:\n{result.stderr}")
    sha = result.stdout.split("\t", 1)[0]
    return normalize_sha(sha, "latest upstream HEAD")


def issue_body(plan: DriftPlan) -> str:
    return textwrap.dedent(
        f"""\
        <!-- browser-use-rs-upstream-drift current={plan.current_sha} latest={plan.latest_sha} -->
        Upstream browser-use has moved beyond the frozen target used by browser-use-rs.

        - Current frozen target: `{plan.current_sha}`
        - Latest upstream HEAD: `{plan.latest_sha}`
        - Compare: {plan.compare_url}

        Keep this issue open until the new upstream commit has been audited. Do not update
        `INITIAL_UPSTREAM_COMMIT` or the docs target until the behavioral delta has either
        been implemented, explicitly scoped out, or split into concrete follow-up issues.

        Suggested audit checklist:

        - Review changes under `browser_use/browser`, `browser_use/dom`, `browser_use/agent`,
          `browser_use/llm`, `browser_use/mcp`, and packaging/CLI entrypoints.
        - File focused parity issues for each new or changed public behavior.
        - Update the frozen target only after those issues are resolved or documented as
          compatibility boundaries.
        - Run `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`,
          `cargo test --workspace`, and the release helper checks before closing.

        Automation note: the scheduled workflow edits this single issue only when the
        latest upstream commit changes, and does nothing when the frozen target is current.
        """
    )


def append_github_output(path: str | None, values: dict[str, str]) -> None:
    if not path:
        return
    with open(path, "a", encoding="utf-8") as handle:
        for key, value in values.items():
            handle.write(f"{key}={value}\n")


def write_body(path: Path | None, body: str) -> None:
    if path is not None:
        path.write_text(body, encoding="utf-8")


def run_self_tests() -> int:
    class UpstreamDriftTests(unittest.TestCase):
        def test_normalize_sha_accepts_full_lower_hex(self) -> None:
            sha = "a" * 40
            self.assertEqual(normalize_sha(sha, "test"), sha)

        def test_normalize_sha_rejects_short_sha(self) -> None:
            with self.assertRaises(ValueError):
                normalize_sha("abc1234", "test")

        def test_read_current_target_finds_core_constant(self) -> None:
            with tempfile.TemporaryDirectory() as temp_dir:
                root = Path(temp_dir)
                source = root / "crates/browser-use-core/src"
                source.mkdir(parents=True)
                (source / "lib.rs").write_text(
                    'pub const INITIAL_UPSTREAM_COMMIT: &str = "'
                    + ("1" * 40)
                    + '";\n',
                    encoding="utf-8",
                )
                self.assertEqual(read_current_target(root), "1" * 40)

        def test_plan_exposes_issue_fields(self) -> None:
            plan = DriftPlan(current_sha="1" * 40, latest_sha="2" * 40)
            self.assertTrue(plan.drifted)
            self.assertEqual(plan.issue_title, "Refresh upstream target to 2222222")
            self.assertIn("/compare/" + ("1" * 40) + "..." + ("2" * 40), plan.compare_url)
            self.assertIn("current=" + ("1" * 40), issue_body(plan))

    result = unittest.TextTestRunner(verbosity=2).run(
        unittest.defaultTestLoader.loadTestsFromTestCase(UpstreamDriftTests)
    )
    return 0 if result.wasSuccessful() else 1


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--current-sha", help="Override the frozen upstream target SHA.")
    parser.add_argument("--latest-sha", help="Override the latest upstream SHA.")
    parser.add_argument("--body-file", type=Path, help="Write the issue body to this path.")
    parser.add_argument(
        "--github-output",
        default=os.environ.get("GITHUB_OUTPUT"),
        help="Optional GitHub Actions output file.",
    )
    parser.add_argument("--self-test", action="store_true", help="Run helper unit tests.")
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    if args.self_test:
        return run_self_tests()

    root = repo_root()
    current_sha = (
        normalize_sha(args.current_sha, "current upstream target")
        if args.current_sha
        else read_current_target(root)
    )
    latest_sha = (
        normalize_sha(args.latest_sha, "latest upstream HEAD")
        if args.latest_sha
        else latest_upstream_commit(root)
    )
    plan = DriftPlan(current_sha=current_sha, latest_sha=latest_sha)
    body = issue_body(plan) if plan.drifted else ""
    write_body(args.body_file, body)

    append_github_output(
        args.github_output,
        {
            "drifted": "true" if plan.drifted else "false",
            "current_sha": plan.current_sha,
            "current_short": plan.current_short,
            "latest_sha": plan.latest_sha,
            "latest_short": plan.latest_short,
            "issue_title": plan.issue_title if plan.drifted else "",
            "compare_url": plan.compare_url if plan.drifted else "",
        },
    )

    if plan.drifted:
        print(
            f"upstream drift detected: {plan.current_short} -> {plan.latest_short} "
            f"({plan.compare_url})"
        )
    else:
        print(f"upstream target is current: {plan.current_sha}")
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except Exception as exc:
        print(f"upstream-drift: {exc}", file=sys.stderr)
        raise SystemExit(1)
