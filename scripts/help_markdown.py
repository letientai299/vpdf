"""Resolve reader links in Pandoc's GFM tree."""

import hashlib
import json
import re
import shutil
import subprocess
import urllib.parse

MEDIA = ("docs/users", "docs/assets")


def pandoc(source, reader, writer):
    result = subprocess.run(
        [
            "pandoc",
            "--from",
            reader,
            "--to",
            writer,
            "--wrap=none",
            "--syntax-highlighting=idiomatic",
            "--fail-if-warnings",
        ],
        input=source,
        text=True,
        capture_output=True,
        timeout=180,
    )
    if result.returncode or result.stderr.strip():
        raise ValueError(f"Help Markdown diagnostics:\n{result.stderr.strip()}")
    return result.stdout


def parse(source, positions=False):
    reader = "gfm+attributes" + ("+sourcepos" if positions else "")
    return json.loads(pandoc(source, reader, "json"))


def nodes(value):
    if isinstance(value, list):
        for child in value:
            yield from nodes(child)
    elif isinstance(value, dict):
        yield value
        yield from nodes(value.get("c", []))


def plain(value):
    return "".join(
        node["c"][1]
        if node["t"] == "Code"
        else node["c"]
        if node["t"] == "Str"
        else " "
        for node in nodes(value)
        if node["t"] in {"Str", "Code", "Space", "SoftBreak", "LineBreak"}
    )


def declaration(document):
    declared = []
    for node in document["blocks"]:
        if node["t"] == "RawBlock":
            match = re.fullmatch(
                r"<!-- help-topic:([\w.-]+) -->\s*", node["c"][1]
            )
            if match:
                declared.append(match[1])
    if len(declared) != 1:
        raise ValueError("Section requires one stable help topic")
    return declared[0]


def typst(value, kind="RawInline"):
    return {"t": kind, "c": ["typst", value]}


def html_blocks(document):
    blocks = []
    for block in document["blocks"]:
        html = [
            node
            for node in nodes(block)
            if node["t"] in {"RawInline", "RawBlock"}
            and node["c"][0] == "html"
            and not node["c"][1].lstrip().startswith("<!--")
        ]
        if html:
            single = {**document, "blocks": [block]}
            rendered = pandoc(json.dumps(single), "json", "html5")
            blocks.extend(
                json.loads(pandoc(rendered, "html", "json"))["blocks"]
            )
        else:
            blocks.append(block)
    return blocks


def redirect_references(source, document, topics):
    overrides = {}
    for node in nodes(document["blocks"]):
        if node["t"] == "RawBlock" and node["c"][0] == "html":
            overrides.update(
                re.findall(r"<!-- help:([\w-]+)=([\w.-]+) -->", node["c"][1])
            )
    if not overrides:
        return document
    occupied = set()
    for node in nodes(parse(source, positions=True)["blocks"]):
        if node["t"] not in {
            "Div",
            "Span",
            "CodeBlock",
            "Header",
            "Link",
            "Image",
        }:
            continue
        attrs = node["c"][1] if node["t"] == "Header" else node["c"][0]
        for name, value in attrs[2]:
            if name != "data-pos":
                continue
            for first, last, column in re.findall(
                r"(\d+):\d+-(\d+):(\d+)", value
            ):
                occupied.update(
                    range(int(first), int(last) + (int(column) != 1))
                )
    replacements = []
    for name, topic in overrides.items():
        if topic not in topics:
            raise ValueError("Unknown help reference override")
        pattern = (
            r"(?mi)^ {0,3}(\["
            + re.escape(name)
            + r"\]:)[ \t]*(?:\n[ \t]*)?(?:<[^>\n]+>|\S+)"
        )
        matches = [
            match
            for match in re.finditer(pattern, source)
            if source.count("\n", 0, match.start()) + 1 not in occupied
        ]
        if not matches:
            raise ValueError("Unknown help reference override")
        replacements.extend(
            (match.start(), match.end(), match[1] + " help:" + topic)
            for match in matches
        )
    for start, end, replacement in sorted(replacements, reverse=True):
        source = source[:start] + replacement + source[end:]
    return parse(source)


