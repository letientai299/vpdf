"""Relay a real terminal through a child pty."""

import errno
import fcntl
import json
import os
import pty
import resource
import select
import signal
import socket
import struct
import subprocess
import sys
import termios
import time
import tty
from collections import deque
from pathlib import Path

from tools.bench.startup import FRAME_END, child_setup, placements, winsize

LIMIT = 4 * 1024 * 1024


class Pending:
    def __init__(self):
        self.chunks = deque()
        self.size = 0

    def append(self, data):
        if data:
            self.chunks.append(memoryview(data))
            self.size += len(data)

    def flush(self, fd):
        try:
            count = os.write(fd, self.chunks[0])
        except BlockingIOError:
            return 0
        self.size -= count
        if count == len(self.chunks[0]):
            self.chunks.popleft()
        else:
            self.chunks[0] = self.chunks[0][count:]
        return count


def publish(path, value):
    temporary = path.with_name(path.name + ".pending")
    temporary.write_text(json.dumps(value))
    temporary.replace(path)


def relay(specification):
    spec = json.loads(specification.read_text())
    saved = termios.tcgetattr(0)
    output_flags = fcntl.fcntl(1, fcntl.F_GETFL)
    master, slave = pty.openpty()
    fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", *winsize()))
    child = None
    connection = None
    server = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    server.bind(spec["socket"])
    os.chmod(spec["socket"], 0o600)
    server.listen(1)
    resized = True

    def resize(*_):
        nonlocal resized
        resized = True

    signal.signal(signal.SIGWINCH, resize)
    try:
        tty.setraw(0)
        os.set_blocking(master, False)
        os.set_blocking(1, False)
        to_child, to_terminal = Pending(), Pending()
        child_eof = False
        child_input_open = True
        environment = dict(os.environ)
        for name in (
            "TMUX",
            "STY",
            "VPDF_PREVIEW",
            "FZF_PREVIEW_COLUMNS",
            "FZF_PREVIEW_LINES",
        ):
            environment.pop(name, None)
        environment.update(spec["env"])
        started = time.monotonic()
        cpu_started = time.process_time()
        launched_ns = time.time_ns()
        delivery = {}
        positions = {}
        queued_output = forwarded_output = 0
        output_tail = bytearray()
        search_from = 0
        child = subprocess.Popen(
            spec["command"],
            cwd=spec.get("cwd"),
            stdin=slave,
            stdout=slave,
            stderr=slave,
            env=environment,
            preexec_fn=child_setup,
        )
        os.close(slave)
        slave = None
        spec_path = Path(spec["result"])
        publish(
            spec_path,
            dict(
                pid=child.pid,
                relay_pid=os.getpid(),
                kitty_pid=os.getppid(),
                geometry=winsize(),
                launched_unix_ns=launched_ns,
            ),
        )
        while True:
            if time.monotonic() - started > spec["timeout"]:
                raise TimeoutError("Relay deadline exceeded")
            if resized:
                geometry = winsize()
                fcntl.ioctl(
                    master, termios.TIOCSWINSZ, struct.pack("HHHH", *geometry)
                )
                publish(spec_path.parent / "geometry.json", geometry)
                resized = False
            if child_eof and not to_terminal.size:
                break
            readers = []
            if not child_eof:
                readers.append(server)
                if child_input_open and to_child.size < LIMIT:
                    readers.append(0)
                    if connection:
                        readers.append(connection)
                if to_terminal.size < LIMIT:
                    readers.append(master)
            writers = (
                [master]
                if to_child.size and child_input_open and not child_eof
                else []
            )
            if to_terminal.size:
                writers.append(1)
            ready, writable, _ = select.select(readers, writers, [], 0.05)
            if server in ready:
                incoming, _ = server.accept()
                incoming.setblocking(False)
                if connection:
                    incoming.close()
                else:
                    connection = incoming
            if 0 in ready and to_child.size < LIMIT:
                try:
                    data = os.read(0, min(65536, LIMIT - to_child.size))
                except BlockingIOError:
                    data = None
                if data == b"":
                    raise RuntimeError("Outer terminal closed")
                if data:
                    to_child.append(data)
            if master in ready:
                try:
                    data = os.read(master, min(65536, LIMIT - to_terminal.size))
                except BlockingIOError:
                    data = None
                except OSError as error:
                    if error.errno != errno.EIO:
                        raise
                    data = b""
                if data == b"":
                    child_eof = True
                if data:
                    to_terminal.append(data)
                    queued_output += len(data)
                    if "first_tile_delivery_ms" not in positions:
                        output_tail.extend(data)
                        while (
                            end := output_tail.find(FRAME_END, search_from)
                        ) >= 0:
                            frame = bytes(output_tail[:end])
                            position = (
                                queued_output
                                - len(output_tail)
                                + end
                                + len(FRAME_END)
                            )
                            del output_tail[: end + len(FRAME_END)]
                            search_from = 0
                            positions.setdefault(
                                "first_ui_delivery_ms", position
                            )
                            base, _ = placements(frame)
                            if base:
                                positions["first_tile_delivery_ms"] = position
                                output_tail.clear()
                                break
                        search_from = max(
                            0, len(output_tail) - len(FRAME_END) + 1
                        )
                        if len(output_tail) > 64 * 1024 * 1024:
                            raise RuntimeError("Relay frame exceeds size limit")
            if connection and connection in ready and to_child.size < LIMIT:
                try:
                    data = connection.recv(min(65536, LIMIT - to_child.size))
                except BlockingIOError:
                    data = None
                if data:
                    to_child.append(data)
                elif data == b"":
                    connection.close()
                    connection = None
            for fd in writable:
                if fd == master and not child_eof and to_child.size:
                    try:
                        to_child.flush(master)
                    except OSError as error:
                        if error.errno != errno.EIO:
                            raise
                        child_input_open = False
                elif fd == 1 and to_terminal.size:
                    forwarded_output += to_terminal.flush(1)
                    for name, position in positions.items():
                        if forwarded_output >= position:
                            delivery.setdefault(
                                name, (time.monotonic() - started) * 1000
                            )
        code = child.wait(timeout=3)
        publish(spec_path.parent / "delivery.json", delivery)
        publish(
            spec_path.parent / "relay-overhead.json",
            dict(
                cpu_ms=(time.process_time() - cpu_started) * 1000,
                elapsed_ms=(time.monotonic() - started) * 1000,
                rss_peak_bytes=resource.getrusage(
                    resource.RUSAGE_SELF
                ).ru_maxrss,
            ),
        )
        publish(
            spec_path,
            dict(
                pid=child.pid,
                relay_pid=os.getpid(),
                kitty_pid=os.getppid(),
                geometry=winsize(),
                exit_code=code,
                launched_unix_ns=launched_ns,
            ),
        )
        return code
    finally:
        if child and child.poll() is None:
            os.killpg(child.pid, signal.SIGTERM)
            try:
                child.wait(timeout=3)
            except subprocess.TimeoutExpired:
                os.killpg(child.pid, signal.SIGKILL)
                child.wait()
        if connection:
            connection.close()
        server.close()
        Path(spec["socket"]).unlink(missing_ok=True)
        os.close(master)
        if slave is not None:
            os.close(slave)
        fcntl.fcntl(1, fcntl.F_SETFL, output_flags)
        termios.tcsetattr(0, termios.TCSANOW, saved)


if __name__ == "__main__":
    specification = Path(sys.argv[1])
    try:
        code = relay(specification)
    except Exception as error:
        publish(
            specification.parent / "relay-error.json",
            dict(error=str(error), type=type(error).__name__),
        )
        code = 1
    sys.exit(code)
