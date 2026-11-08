"""Regenerates rust/crates/minihou-app/assets/app_icon.png and app_icon.ico: the blue rounded square with the white M
that the app draws in its title bar (ui/topbar.rs). Run: python tools/make_app_icon.py  (needs Pillow)."""
from pathlib import Path
from PIL import Image, ImageDraw

BLUE = (79, 156, 240, 255)          # the app's accent colour, as in the title bar logo
ASSETS = Path(__file__).resolve().parents[1] / "rust/crates/minihou-app/assets"
SUPER = 2048                         # drawn large, then scaled down for smooth edges

def stroke(points, width):
    """Polygon for a polyline stroke with flat ends and mitred joins."""
    import math
    n = len(points)
    def unit(a, b):
        dx, dy = b[0] - a[0], b[1] - a[1]; l = math.hypot(dx, dy); return dx / l, dy / l
    left, right = [], []
    for i, p in enumerate(points):
        d0 = unit(points[i - 1], p) if i > 0 else None
        d1 = unit(p, points[i + 1]) if i < n - 1 else None
        d0 = d0 or d1; d1 = d1 or d0
        n0, n1 = (-d0[1], d0[0]), (-d1[1], d1[0])
        mx, my = n0[0] + n1[0], n0[1] + n1[1]
        k = (n0[0] * mx + n0[1] * my)                # miter length factor
        off = (mx / k * width / 2, my / k * width / 2)
        left.append((p[0] + off[0], p[1] + off[1])); right.append((p[0] - off[0], p[1] - off[1]))
    return left + right[::-1]

def render():
    img = Image.new("RGBA", (SUPER, SUPER), (0, 0, 0, 0))
    d = ImageDraw.Draw(img)
    unit = SUPER / 20.0                               # the title bar logo is laid out on a 20 x 20 grid
    d.rounded_rectangle((1 * unit, 1 * unit, 19 * unit, 19 * unit), radius=4 * unit, fill=BLUE)
    m = [(5, 14), (5, 6), (10, 11), (15, 6), (15, 14)]
    d.polygon(stroke([(x * unit, y * unit) for x, y in m], 2.4 * unit), fill=(255, 255, 255, 255))
    return img

if __name__ == "__main__":
    big = render()
    png = big.resize((256, 256), Image.LANCZOS)
    png.save(ASSETS / "app_icon.png")
    sizes = [16, 20, 24, 32, 40, 48, 64, 128, 256]
    frames = [big.resize((s, s), Image.LANCZOS) for s in sizes]
    frames[-1].save(ASSETS / "app_icon.ico", format="ICO", sizes=[(s, s) for s in sizes], append_images=frames[:-1])
    print("wrote", ASSETS / "app_icon.png", "and", ASSETS / "app_icon.ico")
