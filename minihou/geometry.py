"""Geometry data model: attribute tables on points, vertices, primitives and detail.

Houdini-style layout:

* points      -- positions (``P``) and any per-point attributes
* vertices    -- one per (primitive, corner); each references a point
* primitives  -- polygons, stored as CSR offsets into the vertex list
* detail      -- global attributes (one value per geometry)

Topology (which points each primitive uses) is kept separate from attribute
data so that "topology unchanged" can be detected and skipped on the wire.

Conventions
-----------
* Floats are ``float32``, integers ``int32``, plus ``bool``. Arrays are C-contiguous
  with shape ``(n,)`` or ``(n, k)``.
* Geometry is **immutable once cooked**. The evaluator calls :meth:`Geometry.freeze`,
  which flips every array to read-only. A node that wants to modify something does
  ``out = geo.copy()`` (shallow: arrays are shared) and then *replaces* arrays.
  Arrays that a node does not touch stay the same Python object, which is what lets
  the link layer skip re-sending them cheaply.
"""
from __future__ import annotations

import hashlib
from typing import Dict, Iterable, Iterator, List, Optional

import numpy as np

DOMAINS = ("point", "vertex", "prim")

_FLOAT = np.dtype(np.float32)
_INT = np.dtype(np.int32)
_BOOL = np.dtype(np.bool_)


def rows(a) -> np.ndarray:
    """``a`` as (n, components): like ``a.reshape(len(a), -1)`` but also right for empty arrays."""
    a = np.asarray(a)
    return a.reshape(a.shape[0], int(np.prod(a.shape[1:], dtype=np.int64)))


def normalize_array(arr, name: str = "<array>") -> np.ndarray:
    """Coerce to a supported dtype (float32/int32/bool) and C-contiguity."""
    a = np.asarray(arr)
    if a.dtype.kind == "f":
        a = a.astype(_FLOAT, copy=False)
    elif a.dtype.kind in "iu":
        a = a.astype(_INT, copy=False)
    elif a.dtype.kind == "b":
        pass
    else:
        raise TypeError(f"attribute {name!r}: unsupported dtype {a.dtype}")
    if a.ndim not in (1, 2):
        raise ValueError(f"attribute {name!r}: expected 1-D or 2-D array, got shape {a.shape}")
    if not a.flags.c_contiguous:
        a = np.ascontiguousarray(a)
    return a


class AttrTable:
    """A named collection of equally long numpy arrays (one domain)."""

    __slots__ = ("size", "_attrs", "_frozen")

    def __init__(self, size: int = 0, attrs: Optional[Dict[str, np.ndarray]] = None):
        self.size = int(size)
        self._attrs: Dict[str, np.ndarray] = {}
        self._frozen = False
        if attrs:
            for k, v in attrs.items():
                self[k] = v

    # -- mapping interface -------------------------------------------------
    def __len__(self) -> int:
        return len(self._attrs)

    def __iter__(self) -> Iterator[str]:
        return iter(self._attrs)

    def __contains__(self, name: str) -> bool:
        return name in self._attrs

    def __getitem__(self, name: str) -> np.ndarray:
        return self._attrs[name]

    def get(self, name: str, default=None):
        return self._attrs.get(name, default)

    def names(self) -> List[str]:
        return list(self._attrs)

    def items(self):
        return self._attrs.items()

    def __setitem__(self, name: str, arr) -> None:
        if self._frozen:
            raise ValueError("table is frozen (cooked geometry is immutable); call geo.copy() first")
        a = normalize_array(arr, name)
        if a.shape[0] != self.size:
            raise ValueError(f"attribute {name!r}: length {a.shape[0]} != table size {self.size}")
        self._attrs[name] = a

    def __delitem__(self, name: str) -> None:
        if self._frozen:
            raise ValueError("table is frozen (cooked geometry is immutable); call geo.copy() first")
        del self._attrs[name]

    # -- misc --------------------------------------------------------------
    def copy(self) -> "AttrTable":
        """Shallow copy: new dict, shared arrays."""
        t = AttrTable(self.size)
        t._attrs = dict(self._attrs)
        return t

    def freeze(self) -> None:
        self._frozen = True
        for a in self._attrs.values():
            a.setflags(write=False)

    @property
    def nbytes(self) -> int:
        return sum(a.nbytes for a in self._attrs.values())

    def __repr__(self) -> str:
        inner = ", ".join(f"{k}:{v.dtype}{list(v.shape[1:]) or ''}" for k, v in self._attrs.items())
        return f"AttrTable(size={self.size}, [{inner}])"


