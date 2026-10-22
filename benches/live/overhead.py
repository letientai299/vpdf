"""Compare trace and relay cost using independent terminal output."""

import argparse
import fcntl
import hashlib
import importlib
import json
import os
import pty
import select
import signal
import socket
import struct
import subprocess
import sys
import tempfile
import termios
import time
from pathlib import Path

from provenance import checkout, embedded

ROOT = Path(__file__).resolve().parents[2]
sys.path[:0] = [str(ROOT / "benches/live"), str(ROOT / "scripts")]
encode = importlib.import_module("keys").encode
mixed = importlib.import_module("runtime").mixed
startup = importlib.import_module("startup")
FRAME_END, child_setup, placements = (
    startup.FRAME_END,
    startup.child_setup,
    startup.placements,
)

CACHE = Path.home() / "Library/Caches/vpdf-bench"
GEOMETRY = [100, 179, 1969, 2100]
REPLY = (
    b"\x1bP>|kitty(0.49.1)\x1b\\\x1b_Gi=31;OK\x1b\\"
    b"\x1b[?2026;2$y\x1b[?1016;2$y\x1b[?2048;2$y\x1b[?29u"
    b"\x1b]22;1\x1b\\\x1b]72;t=q\x1b\\\x1b[?62;52;c"
)


