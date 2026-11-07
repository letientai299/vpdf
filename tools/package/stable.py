"""Check stable release inputs before publication."""

import subprocess
import sys

from tools.package.notes import STABLE, render, repository


def validate(tag):
    if not STABLE.fullmatch(tag):
        raise ValueError("Stable release tag required")


def version_text(root, tag):
    validate(tag)
    notes = render(root, tag)
    return f"vpdf {tag[1:]}\n\n{notes}\n\n{repository(root)}\n"


def verify_version(root, tag, output):
    if output != version_text(root, tag):
        raise ValueError("Packaged version or changelog differs")


def main():
    tag = sys.argv[1]
    validate(tag)
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
