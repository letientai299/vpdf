import os
import tempfile
import threading
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import patch

import build_worker
import demo
import demo_video as video
import demo_window


class TimelineTests(unittest.TestCase):
    def test_dispatch_times_drive_overlays(self):
        with tempfile.TemporaryDirectory() as directory:
            gallery = demo.Gallery(
                SimpleNamespace(kitten="kitten", corpus=Path(directory)),
                directory,
            )
            gallery.window = "7"
            timeline = []
            with (
                patch.object(gallery, "remote") as remote,
                patch.object(demo.time, "sleep"),
                patch.object(
                    demo.time,
                    "monotonic_ns",
                    side_effect=[
                        102_500_000_000,
                        102_520_000_000,
                        104_750_000_000,
                        104_800_000_000,
                    ],
                ),
            ):
                gallery.actions(
                    [("key", "ctrl+w"), ("text", "v")],
                    timeline,
                    phase="demo",
                    pace=0,
                )
            self.assertEqual(
                [call.args[0] for call in remote.call_args_list],
                ["send-key", "send-text"],
            )
            for event, label in zip(timeline, ["Ctrl-w", "v"]):
                event.update(
                    label=label, caption="Split the pane.", planned_seconds=1
                )
            segments = video.overlay_segments(
                timeline, 100_000_000_000, 20, "Compare"
            )
            self.assertEqual(
                [
                    (segment["start"], segment["end"], segment["label"])
                    for segment in segments
                ],
                [(0, 2.5, ""), (2.5, 4.75, "Ctrl-w"), (4.75, 20, "v")],
            )
            self.assertTrue(
                all(
                    event["completed_monotonic_ns"]
                    >= event["input_monotonic_ns"]
                    for event in timeline
                )
            )

    def test_setup_and_assertions_excluded(self):
        events = [
            {
                "phase": phase,
                "input_monotonic_ns": 1,
                "label": "setup",
                "caption": "Setup",
            }
            for phase in ("setup", "assert")
        ]
        self.assertEqual(
            video.overlay_segments(events, 10, 20, "Ready"),
            [{"start": 0.0, "end": 20, "label": "", "caption": "Ready"}],
        )

    def test_invalid_recording_times_rejected(self):
        for times in ((-1,), (21,), (3, 2)):
            events = [
                {
                    "phase": "demo",
                    "input_monotonic_ns": int(at * 1e9),
                    "label": "j",
                    "caption": "Scroll",
                }
                for at in times
            ]
            with self.subTest(times=times), self.assertRaises(RuntimeError):
                video.overlay_segments(events, 0, 20, "Ready")

    def test_key_strip_preserves_viewport(self):
        with tempfile.TemporaryDirectory() as directory:
            with patch.object(video.subprocess, "run") as run:
                strip = video.overlays(
                    Path(directory),
                    1280,
                    [
                        {
                            "start": 0,
                            "end": 2,
                            "label": "Enter",
                            "caption": "Follow",
                        }
                    ],
                    Path("/font.ttf"),
                )
            self.assertIn("1280x112", run.call_args.args[0])
            self.assertIn("duration 2.000000000", strip.read_text())
            self.assertIn("Enter", run.call_args.args[0])

    def test_encoding_keeps_normal_time_and_source_pixels(self):
        with tempfile.TemporaryDirectory() as directory:
            target = Path(directory) / "video.mp4"
            target.write_bytes(b"encoded")
            with patch.object(video.subprocess, "run") as run:
                video.encode(
                    Path("/raw.mov"), target, Path("/keys.ffconcat"), 23.75
                )
            args = run.call_args.args[0]
            self.assertEqual(args[args.index("-t") + 1], "23.75")
            filters = args[args.index("-filter_complex") + 1]
            self.assertIn("pad=iw:ih+112:0:0", filters)
            self.assertIn("overlay=0:main_h-overlay_h", filters)
            self.assertNotIn("setpts", filters)
            self.assertNotIn("scale=", filters)

    def test_recording_list_stays_offline(self):
        with (
            patch.object(
                video.demo,
                "preflight",
                side_effect=AssertionError("Unexpected preflight"),
            ),
            patch.object(video.demo, "arg_parser", wraps=demo.arg_parser),
            patch("sys.argv", ["demo_video.py", "--list-scenes"]),
        ):
            video.main()


class WindowTests(unittest.TestCase):
    def window(self):
        window = object.__new__(demo_window.Window)
        window._pid = 99
        window.focus = ("user.editor", 11)
        window.check_window = False
        window.foreground_states = []
        window.checks = 0
        return window

    def state(self, pid):
        return {
            "front": "application",
            "front_pid": pid,
            "locked": False,
            "builds": [],
        }

    def test_user_application_changes_allowed(self):
        window = self.window()
        window.validate(self.state(11))
        window.validate(self.state(22))
        self.assertEqual([row[1] for row in window.foreground_states], [11, 22])
        self.assertEqual(window.checks, 2)

    def test_capture_application_focus_rejected(self):
        with self.assertRaisesRegex(RuntimeError, "keyboard focus"):
            self.window().validate(self.state(99))

    def test_existing_build_and_lock_guards_preserved(self):
        for changes in ({"builds": ["rustc:3"]}, {"locked": True}):
            with self.subTest(changes=changes), self.assertRaises(RuntimeError):
                self.window().validate({**self.state(11), **changes})


