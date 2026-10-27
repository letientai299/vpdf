"""Build and verify embedded help assets."""

import argparse
import fcntl
import hashlib
import json
import os
import shutil
import subprocess
import sys
import tempfile
import tomllib
import urllib.parse
from pathlib import Path

import help_markdown

ROOT = Path(__file__).resolve().parent.parent
INPUTS = (
    *help_markdown.MEDIA,
    "scripts/help",
    "docs/licenses",
    "LICENSE",
    "LICENSE-AGPL",
    "scripts/help_assets.py",
    "crates/ui/assets",
    "scripts/help_markdown.py",
    "scripts/help_font.py",
    "crates/ui/src",
    "crates/ui/examples/help_metadata.rs",
    "crates/ui/Cargo.toml",
    "Cargo.toml",
    "Cargo.lock",
    ".config/mise.toml",
)
OUTPUTS = ("base.pdf", "topics.json", "facts.json", "third-party-notices.txt")


def encode(value):
    return (
        json.dumps(value, sort_keys=True, ensure_ascii=False) + "\n"
    ).encode()


def digest(data):
    return hashlib.sha256(data).hexdigest()


def inventory(root):
    paths = []
    for name in INPUTS:
        path = root / name
        if not path.exists():
            raise ValueError("Missing help build input")
        paths.extend(sorted(path.rglob("*")) if path.is_dir() else [path])
    return {
        path.relative_to(root).as_posix(): digest(path.read_bytes())
        for path in paths
        if path.is_file()
    }


def identity(root, facts):
    return {
        "schema": 1,
        "inputs": inventory(root),
        "facts": digest(encode(facts)),
    }


def run(*args, timeout=180, **kwargs):
    return subprocess.check_output(args, text=True, timeout=timeout, **kwargs)


def render_tool(*args, env):
    result = subprocess.run(
        ["typst", *args], env=env, capture_output=True, text=True, timeout=180
    )
    if result.returncode or result.stderr.strip():
        raise ValueError(f"Help renderer diagnostics:\n{result.stderr.strip()}")
    return result.stdout


def export_facts(root):
    host = run("rustc", "--print", "host-tuple").strip()
    return json.loads(
        run(
            "cargo",
            "run",
            "--locked",
            "-p",
            "vpdf-ui",
            "--example",
            "help_metadata",
            "--target",
            host,
            cwd=root,
            timeout=600,
        )
    )


def topic_map(facts):
    topics = {topic["id"]: topic for topic in facts["topics"]}
    if len(topics) != len(facts["topics"]):
        raise ValueError("Duplicate help topic ID")
    return topics


def repository_link(destination, anchor):
    destination = destination.resolve()
    try:
        relative = destination.relative_to(ROOT.resolve())
    except ValueError as error:
        raise ValueError("Help link leaves the repository") from error
    if not destination.is_file() and not destination.is_dir():
        raise ValueError("Missing repository link target")
    package = tomllib.loads((ROOT / "Cargo.toml").read_text())["package"]
    repository = package["repository"].rstrip("/")
    kind = "tree" if destination.is_dir() else "blob"
    target = (
        f"{repository}/{kind}/main/{urllib.parse.quote(relative.as_posix())}"
    )
    return target + ("#" + urllib.parse.quote(anchor) if anchor else "")


def chapter(path, facts, topics, stage=None):
    return help_markdown.render(
        path,
        facts,
        topics,
        repository_link,
        stage,
        image_roots=tuple(ROOT / name for name in help_markdown.MEDIA),
    )


def verify(directory, expected):
    manifest = json.loads((directory / "manifest.json").read_bytes())
    if manifest["identity"] != expected:
        raise ValueError("Stale help assets")
    if manifest["fingerprint"] != digest(encode(expected)):
        raise ValueError("Help fingerprint mismatch")
    if set(manifest["outputs"]) != set(OUTPUTS):
        raise ValueError("Incomplete help assets")
    for name, value in manifest["outputs"].items():
        if digest((directory / name).read_bytes()) != value:
            raise ValueError("Help output hash mismatch")
    return manifest


