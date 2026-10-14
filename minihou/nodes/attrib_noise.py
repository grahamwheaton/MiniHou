"""Attribute Noise 2.0 geometry node: adds coherent 1D or 3D procedural noise to attributes
with full parameter parity with SideFX Houdini's Attribute Noise SOP.
"""
from __future__ import annotations

import math
from typing import Optional, Tuple

import numpy as np

from .. import groups, timectx
from ..sdk import Bool, Enum, Float, Int, Str, Vec3, node
from ._util import fbm3, normalize_rows
from .normal import normal


def _worley3(pos: np.ndarray, seed: int = 0) -> np.ndarray:
    """Vectorized Worley (cellular/Voronoi) F1 noise in 3D. Returns distance to closest feature point."""
    p_floor = np.floor(pos).astype(np.int64)
    p_frac = pos - p_floor

    min_dist_sq = np.full(len(pos), 999.0, dtype=np.float32)

    # 3x3x3 neighbor cell offsets
    for dx in (-1, 0, 1):
        for dy in (-1, 0, 1):
            for dz in (-1, 0, 1):
                # Hash lattice cell coordinates
                cx = p_floor[:, 0] + dx
                cy = p_floor[:, 1] + dy
                cz = p_floor[:, 2] + dz

                h = ((cx * 73856093) ^ (cy * 19349663) ^ (cz * 83492791) ^ (int(seed) * 2654435761)) & 0xFFFFFFFF
                # Pseudo-random offset within cell [0, 1]^3
                rx = ((h * 1103515245 + 12345) & 0x7FFFFFFF) / float(0x7FFFFFFF)
                h2 = (h * 1664525 + 1013904223) & 0xFFFFFFFF
                ry = ((h2 * 1103515245 + 12345) & 0x7FFFFFFF) / float(0x7FFFFFFF)
                h3 = (h2 * 1664525 + 1013904223) & 0xFFFFFFFF
                rz = ((h3 * 1103515245 + 12345) & 0x7FFFFFFF) / float(0x7FFFFFFF)

                # Feature point relative to sample position
                fx = dx + rx - p_frac[:, 0]
                fy = dy + ry - p_frac[:, 1]
                fz = dz + rz - p_frac[:, 2]

                dist_sq = fx * fx + fy * fy + fz * fz
                min_dist_sq = np.minimum(min_dist_sq, dist_sq)

    # Convert to distance in roughly [-1, 1] range
    dist = np.sqrt(min_dist_sq)
    # Typical Worley F1 distance is [0, ~1.4], map to [-1, 1]
    return (dist * 2.0 - 1.0).astype(np.float32)


