"""Draw the documentation site's favicon: "bg" in white Paper Mono, slanted like the wordmark, on
the wordmark's blue.

Browsers draw a favicon as an image without web fonts, so the letters are outlines taken from the
font, slanted 14 degrees as the browser slants the wordmark, and heavier than the wordmark's 500
so they hold up at 16px in a tab.

    uv run --with fonttools --with brotli python scripts/favicon.py web/public/fonts/PaperMono-Variable.woff2 web/public/favicon.svg
"""
import math
import sys

from fontTools.pens.boundsPen import BoundsPen
from fontTools.pens.svgPathPen import SVGPathPen
from fontTools.pens.transformPen import TransformPen
from fontTools.ttLib import TTFont
from fontTools.varLib.instancer import instantiateVariableFont

TEXT = 'bg'
WEIGHT = 700
SLANT = math.tan(math.radians(14))
SIZE = 32  # the icon's size in user units
MARGIN = 4  # the space around the letters
BLUE = '#0044FF'


def main():
    source, target = sys.argv[1], sys.argv[2]
    font = instantiateVariableFont(TTFont(source), {'wght': WEIGHT})
    glyphs = font.getGlyphSet()
    cmap = font.getBestCmap()
    names = [cmap[ord(c)] for c in TEXT]

    # Lay the glyphs out side by side, slanted, in font units with y up.
    def draw(pen):
        x = 0
        for name in names:
            # Shear around the baseline, then move to the glyph's place.
            glyphs[name].draw(TransformPen(pen, (1, 0, SLANT, 1, x, 0)))
            x += glyphs[name].width

    bounds = BoundsPen(glyphs)
    draw(bounds)
    x0, y0, x1, y1 = bounds.bounds
    scale = (SIZE - 2 * MARGIN) / max(x1 - x0, y1 - y0)
    # Centre the letters, flipping y for SVG.
    dx = (SIZE - (x1 - x0) * scale) / 2 - x0 * scale
    dy = (SIZE + (y1 - y0) * scale) / 2 + y0 * scale

    path = SVGPathPen(glyphs, ntos=lambda v: f'{v:.2f}'.rstrip('0').rstrip('.'))
    draw(TransformPen(path, (scale, 0, 0, -scale, dx, dy)))
    svg = (
        f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {SIZE} {SIZE}">'
        f'<rect width="{SIZE}" height="{SIZE}" fill="{BLUE}"/>'
        f'<path d="{path.getCommands()}" fill="#fff"/>'
        '</svg>\n'
    )
    with open(target, 'w', encoding='utf-8') as f:
        f.write(svg)
    print(f'{target}: {len(svg)} bytes')


if __name__ == '__main__':
    main()
