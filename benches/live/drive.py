#!/usr/bin/env python3
"""Run background native and Kitty scenarios."""

import argparse
import errno
import fcntl
import json
import os
import select
import shutil
import signal
import subprocess
import sys
import time
from functools import partial
from pathlib import Path

from kitty import Kitty
from provenance import checkout, embedded
from report import analyze, markdown, paired
from runtime import Runtime, command

from scenario import digest, scenario

ROOT = Path(__file__).resolve().parents[2]
CACHE = Path.home() / "Library/Caches/vpdf-bench"


class Native(Runtime):
    def __init__(self, args, binary, directory, probe, parse):
        super().__init__(args, directory, probe)
        self.fifo = directory / "events"
        self.writer = None
        app = directory / "Reader.app"
        shutil.copytree(args.app, app)
        shutil.copy2(binary, app / "Contents/MacOS/vpdf-gui")
        self.binary = app / "Contents/MacOS/vpdf-gui"
        self.source_sha256 = digest(self.binary)
        subprocess.run(
            [
                "codesign",
                "--force",
                "--sign",
                "-",
                "--timestamp=none",
                str(app),
            ],
            check=True,
            capture_output=True,
        )
        os.mkfifo(self.fifo)
        self.files, self.steps, self.size, self.crop = parse(
            args.scenario, args.corpus
        )
        self.size = tuple(args.window or self.size)
        self.place = tuple(args.place or self.choose_place(self.size))

    def launch(self):
        self.launched_ns = time.time_ns()
        subprocess.run(
            [
                "open",
                "-g",
                "-n",
                "-a",
                str(self.binary.parents[2]),
                "--env",
                f"VPDF_TRACE={self.trace}",
                "--env",
                "VPDF_CACHE=off",
                "--env",
                f"XDG_STATE_HOME={self.directory / 'state'}",
                "--stdout",
                str(self.directory / "stdout"),
                "--stderr",
                str(self.directory / "stderr"),
                "--args",
                "--background",
                "--events",
                str(self.fifo),
                *map(str, self.files),
            ],
            check=True,
        )

    def send(self, text):
        while self.writer is None:
            self.watch()
            try:
                self.writer = os.open(self.fifo, os.O_WRONLY | os.O_NONBLOCK)
            except OSError as error:
                if error.errno != errno.ENXIO:
                    raise
                time.sleep(0.01)
        data = memoryview((text + "\n").encode())
        while data:
            self.watch()
            try:
                data = data[os.write(self.writer, data) :]
            except BlockingIOError:
                select.select([], [self.writer], [], 0.01)

    def run(self):
        def identify():
            result = subprocess.run(
                ["pgrep", "-f", str(self.binary)],
                capture_output=True,
                text=True,
            )
            if result.stdout.strip():
                self.pid = int(result.stdout.split()[0])
            return self.pid is not None

        self.wait(identify)
        self.wait(lambda: any(r.get("complete") for r in self.rows()))
        self.send(f"place {self.place[0]} {self.place[1]}")
        self.send(f"resize {self.size[0]} {self.size[1]}")
        self.marker("placed")
        self.enable_window()
        return self.execute()

    def close(self):
        if self.writer is not None:
            os.close(self.writer)
            self.writer = None
        result = subprocess.run(
            ["pgrep", "-f", str(self.binary)], capture_output=True, text=True
        )
        for pid in result.stdout.split():
            try:
                os.kill(int(pid), signal.SIGTERM)
            except ProcessLookupError:
                pass
        until = time.monotonic() + 5
        while time.monotonic() < until:
            if subprocess.run(
                ["pgrep", "-f", str(self.binary)], capture_output=True
            ).returncode:
                return
            time.sleep(0.05)
        raise RuntimeError("Native reader did not exit")


def gates(allow_battery=False, allow_readers=()):
    for name in (
        "vpdf",
        "vpdf-gui",
        "cargo",
        "cargo-clippy",
        "rustc",
        "clippy-driver",
    ):
        result = subprocess.run(
            ["pgrep", "-x", name], capture_output=True, text=True
        )
        pids = {int(pid) for pid in result.stdout.split()}
        if name in ("vpdf", "vpdf-gui"):
            pids -= set(allow_readers)
        if pids:
            raise RuntimeError(f"Process running: {name}")
    if os.getloadavg()[0] > (os.cpu_count() or 1):
        raise RuntimeError("Machine load exceeds CPU count")
    power = command("pmset", "-g", "batt")
    if "AC Power" not in power and not allow_battery:
        raise RuntimeError("AC power required; use --allow-battery")
    return power


def wait_gates(allow_battery, timeout, allow_readers=()):
    deadline = time.monotonic() + timeout
    last = None
    quiet_since = None
    while True:
        try:
            power = gates(allow_battery, allow_readers)
            if timeout == 0:
                return power
            quiet_since = quiet_since or time.monotonic()
            if time.monotonic() - quiet_since >= 1:
                return power
            if time.monotonic() >= deadline:
                raise RuntimeError("Machine quiet gate timed out")
            time.sleep(0.25)
        except RuntimeError as error:
            quiet_since = None
            if time.monotonic() >= deadline:
                raise
            if str(error) != last:
                print(f"Waiting: {error}", file=sys.stderr, flush=True)
                last = str(error)
            time.sleep(0.25)


