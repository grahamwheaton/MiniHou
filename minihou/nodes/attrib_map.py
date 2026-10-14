"""Attribute from Map geometry node: samples texture images or color maps onto geometry
using UV coordinates (Houdini's Attribute from Map SOP).
"""
from __future__ import annotations

import math
import os
from typing import Dict, Optional, Tuple

import numpy as np
try:
    from PIL import Image
except ImportError:
    Image = None

from .. import groups
from ..sdk import Bool, Enum, Float, Input, Str, Vec3, node

_image_cache: Dict[Tuple[str, float], np.ndarray] = {}


def _load_image(path: str) -> Optional[np.ndarray]:
    """Load an image file and return float32 array in [0, 1] of shape (H, W, C)."""
    if not path or not os.path.isfile(path):
        return None
    if Image is None:
        raise RuntimeError("Attribute from Map requires Pillow to load image files: pip install pillow")
    try:
        mtime = os.path.getmtime(path)
        key = (path, mtime)
        if key in _image_cache:
            return _image_cache[key]
        with Image.open(path) as img:
            img = img.convert("RGBA")
            data = np.asarray(img, dtype=np.float32) / 255.0
            _image_cache[key] = data
            return data
    except Exception:
        return None


def _sample_texture(img: np.ndarray, u: np.ndarray, v: np.ndarray,
                    wrap: str, border_color: Tuple[float, float, float],
                    filter_mode: str) -> Tuple[np.ndarray, np.ndarray]:
    """Sample texture at UV coordinates (u, v) and return (rgba, valid_mask)."""
    H, W, C = img.shape
    u = u.astype(np.float64, copy=True)
    v = v.astype(np.float64, copy=True)

    valid_mask = np.ones(len(u), dtype=bool)

    # 1. Apply UV Wrapping
    if wrap == "repeat":
        u = np.mod(u, 1.0)
        v = np.mod(v, 1.0)
    elif wrap == "mirror":
        u = 1.0 - np.abs(np.mod(u, 2.0) - 1.0)
        v = 1.0 - np.abs(np.mod(v, 2.0) - 1.0)
    elif wrap == "clamp":
        u = np.clip(u, 0.0, 1.0)
        v = np.clip(v, 0.0, 1.0)
    elif wrap == "border":
        outside = (u < 0.0) | (u > 1.0) | (v < 0.0) | (v > 1.0)
        valid_mask = ~outside
        u = np.clip(u, 0.0, 1.0)
        v = np.clip(v, 0.0, 1.0)

    # Invert V for top-left image origin
    img_x = u * (W - 1)
    img_y = (1.0 - v) * (H - 1)

    if filter_mode == "nearest":
        ix = np.clip(np.round(img_x).astype(np.int64), 0, W - 1)
        iy = np.clip(np.round(img_y).astype(np.int64), 0, H - 1)
        samples = img[iy, ix]
    else:  # bilinear (default)
        x0 = np.clip(np.floor(img_x).astype(np.int64), 0, W - 1)
        x1 = np.clip(x0 + 1, 0, W - 1)
        y0 = np.clip(np.floor(img_y).astype(np.int64), 0, H - 1)
        y1 = np.clip(y0 + 1, 0, H - 1)

        fx = (img_x - x0)[:, None].astype(np.float32)
        fy = (img_y - y0)[:, None].astype(np.float32)

        c00 = img[y0, x0]
        c10 = img[y0, x1]
        c01 = img[y1, x0]
        c11 = img[y1, x1]

        top = c00 * (1.0 - fx) + c10 * fx
        bot = c01 * (1.0 - fx) + c11 * fx
        samples = top * (1.0 - fy) + bot * fy

    # Apply border color if outside
    if wrap == "border" and np.any(~valid_mask):
        border_rgba = np.array([border_color[0], border_color[1], border_color[2], 1.0], dtype=np.float32)
        samples[~valid_mask] = border_rgba

    return samples, valid_mask


