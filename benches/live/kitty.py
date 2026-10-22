"""Background Kitty with a persistent pty relay."""

import json
import os
import shutil
import signal
import socket
import subprocess
import sys
import tempfile
import time
from pathlib import Path

from keys import encode, mark, wheel
from runtime import Runtime

from scenario import digest


class Kitty(Runtime):
    def __init__(self, args, binary, directory, probe, parse):
        super().__init__(args, directory, probe)
        self.files, self.steps, self.size, self.crop = parse(
            args.scenario, args.corpus
        )
        self.size = tuple(args.window or self.size)
        self.place = tuple(args.place or self.choose_place(self.size))
        self.binary = directory / "vpdf"
        shutil.copy2(binary, self.binary)
        self.source_sha256 = digest(self.binary)
        self.control = None
        self.reader = None
        self.relay_pid = None
        self.temporary = tempfile.TemporaryDirectory(prefix="vb-", dir="/tmp")
        folder = Path(self.temporary.name)
        self.socket = folder / "relay"
        self.remote_path = folder / "kitty"
        self.remote = "unix:" + str(self.remote_path)
        self.kitten = args.kitty / "Contents/MacOS/kitten"
        self.result = directory / "relay.json"
        specification = directory / "relay-spec.json"
        specification.write_text(
            json.dumps(
                {
                    "command": [
                        str(self.binary.resolve()),
                        "--no-config",
                        *map(str, self.files),
                    ],
                    "env": {
                        "VPDF_TRACE": str(self.trace),
                        "VPDF_CACHE": "off",
                        "XDG_STATE_HOME": str(directory / "state"),
                        "XDG_CACHE_HOME": str(directory / "cache"),
                    },
                    "socket": str(self.socket),
                    "result": str(self.result),
                    "timeout": args.timeout,
                }
            )
        )
        self.relay = Path(__file__).with_name("pty_bridge.py")
        self.specification = specification

    def launch(self):
        subprocess.run(
            [
                "open",
                "-g",
                "-n",
                "-a",
                str(self.args.kitty),
                "--stdout",
                str(self.directory / "kitty-stdout"),
                "--stderr",
                str(self.directory / "kitty-stderr"),
                "--args",
                "--config",
                "NONE",
                "--listen-on",
                self.remote,
                "--start-as",
                "hidden",
                "--single-instance",
                "--instance-group",
                self.remote_path.parent.name,
                "-o",
                "allow_remote_control=socket-only",
                "-o",
                f"initial_window_width={self.size[0]}",
                "-o",
                f"initial_window_height={self.size[1]}",
                "-o",
                "remember_window_size=no",
                "-o",
                "remember_window_position=no",
                "-o",
                "confirm_os_window_close=0",
                "-o",
                "macos_quit_when_last_window_closed=no",
                "-o",
                "window_padding_width=0",
                "-o",
                "font_size=16",
                "--directory",
                str(self.directory),
                sys.executable,
                "-B",
                str(self.relay),
                str(self.specification),
            ],
            check=True,
        )

    def raw(self, data):
        self.watch()
        self.control.sendall(data)

    def send(self, text):
        verb, _, rest = text.partition(" ")
        if verb == "key":
            self.raw(encode(rest))
        elif verb == "wheel":
            self.raw(wheel(*map(int, rest.split()), self.geometry))
        elif verb == "mark":
            self.raw(mark(rest))
        elif verb == "resize":
            width, height = map(int, rest.split())
            previous = self.geometry
            rows, columns, pixels_x, pixels_y = previous
            if min(previous) <= 0:
                raise ValueError("Invalid terminal geometry")
            cell_width, cell_height = pixels_x // columns, pixels_y // rows
            if (
                min(cell_width, cell_height) <= 0
                or width < cell_width
                or height < 2 * cell_height
            ):
                raise ValueError("Resize grid is too small")
            expected = [
                height // cell_height,
                width // cell_width,
                width // cell_width * cell_width,
                height // cell_height * cell_height,
            ]
            subprocess.run(
                [
                    str(self.kitten),
                    "@",
                    "--to",
                    self.remote,
                    "resize-os-window",
                    "--unit",
                    "pixels",
                    "--width",
                    str(width),
                    "--height",
                    str(height),
                ],
                check=True,
                capture_output=True,
            )
            geometry = self.directory / "geometry.json"

            def resized():
                if not geometry.exists():
                    return False
                current = json.loads(geometry.read_text())
                if current != expected:
                    return False
                self.geometry = current
                return True

            self.wait(resized)
        elif verb == "place":
            if tuple(map(int, rest.split())) != self.place:
                raise ValueError("Kitty placement is launch-only")
        else:
            raise ValueError(f"Unsupported Kitty step: {verb}")

    def identify(self):
        result = subprocess.run(
            ["pgrep", "-f", self.remote], capture_output=True, text=True
        )
        if result.stdout.strip():
            self.pid = int(result.stdout.split()[0])
        return self.pid is not None

    def run(self):
        self.wait(self.identify)
        self.wait(self.remote_path.exists)
        self.wait(self.result.exists)
        metadata = json.loads(self.result.read_text())
        self.launched_ns = metadata["launched_unix_ns"]
        self.geometry = metadata["geometry"]
        self.reader = metadata["pid"]
        self.relay_pid = metadata["relay_pid"]
        self.pid = metadata["kitty_pid"]
        self.control = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self.control.settimeout(3)
        self.wait(self.socket.exists)
        self.control.connect(str(self.socket))
        self.wait(lambda: any(r.get("complete") for r in self.rows()))
        placed = subprocess.run(
            [
                str(self.kitten),
                "@",
                "--to",
                self.remote,
                "kitten",
                str(Path(__file__).with_name("place.py")),
                *map(str, self.place),
            ],
            check=True,
            capture_output=True,
        )
        (self.directory / "placement.txt").write_bytes(placed.stdout)
        self.wait(
            lambda: (
                abs(
                    self.inspect().get("window", {}).get("X", float("inf"))
                    - self.place[0]
                )
                < 2
            )
        )
        self.marker("placed")
        self.enable_window()
        return self.execute()

    def close(self):
        if self.reader is None and self.result.exists():
            metadata = json.loads(self.result.read_text())
            self.reader = metadata["pid"]
            self.relay_pid = metadata["relay_pid"]
            self.pid = metadata["kitty_pid"]

        if self.pid is None:
            self.identify()
        if self.control:
            try:
                self.control.sendall(encode("esc : q a enter"))
            except (OSError, RuntimeError, TimeoutError):
                pass
            self.control.close()
        if self.reader:
            until = time.monotonic() + 3
            while time.monotonic() < until:
                try:
                    os.kill(self.reader, 0)
                except ProcessLookupError:
                    break
                time.sleep(0.05)
            else:
                try:
                    os.killpg(self.reader, signal.SIGTERM)
                except ProcessLookupError:
                    pass
        if self.relay_pid:
            until = time.monotonic() + 3
            while time.monotonic() < until:
                try:
                    os.kill(self.relay_pid, 0)
                except ProcessLookupError:
                    break
                time.sleep(0.01)
        if self.pid:
            try:
                os.kill(self.pid, signal.SIGTERM)
            except ProcessLookupError:
                pass
        self.temporary.cleanup()
        if self.reader:
            result = json.loads(self.result.read_text())
            if result.get("exit_code") != 0:
                raise RuntimeError(
                    f"Reader exit failed: {result.get('exit_code')}"
                )