def render(root, stage, facts):
    pin = json.loads((root / "scripts/help/renderer.json").read_bytes())
    env = {k: v for k, v in os.environ.items() if not k.startswith("TYPST_")}
    markdown_version = run("pandoc", "--version").splitlines()[0]
    if markdown_version != pin["markdown"]:
        raise ValueError("Help parser differs from pin; run mise install")
    version = render_tool("--version", env=env).strip()
    if version != pin["version"]:
        raise ValueError("Help renderer differs from pin; run mise install")
    for path in (root / "scripts/help").iterdir():
        if path.is_dir():
            shutil.copytree(path, stage / path.name)
        else:
            shutil.copyfile(path, stage / path.name)
    shutil.copytree(root / "docs/users", stage / "users")
    topics = topic_map(facts)
    chapters = json.loads((stage / "contents.json").read_bytes())["sections"]
    if set(chapters) != {path.name for path in (stage / "users").glob("*.md")}:
        raise ValueError("User sections differ from help order")
    expected = {
        key for key, topic in topics.items() if topic["kind"] == "authored"
    }
    declared, content = [], []
    for name in chapters:
        if Path(name).name != name or not name.endswith(".md"):
            raise ValueError("Invalid help chapter path")
        key, source = chapter(root / "docs/users" / name, facts, topics, stage)
        declared.append(key)
        content.append(source)
    if set(declared) != expected or len(declared) != len(expected):
        raise ValueError("Authored help topics differ from registry")
    legal = help_markdown.parse((stage / "legal.md").read_text())
    (stage / "legal.typ").write_text(
        help_markdown.pandoc(json.dumps(legal), "json", "typst")
    )
    template = (stage / "manual.typ").read_text()
    if template.count("HELP_CHAPTERS") != 1:
        raise ValueError("Missing help chapter placeholder")
    (stage / "manual.typ").write_text(
        template.replace("HELP_CHAPTERS", "\n\n".join(content))
    )
    shutil.copytree(root / "crates/ui/assets", stage / "fonts")
    for name, expected in pin["font_files"].items():
        if digest((stage / "fonts" / name).read_bytes()) != expected:
            raise ValueError("Bundled help font hash mismatch")
    args = [
        "--ignore-system-fonts",
        "--font-path",
        str(stage / "fonts"),
        "--root",
        str(stage),
    ]
    render_tool(
        "compile",
        *args,
        "--creation-timestamp",
        "0",
        str(stage / "manual.typ"),
        str(stage / "base.pdf"),
        env=env,
    )
    compressed = stage / "compressed.pdf"
    run(
        "qpdf",
        "--object-streams=generate",
        "--recompress-flate",
        "--compression-level=9",
        "--deterministic-id",
        str(stage / "base.pdf"),
        str(compressed),
    )
    compressed.replace(stage / "base.pdf")
    rows = json.loads(
        render_tool(
            "eval",
            *args,
            "--in",
            str(stage / "manual.typ"),
            "query(<help-topic>).map(it => it.value)",
            env=env,
        )
    )
    if [row["id"] for row in rows] != declared:
        raise ValueError("Rendered help destinations differ")
    manifest = {
        "schema": 1,
        "authored": [{**topics[row["id"]], **row} for row in rows],
        "dynamic": [
            topic for topic in facts["topics"] if topic["kind"] != "authored"
        ],
    }
    (stage / "topics.json").write_bytes(encode(manifest))
    (stage / "facts.json").write_bytes(encode(facts))
    notices = "\n\n".join(
        path.read_text() for path in sorted((stage / "notices").iterdir())
    )
    notices += "\n\n" + "\n\n".join(
        path.read_text()
        for path in [
            root / "LICENSE",
            root / "LICENSE-AGPL",
            *sorted((root / "docs/licenses").glob("*.txt")),
        ]
    )
    (stage / "third-party-notices.txt").write_text(notices)


def generation(root, output, facts):
    expected = identity(root, facts)
    return output / digest(encode(expected)), expected


def check_assets(root, output, facts):
    directory, expected = generation(root, output, facts)
    verify(directory, expected)
    if identity(root, facts) != expected:
        raise ValueError("Help inputs changed during verification")
    return directory


def prepare(root, output, facts):
    output.mkdir(parents=True, exist_ok=True)
    with (output / ".lock").open("a") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        directory, expected = generation(root, output, facts)
        if directory.exists():
            return check_assets(root, output, facts)
        with tempfile.TemporaryDirectory(
            prefix=".generate-", dir=output
        ) as temporary:
            stage = Path(temporary)
            render(root, stage, facts)
            if identity(root, facts) != expected:
                raise ValueError("Help inputs changed during generation")
            manifest = {
                "identity": expected,
                "fingerprint": directory.name,
                "outputs": {
                    name: digest((stage / name).read_bytes())
                    for name in OUTPUTS
                },
            }
            (stage / "manifest.json").write_bytes(encode(manifest))
            verify(stage, expected)
            os.rename(stage, directory)
    return directory


def check_pdf(directory):
    pdf = str(directory / "base.pdf")
    run("qpdf", "--check", pdf)
    fonts = run("pdffonts", pdf).splitlines()[2:]
    if not fonts or any(
        row.split()[-5] != "yes" or row.split()[-3] != "yes" for row in fonts
    ):
        raise ValueError("Help font embedding or Unicode map missing")
    text = run("pdftotext", "-enc", "UTF-8", pdf, "-")
    topics = json.loads((directory / "topics.json").read_bytes())
    for title in [row["title"] for row in topics["authored"]]:
        if title not in text:
            raise ValueError("Missing help chapter text")
    for needle in (
        "café",
        "Ελληνικά",
        "SIL Open Font License",
        "Renzhi Li",
        "MuPDF",
        "AGPL",
    ):
        if needle not in text:
            raise ValueError("Missing help practice or notice text")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true")
    parser.add_argument("--facts", type=Path)
    parser.add_argument("--verify-pdf", action="store_true")
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    facts = (
        json.loads(args.facts.read_bytes())
        if args.facts
        else export_facts(ROOT)
    )
    output = (
        args.output
        or Path(os.environ.get("CARGO_TARGET_DIR", ROOT / "target")) / "help"
    )
    action = check_assets if args.check else prepare
    directory = action(ROOT, output.resolve(), facts)
    if args.verify_pdf:
        check_pdf(directory)
    print(directory)


if __name__ == "__main__":
    try:
        main()
    except (OSError, ValueError, KeyError, subprocess.SubprocessError) as error:
        print(
            f"Help assets unavailable: {error}\nRun: mise run help:build",
            file=sys.stderr,
        )
        sys.exit(1)
