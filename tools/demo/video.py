"""Record Kitty interactions with timed captions."""

import json
import select
import shutil
import subprocess
import tempfile
import time
from pathlib import Path

from tools.demo import capture as demo
from tools.demo.media import atomic_json, metadata
from tools.demo.window import CACHE, compile_helper

BAR_HEIGHT = 112
RECIPES = {
    "find-return": {
        "title": "Find and return",
        "seconds": 24,
        "poster_at": 12,
        "page": 2,
        "documents": ["paper"],
        "setup": [("text", "zc=")],
        "steps": [
            (
                2,
                "text",
                "/multi-head",
                "/multi-head",
                "Search this document.",
                None,
            ),
            (4, "key", "enter", "Enter", "Show matching passages.", "matches"),
            (7, "text", "n", "n", "Move to the next match.", None),
            (
                10,
                "text",
                "gs",
                "g then s",
                "Inspect search results.",
                "RESULTS",
            ),
            (14, "key", "enter", "Enter", "Follow the selected result.", None),
            (
                18,
                "key",
                "ctrl+o",
                "Ctrl-o",
                "Return to the source passage.",
                None,
            ),
        ],
    },
    "inspect-reference": {
        "title": "Inspect a reference",
        "seconds": 27,
        "poster_at": 8,
        "page": 2,
        "documents": ["paper"],
        "setup": [
            ("text", "zc=/[13]\r"),
            ("wait", "matches"),
            ("text", ":nohlsearch\r"),
        ],
        "steps": [
            (2, "text", "i", "i", "Place the glyph cursor.", None),
            (4, "text", "l", "l", "Move onto the citation.", None),
            (6, "text", "K", "K", "Preview the reference.", "╭"),
            (10, "text", "j", "j", "Scroll the preview.", None),
            (12, "text", "k", "k", "Scroll back.", None),
            (15, "key", "escape", "Escape", "Close the preview.", None),
            (18, "text", "gd", "g then d", "Resolve the reference.", None),
            (22, "key", "ctrl+o", "Ctrl-o", "Return to the citation.", None),
        ],
    },
    "compare-passages": {
        "title": "Compare passages",
        "seconds": 25,
        "poster_at": 14,
        "page": 2,
        "documents": ["paper"],
        "setup": [("text", "zc=")],
        "steps": [
            (2, "key", "ctrl+w", "Ctrl-w", "Begin a pane command.", None),
            (3, "text", "v", "v", "Duplicate this passage.", None),
            (6, "text", "3gg", "3 g g", "Move the second pane.", None),
            (8, "key", "ctrl+w", "Ctrl-w", "Choose another pane.", None),
            (9, "text", "p", "p", "Return focus to the first pane.", None),
            (12, "text", "8", "8", "Set a resize count.", None),
            (12.5, "key", "ctrl+w", "Ctrl-w", "Begin a pane command.", None),
            (13, "text", ">", ">", "Widen the focused pane.", None),
            (16, "key", "ctrl+w", "Ctrl-w", "Begin a pane command.", None),
            (17, "text", "=", "=", "Equalize the panes.", None),
            (20, "key", "ctrl+w", "Ctrl-w", "Begin a pane command.", None),
            (21, "text", "c", "c", "Close the focused pane.", None),
        ],
    },
    "select-capture": {
        "title": "Select and capture",
        "seconds": 26,
        "poster_at": 19.5,
        "page": 3,
        "documents": ["paper"],
        "setup": [
            ("text", "/Encoder\r"),
            ("wait", "matches"),
            ("text", ":nohlsearch\r"),
        ],
        "steps": [
            (2, "text", "i", "i", "Enter Cursor mode.", None),
            (4, "text", "v", "v", "Start a selection.", "VISUAL"),
            (6, "text", "2e", "2 e", "Extend the selection.", None),
            (8, "text", "R", "R", "Start a capture rectangle.", "REGION"),
            (11, "text", "8l", "8 l", "Widen the rectangle.", None),
            (13, "text", "4j", "4 j", "Extend its height.", None),
            (
                16,
                "key",
                "enter",
                "Enter",
                "Confirm the rectangle.",
                "Copy text",
            ),
            (18, "text", "l", "l", "Choose Copy text.", None),
            (
                20,
                "key",
                "enter",
                "Enter",
                "Copy the captured text.",
                "Text copied",
            ),
            (24, "key", "escape", "Escape", "Close capture actions.", None),
        ],
    },
}


def overlay_segments(events, origin, duration, title):
    segments = [{"start": 0.0, "label": "", "caption": title}]
    for event in events:
        if event["phase"] != "demo" or "label" not in event:
            continue
        start = (event["input_monotonic_ns"] - origin) / 1e9
        if start < 0 or start >= duration or start < segments[-1]["start"]:
            raise RuntimeError("Video input lies outside recording timeline")
        segments.append(
            {
                "start": start,
                "label": event["label"],
                "caption": event["caption"],
            }
        )
    for index, segment in enumerate(segments):
        segment["end"] = (
            segments[index + 1]["start"]
            if index + 1 < len(segments)
            else duration
        )
    return segments