class HelperTests(unittest.TestCase):
    def test_cache_uses_source_and_flags(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source, target = root / "helper.swift", root / "helper"

            def compile(args, **kwargs):
                output = Path(args[args.index("-o") + 1])
                output.write_bytes(source.read_bytes())

            with patch.object(
                demo_window.subprocess, "run", side_effect=compile
            ) as run:
                source.write_bytes(b"first source")
                first = demo_window.compile_helper(source, target)
                first_bytes = first.read_bytes()
                source.write_bytes(b"second source")
                os.utime(source, ns=(1, 1))
                second = demo_window.compile_helper(source, target)
                self.assertEqual(second.read_bytes(), b"second source")
                self.assertEqual(first.read_bytes(), first_bytes)
                self.assertNotEqual(first, second)
                self.assertEqual(
                    demo_window.compile_helper(source, target), second
                )
                third = demo_window.compile_helper(source, target, ("-O",))
                self.assertNotEqual(second, third)
                self.assertEqual(run.call_count, 3)


@unittest.skipUnless(
    video.shutil.which("ffmpeg")
    and video.shutil.which("magick")
    and Path("/System/Library/Fonts/Menlo.ttc").exists(),
    "Requires video tools and Menlo",
)
class EncodingTests(unittest.TestCase):
    def test_all_caption_intervals(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source, target = root / "source.mp4", root / "encoded.mp4"
            video.subprocess.run(
                [
                    "ffmpeg",
                    "-hide_banner",
                    "-loglevel",
                    "error",
                    "-f",
                    "lavfi",
                    "-i",
                    "color=c=white:s=320x100:r=30:d=4",
                    "-c:v",
                    "libx264",
                    "-threads",
                    "1",
                    str(source),
                ],
                check=True,
            )
            segments = [
                {"start": i, "end": i + 1, "label": label, "caption": caption}
                for i, (label, caption) in enumerate(
                    [
                        ("", "Compare"),
                        ("8", "Set count."),
                        ("Ctrl-w", "Pane command."),
                        (">", "Widen pane."),
                    ]
                )
            ]
            strip = video.overlays(
                root, 320, segments, Path("/System/Library/Fonts/Menlo.ttc")
            )
            video.encode(source, target, strip, 4)
            self.assertAlmostEqual(
                video.metadata(target, "video")["duration"], 4, delta=1 / 30
            )
            for second in (0.5, 1.5, 2.5, 3.5):
                frame = root / f"bar-{second}.png"
                video.subprocess.run(
                    [
                        "ffmpeg",
                        "-hide_banner",
                        "-loglevel",
                        "error",
                        "-ss",
                        str(second),
                        "-i",
                        str(target),
                        "-vf",
                        "crop=320:112:0:100",
                        "-frames:v",
                        "1",
                        str(frame),
                    ],
                    check=True,
                )
                spread = float(
                    video.subprocess.check_output(
                        [
                            "magick",
                            str(frame),
                            "-format",
                            "%[fx:standard_deviation]",
                            "info:",
                        ],
                        text=True,
                    )
                )
                self.assertGreater(spread, 0.02, f"Missing caption at {second}")


class LeaseTests(unittest.TestCase):
    def test_capture_blocks_build_worker(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "leases").mkdir()
            entered = threading.Event()
            started = threading.Event()
            failures = []

            def worker():
                started.set()
                try:
                    with build_worker.worker_lease(root, root, 1):
                        entered.set()
                except BaseException as error:
                    failures.append(error)

            with (
                patch.object(
                    demo_window.platform, "system", return_value="Darwin"
                ),
                patch.object(demo_window.Path, "home", return_value=root),
                patch.object(demo_window, "cache_root", return_value=root),
                patch.object(
                    demo_window, "compile_helper", return_value=root / "probe"
                ),
                patch.object(
                    demo_window.Window,
                    "inspect",
                    return_value={"front": "user", "front_pid": 11},
                ),
            ):
                window = demo_window.Window(SimpleNamespace(), root)
            thread = threading.Thread(target=worker)
            thread.start()
            try:
                self.assertTrue(started.wait(1))
                self.assertFalse(entered.wait(0.05))
            finally:
                window.close()
                thread.join(2)
            self.assertFalse(thread.is_alive())
            self.assertEqual(failures, [])
            self.assertTrue(entered.is_set())
