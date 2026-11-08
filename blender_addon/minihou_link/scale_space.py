"""Object scale for the MiniHou side of a Blender modifier.

Blender evaluates a modifier stack in object space and applies the object's transform afterwards, so a
non-uniformly scaled object looks very different in Blender than its raw modifier input does. MiniHou should
show and edit the geometry at the size it has in the scene, so the input points are multiplied by the object's
world scale on the way in and divided by it on the way out. Rotation and translation stay with Blender.

numpy only, so the arithmetic is testable without Blender.
"""
from __future__ import annotations

from typing import Dict, Optional, Tuple

import numpy as np

from .protocol import attr_key

P_KEY = attr_key("point", "P")
_EPS = 1e-9


def world_scale(obj) -> Optional[Tuple[float, float, float]]:
    """The object's world scale, or None when it is 1 on every axis (nothing to do) or unusable."""
    try:
        s = tuple(float(c) for c in obj.matrix_world.to_scale())
    except (AttributeError, TypeError, ValueError):
        return None
    if any(abs(c) < _EPS or not np.isfinite(c) for c in s):
        return None                                          # a zero scale cannot be undone
    if all(abs(c - 1.0) < 1e-6 for c in s):
        return None
    return s


def scale_arrays(arrays: Dict[str, np.ndarray], scale) -> None:
    """Multiply the point positions in ``arrays`` by ``scale`` (in place; the position array is replaced, not edited)."""
    if scale is None or P_KEY not in arrays:
        return
    arrays[P_KEY] = np.ascontiguousarray(arrays[P_KEY] * np.asarray(scale, np.float32), dtype=np.float32)


def unscale_arrays(arrays: Dict[str, np.ndarray], scale) -> None:
    """The inverse of :func:`scale_arrays`."""
    if scale is None or P_KEY not in arrays:
        return
    arrays[P_KEY] = np.ascontiguousarray(arrays[P_KEY] / np.asarray(scale, np.float32), dtype=np.float32)
