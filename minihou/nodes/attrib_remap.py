"""Attribute Remap geometry node: remaps attribute values using min/max ranges, clamping,
and curve shaping profiles (Houdini's Attribute Remap SOP).
"""
from __future__ import annotations

import fnmatch
from typing import Optional

import numpy as np

from .. import groups
from ..sdk import Bool, Enum, Float, Str, node

DOMAINS = ["guess", "point", "prim", "vertex", "detail"]
GROUP_TYPES = ["guess", "points", "primitives", "vertices"]
CLAMP_TYPES = ["clamp", "none", "repeat", "ping_pong"]
CURVE_SHAPES = [
    "linear",
    "smoothstep",
    "smootherstep",
    "ease_in",
    "ease_out",
    "s_curve",
    "exponential",
    "invert",
    "bell",
]


def _evaluate_curve(t: np.ndarray, shape: str) -> np.ndarray:
    """Evaluate curve ramp shaping on normalized t in [0, 1]."""
    if shape == "linear":
        return t
    elif shape == "smoothstep":
        # 3t^2 - 2t^3
        tc = np.clip(t, 0.0, 1.0)
        return tc * tc * (3.0 - 2.0 * tc)
    elif shape == "smootherstep":
        # 6t^5 - 15t^4 + 10t^3
        tc = np.clip(t, 0.0, 1.0)
        return tc * tc * tc * (tc * (tc * 6.0 - 15.0) + 10.0)
    elif shape == "ease_in":
        # Quadratic ease in: t^2
        return t * t
    elif shape == "ease_out":
        # Quadratic ease out: 1 - (1 - t)^2
        return 1.0 - (1.0 - t) * (1.0 - t)
    elif shape == "s_curve":
        # Sigmoid S-curve centered at 0.5
        sig = 1.0 / (1.0 + np.exp(-10.0 * (t - 0.5)))
        sig_0 = 1.0 / (1.0 + np.exp(5.0))
        sig_1 = 1.0 / (1.0 + np.exp(-5.0))
        return (sig - sig_0) / (sig_1 - sig_0)
    elif shape == "exponential":
        # Exponential growth
        tc = np.clip(t, 0.0, 1.0)
        return (np.power(2.0, 10.0 * (tc - 1.0)) - 0.0009765625) / (1.0 - 0.0009765625)
    elif shape == "invert":
        return 1.0 - t
    elif shape == "bell":
        # Sine bell curve peak at 0.5
        return np.sin(np.clip(t, 0.0, 1.0) * np.pi)
    return t


@node("attribute_remap", inputs=["geo"], category="attribute", version=1, params=[
    Str("group", "", doc="Element group to limit the remapping to"),
    Enum("group_type", GROUP_TYPES, default="guess", doc="Group domain: points, primitives, or vertices"),
    Enum("class", DOMAINS, default="guess", doc="Attribute class/domain: point, prim, vertex, detail"),
    Str("original_name", "Cd", doc="Attribute to remap"),
    Str("new_name", "", doc="New attribute name (empty overwrites original in place)"),
    Float("inputmin", 0.0, doc="Input range minimum"),
    Bool("compute_range", True, doc="Compute input range automatically from the data"),
    Float("inputmax", 1.0, doc="Input range maximum"),
    Float("outputmin", 0.0, doc="Output range minimum"),
    Float("outputmax", 1.0, doc="Output range maximum"),
    Enum("clamptype", CLAMP_TYPES, default="clamp", doc="How out of range values are treated"),
    Bool("useramp", True, doc="Enable curve shaping profile"),
    Enum("remap", CURVE_SHAPES, default="linear", doc="Ramp curve shaping profile"),
])
def attribute_remap(geo, group="", group_type="guess", original_name="Cd", new_name="",
                    inputmin=0.0, compute_range=True, inputmax=1.0, outputmin=0.0, outputmax=1.0,
                    clamptype="clamp", useramp=True, remap="linear", **kwargs):
    """Remap attribute values using input/output ranges and curve profiles (Houdini's Attribute Remap SOP)."""
    attrib_class = kwargs.get("class", "guess")
    orig = original_name.strip()
    if not orig:
        return geo

    out = geo.copy()
    dst_name = new_name.strip() if new_name.strip() else orig

    # 1. Determine domain
    target_dom: Optional[str] = None
    if attrib_class != "guess":
        target_dom = attrib_class
    else:
        # Search point, prim, vertex, detail
        for d in ("point", "prim", "vertex", "detail"):
            if d == "detail":
                if orig in out.detail:
                    target_dom = d
                    break
            elif orig in out.table(d):
                target_dom = d
                break

    if target_dom is None:
        return geo

    # 2. Extract values
    if target_dom == "detail":
        val = out.detail.get(orig)
        if val is None:
            return geo
        arr = np.asarray(val, dtype=np.float64)
        is_scalar = arr.ndim == 0 or (arr.ndim == 1 and len(arr) == 1)
    else:
        table = out.table(target_dom)
        if orig not in table:
            return geo
        arr = np.asarray(table[orig], dtype=np.float64)
        is_scalar = arr.ndim == 1 or (arr.ndim == 2 and arr.shape[1] == 1)

    # 3. Calculate Input Range
    if compute_range:
        in_lo = float(np.min(arr))
        in_hi = float(np.max(arr))
    else:
        in_lo = float(inputmin)
        in_hi = float(inputmax)

    range_span = in_hi - in_lo
    if abs(range_span) < 1e-12:
        t = np.zeros_like(arr, dtype=np.float64)
    else:
        t = (arr - in_lo) / range_span

    # 4. Apply Out of Range Values (Clamping / Wrap)
    if clamptype == "clamp":
        t = np.clip(t, 0.0, 1.0)
    elif clamptype == "repeat":
        t = np.mod(t, 1.0)
    elif clamptype == "ping_pong":
        # Triangle wave 0 -> 1 -> 0
        mod2 = np.mod(t, 2.0)
        t = 1.0 - np.abs(mod2 - 1.0)
    # else "none": do not clamp

    # 5. Apply Ramp Curve Shaping
    if useramp:
        t = _evaluate_curve(t, remap)

    # 6. Map to Output Range
    out_lo = float(outputmin)
    out_hi = float(outputmax)
    remapped = out_lo + t * (out_hi - out_lo)

    # 7. Write back
    orig_dtype = np.float32
    if target_dom != "detail" and orig in out.table(target_dom):
        orig_dtype = out.table(target_dom)[orig].dtype
        if not np.issubdtype(orig_dtype, np.floating):
            orig_dtype = np.float32

    res_arr = remapped.astype(orig_dtype)

    if target_dom == "detail":
        out.detail[dst_name] = res_arr.item() if is_scalar else res_arr.tolist()
        return out

    # Handle element group mask if applicable
    if group and target_dom in ("point", "prim"):
        mask = groups.mask(geo, target_dom, group)
        if mask is not None:
            curr = out.table(target_dom)[orig]
            if orig == dst_name:
                final = curr.copy()
                final[mask] = res_arr[mask]
            else:
                final = curr.copy() if dst_name in out.table(target_dom) else np.zeros_like(res_arr)
                final[mask] = res_arr[mask]
            res_arr = final

    out.table(target_dom)[dst_name] = res_arr
    return out
