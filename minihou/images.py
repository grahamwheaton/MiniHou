"""Images as geometry, and back (no Blender, no GUI): pixel grids, rasterising, colour spaces and PNG files.

An image in a graph is a **pixel grid**: a ``grid`` of ``columns x rows`` points (one per pixel, row by row from the bottom left, as
Blender stores images), ``Cd`` / ``Alpha`` point attributes holding the colour, ``uv`` and the detail ``grid_columns`` /
``grid_rows`` that the heightfield nodes already understand. That makes every geometry node an image node: wrangles can read and
write pixels, heightfield nodes erode a height image, ``attribute_blur`` blurs it, and ``image_out`` turns any grid (or any points,
baked by position) back into an image in Blender or in a PNG file.
"""
from __future__ import annotations

import struct
import zlib
from typing import Optional, Tuple

import numpy as np

from .geometry import Geometry

MAX_PIXELS = 16_000_000                # a safety limit for images in a graph (a 4096 x 4096 image)


# -- colour spaces -------------------------------------------------------------------------------------------------------
def srgb_to_linear(a: np.ndarray) -> np.ndarray:
    a = np.asarray(a, np.float32)
    return np.where(a <= 0.04045, a / 12.92, ((a + 0.055) / 1.055) ** 2.4).astype(np.float32)


def linear_to_srgb(a: np.ndarray) -> np.ndarray:
    a = np.clip(np.asarray(a, np.float32), 0.0, None)
    return np.where(a <= 0.0031308, a * 12.92, 1.055 * np.power(a, 1 / 2.4) - 0.055).astype(np.float32)


