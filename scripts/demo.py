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

from demo_window import Window

ROOT = Path(__file__).resolve().parent.parent
CACHE = Path.home() / ".cache/vpdf/demo-media" / ROOT.name
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
            ("text", "zc=/[13]\r"),
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
            ("hover", "paper.pdf"),
            ("hover", "scan.pdf"),
            ("pause", 3),
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
            ("wait", "Model Architecture"),
            ("wait", "/multi-head/"),
        ],
    ),
    "ocr": (
        7,
        ["scan"],
        [
            ("pause", 2),
            ("text", ":vsplit\r"),
            ("text", ":text\r"),
            ("wait", "DEFINITIONS"),
            ("text", "i0v3e"),
            ("wait", "VISUAL"),
        ],
    ),
    "help-split": (
        39,
        ["book"],
        [("text", ":help! panes\r"), ("wait", "Help")],
    ),
    "definition": (
        2,
        ["paper"],
        [
            ("text", "/Figure 1\r"),
            ("wait", "matches"),
            ("text", "n:nohlsearch\rilgd"),
            ("wait", "Reference found"),
            ("pause", 1),
        ],
    ),
    "continuous-text": (
        2,
        ["paper"],
        [("text", ":text\ri50j"), ("pause", 1)],
    ),
    "annotations": (
        2,
        ["paper"],
        [
            ("text", ":help sample\r"),
            ("wait", "Help"),
            ("text", "ga"),
            ("wait", "ANNOTATIONS"),
            ("text", "jK"),
            ("wait", "Notes explain"),
        ],
    ),
    "figures": (
        3,
        ["paper"],
        [("text", "]fzFgfj"), ("wait", "FIGURES")],
    ),
    "marks": (
        2,
        ["paper"],
        [("text", "ma4ggmA2gg:marks\r"), ("wait", "MARKS")],
    ),
}

SCENES["annotations"][2].extend([("note", "hover"), ("wait", "Vpdf")])
SCENES["annotation-actions"] = (
    2,
    ["paper"],
    [
        *SCENES["annotations"][2],
        ("key", "escape"),
        ("note", "context"),
        ("wait", "Copy comment"),
    ],
)
SCENES["zoom"] = (
    2,
    ["paper"],
    [
        ("text", ":help sample\r"),
        ("wait", "Help"),
        ("text", ":vsplit\r"),
        ("text", "zc=+zr"),
        ("key", "ctrl+w"),
        ("text", "p"),
        ("text", ":text\r"),
    ],
)
for theme in ("dark", "light", "nord"):
    SCENES["theme-" + theme] = (
        2,
        ["paper"],
        [
            ("text", ":help sample\r"),
            ("wait", "Help"),
            ("text", ":theme " + theme + "\r"),
            ("pause", 2),
            ("text", "go"),
            ("wait", "CONTENTS"),
        ],
    )


def run(*args):
    return subprocess.check_output(args, text=True, cwd=ROOT).rstrip()


def digest(path):
    with path.open("rb") as file:
        return hashlib.file_digest(file, "sha256").hexdigest()


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
        "dimensions": [args.width, args.height],
        "capture": "Kitty screenshot API; graphics included; no resizing",
        "cache_conditions": "Fresh reader, private saved state and empty content cache per scene",
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