def one(output, name, binary, traced, relay, hz, capture=False):
    folder = output / name
    folder.mkdir()
    master, slave = pty.openpty()
    fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", *GEOMETRY))
    env = dict(
        os.environ,
        VPDF_CACHE="off",
        XDG_STATE_HOME=str(folder / "state"),
        XDG_CACHE_HOME=str(folder / "cache"),
        TERM="xterm-kitty",
    )
    for key in (
        "VPDF_TRACE",
        "TMUX",
        "STY",
        "VPDF_PREVIEW",
        "FZF_PREVIEW_COLUMNS",
        "FZF_PREVIEW_LINES",
    ):
        env.pop(key, None)
    if traced:
        env["VPDF_TRACE"] = str(folder / "trace.jsonl")
    command = [str(binary), "--no-config", str(ROOT / "tmp/huge/sdm.pdf")]
    monitor = control = None
    process = None
    try:
        with tempfile.TemporaryDirectory(prefix="ve-", dir="/tmp") as transport:
            if relay:
                spec = dict(
                    command=command,
                    env=env,
                    socket=transport + "/socket",
                    result=str(folder / "relay.json"),
                    timeout=20,
                )
                (folder / "spec.json").write_text(json.dumps(spec))
                command = [
                    sys.executable,
                    "-B",
                    str(ROOT / "benches/live/pty_bridge.py"),
                    str(folder / "spec.json"),
                ]
            before = time.monotonic()
            cpu_before = time.process_time()
            process = subprocess.Popen(
                command,
                stdin=slave,
                stdout=slave,
                stderr=slave,
                env=env,
                preexec_fn=child_setup,
            )
            os.close(slave)
            slave = None
            if relay:
                (folder / "monitor-pid").write_text("-1")
                monitor = subprocess.Popen(
                    [
                        str(CACHE / "inspect-live"),
                        "--watch",
                        str(folder / "monitor-pid"),
                    ],
                    stdout=subprocess.PIPE,
                    stderr=subprocess.PIPE,
                )
                control = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
            os.set_blocking(master, False)
            pending = bytearray()
            search_from = 0
            tail = b""
            frame_rows = []
            raw_frames = []
            snapshot_end = None
            monitor_pending = b""
            initial_focus = None
            first = None
            start = None
            index = 0
            sent = []
            quitting = False
            active_s = 3
            choices = mixed(
                "7 C-d | C-d | C-d | C-d | space | space | space | space | C-u | C-u | S-space | S-space"
            )
            while True:
                now = time.monotonic()
                if now - before > 18:
                    raise RuntimeError("Evaluation deadline exceeded")
                if relay and first is None and (folder / "relay.json").exists():
                    metadata = json.loads((folder / "relay.json").read_text())
                    first = metadata["pid"]
                    control.connect(transport + "/socket")
                if frame_rows and start is None:
                    start = frame_rows[0]["at"] + 1.5
                if start is not None:
                    if index < active_s * hz and now >= start + index / hz:
                        payload = encode(next(choices))
                        if control:
                            control.sendall(payload)
                        else:
                            os.write(master, payload)
                        sent.append(time.monotonic())
                        index += 1
                    if now >= start + active_s + 1 and not quitting:
                        if control:
                            control.sendall(encode("esc : q a enter"))
                        else:
                            os.write(master, encode("esc : q a enter"))
                        quitting = True
                        snapshot_end = len(raw_frames)
                readers = [master] + ([monitor.stdout] if monitor else [])
                due = (
                    start + index / hz
                    if start and index < active_s * hz
                    else now + 0.01
                )
                ready, _, _ = select.select(
                    readers, [], [], max(0, min(0.01, due - now))
                )
                if monitor and monitor.stdout in ready:
                    monitor_pending += os.read(monitor.stdout.fileno(), 65536)
                    while b"\n" in monitor_pending:
                        line, monitor_pending = monitor_pending.split(b"\n", 1)
                        state = json.loads(line)
                        focus = (state["front"], state.get("front_pid"))
                        initial_focus = initial_focus or focus
                        if (
                            state["locked"]
                            or state["builds"]
                            or focus != initial_focus
                        ):
                            raise RuntimeError(
                                "Environment changed during evaluation"
                            )
                if master in ready:
                    try:
                        data = os.read(master, 65536)
                    except OSError:
                        data = b""
                    if not data:
                        break
                    at = time.monotonic()
                    joined = tail + data
                    # The app terminates each capability round with DA1.
                    for _ in range(joined.count(b"\x1b[c")):
                        os.write(master, REPLY)
                    tail = joined[-2:]
                    pending.extend(data)
                    while (end := pending.find(FRAME_END, search_from)) >= 0:
                        frame = bytes(pending[:end])
                        del pending[: end + len(FRAME_END)]
                        search_from = 0
                        base, _ = placements(frame)
                        if capture:
                            raw_frames.append(frame)
                        # A content signature independent of image ids.
                        import re

                        status = re.findall(rb"(\d+)/(5364)", frame[-8192:])
                        frame_rows.append(
                            dict(
                                at=at,
                                base=base,
                                bytes=len(frame),
                                status=status[-1][0].decode()
                                if status
                                else None,
                            )
                        )
                    search_from = max(0, len(pending) - len(FRAME_END) + 1)
                    if len(pending) > 64 * 1024 * 1024:
                        raise RuntimeError("Frame exceeds bound")
            pid, status, usage = os.wait4(process.pid, 0)
            process.returncode = os.waitstatus_to_exitcode(status)
            if process.returncode or index != hz * active_s:
                raise RuntimeError(f"Exit {process.returncode}; inputs {index}")
            active = [
                f for f in frame_rows if start <= f["at"] < start + active_s
            ]
            gaps = [
                (b["at"] - a["at"]) * 1000 for a, b in zip(active, active[1:])
            ]
            samples = []
            if traced:
                rows = [
                    json.loads(line)
                    for line in (folder / "trace.jsonl")
                    .read_text()
                    .splitlines()
                ]
                frames = [r for r in rows if r["kind"] == "frame"]
                clock = (
                    next(r for r in rows if r["kind"] == "clock")["unix_ns"]
                    / 1e9
                )
                # Align reader monotonic stamps using wall-clock offset.
                trace_start = (
                    start + time.time() - time.monotonic() - clock
                ) * 1000
                trace_active = [
                    r
                    for r in frames
                    if trace_start <= r["t_ms"] < trace_start + 3000
                ]
                samples = [r["rss"] for r in rows if r["kind"] == "sample"]
            else:
                frames = trace_active = []
            relay_result = (
                json.loads((folder / "relay.json").read_text()) if relay else {}
            )
            if relay and relay_result.get("exit_code") != 0:
                raise RuntimeError("Relayed app failed")
            result = dict(
                name=name,
                process_scope="relay and waited descendants"
                if relay
                else "reader",
                traced=traced,
                relay=relay,
                hz=hz,
                frames=len(active),
                gaps_ms=gaps,
                empty_frames=sum(f["base"] == 0 for f in active),
                placements=sum(f["base"] for f in active),
                bytes=sum(f["bytes"] for f in active),
                last_status=next(
                    (f["status"] for f in reversed(frame_rows) if f["status"]),
                    None,
                ),
                inputs=index,
                input_lateness_ms=[
                    (at - start - i / hz) * 1000 for i, at in enumerate(sent)
                ],
                process_cpu_ms=(usage.ru_utime + usage.ru_stime) * 1000,
                process_rss_peak=usage.ru_maxrss,
                controller_cpu_ms=(time.process_time() - cpu_before) * 1000,
                elapsed_ms=(time.monotonic() - before) * 1000,
                trace_final=frames[-1] if frames else None,
                trace_active_blank=sum(
                    r.get("blank", False) for r in trace_active
                )
                if traced
                else None,
                trace_peak_rss=max(samples, default=None),
            )
            if capture:
                result["fidelity"] = fidelity(raw_frames[:snapshot_end])
            if relay:
                result["relay_overhead"] = json.loads(
                    (folder / "relay-overhead.json").read_text()
                )
                monitor.terminate()
                _, status, cost = os.wait4(monitor.pid, 0)
                monitor.returncode = os.waitstatus_to_exitcode(status)
                result["monitor_cpu_ms"] = (
                    cost.ru_utime + cost.ru_stime
                ) * 1000
                result["monitor_rss_peak"] = cost.ru_maxrss
            (folder / "measure.json").write_text(
                json.dumps(result, indent=2) + "\n"
            )
            return result
    finally:
        if process and process.poll() is None:
            os.killpg(process.pid, signal.SIGTERM)
            process.wait(timeout=5)
        if monitor:
            if monitor.poll() is None:
                monitor.terminate()
                monitor.wait(timeout=3)
            monitor.stdout.close()
            monitor.stderr.close()
        if control:
            control.close()
        os.close(master)
        if slave is not None:
            os.close(slave)


