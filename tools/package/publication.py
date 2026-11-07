"""Bind published assets to their source revision."""

import argparse
import hashlib
import json
import subprocess
from pathlib import Path

from tools.package.notes import render
from tools.package.stable import validate

PLATFORMS = ("linux-x86_64", "macos-aarch64")


def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def identity(root, tag):
    validate(tag)
    revision = subprocess.check_output(
        ["git", "rev-parse", "HEAD"], cwd=root, text=True
    ).strip()
    return dict(
        tag=tag,
        revision=revision,
        changelog_sha256=hashlib.sha256(render(root, tag).encode()).hexdigest(),
    )


def assets(directory):
    result = {}
    for platform in PLATFORMS:
        name = f"vpdf-{platform}.tar.gz"
        checksum = digest(directory / name)
        if (
            directory / f"{name}.sha256"
        ).read_text() != f"{checksum}  {name}\n":
            raise ValueError("Published asset checksum differs")
        result[name] = checksum
    return result


def write(root, tag, directory):
    manifest = identity(root, tag) | dict(assets=assets(directory))
    (directory / "source.json").write_text(
        json.dumps(manifest, indent=2) + "\n"
    )


def verify(root, tag, directory):
    manifest = json.loads((directory / "source.json").read_text())
    if any(
        manifest.get(key) != value for key, value in identity(root, tag).items()
    ):
        raise ValueError("Published source identity differs")
    if manifest.get("assets") != assets(directory):
        raise ValueError("Published asset manifest differs")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("tag")
    parser.add_argument("directory", type=Path)
    parser.add_argument("--verify", action="store_true")
    args = parser.parse_args()
    (verify if args.verify else write)(Path.cwd(), args.tag, args.directory)


if __name__ == "__main__":
    main()