def overlays(directory, width, segments, font):
    lines = ["ffconcat version 1.0"]
    for index, segment in enumerate(segments):
        image = directory / f"keys-{index:03}.png"
        subprocess.run(
            [
                "magick",
                "-size",
                f"{width}x{BAR_HEIGHT}",
                "canvas:#181b24",
                "-font",
                str(font),
                "-fill",
                "#d8dee9",
                "-pointsize",
                "24",
                "-gravity",
                "NorthWest",
                "-annotate",
                "+24+12",
                segment["caption"],
                "-fill",
                "#88c0d0",
                "-pointsize",
                "30",
                "-annotate",
                "+24+52",
                segment["label"],
                "PNG24:" + str(image),
            ],
            check=True,
        )
        lines.extend(
            [
                f"file '{image.name}'",
                f"duration {segment['end'] - segment['start']:.9f}",
            ]
        )
    lines.append(f"file 'keys-{len(segments) - 1:03}.png'")
    path = directory / "keys.ffconcat"
    path.write_text("\n".join(lines) + "\n")
    return path


def encode(source, target, strip, duration):
    for quality in (20, 23):
        subprocess.run(
            [
                "ffmpeg",
                "-hide_banner",
                "-loglevel",
                "error",
                "-y",
                "-i",
                str(source),
                "-f",
                "concat",
                "-safe",
                "0",
                "-i",
                str(strip),
                "-filter_complex",
                f"[0:v]tpad=stop_mode=clone:stop_duration=1,fps=30,pad=iw:ih+{BAR_HEIGHT}:0:0:color=0x181b24[base];[base][1:v]overlay=0:main_h-overlay_h:eof_action=repeat,format=yuv420p[out]",
                "-map",
                "[out]",
                "-an",
                "-c:v",
                "libx264",
                "-threads",
                "2",
                "-preset",
                "slow",
                "-crf",
                str(quality),
                "-movflags",
                "+faststart",
                "-t",
                str(duration),
                "-r",
                "30",
                "-fps_mode",
                "cfr",
                str(target),
            ],
            check=True,
            timeout=300,
        )
        if target.stat().st_size < 10_000_000:
            return {
                "codec": "H.264",
                "encoder": "libx264",
                "crf": quality,
                "encoded_fps": 30,
                "overlay": "FFmpeg overlay; timestamped ImageMagick key strip outside viewport",
                "edits": "No cuts or time compression; idle frames retained; constant-rate encoding",
            }
    raise RuntimeError("Encoded video exceeds upload limit")


def ready(process, gallery):
    deadline = time.monotonic() + 15
    while time.monotonic() < deadline:
        gallery.host.watch()
        if process.poll() is not None:
            raise RuntimeError("Window recorder stopped before first frame")
        readable, _, _ = select.select([process.stdout], [], [], 0.1)
        if readable:
            row = json.loads(process.stdout.readline())
            if row.get("ready"):
                return round(row["first_frame_monotonic"] * 1e9)
    raise RuntimeError("Window recorder first frame timed out")


def record(gallery, name, recipe, recorder, directory):
    events = []
    gallery.launch_scene(name, recipe["page"], recipe["documents"])
    gallery.actions(recipe["setup"], events, phase="setup")
    gallery.actions([("pause", 1)], events, phase="setup", pace=0)
    raw = directory / f"{name}.mov"
    report = directory / f"{name}-recording.json"
    with (directory / f"{name}-recorder.log").open("w") as log:
        process = subprocess.Popen(
            [
                str(recorder),
                str(gallery.os_window),
                str(recipe["seconds"]),
                str(raw),
                str(report),
            ],
            stdout=subprocess.PIPE,
            stderr=log,
            text=True,
        )
        try:
            origin = ready(process, gallery)
            for due, kind, value, label, caption, expected in recipe["steps"]:
                gallery.host.pace(origin / 1e9 + due)
                if process.poll() is not None:
                    raise RuntimeError("Window recording ended before inputs")
                gallery.actions([(kind, value)], events, phase="demo", pace=0)
                events[-1].update(
                    label=label, caption=caption, planned_seconds=due
                )
                if expected:
                    gallery.actions(
                        [("wait", expected)], events, phase="assert", pace=0
                    )
            deadline = time.monotonic() + 15
            while process.poll() is None:
                gallery.host.watch()
                if raw.exists() and raw.stat().st_size > 512_000_000:
                    raise RuntimeError("Raw video exceeds capture bound")
                if time.monotonic() > deadline:
                    raise RuntimeError("Window recording exceeded deadline")
                time.sleep(0.1)
            if process.returncode:
                raise RuntimeError(
                    "Window recorder failed; inspect recorder log"
                )
        finally:
            if process.poll() is None:
                process.terminate()
                try:
                    process.wait(timeout=3)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait()
            process.stdout.close()
            atomic_json(directory / f"{name}-inputs.json", events)
    capture = json.loads(report.read_text())
    if capture["writer_dropped_frames"]:
        raise RuntimeError("Recording lost encoded frames; retry capture")
    capture.update(
        input_timeline=events,
        first_frame_monotonic_ns=origin,
        window_checks=gallery.host.evidence(),
        viewport=[gallery.args.width, gallery.args.height],
    )
    atomic_json(report, capture)
    return raw, capture