# -- pixels <-> geometry -------------------------------------------------------------------------------------------------
def downsample(pix: np.ndarray, max_size: int) -> np.ndarray:
    """Average ``pix`` (H, W, C) in blocks so that neither side is larger than ``max_size`` (never enlarges)."""
    h, w = pix.shape[:2]
    f = int(np.ceil(max(h, w) / max(int(max_size), 1)))
    if f <= 1:
        return pix
    h2, w2 = h // f * f, w // f * f
    crop = pix[:h2, :w2].astype(np.float32)
    return crop.reshape(h2 // f, f, w2 // f, f, pix.shape[2]).mean(axis=(1, 3))


def luminance(rgb: np.ndarray) -> np.ndarray:
    return (rgb[..., 0] * 0.2126 + rgb[..., 1] * 0.7152 + rgb[..., 2] * 0.0722).astype(np.float32)


def grid_from_pixels(pix: np.ndarray, size: float = 1.0, center=(0.0, 0.0, 0.0), height: str = "none", height_scale: float = 1.0,
                     name: str = "") -> Geometry:
    """A pixel grid for an image (H, W, 4 floats): ``size`` is the width in world units (the height follows the aspect ratio).
    ``height`` (luminance, red, green, blue or alpha) lifts the points: ``z = value * height_scale``, also stored as ``height``."""
    h, w = pix.shape[:2]
    if w < 2 or h < 2:
        raise ValueError("an image needs at least 2 x 2 pixels to make a grid")
    if w * h > MAX_PIXELS:
        raise ValueError(f"{w} x {h} pixels is more than the limit ({MAX_PIXELS:,}); lower max_size")
    sx = float(size)
    sy = sx * h / w
    u = np.linspace(0.0, 1.0, w, dtype=np.float32)
    v = np.linspace(0.0, 1.0, h, dtype=np.float32)
    uu, vv = np.meshgrid(u, v)
    P = np.empty((h * w, 3), np.float32)
    P[:, 0] = ((uu - 0.5) * sx + center[0]).ravel()
    P[:, 1] = ((vv - 0.5) * sy + center[1]).ravel()
    P[:, 2] = center[2]
    ii, jj = np.meshgrid(np.arange(w - 1, dtype=np.int32), np.arange(h - 1, dtype=np.int32))
    base = (jj * w + ii).ravel()
    vertex_point = np.stack([base, base + 1, base + w + 1, base + w], axis=1).ravel()
    g = Geometry.from_arrays(P, vertex_point, np.arange(base.shape[0] + 1, dtype=np.int32) * 4)
    flat = pix.reshape(h * w, -1)
    g.point["Cd"] = np.ascontiguousarray(flat[:, :3], np.float32)
    if flat.shape[1] >= 4 and not np.allclose(flat[:, 3], 1.0):
        g.point["Alpha"] = np.ascontiguousarray(flat[:, 3], np.float32)
    g.point["uv"] = np.stack([uu.ravel(), vv.ravel()], axis=1)
    if height != "none":
        chan = {"red": 0, "green": 1, "blue": 2, "alpha": 3}
        if height == "luminance":
            hv = luminance(flat[:, :3])
        elif height in chan:
            hv = flat[:, min(chan[height], flat.shape[1] - 1)] if chan[height] < flat.shape[1] else np.ones(h * w, np.float32)
        else:
            raise ValueError(f"unknown height channel {height!r}")
        g.point["height"] = np.ascontiguousarray(hv, np.float32)
        g.point["P"] = np.concatenate([P[:, :2], (P[:, 2] + hv * float(height_scale))[:, None]], axis=1).astype(np.float32)
    g.detail["grid_columns"] = int(w)
    g.detail["grid_rows"] = int(h)
    if name:
        g.detail["image"] = str(name)
    return g.freeze()


def lattice(geo: Geometry) -> Tuple[int, int]:
    """``(columns, rows)`` when the points are a lattice row by row (a grid, an image, a heightfield), else a clear error."""
    c, r = geo.detail.get("grid_columns"), geo.detail.get("grid_rows")
    if not isinstance(c, int) or not isinstance(r, int) or c < 1 or r < 1 or c * r != geo.npoints:
        raise ValueError("this geometry is not a pixel grid (no grid_columns / grid_rows for its points): use mode 'bake' to raster "
                         "points by their position, or start from a grid or an image")
    return int(c), int(r)


def channels(values: np.ndarray, normalize: bool = False) -> np.ndarray:
    """An attribute (n,) or (n, k) as RGBA floats (n, 4): a scalar is grey, k = 2 is (r, g, 0), 3 is rgb, 4 is rgba."""
    v = np.asarray(values, np.float32)
    if v.ndim == 1:
        v = v[:, None]
    n, k = v.shape
    if normalize:
        lo, hi = float(v.min()), float(v.max())
        v = (v - lo) / (hi - lo) if hi > lo else np.zeros_like(v)
    out = np.ones((n, 4), np.float32)
    if k == 1:
        out[:, :3] = v
    elif k == 2:
        out[:, :2] = v
        out[:, 2] = 0.0
    elif k == 3:
        out[:, :3] = v
    else:
        out[:, :4] = v[:, :4]
    return out


def pixels_from_lattice(geo: Geometry, values: np.ndarray, normalize: bool = False) -> np.ndarray:
    c, r = lattice(geo)
    return channels(values, normalize).reshape(r, c, 4)


def bake(geo: Geometry, values: np.ndarray, width: int, height: int, normalize: bool = False) -> np.ndarray:
    """Raster points into an image by their X / Y position (the bounding box fills the image); pixels with several points take their
    mean, pixels with none are transparent black. The alpha channel is therefore coverage unless the attribute has its own."""
    if width < 1 or height < 1 or width * height > MAX_PIXELS:
        raise ValueError(f"image size {width} x {height} is out of range (1 to {MAX_PIXELS:,} pixels)")
    P = np.asarray(geo.P, np.float64)
    if not len(P):
        raise ValueError("there are no points to bake")
    lo, hi = P[:, :2].min(0), P[:, :2].max(0)
    span = np.where(hi > lo, hi - lo, 1.0)
    ix = np.clip(((P[:, 0] - lo[0]) / span[0] * width).astype(np.int64), 0, width - 1)
    iy = np.clip(((P[:, 1] - lo[1]) / span[1] * height).astype(np.int64), 0, height - 1)
    cell = iy * width + ix
    ch = channels(values, normalize).astype(np.float64)
    out = np.zeros((height * width, 4), np.float64)
    count = np.bincount(cell, minlength=height * width).astype(np.float64)
    for c in range(4):
        out[:, c] = np.bincount(cell, weights=ch[:, c], minlength=height * width)
    covered = count > 0
    out[covered] /= count[covered, None]
    out[~covered] = 0.0
    return out.astype(np.float32).reshape(height, width, 4)


# -- PNG -----------------------------------------------------------------------------------------------------------------
_SIG = b"\x89PNG\r\n\x1a\n"


def _chunk(kind: bytes, data: bytes) -> bytes:
    return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data) & 0xFFFFFFFF)


