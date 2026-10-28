"""Encode scenario keys for the terminal decoder."""

import base64


def mark(name):
    value = base64.b64encode(name.encode()).decode()
    return f"\x1b]777;vpdf-bench-mark={value}\x1b\\".encode()


def encode(specification):
    named = dict(
        space=32, enter=13, return_=13, esc=27, escape=27, tab=9, backspace=127
    )
    tilde = dict(home=1, delete=3, end=4, pageup=5, pagedown=6)
    arrows = dict(up="A", down="B", right="C", left="D")
    result = bytearray()
    for key in specification.split():
        bits = 0
        while len(key) > 2 and key[1] == "-" and key[0] in "CSAD":
            bits |= dict(C=4, S=1, A=2, D=8)[key[0]]
            key = key[2:]
        if key == "_" and bits & 14:
            bits |= 1
        lower = key.lower()
        if lower in tilde:
            result.extend(f"\x1b[{tilde[lower]};{bits + 1}~".encode())
        elif lower in arrows:
            result.extend(f"\x1b[1;{bits + 1}{arrows[lower]}".encode())
        else:
            code = named.get("return_" if lower == "return" else lower)
            if code is None:
                if len(key) != 1:
                    raise ValueError(f"Unknown scenario key: {key}")
                code = ord(key)
            result.extend(f"\x1b[{code};{bits + 1}u".encode())
    if not result:
        raise ValueError("Empty scenario key list")
    return bytes(result)


def wheel(dx, dy, geometry):
    _, _, width, height = geometry
    if not width or not height or max(abs(dx), abs(dy)) > 64:
        raise ValueError("Invalid terminal wheel geometry or turns")
    x, y = width // 2 + 1, height // 2 + 1
    result = bytearray()
    for delta, positive, negative in ((dx, 66, 67), (dy, 64, 65)):
        code = positive if delta > 0 else negative
        result.extend(f"\x1b[<{code};{x};{y}M".encode() * abs(delta))
    return bytes(result)
