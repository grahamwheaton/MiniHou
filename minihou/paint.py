"""Painted attributes: sparse per-element edits stored in a node, and the arithmetic of brush strokes.

The ``attrib_paint`` node holds one attribute on one domain. What was painted is a list of ``(element number, value)`` pairs, the
node's ``data`` parameter, so an unpainted element keeps what the input gave it (or the node's default) and a stroke only costs what it
touched. Elements are numbered the way the viewer numbers them: points, primitives (faces) and *derived edges* (the sorted endpoint
pairs of :func:`minihou.topo.edges`, whose edge group is the attribute). That makes the edits depend on the topology upstream, like a
``blast`` pattern: after a change that renumbers elements the stale ones are ignored (and counted in the detail attribute
``paint_stale_<name>``), never an error.

``data`` is ``mhp1:`` followed by base64 of a zlib stream: a uint32 count, then ``count`` int32 element numbers, then ``count x
components`` float32 values (little endian). The value width comes from the node's ``kind``.
"""
from __future__ import annotations

import base64
import re
import zlib
from typing import Optional, Tuple

import numpy as np

TAG = "mhp1:"
DOMAINS = ("point", "prim", "edge")
KINDS = {"float": (1, np.float32), "int": (1, np.int32), "vec3": (3, np.float32)}
MAX_EDITS = 4_000_000                         # elements one node may hold edits for
MAX_TEXT = 48_000_000                         # characters of encoded data
NAME = re.compile(r"^[A-Za-z_][A-Za-z0-9_]*$")


def comps(kind: str) -> int:
    return KINDS[kind][0]


def check_name(name: str, domain: str) -> str:
    if not NAME.match(name or ""):
        raise ValueError(f"attribute name {name!r} must be letters, digits and _ (not starting with a digit)")
    if name == "P":
        raise ValueError("P cannot be painted (use Transform, or Attribute Wrangle)")
    if domain == "edge" and name.startswith("edge_group_"):
        raise ValueError("an edge attribute's name is the group's name, without the edge_group_ prefix")
    return name


# -- the codec ---------------------------------------------------------------------------------------------------------------
def encode(idx: np.ndarray, val: np.ndarray) -> str:
    idx = np.asarray(idx, np.int32).reshape(-1)
    if len(idx) == 0:
        return ""
    val = np.asarray(val, np.float32).reshape(len(idx), -1)
    if len(idx) > MAX_EDITS:
        raise ValueError(f"too many painted elements ({len(idx):,}; at most {MAX_EDITS:,}): paint fewer, or bake the attribute with a wrangle")
    raw = np.uint32(len(idx)).tobytes() + idx.astype("<i4").tobytes() + val.astype("<f4").tobytes()
    text = TAG + base64.b64encode(zlib.compress(raw, 3)).decode("ascii")
    if len(text) > MAX_TEXT:
        raise ValueError("the painted data is too large for one node; paint fewer elements")
    return text


def decode(text: str, ncomp: int) -> Tuple[np.ndarray, np.ndarray]:
    """``(element numbers int64, values float32 (n, ncomp))``. Raises ``ValueError`` for anything that is not a valid ``data`` string."""
    if not text:
        return np.zeros(0, np.int64), np.zeros((0, ncomp), np.float32)
    if not text.startswith(TAG):
        raise ValueError("painted data is not in the mhp1 format (clear the data parameter to start over)")
    if len(text) > MAX_TEXT:
        raise ValueError("painted data is too large")
    try:
        raw = zlib.decompress(base64.b64decode(text[len(TAG):], validate=True), bufsize=1 << 20)
    except Exception as e:  # noqa: BLE001 - base64, zlib: any failure is just "damaged"
        raise ValueError(f"painted data is damaged ({type(e).__name__}); clear the data parameter to start over") from None
    if len(raw) < 4:
        raise ValueError("painted data is damaged (too short)")
    n = int(np.frombuffer(raw[:4], "<u4")[0])
    if n > MAX_EDITS or len(raw) != 4 + n * 4 + n * ncomp * 4:
        raise ValueError(f"painted data does not fit a {ncomp}-component attribute (damaged, or the node's type was changed)")
    idx = np.frombuffer(raw, "<i4", n, 4).astype(np.int64)
    val = np.frombuffer(raw, "<f4", n * ncomp, 4 + 4 * n).reshape(n, ncomp).astype(np.float32)
    if not np.isfinite(val).all():
        raise ValueError("painted data holds a value that is not a number")
    return idx, val


