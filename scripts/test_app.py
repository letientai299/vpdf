import json
import os
import plistlib
import re
import shutil
import signal
import socket
import subprocess
import sys
import tempfile
import time
import unittest
from pathlib import Path

import replace_app

ROOT = Path(__file__).resolve().parent.parent
DIST = Path(os.environ.get("CARGO_TARGET_DIR", ROOT / "target")) / "dist"


@unittest.skipUnless(sys.platform == "darwin", "Requires macOS")
class AppTests(unittest.TestCase):
    def test_atomic_replacement(self):
        with tempfile.TemporaryDirectory() as temporary:
            destination = Path(temporary) / "installed.app"
            destination.mkdir()
            (destination / "previous").touch()
            source = Path(temporary) / "missing.app"
            with self.assertRaises(OSError):
                replace_app.replace(source, destination)
            self.assertTrue((destination / "previous").exists())
            source.mkdir()
            (source / "new").touch()
            replace_app.replace(source, destination)
            self.assertTrue((destination / "new").exists())
            self.assertTrue((source / "previous").exists())

    def test_install_interrupt(self):
        with tempfile.TemporaryDirectory() as temporary:
            home = Path(temporary)
            commands = home / "bin"
            commands.mkdir()
            for name, script in {
                "ditto": 'kill -TERM "$PPID"\n',
                "codesign": 'touch "$HOME/signing-ran"\n',
            }.items():
                command = commands / name
                command.write_text("#!/bin/sh\n" + script)
                command.chmod(0o755)
            result = subprocess.run(
                ["sh", "scripts/install-app.sh"],
                cwd=ROOT,
                env={
                    **os.environ,
                    "HOME": str(home),
                    "PATH": str(commands) + os.pathsep + os.environ["PATH"],
                },
                timeout=60,
            )
            self.assertNotEqual(result.returncode, 0)
            self.assertFalse((home / "signing-ran").exists())
            self.assertEqual(list((home / "Applications").iterdir()), [])

    def test_macos_minimum(self):
        with tempfile.TemporaryDirectory() as temporary:
            source = Path(temporary) / "main.c"
            source.write_text("int main(void) { return 0; }\n")
            for target, expected in (
                ("x86_64-apple-macos10.12", "10.12"),
                ("arm64-apple-macos11", "11.0"),
            ):
                with self.subTest(target=target):
                    binary = Path(temporary) / target
                    subprocess.run(
                        [
                            "xcrun",
                            "clang",
                            "-target",
                            target,
                            source,
                            "-o",
                            binary,
                        ],
                        check=True,
                        timeout=60,
                    )
                    minimum = subprocess.check_output(
                        ["sh", "scripts/macos-minimum.sh", binary],
                        cwd=ROOT,
                        text=True,
                        timeout=60,
                    ).strip()
                    self.assertEqual(minimum, expected)

    def verify_config_output(self, binary):
        with tempfile.TemporaryDirectory(prefix="vpdf-config-") as temporary:
            config = Path(temporary) / "broken.toml"
            config.write_text("[broken")
            for flag in ("--help", "--version"):
                result = subprocess.run(
                    [binary, "--config", config, flag],
                    capture_output=True,
                    text=True,
                    timeout=30,
                    check=True,
                )
                self.assertEqual(result.stderr, "")

        reader, writer = socket.socketpair()
        reader.close()
        with writer:
            result = subprocess.run(
                [binary, "--version"],
                stdout=writer,
                stderr=subprocess.PIPE,
                timeout=30,
                check=True,
            )
        self.assertEqual(result.stderr, b"")

    def verify_app(self, app):
        subprocess.run(["codesign", "--verify", "--strict", app], check=True)
        with (app / "Contents/Info.plist").open("rb") as source:
            info = plistlib.load(source)
        self.assertEqual(info["CFBundlePackageType"], "APPL")
        self.assertEqual(info["CFBundleName"], "Vpdf")
        self.assertEqual(info["CFBundleDisplayName"], "Vpdf")
        self.assertTrue(
            any(
                "com.adobe.pdf" in document.get("LSItemContentTypes", [])
                and document.get("CFBundleTypeRole") == "Viewer"
                for document in info.get("CFBundleDocumentTypes", [])
            )
        )
        icon = app / "Contents/Resources" / info["CFBundleIconFile"]
        self.assertEqual(
            icon.read_bytes(), (ROOT / "docs/assets/vpdf.icns").read_bytes()
        )
        binary = app / "Contents/MacOS" / info["CFBundleExecutable"]
        self.verify_config_output(binary)
        version = subprocess.check_output([binary, "--version"], text=True)
        lines = version.splitlines()
        self.assertIn("https://github.com/letientai299/vpdf", lines)
        self.assertTrue(lines[0].startswith("Vpdf:"))
        self.assertTrue(lines[-1].startswith("vpdf-gui "))
        self.assertRegex(version, r"(?m)^Help assets: [0-9a-f]{64}$")
        notices = (
            app / "Contents/Resources/third-party-notices.txt"
        ).read_text()
        for path in (ROOT / "scripts/help/notices").glob("*.txt"):
            self.assertIn(path.read_text(), notices)
        for path in (ROOT / "docs/licenses").glob("*.txt"):
            self.assertIn(path.read_text(), notices)
        metadata = json.loads(
            subprocess.check_output(
                [
                    "cargo",
                    "metadata",
                    "--locked",
                    "--no-deps",
                    "--format-version",
                    "1",
                ],
                cwd=ROOT,
                text=True,
            )
        )
        package = next(
            p for p in metadata["packages"] if p["name"] == "vpdf-gui"
        )
        self.assertEqual(info["CFBundleShortVersionString"], package["version"])
        self.assertEqual(
            info["CFBundleVersion"], info["CFBundleShortVersionString"]
        )
        for name in ("LICENSE", "LICENSE-AGPL"):
            self.assertEqual(
                (app / "Contents/Resources" / name).read_bytes(),
                (ROOT / name).read_bytes(),
            )

    def test_disk_image(self):
        self.verify_app(DIST / "Vpdf.app")
        with tempfile.TemporaryDirectory(prefix="vpdf-dmg-") as temporary:
            mount = Path(temporary) / "volume"
            subprocess.run(
                [
                    "hdiutil",
                    "attach",
                    "-quiet",
                    "-readonly",
                    "-nobrowse",
                    "-mountpoint",
                    mount,
                    DIST / "Vpdf.dmg",
                ],
                check=True,
                timeout=60,
            )
            try:
                self.assertEqual(
                    {
                        p.name
                        for p in mount.iterdir()
                        if not p.name.startswith(".")
                    },
                    {"Vpdf.app", "Applications"},
                )
                self.assertEqual(
                    os.readlink(mount / "Applications"), "/Applications"
                )
                self.verify_app(mount / "Vpdf.app")
                self.assertEqual(
                    (mount / "Vpdf.app/Contents/MacOS/vpdf-gui").read_bytes(),
                    (DIST / "Vpdf.app/Contents/MacOS/vpdf-gui").read_bytes(),
                )
            finally:
                subprocess.run(
                    ["hdiutil", "detach", "-quiet", "-force", mount],
                    check=True,
                    timeout=60,
                )

    def test_icon_resolutions(self):
        with tempfile.TemporaryDirectory() as temporary:
            icons = Path(temporary) / "Vpdf.iconset"
            subprocess.run(
                [
                    "iconutil",
                    "-c",
                    "iconset",
                    "-o",
                    icons,
                    DIST / "Vpdf.app/Contents/Resources/Vpdf.icns",
                ],
                check=True,
                timeout=60,
            )
            for size in (16, 32, 128, 256, 512):
                for scale, suffix in ((1, ""), (2, "@2x")):
                    png = icons / f"icon_{size}x{size}{suffix}.png"
                    header = png.read_bytes()[:24]
                    self.assertEqual(header[:8], b"\x89PNG\r\n\x1a\n")
                    self.assertEqual(
                        int.from_bytes(header[16:20], "big"), size * scale
                    )
                    self.assertEqual(header[16:20], header[20:24])

    def test_install_replaces(self):
        with tempfile.TemporaryDirectory(prefix="vpdf home ") as temporary:
            home = Path(temporary)
            applications = home / "Applications"
            for attempt in range(2):
                with self.subTest(attempt=attempt):
                    subprocess.run(
                        ["sh", "scripts/install-app.sh"],
                        cwd=ROOT,
                        env={**os.environ, "HOME": str(home)},
                        check=True,
                        timeout=60,
                    )
                    app = applications / "Vpdf.app"
                    self.verify_app(app)
                    self.assertFalse((app / "stale").exists())
                    self.assertEqual(
                        {p.name for p in applications.iterdir()}, {"Vpdf.app"}
                    )
                    (app / "stale").write_text("old installation")