def png_encode(pix: np.ndarray, bits: int = 8, srgb: bool = True) -> bytes:
    """PNG bytes for an image (H, W, 4 floats in 0..1; rows from the bottom as Blender stores them, so they are flipped). 8 or 16 bits;
    ``srgb`` encodes linear colour with the sRGB curve first (alpha stays linear)."""
    pix = np.asarray(pix, np.float32)
    h, w, c = pix.shape
    rgb = pix[..., :3]
    if srgb:
        rgb = linear_to_srgb(rgb)
    data = np.concatenate([np.clip(rgb, 0, 1), np.clip(pix[..., 3:4], 0, 1)], axis=-1) if c >= 4 else np.clip(rgb, 0, 1)
    data = data[::-1]                                                  # PNG rows run from the top
    if bits == 16:
        raw = np.rint(data * 65535).astype(">u2")
    else:
        raw = np.rint(data * 255).astype(np.uint8)
    channels_ = data.shape[2]
    color_type = {3: 2, 4: 6}[channels_]
    rows = raw.reshape(h, -1)
    body = b"".join(b"\x00" + rows[i].tobytes() for i in range(h))
    ihdr = struct.pack(">IIBBBBB", w, h, 16 if bits == 16 else 8, color_type, 0, 0, 0)
    return _SIG + _chunk(b"IHDR", ihdr) + _chunk(b"IDAT", zlib.compress(body, 6)) + _chunk(b"IEND", b"")


def _unfilter(raw: bytes, w: int, h: int, bpp: int) -> np.ndarray:
    stride = w * bpp
    out = np.zeros((h, stride), np.uint8)
    pos = 0
    prev = np.zeros(stride, np.int32)
    for y in range(h):
        ft = raw[pos]
        line = np.frombuffer(raw, np.uint8, stride, pos + 1).astype(np.int32)
        pos += stride + 1
        if ft == 0:
            cur = line
        elif ft == 1:                                                  # Sub
            cur = (np.cumsum(line.reshape(w, bpp), axis=0) & 255).reshape(-1)
        elif ft == 2:                                                  # Up
            cur = (line + prev) & 255
        elif ft in (3, 4):                                             # Average, Paeth: each pixel needs the one before it
            cur = np.zeros(stride, np.int32)
            for x in range(stride):
                a = cur[x - bpp] if x >= bpp else 0
                b = prev[x]
                if ft == 3:
                    cur[x] = (line[x] + ((a + b) >> 1)) & 255
                else:
                    c = prev[x - bpp] if x >= bpp else 0
                    p = a + b - c
                    pa, pb, pc = abs(p - a), abs(p - b), abs(p - c)
                    pred = a if (pa <= pb and pa <= pc) else (b if pb <= pc else c)
                    cur[x] = (line[x] + pred) & 255
        else:
            raise ValueError(f"unknown PNG filter {ft}")
        out[y] = cur
        prev = cur
    return out


