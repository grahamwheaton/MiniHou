"""Generate test image fixtures and golden JSON output for image_file tests."""
import json
import os
import numpy as np

from minihou import images
from minihou.nodes.image_nodes import image_file

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
FIXTURES = os.path.join(ROOT, "rust", "crates", "minihou-core", "tests", "fixtures")
os.makedirs(FIXTURES, exist_ok=True)

# 1. 8-bit fixture (4x3)
h, w = 3, 4
pix8 = np.zeros((h, w, 4), dtype=np.float32)
pix8[..., 0] = np.linspace(0.1, 0.9, w, dtype=np.float32)[None, :]
pix8[..., 1] = np.linspace(0.2, 0.8, h, dtype=np.float32)[:, None]
pix8[..., 2] = 0.5
pix8[..., 3] = 0.75  # non-opaque

p8 = os.path.join(FIXTURES, "image_file_test.png")
images.write_png(p8, pix8, bits=8, srgb=True)

geo8 = image_file(file=p8, max_size=1024, colorspace="auto", size=10.0, center=[1.0, 2.0, 3.0], height="red", height_scale=2.0)

# 2. 16-bit fixture (5x4)
h16, w16 = 4, 5
pix16 = np.zeros((h16, w16, 4), dtype=np.float32)
pix16[..., 0] = np.linspace(0.0, 1.0, w16, dtype=np.float32)[None, :]
pix16[..., 1] = np.linspace(0.0, 1.0, h16, dtype=np.float32)[:, None]
pix16[..., 2] = 0.25
pix16[..., 3] = 1.0  # opaque

p16 = os.path.join(FIXTURES, "image_file_16bit.png")
images.write_png(p16, pix16, bits=16, srgb=False)

geo16 = image_file(file=p16, max_size=1024, colorspace="stored", size=5.0, center=[0.0, 0.0, 0.0], height="luminance", height_scale=1.5)

golden = {
    "geo8": {
        "npoints": geo8.npoints,
        "nprims": geo8.nprims,
        "vertex_point": geo8.topology.vertex_point.tolist(),
        "prim_offsets": geo8.topology.prim_offsets.tolist(),
        "P": geo8.P.tolist(),
        "Cd": geo8.point["Cd"].tolist(),
        "Alpha": geo8.point["Alpha"].tolist() if "Alpha" in geo8.point else None,
        "uv": geo8.point["uv"].tolist(),
        "height": geo8.point["height"].tolist() if "height" in geo8.point else None,
        "grid_columns": geo8.detail.get("grid_columns"),
        "grid_rows": geo8.detail.get("grid_rows"),
        "image": geo8.detail.get("image"),
    },
    "geo16": {
        "npoints": geo16.npoints,
        "nprims": geo16.nprims,
        "vertex_point": geo16.topology.vertex_point.tolist(),
        "prim_offsets": geo16.topology.prim_offsets.tolist(),
        "P": geo16.P.tolist(),
        "Cd": geo16.point["Cd"].tolist(),
        "Alpha": geo16.point["Alpha"].tolist() if "Alpha" in geo16.point else None,
        "uv": geo16.point["uv"].tolist(),
        "height": geo16.point["height"].tolist() if "height" in geo16.point else None,
        "grid_columns": geo16.detail.get("grid_columns"),
        "grid_rows": geo16.detail.get("grid_rows"),
        "image": geo16.detail.get("image"),
    },
}

with open(os.path.join(FIXTURES, "image_file_golden.json"), "w") as f:
    json.dump(golden, f, indent=2)

print("Generated fixtures in", FIXTURES)
