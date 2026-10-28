"""Reuse the live runner's background window checks."""

import contextlib
import fcntl
import hashlib
import json
import os
import platform
import signal
import subprocess
import tempfile
import time
from pathlib import Path
from types import SimpleNamespace

from tools.bench.live import runtime
from tools.build.cache import cache_root, locked

ROOT = Path(__file__).resolve().parents[2]
CACHE = Path.home() / ".cache/vpdf/demo-media/helpers"


def compile_helper(source, target, flags=()):
    identity = hashlib.sha256(
        source.read_bytes() + b"\0" + json.dumps(list(flags)).encode()
    ).hexdigest()
    target = target.with_name(target.name + "-" + identity)
    target.parent.mkdir(parents=True, exist_ok=True)
    if not target.exists():
        with tempfile.TemporaryDirectory(dir=target.parent) as directory:
            staged = Path(directory) / target.name
            subprocess.run(
                ["swiftc", *flags, str(source), "-o", str(staged)],
                check=True,
            )
            staged.replace(target)
    return target


class Window(runtime.Runtime):
    def __init__(self, args, directory):
        if platform.system() != "Darwin":
            raise RuntimeError("Background demo capture requires macOS")
        lock = Path.home() / "Library/Caches/vpdf-bench/live.lock"
        lock.parent.mkdir(parents=True, exist_ok=True)
        self.resources = contextlib.ExitStack()
        self.lease = self.resources.enter_context(lock.open("a"))
        try:
            fcntl.flock(self.lease, fcntl.LOCK_EX | fcntl.LOCK_NB)
            build_gate = cache_root(os.environ, ROOT) / "leases/worker"
            build_gate.parent.mkdir(parents=True, exist_ok=True)
            deadline = time.monotonic() + 60
            announced = False
            while True:
                try:
                    self.resources.enter_context(
                        locked(build_gate, fcntl.LOCK_EX | fcntl.LOCK_NB)
                    )
                    break
                except BlockingIOError:
                    if time.monotonic() > deadline:
                        raise RuntimeError(
                            "Repository build lease unavailable"
                        ) from None
                    if not announced:
                        print(
                            "Waiting for repository builds before capture",
                            flush=True,
                        )
                        announced = True
                    time.sleep(0.2)
            probe = compile_helper(
                ROOT / "tools/bench/live/inspect.swift", CACHE / "inspect"
            )
            self.checks = 0
            self.foreground_states = []
            self.arguments = args
            self.remote_address = None
            super().__init__(
                SimpleNamespace(timeout=600), Path(directory), probe
            )
        except BaseException:
            self.resources.close()
            raise

    def validate(self, state):
        observed = (state["front"], state.get("front_pid"))
        if self.pid is not None and observed[1] == self.pid:
            raise RuntimeError("Capture window received keyboard focus")
        self.focus = observed
        super().validate(state)
        if not self.foreground_states or self.foreground_states[-1] != list(
            observed
        ):
            self.foreground_states.append(list(observed))
        self.checks += 1

    def launch(self, command, address, log):
        self.remote_address = address
        self.check()
        self.place = self.arguments.place or self.choose_place(
            (self.arguments.width, self.arguments.height)
        )
        executable = Path(command[0]).resolve()
        app = next(
            (
                parent
                for parent in executable.parents
                if parent.suffix == ".app"
            ),
            None,
        )
        if app is None:
            raise RuntimeError("Kitty macOS application bundle required")
        command = [
            "open",
            "-g",
            "-n",
            "-a",
            str(app),
            "--stdout",
            str(Path(log.name).resolve()),
            "--stderr",
            str(Path(log.name).resolve()),
            "--args",
            *command[1:],
        ]
        subprocess.run(command, check=True)
        self.start_monitor()

    def identify(self):
        result = subprocess.run(
            ["pgrep", "-f", self.remote_address], capture_output=True, text=True
        )
        if result.returncode == 0 and result.stdout.strip():
            self.pid = int(result.stdout.split()[0])
        return self.pid is not None

    def show(self, remote):
        if not self.identify():
            raise RuntimeError("Background Kitty process missing")
        remote(
            "kitten",
            str(ROOT / "tools/bench/live/place.py"),
            *map(str, self.place),
        )
        deadline = time.monotonic() + 5
        while True:
            state = self.inspect()
            self.validate(state)
            bounds = state.get("window", {})
            placed = all(
                abs(bounds.get(key, float("inf")) - value) < 2
                for key, value in zip(("X", "Y"), self.place)
            )
            if (
                placed
                and not state.get("occluded", True)
                and "display" in state
            ):
                break
            if time.monotonic() > deadline:
                raise RuntimeError(
                    "Capture window lacks a visible display slot"
                )
            time.sleep(0.05)
        self.enable_window()
        self.check()

    def evidence(self):
        self.watch()
        self.check()
        return {
            "launch": "open -g -n; hidden Kitty; live place helper",
            "front": list(self.focus),
            "checks": self.checks,
            "fully_visible": not self.last_state.get("occluded", True),
            "window": self.last_state.get("window"),
            "task_received_focus": False,
            "foreground_states": [list(row) for row in self.foreground_states],
        }

    def close(self):
        try:
            self.stop_monitor()
        finally:
            try:
                if self.pid is None and self.remote_address:
                    self.identify()
                if self.pid is not None:
                    try:
                        os.kill(self.pid, signal.SIGTERM)
                    except ProcessLookupError:
                        pass
                    deadline = time.monotonic() + 3
                    while time.monotonic() < deadline:
                        try:
                            os.kill(self.pid, 0)
                        except ProcessLookupError:
                            break
                        time.sleep(0.05)
            finally:
                self.resources.close()
