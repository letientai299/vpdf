import subprocess
import tempfile
import unittest
from pathlib import Path

from tools.package.notes import render

COMPARE = "[compare]: https://example.com/vpdf/compare/"


class Repository:
    def __init__(self, directory):
        self.root = Path(directory)
        self.git("init", "-q", "-b", "main")
        self.git("config", "user.name", "Test")
        self.git("config", "user.email", "test@example.com")
        (self.root / "Cargo.toml").write_text(
            '[package]\nrepository = "https://example.com/vpdf"\n'
        )

    def git(self, *args):
        return subprocess.run(
            ["git", *args],
            cwd=self.root,
            check=True,
            capture_output=True,
            text=True,
        ).stdout.strip()

    def commit(self, message, path="src/lib.rs", tag=None):
        file = self.root / path
        file.parent.mkdir(parents=True, exist_ok=True)
        with file.open("a") as stream:
            stream.write(f"{message}\n")
        self.git("add", "--", path)
        self.git("commit", "-q", "-m", message)
        if tag:
            self.git("tag", tag)


class NotesTests(unittest.TestCase):
    def setUp(self):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.repo = Repository(directory.name)

    def test_render(self):
        repo = self.repo
        repo.commit("feat: first release", tag="v0.1.0")
        repo.commit("feat(search): add regex search")
        repo.commit("docs: explain search")
        repo.commit("fix: keep cursor on reload")
        repo.commit("refactor!: rename config keys")
        repo.commit("perf: cache outlines\n\nBREAKING CHANGE: new path")
        repo.commit("feat(ui)!: drop the old theme")
        repo.commit("fix: rename flag\n\nBREAKING-CHANGE: new flag")
        repo.commit("fix: stabilize a flaky test", path="tests/it.rs")
        repo.commit("perf: speed up checks", path="src/sidebar/tests.rs")
        repo.commit("not conventional")
        repo.commit("feat: shiny thing")
        repo.git("revert", "--no-edit", "HEAD")
        repo.commit("chore: tidy\n\nnote\x1efeat: injected entry")
        repo.commit("feat: preview build", tag="v0.2.0-rc.1")
        repo.commit("fix: drop stale tiles", tag="v0.2.0")
        repo.commit("feat: unreleased work")
        for tag, expected in [
            ("v0.1.0", "# Vpdf v0.1.0\n\n## Features\n\n- First release"),
            (
                "v0.2.0",
                "# Vpdf v0.2.0\n\n## Breaking changes\n\n"
                "- Rename config keys\n- Cache outlines\n"
                "- Drop the old theme\n- Rename flag\n\n"
                "## Features\n\n- Add regex search\n- Preview build\n\n"
                "## Fixes\n\n- Keep cursor on reload\n- Drop stale tiles\n\n"
                "[Changes since v0.1.0][compare]\n\n"
                f"{COMPARE}v0.1.0...v0.2.0",
            ),
            (
                "v0.3.0",
                "# Vpdf v0.3.0\n\n## Features\n\n- Unreleased work\n\n"
                "[Changes since v0.2.0][compare]\n\n"
                f"{COMPARE}v0.2.0...v0.3.0",
            ),
        ]:
            with self.subTest(tag=tag):
                self.assertEqual(render(repo.root, tag), expected)

    def test_rewritten_tag(self):
        repo = self.repo
        repo.commit("feat: base")
        repo.git("switch", "-q", "-c", "old")
        repo.commit("fix: shipped once", tag="v0.1.0")
        repo.git("switch", "-q", "main")
        repo.commit("fix: shipped once")
        repo.commit("feat: new work", tag="v0.2.0")
        self.assertIn(
            "## Features\n\n- New work\n\n[", render(repo.root, "v0.2.0")
        )
        self.assertNotIn("Shipped once", render(repo.root, "v0.2.0"))

    def test_quiet_release(self):
        self.repo.commit("feat: first release", tag="v0.1.0")
        self.repo.commit("ci: bump runners", tag="v0.1.1")
        self.assertIn(
            "No user-facing changes.",
            render(self.repo.root, "v0.1.1").split("\n\n"),
        )

    def test_invalid_tag(self):
        self.repo.commit("feat: first release", tag="v0.1.0")
        for tag in ["1.2.3", "v1.2.3foo", "v1.2.3-rc.1", "v01.2.3"]:
            with self.subTest(tag=tag), self.assertRaises(ValueError):
                render(self.repo.root, tag)

    def test_shallow_clone(self):
        self.repo.commit("feat: first release", tag="v0.1.0")
        self.repo.commit("feat: second", tag="v0.2.0")
        with tempfile.TemporaryDirectory() as directory:
            subprocess.run(
                ["git", "clone", "-q", "--depth", "1", "--no-local"]
                + [self.repo.root.as_uri(), directory],
                check=True,
            )
            with self.assertRaisesRegex(RuntimeError, "full history"):
                render(Path(directory), "v0.3.0")
