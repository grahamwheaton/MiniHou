"""Images in the graph: read one from Blender or a PNG file as a pixel grid, and write a grid (or baked points) back as an image.

See :mod:`minihou.images` for what a pixel grid is. ``image_out`` is a sink like ``scene_objects``: the link sends Blender the pixels and
the add-on makes or updates the image data-block.
"""
import os

import numpy as np

from .. import images
from ..inputs import INPUTS
from ..sdk import Bool, Enum, Float, Int, Str, Vec3, node

HEIGHT = ["none", "luminance", "red", "green", "blue", "alpha"]


def _file_token(params) -> str:
    path = str(params.get("file") or "")
    try:
        st = os.stat(path)
        return f"{st.st_mtime_ns}:{st.st_size}"
    except OSError:
        return "missing"


def _linearise(pix: np.ndarray, colorspace: str, is_float: bool, stored_space: str) -> np.ndarray:
    """``auto``: 8-bit sRGB images hold sRGB-encoded numbers; MiniHou colours are linear, as Blender's colour attributes are."""
    if colorspace == "auto" and not is_float and str(stored_space).lower().startswith("srgb"):
        out = pix.copy()
        out[..., :3] = images.srgb_to_linear(pix[..., :3])
        return out
    return pix


@node("blender_image", category="source", key_extra=lambda p: INPUTS.image_token(str(p.get("image") or "")), params=[
    Str("image", "", doc="Name of the Blender image (Image Editor; a texture, a painted mask, a height map). It is watched: paint on it "
                         "and the graph recooks"),
    Int("max_size", 1024, min=2, max=8192, doc="The longest side in pixels (bigger images are averaged down in blocks)"),
    Enum("colorspace", ["auto", "stored"], doc="auto: 8-bit sRGB images become linear colour like everything else in MiniHou; "
                                               "stored: the numbers exactly as the image holds them (masks, heights, normal maps)"),
    Float("size", 10.0, min=0.0001, soft_max=100.0, doc="Width of the grid in world units (the height follows the image's aspect)"),
    Vec3("center", (0.0, 0.0, 0.0)),
    Enum("height", HEIGHT, doc="Lift the points by this channel (a height map): z = value * height_scale; also stored as `height`"),
    Float("height_scale", 1.0, soft_min=-10.0, soft_max=10.0),
])
def blender_image(image, max_size, colorspace, size, center, height, height_scale):
    """A Blender image as a pixel grid: a point per pixel (row by row from the bottom left, like Blender), `Cd` and `Alpha` point
    attributes for the colour, `uv`, and the grid detail attributes the heightfield nodes use. Wrangles read and write pixels, heightfield
    nodes erode a height map, `image_out` sends the result back."""
    if not image:
        raise ValueError("set 'image' to the name of a Blender image")
    data = INPUTS.get_image(image)
    if data is None:
        raise ValueError(f"waiting for Blender image {image!r} (is Blender connected to this engine?)")
    if data.pixels is None:
        raise ValueError(f"Blender image {image!r}: {data.error}")
    pix = images.downsample(_linearise(data.pixels, colorspace, bool(data.meta.get("is_float")), data.meta.get("colorspace", "")),
                            int(max_size))
    return images.grid_from_pixels(pix, size, tuple(center), height, height_scale, image)


@node("image_file", category="source", key_extra=_file_token, params=[
    Str("file", "", doc="A PNG file (any format Pillow reads, when it is installed). A relative path starts in the folder the engine runs in"),
    Int("max_size", 1024, min=2, max=8192, doc="The longest side in pixels (bigger images are averaged down in blocks)"),
    Enum("colorspace", ["auto", "stored"], doc="auto: colour images are sRGB, so they become linear colour; stored: the numbers as they are "
                                               "(masks, heights)"),
    Float("size", 10.0, min=0.0001, soft_max=100.0, doc="Width of the grid in world units (the height follows the image's aspect)"),
    Vec3("center", (0.0, 0.0, 0.0)),
    Enum("height", HEIGHT, doc="Lift the points by this channel (a height map): z = value * height_scale; also stored as `height`"),
    Float("height_scale", 1.0, soft_min=-10.0, soft_max=10.0),
])
def image_file(file, max_size, colorspace, size, center, height, height_scale):
    """A PNG file as a pixel grid, like `blender_image` but from disk: the way to bring in height maps and masks that came from Houdini or
    any paint program."""
    if not str(file).strip():
        raise ValueError("set 'file' to a PNG file")
    path = str(file).strip()
    if not os.path.isfile(path):
        raise ValueError(f"no such file: {path}")
    try:
        pix = images.read_png(path, srgb=(colorspace == "auto"))
    except (ValueError, OSError) as e:
        raise ValueError(f"cannot read {os.path.basename(path)}: {e}") from None
    return images.grid_from_pixels(images.downsample(pix, int(max_size)), size, tuple(center), height, height_scale, os.path.basename(path))