def publish_video(
    args, manifest, identity, name, recipe, source, capture, directory
):
    info = metadata(source, "video")
    if not 15 <= info["duration"] <= 35:
        raise RuntimeError("Video duration exceeds scene bounds")
    segments = overlay_segments(
        capture["input_timeline"],
        capture["first_frame_monotonic_ns"],
        info["duration"],
        recipe["title"],
    )
    capture = {
        **capture,
        "source_sha256": demo.digest(source),
        "source_bytes": source.stat().st_size,
        "source_media": info,
    }
    strip = overlays(directory, args.width, segments, args.overlay_font)
    target = directory / f"{name}.mp4"
    encoding = encode(source, target, strip, info["duration"])
    final = metadata(target, "video")
    scene = {
        "kind": "video",
        "image": target.name,
        "sha256": demo.digest(target),
        "bytes": target.stat().st_size,
        "documents": recipe["documents"],
        "start_page": recipe["page"],
        "dimensions": final["dimensions"],
        "duration": final["duration"],
        "frame_rate": final["frame_rate"],
        "pixel_format": final["pixel_format"],
        "encoding": encoding,
        "recording": capture,
        "overlay_segments": segments,
        "overlay_font_sha256": demo.digest(args.overlay_font),
    }
    manifest = demo.publish_scene(
        args.output, manifest, identity, name, scene, target, args.manifest
    )
    poster = directory / f"{name}-poster.jpg"
    frame = directory / f"{name}-poster.png"
    subprocess.run(
        [
            "ffmpeg",
            "-hide_banner",
            "-loglevel",
            "error",
            "-y",
            "-ss",
            str(recipe["poster_at"]),
            "-i",
            str(target),
            "-frames:v",
            "1",
            "-pix_fmt",
            "rgb24",
            str(frame),
        ],
        check=True,
        timeout=60,
    )
    demo.optimize_image(frame, poster, final["dimensions"])
    poster_scene = {
        "kind": "poster",
        "image": poster.name,
        "sha256": demo.digest(poster),
        "bytes": poster.stat().st_size,
        "documents": recipe["documents"],
        "dimensions": final["dimensions"],
        "video": name,
        "video_sha256": scene["sha256"],
        "frame_seconds": recipe["poster_at"],
        "encoding": demo.ENCODING + "; frame extracted from captioned video",
    }
    return demo.publish_scene(
        args.output,
        manifest,
        identity,
        name + "-poster",
        poster_scene,
        poster,
        args.manifest,
    )


def main():
    parser = demo.arg_parser()
    parser.description = __doc__
    parser.add_argument(
        "--overlay-font",
        type=Path,
        default=Path("/System/Library/Fonts/Menlo.ttc"),
    )
    parser.add_argument("--recipe", choices=RECIPES, action="append")
    args = demo.resolve_args(parser.parse_args())
    if args.list_scenes:
        print("\n".join(RECIPES))
        return
    selected = list(dict.fromkeys(args.recipe or RECIPES))
    for tool in ("ffmpeg", "ffprobe", "swiftc"):
        if not shutil.which(tool):
            raise RuntimeError(f"Video capture requires {tool}")
    if not args.overlay_font.is_file():
        raise RuntimeError("Overlay font file missing")
    if args.width % 2 or args.height % 2:
        raise RuntimeError("Video dimensions must be even")
    demo.preflight(args, ["search"])
    if args.corpus == demo.ROOT / "tmp/corpus":
        subprocess.run(
            [
                "sh",
                str(demo.ROOT / "tools/corpus/corpus.sh"),
                "corpus",
                "paper",
            ],
            check=True,
        )
    inputs = demo.input_hashes(args.corpus, ["search"])
    recorder = compile_helper(
        demo.ROOT / "tools/demo/record.swift",
        CACHE / "record",
        ("-parse-as-library",),
    )
    for path in (args.output, args.raw, args.manifest.parent):
        path.mkdir(parents=True, exist_ok=True)
    manifest = demo.load_manifest(args.manifest)
    for name in selected:
        raw_directory = args.raw / name / str(time.time_ns())
        raw_directory.mkdir(parents=True)
        with tempfile.TemporaryDirectory(
            prefix="vpdf-video-", dir="/tmp"
        ) as directory:
            gallery = demo.Gallery(args, directory)
            with (raw_directory / "kitty.log").open("w") as log:
                try:
                    options = gallery.start(log)
                    identity = demo.capture_identity(
                        args, options, None, inputs
                    )
                    identity["capture"] = (
                        "ScreenCaptureKit desktop-independent window; visible Kitty; no activation"
                    )
                    source, capture = record(
                        gallery, name, RECIPES[name], recorder, raw_directory
                    )
                    demo.verify_inputs(args, identity)
                finally:
                    gallery.close()
        manifest = publish_video(
            args,
            manifest,
            identity,
            name,
            RECIPES[name],
            source,
            capture,
            raw_directory,
        )
        print(f"Recorded {name}", flush=True)


if __name__ == "__main__":
    main()
