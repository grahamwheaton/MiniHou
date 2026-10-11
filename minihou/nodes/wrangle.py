import numpy as np

from .. import groups
from ..sdk import Enum, Str, node
from ..wrangle import WrangleError, compile_snippet

RUN_OVER = {"points": "point", "primitives": "prim", "vertices": "vertex"}


@node("wrangle", inputs=["geo"], category="modify", version=3, params=[
    Str("snippet", "@P.z += sin(@P.x)", kind="code",
        doc="VEX-like expression run over every element, e.g. '@P.z += sin(@P.x * 2) * 0.5'. See the wrangle reference."),
    Enum("run_over", ["points", "primitives", "vertices"],
         doc="Domain the snippet runs over: points, primitives (faces) or vertices (face corners)"),
    Str("group", "", doc="Only change these elements: group names, ^exclusions, numbers/ranges, or an expression like "
                         "'@P.z > 0' (empty: all). Over vertices, a primitive group selects its corners"),
])
def wrangle(geo, snippet, run_over, group=""):
    """Attribute wrangle: run a snippet over points, primitives or vertices (compiled once per snippet, numpy speed).
    Reads attributes of other domains (promoted), writes to the domain it runs over; uv, Cd, N, material_index and
    sharp_face arrive in Blender as UV map, color attribute, normals, material index and flat shading. Groups are
    @group_<name> ints: writing i@group_top = @P.z > 1 makes a group."""
    dom = RUN_OVER[run_over]
    written = run_snippet(geo, snippet, dom)
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


def run_snippet(geo, snippet: str, dom: str) -> dict:
    """Run a snippet over a domain and return what it wrote (name -> array); the geometry is not changed."""
    prog = compile_snippet(snippet)
    n = {"point": geo.npoints, "prim": geo.nprims, "vertex": geo.nvertices}[dom]
    ctx = _Domains(geo)
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

    def __init__(self, geo):
        self.geo = geo
        self._prim_of_vertex = None

    def prim_of_vertex(self) -> np.ndarray:
        if self._prim_of_vertex is None:
            self._prim_of_vertex = np.repeat(np.arange(self.geo.nprims, dtype=np.int32), self.geo.topology.prim_sizes())
        return self._prim_of_vertex

    def per_prim_mean(self, vertex_values: np.ndarray) -> np.ndarray:
        """Average of vertex-ordered values over each primitive (bincount: fast, and empty prims give 0)."""
        g = self.geo
        idx = self.prim_of_vertex()
        sizes = np.maximum(g.topology.prim_sizes(), 1).astype(np.float64)
        v = vertex_values.reshape(len(vertex_values), -1).astype(np.float64)
        cols = [np.bincount(idx, weights=v[:, c], minlength=g.nprims) / sizes for c in range(v.shape[1])]
        out = np.stack(cols, 1).astype(np.float32)
        return out[:, 0] if vertex_values.ndim == 1 else out

    def read(self, dom: str, name: str):
        g = self.geo
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
        return None

    def specials(self, dom: str) -> dict:
        g = self.geo
        col = lambda a: np.asarray(a, np.int32).reshape(-1, 1)  # noqa: E731
        s = {"numpt": np.int32(g.npoints), "numprim": np.int32(g.nprims), "numvtx": np.int32(g.nvertices)}
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