def run_backend(args, binary, directory, probe, parse, power):
    runtime = None
    error = None
    cleanup = []
    try:
        backend = Native if args.backend == "native" else Kitty
        runtime = backend(args, binary, directory, probe, parse)
        runtime.binary_version = command(str(runtime.binary), "--version")
        if "[bench-events]" not in runtime.binary_version:
            raise RuntimeError("Binary lacks bench-events feature")
        runtime.start_monitor()
        runtime.launch()
        display = runtime.run()
        runtime.check()
        runtime.watch()
    except BaseException as caught:
        error = caught
    finally:
        if runtime:
            for finish in (runtime.stop_monitor, runtime.close):
                try:
                    finish()
                except Exception as caught:
                    cleanup.append(str(caught))
            error = error or runtime.monitor_error
    if error or cleanup:
        failure = dict(
            error=str(error) if error else None,
            cleanup_errors=cleanup,
            backend=args.backend,
            power=power,
            invocation=[sys.executable, *sys.argv],
            binary=str(binary),
        )
        if runtime:
            failure.update(focus=runtime.focus, observed=runtime.last_state)
        (directory / "failure.json").write_text(
            json.dumps(failure, indent=2) + "\n"
        )
        if error:
            raise error
        raise RuntimeError("; ".join(cleanup))
    return runtime, display


def main():
    parser = argparse.ArgumentParser(
        description=__doc__,
        epilog="Scenario assertions: expect FIELD VALUE (headless help lists fields).",
    )
    parser.add_argument("backend", choices=["native", "kitty"])
    parser.add_argument("scenario", type=Path)
    parser.add_argument(
        "--app", type=Path, default=ROOT / "target/dist/Vpdf.app"
    )
    parser.add_argument(
        "--kitty", type=Path, default=Path("/Applications/kitty.app")
    )
    parser.add_argument("--base", type=Path)
    parser.add_argument("--candidate", type=Path)
    parser.add_argument("--rounds", type=int, default=1)
    parser.add_argument("--timeout", type=float, default=120)
    parser.add_argument("--allow-reader", action="append", type=int, default=[])
    parser.add_argument("--gate-wait", type=float, default=0)
    parser.add_argument("--allow-battery", action="store_true")
    parser.add_argument("--window", type=int, nargs=2)
    parser.add_argument("--place", type=int, nargs=2)
    parser.add_argument("--huge", type=Path, default=ROOT / "tmp/huge")
    parser.add_argument("--corpus", type=Path, default=ROOT / "tmp/corpus")
    args = parser.parse_args()
    if not 1 <= args.rounds <= 100 or not 0 < args.timeout <= 600:
        parser.error("Invalid rounds or timeout")
    if not 0 <= args.gate_wait <= 600:
        parser.error("Invalid gate wait")
    if args.window and any(not 1 <= size <= 8192 for size in args.window):
        parser.error("Invalid window dimensions")
    CACHE.mkdir(parents=True, exist_ok=True)
    with (CACHE / "live.lock").open("w") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        wait_gates(args.allow_battery, args.gate_wait, args.allow_reader)
        probe = CACHE / "inspect-live"
        source = Path(__file__).with_name("inspect.swift")
        if not probe.exists() or source.stat().st_mtime > probe.stat().st_mtime:
            subprocess.run(
                ["swiftc", str(source), "-o", str(probe)], check=True
            )
        output = CACHE / f"live-{time.time_ns()}"
        output.mkdir()
        default = (
            args.app / "Contents/MacOS/vpdf-gui"
            if args.backend == "native"
            else Path.home() / ".local/bin/vpdf"
        )
        pairs = [
            ("base", args.base or default),
            ("candidate", args.candidate or default),
        ]
        reports = []
        for round_ in range(args.rounds):
            for label, binary in pairs[:: 1 if round_ % 2 == 0 else -1]:
                power = wait_gates(
                    args.allow_battery, args.gate_wait, args.allow_reader
                )
                state = json.loads(command(str(probe), "-1"))
                if state["locked"]:
                    raise RuntimeError("Session is locked")
                directory = output / f"{round_}-{label}"
                directory.mkdir()
                native, display = run_backend(
                    args,
                    binary,
                    directory,
                    probe,
                    partial(scenario, huge=args.huge),
                    power,
                )
                report = analyze(directory / "trace.jsonl")
                report.update(
                    binary_sha256=native.source_sha256,
                    invocation=[sys.executable, *sys.argv],
                    executed_binary_sha256=digest(native.binary),
                    scenario_sha256=digest(args.scenario),
                    pdfs={str(p): digest(p) for p in native.files},
                    backend=args.backend,
                    round=round_,
                    label=label,
                    display=display,
                    machine=command("uname", "-a"),
                    cache="off",
                    **checkout(ROOT),
                    build_source=embedded(native.binary_version),
                    binary_version=native.binary_version,
                    scenario=str(args.scenario.resolve()),
                    startup=native.startup(),
                    pacing=native.pacing,
                    driver_overhead=native.overhead(),
                    target_fps=120,
                    window_requested=native.size,
                    place_requested=native.place,
                    allowed_background_readers=args.allow_reader,
                    power_before=power,
                    power_after=command("pmset", "-g", "batt"),
                )
                (directory / "report.json").write_text(
                    json.dumps(report, indent=2) + "\n"
                )
                reports.append(report)
                print(markdown(report), flush=True)
        (output / "pairs.json").write_text(
            json.dumps(paired(reports), indent=2) + "\n"
        )
        print(output)


if __name__ == "__main__":
    main()
