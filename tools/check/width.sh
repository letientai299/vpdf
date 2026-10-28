#!/bin/sh
set -eu
if [ "$#" -eq 0 ]; then
  find src tests crates tools -name '*.rs' -type f \
    -exec sh "$0" {} +
else
  python3 -B - "$@" <<'PYTHON'
import sys
from pathlib import Path

bad = False
for name in sys.argv[1:]:
    with Path(name).open(encoding="utf-8") as source:
        for number, line in enumerate(source, 1):
            if len(line.rstrip("\r\n")) > 80:
                print(f"{name}:{number}: exceeds 80 columns")
                bad = True
sys.exit(int(bad))
PYTHON
fi
