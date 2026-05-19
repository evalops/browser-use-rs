#!/usr/bin/env python3
"""Cut and verify the shared browser-use-rs workspace version."""

from __future__ import annotations

import argparse
import dataclasses
import os
import re
import subprocess
import sys
from pathlib import Path

try:
    import tomllib
except ModuleNotFoundError:  # pragma: no cover - Python < 3.11 fallback.
    import tomli as tomllib  # type: ignore[no-redef]


SEMVER_RE = re.compile(
    r"^v?"
    r"(?P<major>0|[1-9]\d*)\."
    r"(?P<minor>0|[1-9]\d*)\."
    r"(?P<patch>0|[1-9]\d*)"
    r"(?:-(?P<prerelease>[0-9A-Za-z.-]+))?"
    r"$"
)
WORKSPACE_PACKAGE_RE = re.compile(r"^\[workspace\.package\]\s*$")
SECTION_RE = re.compile(r"^\[.*\]\s*$")
VERSION_LINE_RE = re.compile(
    r'^(?P<prefix>\s*version\s*=\s*")(?P<version>[^"]+)(?P<suffix>".*)$'
)
CONVENTIONAL_BREAKING_RE = re.compile(r"^[a-z]+(?:\([^)]+\))?!:")
BREAKING_MARKER_RE = re.compile(
    r"\bBREAKING(?: |-)?CHANGE\b|^\s*BREAKING:", re.IGNORECASE | re.MULTILINE
)
CONVENTIONAL_FEATURE_RE = re.compile(r"^feat(?:\([^)]+\))?:", re.IGNORECASE)
PUBLIC_WORK_SUBJECT_RE = re.compile(
    r"^(add|complete|cover|enable|expose|finish|harden|implement|introduce|"
    r"publish|resolve|route|support|wire)\b",
    re.IGNORECASE,
)
RELEASE_IMPACT_RE = re.compile(
    r"(?im)^\s*(?:Release-Impact|Semver-Impact):\s*(?P<impact>major|minor|patch|none)\s*$"
)
SUBSTANTIAL_CAPABILITY_SUBJECT_RE = re.compile(
    r"\b("
    r"accept_downloads|action|agent|artifact|auto_download_pdfs|cdp|cli|cloud|"
    r"download|dom|iframe|launch|lifecycle|llm|mcp|protocol|provider|proxy|"
    r"recording|runtime|schema|session|state|storage|tool|trace|video|viewport"
    r")\b",
    re.IGNORECASE,
)
PATCH_SCOPE_SUBJECT_RE = re.compile(
    r"\b("
    r"alias(?:es)?|ci|clippy|config(?:uration)?|default(?:s)?|dependenc(?:y|ies)|docs?|"
    r"bookkeeping|cleanup|documentation|format|homebrew|internal|launchd|lint|lockfile|"
    r"metadata|readme|refactor|release|roadmap|serde|seriali[sz]e|systemd|test(?:s)?|"
    r"toolchain|typo|workflow"
    r")\b",
    re.IGNORECASE,
)
SUBSTANTIAL_PUBLIC_SOURCE_LINE_THRESHOLD = 240
TESTED_PUBLIC_SOURCE_LINE_THRESHOLD = 160
CROSS_CRATE_PUBLIC_SOURCE_LINE_THRESHOLD = 120
BULK_PUBLIC_SOURCE_LINE_THRESHOLD = 400
SUBSTANTIAL_PUBLIC_FILE_COUNT_THRESHOLD = 3
RELEASE_VERSION_PATTERN = r"v\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?"
RELEASE_COMMIT_RE = re.compile(rf"^Cut browser-use-rs {RELEASE_VERSION_PATTERN}$")
RELEASE_MAINTENANCE_COMMIT_RE = re.compile(
    rf"^Refresh lockfile for {RELEASE_VERSION_PATTERN} release$"
)
RELEASE_WORTHY_EXACT_PATHS = {
    "Cargo.lock",
    "Cargo.toml",
    "LICENSE",
    "NOTICE",
    "README.md",
    "docs/ARCHITECTURE.md",
    "docs/CLI.md",
    "docs/CONFORMANCE.md",
    "docs/DAEMON_SUPERVISION.md",
    "docs/INSTALL.md",
    "docs/MCP.md",
    "docs/RELEASE.md",
    "rust-toolchain.toml",
    "packaging/homebrew/browser-use-rs.rb.template",
    "packaging/homebrew/publish-tap.sh",
}
RELEASE_WORTHY_PREFIXES = (
    "crates/",
    "packaging/launchd/",
    "packaging/systemd/",
)
PUBLIC_CAPABILITY_DOC_PATHS = {
    "README.md",
    "docs/ARCHITECTURE.md",
    "docs/CLI.md",
    "docs/CONFORMANCE.md",
    "docs/INSTALL.md",
    "docs/MCP.md",
    "docs/RELEASE.md",
}
PUBLIC_SOURCE_PREFIXES = (
    "crates/browser-use-agent/src/",
    "crates/browser-use-cdp/src/",
    "crates/browser-use-cli/src/",
    "crates/browser-use-dom/src/",
    "crates/browser-use-llm/src/",
    "crates/browser-use-mcp/src/",
)
PUBLIC_TEST_PREFIXES = tuple(prefix.replace("/src/", "/tests/") for prefix in PUBLIC_SOURCE_PREFIXES)
RELEASE_TYPE_ORDER = {
    "patch": 1,
    "minor": 2,
    "major": 3,
}


