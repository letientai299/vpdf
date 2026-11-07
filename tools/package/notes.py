"""Render release notes from Conventional Commits since the last stable tag."""

import argparse
import re
import subprocess
import sys
import tomllib
from pathlib import Path

STABLE = re.compile(r"v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)")
SUBJECT = re.compile(
    r"(?P<kind>[a-z]+)(?:\([^)]*\))?(?P<breaking>!)?: (?P<text>.+)"
)
REVERTED = re.compile(r"^This reverts commit ([0-9a-f]{40})", re.MULTILINE)
BREAKING = "Breaking changes"
SECTIONS = dict(feat="Features", fix="Fixes", perf="Performance")
# Notes skip commits that change none of what users get.
SHIPPED = (
    "src",
    "crates",
    "Cargo.toml",
    "Cargo.lock",
    ":(glob)tools/build/*.rs",
    ":(glob)tools/build/*.sh",
    "tools/help",
    "tools/package",
    "docs/users",
    "docs/assets",
    "docs/licenses",
    ":(exclude)crates/vpdf-dev",
    ":(exclude,glob)**/tests/**",
    ":(exclude,glob)**/tests.rs",
    ":(exclude,glob)**/test_*.py",
    ":(exclude,glob)**/benches/**",
)


def git(root, *args):
    return subprocess.check_output(["git", *args], cwd=root, text=True)


def repository(root):
    package = tomllib.loads((root / "Cargo.toml").read_text())["package"]
    return package["repository"]


def core(tag):
    match = STABLE.fullmatch(tag)
    if not match:
        raise ValueError(f"Stable release tag required: {tag}")
    return tuple(map(int, match.groups()))


def end(root, tag):
    found = subprocess.run(
        ["git", "rev-parse", "--verify", "--quiet", f"refs/tags/{tag}"],
        cwd=root,
        capture_output=True,
    )
    if found.returncode == 0:
        return tag
    print(f"Previewing HEAD as untagged {tag}", file=sys.stderr)
    return "HEAD"


def previous(root, tag):
    tags = git(root, "tag", "--list", "v*").split()
    older = [t for t in tags if STABLE.fullmatch(t) and core(t) < core(tag)]
    return max(older, key=core, default=None)


def commits(root, start, head):
    if git(root, "rev-parse", "--is-shallow-repository").strip() == "true":
        raise RuntimeError("Release notes need full history")
    # Patch-equivalent copies of commits under a rewritten tag drop out.
    span = ["--cherry-pick", "--right-only", f"{start}...{head}"]
    log = git(
        root,
        "log",
        "-z",
        "--reverse",
        "--no-merges",
        "--format=%H%n%s%n%b",
        *(span if start else [head]),
        "--",
        *SHIPPED,
    )
    # Git forbids NUL in messages, so it safely ends each record.
    for record in filter(None, log.split("\0")):
        identity, subject, body = (record + "\n").split("\n", 2)
        yield identity, subject, body


def groups(entries):
    entries = list(entries)
    inside = {identity for identity, _, _ in entries}
    # A revert and its target cancel out within one release.
    dropped = set()
    for identity, _, body in entries:
        if targets := set(REVERTED.findall(body)) & inside:
            dropped |= targets | {identity}
    result = {title: [] for title in (BREAKING, *SECTIONS.values())}
    for identity, subject, body in entries:
        match = identity not in dropped and SUBJECT.fullmatch(subject)
        if not match:
            continue
        text = match["text"].strip()
        text = text[:1].upper() + text[1:]
        if match["breaking"] or re.search(
            r"^BREAKING[ -]CHANGE:", body, re.MULTILINE
        ):
            result[BREAKING].append(text)
        elif match["kind"] in SECTIONS:
            result[SECTIONS[match["kind"]]].append(text)
    return result


def render(root, tag):
    core(tag)
    head = end(root, tag)
    start = previous(root, tag)
    lines = [f"# Vpdf {tag}"]
    for title, items in groups(commits(root, start, head)).items():
        if items:
            lines += ["", f"## {title}", "", *(f"- {item}" for item in items)]
    if len(lines) == 1:
        lines += ["", "No user-facing changes."]
    if start:
        compare = f"{repository(root)}/compare/{start}...{tag}"
        lines += [
            "",
            f"[Changes since {start}][compare]",
            "",
            f"[compare]: {compare}",
        ]
    return "\n".join(lines)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("tag", help="stable tag such as v1.2.3")
    args = parser.parse_args()
    try:
        print(render(Path.cwd(), args.tag))
    except (ValueError, RuntimeError) as error:
        parser.exit(1, f"{error}\n")


if __name__ == "__main__":
    main()
