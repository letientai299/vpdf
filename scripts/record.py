#!/usr/bin/env python3
"""Records raw terminal input for decoder fixtures: enables the modes
vpdf uses, sends its queries, and logs each read on its own line as a
Python bytes literal. Run it inside the terminal under test.
Usage: record.py OUT SECONDS [pixels]"""
import os, select, sys, termios, time, tty

out, seconds = sys.argv[1], float(sys.argv[2])
pixels = len(sys.argv) > 3
fd = os.open("/dev/tty", os.O_RDWR)
saved = termios.tcgetattr(fd)
tty.setraw(fd)
setup = (
    b"\x1b[?1049h\x1b[?1000h\x1b[?1002h\x1b[?1003h\x1b[?1006h"
    + (b"\x1b[?1016h" if pixels else b"")
    + b"\x1b[?2004h\x1b[?1004h\x1b[?2048h\x1b[>5u"
    + b"\x1b[>0q"
    + b"\x1b_Gi=31,s=1,v=1,a=q,t=d,f=24;AAAA\x1b\\"
    + b"\x1b_Ga=p,i=999999\x1b\\"
    + b"\x1b[?2026$p\x1b[?1016$p\x1b[?2048$p\x1b[?u"
    + b"\x1b]22;?pointer,crosshair\x1b\\"
    + b"\x1b]72;t=q\x1b\\"
    + b"\x1b[16t\x1b[14t"
    + b"\x1b[c"
)
try:
    os.write(fd, setup)
    os.write(fd, b"recording\r\n")
    end = time.time() + seconds
    with open(out, "w") as log:
        while (left := end - time.time()) > 0:
            ready, _, _ = select.select([fd], [], [], left)
            if ready:
                chunk = os.read(fd, 16384)
                log.write(repr(chunk) + "\n")
                log.flush()
finally:
    os.write(fd, b"\x1b[<u\x1b[?2048l\x1b[?1004l\x1b[?2004l\x1b[?1016l"
             b"\x1b[?1006l\x1b[?1003l\x1b[?1002l\x1b[?1000l\x1b[?1049l")
    termios.tcsetattr(fd, termios.TCSANOW, saved)