@dataclasses.dataclass(frozen=True)
class Version:
    major: int
    minor: int
    patch: int
    prerelease: tuple[str, ...] = ()

    @classmethod
    def parse(cls, raw: str) -> "Version":
        match = SEMVER_RE.match(raw.strip())
        if not match:
            raise ValueError(
                f"{raw!r} is not a supported SemVer version; use X.Y.Z or X.Y.Z-prerelease"
            )
        prerelease = match.group("prerelease")
        return cls(
            major=int(match.group("major")),
            minor=int(match.group("minor")),
            patch=int(match.group("patch")),
            prerelease=tuple(prerelease.split(".")) if prerelease else (),
        )

    def bump(self, release_type: str) -> "Version":
        if release_type == "major":
            return Version(self.major + 1, 0, 0)
        if release_type == "minor":
            return Version(self.major, self.minor + 1, 0)
        if release_type == "patch":
            return Version(self.major, self.minor, self.patch + 1)
        raise ValueError(f"unsupported release type: {release_type}")

    def __str__(self) -> str:
        base = f"{self.major}.{self.minor}.{self.patch}"
        if self.prerelease:
            return f"{base}-{'.'.join(self.prerelease)}"
        return base

    def precedence_key(self) -> tuple[int, int, int]:
        return (self.major, self.minor, self.patch)

    def __lt__(self, other: "Version") -> bool:
        if self.precedence_key() != other.precedence_key():
            return self.precedence_key() < other.precedence_key()
        if not self.prerelease and other.prerelease:
            return False
        if self.prerelease and not other.prerelease:
            return True
        return compare_prerelease(self.prerelease, other.prerelease) < 0


@dataclasses.dataclass(frozen=True)
class Commit:
    sha: str
    subject: str
    body: str

    @property
    def full_message(self) -> str:
        return f"{self.subject}\n{self.body}"


@dataclasses.dataclass(frozen=True)
class ChangeStats:
    additions: int
    deletions: int

    @property
    def changed_lines(self) -> int:
        return self.additions + self.deletions


@dataclasses.dataclass(frozen=True)
class ReleasePlan:
    should_release: bool
    release_type: str | None
    base_ref: str | None
    reason: str
    commit_count: int
    changed_files: tuple[str, ...]


def compare_prerelease(left: tuple[str, ...], right: tuple[str, ...]) -> int:
    for left_part, right_part in zip(left, right):
        if left_part == right_part:
            continue
        left_numeric = left_part.isdigit()
        right_numeric = right_part.isdigit()
        if left_numeric and right_numeric:
            return -1 if int(left_part) < int(right_part) else 1
        if left_numeric:
            return -1
        if right_numeric:
            return 1
        return -1 if left_part < right_part else 1
    if len(left) == len(right):
        return 0
    return -1 if len(left) < len(right) else 1


def repo_root() -> Path:
    return Path(__file__).resolve().parents[1]


def read_toml(path: Path) -> dict:
    with path.open("rb") as handle:
        return tomllib.load(handle)


def workspace_member_manifest_paths(root: Path, root_manifest: dict) -> list[Path]:
    members = root_manifest.get("workspace", {}).get("members", [])
    paths: list[Path] = []
    for member in members:
        matches = sorted(root.glob(f"{member}/Cargo.toml"))
        if not matches:
            raise RuntimeError(f"workspace member {member!r} did not resolve to a Cargo.toml")
        paths.extend(matches)
    return paths


