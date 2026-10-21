import json
import os
import pty
import select
import socket
import subprocess
import sys
import tempfile
import time
import unittest
from contextlib import contextmanager
from pathlib import Path

from keys import encode, mark, wheel


class TransportTests(unittest.TestCase):
    @contextmanager
    def relay(self, folder, child_code, timeout=10):
        ready = folder / "ready"
        code = (
            "import os,time,tty;from pathlib import Path;tty.setraw(0);"
            f"Path({str(ready)!r}).touch()\n" + child_code
        )
        spec = folder / "spec.json"
        spec.write_text(
            json.dumps(
                dict(
                    command=[sys.executable, "-B", "-c", code],
                    env={},
                    socket=str(folder / "socket"),
                    result=str(folder / "relay.json"),
                    timeout=timeout,
                )
            )
        )
        master, slave = pty.openpty()
        process = subprocess.Popen(
            [
                sys.executable,
                "-B",
                str(Path(__file__).with_name("pty_bridge.py")),
                str(spec),
            ],
            stdin=slave,
            stdout=slave,
            stderr=slave,
        )
        os.close(slave)
        control = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        control.settimeout(2)
        try:
            deadline = time.monotonic() + 5
            while not ready.exists():
                if process.poll() is not None or time.monotonic() > deadline:
                    self.fail("Relay child failed to start")
                time.sleep(0.005)
            control.connect(str(folder / "socket"))
            yield process, master, control
        finally:
            control.close()
            if process.poll() is None:
                process.terminate()
                process.wait(timeout=3)
            os.close(master)

    def drain(self, process, master):
        result = bytearray()
        deadline = time.monotonic() + 5
        while True:
            if time.monotonic() > deadline:
                self.fail("Relay deadline was not enforced")
            if select.select([master], [], [], 0.01)[0]:
                try:
                    data = os.read(master, 65536)
                except OSError:
                    break
                if not data:
                    break
                result.extend(data)
            elif process.poll() is not None:
                break
        process.wait(timeout=3)
        return bytes(result)

    def test_key_bytes(self):
        self.assertEqual(
            encode("C-d S-space g o"),
            b"\x1b[100;5u\x1b[32;2u\x1b[103;1u\x1b[111;1u",
        )
        self.assertEqual(
            encode("pagedown S-tab 中"), b"\x1b[6;1~\x1b[9;2u\x1b[20013;1u"
        )
        self.assertEqual(
            mark("phase"), b"\x1b]777;vpdf-bench-mark=cGhhc2U=\x1b\\"
        )
        with self.assertRaises(ValueError):
            encode("unknown")

    def test_wheel_bytes(self):
        self.assertEqual(
            wheel(0, -1, [40, 100, 1000, 800]), b"\x1b[<65;501;401M"
        )
        with self.assertRaises(ValueError):
            wheel(0, 1, [40, 100, 0, 0])

    def test_relay_preserves_fast_stream(self):
        with tempfile.TemporaryDirectory(
            prefix="vb-test-", dir="/tmp"
        ) as folder:
            folder = Path(folder)
            result = folder / "received"
            code = (
                "data=b''\nwhile data.count(b'u')<120:data+=os.read(0,65536)\n"
                f"Path({str(result)!r}).write_bytes(data)\n"
            )
            with self.relay(folder, code) as (process, master, control):
                packet = encode("C-d")
                started = time.monotonic()
                for index in range(120):
                    time.sleep(max(0, started + index / 240 - time.monotonic()))
                    control.sendall(packet)
                self.drain(process, master)
                self.assertEqual(process.returncode, 0)
                self.assertEqual(result.read_bytes(), packet * 120)

    def test_bidirectional_backpressure_preserves_output_and_input(self):
        with tempfile.TemporaryDirectory(
            prefix="vb-test-", dir="/tmp"
        ) as folder:
            folder = Path(folder)
            result = folder / "received"
            code = (
                "time.sleep(.1)\noutput=b'x'*(1024*1024)\n"
                "while output:output=output[os.write(1,output):]\n"
                "data=b''\nwhile len(data)<100000:data+=os.read(0,65536)\n"
                f"Path({str(result)!r}).write_bytes(data)\n"
                "os.write(1,b'done')\n"
            )
            with self.relay(folder, code, timeout=2) as (
                process,
                master,
                control,
            ):
                control.sendall(b"k" * 100000)
                output = self.drain(process, master)
                self.assertEqual(process.returncode, 0)
                self.assertEqual(result.read_bytes(), b"k" * 100000)
                self.assertEqual(output, b"x" * (1024 * 1024) + b"done")

    def test_blocked_output_still_enforces_deadline(self):
        with tempfile.TemporaryDirectory(
            prefix="vb-test-", dir="/tmp"
        ) as folder:
            folder = Path(folder)
            code = "output=b'x'*(8*1024*1024)\nwhile output:output=output[os.write(1,output):]\n"
            with self.relay(folder, code, timeout=0.3) as (process, master, _):
                process.wait(timeout=3)
                self.assertNotEqual(process.returncode, 0)
                failure = json.loads((folder / "relay-error.json").read_text())
                self.assertEqual(failure["type"], "TimeoutError")
                child = json.loads((folder / "relay.json").read_text())["pid"]
                with self.assertRaises(ProcessLookupError):
                    os.kill(child, 0)


if __name__ == "__main__":
    unittest.main()
