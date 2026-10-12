import numpy as np

from .. import groups, timectx
from ..geometry import rows
from ..sdk import Enum, Input, Str, node
from ..wrangle import _OTHER_INPUT, WrangleError, compile_snippet

RUN_OVER = {"points": "point", "primitives": "prim", "vertices": "vertex"}


@node("wrangle", inputs=["geo", Input("opinput1", optional=True), Input("opinput2", optional=True),
                         Input("opinput3", optional=True)], category="modify", version=3, params=[
    Str("snippet", "@P.z += sin(@P.x)", kind="code",
        doc="VEX-like expression run over every element, e.g. '@P.z += sin(@P.x * 2) * 0.5'. See the wrangle reference."),
    Enum("run_over", ["points", "primitives", "vertices"],
         doc="Domain the snippet runs over: points, primitives (faces) or vertices (face corners)"),
    Str("group", "", doc="Only change these elements: group names, ^exclusions, numbers/ranges, or an expression like "
                         "'@P.z > 0' (empty: all). Over vertices, a primitive group selects its corners"),
])
def wrangle(geo, opinput1=None, opinput2=None, opinput3=None, *, snippet, run_over, group=""):
    """Attribute wrangle: run a snippet over points, primitives or vertices (compiled once per snippet, numpy speed).
    Reads attributes of other domains (promoted), writes to the domain it runs over; uv, Cd, N, material_index and
    sharp_face arrive in Blender as UV map, color attribute, normals, material index and flat shading. Groups are
    @group_<name> ints: writing i@group_top = @P.z > 1 makes a group. The extra inputs are read with @opinput1_<name>
    (same element number) and @near1_<name> (nearest point)."""
    dom = RUN_OVER[run_over]
    written = run_snippet(geo, snippet, dom, (opinput1, opinput2, opinput3))
    sel = domain_mask(geo, dom, group)
    out = geo.copy()
    for name, arr in written.items():
        if name == "P" and dom != "point":
            raise WrangleError("@P can only be written when running over points")
        if sel is not None:                                 # unselected elements keep their value (new: zero)
            old = geo.table(dom).get(name)
            if old is None or old.shape != arr.shape:
                old = np.zeros_like(arr)
            arr = np.where(sel.reshape((-1,) + (1,) * (arr.ndim - 1)), arr, old.astype(arr.dtype, copy=False))
        for other in ("point", "prim", "vertex"):          # one domain per name (like Blender): writing moves it
            if other != dom and name in out.table(other):
                del out.table(other)[name]
        out.table(dom)[name] = arr
    return out


def run_snippet(geo, snippet: str, dom: str, others=()) -> dict:
    """Run a snippet over a domain and return what it wrote (name -> array); the geometry is not changed. ``others``
    are the geometries of the extra inputs (None where unconnected)."""
    prog = compile_snippet(snippet)
    n = {"point": geo.npoints, "prim": geo.nprims, "vertex": geo.nvertices}[dom]
    ctx = _Domains(geo, others)
    attrs = {name: arr for name in prog.reads | set(prog.writes) if (arr := ctx.read(dom, name)) is not None}
    return prog.run(attrs, n, ctx.specials(dom))


def domain_mask(geo, dom: str, pattern: str):
    """Group pattern -> bool mask on any domain (vertices follow their primitive), or None for all."""
    if dom == "vertex":
        m = groups.mask(geo, "prim", pattern)
        return None if m is None else m[np.repeat(np.arange(geo.nprims), geo.topology.prim_sizes())]
    return groups.mask(geo, dom, pattern)


