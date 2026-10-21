import hashlib
import tempfile
import unittest
from pathlib import Path

from scenario import scenario, validate_step


class ScenarioTests(unittest.TestCase):
    def test_metadata_rejects_unsafe_names_and_duplicate_headers(self):
        with tempfile.TemporaryDirectory() as folder:
            folder = Path(folder)
            pdf = folder / "sdm.pdf"
            pdf.write_bytes(b"hash validation input")
            sha = hashlib.sha256(pdf.read_bytes()).hexdigest()
            header = f"# pdf sdm {sha}\n# window 1000 700\n# crop off\n"
            path = folder / "scenario.txt"
            path.write_text(header + "key\tC-d\nquit\n")
            self.assertEqual(scenario(path, folder)[1], ["key C-d", "quit"])
            for text in (
                header + "# window 10 10\nquit",
                header,
                header.replace("pdf sdm", "pdf ../sdm") + "quit",
                header.replace(sha, "0" * 64) + "quit",
            ):
                path.write_text(text)
                with self.assertRaises(ValueError):
                    scenario(path, folder)

    def test_invalid_steps_fail_before_launch(self):
        for step in (
            "quit later",
            "wheel 0 65",
            "resize 0 100",
            "wait -1",
            "wait nan",
            "wait 0.5s",
            "mark",
            "repeat 0s @240 key j",
            "repeat 1s 240 key j",
            "repeat 1s @240 resize 500 500",
            "mix -1 j | k",
            "mix 7 j |",
            "unknown",
        ):
            with self.subTest(step=step), self.assertRaises(ValueError):
                validate_step(step)


if __name__ == "__main__":
    unittest.main()