class Topology:
    """Primitive connectivity: vertex->point map plus CSR primitive offsets.

    ``vertex_point[v]`` is the point used by vertex ``v``.
    Primitive ``i`` owns vertices ``prim_offsets[i] : prim_offsets[i+1]``.
    ``closed[i]`` says whether primitive ``i`` is a closed polygon (a face) or an open polyline (a curve, like
    Houdini's open polygons); ``None`` means all closed, which is what most geometry is.
    """

    __slots__ = ("vertex_point", "prim_offsets", "closed", "_digest", "_draw_stats", "_point_prims")

    def __init__(self, vertex_point=None, prim_offsets=None, closed=None):
        vp = np.zeros(0, _INT) if vertex_point is None else np.ascontiguousarray(vertex_point, dtype=_INT)
        po = np.zeros(1, _INT) if prim_offsets is None else np.ascontiguousarray(prim_offsets, dtype=_INT)
        if vp.ndim != 1 or po.ndim != 1:
            raise ValueError("topology arrays must be 1-D")
        self.vertex_point = vp
        self.prim_offsets = po
        if closed is not None:
            closed = np.ascontiguousarray(closed, dtype=_BOOL).reshape(-1)
            if closed.all():
                closed = None                             # canonical: all-closed is stored as None
        self.closed: Optional[np.ndarray] = closed
        self._digest: Optional[str] = None
        self._draw_stats = None                           # proxy.draw_stats, cached once the arrays are frozen
        self._point_prims = None                          # rust.point_prims (the primitives around each point), likewise

    @property
    def has_open(self) -> bool:
        return self.closed is not None

    def closed_mask(self) -> np.ndarray:
        """Bool per primitive: True for closed polygons, False for open polylines."""
        return np.ones(self.nprims, _BOOL) if self.closed is None else self.closed

    @property
    def nvertices(self) -> int:
        return int(self.vertex_point.shape[0])

    @property
    def nprims(self) -> int:
        return int(self.prim_offsets.shape[0]) - 1

    def prim_sizes(self) -> np.ndarray:
        return np.diff(self.prim_offsets)

    def freeze(self) -> None:
        self.vertex_point.setflags(write=False)
        self.prim_offsets.setflags(write=False)
        if self.closed is not None:
            self.closed.setflags(write=False)

    def validate(self, npoints: int) -> None:
        po, vp = self.prim_offsets, self.vertex_point
        if self.closed is not None and self.closed.shape[0] != po.shape[0] - 1:
            raise ValueError(f"closed has {self.closed.shape[0]} entries for {po.shape[0] - 1} primitives")
        if po.shape[0] < 1 or po[0] != 0:
            raise ValueError("prim_offsets must start at 0")
        if po[-1] != vp.shape[0]:
            raise ValueError(f"prim_offsets ends at {po[-1]} but there are {vp.shape[0]} vertices")
        if po.shape[0] > 1 and np.any(np.diff(po) < 0):
            raise ValueError("prim_offsets must be non-decreasing")
        if vp.shape[0] and (vp.min() < 0 or vp.max() >= npoints):
            raise ValueError(f"vertex_point has indices outside [0, {npoints})")

    @property
    def digest(self) -> str:
        """Content digest (cached). Used to decide whether topology changed."""
        if self._digest is None:
            # the arrays' own buffers, not .tobytes() copies (the same bytes, so the same digest); hashlib releases the GIL
            def buf(a):
                return memoryview(np.ascontiguousarray(a)).cast("B")
            h = hashlib.blake2b(digest_size=16)
            h.update(buf(self.vertex_point))
            h.update(b"|")
            h.update(buf(self.prim_offsets))
            if self.closed is not None:
                h.update(b"|open|")
                h.update(buf(self.closed))
            self._digest = h.hexdigest()
        return self._digest

    @classmethod
    def from_polys(cls, polys: Iterable[Iterable[int]], closed=None) -> "Topology":
        vp: List[int] = []
        offs = [0]
        for p in polys:
            vp.extend(p)
            offs.append(len(vp))
        return cls(np.asarray(vp, _INT), np.asarray(offs, _INT), closed)

    def polys(self) -> List[List[int]]:
        """Primitives as lists of point indices (slow; for tests/debug)."""
        o, v = self.prim_offsets, self.vertex_point
        return [v[o[i]:o[i + 1]].tolist() for i in range(self.nprims)]

    @property
    def nbytes(self) -> int:
        return self.vertex_point.nbytes + self.prim_offsets.nbytes + (0 if self.closed is None else self.closed.nbytes)


