"""Blender images <-> the engine (``blender_image`` reads one, ``image_out`` makes or updates one). Main thread only.

* Reading sends the pixels as they are stored: a float32 array (height, width, 4), rows from the bottom as Blender keeps them. The engine
  decides what to do with the colour space (8-bit sRGB images become linear for ``Cd``).
* Writing takes the pixels the engine computed. ``encoding`` says what the image is: ``linear`` a float image in the scene's linear
  space, ``srgb`` an 8-bit sRGB image (the engine has already applied the sRGB curve), ``data`` a float Non-Color image.
* The image Blender shows is a normal data-block: it keeps its users (materials, the Image Editor) as the data changes. Save or pack it
  to keep it; the engine can also write a PNG next to it.
"""
from __future__ import annotations

import hashlib
from typing import Dict, Tuple

import numpy as np

import bpy

from . import scene_plan as sp
from .mesh_writer import assert_main_thread

MAX_PIXELS = 16_000_000
DIGEST = "minihou_digest"


def read_image(img) -> Tuple[dict, Dict[str, np.ndarray]]:
    """``(header fields, arrays)`` for an ``image_data`` message. Raises ValueError if the image cannot be read."""
    assert_main_thread()
    if img.get(sp.MARK):
        raise ValueError("this image is made by a MiniHou image_out node; reading it back would feed the result into itself")
    w, h = int(img.size[0]), int(img.size[1])
    if w == 0 or h == 0:
        raise ValueError("the image has no pixels (an empty or unloaded image)")
    if w * h > MAX_PIXELS:
        raise ValueError(f"{w} x {h} pixels is more than the limit ({MAX_PIXELS:,})")
    channels = int(getattr(img, "channels", 4) or 4)
    raw = np.empty(w * h * channels, np.float32)
    img.pixels.foreach_get(raw)
    pix = raw.reshape(h, w, channels)
    if channels != 4:                                    # grey or RGB images: fill out to RGBA
        rgba = np.ones((h, w, 4), np.float32)
        rgba[..., :min(channels, 3)] = pix[..., :min(channels, 3)] if channels >= 3 else pix[..., :1]
        pix = rgba
    return ({"width": w, "height": h, "is_float": bool(img.is_float), "colorspace": str(img.colorspace_settings.name),
             "source": str(img.source)}, {"pixels": np.ascontiguousarray(pix)})


def apply_image(manifest: dict, arrays: dict, tag: str) -> dict:
    """Make or update the image a ``scene_objects`` manifest of mode ``image`` describes. Returns the usual counts."""
    assert_main_thread()
    if not arrays or "pixels" not in arrays:
        raise ValueError("an image manifest arrived without its pixels")
    name = str(manifest["image"])
    w, h = int(manifest["width"]), int(manifest["height"])
    pix = np.asarray(arrays["pixels"], np.float32).reshape(h, w, 4)
    encoding = str(manifest.get("encoding") or "linear")
    digest = hashlib.blake2b(pix.tobytes() + f"{w}x{h}{encoding}".encode(), digest_size=12).hexdigest()
    img = bpy.data.images.get(name)
    created = 0
    if img is None:
        img = bpy.data.images.new(name, w, h, alpha=True, float_buffer=(encoding != "srgb"), is_data=(encoding == "data"))
        created = 1
    elif img.get(DIGEST) == digest and (int(img.size[0]), int(img.size[1])) == (w, h):
        return {"created": 0, "moved": 0, "retargeted": 0, "hidden": 0, "shown": 0, "total": 0, "missing": [], "rewritten": 0}
    if (int(img.size[0]), int(img.size[1])) != (w, h):
        img.scale(w, h)
    try:
        if encoding == "data":
            img.colorspace_settings.name = "Non-Color"
        elif encoding == "srgb":
            img.colorspace_settings.name = "sRGB"
    except (TypeError, RuntimeError, ValueError):        # a colour-management config without these names: leave the image's own
        pass
    img.pixels.foreach_set(np.ascontiguousarray(pix).ravel())
    img.update()
    img[sp.MARK] = tag
    img[DIGEST] = digest
    return {"created": created, "moved": 0, "retargeted": 0, "hidden": 0, "shown": 0, "total": 0, "missing": [], "rewritten": 1}
