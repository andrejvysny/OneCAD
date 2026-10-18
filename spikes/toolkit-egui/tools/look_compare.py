"""Compare a spike screenshot with OneCAD.png: mean sRGB per chrome region, plus composites.

usage: uv run --no-project --with pillow python -I look_compare.py <OneCAD.png> <spike.png> <out_dir>
Both images are 1x captures of a ~1520x920 window. Regions are flat chrome areas that hold no
text or icon in either image, so a delta there is a colour/token delta, not a layout one.
"""
import sys
from pathlib import Path

from PIL import Image, ImageChops, ImageDraw

REGIONS = {
    "titlebar": (420, 6, 1380, 36),
    "tree panel": (150, 100, 175, 700),
    "inspector panel": (1290, 320, 1500, 820),
    "status bar": (300, 893, 1000, 915),
    "canvas (above grid)": (240, 108, 440, 175),
    "toolbar fill": (600, 60, 1000, 63),
    "border tree|viewport": (219, 300, 220, 600),
    "border viewport|inspector": (1262, 300, 1263, 600),
    "border title bottom": (400, 43, 1100, 44),
    "border status top": (300, 886, 1000, 887),
}


def mean(img, box):
    crop = img.crop(box)
    px = [crop.getpixel((x, y)) for y in range(crop.height) for x in range(crop.width)]
    return tuple(round(sum(p[i] for p in px) / len(px)) for i in range(3))


def pill(img, y=899):
    """The active Persp segment: the longest run of blue (sel-bg) pixels on the status row."""
    runs, cur = [], []
    for x in range(1000, 1200):
        r, _, b = img.getpixel((x, y))
        if b - r > 40:
            cur.append(x)
        elif cur:
            runs.append(cur)
            cur = []
    run = max(runs + [cur], key=len)
    return run[0], run[-1], img.getpixel((run[0] + 1, y))


ref = Image.open(sys.argv[1]).convert("RGB")
ours = Image.open(sys.argv[2]).convert("RGB")
out = Path(sys.argv[3])
print(f"sizes ref={ref.size} ours={ours.size}")
print(f"{'region':26} {'OneCAD.png':>15} {'spike':>15}  max|d|")
for name, box in REGIONS.items():
    a, b = mean(ref, box), mean(ours, box)
    print(f"{name:26} {str(a):>15} {str(b):>15}  {max(abs(x - y) for x, y in zip(a, b))}")

pa, pb = pill(ref), pill(ours)
print(f"{'Persp pill fill / x-span':26} {str(pa[2]):>15} {str(pb[2]):>15}  {max(abs(x - y) for x, y in zip(pa[2], pb[2]))}"
      f"  (x {pa[0]}-{pa[1]} vs {pb[0]}-{pb[1]})")

w, h = min(ref.width, ours.width), min(ref.height, ours.height)
ref_c, ours_c = ref.crop((0, 0, w, h)), ours.crop((0, 0, w, h))
side = Image.new("RGB", (2 * w + 8, h), (255, 0, 255))
side.paste(ref_c, (0, 0))
side.paste(ours_c, (w + 8, 0))
side.resize((side.width // 2, side.height // 2), Image.LANCZOS).save(out / "compare-onecad-side-by-side.png")

# 2x zooms of text-heavy chrome: reference row above spike row.
zooms = [(0, 44, 220, 200), (1262, 44, 1521, 200), (1040, 886, 1521, 921), (450, 50, 1060, 106)]
tiles = []
for box in zooms:
    a = ref.crop(box)
    b = ours.crop(box)
    tile = Image.new("RGB", (a.width, a.height * 2 + 4), (255, 0, 255))
    tile.paste(a, (0, 0))
    tile.paste(b, (0, a.height + 4))
    tiles.append(tile.resize((tile.width * 2, tile.height * 2), Image.NEAREST))
sheet = Image.new("RGB", (max(t.width for t in tiles), sum(t.height + 12 for t in tiles)), (30, 30, 30))
y = 0
for t in tiles:
    sheet.paste(t, (0, y))
    y += t.height + 12
sheet.save(out / "compare-onecad-text-zoom.png")

diff = ImageChops.difference(ref_c, ours_c).convert("L").point(lambda v: min(255, v * 3))
d = ImageDraw.Draw(diff)
for box in REGIONS.values():
    d.rectangle(box, outline=255)
diff.save(out / "compare-onecad-diff.png")
print("wrote", *(p.name for p in sorted(out.glob("compare-onecad-*.png"))))
