"""Rebuild vendored fonts from the pinned Nerd Font."""

import hashlib
import json
import sys
import tempfile
import unicodedata
from pathlib import Path

import fontTools
from fontTools import subset
from fontTools.pens.boundsPen import BoundsPen
from fontTools.pens.recordingPen import DecomposingRecordingPen
from fontTools.pens.transformPen import TransformPen
from fontTools.pens.ttGlyphPen import TTGlyphPen
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
        fit_cells(font)
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


# Ink may overhang its cell by this share of the advance.
OVERHANG = 0.1
# Fitted ink keeps this share of the advance as side margins.
MARGIN = 0.05


def fit_cells(font):
    """Shrink wide symbols into their one-cell advance.

    The Nerd Font Mono patch gives wide Iosevka symbols, such as
    circles and arrows, a one-cell advance without narrowing their
    outlines, so they overlap the next character. Combining marks and
    private-use icons overhang by design, and East Asian wide characters
    take two cells, so they stay unchanged.
    """
    glyphs = font["glyf"]
    metrics = font["hmtx"]
    targets = set()
    for code, name in font.getBestCmap().items():
        glyph = glyphs[name]
        advance = metrics[name][0]
        if (
            glyph.numberOfContours == 0
            or advance == 0
            or unicodedata.combining(chr(code))
            or unicodedata.category(chr(code)) in {"Mn", "Me", "Co"}
            or unicodedata.east_asian_width(chr(code)) in {"W", "F"}
        ):
            continue
        glyph.recalcBounds(glyphs)
        if max(glyph.xMax - advance, -glyph.xMin) > advance * OVERHANG:
            targets.add(name)
    # Composites that use a target keep their original outlines.
    dependents = {
        name
        for name in glyphs.keys()
        if name not in targets and uses(glyphs, name, targets)
    }
    shapes = font.getGlyphSet()
    outlines = {}
    for name in sorted(targets | dependents):
        pen = DecomposingRecordingPen(shapes)
        shapes[name].draw(pen)
        bounds = BoundsPen(shapes)
        pen.replay(bounds)
        outlines[name] = (pen, *bounds.bounds)
    for name, (pen, x0, y0, x1, y1) in outlines.items():
        advance = metrics[name][0]
        transform = (1, 0, 0, 1, 0, 0)
        if name in targets:
            scale = min(1, advance * (1 - 2 * MARGIN) / (x1 - x0))
            middle = (y0 + y1) / 2
            transform = (
                scale,
                0,
                0,
                scale,
                advance / 2 - scale * (x0 + x1) / 2,
                middle - scale * middle,
            )
        output = TTGlyphPen(None)
        pen.replay(TransformPen(output, transform))
        glyph = output.glyph()
        glyph.recalcBounds(glyphs)
        glyphs[name] = glyph
        metrics[name] = (advance, glyph.xMin)


def uses(glyphs, name, targets):
    pending = [name]
    while pending:
        glyph = glyphs[pending.pop()]
        if not glyph.isComposite():
            continue
        for component in glyph.components:
            if component.glyphName in targets:
                return True
            pending.append(component.glyphName)
    return False


if __name__ == "__main__":
    main()
