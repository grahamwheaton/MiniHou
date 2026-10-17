"""Real headless Blender: image export payloads (generated, packed, external, oversized, empty). Set BLENDER_EXE."""
import os
from pathlib import Path
import struct
import subprocess
import sys
import tempfile
import unittest
import zlib

try:
    import bpy
except ImportError:
    bpy = None

ROOT = Path(__file__).resolve().parent.parent
sys.path[:0] = [str(ROOT), str(ROOT / "blender_addon")]


@unittest.skipIf(bpy is not None or not os.getenv("BLENDER_EXE"), "set BLENDER_EXE")
class LaunchBlender(unittest.TestCase):
    def test_image_export(self):
        proc = subprocess.run([os.environ["BLENDER_EXE"], "-b", "--factory-startup", "--python-exit-code", "1",
                               "--python", str(Path(__file__).resolve())], capture_output=True, text=True,
                              encoding="utf-8", errors="replace", timeout=180,
                              creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0))
        self.assertEqual(proc.returncode, 0, (proc.stdout + proc.stderr)[-6000:])
        self.assertIn("IMAGE_EXPORT_OK", proc.stdout)


def _decode(png):
    assert png.startswith(b"\x89PNG\r\n\x1a\n"), png[:8]
    pos, idat, size = 8, b"", None
    while pos < len(png):
        n, tag = struct.unpack(">I4s", png[pos:pos + 8])
        body = png[pos + 8:pos + 8 + n]
        if tag == b"IHDR":
            size = struct.unpack(">II", body[:8])
        elif tag == b"IDAT":
            idat += body
        pos += 12 + n
    w, h = size
    raw = zlib.decompress(idat)
    stride = w * 4 + 1
    assert len(raw) == h * stride and all(raw[y * stride] == 0 for y in range(h)), "unexpected PNG filters"
    return w, h, [tuple(raw[y * stride + 1 + x * 4:y * stride + 5 + x * 4]) for y in range(h) for x in range(w)]


def run_in_blender():
    from minihou_link import material_export as me
    colors = [(255, 0, 0, 255), (0, 255, 0, 255), (0, 0, 255, 255), (255, 255, 0, 255)]  # top-left, top-right, bottom-left, bottom-right
    tmp = Path(tempfile.mkdtemp(prefix="minihou-img-"))
    path = tmp / "ext.png"
    path.write_bytes(me._png(2, 2, bytes(v for c in colors for v in c)))

    def mat_for(image):
        mat = bpy.data.materials.new("m")
        mat.use_nodes = True
        nodes = mat.node_tree.nodes
        shader = next(n for n in nodes if n.type == "BSDF_PRINCIPLED")
        tex = nodes.new("ShaderNodeTexImage")
        tex.image = image
        mat.node_tree.links.new(tex.outputs["Color"], shader.inputs["Base Color"])
        return mat

    def export(image):
        manifest, payloads = me.export_materials([mat_for(image)], "obj")
        return manifest, payloads

    # Generated 2x2 (distinct colours, bottom row first in Blender's pixel order) -> exact pixels, non-empty.
    gen = bpy.data.images.new("gen", 2, 2, alpha=True)
    gen.pixels[:] = [0, 0, 1, 1, 1, 1, 0, 1, 1, 0, 0, 1, 0, 1, 0, 1]  # bottom row: blue, yellow; top row: red, green
    manifest, payloads = export(gen)
    assert not manifest["diagnostics"], manifest["diagnostics"]
    (payload,) = payloads.values()
    assert _decode(payload) == (2, 2, colors), _decode(payload)
    # Packed and external PNGs.
    ext = bpy.data.images.load(str(path), check_existing=False)
    packed = bpy.data.images.load(str(path), check_existing=False)
    packed.pack()
    for image in (ext, packed):
        manifest, payloads = export(image)
        assert not manifest["diagnostics"], manifest["diagnostics"]
        (payload,) = payloads.values()
        assert _decode(payload) == (2, 2, colors)
    # Over the pixel limit: downscaled under it instead of dropped, source untouched.
    big = bpy.data.images.new("big", 40, 40, alpha=True)
    big.pixels[:] = [1, 0, 0, 1] * 1600
    old = me.MAX_PIXELS
    me.MAX_PIXELS = 100
    try:
        manifest, payloads = export(big)
        assert not manifest["diagnostics"], manifest["diagnostics"]
        (payload,) = payloads.values()
        w, h, px = _decode(payload)
        assert 0 < w * h <= 100 and px[0][:3] == (255, 0, 0), (w, h, px[0])
    finally:
        me.MAX_PIXELS = old
    assert tuple(big.size) == (40, 40) and not any(i.name.startswith("big.") for i in bpy.data.images)
    # Genuinely empty / missing image: clear message and a missing-asset placeholder, nothing sent.
    gone = bpy.data.images.load(str(path), check_existing=False)
    gone.filepath = str(tmp / "does_not_exist.png")
    gone.reload()
    manifest, payloads = export(gone)
    assert not payloads and "empty or could not be loaded" in manifest["diagnostics"]["blender:obj:0"], manifest["diagnostics"]
    print("IMAGE_EXPORT_OK")


if bpy is not None:
    run_in_blender()
