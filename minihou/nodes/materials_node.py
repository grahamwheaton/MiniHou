"""Give faces a material by name (see :mod:`minihou.materials`)."""
from .. import groups
from ..materials import assign, clean
from ..sdk import Str, node


@node("set_material", inputs=["geo"], category="attribute", params=[
    Str("group", "", doc="Faces to give the material: group names, ranges or '@area > 0.1' (empty: every face)"),
    Str("material", "", doc="Material name. In Blender the object's slot is filled with the material of that name (made if "
                            "missing); in USD it becomes a material binding. Empty: faces go back to whatever the target has"),
])
def set_material(geo, group, material):
    """Assign a material by name to a group of faces (Houdini's Material SOP). The faces keep their `material_index`; the
    name list that gives the numbers their meaning travels with the geometry, through merges and booleans, into
    Blender's material slots and USD's material bindings."""
    mask = groups.mask(geo, "prim", group)
    if mask is not None and not mask.any():
        return geo
    return assign(geo, mask, clean(material))