@node("attribute_from_map", inputs=["geo", Input("texture_geo", optional=True, doc="Optional 2D volume or texture input")], category="attribute", version=1, params=[
    Str("group", "", doc="Point group to limit the sampling to"),
    Enum("use_ful", ["file", "volume"], default="file", doc="Map source: disk file or input geometry volume"),
    Bool("udim", False, doc="UDIM filename expansion"),
    Str("texture_channel", "", doc="Image file path to sample"),
    Str("volume_name", "", doc="Volume primitive name if sampling from second input"),
    Str("uvattrib", "uv", doc="UV attribute on the geometry (point or vertex)"),
    Str("export_attribute", "Cd", doc="Attribute name to write the sampled values to"),
    Enum("srccolorspace", ["srgb", "linear"], default="srgb", doc="Input color space"),
    Enum("color_channel", ["rgb", "r", "g", "b", "luminance", "alpha"], default="rgb", doc="Color channels to export"),
    Float("rgb_color_influence1", 1.0, doc="Sample influence / blend weight"),
    Bool("visualize_map", False, doc="Viewport preview visualization"),
    Bool("doremap", False, doc="Apply remap curve"),
    Float("rgb_scale", 1.0, doc="Color intensity scaling multiplier"),
    Float("rgb_contrast", 1.0, doc="Contrast adjustment"),
    Float("rgb_contrast_rolloff", 0.0, doc="Contrast rolloff"),
    Float("remap_influence", 1.0, doc="Remap blend weight"),
    Enum("filter", ["bilinear", "nearest", "bicubic"], default="bilinear", doc="Texture filtering mode"),
    Float("filter_width", 1.0, doc="Filter width / blur"),
    Enum("wrap", ["repeat", "clamp", "mirror", "border"], default="repeat", doc="Out-of-range UV wrap mode"),
    Vec3("borderrr", (0.0, 0.0, 0.0), doc="Border color when wrap mode is border"),
    Enum("vertexmethod", ["average", "first", "last"], default="average", doc="Vertex to point UV promotion method"),
    Bool("uv_invertu", False, doc="Invert U coordinate"),
    Bool("uv_invertv", False, doc="Invert V coordinate"),
    Vec3("uv_scale", (1.0, 1.0, 0.0), doc="UV coordinate scaling"),
    Float("uv_rz", 0.0, doc="UV rotation in degrees"),
    Vec3("uv_tx", (0.0, 0.0, 0.0), doc="UV translation offset"),
])
def attribute_from_map(geo, texture_geo=None, group="", use_ful="file", udim=False,
                       texture_channel="", volume_name="", uvattrib="uv", export_attribute="Cd",
                       srccolorspace="srgb", color_channel="rgb", rgb_color_influence1=1.0,
                       visualize_map=False, doremap=False, rgb_scale=1.0, rgb_contrast=1.0,
                       rgb_contrast_rolloff=0.0, remap_influence=1.0, filter="bilinear",
                       filter_width=1.0, wrap="repeat", borderrr=(0.0, 0.0, 0.0),
                       vertexmethod="average", uv_invertu=False, uv_invertv=False,
                       uv_scale=(1.0, 1.0, 0.0), uv_rz=0.0, uv_tx=(0.0, 0.0, 0.0), **kwargs):
    """Sample texture map colors onto attributes using UVs (Houdini's Attribute from Map SOP)."""
    if geo.npoints == 0:
        return geo

    out = geo.copy()
    attrib_name = export_attribute.strip() or "Cd"

    # 1. Obtain Image Data
    img_data: Optional[np.ndarray] = None
    if use_ful == "volume" and texture_geo is not None:
        # Check if texture_geo has an image attribute or is an image/volume node
        for tbl in ("point", "prim"):
            for col in texture_geo.table(tbl).names():
                if "Cd" in col or "image" in col or "color" in col:
                    cand = texture_geo.table(tbl)[col]
                    if cand.ndim >= 2:
                        img_data = np.asarray(cand, dtype=np.float32)
                        break
    if img_data is None:
        img_data = _load_image(texture_channel.strip())

    if img_data is None:
        # Default placeholder: 2x2 white-gray checkerboard
        img_data = np.array([
            [[1.0, 1.0, 1.0, 1.0], [0.5, 0.5, 0.5, 1.0]],
            [[0.5, 0.5, 0.5, 1.0], [1.0, 1.0, 1.0, 1.0]],
        ], dtype=np.float32)

    # 2. Extract UV Coordinates
    u = None
    v = None
    if uvattrib in out.point:
        uv_raw = out.point[uvattrib]
        u = uv_raw[:, 0]
        v = uv_raw[:, 1]
    elif uvattrib in out.vertex:
        # Promote vertex UV to point
        v_uv = out.vertex[uvattrib]
        t = out.topology
        vp = t.vertex_point
        # Average or first
        count = np.bincount(vp, minlength=out.npoints)
        count = np.maximum(count, 1)
        sum_u = np.bincount(vp, weights=v_uv[:, 0], minlength=out.npoints)
        sum_v = np.bincount(vp, weights=v_uv[:, 1], minlength=out.npoints)
        u = (sum_u / count).astype(np.float32)
        v = (sum_v / count).astype(np.float32)
    else:
        # Generate planar projection UVs from P (XY bounding box)
        P = out.P
        min_p = P.min(axis=0)
        max_p = P.max(axis=0)
        span = np.maximum(max_p - min_p, 1e-6)
        u = ((P[:, 0] - min_p[0]) / span[0]).astype(np.float32)
        v = ((P[:, 1] - min_p[1]) / span[1]).astype(np.float32)

    u = u.copy()
    v = v.copy()

    # 3. Transform UVs
    if uv_invertu:
        u = 1.0 - u
    if uv_invertv:
        v = 1.0 - v

    # Translation
    u -= uv_tx[0]
    v -= uv_tx[1]

    # Rotation (around [0.5, 0.5])
    if abs(uv_rz) > 1e-6:
        rad = math.radians(uv_rz)
        cos_a = math.cos(rad)
        sin_a = math.sin(rad)
        uc = u - 0.5
        vc = v - 0.5
        u = 0.5 + uc * cos_a - vc * sin_a
        v = 0.5 + uc * sin_a + vc * cos_a

    # Scale
    sx = uv_scale[0] if abs(uv_scale[0]) > 1e-12 else 1.0
    sy = uv_scale[1] if abs(uv_scale[1]) > 1e-12 else 1.0
    u *= sx
    v *= sy

    # 4. Sample Texture
    samples, valid = _sample_texture(img_data, u, v, wrap=wrap, border_color=borderrr, filter_mode=filter)

    # sRGB to linear conversion if requested
    if srccolorspace == "srgb":
        # Approximate standard sRGB gamma 2.2
        samples[:, :3] = np.power(np.maximum(samples[:, :3], 0.0), 2.2)

    # Contrast and scale adjustments
    if abs(rgb_contrast - 1.0) > 1e-5:
        samples[:, :3] = np.clip((samples[:, :3] - 0.5) * rgb_contrast + 0.5, 0.0, 1.0)
    samples[:, :3] *= float(rgb_scale) * float(rgb_color_influence1)

    # 5. Extract Channels
    if color_channel == "rgb":
        result = samples[:, :3]
    elif color_channel == "r":
        result = samples[:, 0]
    elif color_channel == "g":
        result = samples[:, 1]
    elif color_channel == "b":
        result = samples[:, 2]
    elif color_channel == "alpha":
        result = samples[:, 3] if samples.shape[1] > 3 else np.ones(len(samples), dtype=np.float32)
    elif color_channel == "luminance":
        # Rec. 709 luminance
        result = 0.2126 * samples[:, 0] + 0.7152 * samples[:, 1] + 0.0722 * samples[:, 2]
    else:
        result = samples[:, :3]

    result = result.astype(np.float32)

    # 6. Group restriction
    if group:
        mask = groups.mask(geo, "point", group)
        if mask is not None:
            if attrib_name in out.point:
                curr = out.point[attrib_name]
                final = curr.copy()
                final[mask] = result[mask]
                result = final
            else:
                final = np.zeros_like(result)
                final[mask] = result[mask]
                result = final

    out.point[attrib_name] = result
    return out
