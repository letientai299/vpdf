#!/usr/bin/env python3
"""Capture real Kitty graphics for the gallery."""

import argparse
import fcntl
import hashlib
import json
import math
import platform
import re
import shutil
import struct
import subprocess
import tempfile
import termios
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
ENCODING = "JPEG; quality 92; 4:4:4; progressive; optimized Huffman coding"
SCENES = {
    "overview": (
        2,
        ["paper"],
        [
            ("open", ("vsplit", "book")),
            ("key", "ctrl+w"),
            ("text", "L"),
            ("text", "zc=+59gg/http\r"),
            ("wait", "matches"),
            ("text", "gs"),
            ("wait", "RESULTS"),
            ("key", "tab"),
            ("text", "16j"),
            ("open", ("split", "scan")),
            ("key", "ctrl+w"),
            ("text", "x"),
            ("text", "=3+7ggv"),
            ("wait", "Selection unavailable"),
            ("text", ":ocr eng\r"),
            ("wait", "OCR ready"),
            ("text", "i}vipzt"),
            ("wait", "VISUAL"),
            ("key", "ctrl+y"),
            ("key", "ctrl+y"),
            ("key", "ctrl+y"),
            ("key", "ctrl+y"),
            ("key", "ctrl+w"),
            ("text", "h"),
            ("text", "zc=3+2gggo"),
            ("wait", "CONTENTS"),
            ("key", "tab"),
            ("text", "/[13]\r"),
            ("wait", "matches"),
            ("text", ":nohlsearch\rilK"),
            ("wait", "╭"),
        ],
    ),
    "help": (
        1,
        ["paper"],
        [
            ("text", ":help navigation\r"),
            ("wait", "Help"),
            ("text", "go"),
            ("wait", "CONTENTS"),
        ],
    ),
    "search": (
        2,
        ["paper"],
        [
            ("text", "zc=+/multi-head\r"),
            ("wait", "matches"),
            ("text", "gs"),
            ("wait", "RESULTS"),
        ],
    ),
    "preview": (
        2,
        ["paper"],
        [
            ("text", "zc=+/[13]\r"),
            ("wait", "matches"),
            ("text", ":nohlsearch\rilK"),
            ("wait", "╭"),
        ],
    ),
    "figure": (
        3,
        ["paper"],
        [
            ("text", "]f[fzFgF"),
            ("wait", "Save PNG"),
            ("hover", "Copy text"),
        ],
    ),
    "mouse-menu": (
        2,
        ["paper"],
        [
            ("text", "=+2gg"),
            ("mouse", ("left-down", 600, 510)),
            ("mouse", ("left-drag", 1700, 690)),
            ("mouse", ("left-up", 1700, 690)),
            ("wait", "VISUAL"),
            ("mouse", ("right-down", 700, 520)),
            ("mouse", ("right-up", 700, 520)),
            ("wait", "Copy as image"),
            ("hover", "Search for selection"),
        ],
    ),
    "mouse-hints": (
        2,
        ["paper"],
        [
            ("text", "zc=+2ggf"),
            ("wait", "HINT"),
            ("hover", "a"),
        ],
    ),
    "mouse-region": (
        3,
        ["paper"],
        [
            ("text", "]f[fzF"),
            ("mouse", ("alt-down", 900, 150)),
            ("mouse", ("alt-drag", 1650, 1250)),
            ("wait", "Release captures; Esc cancels"),
        ],
    ),
    "pages": (
        64,
        ["book", "paper"],
        [
            ("text", "zfgp"),
            ("wait", "PAGES"),
        ],
    ),
    "recent": (
        2,
        ["paper", "book", "scan"],
        [
            ("text", "gt64gg"),
            ("text", "gt"),
            ("text", "gt3gg"),
            ("text", ":recent\r"),
            ("wait", "RECENT"),
            ("wait", "Metadata"),
            ("hover", "scan.pdf"),
            ("wait", "49"),
        ],
    ),
    "jumps": (
        2,
        ["paper"],
        [
            ("text", "zc=+2gg]]]]]]"),
            ("text", "/multi-head\r"),
            ("wait", "matches"),
            ("text", "nn"),
            ("key", "ctrl+o"),
            ("text", ":jumps\r"),
            ("wait", "JUMPS"),
            ("wait", "Heading:"),
            ("wait", "Search ["),
        ],
    ),
    "ocr": (
        7,
        ["scan"],
        [
            ("text", "v"),
            ("wait", "Selection unavailable"),
            ("text", "=3+7gg"),
            ("text", ":vsplit\r"),
            ("text", "7gg:ocr eng\r"),
            ("wait", "OCR ready"),
            ("text", ":text\r"),
            ("text", "i}vip"),
            ("wait", "VISUAL"),
        ],
    ),
}


