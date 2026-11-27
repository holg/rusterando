"""An original chalkboard texture for the "Tafel" menu theme (no third-party
image): dark slate, cloudy unevenness, wiped-chalk swirls, chalk dust.

    python3 -I scripts/gen_blackboard.py OUT.jpg WIDTH_MM HEIGHT_MM DPI [SEED]

The "Tafel" menu theme's board (crates/rusterando-server/assets/tafel.jpg):
    python3 -I scripts/gen_blackboard.py crates/rusterando-server/assets/tafel.jpg 466 232 200 11
"""
import math
import sys

import numpy as np
from PIL import Image, ImageDraw, ImageFilter

out, w_mm, h_mm, dpi = sys.argv[1], float(sys.argv[2]), float(sys.argv[3]), int(sys.argv[4])
seed = int(sys.argv[5]) if len(sys.argv) > 5 else 7
rng = np.random.default_rng(seed)
W, H = round(w_mm / 25.4 * dpi), round(h_mm / 25.4 * dpi)
mm = dpi / 25.4  # pixels per mm


def noise(cells_y, smooth=Image.BICUBIC):
    """Value noise: random grid, scaled up smoothly, 0..1."""
    cells_x = max(2, round(cells_y * W / H))
    g = rng.random((cells_y + 1, cells_x + 1)).astype(np.float32)
    img = Image.fromarray((g * 255).astype(np.uint8), "L").resize((W, H), smooth)
    return np.asarray(img, np.float32) / 255.0


def fbm(base, octaves, falloff=0.55):
    total, amp, acc = 0.0, 1.0, np.zeros((H, W), np.float32)
    for o in range(octaves):
        acc += amp * noise(base * 2 ** o)
        total += amp
        amp *= falloff
    return acc / total


# 1. Slate: a dark green-grey, cloudy.
base = np.array([36, 50, 45], np.float32)
cloud = fbm(3, 6) - 0.5                   # -0.5..0.5, broad
grain = rng.normal(0, 1, (H, W)).astype(np.float32)
img = base[None, None, :] * (1 + 0.22 * cloud[..., None]) + 2.2 * grain[..., None]

# 2. Wiped chalk: soft swirls where a sponge went over old writing.
wipe = Image.new("L", (W, H), 0)
d = ImageDraw.Draw(wipe)
for _ in range(int(28 * (W * H) / (4000 * 2000))):
    cx, cy = rng.uniform(-0.1, 1.1) * W, rng.uniform(-0.1, 1.1) * H
    rx = rng.uniform(25, 90) * mm
    ry = rx * rng.uniform(0.35, 0.8)
    a0 = rng.uniform(0, 360)
    span = rng.uniform(70, 220)
    width = max(2, int(rng.uniform(6, 22) * mm))
    shade = int(rng.uniform(8, 26))
    d.arc([cx - rx, cy - ry, cx + rx, cy + ry], a0, a0 + span, fill=shade, width=width)
wipe = wipe.filter(ImageFilter.GaussianBlur(2.5 * mm))
wipe = np.asarray(wipe, np.float32) / 255.0
# The sponge leaves streaks: noise stretched along the wipe (mostly sideways).
# Short streaks (~8 mm long, ~0.5 mm high), not full-width lines.
fine = Image.fromarray((rng.random((max(2, H // 4), max(2, W // 60))) * 255).astype(np.uint8), "L").resize((W, H), Image.BICUBIC)
streaks = np.asarray(fine, np.float32) / 255.0
img += (wipe * (0.7 + 0.5 * streaks))[..., None] * np.array([150, 158, 152], np.float32)
# A light chalk film over everything, uneven.
haze = np.clip(fbm(5, 5) - 0.42, 0, 1) * 2.0
img += (haze * 22)[..., None]

# 3. Chalk dust: fine specks, a little denser towards the bottom (the tray).
dust = rng.random((H, W)).astype(np.float32)
bottom = np.linspace(0.0, 1.0, H, dtype=np.float32)[:, None] ** 3
specks = (dust > 0.993 - 0.003 * bottom).astype(np.float32)
specks = np.asarray(Image.fromarray((specks * 255).astype(np.uint8)).filter(ImageFilter.GaussianBlur(0.35 * mm)), np.float32) / 255.0
img += (specks * rng.uniform(40, 90))[..., None]

# 4. Darker towards the edges.
yy, xx = np.mgrid[0:H, 0:W].astype(np.float32)
r = np.sqrt(((xx / W) - 0.5) ** 2 * 1.6 + ((yy / H) - 0.5) ** 2 * 2.2)
img *= (1 - 0.28 * np.clip(r - 0.25, 0, 1))[..., None]

Image.fromarray(np.clip(img, 0, 255).astype(np.uint8), "RGB").save(out, quality=88, dpi=(dpi, dpi))
print(f"{out}: {W}×{H} px ({w_mm}×{h_mm} mm at {dpi} dpi)")