@node("image_out", inputs=["geo"], category="output", params=[
    Str("image", "MiniHou", doc="Name of the Blender image to make or update (it keeps its place in the file; its size follows the data)"),
    Enum("mode", ["lattice", "bake"], doc="lattice: the points are a pixel grid (an image, a grid, a heightfield) and give one pixel each; "
                                          "bake: any points are binned into the image by their X / Y position (mean per pixel, "
                                          "empty pixels transparent)"),
    Str("source", "Cd", doc="Point attribute written to the pixels: a scalar is grey, a vector is red green blue, 4 values include alpha. "
                            "`P` works too (positions as colours)"),
    Int("width", 512, min=1, max=8192, doc="bake: image width in pixels"),
    Int("height", 512, min=1, max=8192, doc="bake: image height in pixels"),
    Bool("normalize", False, doc="Stretch the values to 0..1 (a height map before it becomes an image)"),
    Enum("encoding", ["linear", "srgb", "data"], doc="linear: scene-linear colour in a float image; srgb: encode for an 8-bit sRGB image; "
                                                      "data: a float Non-Color image (heights, masks, vectors)"),
    Str("file", "", doc="Also write a PNG here (8-bit sRGB for 'srgb', else 16-bit linear numbers): the way to hand an image to Houdini"),
])
def image_out(geo, image, mode, source, width, height, normalize, encoding, file):
    """Send an image to Blender: the geometry passes through unchanged, and the link makes (or updates) the Blender image `image` from
    the attribute `source`, optionally writing a PNG as well. Float images keep every value; `srgb` makes an 8-bit sRGB image."""
    pixels(geo, mode, source, width, height, normalize)            # validates: a bad setup is an error on this node
    return geo


def pixels(geo, mode: str, source: str, width: int, height: int, normalize: bool) -> np.ndarray:
    name = str(source).strip() or "Cd"
    if name == "P":
        values = np.asarray(geo.P, np.float32)
    else:
        values = geo.point.get(name)
        if values is None:
            raise ValueError(f"no point attribute {name!r} to make an image from; available: {', '.join(geo.point.names()) or 'none'}")
        if values.dtype.kind not in "fiub" or values.ndim > 2:
            raise ValueError(f"point attribute {name!r} cannot be an image (it must be numbers: 1 to 4 per point)")
    if mode == "bake":
        return images.bake(geo, values, int(width), int(height), bool(normalize))
    return images.pixels_from_lattice(geo, values, bool(normalize))


def build_image_manifest(node_id: str, params: dict, geo, key: str):
    """``(header, arrays)``: a ``scene_objects`` message with ``mode: "image"`` the add-on turns into a Blender image. Header: ``image`` (name),
    ``width``, ``height``, ``encoding``; array ``pixels`` (float32, H x W x 4, rows from the bottom)."""
    pix = pixels(geo, params["mode"], params["source"], params["width"], params["height"], params["normalize"])
    if params["file"]:
        images.write_png(str(params["file"]).strip(), pix, bits=8 if params["encoding"] == "srgb" else 16, srgb=params["encoding"] == "srgb")
    h, w = pix.shape[:2]
    if params["encoding"] == "srgb":                       # an 8-bit sRGB image holds the encoded numbers
        pix = pix.copy()
        pix[..., :3] = images.linear_to_srgb(pix[..., :3])
    name = str(params["image"]).strip() or "MiniHou"
    header = dict(type="scene_objects", node=node_id, key=key, mode="image", prefix="", collection="", sources=[], id=[],
                  image=name[:63], width=int(w), height=int(h), encoding=params["encoding"])
    return header, {"pixels": np.ascontiguousarray(pix, np.float32)}