def run(*args):
    return subprocess.check_output(args, text=True, cwd=ROOT).rstrip()


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def scene_documents(name):
    _, documents, steps = SCENES[name]
    opened = [value[1] for kind, value in steps if kind == "open"]
    return list(dict.fromkeys([*documents, *opened]))


def input_hashes(corpus, selected):
    names = sorted({doc for name in selected for doc in scene_documents(name)})
    return {
        name: {"file": f"{name}.pdf", "sha256": digest(corpus / f"{name}.pdf")}
        for name in names
    }


def binary_identity(binary, profile):
    version = next(
        line
        for line in run(str(binary), "--version").splitlines()
        if line.startswith("vpdf ")
    )
    match = re.fullmatch(
        r"vpdf ([0-9a-f]{7,40}) \(\d{4}-\d{2}-\d{2}\)", version
    )
    commit = None
    if match:
        commit = match[1]
        resolved = subprocess.run(
            ["git", "rev-parse", "--verify", f"{commit}^{{commit}}"],
            cwd=ROOT,
            text=True,
            capture_output=True,
            check=False,
        )
        if resolved.returncode == 0:
            commit = resolved.stdout.strip()
    return {
        "commit": commit,
        "binary_version": version,
        "build_profile": profile,
        "binary_sha256": digest(binary),
    }


def capture_identity(args, options, ocr, inputs):
    return {
        "captured_at": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        **binary_identity(args.binary, args.build_profile),
        "platform": platform.platform(),
        "machine": run("sysctl", "-n", "machdep.cpu.brand_string")
        if platform.system() == "Darwin"
        else platform.machine(),
        "kitty": run(args.kitty, "--version"),
        "kitty_options": options,
        "font_scale": args.scale,
        "base_font_points": args.base_font,
        "dimensions": [2560, 1440],
        "capture": "Kitty screenshot API; graphics included; no resizing",
        "cache_conditions": "Fresh reader and private saved state per scene",
        "encoding": ENCODING,
        "ocr": ocr,
        "inputs": inputs,
    }


def capture_key(identity):
    return hashlib.sha256(
        json.dumps(identity, sort_keys=True).encode()
    ).hexdigest()[:16]


def load_manifest(path):
    if not path.exists():
        return {"captures": {}, "scenes": {}}
    manifest = json.loads(path.read_text())
    if "captures" in manifest:
        return manifest
    previous = {
        key: value for key, value in manifest.items() if key != "scenes"
    }
    old_id = capture_key(previous)
    return {
        "captures": {old_id: previous},
        "scenes": {
            name: {**scene, "capture": old_id}
            for name, scene in manifest["scenes"].items()
        },
    }


def verify_inputs(args, identity):
    if digest(args.binary) != identity["binary_sha256"]:
        raise RuntimeError("Capture executable changed")
    for name, info in identity["inputs"].items():
        if digest(args.corpus / info["file"]) != info["sha256"]:
            raise RuntimeError(f"Source PDF changed: {name}")


