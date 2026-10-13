"""Give faces (or points) a material by name (see :mod:`minihou.materials`)."""
import numpy as np

from .. import groups
from ..materials import assign, assign_picks, clean
from ..sdk import Enum, Str, node


@node("set_material", inputs=["geo"], category="attribute", params=[
    Enum("domain", ["primitives", "points"], doc="primitives: faces get the material (Blender's material slots, USD's bindings); points: "
                                                 "the points do, which Scene Objects reads to give each instance its own material"),
    Str("group", "", doc="Which ones: group names, ranges or '@area > 0.1' (empty: all of them)"),
    Str("material", "", doc="Material name. In Blender the object's slot is filled with the material of that name (made if "
                            "missing); in USD it becomes a material binding. Empty: back to whatever the target has"),
    Str("names", "", doc="Several names, separated by commas: with 'index_attrib', element number n gets the name at position "
                         "n (counting from 0, wrapping around). Used instead of 'material'"),
    Str("index_attrib", "", doc="An integer attribute on the same domain (a piece or id number, or one made by attribute_randomize) "
                                "that picks from 'names'. Empty: every element gets 'material'"),
])
def set_material(geo, domain, group, material, names, index_attrib):
    """Assign materials by name (Houdini's Material SOP). Faces keep their `material_index`; the name list that gives the numbers
    their meaning travels with the geometry, through merges and booleans, into Blender's material slots and USD's material
    bindings. To give every piece of a fracture its own material, set `index_attrib` to the piece attribute and list names; to give
    Scene Objects instances their own, use the points domain."""
    dom = "prim" if domain == "primitives" else "point"
    mask = groups.mask(geo, dom, group)
    if mask is not None and not mask.any():
        return geo
    if not str(index_attrib).strip():
        return assign(geo, mask, clean(material), dom)
    choices = [clean(x) for x in str(names).split(",") if x.strip()]
    if not choices:
        raise ValueError("list the material names in 'names' (separated by commas) for 'index_attrib' to pick from")
    a = (geo.prim if dom == "prim" else geo.point).get(str(index_attrib).strip())
    if a is None:
        raise ValueError(f"no {'primitive' if dom == 'prim' else 'point'} attribute {str(index_attrib).strip()!r}")
    if a.ndim != 1 or a.dtype.kind not in "iub":
        raise ValueError(f"{str(index_attrib).strip()!r} must be an integer attribute (one value per element) to pick a material")
    return assign_picks(geo, mask, np.asarray(a).astype(np.int64) % len(choices), choices, dom)
