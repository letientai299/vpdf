"""Capture the macOS app without activating it."""

import argparse
import hashlib
import platform
import subprocess
import tempfile
import time
from pathlib import Path
from types import SimpleNamespace

from tools.bench.live.drive import Native, finish
from tools.demo import capture as demo
from tools.demo.window import CACHE, Unfocused, compile_helper, lease

# Start page, documents, and event lines; see tools/bench/live/inject docs.
SCENES = {
    "app-toolbar": (
        3,
        ["paper"],
        ["native tool Find", "key a t t e n t i o n enter", "wait 1s"],
    ),
    "app-sidebar": (5, ["paper"], ["native tool Sidebar", "wait 1s"]),
}


class App(Unfocused, Native):
    launch_note = "open -g -n; bench-events FIFO; no activation"

    def enable_window(self):
        # Geometry uses UI points; UI scale changes their size.
        samples = []
        bounds = self.settle(
            lambda bounds: samples.append(bounds) or samples[-2:-1] == [bounds]
        )
        factor = bounds["Width"] / self.size[0]
        if abs(factor - 1) > 0.01:
            self.send(
                f"place {round(self.place[0] / factor)} "
                f"{round(self.place[1] / factor)}"
            )
            self.send(
                f"resize {round(self.size[0] / factor)} "
                f"{round(self.size[1] / factor)}"
            )
            self.marker("scaled")
        self.settle(
            lambda bounds: (
                abs(bounds["Width"] - self.size[0]) < 2
                and abs(bounds["X"] - self.place[0]) < 2
                and abs(bounds["Y"] - self.place[1]) < 2
            )
        )
        super().enable_window()

    def settle(self, placed):
        # The window server applies geometry after the app does.
        deadline = time.monotonic() + 5
        while time.monotonic() < deadline:
            state = self.inspect()
            self.validate(state)
            bounds = state.get("window")
            if bounds and placed(bounds):
                return bounds
            time.sleep(0.05)
        raise RuntimeError("App window geometry never settled")


def arg_parser():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--app", type=Path, default=demo.ROOT / "target/dist/Vpdf.app"
    )
    parser.add_argument("--build-profile", default="unknown")
    parser.add_argument("--corpus", type=Path, default=demo.ROOT / "tmp/corpus")
    parser.add_argument("--output", type=Path, default=demo.CACHE / "encoded")
    parser.add_argument(
        "--manifest",
        type=Path,
        default=demo.ROOT / "docs/assets/capture.json",
    )
    parser.add_argument("--raw", type=Path, default=demo.CACHE / "raw")
    parser.add_argument("--width", type=int, default=1600)
    parser.add_argument("--height", type=int, default=900)
    parser.add_argument("--place", type=int, nargs=2, metavar=("X", "Y"))
    parser.add_argument("--scene", choices=SCENES, action="append")
    return parser


def launch(args, name, directory, probe):
    page, documents, _ = SCENES[name]
    files = [
        (args.corpus / f"{doc}.pdf").resolve().as_uri()
        + (f"#page={page}" if index == 0 else "")
        for index, doc in enumerate(documents)
    ]
    options = SimpleNamespace(
        app=args.app,
        scenario=None,
        corpus=args.corpus,
        window=(args.width, args.height),
        place=args.place,
        timeout=120,
    )
    return App(
        options,
        args.app / "Contents/MacOS/vpdf-gui",
        directory,
        probe,
        lambda *_: (files, [], (args.width, args.height), "off"),
    )


def screenshot(app, raw):
    number = app.inspect().get("number")
    if number is None:
        raise RuntimeError("App window number missing")
    previous = None
    for _ in range(20):
        app.watch()
        app.check()
        subprocess.run(
            ["screencapture", "-x", "-o", "-l", str(number), str(raw)],
            check=True,
        )
        pixels = subprocess.check_output(
            ["magick", str(raw), "-depth", "8", "rgba:-"]
        )
        current = hashlib.sha256(pixels).hexdigest()
        if current == previous:
            return
        previous = current
        time.sleep(0.5)
    raise RuntimeError("Window screenshot never settled")


