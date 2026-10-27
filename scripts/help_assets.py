"""Build and verify embedded help assets."""

import argparse
import fcntl
import hashlib
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import tomllib
import urllib.parse
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
INPUTS = (
    "docs/users",
    "scripts/help",
    "scripts/help_assets.py",
    "crates/vpdf-pdf/assets/DejaVuSansMono.ttf",
    "crates/ui/src",
    "crates/ui/examples/help_metadata.rs",
    "crates/ui/Cargo.toml",
    "Cargo.toml",
    "Cargo.lock",
    ".config/mise.toml",
)
OUTPUTS = ("base.pdf", "topics.json", "facts.json", "font-notices.txt")


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


def run(*args, **kwargs):
    return subprocess.check_output(args, text=True, timeout=180, **kwargs)


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
            "--quiet",
            "--locked",
            "-p",
            "vpdf-ui",
            "--example",
            "help_metadata",
            "--target",
            host,
            cwd=root,
        )
    )


def quote(text):
    return json.dumps(text, ensure_ascii=False)


def topic_map(facts):
    topics = {topic["id"]: topic for topic in facts["topics"]}
    if len(topics) != len(facts["topics"]):
        raise ValueError("Duplicate help topic ID")
    return topics


def inline(text, refs, topics):
    pattern = r"(`[^`]+`|\[\^[\w-]+\]|\[[^\]]+\]\[[\w-]+\])"
    pieces = []
    for token in re.split(pattern, text):
        if token.startswith("`"):
            pieces.append(f"code({quote(token[1:-1])})")
        elif token.startswith("[^"):
            name = token[2:-1]
            if "^" + name not in refs:
                raise ValueError("Missing help footnote")
            pieces.append(f"footnote(text({quote(refs['^' + name])}))")
        elif match := re.fullmatch(r"\[([^\]]+)\]\[([\w-]+)\]", token):
            title, name = match.groups()
            target = refs.get(name, "")
            if target.startswith("https://"):
                pieces.append(f"link({quote(target)}, text({quote(title)}))")
                continue
            if target.startswith("section:"):
                pieces.append(
                    f"local-link({quote(target[8:])}, {quote(title)})"
                )
                continue
            if not target.startswith("help:") or target[5:] not in topics:
                raise ValueError("Unknown help link destination")
            key = target[5:]
            kind = topics[key]["kind"]
            function = "local-link" if kind == "authored" else "reference"
            pieces.append(f"{function}({quote(key)}, {quote(title)})")
        else:
            if re.search(r"\[|\]|\*|\{\{|\}\}", token):
                raise ValueError("Unsupported help Markdown")
            pieces.append(f"text({quote(token)})")
    return " + ".join(pieces)


def continued(lines, index, indent):
    content = []
    while index < len(lines) and lines[index].startswith(" " * indent):
        content.append(lines[index].strip())
        index += 1
    return index, content


def references(text):
    refs, body = {}, []
    lines = text.splitlines()
    index = 0
    while index < len(lines):
        line = lines[index]
        index += 1
        if match := re.fullmatch(r"\[([\w^-]+)\]:\s*(.*)", line):
            name, target = match.groups()
            if name in refs:
                raise ValueError("Duplicate help reference")
            if name.startswith("^"):
                index, extra = continued(lines, index, 4)
                target = " ".join([target, *extra])
            elif not target and index < len(lines):
                target = lines[index].strip()
                index += 1
            if not name.startswith("^"):
                target = target.removeprefix("<").removesuffix(">")
            refs[name] = target
        else:
            body.append(line)
    return refs, "\n".join(body).strip()


def numbered(block):
    lines = block.splitlines()
    items, index = [], 0
    while index < len(lines):
        match = re.fullmatch(r"(\d+\.|-) (.+)", lines[index])
        if not match:
            raise ValueError("Unsupported help list continuation")
        index, extra = continued(lines, index + 1, len(match[1]) + 1)
        items.append(" ".join([match[2], *extra]))
    return (
        1 if lines[0].startswith("- ") else int(lines[0].split(".", 1)[0])
    ), items


def authored_heading(text):
    return re.sub(
        r"(?m)^# (.+)\n\n<!-- help-topic:([\w.-]+) -->",
        lambda match: f"# {match[1]} {{#{match[2]}}}",
        text,
    )


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


