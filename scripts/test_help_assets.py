import subprocess
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import help_assets as help


class HelpAssetTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.output = self.root / "target/help"
        self.facts = {"schema": 1, "topics": [], "defaults": []}
        self.inputs = [
            "docs/users/intro.md",
            "docs/users/readme.md",
            "scripts/help/book.json",
            "scripts/help/manual.typ",
            "scripts/help/renderer.json",
            "scripts/help/notices/libertinus.txt",
            "scripts/help_assets.py",
            "crates/vpdf-pdf/assets/DejaVuSansMono.ttf",
            "crates/ui/src/topics.rs",
            "crates/ui/examples/help_metadata.rs",
            "crates/ui/Cargo.toml",
            "Cargo.lock",
            ".config/mise.toml",
        ]
        for name in self.inputs:
            path = self.root / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(name)

    def render(self, root, stage, facts):
        for name in help.OUTPUTS:
            (stage / name).write_text(name)

    def prepare(self):
        return help.prepare(self.root, self.output, self.facts)

    def check(self):
        return help.check_assets(self.root, self.output, self.facts)

    def test_reuse_and_input_changes(self):
        with patch.object(help, "render", side_effect=self.render) as render:
            first = self.prepare()
            original = (first / "base.pdf").stat().st_mtime_ns
            self.assertEqual(self.prepare(), first)
            self.assertEqual(self.check(), first)
            self.assertEqual(render.call_count, 1)
            self.assertEqual((first / "base.pdf").stat().st_mtime_ns, original)
            for name in self.inputs:
                with self.subTest(name=name):
                    path = self.root / name
                    content = path.read_text()
                    path.write_text(content + "changed")
                    with self.assertRaises(FileNotFoundError):
                        self.check()
                    self.assertNotEqual(self.prepare(), first)
                    path.write_text(content)
            added = self.root / "docs/users/new.md"
            added.write_text("new")
            self.assertNotEqual(self.prepare(), first)
            added.unlink()
            self.facts["topics"].append({"id": "new-action"})
            self.assertNotEqual(self.prepare(), first)

    def test_incomplete_and_corrupt_outputs(self):
        with patch.object(help, "render", side_effect=self.render):
            directory = self.prepare()
            for name in (*help.OUTPUTS, "manifest.json"):
                with self.subTest(name=name):
                    path = directory / name
                    original = path.read_bytes()
                    path.unlink()
                    with self.assertRaises(FileNotFoundError):
                        self.check()
                    path.write_bytes(original + b"corrupt")
                    with self.assertRaises(ValueError):
                        self.check()
                    path.write_bytes(original)

    def test_failure_preserves_published_assets(self):
        with patch.object(help, "render", side_effect=self.render):
            previous = self.prepare()
        (self.root / self.inputs[0]).write_text("changed")
        with patch.object(
            help, "render", side_effect=RuntimeError("renderer failed")
        ):
            with self.assertRaisesRegex(RuntimeError, "renderer failed"):
                self.prepare()
        self.assertEqual(
            sorted(p.name for p in self.output.iterdir()),
            [".lock", previous.name],
        )
        self.assertTrue((previous / "base.pdf").is_file())
        with self.assertRaises(FileNotFoundError):
            self.check()

    def test_inputs_cannot_change_during_generation(self):
        def changing(root, stage, facts):
            self.render(root, stage, facts)
            (root / self.inputs[0]).write_text("changed")

        with patch.object(help, "render", side_effect=changing):
            with self.assertRaisesRegex(
                ValueError, "changed during generation"
            ):
                self.prepare()
        self.assertEqual(list(self.output.iterdir()), [self.output / ".lock"])

    def test_inputs_change_while_waiting(self):
        source = self.root / self.inputs[0]
        with patch.object(help, "render", side_effect=self.render):
            first = self.prepare()
            with patch.object(
                help.fcntl,
                "flock",
                side_effect=lambda *_: source.write_text("edited during wait"),
            ):
                second = self.prepare()
        self.assertNotEqual(first, second)
        self.assertEqual(self.check(), second)

    def test_inputs_change_during_verification(self):
        with patch.object(help, "render", side_effect=self.render):
            self.prepare()
        verify = help.verify

        def changed(directory, expected):
            verify(directory, expected)
            (self.root / self.inputs[0]).write_text(
                "edited during verification"
            )

        for action in [self.check, self.prepare]:
            source = self.root / self.inputs[0]
            original = source.read_text()
            with (
                self.subTest(action=action.__name__),
                patch.object(help, "verify", side_effect=changed),
                self.assertRaisesRegex(
                    ValueError, "changed during verification"
                ),
            ):
                action()
            source.write_text(original)

    def test_chapter_contract(self):
        path = self.root / "docs/users/intro.md"
        topics = {"introduction": {"kind": "authored", "title": "Introduction"}}
        source = "# Introduction {#introduction}\n\nFollow [home][home].\n\n[home]: help:introduction\n"
        path.write_text(source)
        name, rendered = help.chapter(path, self.facts, topics)
        self.assertEqual(name, "introduction")
        self.assertIn('local-link("introduction"', rendered)
        for content in [
            source.replace("help:introduction", "help:unknown"),
            source.replace("{#introduction}", "{#unknown}"),
            source.replace("# Introduction", "# Renamed"),
            source + "\n{{keys:missing:normal}}\n",
            source + "\n[broken][missing]\n",
            source + "\n**unsupported**\n",
        ]:
            path.write_text(content)
            with self.subTest(content=content), self.assertRaises(ValueError):
                help.chapter(path, self.facts, topics)

    def test_renderer_diagnostics(self):
        for code, stderr in [
            (0, "warning: unknown font family"),
            (1, "failed"),
        ]:
            result = subprocess.CompletedProcess([], code, "", stderr)
            with (
                self.subTest(code=code),
                patch.object(help.subprocess, "run", return_value=result),
                self.assertRaisesRegex(ValueError, "Help renderer diagnostics"),
            ):
                help.render_tool("compile", env={})
        result = subprocess.CompletedProcess([], 0, "result", "")
        with patch.object(help.subprocess, "run", return_value=result):
            self.assertEqual(help.render_tool("eval", env={}), "result")

    def test_web_links_and_examples(self):
        path = self.root / "docs/users/readme.md"
        other = path.with_name("configuration.md")
        other.write_text("# Configuration {#configuration}\n\n## Live values\n")
        topics = {
            "introduction": {"kind": "authored", "title": "Manual"},
            "configuration": {"kind": "authored", "title": "Configuration"},
            "config": {"kind": "configuration", "title": "Snapshot"},
        }
        source = (
            "# Manual\n\n<!-- help-topic:introduction -->\n\n"
            "Read [values][values] or [snapshot][snapshot] or [upstream][web].\n\n"
            '```toml\n[reading]\nzoom = 1.0\n\n[ocr]\nlanguage = "eng"\n```\n\n'
            "- First item\n  with continuation.\n- Second item.\n\n"
            "<!-- help:snapshot=config -->\n\n"
            "[values]: configuration.md#live-values\n"
            "[snapshot]: configuration.md\n"
            "[web]:\n  <https://example.com/>\n"
        )
        path.write_text(source)
        _, rendered = help.chapter(path, self.facts, topics)
        self.assertIn('local-link("configuration.live-values"', rendered)
        self.assertIn('reference("config"', rendered)
        self.assertIn('link("https://example.com/"', rendered)
        self.assertIn('#code-block("[reading]\\nzoom', rendered)
        self.assertIn("First item with continuation.", rendered)
        for bad in [
            source.replace("#live-values", "#missing"),
            source.replace("snapshot=config", "snapshot=missing"),
            source.replace(
                "[snapshot]: configuration.md", "[absent]: configuration.md"
            ),
            source.replace("configuration.md#live-values", "../outside.md"),
            source + "\n![figure](panes.svg)\n",
        ]:
            path.write_text(bad)
            with self.subTest(source=bad), self.assertRaises(ValueError):
                help.chapter(path, self.facts, topics)

    def test_wrapped_markdown(self):
        refs, body = help.references(
            "Body.[^note]\n\n[^note]: First line\n"
            "    continued note\n    and another line.\n\nAfter note."
        )
        self.assertEqual(
            refs["^note"], "First line continued note and another line."
        )
        self.assertNotIn("continued note", body)
        self.assertIn("After note.", body)
        self.assertEqual(
            help.numbered(
                "3. First item with\n   continued text.\n4. Second item."
            ),
            (3, ["First item with continued text.", "Second item."]),
        )
        with self.assertRaisesRegex(ValueError, "list continuation"):
            help.numbered("1. First item.\nUnindented continuation.")

    def test_fenced_examples_stay_literal(self):
        path = self.root / "docs/users/readme.md"
        topics = {"introduction": {"kind": "authored", "title": "Manual"}}
        example = (
            "[home]: not-a-destination\n\n"
            "{{keys:missing:normal}}\n"
            "<!-- help:unknown=missing -->\n"
            "# Another title\n\n<!-- help-topic:unknown -->"
        )
        path.write_text(
            "# Manual\n\n<!-- help-topic:introduction -->\n\n"
            f"```text\n{example}\n```\n\n"
            "Follow [home][home].\n\n[home]: help:introduction\n"
        )
        _, rendered = help.chapter(path, self.facts, topics)
        self.assertIn(f"#code-block({help.quote(example)})", rendered)
        self.assertIn('local-link("introduction", "home")', rendered)


if __name__ == "__main__":
    unittest.main()
