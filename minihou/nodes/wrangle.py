from ..sdk import Str, node
from ..wrangle import compile_snippet


@node("wrangle", inputs=["geo"], category="modify", params=[
    Str("snippet", "@P.z += sin(@P.x)", kind="code",
        doc="VEX-like expression run over points, e.g. '@P.z += sin(@P.x * 2) * 0.5'. See minihou/wrangle.py."),
])
def wrangle(geo, snippet):
    """Run a point wrangle snippet (compiled once per distinct snippet, executed with numpy)."""
    prog = compile_snippet(snippet)
    attrs = {name: arr for name, arr in geo.point.items()}
    written = prog.run(attrs, geo.npoints)
    out = geo.copy()
    for name, arr in written.items():
        out.point[name] = arr
    return out
