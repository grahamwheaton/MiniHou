"""Attribute housekeeping: delete and rename (Houdini's Attribute Delete / Attribute Rename)."""
import fnmatch

from ..sdk import Str, node

DOMAINS = ("point", "vertex", "prim")


def _patterns(text):
    return [p for p in text.replace(",", " ").split() if p]


@node("attribute_delete", inputs=["geo"], category="attribute", params=[
    Str("names", "", doc="Attributes to delete, space separated; wildcards work (e.g. 'N uv tmp_*'). P is never deleted"),
])
def attribute_delete(geo, names):
    """Delete attributes by name or wildcard on every domain (also detail). Useful to stop an attribute reaching Blender,
    e.g. delete N to let Blender compute its own normals."""
    pats = _patterns(names)
    out = geo.copy()
    for dom in DOMAINS:
        table = out.table(dom)
        for name in list(table.names()):
            if name != "P" and any(fnmatch.fnmatchcase(name, p) for p in pats):
                del table[name]
    for name in list(out.detail):
        if any(fnmatch.fnmatchcase(name, p) for p in pats):
            del out.detail[name]
    return out


@node("attribute_rename", inputs=["geo"], category="attribute", params=[
    Str("from_name", "", doc="Existing attribute (any domain)"),
    Str("to_name", "", doc="New name; replaces an attribute of that name on any domain"),
])
def attribute_rename(geo, from_name, to_name):
    """Rename an attribute, e.g. a Blender color attribute 'Col' to 'Cd', or 'weight' to 'pscale'."""
    if not from_name or not to_name or from_name == to_name:
        return geo
    if "P" in (from_name, to_name):
        raise ValueError("P cannot be renamed")
    src = next((d for d in DOMAINS if from_name in geo.table(d)), None)
    if src is None:
        have = sorted({n for d in DOMAINS for n in geo.table(d).names()})
        raise ValueError(f"no attribute {from_name!r}; have {have}")
    out = geo.copy()
    arr = out.table(src)[from_name]
    for d in DOMAINS:
        if to_name in out.table(d):
            del out.table(d)[to_name]
    del out.table(src)[from_name]
    out.table(src)[to_name] = arr
    return out
