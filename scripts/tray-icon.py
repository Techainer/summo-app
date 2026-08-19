#!/usr/bin/env python3
"""The icon in the menu bar, which is not the icon in the dock.

    python3 scripts/tray-icon.py

The tray was drawn from `app.default_window_icon()` — the application icon, which is a dark rounded
square with three green bars on it. That is right for a dock and wrong for a menu bar, and on macOS
it is wrong in a specific way: `icon_as_template(true)` tells the system to ignore the colours and
render the **alpha channel** in the menu bar's own colour, so the whole thing can adapt to light and
dark. Feed it a picture whose alpha is a filled square and macOS faithfully draws a filled square.

That is what a user reported: a grey square where the logo should be.

So the tray gets its own file: the three bars, nothing behind them. Transparent everywhere else, so
the template renders the mark and not its background — and on Windows and Linux, where there is no
template concept, a transparent background is what an icon should have anyway.

22 points is the macOS menu bar's own size; `tray.png` is drawn at 44 for a Retina display and
scaled down by the system on a non-Retina one. The bars are the same three from the wordmark, in
the same proportions.
"""

from pathlib import Path

from PIL import Image, ImageDraw

OUT = Path(__file__).resolve().parent.parent / "apps/desktop/src-tauri/icons"

# 44 = 22 points at 2×. Drawn at 4× and reduced, so the rounded ends are not staircases.
SIZE = 44
SCALE = 4

# Heights as a fraction of the icon, and the mark's proportions: short, tall, middle.
BARS = (0.42, 0.82, 0.60)


def compose() -> Image.Image:
    canvas = SIZE * SCALE
    image = Image.new("RGBA", (canvas, canvas), (0, 0, 0, 0))
    draw = ImageDraw.Draw(image)

    width = canvas * 0.15
    gap = canvas * 0.10
    total = len(BARS) * width + (len(BARS) - 1) * gap
    left = (canvas - total) / 2
    # Sat on a baseline rather than centred: the bars are a level meter, and a meter grows from the
    # bottom.
    baseline = canvas * 0.88

    for index, fraction in enumerate(BARS):
        x = left + index * (width + gap)
        height = canvas * fraction
        draw.rounded_rectangle(
            [x, baseline - height, x + width, baseline],
            radius=width / 2,
            # White, and the colour is irrelevant on macOS — only the alpha is read. On Windows and
            # Linux the icon is drawn as it is, and a menu bar there is dark far more often than
            # not.
            fill=(255, 255, 255, 255),
        )

    return image.resize((SIZE, SIZE), Image.LANCZOS)


picture = compose()
picture.save(OUT / "tray.png")
print(f"tray.png — {picture.size[0]}×{picture.size[1]}, alpha-only")