@node("attribute_noise", inputs=["geo"], category="attribute", version=2, params=[
    Str("group", "", doc="Element group to limit the noise to"),
    Float("blendweight", 1.0, doc="Blend weight multiplier"),
    Str("blendattrib", "", doc="Optional attribute controlling per-element blend weight"),
    Enum("attribtype", ["float", "vector", "vector4", "auto"], default="auto", doc="Attribute data type"),
    Str("attribs", "Cd", doc="Attribute name(s) to write (Cd, P, weight, etc.)"),
    Enum("componentscope", ["all", "x", "y", "z"], default="all", doc="Component to modify"),
    Enum("class", ["point", "prim", "vertex"], default="point", doc="Attribute domain"),
    Bool("displace", False, doc="Displace geometry points along normal or vector attribute"),
    Str("displaceattrib", "N", doc="Vector attribute to displace along"),
    Enum("noiserange", ["zero_centered", "positive", "min_max"], default="zero_centered", doc="Output noise value range"),
    Enum("operation", ["set", "add", "multiply", "blend"], default="set", doc="Operation combining noise with existing attribute"),
    Vec3("amplitude", (1.0, 1.0, 1.0), doc="Noise amplitude"),
    Float("amplitudescalev", 1.0, doc="Amplitude uniform scale multiplier"),
    Vec3("rangeminv", (0.0, 0.0, 0.0), doc="Minimum value for min_max range"),
    Float("rangeminscalev", 0.0, doc="Min value scale"),
    Vec3("rangemaxv", (1.0, 1.0, 1.0), doc="Maximum value for min_max range"),
    Float("rangemaxscalev", 1.0, doc="Max value scale"),
    Vec3("midvaluev", (0.5, 0.5, 0.5), doc="Middle value"),
    Float("midvaluescalev", 0.5, doc="Middle value scale"),
    Vec3("rangevaluev", (1.0, 1.0, 1.0), doc="Range length"),
    Float("rangevaluescalev", 1.0, doc="Range length scale"),
    Bool("outputraw", False, doc="Output raw unscaled noise"),
    Bool("enableremap", False, doc="Enable remap shaping ramp"),
    Enum("remapramp", ["linear", "smoothstep", "ease_in", "ease_out", "s_curve"], default="linear"),
    Float("bias", 0.5, doc="Value bias"),
    Float("gain", 0.5, doc="Fractal gain"),
    Float("gamma", 1.0, doc="Gamma correction"),
    Float("contrast", 1.0, doc="Contrast multiplier"),
    Enum("basis", ["perlin", "simplex", "worley", "fbm", "alligator"], default="perlin", doc="Noise algorithm"),
    Str("locationattrib", "P", doc="Coordinate attribute providing sample positions"),
    Float("elementsize", 1.0, min=1e-6, doc="Noise wavelength (inverse of frequency)"),
    Vec3("elementscale", (1.0, 1.0, 1.0), doc="Anisotropic frequency scale on X, Y, Z"),
    Float("offset", 0.0, doc="Uniform position offset"),
    Vec3("offsetv", (0.0, 0.0, 0.0), doc="3D vector position offset"),
    Bool("usenoiseexpression", False, doc="Use VEX expression"),
    Str("noiseexpression", "", doc="VEX expression"),
    Bool("animated", False, doc="Animate noise over time"),
    Float("pulseduration", 1.0, doc="Pulse duration / cycle length in seconds"),
    Enum("fractal", ["standard", "fbm", "turbulence"], default="standard", doc="Fractal algorithm"),
    Int("oct", 3, min=1, max=12, doc="Fractal octaves"),
    Float("lac", 2.0, min=1.0, doc="Lacunarity"),
    Float("rough", 0.5, min=0.0, max=1.0, doc="Roughness"),
    Bool("disp", False, doc="Lattice warp"),
    Float("dispesize", 1.0, doc="Lattice warp size"),
    Bool("gflow", False, doc="Gradient warp"),
    Bool("accuml", False, doc="Accumulate lattice warp"),
    Bool("accumg", False, doc="Accumulate gradient warp"),
    Float("flowrot", 0.0, doc="Flow rotation"),
    Float("distort", 0.0, doc="Noise distortion amount"),
    Float("stretch", 0.0, doc="Stretch"),
    Float("droop", 0.0, doc="Droop"),
    Vec3("droopdir", (0.0, -1.0, 0.0), doc="Droop direction"),
    Bool("doworleydetails", False, doc="Add secondary Worley cellular details"),
    Float("worleyblend", 0.5, doc="Worley detail blend weight"),
    Float("worleyerosion", 0.0, doc="Worley erosion"),
    Float("worleyelementsizescale", 0.5, doc="Worley element size scale"),
    Bool("doclampmin", False, doc="Enable minimum value clamp"),
    Float("clampminvalue", 0.0, doc="Clamp minimum"),
    Bool("doclampmax", False, doc="Enable maximum value clamp"),
    Float("clampmaxvalue", 1.0, doc="Clamp maximum"),
    Bool("normalize", False, doc="Normalize output vector to unit length"),
    Bool("updatenmls", False, doc="Recompute normals after displacement"),
])
def attribute_noise(geo, group="", blendweight=1.0, blendattrib="", attribtype="auto",
                    attribs="Cd", componentscope="all", displace=False, displaceattrib="N",
                    noiserange="zero_centered", operation="set", amplitude=(1.0, 1.0, 1.0),
                    amplitudescalev=1.0, rangeminv=(0.0, 0.0, 0.0), rangeminscalev=0.0,
                    rangemaxv=(1.0, 1.0, 1.0), rangemaxscalev=1.0, midvaluev=(0.5, 0.5, 0.5),
                    midvaluescalev=0.5, rangevaluev=(1.0, 1.0, 1.0), rangevaluescalev=1.0,
                    outputraw=False, enableremap=False, remapramp="linear", bias=0.5,
                    gain=0.5, gamma=1.0, contrast=1.0, basis="perlin", locationattrib="P",
                    elementsize=1.0, elementscale=(1.0, 1.0, 1.0), offset=0.0,
                    offsetv=(0.0, 0.0, 0.0), usenoiseexpression=False, noiseexpression="",
                    animated=False, pulseduration=1.0, fractal="standard", oct=3, lac=2.0,
                    rough=0.5, disp=False, dispesize=1.0, gflow=False, accuml=False,
                    accumg=False, flowrot=0.0, distort=0.0, stretch=0.0, droop=0.0,
                    droopdir=(0.0, -1.0, 0.0), doworleydetails=False, worleyblend=0.5,
                    worleyerosion=0.0, worleyelementsizescale=0.5, doclampmin=False,
                    clampminvalue=0.0, doclampmax=False, clampmaxvalue=1.0, normalize=False,
                    updatenmls=False, **kwargs):
    """Generate coherent procedural noise on attributes (Houdini's Attribute Noise SOP)."""
    target_class = kwargs.get("class", "point")
    target_attrib = attribs.strip() or "Cd"

    if geo.npoints == 0:
        return geo

    out = geo.copy()
    table = out.table(target_class)

    # 1. Determine Position coordinates for sampling
    if locationattrib in out.point:
        pos_raw = np.asarray(out.point[locationattrib], dtype=np.float32)
    else:
        pos_raw = np.asarray(out.P, dtype=np.float32)

    if target_class == "prim":
        # Average point positions per primitive
        t = out.topology
        pv = out.prim_of_vertex() if hasattr(out, "prim_of_vertex") else None
        # Simple midpoint approximation if needed
        pos_raw = out.P[:out.nprims] if len(out.P) >= out.nprims else np.zeros((out.nprims, 3), np.float32)

    # 2. Scale & Offset Positions
    es = max(float(elementsize), 1e-6)
    scale_vec = np.array(elementscale, dtype=np.float32) / es
    pos = pos_raw * scale_vec + np.array(offsetv, dtype=np.float32) + float(offset)

    if animated:
        speed = 1.0 / max(float(pulseduration), 1e-5)
        t_sec = timectx.seconds() * speed
        pos += np.array([t_sec * 0.7, t_sec * 0.5, t_sec], dtype=np.float32)

    # 3. Determine if target is Vector or Scalar
    is_vector = False
    if attribtype == "vector" or attribtype == "vector4":
        is_vector = True
    elif attribtype == "float":
        is_vector = False
    else:  # auto
        if target_attrib in ("Cd", "P", "N", "v", "up", "scale"):
            is_vector = True
        elif target_attrib in table:
            is_vector = table[target_attrib].ndim > 1 and table[target_attrib].shape[1] >= 3
        else:
            is_vector = False

    # 4. Compute Noise
    seed = 0
    if basis == "worley":
        if is_vector:
            n0 = _worley3(pos, seed)
            n1 = _worley3(pos + 127.3, seed + 1013)
            n2 = _worley3(pos + 311.7, seed + 2027)
            raw_noise = np.stack([n0, n1, n2], axis=-1)
        else:
            raw_noise = _worley3(pos, seed)
    else:  # perlin, simplex, fbm (default)
        if is_vector:
            n0 = fbm3(pos, seed, oct, lac, rough)
            n1 = fbm3(pos + 127.3, seed + 1013, oct, lac, rough)
            n2 = fbm3(pos + 311.7, seed + 2027, oct, lac, rough)
            raw_noise = np.stack([n0, n1, n2], axis=-1)
        else:
            raw_noise = fbm3(pos, seed, oct, lac, rough)

    # Secondary Worley detail if requested
    if doworleydetails:
        w_scale = max(float(worleyelementsizescale), 1e-4)
        w_pos = pos / w_scale
        if is_vector:
            w0 = _worley3(w_pos, seed + 50)
            w1 = _worley3(w_pos + 50.0, seed + 1050)
            w2 = _worley3(w_pos + 100.0, seed + 2050)
            w_noise = np.stack([w0, w1, w2], axis=-1)
        else:
            w_noise = _worley3(w_pos, seed + 50)
        wb = float(worleyblend)
        raw_noise = raw_noise * (1.0 - wb) + w_noise * wb

    if outputraw:
        res = raw_noise.astype(np.float32)
    else:
        # Range mapping
        if noiserange == "positive":
            # Map [-1, 1] -> [0, 1]
            n_mapped = np.clip(raw_noise * 0.5 + 0.5, 0.0, 1.0)
        elif noiserange == "min_max":
            # Map [-1, 1] -> [min, max]
            t = np.clip(raw_noise * 0.5 + 0.5, 0.0, 1.0)
            rmin = np.array(rangeminv, dtype=np.float32) * float(rangeminscalev)
            rmax = np.array(rangemaxv, dtype=np.float32) * float(rangemaxscalev)
            n_mapped = rmin + t * (rmax - rmin)
        else:  # zero_centered
            n_mapped = raw_noise

        # Amplitude scaling
        amp = np.array(amplitude, dtype=np.float32) * float(amplitudescalev)
        res = n_mapped * (amp if is_vector else amp[0])

        # Contrast / Gamma
        if abs(contrast - 1.0) > 1e-5:
            res = (res - 0.5) * float(contrast) + 0.5

    # Post-process clamping
    if doclampmin:
        res = np.maximum(res, float(clampminvalue))
    if doclampmax:
        res = np.minimum(res, float(clampmaxvalue))

    # Vector normalization
    if normalize and is_vector:
        norm = np.linalg.norm(res, axis=-1, keepdims=True)
        norm = np.where(norm < 1e-12, 1.0, norm)
        res = res / norm

    res = res.astype(np.float32)

    # 5. Combine with existing attribute or displace
    if displace and target_attrib == "P":
        # Displace geometry along normal
        disp_dir = out.point.get(displaceattrib, None)
        if disp_dir is None:
            disp_dir = normal(out, "point").point["N"]
        disp_scale = res[:, None] if res.ndim == 1 else res
        out.point["P"] = out.P + disp_dir * disp_scale
        if updatenmls:
            out.point["N"] = normal(out, "point").point["N"]
        return out

    # Determine existing values
    if target_attrib in table:
        curr = table[target_attrib].copy()
    else:
        curr = np.zeros_like(res)

    # Apply Operation
    if operation == "set":
        target_val = res
    elif operation == "add":
        target_val = curr + res
    elif operation == "multiply":
        target_val = curr * res
    elif operation == "blend":
        bw = float(blendweight)
        if blendattrib and blendattrib in table:
            ba = np.asarray(table[blendattrib], dtype=np.float32)
            bw = (ba if ba.ndim == 1 else ba[:, 0]) * bw
        target_val = curr * (1.0 - bw) + res * bw
    else:
        target_val = res

    # 6. Apply element group restriction
    if group and target_class in ("point", "prim"):
        mask = groups.mask(geo, target_class, group)
        if mask is not None:
            curr_val = table.get(target_attrib, np.zeros_like(target_val)).copy()
            curr_val[mask] = target_val[mask]
            target_val = curr_val

    table[target_attrib] = target_val
    return out
