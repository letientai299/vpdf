import unittest
from unittest.mock import Mock

from tools.bench.live.runtime import Runtime, ready


class WindowTests(unittest.TestCase):
    def test_repeated_mark_waits_for_new_acknowledgement(self):
        runtime = Runtime.__new__(Runtime)
        rows = [dict(kind="mark", name="same", t_ms=1)]
        runtime.rows = lambda: rows
        runtime.send = Mock()

        def wait(predicate):
            self.assertFalse(predicate())
            rows.append(dict(kind="mark", name="same", t_ms=2))
            self.assertTrue(predicate())

        runtime.wait = wait
        runtime.marker("same")
        runtime.send.assert_called_once_with("mark same")

    def test_build_start_rejects_running_measurement(self):
        runtime = Runtime.__new__(Runtime)
        runtime.focus = ("kitty", 10)
        with self.assertRaisesRegex(RuntimeError, "Build started"):
            runtime.validate(
                dict(
                    front="kitty",
                    front_pid=10,
                    locked=False,
                    builds=["cargo:20"],
                )
            )

    def test_idle_requires_current_completed_work(self):
        row = dict(t_ms=30, built_ms=10, complete=True, busy=False)
        self.assertFalse(ready(row, 20, True))
        self.assertFalse(ready(dict(row, built_ms=25, busy=True), 20, True))
        self.assertFalse(
            ready(dict(row, built_ms=25, complete=False), 20, True)
        )
        self.assertTrue(ready(dict(row, built_ms=25), 20, True))
        self.assertTrue(ready(dict(row, built_ms=25, busy=True), 20, False))
        self.assertFalse(ready(dict(t_ms=30, complete=True), 20, True))

    def test_rejects_job_errors(self):
        for step in ("wait idle", "wait complete", "expect sharp true"):
            runtime = Runtime.__new__(Runtime)
            rows = []

            def marker(token):
                rows.extend(
                    [
                        dict(kind="mark", name=token, t_ms=100),
                        dict(
                            kind="frame",
                            t_ms=102,
                            built_ms=101,
                            complete=True,
                            busy=False,
                            missing=0,
                            stand_ins=0,
                            error="search failed",
                        ),
                    ]
                )

            runtime.marker = marker
            runtime.rows = lambda: rows
            runtime.wait = lambda predicate: predicate()
            with (
                self.subTest(step=step),
                self.assertRaisesRegex(
                    RuntimeError, "Scenario job failed: search failed"
                ),
            ):
                runtime.step(step)

    def test_checkpoint_tracks_motion(self):
        runtime = Runtime.__new__(Runtime)
        runtime.checkpoints = {}
        rows = []
        state = dict(
            document="sdm.pdf",
            pane=1,
            document_generation=0,
            page=1,
            scroll=0.0,
            error=None,
        )

        def marker(name):
            at = len(rows) * 10 + 10
            rows.extend(
                [
                    dict(kind="mark", name=name, t_ms=at),
                    dict(state, kind="frame", t_ms=at + 2, built_ms=at + 1),
                ]
            )

        runtime.marker = marker
        runtime.rows = lambda: rows
        runtime.wait = lambda predicate: self.assertTrue(predicate())
        runtime.step("mark keys")
        with self.assertRaises(AssertionError):
            runtime.step("expect moved keys")
        state["scroll"] = 10.0
        runtime.step("expect moved\tkeys")
        self.assertEqual(runtime.checkpoints["keys"]["scroll"], 0.0)

    def test_geometry_gate_ignores_stale_samples(self):
        runtime = Runtime.__new__(Runtime)
        runtime.focus = ("kitty", 10)
        runtime._pid = 20
        runtime.check_window = True
        runtime.window_since = 100
        state = dict(
            front="kitty",
            front_pid=10,
            pid=20,
            locked=False,
            sampled_unix_ns=99,
            occluded=True,
        )
        runtime.validate(state)
        with self.assertRaisesRegex(RuntimeError, "Window occluded"):
            runtime.validate(dict(state, sampled_unix_ns=101))
        runtime.validate(
            dict(state, sampled_unix_ns=102, occluded=False, display=1)
        )


if __name__ == "__main__":
    unittest.main()