def publish_scene(output, manifest, identity, name, scene, image):
    capture_id = capture_key(identity)
    scenes = {**manifest["scenes"], name: {**scene, "capture": capture_id}}
    used = {record["capture"] for record in scenes.values()}
    captures = {**manifest["captures"], capture_id: identity}
    updated = {
        "captures": {
            key: value for key, value in captures.items() if key in used
        },
        "scenes": scenes,
    }
    target = output / scene["image"]
    with tempfile.TemporaryDirectory(
        prefix=".capture-", dir=output
    ) as directory:
        directory = Path(directory)
        staged = directory / scene["image"]
        shutil.copyfile(image, staged)
        report = directory / "capture.json"
        report.write_text(json.dumps(updated, indent=2) + "\n")
        backup = directory / "previous.jpg"
        if target.exists():
            shutil.copyfile(target, backup)
        installed = False
        try:
            staged.replace(target)
            installed = True
            report.replace(output / "capture.json")
        except BaseException:
            if installed:
                if backup.exists():
                    backup.replace(target)
                else:
                    target.unlink()
            raise
    return updated


class Gallery:
    def __init__(self, args, directory):
        self.args = args
        self.directory = Path(directory)
        self.socket = "unix:" + str(self.directory / "kitty.sock")
        self.window = None

    def remote(self, *args):
        return run(self.args.kitten, "@", "--to", self.socket, *args)

    def wait(self, expected):
        deadline = time.monotonic() + 30
        while time.monotonic() < deadline:
            text = self.remote("get-text", "--match", f"id:{self.window}")
            if expected in text:
                return
            time.sleep(0.2)
        (self.args.raw / "failed-state.txt").write_text(text + "\n")
        raise RuntimeError(f"Reader state missing: {expected}")

    def pointer(self, event, x, y):
        buttons = {
            "move": 35,
            "left-down": 0,
            "left-up": 0,
            "left-drag": 32,
            "right-down": 2,
            "right-up": 2,
            "right-drag": 34,
            "alt-down": 8,
            "alt-drag": 40,
            "alt-up": 8,
        }
        end = "m" if event.endswith("-up") else "M"
        report = f"\x1b[<{buttons[event]};{int(x)};{int(y)}{end}"
        self.remote("send-text", "--match", f"id:{self.window}", report)

    def hover(self, label):
        windows = json.loads(self.remote("ls"))
        window = next(
            window
            for os_window in windows
            for tab in os_window["tabs"]
            for window in tab["windows"]
            if str(window["id"]) == self.window
        )
        tty = run("ps", "-p", str(window["pid"]), "-o", "tty=").strip()
        with open("/dev/" + tty, "rb") as terminal:
            size = fcntl.ioctl(terminal, termios.TIOCGWINSZ, bytes(8))
        rows, columns, width, height = struct.unpack("HHHH", size)
        cell = (width // columns, height // rows)
        if not all(cell):
            raise RuntimeError("Mouse capture requires pixel cell dimensions")
        screen = self.remote("get-text", "--match", f"id:{self.window}")
        for row, line in enumerate(screen.splitlines()):
            match = re.search(r"(?<!\w)" + re.escape(label) + r"(?!\w)", line)
            if match:
                self.pointer(
                    "move",
                    (match.start() + 0.5) * cell[0],
                    (row + 0.5) * cell[1],
                )
                return
        raise RuntimeError(f"Pointer label missing: {label}")

    def capture(self, name):
        page, documents, steps = SCENES[name]
        previous = self.window
        self.window = self.remote(
            "launch",
            "--type",
            "tab",
            "--cwd",
            str(ROOT),
            "--env",
            f"XDG_STATE_HOME={self.directory / name}",
            "--env",
            f"VPDF_TRACE={self.args.raw / (name + '.jsonl')}",
            str(self.args.binary),
            "-u",
            "NONE",
            *(
                (self.args.corpus / f"{doc}.pdf").resolve().as_uri()
                + (f"#page={page}" if index == 0 else "")
                for index, doc in enumerate(documents)
            ),
        )
        if previous:
            self.remote("close-window", "--match", f"id:{previous}")
        self.wait("NORMAL")
        for kind, value in steps:
            if kind == "wait":
                self.wait(value)
                continue
            if kind == "mouse":
                self.pointer(*value)
            elif kind == "hover":
                self.hover(value)
            elif kind == "open":
                command, document = value
                self.remote(
                    "send-text",
                    "--match",
                    f"id:{self.window}",
                    f":{command} {self.args.corpus / (document + '.pdf')}\r",
                )
            else:
                command = "send-text" if kind == "text" else "send-key"
                self.remote(
                    command,
                    "--match",
                    f"id:{self.window}",
                    value.replace("{corpus}", str(self.args.corpus)),
                )
            time.sleep(0.3)

        raw = self.args.raw / f"{name}.png"
        # Wait for raster work and animations.
        time.sleep(1)
        previous_hash, stable = None, 0
        deadline = time.monotonic() + 30
        while stable < 3:
            if time.monotonic() > deadline:
                raise RuntimeError(f"Screenshot never settled: {name}")
            self.remote(
                "screenshot",
                "--match-tab",
                f"window_id:{self.window}",
                str(raw),
            )
            current = digest(raw)
            stable = stable + 1 if current == previous_hash else 0
            previous_hash = current
            time.sleep(0.4)
        screen = self.remote("get-text", "--match", f"id:{self.window}")
        (self.args.raw / f"{name}.txt").write_text(screen + "\n")
        target = self.directory / f"{name}.jpg"
        run("sh", str(ROOT / "scripts/demo-optimize.sh"), str(raw), str(target))
        print(f"Captured {name}: {target.stat().st_size} bytes", flush=True)
        scene = {
            "image": target.name,
            "encoding": ENCODING,
            "sha256": digest(target),
            "bytes": target.stat().st_size,
            "start_page": page,
            "documents": scene_documents(name),
            "launch_documents": documents,
            "steps": steps,
        }
        return scene, target

    def close(self):
        if not hasattr(self, "process"):
            return
        self.process.terminate()
        try:
            self.process.wait(timeout=10)
        except subprocess.TimeoutExpired:
            self.process.kill()
            self.process.wait()

    def start(self, log):
        font_size = self.args.base_font * self.args.scale
        options = {
            "allow_remote_control": "socket-only",
            "initial_window_width": "2560",
            "initial_window_height": "1440",
            "remember_window_size": "no",
            "hide_window_decorations": "yes",
            "tab_bar_style": "hidden",
            "window_padding_width": "0",
            "font_family": self.args.font,
            "font_size": str(font_size),
            "cursor_blink_interval": "0",
            "background": "#181b24",
            "foreground": "#d8dee9",
            "color0": "#20242f",
            "color1": "#bf616a",
            "color2": "#a3be8c",
            "color3": "#ebcb8b",
            "color4": "#81a1c1",
            "color5": "#b48ead",
            "color6": "#88c0d0",
            "color7": "#e5e9f0",
            "color8": "#4c566a",
            "color12": "#88c0d0",
        }
        command = [
            self.args.kitty,
            "--config",
            "NONE",
            "--listen-on",
            self.socket,
            "--start-as",
            "normal",
        ]
        for key, value in options.items():
            command.extend(["-o", f"{key}={value}"])
        command.extend(["/bin/sleep", "3600"])
        self.process = subprocess.Popen(
            command, cwd=ROOT, stdout=log, stderr=log
        )
        deadline = time.monotonic() + 30
        while not (self.directory / "kitty.sock").exists():
            if self.process.poll() is not None or time.monotonic() > deadline:
                raise RuntimeError("Kitty startup failed; inspect kitty.log")
            time.sleep(0.2)
        # Never persist environments from ls.
        windows = json.loads(self.remote("ls"))
        self.window = str(windows[0]["tabs"][0]["windows"][0]["id"])
        return options


def parse_args(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--binary", type=Path, default=ROOT / "target/release/vpdf"
    )
    parser.add_argument("--build-profile", default="unknown")
    parser.add_argument("--corpus", type=Path, default=ROOT / "tmp/corpus")
    parser.add_argument("--output", type=Path, default=ROOT / "docs/assets")
    parser.add_argument("--raw", type=Path, default=ROOT / "tmp/demo")
    parser.add_argument("--kitty", default=shutil.which("kitty"))
    parser.add_argument("--kitten", default=shutil.which("kitten"))
    parser.add_argument("--font", default="monospace")
    parser.add_argument("--base-font", type=float, default=13)
    parser.add_argument("--scale", type=float, default=2)
    parser.add_argument(
        "--scene",
        choices=SCENES,
        action="append",
        help="Recapture this scene; repeat for several; omit for all",
    )
    parser.add_argument("--list-scenes", action="store_true")
    args = parser.parse_args(argv)
    for name in ("binary", "corpus", "output", "raw"):
        setattr(args, name, getattr(args, name).resolve())
    return args


def preflight(args, selected):
    if not args.kitty or not args.kitten or not shutil.which("magick"):
        raise RuntimeError("Kitty, kitten, and ImageMagick are required")
    if not args.binary.is_file():
        raise RuntimeError("Reader missing; run mise run build")
    if any(
        not math.isfinite(value) or value <= 0
        for value in (args.scale, args.base_font)
    ):
        raise RuntimeError(
            "Font size and scale must be positive finite numbers"
        )
    run(args.kitten, "@", "screenshot", "--help")
    if not any(
        kind == "text" and ":ocr " in value
        for name in selected
        for kind, value in SCENES[name][2]
    ):
        return None
    if not shutil.which("tesseract"):
        raise RuntimeError("OCR capture requires Tesseract")
    if "eng" not in run("tesseract", "--list-langs").splitlines():
        raise RuntimeError("OCR capture requires English language data")
    if not shutil.which("pdftotext"):
        raise RuntimeError("OCR capture requires Poppler's pdftotext")
    if run("pdftotext", str(args.corpus / "scan.pdf"), "-"):
        raise RuntimeError("OCR demo scan must have no text")
    return {
        "engine": run("tesseract", "--version").splitlines()[0],
        "language": "eng",
        "input_has_embedded_text": False,
    }


def main():
    args = parse_args()
    if args.list_scenes:
        print("\n".join(SCENES))
        return
    selected = list(dict.fromkeys(args.scene or SCENES))
    if args.corpus == ROOT / "tmp/corpus":
        documents = sorted(
            {doc for name in selected for doc in scene_documents(name)}
        )
        subprocess.run(
            ["sh", str(ROOT / "scripts/corpus.sh"), "corpus", *documents],
            check=True,
            cwd=ROOT,
        )
    ocr = preflight(args, selected)
    inputs = input_hashes(args.corpus, selected)
    args.output.mkdir(parents=True, exist_ok=True)
    args.raw.mkdir(parents=True, exist_ok=True)
    path = args.output / "capture.json"
    manifest = load_manifest(path)
    if not args.scene:
        manifest["scenes"] = {
            name: scene
            for name, scene in manifest["scenes"].items()
            if name in SCENES
        }
    with tempfile.TemporaryDirectory(
        prefix="vpdf-demo-", dir="/tmp"
    ) as directory:
        gallery = Gallery(args, directory)
        with (args.raw / "kitty.log").open("w") as log:
            try:
                options = gallery.start(log)
                identity = capture_identity(args, options, ocr, inputs)
                for name in selected:
                    scene, image = gallery.capture(name)
                    verify_inputs(args, identity)
                    manifest = publish_scene(
                        args.output, manifest, identity, name, scene, image
                    )
            finally:
                gallery.close()


if __name__ == "__main__":
    main()
