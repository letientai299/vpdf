import json
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import Mock, patch

from drive import run_backend
from kitty import Kitty
from runtime import Runtime


class LifecycleTests(unittest.TestCase):
    def backend(self):
        runtime = Mock()
        runtime.binary = Path("reader")
        runtime.monitor_error = None
        runtime.focus = ("app", 1)
        runtime.last_state = {}
        return runtime

    def run_backend(self, runtime, folder):
        with (
            patch("drive.Native", return_value=runtime),
            patch("drive.command", return_value="[bench-events]"),
        ):
            return run_backend(
                SimpleNamespace(backend="native"),
                Path("reader"),
                folder,
                Path("probe"),
                None,
                "battery",
            )

    def test_start_failure_still_cleans_owned_processes(self):
        runtime = self.backend()
        runtime.launch.side_effect = RuntimeError("launch failed")
        with tempfile.TemporaryDirectory() as folder:
            folder = Path(folder)
            with self.assertRaisesRegex(RuntimeError, "launch failed"):
                self.run_backend(runtime, folder)
            runtime.stop_monitor.assert_called_once()
            runtime.close.assert_called_once()
            self.assertEqual(
                json.loads((folder / "failure.json").read_text())["error"],
                "launch failed",
            )

    def test_late_monitor_error_rejects_completed_run(self):
        runtime = self.backend()

        def stop():
            runtime.monitor_error = RuntimeError("Focus changed")

        runtime.stop_monitor.side_effect = stop
        with tempfile.TemporaryDirectory() as folder:
            with self.assertRaisesRegex(RuntimeError, "Focus changed"):
                self.run_backend(runtime, Path(folder))
            runtime.close.assert_called_once()

    def test_cleanup_failure_rejects_completed_run(self):
        runtime = self.backend()
        runtime.close.side_effect = RuntimeError("Reader exit failed: 3")
        with tempfile.TemporaryDirectory() as folder:
            folder = Path(folder)
            with self.assertRaisesRegex(RuntimeError, "Reader exit failed"):
                self.run_backend(runtime, folder)
            self.assertEqual(
                json.loads((folder / "failure.json").read_text())[
                    "cleanup_errors"
                ],
                ["Reader exit failed: 3"],
            )

    def test_quit_stops_remaining_steps(self):
        runtime = Runtime.__new__(Runtime)
        runtime.crop = "off"
        runtime.steps = ["quit", "key C-d"]
        runtime.send, runtime.marker, runtime.inspect = Mock(), Mock(), Mock()
        runtime.execute()
        runtime.send.assert_not_called()
        runtime.marker.assert_called_once_with("finished")

    def test_standalone_mix_sends_one_seeded_choice(self):
        runtime = Runtime.__new__(Runtime)
        runtime.send = Mock()
        runtime.step("mix 7 j | k")
        runtime.send.assert_called_once_with("key j")

    def test_resize_ignores_unrelated_geometry_and_accepts_noop(self):
        with tempfile.TemporaryDirectory() as folder:
            runtime = Kitty.__new__(Kitty)
            runtime.directory = Path(folder)
            runtime.geometry = [36, 100, 1000, 684]
            runtime.kitten, runtime.remote = Path("kitten"), "unix:test"
            geometry = runtime.directory / "geometry.json"
            geometry.write_text(json.dumps(runtime.geometry))

            def wait(predicate):
                self.assertFalse(predicate())
                geometry.write_text(json.dumps([36, 90, 900, 684]))
                self.assertFalse(predicate())
                geometry.write_text(json.dumps([36, 50, 500, 684]))
                self.assertTrue(predicate())

            runtime.wait = wait
            with patch("kitty.subprocess.run"):
                runtime.send("resize 500 700")
                runtime.wait = lambda predicate: self.assertTrue(predicate())
                runtime.send("resize 500 700")
            self.assertEqual(runtime.geometry, [36, 50, 500, 684])


if __name__ == "__main__":
    unittest.main()
