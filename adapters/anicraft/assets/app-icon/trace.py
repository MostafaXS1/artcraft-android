"""Trace the rabbit artwork into the app icon's master SVG.

Needs numpy, Pillow, SciPy and vtracer (`pip install vtracer`); see README.md for the exact run.
usage: python trace.py SOURCE OUT_SVG WORK_PX SPECKLE CROP_PX

CROP_PX keeps the top-left CROP_PX square of the source: the drawing stops short of its right and
bottom edges, so the crop lets the body run off the tile like the other Crafting Apps icons.
"""

import re
import sys
import tempfile
from pathlib import Path

import numpy as np
import vtracer
from PIL import Image, ImageFilter
from scipy import ndimage

# Ink and paper are the Crafting Apps' shared engraving colours; the paper is a neutral white so
# the rabbit doesn't read as cream against the yellow.
INK, PAPER, FIELD = "#0b0b0c", "#f2f1ee", "#dac604"


def trace(mask: np.ndarray, speckle: int, d: Path) -> str:
    """One colour layer: vtracer's binary trace of `mask`, as path elements."""
    png, svg = d / "mask.png", d / "mask.svg"
    Image.fromarray(np.where(mask, 0, 255).astype(np.uint8)).filter(ImageFilter.MedianFilter(3)).save(png)
    vtracer.convert_image_to_svg_py(
        str(png), str(svg), colormode="binary", mode="spline", filter_speckle=speckle,
        corner_threshold=60, length_threshold=4.0, splice_threshold=45, path_precision=1,
    )
    found = re.findall(r'<path d="([^"]+)"[^>]*transform="translate\(([-\d.]+),([-\d.]+)\)"', svg.read_text())
    return "".join(f'<path transform="translate({x} {y})" d="{p.strip()}"/>' for p, x, y in found)


def main() -> None:
    src, out, n, speckle, crop = sys.argv[1], Path(sys.argv[2]), int(sys.argv[3]), int(sys.argv[4]), int(sys.argv[5])
    im = Image.open(src).convert("RGB").crop((0, 0, crop, crop)).resize((n, n), Image.LANCZOS)
    a = np.asarray(im).astype(np.int32)
    r, g, b = a[..., 0], a[..., 1], a[..., 2]
    # The field is saturated yellow: blue far below red and green (ink over it stays yellowish).
    yellow = (np.minimum(r, g) - b) > 0.55 * np.maximum(r, 1)
    dark = 0.299 * r + 0.587 * g + 0.114 * b < 120

    # The tile's rounding is done by SVG clips, so the art runs past it here. Only the source's own
    # dark olive margin goes: the dark regions joined to an image corner.
    olive, _ = ndimage.label(dark & (b < 0.25 * np.maximum(r, 1)))
    margin = np.isin(olive, [c for c in {olive[0, 0], olive[0, -1], olive[-1, 0], olive[-1, -1]} if c])
    inside = ~ndimage.binary_dilation(margin, iterations=2)

    # Paper is the rabbit's silhouette: everything not yellow, opened so whiskers and stray fur
    # strokes over the field stay ink on yellow (no white halo), holes filled.
    disk = np.hypot(*np.mgrid[-3:4, -3:4]) <= 3.2
    silhouette = ndimage.binary_fill_holes(ndimage.binary_opening(inside & ~yellow, structure=disk, iterations=max(1, n // 1000)))

    with tempfile.TemporaryDirectory() as d:
        paper = trace(silhouette & inside, speckle, Path(d))
        ink = trace(inside & dark, speckle, Path(d))
    # The Crafting Apps tile (rx 112 of 512) filled with the field, and the art clipped 6 units inside
    # it (a concentric rx 106), so a band of field colour runs round every edge as on the other icons.
    out.write_text(
        '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 512 512">'
        '<defs><clipPath id="a"><rect x="6" y="6" width="500" height="500" rx="106"/></clipPath></defs>'
        f'<rect width="512" height="512" rx="112" fill="{FIELD}"/>'
        f'<g clip-path="url(#a)"><g transform="scale({512 / n:g})" fill-rule="evenodd">'
        f'<g fill="{PAPER}">{paper}</g><g fill="{INK}">{ink}</g></g></g></svg>\n',
        encoding="utf-8",
    )


if __name__ == "__main__":
    main()
