import unittest
from pathlib import Path
from unittest.mock import patch

from tools.bench.live.drive import Native
from tools.demo import native


class AppTests(unittest.TestCase):
    def app(self, scale):
        app = native.App.__new__(native.App)
        app.size, app.place = (1600, 900), (3940, 80)
        app.sent = []
        app.bounds = dict(X=3940 * scale, Y=80 * scale, Width=1600 * scale)

        def send(line):
            app.sent.append(line)
            verb, *numbers = line.split()
            if verb == "place":
                app.bounds["X"] = int(numbers[0]) * scale
                app.bounds["Y"] = int(numbers[1]) * scale
            if verb == "resize":
                app.bounds["Width"] = int(numbers[0]) * scale

        app.send = send
        app.marker = lambda name: None
        app.inspect = lambda: dict(
            front="other", front_pid=1, window=dict(app.bounds)
        )
        app.validate = lambda state: None
        return app

    def test_ui_scale_correction(self):
        for scale, sent in (
            (1, []),
            (0.9, ["place 4378 89", "resize 1778 1000"]),
        ):
            with (
                self.subTest(scale=scale),
                patch.object(Native, "enable_window") as enabled,
            ):
                app = self.app(scale)
                app.enable_window()
                self.assertEqual(app.sent, sent)
                enabled.assert_called_once()

    def test_page_targets(self):
        args = native.arg_parser().parse_args(["--corpus", "/corpus dir"])
        with patch.object(native, "App") as app:
            native.launch(args, "app-sidebar", Path("/tmp"), Path("/probe"))
        parse = app.call_args.args[4]
        files, steps, size, crop = parse()
        page, documents, _ = native.SCENES["app-sidebar"]
        self.assertEqual(len(files), len(documents))
        self.assertTrue(files[0].startswith("file:///corpus%20dir/"))
        self.assertTrue(files[0].endswith(f".pdf#page={page}"))
        self.assertEqual((steps, size, crop), ([], (1600, 900), "off"))