def merge(text: str, ncomp: int, idx, val=None, erase: bool = False) -> str:
    """The data after ``idx`` get ``val`` (one row per element) or, with ``erase``, go back to the input's value."""
    old_i, old_v = decode(text, ncomp)
    idx = np.asarray(idx, np.int64).reshape(-1)
    if (idx < 0).any():
        raise ValueError("element numbers cannot be negative")
    if len(idx) > MAX_EDITS:
        raise ValueError("too many elements in one assignment")
    if erase:
        keep = ~np.isin(old_i, idx)
        return encode(old_i[keep], old_v[keep])
    val = np.asarray(val, np.float32).reshape(len(idx), ncomp)
    if not np.isfinite(val).all():
        raise ValueError("a painted value is not a number")
    i = np.concatenate([old_i, idx])
    v = np.concatenate([old_v, val])
    _, last = np.unique(i[::-1], return_index=True)             # the newest value of each element wins
    sel = np.sort(len(i) - 1 - last)
    return encode(i[sel], v[sel])


# -- elements and values ---------------------------------------------------------------------------------------------------------
def count(geo, domain: str) -> int:
    if domain == "point":
        return geo.npoints
    if domain == "prim":
        return geo.nprims
    if domain == "edge":
        from . import topo
        return len(topo.edges(geo).points)
    raise ValueError(f"domain must be one of {', '.join(DOMAINS)}")


def base_values(geo, domain: str, attr: str, kind: str, default) -> np.ndarray:
    """What the elements hold before any painting: the input's attribute (converted to ``kind``) or ``default``."""
    n, (nc, dt) = count(geo, domain), KINDS[kind]
    d = np.asarray(default, np.float64).reshape(-1)
    out = np.tile(np.resize(d, nc) if d.size else np.zeros(nc), (n, 1))
    src = None
    if domain == "edge":
        from . import edges
        if "edge_group_" + attr in geo.vertex:
            src = edges.values(geo, "edge_group_" + attr).reshape(-1, 1)
    else:
        a = geo.table(domain).get(attr)
        if a is not None and a.dtype.kind in "fiub":
            src = a.reshape(len(a), -1)
    if src is not None and len(src) == n and n:
        out = (src[:, :nc] if src.shape[1] >= nc else np.broadcast_to(src[:, :1], (n, nc))).astype(np.float64)
        if src.shape[1] > 1 and nc == 1:
            out = src[:, :1].astype(np.float64)
    return np.rint(out) if kind == "int" else out


def apply(geo, domain: str, attr: str, kind: str, default, data: str):
    """``geo`` with the attribute painted. Returns ``(geometry, number of stale edits)``."""
    check_name(attr, domain)
    nc, dt = KINDS[kind]
    idx, val = decode(data, nc)
    vals = base_values(geo, domain, attr, kind, default)
    n = len(vals)
    ok = (idx >= 0) & (idx < n)
    vals[idx[ok]] = np.rint(val[ok]) if kind == "int" else val[ok]
    out = geo.copy()
    arr = vals.astype(dt)
    if nc == 1:
        arr = arr.reshape(-1)
    if domain == "point":
        out.point[attr] = arr
    elif domain == "prim":
        out.prim[attr] = arr
    else:
        from . import edges
        out = edges.write(out, "edge_group_" + attr, arr.reshape(-1).astype(np.int32 if kind == "int" else np.float32))
    return out, int((~ok).sum())


# -- brush arithmetic (the viewer does the same in JavaScript; this is the reference the tests check it against) ---------------------
def falloff(t: np.ndarray, shape: str = "smooth") -> np.ndarray:
    """Weight from normalised distance ``t`` (0 at the centre, 1 at the edge, beyond 1 nothing)."""
    t = np.clip(np.asarray(t, np.float64), 0.0, 1.0)
    if shape == "constant":
        return np.where(t < 1.0, 1.0, 0.0)
    if shape == "linear":
        return 1.0 - t
    return (1.0 - t * t) ** 2


def blend(current: np.ndarray, target, weight: np.ndarray, mode: str = "set") -> np.ndarray:
    """Values after one dab: ``weight`` is falloff times strength per element. ``set`` moves toward ``target``, ``add`` adds
    ``target * weight``, ``multiply`` scales by ``1 + (target - 1) * weight``."""
    cur = np.asarray(current, np.float64)
    w = np.asarray(weight, np.float64).reshape(-1, *([1] * (cur.ndim - 1)))
    t = np.asarray(target, np.float64)
    if mode == "add":
        return cur + t * w
    if mode == "multiply":
        return cur * (1.0 + (t - 1.0) * w)
    return cur + (t - cur) * w
