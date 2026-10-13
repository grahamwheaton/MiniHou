"""Painted attributes (see :mod:`minihou.paint`): the node the viewer's Paint mode, the Quick Assign panel and the spreadsheet write to."""
from .. import paint
from ..sdk import Enum, Float, Str, Vec3, node


@node("attrib_paint", inputs=["geo"], category="attribute", params=[
    Str("attribute", "paint", doc="Name of the attribute to paint (made if the input has none; an edge attribute is an edge group of this name)"),
    Enum("domain", list(paint.DOMAINS), doc="point; prim (faces and other primitives); edge (the derived edges: painted as membership of an edge group)"),
    Enum("kind", list(paint.KINDS), doc="float, int or vec3 (a colour or a direction)"),
    Float("default", 0.0, soft_min=-1.0, soft_max=1.0, doc="float / int: the value of elements that were not painted and the input has no value for"),
    Vec3("default_vec", (0.0, 0.0, 0.0), doc="vec3: the same for a vector attribute"),
    Str("data", "", kind="hidden", doc="What was painted: element numbers and values (written by the viewer's Paint mode, Quick Assign and the spreadsheet)"),
])
def attrib_paint(geo, attribute, domain, kind, default, default_vec, data):
    """Hold painted values of one attribute on points, primitives (faces) or edges. Elements nobody painted keep the input's value of that
    attribute, or the default. The painted elements are remembered by number, so they follow the geometry only while the elements upstream
    keep their numbers; ones that no longer exist are ignored and counted in the detail attribute `paint_stale_<name>`. Paint it in
    the 3D view (Paint mode), assign to a selection with Quick Assign, or edit cells in the spreadsheet."""
    out, stale = paint.apply(geo, domain, attribute, kind, default_vec if kind == "vec3" else default, data)
    if stale:
        out.detail["paint_stale_" + attribute] = int(stale)
    return out