def cargo_metadata(root: Path) -> dict:
    import json

    result = subprocess.run(
        ["cargo", "metadata", "--format-version", "1", "--no-deps"],
        cwd=root,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
    )
    if result.returncode != 0:
        raise RuntimeError(f"cargo metadata failed:\n{result.stderr}")
    return json.loads(result.stdout)


def git(root: Path, *args: str, check: bool = True) -> str:
    result = subprocess.run(
        ["git", *args],
        cwd=root,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
    )
    if check and result.returncode != 0:
        raise RuntimeError(f"git {' '.join(args)} failed:\n{result.stderr}")
    return result.stdout


def validate_workspace(root: Path, expect_version: Version | None) -> Version:
    root_manifest_path = root / "Cargo.toml"
    root_manifest = read_toml(root_manifest_path)
    raw_version = root_manifest.get("workspace", {}).get("package", {}).get("version")
    if not isinstance(raw_version, str):
        raise RuntimeError("Cargo.toml must define [workspace.package] version")

    current = Version.parse(raw_version)
    if expect_version is not None and current != expect_version:
        raise RuntimeError(f"workspace version is {current}, expected {expect_version}")

    member_manifests = workspace_member_manifest_paths(root, root_manifest)
    for manifest_path in member_manifests:
        package = read_toml(manifest_path).get("package", {})
        version_value = package.get("version")
        if version_value != {"workspace": True}:
            raise RuntimeError(f"{manifest_path.relative_to(root)} must use version.workspace = true")

    metadata = cargo_metadata(root)
    member_manifest_paths = {
        str(manifest_path.resolve())
        for manifest_path in member_manifests
    }
    workspace_versions = {
        package["name"]: package["version"]
        for package in metadata["packages"]
        if package["manifest_path"] in member_manifest_paths
    }
    mismatched = {
        name: version
        for name, version in workspace_versions.items()
        if version != str(current)
    }
    if mismatched:
        details = ", ".join(f"{name}={version}" for name, version in sorted(mismatched.items()))
        raise RuntimeError(f"workspace package versions do not match {current}: {details}")

    return current


def write_workspace_version(root: Path, new_version: Version) -> None:
    manifest_path = root / "Cargo.toml"
    lines = manifest_path.read_text(encoding="utf-8").splitlines(keepends=True)
    in_workspace_package = False
    workspace_package_line = -1

    for index, line in enumerate(lines):
        if WORKSPACE_PACKAGE_RE.match(line):
            in_workspace_package = True
            workspace_package_line = index
            continue
        if in_workspace_package and SECTION_RE.match(line):
            lines.insert(workspace_package_line + 1, f'version = "{new_version}"\n')
            break
        if in_workspace_package:
            match = VERSION_LINE_RE.match(line)
            if match:
                newline = "\n" if line.endswith("\n") else ""
                lines[index] = (
                    f'{match.group("prefix")}{new_version}{match.group("suffix")}{newline}'
                )
                break
    else:
        if workspace_package_line < 0:
            raise RuntimeError("Cargo.toml is missing [workspace.package]")
        lines.insert(workspace_package_line + 1, f'version = "{new_version}"\n')

    manifest_path.write_text("".join(lines), encoding="utf-8")


def latest_stable_tag(root: Path) -> str | None:
    tags = []
    for raw_tag in git(root, "tag", "--list", "v[0-9]*", "--merged", "HEAD").splitlines():
        raw_tag = raw_tag.strip()
        if not raw_tag:
            continue
        try:
            version = Version.parse(raw_tag)
        except ValueError:
            continue
        if version.prerelease:
            continue
        tags.append((version, raw_tag))
    if not tags:
        return None
    return sorted(tags, key=lambda item: item[0])[-1][1]


def commits_since(root: Path, base_ref: str | None) -> list[Commit]:
    range_spec = f"{base_ref}..HEAD" if base_ref else "HEAD"
    raw = git(root, "log", "--format=%H%x1f%s%x1f%b%x1e", range_spec)
    commits = []
    for entry in raw.strip("\x1e\n").split("\x1e"):
        if not entry.strip():
            continue
        sha, subject, body = (entry.lstrip("\n").split("\x1f", 2) + ["", ""])[:3]
        if is_release_bookkeeping_subject(subject.strip()):
            continue
        commits.append(Commit(sha=sha, subject=subject.strip(), body=body.strip()))
    return commits


def is_release_bookkeeping_subject(subject: str) -> bool:
    return bool(
        RELEASE_COMMIT_RE.match(subject)
        or RELEASE_MAINTENANCE_COMMIT_RE.match(subject)
    )


