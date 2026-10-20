#!/usr/bin/env python3
"""Build immutable benchmark snapshots before timing.

Run under mise in each checkout. Select only the targets needed by the
comparison. Cargo's artifact messages locate the exact executable; copy it
before another checkout can overwrite a shared target directory. Never build
while a measurement runs. --symbols retains symbols for later profiling; it
does not guarantee usable call chains. Timings from profiled runs are separate.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import shutil
import subprocess

ROOT = Path(__file__).resolve().parent.parent
TARGETS = {
    "first_use": ("vpdf", "first_use"),
    "latency": ("vpdf", "latency"),
    "stages": ("vpdf", "stages"),
    "pdf-micro": ("vpdf-pdf", "micro"),
    "ui-micro": ("vpdf-ui", "micro"),
}


def digest(path):
    value = hashlib.sha256()
    with Path(path).open("rb") as file:
        for block in iter(lambda: file.read(1 << 20), b""):
            value.update(block)
    return value.hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output", type=Path, help="New snapshot directory")
    parser.add_argument("--bench", choices=TARGETS, action="append", required=True)
    parser.add_argument("--symbols", action="store_true")
    args = parser.parse_args()
    args.output = args.output.resolve()
    args.output.mkdir(parents=True, exist_ok=False)
    env = os.environ.copy()
    if args.symbols:
        env["CARGO_PROFILE_RELEASE_STRIP"] = "none"
    harness = hashlib.sha256()
    for folder in sorted([ROOT / "benches", *(ROOT / "crates").glob("*/benches")]):
        for path in sorted(folder.rglob("*")):
            if path.is_file() and path.suffix in (".rs", ".py", ".sh"):
                harness.update(str(path.relative_to(ROOT)).encode())
                harness.update(path.read_bytes())
    manifest = {
        "harness_sha256": harness.hexdigest(),
        "flags": {key: env.get(key) for key in ["RUSTFLAGS", "CARGO_ENCODED_RUSTFLAGS",
                  "CARGO_BUILD_TARGET", "CARGO_PROFILE_RELEASE_STRIP"]},
        "revision": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip(),
        "dirty": bool(subprocess.check_output(["git", "status", "--porcelain"], cwd=ROOT)),
        "rustc": subprocess.check_output(["rustc", "-Vv"], text=True),
        "machine": platform.platform(), "binaries": {}, "commands": [],
    }
    for name in dict.fromkeys(args.bench):
        package, target = TARGETS[name]
        command = ["cargo", "bench", "--locked", "--profile", "release", "-p", package,
                   "--bench", target, "--no-run", "--message-format=json"]
        manifest["commands"].append(command)
        result = subprocess.run(command, cwd=ROOT, env=env, text=True, stdout=subprocess.PIPE)
        executables = []
        for line in result.stdout.splitlines():
            message = json.loads(line)
            if message.get("reason") == "compiler-message":
                print(message["message"].get("rendered", ""), end="")
            if (message.get("reason") == "compiler-artifact"
                    and message["target"]["name"] == target
                    and "bench" in message["target"]["kind"] and message.get("executable")):
                executables.append(Path(message["executable"]))
        result.check_returncode()
        if len(executables) != 1:
            raise ValueError(f"Expected one benchmark artifact: {name}")
        dest = args.output / name
        shutil.copy2(executables[0], dest)
        manifest["binaries"][name] = digest(dest)
    (args.output / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")


if __name__ == "__main__":
    main()