@unittest.skipUnless(
    sys.platform == "darwin" and os.environ.get("VPDF_TEST_OPEN") == "1",
    "Requires an interactive macOS session; run mise run test:app:open",
)
class DocumentTests(unittest.TestCase):
    def wait(self, condition):
        until = time.monotonic() + 30
        while time.monotonic() < until:
            if result := condition():
                return result
            time.sleep(0.05)
        self.fail("Document event timed out")

    def test_document_events(self):
        register = (
            "/System/Library/Frameworks/CoreServices.framework/Frameworks/"
            "LaunchServices.framework/Support/lsregister"
        )
        with tempfile.TemporaryDirectory(prefix="vpdf-open-") as temporary:
            root = Path(temporary).resolve()
            app = root / "Vpdf.app"
            shutil.copytree(DIST / "Vpdf.app", app)
            plist = app / "Contents/Info.plist"
            info = plistlib.loads(plist.read_bytes())
            bundle = "io.github.letientai299.vpdf.test." + root.name
            info["CFBundleIdentifier"] = bundle
            plist.write_bytes(plistlib.dumps(info))
            subprocess.run(
                ["codesign", "--force", "--sign", "-", "--timestamp=none", app],
                check=True,
                timeout=30,
            )
            trace = root / "trace.jsonl"
            files = [
                root / name
                for name in (
                    "report.pdf",
                    "báo cáo #1.pdf",
                    "third.pdf",
                    "fourth.pdf",
                )
            ]
            for path in files:
                shutil.copyfile(ROOT / "tmp/corpus/paper.pdf", path)
            binary = app / "Contents/MacOS" / info["CFBundleExecutable"]

            def pids():
                result = subprocess.run(
                    ["pgrep", "-f", re.escape(str(binary))],
                    capture_output=True,
                    text=True,
                    timeout=10,
                )
                return [int(pid) for pid in result.stdout.split()]

            def opened(path, tab):
                if not trace.exists():
                    return False
                for line in trace.read_text().splitlines():
                    try:
                        row = json.loads(line)
                    except json.JSONDecodeError:
                        continue
                    self.assertFalse(row.get("error"), row)
                    if (
                        row.get("document") == str(path)
                        and row.get("tab") == tab
                        and row.get("complete")
                        and not row.get("busy")
                        and not row.get("blank")
                    ):
                        return True
                return False

            try:
                subprocess.run(
                    [
                        "open",
                        "-g",
                        "-n",
                        "-a",
                        app,
                        files[0],
                        "--env",
                        f"VPDF_TRACE={trace}",
                        "--env",
                        "VPDF_CACHE=off",
                        "--env",
                        f"XDG_STATE_HOME={root / 'state'}",
                        "--stdout",
                        root / "stdout",
                        "--stderr",
                        root / "stderr",
                        "--args",
                        "--background",
                        "-u",
                        "NONE",
                    ],
                    check=True,
                    timeout=30,
                )
                self.wait(lambda: opened(files[0], 1))
                owners = pids()
                self.assertEqual(len(owners), 1)
                for paths, tab in [(files[1:2], 2), (files[2:], 4)]:
                    subprocess.run(
                        ["open", "-g", "-a", app, *paths],
                        check=True,
                        timeout=30,
                    )
                    self.wait(lambda: opened(paths[-1], tab))
                    self.assertEqual(pids(), owners)
                subprocess.run(
                    [
                        "osascript",
                        "-e",
                        f'tell application id "{bundle}" to quit',
                    ],
                    check=True,
                    timeout=30,
                )
                self.wait(lambda: not pids())
            finally:
                for pid in pids():
                    try:
                        os.kill(pid, signal.SIGTERM)
                    except ProcessLookupError:
                        pass
                self.wait(lambda: not pids())
                subprocess.run([register, "-u", app], check=True, timeout=30)


if __name__ == "__main__":
    unittest.main()