def changed_files_for_commits(root: Path, commits: list[Commit]) -> tuple[str, ...]:
    changed_files = set[str]()
    for commit in commits:
        raw = git(root, "diff-tree", "--no-commit-id", "--name-only", "-r", commit.sha)
        changed_files.update(file for file in raw.splitlines() if file.strip())
    return tuple(sorted(changed_files))


def changed_file_stats_for_commits(root: Path, commits: list[Commit]) -> dict[str, ChangeStats]:
    stats: dict[str, ChangeStats] = {}
    for commit in commits:
        raw = git(root, "diff-tree", "--no-commit-id", "--numstat", "-r", commit.sha)
        for line in raw.splitlines():
            parts = line.split("\t", 2)
            if len(parts) != 3:
                continue
            additions, deletions, path = parts
            if additions == "-" or deletions == "-":
                continue
            current = stats.get(path, ChangeStats(additions=0, deletions=0))
            stats[path] = ChangeStats(
                additions=current.additions + int(additions),
                deletions=current.deletions + int(deletions),
            )
    return stats


def is_release_worthy_path(path: str) -> bool:
    if path in RELEASE_WORTHY_EXACT_PATHS:
        return True
    return any(path.startswith(prefix) for prefix in RELEASE_WORTHY_PREFIXES)


def release_worthy_files(changed_files: tuple[str, ...]) -> tuple[str, ...]:
    return tuple(path for path in changed_files if is_release_worthy_path(path))


def is_source_behavior_path(path: str) -> bool:
    return path.startswith("crates/") and "/src/" in path and path.endswith(".rs")


def source_behavior_files(changed_files: tuple[str, ...]) -> tuple[str, ...]:
    return tuple(path for path in changed_files if is_source_behavior_path(path))


def public_implementation_files(changed_files: tuple[str, ...]) -> tuple[str, ...]:
    return tuple(
        path
        for path in changed_files
        if path.endswith(".rs")
        and any(path.startswith(prefix) for prefix in PUBLIC_SOURCE_PREFIXES)
    )


def public_source_files(changed_files: tuple[str, ...]) -> tuple[str, ...]:
    return tuple(
        path
        for path in changed_files
        if path.endswith(".rs")
        and any(
            path.startswith(prefix)
            for prefix in (*PUBLIC_SOURCE_PREFIXES, *PUBLIC_TEST_PREFIXES)
        )
    )


def public_test_files(changed_files: tuple[str, ...]) -> tuple[str, ...]:
    return tuple(
        path
        for path in changed_files
        if path.endswith(".rs")
        and any(path.startswith(prefix) for prefix in PUBLIC_TEST_PREFIXES)
    )


def public_capability_doc_files(changed_files: tuple[str, ...]) -> tuple[str, ...]:
    return tuple(path for path in changed_files if path in PUBLIC_CAPABILITY_DOC_PATHS)


def changed_workspace_crates(changed_files: tuple[str, ...]) -> tuple[str, ...]:
    crates = set[str]()
    for path in changed_files:
        parts = path.split("/")
        if len(parts) >= 2 and parts[0] == "crates":
            crates.add(parts[1])
    return tuple(sorted(crates))


def commit_has_breaking_marker(commit: Commit) -> bool:
    return bool(
        CONVENTIONAL_BREAKING_RE.match(commit.subject)
        or BREAKING_MARKER_RE.search(commit.full_message)
    )


def release_impact_trailers(commit: Commit) -> tuple[str, ...]:
    return tuple(
        match.group("impact").lower()
        for match in RELEASE_IMPACT_RE.finditer(commit.full_message)
    )


def highest_release_type(release_types: tuple[str, ...]) -> str:
    return sorted(release_types, key=lambda release_type: RELEASE_TYPE_ORDER[release_type])[-1]


def commit_subject_is_patch_scoped(commit: Commit) -> bool:
    return bool(PATCH_SCOPE_SUBJECT_RE.search(commit.subject))


def commit_requests_no_release(commit: Commit) -> bool:
    return "none" in release_impact_trailers(commit)


def commit_requests_substantial_release(commit: Commit) -> bool:
    if CONVENTIONAL_FEATURE_RE.match(commit.subject):
        return True
    if commit_subject_is_patch_scoped(commit):
        return False
    return bool(
        PUBLIC_WORK_SUBJECT_RE.match(commit.subject)
        and SUBSTANTIAL_CAPABILITY_SUBJECT_RE.search(commit.full_message)
    )


