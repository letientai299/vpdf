"""Rebuild vendored fonts from the pinned Nerd Font."""

import hashlib
import json
import sys
import tempfile
from pathlib import Path

import fontTools
from fontTools import subset
from fontTools.ttLib import TTFont

ROOT = Path(__file__).resolve().parents[2]


def main():
    pin = json.loads((ROOT / "tools/help/help/renderer.json").read_bytes())
    if fontTools.version != pin["font_source"]["fonttools"]:
        raise ValueError("FontTools version differs from pin")
    source = Path(sys.argv[1])
    with tempfile.TemporaryDirectory(
        dir=ROOT / "crates/ui/assets"
    ) as temporary:
        rebuild(pin, source, Path(temporary))


def rebuild(pin, source, staging):
    for name in pin["font_files"]:
        style = next(
            (
                style
                for style in ["BoldItalic", "Bold", "Italic"]
                if style in name
            ),
            "Regular",
        )
        origin = (
            pin["font_source"]
            if style == "Regular"
            else pin["font_source"]["styles"][style]
        )
        file = source.parent / origin["source"].rsplit("/", 1)[1]
        if hashlib.sha256(file.read_bytes()).hexdigest() != origin["sha256"]:
            raise ValueError("Source font hash differs from pin")
        font = TTFont(file, recalcTimestamp=False)
        pencil = font.getBestCmap()[0xF040]
        for table in font["cmap"].tables:
            if table.isUnicode():
                table.cmap[0x270E] = pencil
        options = subset.Options()
        options.layout_features = []
        subsetter = subset.Subsetter(options=options)
        subsetter.populate(
            unicodes=(
                font.getBestCmap() if name == "VpdfMono.ttf" else range(0x3000)
            )
        )
        subsetter.subset(font)
        for identifier, text in [
            (1, "Vpdf Mono"),
            (2, style),
            (3, f"Vpdf Mono {style} 34.8.0 NF 3.5.1"),
            (4, f"Vpdf Mono {style}"),
            (6, f"VpdfMono-{style}"),
            (16, "Vpdf Mono"),
            (17, style),
        ]:
            font["name"].removeNames(nameID=identifier)
            font["name"].setName(text, identifier, 3, 1, 0x409)
            font["name"].setName(text, identifier, 1, 0, 0)
        output = staging / name
        font.save(output)
        checksum = hashlib.sha256(output.read_bytes()).hexdigest()
        if "--update-pin" in sys.argv:
            pin["font_files"][name] = checksum
        elif checksum != pin["font_files"][name]:
            raise ValueError("Rebuilt font hash differs from pin")
    for name in pin["font_files"]:
        (staging / name).replace(ROOT / "crates/ui/assets" / name)
    if "--update-pin" in sys.argv:
        (ROOT / "tools/help/help/renderer.json").write_text(
            json.dumps(pin, indent=2) + "\n"
        )


if __name__ == "__main__":
    main()