def fidelity(frames):
    import base64
    import re
    import zlib

    images = {}
    pixel_data = {}
    current = None
    chunks = bytearray()
    final = []
    canvas = bytearray()
    for frame in frames:
        placed = []
        for match in re.finditer(rb"\x1b_G([^;]*);(.*?)\x1b\\", frame, re.S):
            fields = dict(
                pair.split(b"=", 1)
                for pair in match[1].split(b",")
                if b"=" in pair
            )
            if fields.get(b"a") == b"t":
                current = fields
                chunks = bytearray()
            if current is not None and (
                fields.get(b"a") == b"t" or b"m" in fields
            ):
                chunks.extend(match[2])
                if fields.get(b"m", b"0") == b"0":
                    pixels = base64.b64decode(chunks)
                    if current.get(b"o") == b"z":
                        pixels = zlib.decompress(pixels)
                    images[current[b"i"]] = hashlib.sha256(pixels).hexdigest()
                    pixel_data[current[b"i"]] = (
                        pixels,
                        int(current[b"s"]),
                        int(current[b"v"]),
                    )
                    current = None
            if fields.get(b"a") == b"p" and fields.get(b"z") == b"-2":
                cursor = re.findall(
                    rb"\x1b\[(\d+);(\d+)H", frame[: match.start()]
                )[-1]
                canonical = {
                    k.decode(): v.decode()
                    for k, v in fields.items()
                    if k not in (b"i", b"p", b"q", b"a")
                }
                placed.append(
                    dict(
                        cursor=[int(v) for v in cursor],
                        geometry=canonical,
                        pixel_sha256=images[fields[b"i"]],
                    )
                )
        if placed:
            final = placed
            canvas = bytearray([255]) * (GEOMETRY[2] * GEOMETRY[3] * 3)
            for match in re.finditer(
                rb"\x1b_G([^;]*);(.*?)\x1b\\", frame, re.S
            ):
                f = dict(
                    pair.split(b"=", 1)
                    for pair in match[1].split(b",")
                    if b"=" in pair
                )
                if f.get(b"a") != b"p" or f.get(b"z") != b"-2":
                    continue
                row, col = map(
                    int,
                    re.findall(rb"\x1b\[(\d+);(\d+)H", frame[: match.start()])[
                        -1
                    ],
                )
                pixels, width, height = pixel_data[f[b"i"]]
                x = (col - 1) * 11 + int(f.get(b"X", 0))
                y = (row - 1) * 21 + int(f.get(b"Y", 0))
                sx, sy = int(f.get(b"x", 0)), int(f.get(b"y", 0))
                w, h = int(f.get(b"w", width)), int(f.get(b"h", height))
                for dy in range(h):
                    source = ((sy + dy) * width + sx) * 3
                    dest = ((y + dy) * GEOMETRY[2] + x) * 3
                    canvas[dest : dest + w * 3] = pixels[
                        source : source + w * 3
                    ]
    assert final, "No final tile placements"
    canonical = json.dumps(final, sort_keys=True)
    return dict(
        sha256=hashlib.sha256(canonical.encode()).hexdigest(),
        canvas_sha256=hashlib.sha256(canvas).hexdigest(),
        tiles=len(final),
        placements=final,
    )


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--rounds", type=int, default=6)
    parser.add_argument("--hz", type=int, default=120)
    parser.add_argument("--smoke", action="store_true")
    parser.add_argument(
        "--binary", type=Path, default=ROOT / "target/release/vpdf"
    )
    parser.add_argument("--allow-battery", action="store_true")
    args = parser.parse_args()
    if not 1 <= args.rounds <= 100 or not 1 <= args.hz <= 1000:
        parser.error("Invalid rounds or input rate")
    import fcntl

    from drive import wait_gates

    from scenario import scenario

    CACHE.mkdir(parents=True, exist_ok=True)
    lock = (CACHE / "live.lock").open("w")
    fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
    wait_gates(args.allow_battery, 0, ())
    scenario(ROOT / "benches/scenarios/keys.txt", ROOT / "tmp/huge")
    probe = CACHE / "inspect-live"
    source = Path(__file__).with_name("inspect.swift")
    if not probe.exists() or source.stat().st_mtime > probe.stat().st_mtime:
        subprocess.run(["swiftc", str(source), "-o", str(probe)], check=True)
    output = CACHE / f"overhead-{time.time_ns()}"
    output.mkdir()
    binary = output / "vpdf"
    import shutil

    shutil.copy2(args.binary.resolve(), binary)
    manifest = dict(
        binary_sha256=hashlib.sha256(binary.read_bytes()).hexdigest(),
        pdf_sha256=hashlib.sha256(
            (ROOT / "tmp/huge/sdm.pdf").read_bytes()
        ).hexdigest(),
        **checkout(ROOT),
        build_source=embedded(
            subprocess.check_output([str(binary), "--version"], text=True)
        ),
        machine=subprocess.check_output(["uname", "-a"], text=True).strip(),
        geometry=GEOMETRY,
        power=subprocess.check_output(["pmset", "-g", "batt"], text=True),
        invocation=sys.argv,
        cache="OS warm after hash; fresh process/state; VPDF_CACHE=off",
        boundary="terminal output drained by headless protocol responder; no GPU/display",
    )
    (output / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
    results = []
    contrasts = [
        ("aa", (False, False), (False, False)),
        ("trace", (False, False), (True, False)),
        ("driver", (True, False), (True, True)),
    ]
    if args.smoke:
        contrasts = [("smoke", (True, False), (True, True))]
    print(output, flush=True)
    for round_ in range(args.rounds):
        for contrast, a, b in contrasts:
            for label, (traced, relay) in [("a", a), ("b", b)][
                :: 1 if round_ % 2 == 0 else -1
            ]:
                name = f"{contrast}-{round_}-{label}"
                r = one(output, name, binary, traced, relay, args.hz)
                results.append(r)
                (output / "results.json").write_text(
                    json.dumps(results, indent=2) + "\n"
                )
                print(
                    name,
                    r["frames"],
                    round(r["process_rss_peak"] / 2**20, 1),
                    flush=True,
                )

    checks = []
    for name, traced, relay in (
        ("off", False, False),
        ("on", True, False),
        ("relay", True, True),
    ):
        checks.append(
            one(
                output,
                "fidelity-" + name,
                binary,
                traced,
                relay,
                args.hz,
                capture=True,
            )
        )
    hashes = {r["fidelity"]["canvas_sha256"] for r in checks}
    placements = {r["fidelity"]["sha256"] for r in checks}
    if len(hashes) != 1 or len(placements) != 1:
        raise RuntimeError("Final tile fidelity differs")
    (output / "fidelity.json").write_text(json.dumps(checks, indent=2) + "\n")
    from overhead_report import summarize

    (output / "summary.json").write_text(
        json.dumps(summarize(output), indent=2) + "\n"
    )


if __name__ == "__main__":
    main()