class Geometry:
    """Points, vertices, primitives and detail attributes."""

    __slots__ = ("point", "vertex", "prim", "detail", "topology", "_frozen", "__weakref__")

    def __init__(self, npoints: int = 0, topology: Optional[Topology] = None):
        self.topology = topology if topology is not None else Topology()
        self.point = AttrTable(npoints)
        self.vertex = AttrTable(self.topology.nvertices)
        self.prim = AttrTable(self.topology.nprims)
        self.detail: Dict[str, object] = {}
        self._frozen = False

    # -- construction ------------------------------------------------------
    @classmethod
    def from_arrays(cls, P, vertex_point=None, prim_offsets=None, closed=None) -> "Geometry":
        """Points, plus primitives as a vertex->point map and CSR offsets; ``closed`` (bool per primitive, default all
        True) marks open polylines (curves) with False."""
        P = normalize_array(P, "P")
        if P.ndim != 2 or P.shape[1] != 3:
            raise ValueError(f"P must have shape (n, 3), got {P.shape}")
        g = cls(P.shape[0], Topology(vertex_point, prim_offsets, closed))
        g.point["P"] = P
        return g

    @classmethod
    def points_only(cls, P) -> "Geometry":
        return cls.from_arrays(P)

    def table(self, domain: str) -> AttrTable:
        if domain not in DOMAINS:
            raise KeyError(f"unknown domain {domain!r}; expected one of {DOMAINS}")
        return getattr(self, domain)

    def set_topology(self, topology: Topology) -> None:
        """Replace topology; vertex/prim tables are reset to match its sizes."""
        if self._frozen:
            raise ValueError("geometry is frozen; call geo.copy() first")
        self.topology = topology
        self.vertex = AttrTable(topology.nvertices)
        self.prim = AttrTable(topology.nprims)

    # -- accessors ---------------------------------------------------------
    @property
    def npoints(self) -> int:
        return self.point.size

    @property
    def nvertices(self) -> int:
        return self.topology.nvertices

    @property
    def nprims(self) -> int:
        return self.topology.nprims

    @property
    def P(self) -> np.ndarray:
        return self.point["P"]

    @P.setter
    def P(self, value) -> None:
        self.point["P"] = value

    # -- lifecycle ---------------------------------------------------------
    def copy(self) -> "Geometry":
        """Shallow copy. Tables are new, arrays are shared. The result is mutable."""
        g = Geometry.__new__(Geometry)
        g.topology = self.topology
        g.point = self.point.copy()
        g.vertex = self.vertex.copy()
        g.prim = self.prim.copy()
        g.detail = dict(self.detail)
        g._frozen = False
        return g

    def freeze(self) -> "Geometry":
        self._frozen = True
        self.topology.freeze()
        for t in (self.point, self.vertex, self.prim):
            t.freeze()
        return self

    @property
    def frozen(self) -> bool:
        return self._frozen

    def validate(self) -> None:
        """Raise ValueError if the geometry is internally inconsistent."""
        if "P" not in self.point:
            raise ValueError("geometry has no point attribute 'P'")
        P = self.point["P"]
        if P.ndim != 2 or P.shape[1] != 3 or P.dtype != _FLOAT:
            raise ValueError(f"P must be float32 (n, 3), got {P.dtype} {P.shape}")
        self.topology.validate(self.npoints)
        if self.vertex.size != self.topology.nvertices or self.prim.size != self.topology.nprims:
            raise ValueError("vertex/prim table sizes do not match topology")
        for dom in DOMAINS:
            t = self.table(dom)
            for name, a in t.items():
                if a.shape[0] != t.size:
                    raise ValueError(f"{dom} attribute {name!r} has length {a.shape[0]}, expected {t.size}")

    @property
    def nbytes(self) -> int:
        return self.point.nbytes + self.vertex.nbytes + self.prim.nbytes + self.topology.nbytes

    def unique_arrays(self) -> List[np.ndarray]:
        out = [self.topology.vertex_point, self.topology.prim_offsets]
        for t in (self.point, self.vertex, self.prim):
            out.extend(t._attrs.values())
        return out

    def __repr__(self) -> str:
        return (f"Geometry(points={self.npoints}, vertices={self.nvertices}, prims={self.nprims}, "
                f"point_attrs={self.point.names()}, vertex_attrs={self.vertex.names()}, "
                f"prim_attrs={self.prim.names()}, detail={list(self.detail)})")
