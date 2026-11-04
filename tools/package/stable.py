"""Check stable release inputs before publication."""

import re
import subprocess
import sys
import tomllib
from pathlib import Path


def validate(root, tag):
    if not re.fullmatch(
        r"v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)", tag
    ):
        raise ValueError("Stable release tag required")
    notes = (root / "changelogs" / f"{tag}.md").read_text().strip()
    heading, body = notes.split("\n", 1)
    if heading != f"# Vpdf {tag}" or not body.strip():
        raise ValueError("Release changelog differs from tag")


def version_text(root, tag):
    validate(root, tag)
    package = tomllib.loads((root / "Cargo.toml").read_text())["package"]
    notes = (root / "changelogs" / f"{tag}.md").read_text().strip()
    return f"vpdf {tag[1:]}\n\n{notes}\n\n{package['repository']}\n"


def verify_version(root, tag, output):
    if output != version_text(root, tag):
        raise ValueError("Packaged version or changelog differs")


def main():
    root = Path.cwd()
    tag = sys.argv[1]
    validate(root, tag)
    revision = subprocess.check_output(
        ["git", "rev-parse", "HEAD"], text=True
    ).strip()
    tagged = subprocess.check_output(
        ["git", "rev-parse", f"refs/tags/{tag}^{{commit}}"], text=True
    ).strip()
    if revision != tagged:
        raise ValueError("Release tag differs from checkout")
    if subprocess.check_output(
        ["git", "status", "--porcelain"], text=True
    ).strip():
        raise ValueError("Release checkout contains changes")


if __name__ == "__main__":
    main()