def changed_line_count(
    files: tuple[str, ...],
    change_stats: dict[str, ChangeStats] | None,
) -> int:
    if change_stats is None:
        return 0
    return sum(
        change_stats.get(path, ChangeStats(additions=0, deletions=0)).changed_lines
        for path in files
    )


def substantial_release_signal(
    commits: list[Commit],
    changed_files: tuple[str, ...],
    change_stats: dict[str, ChangeStats] | None = None,
) -> str | None:
    """Return a reason when the unreleased batch looks like new public behavior.

    Auto mode should not infer a minor bump from cadence, commit count, or a
    polished additive subject. A minor release needs either explicit maintainer
    intent, Conventional Commit feature intent, or enough source evidence to
    look like substantial public behavior instead of a narrow config/doc/alias
    update.
    """

    feature_commit = next(
        (commit.subject for commit in commits if CONVENTIONAL_FEATURE_RE.match(commit.subject)),
        None,
    )
    if feature_commit:
        return f"feature commit found: {feature_commit}"

    substantial_subject = next(
        (commit.subject for commit in commits if commit_requests_substantial_release(commit)),
        None,
    )
    if (
        not substantial_subject
        and commits
        and all(commit_subject_is_patch_scoped(commit) for commit in commits)
    ):
        return None

    public_sources = public_implementation_files(changed_files)
    if not public_sources:
        return None

    public_source_lines = changed_line_count(public_sources, change_stats)
    capability_docs = public_capability_doc_files(changed_files)
    public_tests = public_test_files(changed_files)
    crates = changed_workspace_crates(public_sources)
    line_summary = f"{public_source_lines} changed public source lines"
    subject_summary = substantial_subject or "unreleased public source batch"

    if public_source_lines >= BULK_PUBLIC_SOURCE_LINE_THRESHOLD:
        return f"large public capability change: {subject_summary} ({line_summary})"

    if (
        len(crates) >= 2
        and public_source_lines >= CROSS_CRATE_PUBLIC_SOURCE_LINE_THRESHOLD
    ):
        return f"cross-crate public capability change: {subject_summary} ({line_summary})"

    if (
        capability_docs
        and public_tests
        and public_source_lines >= TESTED_PUBLIC_SOURCE_LINE_THRESHOLD
    ):
        return f"tested public capability change: {subject_summary} ({line_summary})"

    if capability_docs and public_source_lines >= SUBSTANTIAL_PUBLIC_SOURCE_LINE_THRESHOLD:
        return f"substantial public source/doc change: {subject_summary} ({line_summary})"

    if (
        capability_docs
        and len(public_sources) >= SUBSTANTIAL_PUBLIC_FILE_COUNT_THRESHOLD
        and public_source_lines >= CROSS_CRATE_PUBLIC_SOURCE_LINE_THRESHOLD
    ):
        return f"multi-file public capability change: {subject_summary} ({line_summary})"

    return None


def classify_auto_release(
    commits: list[Commit],
    changed_files: tuple[str, ...],
    change_stats: dict[str, ChangeStats] | None = None,
) -> tuple[str | None, str]:
    explicit_release_types = tuple(
        impact
        for commit in commits
        for impact in release_impact_trailers(commit)
        if impact != "none"
    )
    if explicit_release_types:
        release_type = highest_release_type(explicit_release_types)
        return release_type, f"Release-Impact trailer requested {release_type}"

    if commits and all(commit_requests_no_release(commit) for commit in commits):
        return None, "all unreleased commits are marked Release-Impact: none"

    if any(commit_has_breaking_marker(commit) for commit in commits):
        return "major", "breaking-change marker found in unreleased commits"

    substantial_reason = substantial_release_signal(commits, changed_files, change_stats)
    if substantial_reason:
        return "minor", substantial_reason

    if source_behavior_files(changed_files):
        return "patch", "Rust crate fix or internal behavior change"
    return "patch", "release-worthy docs, dependency, or packaged install asset changed"


