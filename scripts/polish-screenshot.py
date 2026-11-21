"""Turns a raw screenshot of the setup window into a clean docs image.

A window capture on Windows 11 carries a 1 px ring of the desktop around
the window, shows the desktop again in its rounded corners, and tints the
title bar with the wallpaper's colour. This script crops that ring, rounds
the corners with real transparency, turns the title bar neutral grey, adds a
hairline edge so a dark window stays visible on a dark page, and a soft
shadow.

    python scripts/polish-screenshot.py RAW.png OUT.png [--edge light|dark]

Run it on the raw capture, never on its own output (it would crop again).
Needs Pillow (`pip install pillow`).
"""

import argparse

from PIL import Image, ImageChops, ImageDraw, ImageFilter

# Windows 11 rounds top-level windows by 8 px; a hair more hides the
# anti-aliased desktop pixels in the corner.
RADIUS = 9
RING = 1  # desktop pixels around the captured window
PAD = 36  # room for the shadow
SCALE = 4  # supersampling for smooth corners


def rounded_mask(size, radius):
    width, height = size
    big = Image.new("L", (width * SCALE, height * SCALE), 0)
    ImageDraw.Draw(big).rounded_rectangle(
        (0, 0, width * SCALE - 1, height * SCALE - 1), radius * SCALE, fill=255
    )
    return big.resize(size, Image.LANCZOS)


def neutral_title_bar(window):
    """Windows 11 paints the title bar with Mica, which tints it with the
    wallpaper's colour. Find where it ends (the first row whose brightness
    jumps at the window's centre) and turn that band neutral grey."""
    px = window.load()
    centre = window.width // 2
    luma = lambda p: (p[0] * 299 + p[1] * 587 + p[2] * 114) // 1000
    end = next((y for y in range(1, 80) if abs(luma(px[centre, y]) - luma(px[centre, y - 1])) > 12), 0)
    for y in range(end):
        for x in range(window.width):
            r, g, b, a = px[x, y]
            grey = luma((r, g, b))
            px[x, y] = (grey, grey, grey, a)
    return window


def polish(raw_path, out_path, edge):
    raw = Image.open(raw_path).convert("RGBA")
    width, height = raw.size
    window = raw.crop((RING, RING, width - RING, height - RING))
    window = neutral_title_bar(window)
    mask = rounded_mask(window.size, RADIUS)
    window.putalpha(mask)

    # A 1 px edge just inside the rounded rectangle.
    inner = rounded_mask(window.size, RADIUS)
    inner = Image.eval(inner, lambda v: v)  # copy
    shrunk = Image.new("L", window.size, 0)
    shrunk.paste(inner.resize((window.width - 2, window.height - 2), Image.LANCZOS), (1, 1))
    ring = ImageChops.subtract(mask, shrunk)
    tint = (0, 0, 0, 0) if edge == "light" else (255, 255, 255, 0)
    strength = 0.16 if edge == "light" else 0.18
    ring_layer = Image.new("RGBA", window.size, tint)
    ring_layer.putalpha(ring.point(lambda v: int(v * strength)))
    window = Image.alpha_composite(window, ring_layer)

    canvas = Image.new("RGBA", (window.width + PAD * 2, window.height + PAD * 2), (0, 0, 0, 0))
    shadow = Image.new("L", canvas.size, 0)
    shadow.paste(mask, (PAD, PAD + 10))
    shadow = shadow.filter(ImageFilter.GaussianBlur(14)).point(lambda v: int(v * 0.30))
    shade = Image.new("RGBA", canvas.size, (0, 0, 0, 0))
    shade.putalpha(shadow)
    canvas = Image.alpha_composite(canvas, shade)
    canvas.alpha_composite(window, (PAD, PAD))
    canvas.save(out_path, optimize=True)
    print(f"Wrote {out_path} ({canvas.width}x{canvas.height})")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    parser.add_argument("raw")
    parser.add_argument("out")
    parser.add_argument("--edge", choices=["light", "dark"], default="light")
    args = parser.parse_args()
    polish(args.raw, args.out, args.edge)
