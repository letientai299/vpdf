"""Shared scenario timing and window checks."""

import json
import os
import resource
import subprocess
import threading
import time


def command(*args):
    return subprocess.check_output(args, text=True).strip()


def duration(text):
    multiplier = 1000 if text.endswith("s") and not text.endswith("ms") else 1
    number = (
        text[:-2]
        if text.endswith("ms")
        else text[:-1]
        if text.endswith("s")
        else text
    )
    if not number.isascii() or not number.isdecimal():
        raise ValueError("Invalid scenario duration")
    millis = int(number) * multiplier
    if millis >= 1 << 64:
        raise ValueError("Scenario duration overflow")
    return millis / 1000


def check_error(row):
    if row.get("error") is not None:
        raise RuntimeError(f"Scenario job failed: {row['error']}")


def ready(row, after, idle):
    check_error(row)
    return (
        row.get("complete", False)
        and row.get("built_ms", -1) >= after
        and (not idle or row.get("busy", True) is False)
    )


def mixed(text):
    seed, choices = text.split(maxsplit=1)
    seed = int(seed)
    choices = [choice.strip() for choice in choices.split("|")]
    while True:
        seed = (seed * 6364136223846793005 + 1) & ((1 << 64) - 1)
        yield choices[(seed >> 33) % len(choices)]


