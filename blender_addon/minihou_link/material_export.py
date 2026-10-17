"""Main-thread, read-only Blender material and image snapshots for viewport transport."""
import hashlib
import os
import struct
import zlib
from types import SimpleNamespace

import bpy
import numpy as np

MAX_PIXELS = 16_000_000


def _png(width, height, rgba):
    def block(tag, data):
        return struct.pack(">I", len(data)) + tag + data + struct.pack(">I", zlib.crc32(tag + data))
    rows = b"".join(b"\0" + rgba[y * width * 4:(y + 1) * width * 4]
                    for y in range(height))
    return (b"\x89PNG\r\n\x1a\n" + block(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 6, 0, 0, 0))
            + block(b"IDAT", zlib.compress(rows, 3)) + block(b"IEND", b""))


class ImageUnavailable(ValueError):
    """The image has no pixel data (missing file, unsupported format, or an unrendered viewer)."""


def _size(image):
    w, h = map(int, image.size)
    if w <= 0 or h <= 0:
        path = image.filepath or image.name
        raise ImageUnavailable(f"image '{image.name}' is empty or could not be loaded ({path})")
    return w, h


def _pixels(image, color_space):
    w, h = _size(image)
    channels = int(image.channels)
    data = np.empty(w * h * channels, dtype=np.float32)
    image.pixels.foreach_get(data)
    data = data.reshape(h, w, channels)
    rgba = np.ones((h, w, 4), np.float32)
    rgba[:, :, :min(channels, 4)] = data[:, :, :min(channels, 4)]
    if channels == 1:
        rgba[:, :, 1:3] = rgba[:, :, :1]
    if w * h > MAX_PIXELS:
        # Over the limit: box-average k x k blocks (k smallest integer that fits) rather than dropping the texture.
        k = 2
        while (w // k) * (h // k) > MAX_PIXELS:
            k += 1
        nw, nh = w // k, h // k
        if nw < 1 or nh < 1:
            raise ImageUnavailable(f"image '{image.name}' has unsupported dimensions {w}x{h}")
        rgba = rgba[:nh * k, :nw * k].reshape(nh, k, nw, k, 4).mean(axis=(1, 3), dtype=np.float32)
        w, h = nw, nh
    if color_space == "srgb":
        rgb = np.clip(rgba[:, :, :3], 0, 1)
        rgba[:, :, :3] = np.where(rgb <= 0.0031308, rgb * 12.92, 1.055 * np.power(rgb, 1 / 2.4) - 0.055)
    raw = np.flipud(np.clip(rgba * 255 + .5, 0, 255).astype(np.uint8)).tobytes()
    return _png(w, h, raw)


def image_bytes(image, color_space):
    if image.source in {"TILED", "MOVIE", "SEQUENCE"}:
        raise ValueError(f"{image.source} images are unsupported")
    w, h = _size(image)
    if w * h > MAX_PIXELS:
        return _pixels(image, color_space)          # too large to forward as-is: downscale to fit
    if image.packed_file and not image.is_dirty:
        data = bytes(image.packed_file.data)
        if data.startswith((b"\x89PNG", b"\xff\xd8")):
            return data
    path = bpy.path.abspath(image.filepath, library=image.library) if image.filepath else ""
    if image.source == "FILE" and path and not image.is_dirty and os.path.isfile(path):
        with open(path, "rb") as stream:
            data = stream.read(256 * 1024 * 1024 + 1)
        if len(data) > 256 * 1024 * 1024:
            raise ValueError("image file exceeds 256 MiB")
        if data.startswith((b"\x89PNG", b"\xff\xd8")):
            return data
    return _pixels(image, color_space)


def _source(socket):
    for _ in range(8):
        if not socket.is_linked:
            return None
        node = socket.links[0].from_node
        if node.type == "REROUTE":
            socket = node.inputs[0]
        elif node.type == "GROUP" and node.node_tree:
            output = next((n for n in node.node_tree.nodes if n.type == "GROUP_OUTPUT" and n.is_active_output), None)
            name = socket.links[0].from_socket.name
            if output is None or name not in output.inputs:
                raise ValueError("group output cannot be resolved")
            socket = output.inputs[name]
        else:
            return node
    raise ValueError("shader link nesting exceeds limit")


def export_object(obj, render_uv=None):
    """Return manifest and hash->bytes. Descriptor IDs include object and slot index."""
    descriptors, bindings, images, diagnostics = {}, [], {}, {}
    if render_uv is None:
        mesh = getattr(obj, "data", None)
        render_uv = next((layer.name for layer in getattr(mesh, "uv_layers", ()) if layer.active_render), None)
    for index, slot in enumerate(obj.material_slots):
        material = slot.material
        key = f"blender:{obj.name}:{index}"
        bindings.append(key)
        factor = list(material.diffuse_color) if material else [1, 1, 1, 1]
        descriptor = {"version": 1, "name": material.name if material else "", "base_color_factor": factor,
                      "textures": {}, "alpha_mode": "OPAQUE", "double_sided": True}
        try:
            if material and material.use_nodes:
                output = next((n for n in material.node_tree.nodes if n.type == "OUTPUT_MATERIAL" and n.is_active_output), None)
                shader = _source(output.inputs["Surface"]) if output else None
                if shader is None or shader.type != "BSDF_PRINCIPLED":
                    raise ValueError("active output is not a Principled BSDF")
                alpha = shader.inputs.get("Alpha")
                mode = getattr(material, "surface_render_method", getattr(material, "blend_method", "OPAQUE"))
                if alpha and alpha.is_linked:
                    raise ValueError(f"non-opaque alpha is unsupported: material {material.name}, Principled Alpha socket is linked, render mode={mode}")
                if alpha and float(alpha.default_value) < .999:
                    raise ValueError(f"non-opaque alpha is unsupported: material {material.name}, Principled Alpha={float(alpha.default_value):.3f}, render mode={mode}")
                color = shader.inputs["Base Color"]
                factor = list(color.default_value)
                descriptor["base_color_factor"] = factor
                node = _source(color)
                if node:
                    if node.type != "TEX_IMAGE":
                        raise ValueError("Base Color shader node is unsupported")
                    image = node.image
                    if image is None:
                        raise ValueError("Image Texture has no image")
                    uv = _source(node.inputs["Vector"])
                    if uv and uv.type == "MAPPING":
                        if (any(abs(float(x)) > 1e-6 for x in uv.inputs["Rotation"].default_value)
                                or any(abs(float(x)) > 1e-6 for x in uv.inputs["Location"].default_value)
                                or any(abs(float(x) - 1) > 1e-6 for x in uv.inputs["Scale"].default_value)):
                            raise ValueError("non-identity Mapping transform is unsupported")
                        uv = _source(uv.inputs["Vector"])
                    if uv and uv.type not in {"UVMAP", "TEX_COORD"}:
                        raise ValueError("texture coordinates are unsupported")
                    if uv and uv.type == "TEX_COORD" and node.inputs["Vector"].links[0].from_socket.name != "UV":
                        raise ValueError("non-UV coordinates are unsupported")
                    color_space = "linear" if image.colorspace_settings.name == "Non-Color" else "srgb"
                    payload = image_bytes(image, color_space)
                    hash_id = "sha256:" + hashlib.sha256(payload).hexdigest()
                    images[hash_id] = payload
                    extension = {"REPEAT": "repeat", "EXTEND": "clamp", "MIRROR": "mirror", "CLIP": "clamp"}.get(node.extension)
                    if not extension:
                        raise ValueError(f"{node.extension} extension is unsupported")
                    if node.interpolation not in {"Linear", "Closest"}:
                        raise ValueError(f"{node.interpolation} interpolation is unsupported")
                    filt = "nearest" if node.interpolation == "Closest" else "linear"
                    uv_set = uv.uv_map if uv and uv.type == "UVMAP" else "uv"
                    if uv_set == render_uv:
                        uv_set = "uv"
                    descriptor["textures"]["base_color"] = {"image_id": hash_id, "uv_set": uv_set,
                        "sampler": {"min": filt, "mag": filt, "wrap_u": extension, "wrap_v": extension}, "color_space": color_space}
        except (ValueError, RuntimeError, KeyError, OSError) as exc:
            diagnostics[key] = str(exc)
            if str(exc) == "Image Texture has no image" or isinstance(exc, (ImageUnavailable, OSError)):
                missing = "sha256:" + hashlib.sha256(("missing:" + key).encode()).hexdigest()
                descriptor["textures"] = {"base_color": {"image_id": missing, "uv_set": "uv", "color_space": "srgb"}}
            else:
                descriptor["textures"] = {}
        descriptors[key] = descriptor
    return {"descriptors": descriptors, "bindings": bindings, "images": list(images), "diagnostics": diagnostics}, images


def export_materials(materials, name, render_uv=None):
    """Export material slots on a disposable evaluated prefix or GN input."""
    return export_object(SimpleNamespace(name=name, material_slots=[SimpleNamespace(material=m) for m in materials]), render_uv)