def render(path, facts, topics, repository_link, stage=None, image_roots=()):
    source = path.read_text()
    document = parse(source)
    document = redirect_references(source, document, topics)
    key = declaration(document)
    if key not in topics or topics[key]["kind"] != "authored":
        raise ValueError("Unknown authored help topic")
    headers = [node for node in document["blocks"] if node["t"] == "Header"]
    if not headers or headers[0]["c"][0] != 1:
        raise ValueError("Help section precedes title")
    title = headers[0]
    if plain(title["c"][2]) != topics[key]["title"]:
        raise ValueError("Help title differs from registry")
    if sum(node["c"][0] == 1 for node in headers) != 1:
        raise ValueError("Section requires one title")

    def destination(target):
        if target.startswith("help:"):
            name = target[5:]
            if name not in topics:
                raise ValueError("Unknown help link destination")
            return name, topics[name]["kind"] == "authored"
        url = urllib.parse.urlsplit(target)
        if url.scheme:
            if url.scheme not in {"https", "http", "mailto"}:
                raise ValueError("Unsupported help link scheme")
            return target, None
        file = urllib.parse.unquote(url.path)
        anchor = urllib.parse.unquote(url.fragment)
        other = (path.parent / file).resolve() if file else path.resolve()
        if other.parent == path.parent.resolve() and other.suffix == ".md":
            tree = (
                document
                if other == path.resolve()
                else parse(other.read_text())
            )
            topic = declaration(tree)
            if topic in topics:
                if anchor:
                    ids = {
                        node["c"][1][0]
                        for node in nodes(tree["blocks"])
                        if node["t"] == "Header"
                    }
                    if anchor not in ids:
                        raise ValueError("Unknown help section link")
                    return topic + "." + anchor, True
                return topic, True
        return repository_link(other, anchor), None

    def transform(value):
        if isinstance(value, list):
            return [transform(child) for child in value]
        if not isinstance(value, dict) or "t" not in value:
            return value
        kind = value["t"]
        if kind in {"AlignDefault", "AlignCenter"}:
            return {"t": "AlignLeft"}
        if kind == "Code":
            return typst(
                "#code(" + json.dumps(value["c"][1], ensure_ascii=False) + ")"
            )
        if kind == "CodeBlock":
            return value
        if kind == "Str":
            text = value["c"]
            matches = list(
                re.finditer(r"\{\{keys:([\w.-]+):([\w-]+)\}\}", text)
            )
            if not matches:
                if "{{" in text or "}}" in text:
                    raise ValueError("Unresolved help template")
                return value
            parts, cursor = [], 0
            for match in matches:
                action, mode = match.groups()
                keys = sorted(
                    {
                        row["keys"]
                        for row in facts["defaults"]
                        if row["action"] == action
                        and row["mode"] == mode
                        and row["preferred"]
                    }
                )
                if not keys:
                    raise ValueError("Missing default help binding")
                parts.append({"t": "Str", "c": text[cursor : match.start()]})
                for index, shortcut in enumerate(keys):
                    if index:
                        parts.append({"t": "Str", "c": ", "})
                    parts.append(typst("#code(" + json.dumps(shortcut) + ")"))
                cursor = match.end()
            parts.append({"t": "Str", "c": text[cursor:]})
            return {"t": "Span", "c": [["", [], []], parts]}
        if kind in {"RawBlock", "RawInline"} and value["c"][0] == "html":
            if value["c"][1].strip().startswith("<!--"):
                return typst("", kind)
            raise ValueError("Unconverted help HTML")
        if kind == "Header":
            level, attrs, children = value["c"]
            if value is title:
                return typst(
                    f"#topic({json.dumps(key)}, {json.dumps(topics[key]['title'])})",
                    "RawBlock",
                )
            return {
                "t": kind,
                "c": [
                    level,
                    [key + "." + attrs[0], *attrs[1:]],
                    transform(children),
                ],
            }
        if kind == "Link":
            attrs, children, (target, tooltip) = value["c"]
            target, local = destination(target)
            if local is not None:
                children = [*children, {"t": "Str", "c": " |" + target + "|"}]
                target = ("#" if local else "vpdf-help:") + target
            return {
                "t": kind,
                "c": [attrs, transform(children), [target, tooltip]],
            }
        if kind == "Image":
            attrs, children, (target, tooltip) = value["c"]
            image = (path.parent / urllib.parse.unquote(target)).resolve()
            repository_link(image, "")
            if not any(
                image.is_relative_to(folder.resolve()) for folder in image_roots
            ):
                raise ValueError("Help image outside inventoried folders")
            if stage:
                name = (
                    "images/"
                    + hashlib.sha256(image.read_bytes()).hexdigest()
                    + image.suffix
                )
                (stage / "images").mkdir(exist_ok=True)
                shutil.copyfile(image, stage / name)
                target = name
            return {
                "t": kind,
                "c": [attrs, transform(children), [target, tooltip]],
            }
        return {**value, "c": transform(value.get("c", []))}

    document["blocks"] = [transform(block) for block in html_blocks(document)]
    return key, pandoc(json.dumps(document), "json", "typst")
