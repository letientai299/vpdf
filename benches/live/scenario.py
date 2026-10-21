"""Validate live scenario headers and steps."""

import hashlib
import re
from pathlib import Path

from keys import encode, wheel
from runtime import duration

ROOT = Path(__file__).resolve().parents[2]


def digest(path):
    with Path(path).open("rb") as source:
        return hashlib.file_digest(source, "sha256").hexdigest()


def coordinates(text):
    words = text.split()
    if len(words) != 2 or any(
        not re.fullmatch(r"[+-]?[0-9]+", word) for word in words
    ):
        raise ValueError("Step needs two coordinates")
    return tuple(map(int, words))


def validate_step(line):
    words = line.split(maxsplit=1)
    verb, rest = words[0], words[1].strip() if len(words) == 2 else ""
    if verb == "key":
        encode(rest)
    elif verb == "wheel":
        wheel(*coordinates(rest), [1, 1, 1, 1])
    elif verb == "resize":
        if any(not 1 <= value <= 8192 for value in coordinates(rest)):
            raise ValueError("Invalid scenario size")
    elif verb == "place":
        if any(
            not -(1 << 31) <= value < 1 << 31 for value in coordinates(rest)
        ):
            raise ValueError("Invalid scenario position")
    elif verb == "mark":
        if not rest:
            raise ValueError("Empty scenario mark")
    elif verb == "expect":
        from expectation import validate

        validate(rest)
    elif verb == "wait":
        if rest not in ("idle", "complete"):
            duration(rest)
    elif verb == "quit":
        if rest:
            raise ValueError("Quit takes no arguments")
    elif verb == "mix":
        seed, choices = rest.split(maxsplit=1)
        if not seed.isascii() or not seed.isdecimal() or int(seed) >= 1 << 64:
            raise ValueError("Invalid scenario seed")
        for choice in choices.split("|"):
            encode(choice)
    elif verb == "repeat":
        span, rate, repeated = rest.split(maxsplit=2)
        if (
            duration(span) <= 0
            or not rate.startswith("@")
            or not rate[1:].isascii()
            or not rate[1:].isdecimal()
            or not 1 <= int(rate[1:]) <= 1000
        ):
            raise ValueError("Invalid scenario repetition")
        repeated = validate_step(repeated)
        if repeated.split()[0] not in ("key", "wheel", "mix"):
            raise ValueError("Repeat needs keys, wheel, or mix")
        rest = f"{span} {rate} {repeated}"
    else:
        raise ValueError(f"Unsupported scenario step: {verb}")
    return f"{verb} {rest}".rstrip()


def scenario(path, corpus, huge=None):
    files, steps, headers = [], [], {}
    cell = (10, 20)
    for number, line in enumerate(path.read_text().splitlines(), 1):
        line = line.strip()
        if not line:
            continue
        try:
            if not line.startswith("#"):
                steps.append(validate_step(line))
                continue
            words = line[1:].split()
            if not words:
                continue
            name, values = words[0], words[1:]
            if name == "pdf":
                pdf_name, sha = values
                if not re.fullmatch(
                    r"[A-Za-z0-9_-]+", pdf_name
                ) or not re.fullmatch(r"[a-fA-F0-9]{64}", sha):
                    raise ValueError("Invalid PDF name or hash")
                pdf = corpus / (pdf_name + ".pdf")
                if not pdf.is_file():
                    pdf = (huge or ROOT / "tmp/huge") / (pdf_name + ".pdf")
                if digest(pdf) != sha.lower():
                    raise ValueError(f"PDF hash mismatch: {pdf}")
                files.append(pdf)
            elif name == "cell":
                cell = coordinates(" ".join(values))
                if any(not 1 <= value <= 65535 for value in cell):
                    raise ValueError("Invalid scenario cell")
            elif name in ("window", "crop"):
                if name in headers:
                    raise ValueError(f"Repeated scenario header: {name}")
                if name == "window":
                    size = coordinates(" ".join(values))
                    if any(not 1 <= value <= 8192 for value in size):
                        raise ValueError("Invalid scenario window")
                    headers[name] = size
                else:
                    (crop,) = values
                    if crop not in ("on", "off"):
                        raise ValueError("Invalid scenario crop")
                    headers[name] = crop
        except (ValueError, OSError) as error:
            raise ValueError(f"Scenario line {number}: {error}") from error
    if not files or not steps or set(headers) != {"window", "crop"}:
        raise ValueError("Scenario headers or steps missing")
    size = headers["window"]
    if size[0] // cell[0] < 1 or size[1] // cell[1] < 2:
        raise ValueError("Scenario grid is too small")
    return files, steps, size, headers["crop"]