def publish_scene(
    output, manifest, identity, name, scene, image, manifest_path=None
):
    capture_id = capture_key(identity)
    scenes = {**manifest["scenes"], name: {**scene, "capture": capture_id}}
    captures = {**manifest["captures"], capture_id: identity}
    updated = {
        "captures": captures,
        "scenes": scenes,
    }
    target = output / scene["image"]
    manifest_path = manifest_path or output / "capture.json"
    with (
        tempfile.TemporaryDirectory(
            prefix=".capture-", dir=output
        ) as directory,
        tempfile.TemporaryDirectory(
            prefix=".capture-", dir=manifest_path.parent
        ) as reports,
    ):
        directory = Path(directory)
        staged = directory / scene["image"]
        shutil.copyfile(image, staged)
        report = Path(reports) / "capture.json"
        report.write_text(json.dumps(updated, indent=2) + "\n")
        backup = directory / "previous.jpg"
        if target.exists():
            shutil.copyfile(target, backup)
        installed = False
        try:
            staged.replace(target)
            installed = True
            report.replace(manifest_path)
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
        if hasattr(self, "host"):
            self.host.watch()
        result = run(self.args.kitten, "@", "--to", self.socket, *args)
        if hasattr(self, "host"):
            self.host.watch()
        return result

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

    def note(self, action):
        if self.resolved_targets:
            x, y = self.resolved_targets[-1]["pixels"]
        else:
            image = self.directory / "note-target.png"
            self.remote(
                "screenshot",
                "--match-tab",
                f"window_id:{self.window}",
                str(image),
            )
            geometry = run(
                "magick",
                str(image),
                "-fuzz",
                "5%",
                "-fill",
                "black",
                "+opaque",
                "#ffff00",
                "-fill",
                "white",
                "-opaque",
                "#ffff00",
                "-format",
                "%@",
                "info:",
            )
            match = re.fullmatch(r"(\d+)x(\d+)\+(\d+)\+(\d+)", geometry)
            if not match:
                raise RuntimeError("Practice note icon missing")
            width, height, left, top = map(int, match.groups())
            if not (4 <= width <= 64 and 4 <= height <= 64):
                raise RuntimeError("Practice note icon is ambiguous")
            x, y = left + width / 2, top + height / 2
        self.pointer("move", x, y)
        if action == "context":
            self.pointer("right-down", x, y)
            self.pointer("right-up", x, y)
        self.resolved_targets.append(
            {"target": "practice note", "action": action, "pixels": [x, y]}
        )

    def launch_scene(self, name, page, documents):
        self.resolved_targets = []
        previous = self.window
        self.window = self.remote(
            "launch",
            "--type",
            "tab",
            "--cwd",
            str(ROOT),
            "--env",
            f"XDG_STATE_HOME={self.directory / name / 'state'}",
            "--env",
            f"XDG_CACHE_HOME={self.directory / name / 'cache'}",
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

    def actions(self, steps, timeline=None, phase="setup", pace=0.3):
        for kind, value in steps:
            started = time.monotonic_ns()
            if kind == "wait":
                self.wait(value)
            elif kind == "pause":
                time.sleep(float(value))
            elif kind == "mouse":
                self.pointer(
                    value[0],
                    value[1] * self.args.width / 2560,
                    value[2] * self.args.height / 1440,
                )
            elif kind == "hover":
                self.hover(value)
            elif kind == "note":
                self.note(value)
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
            if timeline is not None:
                timeline.append(
                    {
                        "phase": phase,
                        "kind": kind,
                        "value": value,
                        "input_monotonic_ns": started,
                        "completed_monotonic_ns": time.monotonic_ns(),
                    }
                )
            time.sleep(pace)

    def capture(self, name):
        page, documents, steps = SCENES[name]
        self.launch_scene(name, page, documents)
        self.actions(steps)
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
        run(
            "sh",
            str(ROOT / "scripts/demo-optimize.sh"),
            str(raw),
            str(target),
            f"{self.args.width}x{self.args.height}",
        )
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
            "dimensions": [self.args.width, self.args.height],
            "window_checks": self.host.evidence(),
            "resolved_targets": self.resolved_targets,
            "ocr_mode": (
                "explicit request"
                if any(
                    kind == "text" and ":ocr" in str(value)
                    for kind, value in steps
                )
                else "automatic; cache initially empty"
                if "scan" in scene_documents(name)
                else None
            ),
        }
        return scene, target

    def close(self):
        if hasattr(self, "host"):
            self.host.close()

    def start(self, log):
        font_size = self.args.base_font * self.args.scale
        options = {
            "allow_remote_control": "socket-only",
            "initial_window_width": str(self.args.width),
            "initial_window_height": str(self.args.height),
            "remember_window_size": "no",
            "confirm_os_window_close": "0",
            "macos_quit_when_last_window_closed": "yes",
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
            "hidden",
            "--single-instance",
            "--instance-group",
            self.directory.name,
        ]
        for key, value in options.items():
            command.extend(["-o", f"{key}={value}"])
        command.extend(["/bin/sleep", "3600"])
        self.host = Window(self.args, self.directory)
        self.host.launch(command, self.socket, log)
        deadline = time.monotonic() + 30
        while not (self.directory / "kitty.sock").exists():
            self.host.watch()
            if time.monotonic() > deadline:
                raise RuntimeError("Kitty startup failed; inspect kitty.log")
            time.sleep(0.2)
        # Never persist environments from ls.
        windows = json.loads(self.remote("ls"))
        self.window = str(windows[0]["tabs"][0]["windows"][0]["id"])
        self.os_window = windows[0]["platform_window_id"]
        self.host.show(self.remote)
        return options


def arg_parser():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--binary", type=Path, default=ROOT / "target/release/vpdf"
    )
    parser.add_argument("--build-profile", default="unknown")
    parser.add_argument("--corpus", type=Path, default=ROOT / "tmp/corpus")
    parser.add_argument("--output", type=Path, default=CACHE / "encoded")
    parser.add_argument(
        "--manifest", type=Path, default=ROOT / "docs/assets/capture.json"
    )
    parser.add_argument("--raw", type=Path, default=CACHE / "raw")
    parser.add_argument("--width", type=int, default=2560)
    parser.add_argument("--height", type=int, default=1440)
    parser.add_argument("--place", type=int, nargs=2, metavar=("X", "Y"))
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
    return parser


def resolve_args(args):
    for name in ("binary", "corpus", "output", "raw", "manifest"):
        setattr(args, name, getattr(args, name).resolve())
    return args


def parse_args(argv=None):
    return resolve_args(arg_parser().parse_args(argv))


def verify_output(path):
    path = path.resolve()
    parent = path
    while not parent.is_dir():
        parent = parent.parent
    repository = subprocess.run(
        ["git", "rev-parse", "--show-toplevel"],
        cwd=parent,
        capture_output=True,
        text=True,
    )
    if repository.returncode != 0:
        return
    tracked = subprocess.run(
        ["git", "ls-files", "-z", "--", str(path)],
        cwd=Path(repository.stdout.strip()),
        capture_output=True,
        check=True,
    )
    result = subprocess.run(
        ["git", "check-ignore", "--no-index", "--quiet", str(path)],
        cwd=Path(repository.stdout.strip()),
    )
    if tracked.stdout or result.returncode != 0:
        raise RuntimeError("Demo output must stay outside tracked paths")


def preflight(args, selected):
    verify_output(args.output)
    verify_output(args.raw)
    if (
        not (1 <= args.width <= 8192 and 1 <= args.height <= 8192)
        or args.width * args.height > 16_777_216
    ):
        raise RuntimeError("Demo dimensions exceed capture bounds")
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
    if not any("scan" in scene_documents(name) for name in selected):
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
    args.manifest.parent.mkdir(parents=True, exist_ok=True)
    path = args.manifest
    manifest = load_manifest(path)
    if not args.scene:
        manifest["scenes"] = {
            name: scene
            for name, scene in manifest["scenes"].items()
            if name in SCENES or scene.get("kind") in {"video", "poster"}
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
                        args.output,
                        manifest,
                        identity,
                        name,
                        scene,
                        image,
                        args.manifest,
                    )
            finally:
                gallery.close()


if __name__ == "__main__":
    main()
