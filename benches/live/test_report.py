import tempfile
import unittest
from pathlib import Path

from report import analyze, paired, summarize


class ReportTests(unittest.TestCase):
    def test_pairs_preserve_repeated_phase_names(self):
        def phase(value):
            return dict(
                name="keys",
                fresh_fps=value,
                gap_p99_ms=None,
                blank_ms=None,
                stand_in_ms=None,
                sharp_after_ms=None,
                cpu_mean=None,
                rss_peak=None,
            )

        result = paired(
            [
                dict(round=0, label="base", phases=[phase(10), phase(20)]),
                dict(round=0, label="candidate", phases=[phase(11), phase(24)]),
            ]
        )
        fps = [r for r in result if r["metric"] == "fresh_fps"]
        self.assertEqual(
            [(r["occurrence"], r["median_delta"]) for r in fps],
            [(1, 1), (2, 4)],
        )

    def test_sharp_waits_for_finished_animation(self):
        rows = [
            dict(kind="frame", t_ms=10, complete=True, busy=True),
            dict(kind="frame", t_ms=20, complete=True, busy=False),
        ]
        self.assertEqual(summarize(rows, 0, 30)["sharp_after_ms"], 20)

    def test_missing_fidelity_metrics_stay_unknown(self):
        phase = summarize([dict(kind="frame", t_ms=0, complete=False)], 0, 10)
        self.assertIsNone(phase["blank_ms"])
        self.assertIsNone(phase["stand_in_ms"])

    def test_frame_costs_and_output_rates(self):
        rows = [
            dict(kind="frame", t_ms=0, fresh=True, build_ms=2, bytes=100),
            dict(kind="frame", t_ms=10, fresh=False, build_ms=2, bytes=200),
            dict(kind="frame", t_ms=20, fresh=True, build_ms=6, bytes=300),
        ]
        phase = summarize(rows, 0, 30)
        self.assertEqual(phase["bytes"], 600)
        self.assertEqual(phase["bytes_per_s"], 20_000)
        self.assertEqual(phase["build_ms_p50"], 6)
        self.assertEqual(phase["build_ms_p99"], 6)
        self.assertIsNone(phase["upload_bytes"])

    def test_phase_carries_previous_frame(self):
        with tempfile.TemporaryDirectory() as directory:
            trace = Path(directory) / "trace.jsonl"
            trace.write_text(
                "\n".join(
                    [
                        '{"kind":"frame","t_ms":0,"fresh":true,"missing":1}',
                        '{"kind":"mark","t_ms":10,"name":"resize"}',
                        '{"kind":"frame","t_ms":30,"fresh":true,"complete":true}',
                        '{"kind":"sample","t_ms":50,"cpu":4,"rss":123}',
                        '{"kind":"mark","t_ms":60,"name":"end"}',
                    ]
                )
            )
            phase = analyze(trace)["phases"][1]
            self.assertEqual(phase["blank_ms"], 20)
            self.assertEqual(phase["sharp_after_ms"], 20)
            self.assertEqual(phase["cpu_mean"], 4)
            self.assertEqual(phase["rss_peak"], 123)

    def test_final_frame_is_counted(self):
        with tempfile.TemporaryDirectory() as directory:
            trace = Path(directory) / "trace.jsonl"
            trace.write_text('{"kind":"frame","t_ms":0,"complete":true}\n')
            report = analyze(trace)
            self.assertEqual(report["total"]["frames"], 1)
            self.assertEqual(report["phases"][0]["frames"], 1)

    def test_pairs_preserve_matching_rounds(self):
        def report(round_, label, value):
            phase = dict(
                name="keys",
                fresh_fps=value,
                gap_p99_ms=None,
                blank_ms=0,
                stand_in_ms=0,
                sharp_after_ms=None,
                cpu_mean=None,
                rss_peak=None,
            )
            return dict(round=round_, label=label, phases=[phase])

        rows = paired(
            [
                report(0, "base", 40),
                report(0, "candidate", 50),
                report(1, "base", 100),
                report(1, "candidate", 90),
                report(2, "base", 300),
            ]
        )
        fps = next(row for row in rows if row["metric"] == "fresh_fps")
        self.assertEqual(fps["samples"], 2)
        self.assertEqual(fps["median_delta"], 0)
        self.assertEqual((fps["min_delta"], fps["max_delta"]), (-10, 10))


if __name__ == "__main__":
    unittest.main()
