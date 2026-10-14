"""Attribute and Group conversion nodes: attribute_from_group and group_from_attribute.
Bridges boolean selection groups and numerical weight/mask attributes.
"""
from __future__ import annotations

import numpy as np

from .. import groups
from ..sdk import Enum, Float, Str, node

DOMAINS = ["point", "prim", "vertex"]
TYPES = ["float", "int"]
OPERATIONS = ["greater", "greater_equal", "less", "less_equal", "equal", "range", "non_zero"]


@node("attribute_from_group", inputs=["geo"], category="attribute", version=1, params=[
    Str("group", "group1", doc="Source group name or pattern"),
    Str("name", "", doc="Destination attribute name (empty uses the group name)"),
    Enum("domain", DOMAINS, default="point", doc="Element domain: point, prim, or vertex"),
    Enum("type", TYPES, default="float", doc="Output attribute type: float or int"),
    Float("value", 1.0, doc="Value assigned to group members"),
    Float("outside_value", 0.0, doc="Value assigned to non-members"),
])
def attribute_from_group(geo, group="group1", name="", domain="point", type="float",
                         value=1.0, outside_value=0.0):
    """Convert a group selection into a numerical attribute (1 for members, 0 for non-members)."""
    dst_name = name.strip() if name.strip() else group.strip()
    if not dst_name:
        return geo

    out = geo.copy()
    mask = groups.mask(geo, domain, group)
    n = {"point": geo.npoints, "prim": geo.nprims, "vertex": geo.nvertices}[domain]

    if mask is None:
        mask = np.zeros(n, dtype=bool)

    if type == "int":
        arr = np.where(mask, int(value), int(outside_value)).astype(np.int32)
    else:
        arr = np.where(mask, float(value), float(outside_value)).astype(np.float32)

    out.table(domain)[dst_name] = arr
    return out


@node("group_from_attribute", inputs=["geo"], category="group", version=1, params=[
    Str("attribute", "weight", doc="Attribute name to evaluate"),
    Str("group", "group1", doc="Group name to create"),
    Enum("domain", DOMAINS, default="point", doc="Element domain: point, prim, or vertex"),
    Enum("operation", OPERATIONS, default="greater", doc="Comparison operation"),
    Float("threshold", 0.5, doc="Threshold value for comparisons"),
    Float("min_value", 0.0, doc="Minimum value for range operation"),
    Float("max_value", 1.0, doc="Maximum value for range operation"),
])
def group_from_attribute(geo, attribute="weight", group="group1", domain="point",
                         operation="greater", threshold=0.5, min_value=0.0, max_value=1.0):
    """Create a group selection by filtering elements based on an attribute threshold or range."""
    grp_name = group.strip()
    attr_name = attribute.strip()
    if not grp_name or not attr_name:
        return geo

    table = geo.table(domain)
    if attr_name not in table:
        raise ValueError(f"no {domain} attribute {attr_name!r}; have {list(table.names())}")

    arr = np.asarray(table[attr_name], dtype=np.float64)
    # If vector attribute, evaluate length or first component
    if arr.ndim > 1:
        if arr.shape[1] == 1:
            arr = arr[:, 0]
        else:
            arr = np.linalg.norm(arr, axis=1)

    if operation == "greater":
        mask = arr > float(threshold)
    elif operation == "greater_equal":
        mask = arr >= float(threshold)
    elif operation == "less":
        mask = arr < float(threshold)
    elif operation == "less_equal":
        mask = arr <= float(threshold)
    elif operation == "equal":
        mask = np.isclose(arr, float(threshold), atol=1e-5)
    elif operation == "range":
        mask = (arr >= float(min_value)) & (arr <= float(max_value))
    elif operation == "non_zero":
        mask = ~np.isclose(arr, 0.0, atol=1e-7)
    else:
        mask = arr > float(threshold)

    return groups.write(geo, domain, grp_name, mask)