def capture(args, name, directory, probe):
    _, documents, steps = SCENES[name]
    app = launch(args, name, directory, probe)
    try:
        app.start_monitor()
        app.launch()
        app.run()
        for line in steps:
            if line.startswith("native "):
                app.send(line)
            else:
                app.step(line)
        app.step("wait idle")
        raw = args.raw / f"{name}.png"
        screenshot(app, raw)
        evidence = app.evidence()
    finally:
        cleanup = finish(app)
    if app.monitor_error:
        raise app.monitor_error
    if cleanup:
        raise RuntimeError("; ".join(cleanup))
    bounds = evidence["window"]
    width, height = map(
        int,
        demo.run("magick", "identify", "-format", "%w %h", str(raw)).split(),
    )
    # Backing scale: 1 on standard, 2 on Retina displays.
    if width * bounds["Height"] != height * bounds["Width"]:
        raise RuntimeError("Screenshot does not match window bounds")
    target = directory / f"{name}.jpg"
    demo.optimize_image(raw, target, [width, height])
    print(f"Captured {name}: {target.stat().st_size} bytes", flush=True)
    scene = {
        "kind": "native",
        "image": target.name,
        "encoding": demo.ENCODING,
        "sha256": demo.digest(target),
        "bytes": target.stat().st_size,
        "start_page": SCENES[name][0],
        "documents": documents,
        "launch_documents": documents,
        "steps": steps,
        "dimensions": [width, height],
        "window_checks": evidence,
    }
    return scene, target, app.source_sha256


def identity(args, binary, version, inputs):
    source = demo.binary_identity(binary, args.build_profile)
    if version != demo.run(str(binary), "--version"):
        raise RuntimeError("App version changed during capture")
    return {
        "captured_at": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        **source,
        "platform": platform.platform(),
        "machine": demo.run("sysctl", "-n", "machdep.cpu.brand_string"),
        "window_size": [args.width, args.height],
        "capture": "screencapture -l window; no shadow; no activation",
        "cache_conditions": "Fresh app, private saved state, VPDF_CACHE=off",
        "encoding": demo.ENCODING,
        "inputs": inputs,
    }


def main():
    args = arg_parser().parse_args()
    for field in ("app", "corpus", "output", "raw", "manifest"):
        setattr(args, field, getattr(args, field).resolve())
    demo.verify_output(args.output)
    demo.verify_output(args.raw)
    binary = args.app / "Contents/MacOS/vpdf-gui"
    version = demo.run(str(binary), "--version")
    if "[bench-events]" not in version:
        raise RuntimeError("App lacks bench-events feature")
    selected = list(dict.fromkeys(args.scene or SCENES))
    documents = sorted({doc for name in selected for doc in SCENES[name][1]})
    demo.fetch_corpus(args.corpus, documents)
    inputs = demo.document_hashes(args.corpus, documents)
    for path in (args.output, args.raw, args.manifest.parent):
        path.mkdir(parents=True, exist_ok=True)
    manifest = demo.load_manifest(args.manifest)
    probe = compile_helper(
        demo.ROOT / "tools/bench/live/inspect.swift", CACHE / "inspect"
    )
    with lease():
        for name in selected:
            with tempfile.TemporaryDirectory(
                prefix="vpdf-native-", dir="/tmp"
            ) as directory:
                scene, image, launched = capture(
                    args, name, Path(directory), probe
                )
                record = identity(args, binary, version, inputs)
                if launched != record["binary_sha256"]:
                    raise RuntimeError("Captured app binary changed")
                demo.verify_inputs(
                    SimpleNamespace(binary=binary, corpus=args.corpus), record
                )
                manifest = demo.publish_scene(
                    args.output,
                    manifest,
                    record,
                    name,
                    scene,
                    image,
                    args.manifest,
                )


if __name__ == "__main__":
    main()