def plan_auto_release(root: Path) -> ReleasePlan:
    base_ref = latest_stable_tag(root)
    commits = commits_since(root, base_ref)
    changed_files = changed_files_for_commits(root, commits)
    change_stats = changed_file_stats_for_commits(root, commits)
    worthy_files = release_worthy_files(changed_files)

    if not commits:
        return ReleasePlan(
            should_release=False,
            release_type=None,
            base_ref=base_ref,
            reason=f"no unreleased commits after {base_ref or 'repository start'}",
            commit_count=0,
            changed_files=worthy_files,
        )
    if not worthy_files:
        return ReleasePlan(
            should_release=False,
            release_type=None,
            base_ref=base_ref,
            reason=f"no release-worthy files changed after {base_ref or 'repository start'}",
            commit_count=len(commits),
            changed_files=worthy_files,
        )

    release_type, reason = classify_auto_release(commits, worthy_files, change_stats)
    if release_type is None:
        return ReleasePlan(
            should_release=False,
            release_type=None,
            base_ref=base_ref,
            reason=reason,
            commit_count=len(commits),
            changed_files=worthy_files,
        )

    return ReleasePlan(
        should_release=True,
        release_type=release_type,
        base_ref=base_ref,
        reason=reason,
        commit_count=len(commits),
        changed_files=worthy_files,
    )


def append_github_output(path: str | None, values: dict[str, str]) -> None:
    if not path:
        return
    with open(path, "a", encoding="utf-8") as handle:
        for key, value in values.items():
            handle.write(f"{key}={value}\n")


def synthetic_commit(subject: str, body: str = "") -> Commit:
    return Commit(sha="0" * 40, subject=subject, body=body)