def png_decode(data: bytes, srgb: bool = True, use_pillow: bool = True) -> np.ndarray:
    """Pixels (H, W, 4 floats, rows from the bottom) from PNG bytes: grey, grey + alpha, RGB, RGBA, palette; 8 or 16 bits; not interlaced.
    ``srgb`` linearises the colour. Uses Pillow when it is installed (any format it reads), else this reader."""
    if data[:8] == _SIG and len(data) > 24 and data[24] == 16:       # Pillow would round 16-bit RGBA down to 8 bits; keep every bit
        use_pillow = False
    try:
        if not use_pillow:
            raise ImportError
        from PIL import Image  # type: ignore
        import io
        im = Image.open(io.BytesIO(data))
        if im.mode in ("I;16", "I;16B", "I"):
            arr = np.asarray(im, np.float32)[..., None] / 65535.0
            arr = np.repeat(arr, 3, axis=2)
            rgba = np.concatenate([arr, np.ones_like(arr[..., :1])], axis=2)
        else:
            rgba = np.asarray(im.convert("RGBA"), np.float32) / 255.0
        rgba = np.ascontiguousarray(rgba[::-1])
        if srgb:
            rgba[..., :3] = srgb_to_linear(rgba[..., :3])
        return rgba
    except ImportError:
        pass
    if data[:8] != _SIG:
        raise ValueError("not a PNG file (and Pillow is not installed to read other formats)")
    pos, idat, plte, ihdr = 8, [], None, None
    while pos + 8 <= len(data):
        n, kind = struct.unpack(">I4s", data[pos:pos + 8])
        body = data[pos + 8:pos + 8 + n]
        pos += 12 + n
        if kind == b"IHDR":
            ihdr = struct.unpack(">IIBBBBB", body)
        elif kind == b"PLTE":
            plte = np.frombuffer(body, np.uint8).reshape(-1, 3)
        elif kind == b"IDAT":
            idat.append(body)
        elif kind == b"IEND":
            break
    if ihdr is None or not idat:
        raise ValueError("damaged PNG file")
    w, h, depth, ctype, _, _, interlace = ihdr
    if interlace:
        raise ValueError("interlaced PNG files are not supported (install Pillow to read them)")
    if depth not in (8, 16):
        raise ValueError(f"{depth}-bit PNG files are not supported (install Pillow to read them)")
    ch = {0: 1, 2: 3, 3: 1, 4: 2, 6: 4}.get(ctype)
    if ch is None:
        raise ValueError(f"PNG colour type {ctype} is not supported")
    bpp = ch * (depth // 8)
    raw = zlib.decompress(b"".join(idat))
    pix = _unfilter(raw, w, h, bpp)
    if depth == 16:
        vals = pix.reshape(h, w, ch, 2)
        vals = (vals[..., 0].astype(np.float32) * 256 + vals[..., 1]) / 65535.0
    else:
        vals = pix.reshape(h, w, ch).astype(np.float32)
        if ctype == 3:
            if plte is None:
                raise ValueError("palette PNG without a palette")
            vals = plte[vals[..., 0].astype(np.int64)].astype(np.float32) / 255.0
        else:
            vals = vals / 255.0
    if vals.shape[2] == 1:
        rgba = np.concatenate([np.repeat(vals, 3, 2), np.ones((h, w, 1), np.float32)], 2)
    elif vals.shape[2] == 2:
        rgba = np.concatenate([np.repeat(vals[..., :1], 3, 2), vals[..., 1:2]], 2)
    elif vals.shape[2] == 3:
        rgba = np.concatenate([vals, np.ones((h, w, 1), np.float32)], 2)
    else:
        rgba = vals
    rgba = np.ascontiguousarray(rgba[::-1], np.float32)
    if srgb:
        rgba[..., :3] = srgb_to_linear(rgba[..., :3])
    return rgba


def write_png(path, pix: np.ndarray, bits: int = 8, srgb: bool = True) -> None:
    import os
    os.makedirs(os.path.dirname(os.path.abspath(path)), exist_ok=True)
    tmp = str(path) + ".part"
    with open(tmp, "wb") as f:
        f.write(png_encode(pix, bits, srgb))
    os.replace(tmp, path)


def read_png(path, srgb: bool = True, use_pillow: bool = True) -> np.ndarray:
    with open(path, "rb") as f:
        return png_decode(f.read(), srgb, use_pillow)
