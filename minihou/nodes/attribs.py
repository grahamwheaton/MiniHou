"""Attribute housekeeping: delete and rename (Houdini's Attribute Delete / Attribute Rename)."""
import fnmatch

from ..sdk import Bool, Enum, Input, Str, node

DOMAINS = ("point", "vertex", "prim")


def _patterns(text):
    return [p for p in text.replace(",", " ").split() if p]


@node("attribute_delete", inputs=["geo", Input("reference", optional=True,
      doc="Delete attributes present on this geometry, matched separately on each domain")], category="attribute", version=2, params=[
    Str("names", "", doc="Attributes to delete, space separated; wildcards work (e.g. 'N uv tmp_*'). P is never deleted"),
    Enum("domain", ["all", "point", "vertex", "prim", "detail"], doc="Attribute class to delete"),
    Bool("invert", False, doc="Keep matching attributes and delete the others; P is always kept"),
])
def attribute_delete(geo, reference=None, names="", domain="all", invert=False):
    """Delete attributes by name or wildcard on every domain (also detail). Useful to stop an attribute reaching Blender,
    e.g. delete N to let Blender compute its own normals."""
    if isinstance(reference, str):  # compatibility with direct Python calls before the second port existed
        names, reference = reference, None
    pats = _patterns(names)
    out = geo.copy()
    for dom in DOMAINS:
        if domain not in ("all", dom):
            continue
        table = out.table(dom)
        for name in list(table.names()):
            match = any(fnmatch.fnmatchcase(name, p) for p in pats) or (reference is not None and name in reference.table(dom))
            if name != "P" and match != invert:
                del table[name]
    if domain in ("all", "detail"):
        for name in list(out.detail):
            match = any(fnmatch.fnmatchcase(name, p) for p in pats) or (reference is not None and name in reference.detail)
            if match != invert:
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