def chapter(path, facts, topics):
    examples = {}

    def protect(match):
        marker = f"\0example{len(examples)}\0"
        examples[marker] = match[0]
        return marker

    text = re.sub(r"(?m)^```[\w-]*\n[\s\S]*?\n```$", protect, path.read_text())
    text = authored_heading(text)
    overrides = dict(re.findall(r"<!-- help:([\w-]+)=([\w.-]+) -->", text))
    text = re.sub(r"<!-- help:[\w-]+=[\w.-]+ -->\n?", "", text)

    def keys(match):
        action, mode = match.groups()
        values = sorted(
            {
                row["keys"]
                for row in facts["defaults"]
                if row["action"] == action
                and row["mode"] == mode
                and row["preferred"]
            }
        )
        if not values:
            raise ValueError("Missing default help binding")
        return ", ".join(f"`{value}`" for value in values)

    text = re.sub(r"\{\{keys:([\w.-]+):([\w-]+)\}\}", keys, text)
    refs, body = references(text)
    for name, target in refs.items():
        if target.startswith("https://") or name.startswith("^"):
            continue
        file, _, anchor = target.partition("#")
        if file.endswith(".md") or (not file and anchor):
            destination = path.parent / file if file else path
            if destination.parent.resolve() != path.parent.resolve():
                refs[name] = repository_link(destination, anchor)
                continue
            source = authored_heading(destination.read_text())
            heading = re.search(r"^# .+ \{#([\w.-]+)\}$", source, re.M)
            if not heading or heading[1] not in topics:
                refs[name] = repository_link(destination, anchor)
                continue
            if anchor:
                headings = re.findall(r"^## (.+)$", source, re.M)
                if anchor not in [slug(title) for title in headings]:
                    raise ValueError("Unknown help section link")
                refs[name] = f"section:{heading[1]}.{anchor}"
            else:
                refs[name] = f"help:{heading[1]}"
        elif file and not target.startswith(("help:", "section:")):
            refs[name] = repository_link(path.parent / file, anchor)
    for name, topic in overrides.items():
        if name not in refs or topic not in topics:
            raise ValueError("Unknown help reference override")
        refs[name] = f"help:{topic}"
    for marker, example in examples.items():
        body = body.replace(marker, example)
    declared = []
    result = []
    for block in re.findall(r"```[^\n]*\n[\s\S]*?\n```|(?:[^\n]+\n?)+", body):
        block = block.strip()
        if match := re.fullmatch(r"# (.+) \{#([\w.-]+)\}", block):
            title, key = match.groups()
            if key not in topics or topics[key]["kind"] != "authored":
                raise ValueError("Unknown authored help topic")
            if title != topics[key]["title"]:
                raise ValueError("Help title differs from registry")
            declared.append(key)
            result.append(f"#topic({quote(key)}, {quote(title)})")
        elif block.startswith("## ") and "\n" not in block:
            if not declared:
                raise ValueError("Help section precedes chapter")
            label = declared[0] + "." + slug(block[3:])
            result.append(
                f"#part(text({quote(block[3:])})) #label({quote(label)})"
            )
        elif match := re.fullmatch(r"```[\w-]*\n([\s\S]*?)\n```", block):
            result.append(f"#code-block({quote(match[1])})")
        elif block.startswith("- "):
            _, items = numbered(block)
            result.append(
                "#list("
                + ", ".join(inline(item, refs, topics) for item in items)
                + ")"
            )
        elif re.match(r"\d+\. ", block):
            start, items = numbered(block)
            rows = [inline(item, refs, topics) for item in items]
            result.append(f"#enum(start: {start}, " + ", ".join(rows) + ")")
        else:
            if any(line.startswith("    ") for line in block.splitlines()):
                raise ValueError("Unsupported help indentation")
            result.append(
                "#(" + inline(" ".join(block.splitlines()), refs, topics) + ")"
            )
    if len(declared) != 1:
        raise ValueError("Chapter requires one stable help topic")
    return declared[0], "\n\n".join(result)


def slug(title):
    return re.sub(r"[^\w -]", "", title.lower()).replace(" ", "-")


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
    chapters = json.loads((stage / "book.json").read_bytes())["chapters"]
    if set(chapters) != {path.name for path in (stage / "users").glob("*.md")}:
        raise ValueError("User chapters differ from book order")
    expected = {
        key for key, topic in topics.items() if topic["kind"] == "authored"
    }
    declared, content = [], []
    for name in chapters:
        if Path(name).name != name or not name.endswith(".md"):
            raise ValueError("Invalid help chapter path")
        key, source = chapter(root / "docs/users" / name, facts, topics)
        declared.append(key)
        content.append(source)
    if set(declared) != expected or len(declared) != len(expected):
        raise ValueError("Authored help topics differ from registry")
    template = (stage / "manual.typ").read_text()
    if template.count("HELP_CHAPTERS") != 1:
        raise ValueError("Missing help chapter placeholder")
    (stage / "manual.typ").write_text(
        template.replace("HELP_CHAPTERS", "\n#pagebreak()\n".join(content))
    )
    args = ["--ignore-system-fonts", "--root", str(stage)]
    render_tool(
        "compile",
        *args,
        "--creation-timestamp",
        "0",
        str(stage / "manual.typ"),
        str(stage / "base.pdf"),
        env=env,
    )
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
    (stage / "font-notices.txt").write_text(notices)


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
    for needle in ("café", "Ελληνικά", "SIL OPEN FONT LICENSE", "Bitstream"):
        if needle not in text:
            raise ValueError("Missing help sample or notice text")


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
