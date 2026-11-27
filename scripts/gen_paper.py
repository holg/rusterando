"""An original dark paper texture for the "Trattoria" menu theme (no
third-party image): warm near-black, soft cloudy unevenness, fine fibres
and grain — the ground of a dark poster cover.

    python3 -I scripts/gen_paper.py OUT.jpg WIDTH_MM HEIGHT_MM DPI [SEED]

The theme's paper (crates/rusterando-server/assets/trattoria.jpg):
    python3 -I scripts/gen_paper.py crates/rusterando-server/assets/trattoria.jpg 466 232 170 5
"""
import sys

import numpy as np
from PIL import Image, ImageFilter

out, w_mm, h_mm, dpi = sys.argv[1], float(sys.argv[2]), float(sys.argv[3]), int(sys.argv[4])
seed = int(sys.argv[5]) if len(sys.argv) > 5 else 5
rng = np.random.default_rng(seed)
W, H = round(w_mm / 25.4 * dpi), round(h_mm / 25.4 * dpi)
mm = dpi / 25.4


def noise(cells_y):
    """Value noise: a random grid scaled up smoothly, 0..1."""
    cells_x = max(2, round(cells_y * W / H))
    g = rng.random((cells_y + 1, cells_x + 1)).astype(np.float32)
    img = Image.fromarray((g * 255).astype(np.uint8), "L").resize((W, H), Image.BICUBIC)
    return np.asarray(img, np.float32) / 255.0


def fbm(base, octaves, falloff=0.55):
    total, amp, acc = 0.0, 1.0, np.zeros((H, W), np.float32)
    for o in range(octaves):
        acc += amp * noise(base * 2 ** o)
        total += amp
        amp *= falloff
    return acc / total


# 1. Paper: warm near-black, cloudy.
base = np.array([25, 23, 21], np.float32)
cloud = fbm(3, 5) - 0.5
img = base[None, None, :] + cloud[..., None] * np.array([9, 8, 7], np.float32)

# 2. Fibres: short faint strands in every direction.
fib = Image.new("L", (W, H), 0)
px = fib.load()
for _ in range(int(W * H / (mm * mm) * 0.35)):
    x, y = rng.uniform(0, W), rng.uniform(0, H)
    a = rng.uniform(0, np.pi)
    n = int(rng.uniform(1.5, 5) * mm)
    v = int(rng.uniform(20, 60))
    for t in range(n):
        xi, yi = int(x + np.cos(a) * t), int(y + np.sin(a) * t)
        if 0 <= xi < W and 0 <= yi < H:
            px[xi, yi] = max(px[xi, yi], v)
fib = np.asarray(fib.filter(ImageFilter.GaussianBlur(0.5)), np.float32) / 255.0
img += fib[..., None] * np.array([16, 15, 13], np.float32)

# 3. Grain.
img += rng.normal(0, 2.2, (H, W, 1)).astype(np.float32)

Image.fromarray(np.clip(img, 0, 255).astype(np.uint8), "RGB").save(out, quality=86, optimize=True)
print(f"wrote {out}: {W}×{H} px")
