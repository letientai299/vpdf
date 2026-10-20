import os
from pathlib import Path
import plistlib
import subprocess
import sys
import tempfile
import unittest

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
                ["sh", "scripts/install-app.sh"], cwd=ROOT,
                env={**os.environ, "HOME": str(home),
                     "PATH": str(commands) + os.pathsep + os.environ["PATH"]},
                timeout=60,
            )
            self.assertNotEqual(result.returncode, 0)
            self.assertFalse((home / "signing-ran").exists())
            self.assertEqual(list((home / "Applications").iterdir()), [])

    def test_macos_minimum(self):
        with tempfile.TemporaryDirectory() as temporary:
            source = Path(temporary) / "main.c"
            source.write_text("int main(void) { return 0; }\n")
            for target, expected in (("x86_64-apple-macos10.12", "10.12"),
                                     ("arm64-apple-macos11", "11.0")):
                with self.subTest(target=target):
                    binary = Path(temporary) / target
                    subprocess.run(["xcrun", "clang", "-target", target,
                                    source, "-o", binary], check=True,
                                   timeout=60)
                    minimum = subprocess.check_output(
                        ["sh", "scripts/macos-minimum.sh", binary],
                        cwd=ROOT, text=True, timeout=60,
                    ).strip()
                    self.assertEqual(minimum, expected)

    def verify_app(self, app):
        subprocess.run(["codesign", "--verify", "--strict", app], check=True)
        with (app / "Contents/Info.plist").open("rb") as source:
            info = plistlib.load(source)
        self.assertEqual(info["CFBundlePackageType"], "APPL")
        self.assertEqual(info["CFBundleName"], "Vpdf")
        self.assertEqual(info["CFBundleDisplayName"], "Vpdf")
        icon = app / "Contents/Resources" / info["CFBundleIconFile"]
        self.assertEqual(icon.read_bytes(),
                         (ROOT / "docs/assets/vpdf.icns").read_bytes())
        binary = app / "Contents/MacOS" / info["CFBundleExecutable"]
        version = subprocess.check_output([binary, "--version"], text=True)
        self.assertEqual(version.strip(),
                         f"vpdf-gui {info['CFBundleShortVersionString']}")
        self.assertEqual(info["CFBundleVersion"],
                         info["CFBundleShortVersionString"])
        for name in ("LICENSE", "LICENSE-AGPL"):
            self.assertEqual((app / "Contents/Resources" / name).read_bytes(),
                             (ROOT / name).read_bytes())

    def test_disk_image(self):
        self.verify_app(DIST / "Vpdf.app")
        with tempfile.TemporaryDirectory(prefix="vpdf-dmg-") as temporary:
            mount = Path(temporary) / "volume"
            subprocess.run([
                "hdiutil", "attach", "-quiet", "-readonly", "-nobrowse",
                "-mountpoint", mount, DIST / "Vpdf.dmg",
            ], check=True, timeout=60)
            try:
                self.assertEqual({p.name for p in mount.iterdir()
                                  if not p.name.startswith(".")},
                                 {"Vpdf.app", "Applications"})
                self.assertEqual(os.readlink(mount / "Applications"),
                                 "/Applications")
                self.verify_app(mount / "Vpdf.app")
                self.assertEqual(
                    (mount / "Vpdf.app/Contents/MacOS/vpdf-gui").read_bytes(),
                    (DIST / "Vpdf.app/Contents/MacOS/vpdf-gui").read_bytes(),
                )
            finally:
                subprocess.run(["hdiutil", "detach", "-quiet", "-force", mount],
                               check=True, timeout=60)

    def test_icon_resolutions(self):
        with tempfile.TemporaryDirectory() as temporary:
            icons = Path(temporary) / "Vpdf.iconset"
            subprocess.run([
                "iconutil", "-c", "iconset", "-o", icons,
                DIST / "Vpdf.app/Contents/Resources/Vpdf.icns",
            ], check=True, timeout=60)
            for size in (16, 32, 128, 256, 512):
                for scale, suffix in ((1, ""), (2, "@2x")):
                    png = icons / f"icon_{size}x{size}{suffix}.png"
                    header = png.read_bytes()[:24]
                    self.assertEqual(header[:8], b"\x89PNG\r\n\x1a\n")
                    self.assertEqual(int.from_bytes(header[16:20], "big"),
                                     size * scale)
                    self.assertEqual(header[16:20], header[20:24])

    def test_install_replaces(self):
        with tempfile.TemporaryDirectory(prefix="vpdf home ") as temporary:
            home = Path(temporary)
            applications = home / "Applications"
            for attempt in range(2):
                with self.subTest(attempt=attempt):
                    subprocess.run(
                        ["sh", "scripts/install-app.sh"], cwd=ROOT,
                        env={**os.environ, "HOME": str(home)}, check=True,
                        timeout=60,
                    )
                    app = applications / "Vpdf.app"
                    self.verify_app(app)
                    self.assertFalse((app / "stale").exists())
                    self.assertEqual({p.name for p in applications.iterdir()},
                                     {"Vpdf.app"})
                    (app / "stale").write_text("old installation")


if __name__ == "__main__":
    unittest.main()