class Runtime:
    def __init__(self, args, directory, probe):
        self.args, self.directory, self.probe = args, directory, probe
        self.trace = directory / "trace.jsonl"
        self.deadline = time.monotonic() + args.timeout
        self.pid = None
        initial = self.inspect()
        self.focus = (initial["front"], initial.get("front_pid"))
        self.last_state = initial
        self.check_window = False
        self.window_since = 0
        self.monitor_stop = threading.Event()
        self.monitor_error = None
        self.monitor = None
        self.trace_offset = 0
        self.trace_pending = b""
        self.trace_rows = []
        self.pacing = []
        self.checkpoints = {}
        self.probe_process = None
        self.monitor_usage = {}
        self.driver_started = None

    def inspect(self):
        return json.loads(command(str(self.probe), str(self.pid or -1)))

    def rows(self):
        if not self.trace.exists():
            return self.trace_rows
        with self.trace.open("rb") as source:
            source.seek(self.trace_offset)
            data = source.read()
            self.trace_offset = source.tell()
        lines = (self.trace_pending + data).split(b"\n")
        self.trace_pending = lines.pop()
        self.trace_rows.extend(json.loads(line) for line in lines if line)
        return self.trace_rows

    @property
    def pid(self):
        return self._pid

    @pid.setter
    def pid(self, value):
        self._pid = value
        (self.directory / "monitor-pid").write_text(str(value or -1))

    def validate(self, state):
        self.last_state = state
        observed = (state["front"], state.get("front_pid"))
        if observed != self.focus:
            raise RuntimeError(f"Focus changed: {self.focus} -> {observed}")
        if state["locked"]:
            raise RuntimeError("Session locked during run")
        if state.get("builds"):
            raise RuntimeError(f"Build started during run: {state['builds']}")
        if (
            self.check_window
            and state.get("pid") == self.pid
            and state.get("sampled_unix_ns", 0) >= self.window_since
            and (state.get("occluded", True) or "display" not in state)
        ):
            raise RuntimeError("Window occluded or outside display")

    def check(self):
        self.validate(self.inspect())

    def choose_place(self, size):
        state = json.loads(command(str(self.probe), str(self.focus[1])))
        screen = state["screen"]
        front = state.get("window")
        width, height = size[0], size[1] + 40
        left, top = screen["x"] + 60, screen["y"] + 80
        right = screen["x"] + screen["width"] - width - 60
        bottom = screen["y"] + screen["height"] - height - 60
        for x, y in (
            (left, top),
            (right, top),
            (left, bottom),
            (right, bottom),
        ):
            if (
                x < screen["x"]
                or y < screen["y"]
                or x + width > screen["x"] + screen["width"]
                or y + height > screen["y"] + screen["height"]
            ):
                continue
            if not front or (
                x + width <= front["X"]
                or x >= front["X"] + front["Width"]
                or y + height <= front["Y"]
                or y >= front["Y"] + front["Height"]
            ):
                return int(x), int(y)
        raise RuntimeError("No visible slot; use smaller window")

    def enable_window(self):
        state = self.inspect()
        self.validate(state)
        self.last_state = state
        if state.get("occluded", True) or "display" not in state:
            raise RuntimeError("Window occluded or outside display")
        self.window_since = time.time_ns()
        self.check_window = True

    def start_monitor(self):
        self.driver_started = time.monotonic()
        self.driver_cpu = time.process_time()
        self.probe_process = subprocess.Popen(
            [str(self.probe), "--watch", str(self.directory / "monitor-pid")],
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
        )

        def monitor():
            try:
                for line in self.probe_process.stdout:
                    self.validate(json.loads(line))
                if not self.monitor_stop.is_set():
                    raise RuntimeError("Window monitor exited")
            except Exception as error:
                self.monitor_error = error

        self.monitor = threading.Thread(target=monitor, daemon=True)
        self.monitor.start()

    def stop_monitor(self):
        if self.driver_started is None:
            return
        self.driver_elapsed = time.monotonic() - self.driver_started
        self.driver_used_cpu = time.process_time() - self.driver_cpu
        self.monitor_stop.set()
        if self.probe_process:
            self.probe_process.terminate()
            _, status, usage = os.wait4(self.probe_process.pid, 0)
            self.probe_process.returncode = os.waitstatus_to_exitcode(status)
            self.monitor_usage = dict(
                cpu_ms=(usage.ru_utime + usage.ru_stime) * 1000,
                rss_peak_bytes=usage.ru_maxrss,
            )
        if self.monitor:
            self.monitor.join(timeout=3)
            if self.monitor.is_alive():
                raise RuntimeError("Window monitor did not exit")
        if self.probe_process:
            self.probe_process.stdout.close()
            self.probe_process.stderr.close()

    def watch(self):
        if time.monotonic() > self.deadline:
            raise TimeoutError("Scenario deadline exceeded")
        if self.monitor_error:
            raise self.monitor_error

    def pace(self, due):
        self.watch()
        remaining = due - time.monotonic()
        if remaining > 0:
            time.sleep(remaining)
        self.watch()

    def wait(self, predicate):
        while True:
            self.watch()
            if predicate():
                return
            time.sleep(0.01)

    def marker(self, name):
        offset = len(self.rows())
        self.send("mark " + name)
        self.wait(
            lambda: any(
                r.get("kind") == "mark" and r.get("name") == name
                for r in self.rows()[offset:]
            )
        )

    def phase_frames(self, after):
        frames = []
        for row in self.rows():
            if row.get("kind") != "frame":
                continue
            if row["t_ms"] >= after:
                check_error(row)
            if row.get("built_ms", -1) >= after:
                frames.append(row)
        return frames

    def step(self, line):
        words = line.split(maxsplit=1)
        verb, rest = words[0], words[1] if len(words) == 2 else ""
        if verb == "wait":
            if rest in ("idle", "complete"):
                token = f"__bench_wait_{time.monotonic_ns()}"
                self.marker(token)
                at = next(
                    r["t_ms"] for r in self.rows() if r.get("name") == token
                )
                self.wait(
                    lambda: any(
                        ready(r, at, rest == "idle")
                        for r in self.phase_frames(at)
                    )
                )
            else:
                until = time.monotonic() + duration(rest)
                self.wait(lambda: time.monotonic() >= until)
        elif verb == "expect":
            from tools.bench.live.expectation import matches

            token = f"__bench_wait_{time.monotonic_ns()}"
            self.marker(token)
            at = next(r["t_ms"] for r in self.rows() if r.get("name") == token)
            field, checkpoint = rest.split(maxsplit=1)
            baseline = None
            if field in ("moved", "selected_changed"):
                if checkpoint not in self.checkpoints:
                    raise ValueError(
                        f"Scenario checkpoint missing: {checkpoint}"
                    )
                baseline = self.checkpoints[checkpoint]
            last_probe = time.monotonic()

            def asserted():
                nonlocal last_probe
                if any(
                    r.get("kind") == "frame"
                    and r.get("built_ms", -1) >= at
                    and matches(rest, r, baseline)
                    for r in self.phase_frames(at)
                ):
                    return True
                if time.monotonic() - last_probe >= 0.1:
                    self.send("mark " + token)
                    last_probe = time.monotonic()
                return False

            self.wait(asserted)
        elif verb == "repeat":
            span, rate, repeated = rest.split(maxsplit=2)
            hz = int(rate.removeprefix("@"))
            if not 1 <= hz <= 1000:
                raise ValueError("Invalid repeat rate")
            if repeated.startswith("mix "):
                choices = mixed(repeated[4:])
            else:
                choices = None
            seconds = duration(span)
            generation_cpu = time.process_time()
            started = time.monotonic()
            index = 0
            previous = None
            gap_max = lateness_max = 0.0
            while time.monotonic() - started < seconds:
                self.pace(started + index / hz)
                before = time.monotonic()
                if before - started >= seconds:
                    break
                lateness_max = max(
                    lateness_max, (before - (started + index / hz)) * 1000
                )
                if choices:
                    self.send("key " + next(choices))
                else:
                    self.send(repeated)
                delivered = time.monotonic()
                if previous is not None:
                    gap_max = max(gap_max, (delivered - previous) * 1000)
                previous = delivered
                index += 1
            elapsed = time.monotonic() - started
            self.pacing.append(
                dict(
                    requested_hz=hz,
                    delivered=index,
                    duration_s=seconds,
                    elapsed_s=elapsed,
                    delivered_hz=index / elapsed,
                    lateness_max_ms=lateness_max,
                    gap_max_ms=gap_max,
                    generation_cpu_ms=(time.process_time() - generation_cpu)
                    * 1000,
                )
            )
        elif verb == "mark":
            self.marker(rest)
            at = next(
                r["t_ms"]
                for r in reversed(self.rows())
                if r.get("name") == rest
            )

            def checkpointed():
                frames = self.phase_frames(at)
                if frames:
                    self.checkpoints[rest] = dict(frames[0])
                    return True
                return False

            self.wait(checkpointed)
        elif verb == "mix":
            self.send("key " + next(mixed(rest)))
        elif verb == "quit":
            if rest:
                raise ValueError("Quit takes no arguments")
            return False
        elif verb in ("key", "wheel", "place", "resize"):
            self.send(line)
        else:
            raise ValueError(f"Unsupported scenario step: {verb}")

    def overhead(self):
        result = dict(
            controller_cpu_ms=self.driver_used_cpu * 1000,
            elapsed_ms=self.driver_elapsed * 1000,
            monitor=self.monitor_usage,
            controller_rss_peak_bytes=resource.getrusage(
                resource.RUSAGE_SELF
            ).ru_maxrss,
        )
        relay = self.directory / "relay-overhead.json"
        if relay.exists():
            result["relay"] = json.loads(relay.read_text())
        return result

    def startup(self):
        rows = self.rows()
        clock = next((row for row in rows if row.get("kind") == "clock"), None)
        complete = next((row for row in rows if row.get("complete")), None)
        result = {}
        if clock and complete:
            result["complete_reader_ms"] = (
                clock["unix_ns"] - self.launched_ns
            ) / 1e6 + complete["t_ms"]
        delivery = self.directory / "delivery.json"
        if delivery.exists():
            result.update(json.loads(delivery.read_text()))
        return result

    def execute(self):
        if self.crop == "on":
            self.send("key z c")
        for step in self.steps:
            if self.step(step) is False:
                break
        self.marker("finished")
        return self.inspect()