class _Domains:
    """Reads an attribute as seen from a domain, promoting from the others (lazily: only what a snippet uses)."""

    def __init__(self, geo, others=()):
        self.geo = geo
        self.others = tuple(others)
        self._prim_of_vertex = None
        self._near = {}

    def prim_of_vertex(self) -> np.ndarray:
        if self._prim_of_vertex is None:
            self._prim_of_vertex = np.repeat(np.arange(self.geo.nprims, dtype=np.int32), self.geo.topology.prim_sizes())
        return self._prim_of_vertex

    def per_prim_mean(self, vertex_values: np.ndarray) -> np.ndarray:
        """Average of vertex-ordered values over each primitive (bincount: fast, and empty prims give 0)."""
        g = self.geo
        idx = self.prim_of_vertex()
        sizes = np.maximum(g.topology.prim_sizes(), 1).astype(np.float64)
        v = rows(vertex_values).astype(np.float64)
        cols = [np.bincount(idx, weights=v[:, c], minlength=g.nprims) / sizes for c in range(v.shape[1])]
        out = np.stack(cols, 1).astype(np.float32)
        return out[:, 0] if vertex_values.ndim == 1 else out

    def _other(self, k: int, what: str):
        o = self.others[k - 1] if k <= len(self.others) else None
        if o is None:
            raise WrangleError(f"@{what} reads input {k}, which is not connected")
        return o

    def _read_other(self, dom: str, kind: str, k: int, name: str):
        n = {"point": self.geo.npoints, "prim": self.geo.nprims, "vertex": self.geo.nvertices}[dom]
        if kind == "opinput":
            o = self._other(k, f"opinput{k}_{name}")
            a = _Domains(o).read(dom, name)
            if a is None:
                raise WrangleError(f"input {k} has no attribute @{name} (over {dom}s)")
            if len(a) >= n:
                return a[:n]
            pad = np.zeros((n - len(a),) + a.shape[1:], a.dtype)             # beyond its end: zero, as in VEX
            return np.concatenate([a, pad])
        o = self._other(k, f"near{k}_{name}")
        if o.npoints == 0:
            raise WrangleError(f"input {k} has no points to be near")
        if k not in self._near:
            from .. import spatial
            idx, dist = spatial.nearest(o.P.astype(np.float64), self.read(dom, "P").astype(np.float64))
            self._near[k] = (idx.astype(np.int32), dist.astype(np.float32))
        idx, dist = self._near[k]
        if name == "dist":
            return dist
        if name == "ptnum":
            return idx
        if name not in o.point:
            raise WrangleError(f"input {k} has no point attribute @{name}")
        return o.point[name][idx]

    def read(self, dom: str, name: str):
        g = self.geo
        m = _OTHER_INPUT.match(name)
        if m:
            return self._read_other(dom, m.group(1), int(m.group(2)), m.group(3))
        if name in g.table(dom):
            return g.table(dom)[name]
        if dom == "vertex":
            if name in g.point:
                return g.point[name][g.topology.vertex_point]
            if name in g.prim:
                return g.prim[name][self.prim_of_vertex()]
        elif dom == "prim":
            if name in g.point:
                return self.per_prim_mean(g.point[name][g.topology.vertex_point])
            if name in g.vertex:
                return self.per_prim_mean(g.vertex[name])
        if name in g.detail:                                   # detail attributes read as constants (VEX's detail())
            v = g.detail[name]
            try:
                a = np.asarray(v, dtype=np.float64)
            except (TypeError, ValueError):
                return None
            if a.ndim > 1 or a.size > 4 or not np.isfinite(a).all():
                return None
            n = {"point": g.npoints, "prim": g.nprims, "vertex": g.nvertices}[dom]
            kind = np.int32 if isinstance(v, (int, bool, np.integer)) else np.float32
            return np.repeat(a.reshape(1, -1), n, axis=0).astype(kind).reshape((n,) if a.ndim == 0 else (n, a.size))
        return None

    def specials(self, dom: str) -> dict:
        g = self.geo
        col = lambda a: np.asarray(a, np.int32).reshape(-1, 1)  # noqa: E731
        s = {"numpt": np.int32(g.npoints), "numprim": np.int32(g.nprims), "numvtx": np.int32(g.nvertices),
             "Frame": np.float32(timectx.frame()), "Time": np.float32(timectx.seconds()),
             "TimeInc": np.float32(timectx.time_inc())}
        if dom == "point":
            s["ptnum"] = col(np.arange(g.npoints))
        elif dom == "prim":
            s["primnum"] = col(np.arange(g.nprims))
            s["closed"] = col(g.topology.closed_mask())
        else:
            s["vtxnum"] = col(np.arange(g.nvertices))
            s["ptnum"] = col(g.topology.vertex_point)
            s["primnum"] = col(self.prim_of_vertex())
            s["closed"] = col(g.topology.closed_mask()[self.prim_of_vertex()])
        return s
