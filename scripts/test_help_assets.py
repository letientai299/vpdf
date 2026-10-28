import json
import shutil
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
            "docs/assets/figure.svg",
            "docs/licenses/dependencies.txt",
            "docs/licenses/native.txt",
            "LICENSE",
            "LICENSE-AGPL",
            "scripts/help/contents.json",
            "scripts/help/manual.typ",
            "scripts/help/renderer.json",
            "scripts/help/notices/iosevka.txt",
            "scripts/help_assets.py",
            "scripts/help_markdown.py",
            "scripts/help_font.py",
            "crates/ui/assets/VpdfMono.ttf",
            "crates/ui/src/topics.rs",
            "crates/ui/examples/help_metadata.rs",
            "crates/ui/Cargo.toml",
            "Cargo.toml",
            "Cargo.lock",
            ".config/mise.toml",
        ]
        for name in self.inputs:
            path = self.root / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(name)

    def test_cold_metadata_budget(self):
        def output(args, *, timeout, **kwargs):
            if args[0] == "rustc":
                self.assertEqual(timeout, 180)
                return "test-host\n"
            if timeout <= 180:
                raise subprocess.TimeoutExpired(args, timeout)
            self.assertLessEqual(timeout, 600)
            return json.dumps(self.facts)

        with patch.object(subprocess, "check_output", side_effect=output):
            self.assertEqual(help.export_facts(self.root), self.facts)

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

    def test_export_replaces_base_symlink(self):
        self.output.mkdir(parents=True)
        base = self.output / "base.pdf"
        base.write_bytes(b"base")
        pdf = self.output / "help.pdf"
        pdf.symlink_to(base.name)

        def export(*args, **kwargs):
            self.assertEqual(kwargs["cwd"], self.root)
            Path(args[-1]).write_bytes(b"readable manual")

        with patch.object(help, "run", side_effect=export):
            help.export_pdf(self.root, self.output)
        self.assertFalse(pdf.is_symlink())
        self.assertEqual(pdf.read_bytes(), b"readable manual")
        self.assertEqual(base.read_bytes(), b"base")

    def test_failed_export_preserves_manual(self):
        self.output.mkdir(parents=True)
        pdf = self.output / "help.pdf"
        pdf.write_bytes(b"previous manual")

        def fail(*args, **kwargs):
            Path(args[-1]).write_bytes(b"partial export")
            raise subprocess.CalledProcessError(1, args)

        with patch.object(help, "run", side_effect=fail):
            with self.assertRaises(subprocess.CalledProcessError):
                help.export_pdf(self.root, self.output)
        self.assertEqual(pdf.read_bytes(), b"previous manual")
        self.assertEqual(list(self.output.iterdir()), [pdf])

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
            sorted([".lock", previous.name]),
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
        source = "# Introduction\n\n<!-- help-topic:introduction -->\n\nFollow [home][home].\n\n[home]: help:introduction\n"
        path.write_text(source)
        name, rendered = help.chapter(path, self.facts, topics)
        self.assertEqual(name, "introduction")
        self.assertIn("link(<introduction>)", rendered)
        for content in [
            source.replace("help:introduction", "help:unknown"),
            source.replace("help-topic:introduction", "help-topic:unknown"),
            source.replace("# Introduction", "# Renamed"),
            source + "\n{{keys:missing:normal}}\n",
        ]:
            path.write_text(content)
            with self.subTest(content=content), self.assertRaises(ValueError):
                help.chapter(path, self.facts, topics)

    def test_literal_brackets(self):
        path = self.root / "docs/users/intro.md"
        path.write_text(
            "# Introduction\n\n<!-- help-topic:introduction -->\n\n"
            + r"Escaped \[key\]\[context\]."
            + "\n"
        )
        topics = {"introduction": {"kind": "authored", "title": "Introduction"}}
        _, rendered = help.chapter(path, self.facts, topics)
        self.assertIn("key", rendered)
        self.assertIn("context", rendered)

    def test_image_inventory(self):
        path = self.root / "docs/users/intro.md"
        (self.root / "Cargo.toml").write_text(
            '[package]\nrepository = "https://github.com/example/reader"\n'
        )
        image = self.root / "docs/assets/figure.svg"
        image.write_text(
            '<svg xmlns="http://www.w3.org/2000/svg" width="12" height="12"><rect width="12" height="12"/></svg>'
        )
        source = "# Introduction\n\n<!-- help-topic:introduction -->\n\n![Figure](../assets/figure.svg)\n"
        path.write_text(source)
        topics = {"introduction": {"kind": "authored", "title": "Introduction"}}
        stage = self.root / "stage"
        stage.mkdir()
        with patch.object(help, "ROOT", self.root):
            _, rendered = help.chapter(path, self.facts, topics, stage)
            self.assertIn("image(", rendered)
            self.assertEqual(
                next(stage.rglob("*.svg")).read_bytes(), image.read_bytes()
            )
            outside = self.root / "other.svg"
            outside.write_bytes(image.read_bytes())
            path.write_text(
                source.replace("../assets/figure.svg", "../../other.svg")
            )
            with self.assertRaisesRegex(ValueError, "outside inventoried"):
                help.chapter(path, self.facts, topics, stage)

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
        other.write_text(
            "# Configuration\n\n<!-- help-topic:configuration -->\n\n## Live values\n"
        )
        topics = {
            "introduction": {"kind": "authored", "title": "Manual"},
            "configuration": {"kind": "authored", "title": "Configuration"},
            "config": {"kind": "configuration", "title": "Snapshot"},
        }
        source = (
            "# Manual\n\n<!-- help-topic:introduction -->\n\n"
            "Read [values][values], [guide][guide], [snapshot][snapshot] or [upstream][web].\n\n"
            '```toml\n[reading]\nzoom = 1.0\n\n[ocr]\nlanguage = "eng"\n```\n\n'
            "- First item\n  with continuation.\n- Second item.\n\n"
            "<!-- help:snapshot=config -->\n\n"
            "[values]: configuration.md#live-values\n"
            "[snapshot]:\n  configuration.md\n"
            "[guide]: configuration.md\n"
            "[web]:\n  <https://example.com/>\n"
        )
        path.write_text(source)
        _, rendered = help.chapter(path, self.facts, topics)
        self.assertIn("link(<configuration.live-values>)", rendered)
        self.assertIn("link(<configuration>)[guide", rendered)
        self.assertIn('link("vpdf-help:config")', rendered)
        self.assertIn('link("https://example.com/"', rendered)
        self.assertIn("```toml\n[reading]\nzoom", rendered)
        self.assertIn("First item with continuation.", rendered)
        for bad in [
            source.replace("#live-values", "#missing"),
            source.replace("snapshot=config", "snapshot=missing"),
            source.replace("[snapshot]:", "[absent]:"),
            source.replace("configuration.md#live-values", "../outside.md"),
            source + "\n![figure](panes.svg)\n",
        ]:
            path.write_text(bad)
            with self.subTest(source=bad), self.assertRaises(ValueError):
                help.chapter(path, self.facts, topics)

    def test_overrides_preserve_examples(self):
        path = self.root / "docs/users/readme.md"
        topics = {
            "introduction": {"kind": "authored", "title": "Manual"},
            "config": {"kind": "configuration", "title": "Snapshot"},
        }
        example = "[first]: literal.md\n[second]: literal.md"
        path.write_text(
            "# Manual\n\n<!-- help-topic:introduction -->\n\n"
            + "```text\n"
            + example
            + "\n```\n\n"
            + "[First][first] and [second][second].\n\n"
            + "<!-- help:first=config -->\n<!-- help:second=config -->\n\n"
            + "[first]:\n  missing.md\n[second]: missing.md\n"
        )
        _, rendered = help.chapter(path, self.facts, topics)
        self.assertIn(example, rendered)
        self.assertEqual(rendered.count('link("vpdf-help:config")'), 2)

    def test_repository_links(self):
        folder = self.root / "etc/themes"
        folder.mkdir(parents=True)
        palette = folder / "green night.toml"
        palette.write_text('background = "#ffffff"')
        (self.root / "Cargo.toml").write_text(
            '[package]\nrepository = "https://github.com/example/reader"\n'
        )
        path = self.root / "docs/users/readme.md"
        source = (
            "# Manual\n\n<!-- help-topic:introduction -->\n\n"
            "Use [themes][themes] or [palette][palette].\n\n"
            "[themes]: ../../etc/themes/\n"
            "[palette]: <../../etc/themes/green night.toml>\n"
        )
        path.write_text(source)
        topics = {"introduction": {"kind": "authored", "title": "Manual"}}
        with patch.object(help, "ROOT", self.root):
            _, rendered = help.chapter(path, self.facts, topics)
            self.assertIn("/tree/main/etc/themes", rendered)
            self.assertIn("/blob/main/etc/themes/green%20night.toml", rendered)
            outside = self.root.parent / "outside-theme.toml"
            (folder / "outside.toml").symlink_to(outside)
            for target in [
                "../../etc/missing.toml",
                "../../../outside-theme.toml",
                "../../etc/themes/outside.toml",
            ]:
                path.write_text(source.replace("../../etc/themes/", target, 1))
                with self.subTest(target=target), self.assertRaises(ValueError):
                    help.chapter(path, self.facts, topics)

    def gfm(self):
        path = self.root / "docs/users/readme.md"
        topics = {"introduction": {"kind": "authored", "title": "Manual"}}
        self.facts["defaults"] = [
            {
                "action": "half_down",
                "mode": "normal",
                "keys": "<C-d>",
                "preferred": True,
            }
        ]
        path.write_text(
            "# Manual\n\n<!-- help-topic:introduction -->\n\n"
            "### Native UI text size\n\n"
            "| Action | Keys |\n| :--- | ---: |\n"
            "| **Open** | `Ctrl-o` |\n\n"
            "- [x] Done\n- [ ] Open\n  - Nested *item*\n\n"
            "~~Removed~~ ***Both*** and https://example.com and <user@example.com>.\n\n"
            "A hard break  \nnext line.\n\n"
            "Icons:  **** ** ******.\n\n"
            "Press {{keys:half_down:normal}}.\n\n"
            "<details><summary>More</summary><p>Visible <b>text</b></p></details>\n\n"
            "<kbd>Ctrl</kbd><br>New line\n\n"
            "---\n\nNote.[^note]\n\n[^note]: **Formatted** note.\n\n"
            "~~~toml\n[appearance]\ntheme = 'light'\n~~~\n\n"
            "Follow [size](#native-ui-text-size).\n"
        )
        return path, topics

    def test_gfm_rendering(self):
        path, topics = self.gfm()
        _, rendered = help.chapter(path, self.facts, topics)
        for expected in [
            "=== Native UI text size",
            "<introduction.native-ui-text-size>",
            "#table(",
            "table.header",
            "align: (left,right,)",
            "#strong[Open]",
            '#code("Ctrl-o")',
            '#code("<C-d>")',
            "☒ Done",
            "☐ Open",
            "#emph[item]",
            "#strike[Removed]",
            '#link("https://example.com")',
            '#link("mailto:user@example.com")',
            "More",
            "#strong[text]",
            "#divider()",
            "#footnote[#strong[Formatted] note.]",
            "```toml\n[appearance]",
            "#link(<introduction.native-ui-text-size>)",
        ]:
            with self.subTest(expected=expected):
                self.assertIn(expected, rendered)
        self.assertNotIn("<details>", rendered)

    def test_gfm_pdf(self):
        _, topics = self.gfm()
        shutil.rmtree(self.root / "scripts/help")
        shutil.copytree(help.ROOT / "scripts/help", self.root / "scripts/help")
        shutil.rmtree(self.root / "crates/ui/assets")
        shutil.copytree(
            help.ROOT / "crates/ui/assets", self.root / "crates/ui/assets"
        )
        (self.root / "docs/users/intro.md").unlink()
        (self.root / "scripts/help/contents.json").write_text(
            json.dumps({"sections": ["readme.md"]})
        )
        stage = self.root / "stage"
        stage.mkdir()
        facts = {
            **self.facts,
            "topics": [{"id": key, **value} for key, value in topics.items()],
        }
        help.render(self.root, stage, facts)
        inspected = help.run("qpdf", "--json", str(stage / "base.pdf"))
        for style in ["Regular", "Bold", "Italic", "BoldItalic"]:
            self.assertIn("VpdfMono-" + style, inspected)

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
        self.assertIn(example, rendered)
        self.assertIn("link(<introduction>)[home |introduction|]", rendered)


if __name__ == "__main__":
    unittest.main()
