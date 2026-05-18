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


def append_github_output(path: str | None, values: dict[str, str]) -> None:
    if not path:
        return
    with open(path, "a", encoding="utf-8") as handle:
        for key, value in values.items():
            handle.write(f"{key}={value}\n")


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--release-type", choices=("major", "minor", "patch"))
    parser.add_argument("--version", help="Exact SemVer version to cut, with or without v prefix.")
    parser.add_argument("--expect-version", help="Fail unless the workspace version matches this.")
    parser.add_argument("--write", action="store_true", help="Update Cargo.toml to the requested version.")
    parser.add_argument("--check", action="store_true", help="Only validate workspace version consistency.")
    parser.add_argument(
        "--github-output",
        default=os.environ.get("GITHUB_OUTPUT"),
        help="Optional GitHub Actions output file.",
    )
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    root = repo_root()
    expect_version = Version.parse(args.expect_version) if args.expect_version else None
    current = validate_workspace(root, expect_version)

    requested: Version | None = None
    if args.version:
        requested = Version.parse(args.version)
    elif args.release_type:
        requested = current.bump(args.release_type)

    if requested is None:
        append_github_output(
            args.github_output,
            {"version": str(current), "tag": f"v{current}", "previous_version": str(current)},
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
            "previous_version": str(current),
            "version": str(requested),
            "tag": f"v{requested}",
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
