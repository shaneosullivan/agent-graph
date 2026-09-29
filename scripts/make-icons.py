#!/usr/bin/env python3
"""Makes every icon and logo size from assets/images/logo_original.png.

    python3 scripts/make-icons.py      # needs Pillow: pip install Pillow

Run it again after changing the logo, and commit what it writes:

- assets/images/logo-256.png, logo-256-dark.png: the logo, for the README
  (the second for GitHub's dark theme).
- assets/windows/agent-graph.ico: the Windows program's icon (build.rs
  embeds it in agent-graph.exe), 16 to 256 px, on a white tile so it shows
  on a dark taskbar.
- src/view/assets/icon.svg: `agent-graph view`'s favicon and header logo,
  and the site's (site/app/icon.svg, a copy): the logo, with a lighter one
  for a dark theme, chosen by the SVG itself.
- site/app/favicon.ico (16, 32, 48 px, on a white tile, for browsers that
  don't take the SVG) and site/app/apple-icon.png (180 px, iOS's home
  screen: opaque, as iOS wants).
- site/public/icons/: icon-192.png and icon-512.png (the web app manifest's,
  app/manifest.ts), maskable-512.png (Android crops it to a circle or
  squircle, so the logo keeps to the middle), and logo-512.png (for the
  sharing image, app/opengraph-image.tsx).

The original has a lot of empty space around it; each size is cropped to
the logo, then placed with the margin it needs.
"""

import base64
import colorsys
import io
from pathlib import Path

from PIL import Image, ImageDraw

ROOT = Path(__file__).resolve().parent.parent
SOURCE = ROOT / "assets/images/logo_original.png"


def load():
    im = Image.open(SOURCE).convert("RGBA")
    # Its alpha has stray faint pixels; what's visible is the logo.
    box = im.getchannel("A").point(lambda a: 255 if a > 8 else 0).getbbox()
    return im.crop(box)


def darkened(logo):
    """The logo for a dark background: its slate parts lightened, the blue kept."""
    out = logo.copy()
    px = out.load()
    for y in range(out.height):
        for x in range(out.width):
            r, g, b, a = px[x, y]
            if a == 0:
                continue
            h, l, s = colorsys.rgb_to_hls(r / 255, g / 255, b / 255)
            # The glasses and lines are a low-saturation slate; the dot's a
            # saturated blue.
            if s < 0.45 or l < 0.35 and s < 0.6:
                l = 0.93 - l * 0.8
                r2, g2, b2 = colorsys.hls_to_rgb(h, l, s)
                px[x, y] = (round(r2 * 255), round(g2 * 255), round(b2 * 255), a)
    return out


def placed(logo, size, fill, tile=None, radius=0.0):
    """`logo` centred on a `size` px square, its longer side `fill` of it;
    on a `tile` colour (rounded by `radius` of the size), or transparent."""
    canvas = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    if tile:
        mask = Image.new("L", (size * 4, size * 4), 0)
        ImageDraw.Draw(mask).rounded_rectangle(
            (0, 0, size * 4 - 1, size * 4 - 1), radius=size * 4 * radius, fill=255
        )
        plate = Image.new("RGBA", (size, size), tile)
        canvas.paste(plate, (0, 0), mask.resize((size, size), Image.LANCZOS))
    scale = fill * size / max(logo.size)
    w, h = max(1, round(logo.width * scale)), max(1, round(logo.height * scale))
    small = logo.resize((w, h), Image.LANCZOS)
    canvas.alpha_composite(small, ((size - w) // 2, (size - h) // 2))
    return canvas


def png_bytes(im, quantize=False):
    if quantize:
        im = im.quantize(colors=256, method=Image.Quantize.FASTOCTREE)
    buf = io.BytesIO()
    im.save(buf, "PNG", optimize=True)
    return buf.getvalue()


def save_png(im, path, quantize=False):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(png_bytes(im, quantize))
    print(f"{path.relative_to(ROOT)}  {im.width}x{im.height}  {path.stat().st_size:,} bytes")


def save_ico(images, path):
    path.parent.mkdir(parents=True, exist_ok=True)
    biggest = max(images, key=lambda im: im.width)
    biggest.save(
        path,
        "ICO",
        sizes=[(im.width, im.height) for im in images],
        append_images=[im for im in images if im is not biggest],
    )
    sizes = ", ".join(str(im.width) for im in sorted(images, key=lambda im: im.width))
    print(f"{path.relative_to(ROOT)}  {sizes}  {path.stat().st_size:,} bytes")


def icon_svg(logo, dark):
    """An SVG of the logo that picks the dark one itself, where the viewer's
    theme is dark: 128 px, enough for a 32 px favicon, and the header's,
    on a 3x screen."""
    size = 128
    light = base64.b64encode(png_bytes(placed(logo, size, 1.0), True)).decode()
    night = base64.b64encode(png_bytes(placed(dark, size, 1.0), True)).decode()
    return (
        f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {size} {size}">'
        "<style>.d{display:none}"
        "@media (prefers-color-scheme:dark){.l{display:none}.d{display:inline}}</style>"
        f'<image class="l" width="{size}" height="{size}" href="data:image/png;base64,{light}"/>'
        f'<image class="d" width="{size}" height="{size}" href="data:image/png;base64,{night}"/>'
        "</svg>\n"
    )


def main():
    logo = load()
    dark = darkened(logo)
    white = (255, 255, 255, 255)

    save_png(placed(logo, 256, 0.96), ROOT / "assets/images/logo-256.png")
    save_png(placed(dark, 256, 0.96), ROOT / "assets/images/logo-256-dark.png")

    # Windows: a white rounded tile, as the taskbar and Explorer can be dark.
    save_ico(
        [placed(logo, s, 0.8, white, 0.22) for s in (16, 24, 32, 48, 64, 128, 256)],
        ROOT / "assets/windows/agent-graph.ico",
    )

    svg = icon_svg(logo, dark)
    for path in (ROOT / "src/view/assets/icon.svg", ROOT / "site/app/icon.svg"):
        path.write_text(svg)
        print(f"{path.relative_to(ROOT)}  {path.stat().st_size:,} bytes")

    site = ROOT / "site"
    save_ico(
        [placed(logo, s, 0.84, white, 0.22) for s in (16, 32, 48)],
        site / "app/favicon.ico",
    )
    save_png(placed(logo, 180, 0.72, white), site / "app/apple-icon.png")
    icons = site / "public/icons"
    save_png(placed(logo, 192, 0.76, white), icons / "icon-192.png")
    save_png(placed(logo, 512, 0.76, white), icons / "icon-512.png")
    # The safe zone is the middle 80% circle: the logo keeps well inside.
    save_png(placed(logo, 512, 0.56, white), icons / "maskable-512.png")
    save_png(placed(logo, 512, 1.0), icons / "logo-512.png")


if __name__ == "__main__":
    main()
