"""For-each: run a node (often a system) on every piece and merge the results (Houdini's For-Each block)."""
import json

import numpy as np

from .. import proxy
from ..sdk import REGISTRY, Enum, Int, Str, node
from .attrib_tools import piece_labels
from .group import keep_prims
from .merge import merge

MAX_PIECES = 10000


def _body_key(params) -> str:
    """The body's code and version (and time, if it uses time) belong in for_each's cache key: editing a system or a
    library node used as the body must recook the loop."""
    nt = REGISTRY.get(params["body"]) if params.get("body") in REGISTRY else None
    if nt is None:
        return "missing"
    extra = nt.key_extra(nt.coerce_params({})) if nt.key_extra else ""
    return f"{nt.name}:{nt.version}:{nt.code_hash}:{extra}"


@node("for_each", inputs=["geo"], category="combine", key_extra=_body_key, params=[
    Str("body", "", doc="Node type to run on each piece: a system (a graph in nodes/) or any node with a geometry input"),
    Str("params", "{}", doc='Parameters for the body as JSON, e.g. {"distance": 0.3}; omitted ones use defaults'),
    Enum("piece", ["connectivity", "attribute", "primitive"],
         doc="connectivity: each connected piece; attribute: primitives sharing a value of 'attribute'; "
             "primitive: every primitive alone"),
    Str("attribute", "class", doc="piece = attribute: a primitive attribute (e.g. from connectivity or a wrangle)"),
    Int("max_pieces", 1000, min=1, max=MAX_PIECES, doc="Safety limit"),
])
def for_each(geo, body, params, piece, attribute, max_pieces):
    """Split the geometry into pieces, run 'body' on each, merge the results. Each piece carries the detail
    attributes iteration (0, 1, ...), numiterations and value (the piece's attribute value), which a wrangle inside a
    system reads as @iteration etc. -- e.g. to vary a seed per piece."""
    if not body:
        raise ValueError("set 'body' to a node type, e.g. a system name or 'polyextrude'")
    if body not in REGISTRY:
        raise ValueError(f"no node type {body!r}")
    nt = REGISTRY.get(body)
    geo_inputs = [i for i in nt.inputs]
    if not geo_inputs:
        raise ValueError(f"{body!r} has no geometry input to feed pieces into")
    try:
        overrides = json.loads(params or "{}")
    except ValueError as e:
        raise ValueError(f"params must be JSON: {e}") from None
    if not isinstance(overrides, dict):
        raise ValueError("params must be a JSON object, e.g. {\"distance\": 0.3}")
    values = proxy.scale_params(nt, nt.coerce_params(overrides))
    if piece == "connectivity":
        _, labels = piece_labels(geo)
    elif piece == "primitive":
        labels = np.arange(geo.nprims)
    else:
        a = geo.prim.get(attribute)
        if a is None:
            raise ValueError(f"no primitive attribute {attribute!r} (connectivity makes 'class')")
        labels = np.asarray(a).reshape(geo.nprims, -1)[:, 0]
    keys, inv = np.unique(labels, return_inverse=True)
    inv = inv.reshape(-1)
    if len(keys) > max_pieces:
        raise ValueError(f"{len(keys)} pieces is more than max_pieces ({max_pieces}); raise it if you mean it")
    outs = []
    for i, k in enumerate(keys.tolist()):
        part = keep_prims(geo, inv == i)
        part.detail["iteration"] = i
        part.detail["numiterations"] = int(len(keys))
        part.detail["value"] = k
        args = [part] + [None] * (len(geo_inputs) - 1)
        res = nt.fn(*args, **values)
        if res is not None:
            outs.append(res)
    if not outs:
        return geo.copy() if geo.nprims == 0 else keep_prims(geo, np.zeros(geo.nprims, bool))
    out = merge(outs) if len(outs) > 1 else outs[0].copy()
    for k in ("iteration", "numiterations", "value"):
        out.detail.pop(k, None)
    return out