def run_self_tests() -> int:
    import unittest

    class ReleaseVersionTests(unittest.TestCase):
        def test_release_worthy_paths_exclude_release_automation(self) -> None:
            changed_files = (
                ".github/workflows/release.yml",
                "scripts/release-version.py",
                "docs/RELEASE_AUTOMATION.md",
                "docs/ROADMAP.md",
            )
            self.assertEqual(release_worthy_files(changed_files), ())

        def test_release_worthy_paths_include_public_artifacts(self) -> None:
            changed_files = (
                "README.md",
                "docs/RELEASE.md",
                "crates/browser-use-cdp/src/lib.rs",
                "packaging/systemd/browser-use-rs.service",
            )
            self.assertEqual(release_worthy_files(changed_files), changed_files)

        def test_release_impact_trailer_overrides_substantial_heuristic(self) -> None:
            release_type, reason = classify_auto_release(
                [synthetic_commit("Add internal cache bookkeeping", "Release-Impact: patch")],
                ("crates/browser-use-cdp/src/lib.rs",),
            )
            self.assertEqual(release_type, "patch")
            self.assertIn("Release-Impact", reason)

        def test_release_impact_none_suppresses_release(self) -> None:
            release_type, reason = classify_auto_release(
                [synthetic_commit("Refresh release helper", "Release-Impact: none")],
                ("crates/browser-use-cdp/src/lib.rs",),
            )
            self.assertIsNone(release_type)
            self.assertIn("none", reason)

        def test_breaking_marker_wins_without_trailer(self) -> None:
            release_type, _ = classify_auto_release(
                [synthetic_commit("refactor!: rename public action result")],
                ("crates/browser-use-cdp/src/lib.rs",),
            )
            self.assertEqual(release_type, "major")

        def test_substantial_public_capability_change_is_minor(self) -> None:
            release_type, reason = classify_auto_release(
                [synthetic_commit("Add BrowserProfile accept_downloads parity")],
                (
                    "crates/browser-use-cdp/src/lib.rs",
                    "crates/browser-use-cdp/tests/browser_profile.rs",
                    "docs/CONFORMANCE.md",
                ),
                {
                    "crates/browser-use-cdp/src/lib.rs": ChangeStats(
                        additions=156,
                        deletions=10,
                    ),
                    "crates/browser-use-cdp/tests/browser_profile.rs": ChangeStats(
                        additions=34,
                        deletions=0,
                    ),
                },
            )
            self.assertEqual(release_type, "minor")
            self.assertIn("tested public capability", reason)

        def test_large_public_source_change_is_minor_without_trailer(self) -> None:
            release_type, reason = classify_auto_release(
                [synthetic_commit("Add BrowserProfile tracing recording parity")],
                (
                    "crates/browser-use-cdp/src/lib.rs",
                    "docs/CONFORMANCE.md",
                ),
                {
                    "crates/browser-use-cdp/src/lib.rs": ChangeStats(
                        additions=240,
                        deletions=22,
                    )
                },
            )
            self.assertEqual(release_type, "minor")
            self.assertIn("substantial public source/doc", reason)

        def test_bulk_public_source_change_is_minor_without_docs(self) -> None:
            release_type, reason = classify_auto_release(
                [synthetic_commit("Add CDP storage replay runtime")],
                ("crates/browser-use-cdp/src/lib.rs",),
                {
                    "crates/browser-use-cdp/src/lib.rs": ChangeStats(
                        additions=395,
                        deletions=12,
                    )
                },
            )
            self.assertEqual(release_type, "minor")
            self.assertIn("large public capability", reason)

        def test_cross_crate_public_capability_change_is_minor(self) -> None:
            release_type, reason = classify_auto_release(
                [synthetic_commit("Add MCP session artifact schema")],
                (
                    "crates/browser-use-cdp/src/lib.rs",
                    "crates/browser-use-mcp/src/lib.rs",
                ),
                {
                    "crates/browser-use-cdp/src/lib.rs": ChangeStats(
                        additions=70,
                        deletions=0,
                    ),
                    "crates/browser-use-mcp/src/lib.rs": ChangeStats(
                        additions=55,
                        deletions=0,
                    ),
                },
            )
            self.assertEqual(release_type, "minor")
            self.assertIn("cross-crate public capability", reason)

        def test_small_config_parity_change_is_patch_without_trailer(self) -> None:
            release_type, reason = classify_auto_release(
                [synthetic_commit("Add BrowserProfile trace path config parity")],
                (
                    "crates/browser-use-cdp/src/lib.rs",
                    "docs/CONFORMANCE.md",
                    "README.md",
                ),
                {
                    "crates/browser-use-cdp/src/lib.rs": ChangeStats(
                        additions=34,
                        deletions=3,
                    )
                },
            )
            self.assertEqual(release_type, "patch")
            self.assertIn("Rust crate", reason)

        def test_additive_subject_and_docs_without_substance_is_patch(self) -> None:
            release_type, reason = classify_auto_release(
                [synthetic_commit("Add BrowserProfile download parity")],
                (
                    "crates/browser-use-cdp/src/lib.rs",
                    "docs/CONFORMANCE.md",
                    "README.md",
                ),
                {
                    "crates/browser-use-cdp/src/lib.rs": ChangeStats(
                        additions=28,
                        deletions=4,
                    )
                },
            )
            self.assertEqual(release_type, "patch")
            self.assertIn("Rust crate", reason)

        def test_multiple_small_commits_do_not_become_minor_by_cadence(self) -> None:
            release_type, reason = classify_auto_release(
                [
                    synthetic_commit("Add BrowserProfile download alias"),
                    synthetic_commit("Document download alias"),
                    synthetic_commit("Fix download alias serialization"),
                ],
                (
                    "crates/browser-use-cdp/src/lib.rs",
                    "docs/CONFORMANCE.md",
                    "README.md",
                ),
                {
                    "crates/browser-use-cdp/src/lib.rs": ChangeStats(
                        additions=38,
                        deletions=5,
                    )
                },
            )
            self.assertEqual(release_type, "patch")
            self.assertIn("Rust crate", reason)

        def test_completed_provider_parity_slice_is_minor_without_trailer(self) -> None:
            release_type, reason = classify_auto_release(
                [synthetic_commit("Complete provider structured-output fallback parity")],
                (
                    "crates/browser-use-llm/src/lib.rs",
                    "crates/browser-use-cli/src/main.rs",
                    "docs/CLI.md",
                    "docs/CONFORMANCE.md",
                    "docs/MCP.md",
                    "docs/RELEASE.md",
                ),
                {
                    "crates/browser-use-llm/src/lib.rs": ChangeStats(
                        additions=176,
                        deletions=36,
                    ),
                    "crates/browser-use-cli/src/main.rs": ChangeStats(
                        additions=47,
                        deletions=0,
                    ),
                },
            )
            self.assertEqual(release_type, "minor")
            self.assertIn("cross-crate public capability", reason)

        def test_patch_scoped_bulk_source_change_stays_patch(self) -> None:
            release_type, reason = classify_auto_release(
                [synthetic_commit("Format browser crate sources")],
                (
                    "crates/browser-use-cdp/src/lib.rs",
                    "docs/CONFORMANCE.md",
                ),
                {
                    "crates/browser-use-cdp/src/lib.rs": ChangeStats(
                        additions=420,
                        deletions=30,
                    )
                },
            )
            self.assertEqual(release_type, "patch")
            self.assertIn("Rust crate", reason)

        def test_feature_commit_is_minor_even_without_docs(self) -> None:
            release_type, reason = classify_auto_release(
                [synthetic_commit("feat(cdp): support HAR recording")],
                ("crates/browser-use-cdp/src/lib.rs",),
            )
            self.assertEqual(release_type, "minor")
            self.assertIn("feature commit", reason)

        def test_additive_source_without_public_artifact_is_patch(self) -> None:
            release_type, reason = classify_auto_release(
                [synthetic_commit("Add internal BrowserProfile cache bookkeeping")],
                ("crates/browser-use-cdp/src/lib.rs",),
            )
            self.assertEqual(release_type, "patch")
            self.assertIn("Rust crate", reason)

        def test_internal_source_or_docs_change_is_patch(self) -> None:
            source_release_type, _ = classify_auto_release(
                [synthetic_commit("Fix stale download cache cleanup")],
                ("crates/browser-use-cdp/src/lib.rs",),
            )
            docs_release_type, _ = classify_auto_release(
                [synthetic_commit("Document release support matrix")],
                ("docs/RELEASE.md",),
            )
            self.assertEqual(source_release_type, "patch")
            self.assertEqual(docs_release_type, "patch")

    result = unittest.TextTestRunner(verbosity=2).run(
        unittest.defaultTestLoader.loadTestsFromTestCase(ReleaseVersionTests)
    )
    return 0 if result.wasSuccessful() else 1


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--release-type", choices=("auto", "major", "minor", "patch"))
    parser.add_argument("--version", help="Exact SemVer version to cut, with or without v prefix.")
    parser.add_argument("--expect-version", help="Fail unless the workspace version matches this.")
    parser.add_argument("--write", action="store_true", help="Update Cargo.toml to the requested version.")
    parser.add_argument("--check", action="store_true", help="Only validate workspace version consistency.")
    parser.add_argument("--self-test", action="store_true", help="Run release classification unit tests.")
    parser.add_argument(
        "--allow-no-release",
        action="store_true",
        help="Allow auto mode to exit successfully when no release-worthy changes exist.",
    )
    parser.add_argument(
        "--github-output",
        default=os.environ.get("GITHUB_OUTPUT"),
        help="Optional GitHub Actions output file.",
    )
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    if args.self_test:
        return run_self_tests()

    root = repo_root()
    expect_version = Version.parse(args.expect_version) if args.expect_version else None
    current = validate_workspace(root, expect_version)

    requested: Version | None = None
    auto_plan: ReleasePlan | None = None
    if args.version:
        requested = Version.parse(args.version)
        selected_release_type = "exact"
    elif args.release_type == "auto":
        auto_plan = plan_auto_release(root)
        selected_release_type = auto_plan.release_type
        if not auto_plan.should_release:
            values = {
                "should_release": "false",
                "previous_version": str(current),
                "version": str(current),
                "tag": f"v{current}",
                "release_type": "",
                "release_base": auto_plan.base_ref or "",
                "release_reason": auto_plan.reason,
                "commit_count": str(auto_plan.commit_count),
                "changed_files_count": str(len(auto_plan.changed_files)),
            }
            append_github_output(args.github_output, values)
            print(auto_plan.reason)
            if args.allow_no_release:
                return 0
            return 0
        requested = current.bump(auto_plan.release_type or "patch")
    elif args.release_type:
        selected_release_type = args.release_type
        requested = current.bump(args.release_type)
    else:
        selected_release_type = ""

    if requested is None:
        append_github_output(
            args.github_output,
            {
                "should_release": "false",
                "version": str(current),
                "tag": f"v{current}",
                "previous_version": str(current),
                "release_type": "",
                "release_base": "",
                "release_reason": "version check only",
                "commit_count": "0",
                "changed_files_count": "0",
            },
        )
        print(f"browser-use-rs workspace version is {current}")
        return 0

    if requested < current or requested == current:
        raise RuntimeError(f"requested version {requested} must be greater than current {current}")

    if args.write:
        write_workspace_version(root, requested)
        validate_workspace(root, requested)
        action = "updated"
    else:
        action = "would update"

    append_github_output(
        args.github_output,
        {
            "should_release": "true",
            "previous_version": str(current),
            "version": str(requested),
            "tag": f"v{requested}",
            "release_type": selected_release_type or "",
            "release_base": auto_plan.base_ref if auto_plan and auto_plan.base_ref else "",
            "release_reason": auto_plan.reason if auto_plan else "explicit release request",
            "commit_count": str(auto_plan.commit_count if auto_plan else 0),
            "changed_files_count": str(len(auto_plan.changed_files) if auto_plan else 0),
        },
    )
    print(f"{action} browser-use-rs workspace version from {current} to {requested}")
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except Exception as exc:
        print(f"release-version: {exc}", file=sys.stderr)
        raise SystemExit(1)
