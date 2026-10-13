"""Persistent SOP flags and portable, non-pickle locked geometry."""
import base64
import hashlib
import io
import json
import zipfile

import numpy as np

from .geometry import Geometry
from .graph import GraphError

NODE_FLAGS = {"bypass", "template", "selectable_template", "unload"}
MAX_LOCK_BYTES = 64 * 1024 * 1024


def freeze_geometry(geo):
    arrays = {"P": geo.P, "vp": geo.topology.vertex_point,
              "offsets": geo.topology.prim_offsets, "closed": geo.topology.closed_mask()}
    names = []
    for domain in ("point", "vertex", "prim"):
        for name, arr in geo.table(domain).items():
            key = f"a{len(names)}"
            names.append([domain, name, key])
            arrays[key] = arr
    # JSON conversion must be lossless; unsupported detail data cannot be locked.
    meta = json.dumps({"names": names, "detail": geo.detail}, allow_nan=False)
    if json.loads(meta)["detail"] != geo.detail:
        raise GraphError("This geometry contains detail values that cannot be saved in a lock")
    arrays["meta"] = np.frombuffer(meta.encode("utf-8"), dtype=np.uint8)
    if sum(a.nbytes for a in arrays.values()) > MAX_LOCK_BYTES:
        raise GraphError("Locked geometry is limited to 64 MiB; use a file cache for larger results")
    buf = io.BytesIO()
    np.savez_compressed(buf, **arrays)
    return base64.b64encode(buf.getvalue()).decode("ascii")


def thaw_geometry(value):
    try:
        if not isinstance(value, str) or len(value) > MAX_LOCK_BYTES * 2:
            raise ValueError("invalid snapshot size")
        data = base64.b64decode(value, validate=True)
        with zipfile.ZipFile(io.BytesIO(data)) as archive:
            if sum(i.file_size for i in archive.infolist()) > MAX_LOCK_BYTES + 1024 * 1024:
                raise ValueError("snapshot exceeds 64 MiB")
        with np.load(io.BytesIO(data), allow_pickle=False) as a:
            g = Geometry.from_arrays(a["P"], a["vp"], a["offsets"], a["closed"])
            meta = json.loads(a["meta"].tobytes().decode("utf-8"))
            for domain, name, key in meta["names"]:
                g.table(domain)[name] = a[key]
            g.detail = meta["detail"]
        g.validate()
        g.freeze()
        return g
    except Exception as exc:
        raise GraphError(f"Invalid locked geometry: {exc}") from exc


def evaluation_graph(graph, feeds):
    """Resolve bypass and hard locks before dependency traversal/cache keys."""
    if not any(n.meta.get("flags", {}).get("bypass") or n.meta.get("locked_geometry") for n in graph.nodes.values()):
        return graph, feeds
    graph, feeds = graph.copy(), dict(feeds or {})
    for nid, n in graph.nodes.items():
        if n.meta.get("flags", {}).get("bypass"):
            nt = graph.registry.get(n.type)
            ref = n.inputs.get(nt.inputs[0].name) if nt.inputs else None
            if n.type == "loop_end":
                begin = graph.nodes.get(n.inputs.get("begin"))
                ref = begin.inputs.get("geo") if begin else None
            if isinstance(ref, list):
                ref = ref[0] if ref else None
            n.type, n.params, n.inputs = "null", {}, {"geo": ref} if ref else {}
            if not ref:
                empty = Geometry.points_only(np.empty((0, 3), np.float32))
                empty.freeze()
                feeds[nid] = ("bypass-empty-v1", empty)
        elif n.meta.get("locked_geometry"):
            blob = n.meta["locked_geometry"]
            feeds[nid] = (hashlib.sha256(blob.encode("ascii")).hexdigest(), thaw_geometry(blob))
    return graph, feeds
