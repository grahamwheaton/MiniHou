"""Connection-owned live modifier documents; Blender data stays in the engine process."""
from dataclasses import dataclass, field
import hashlib
from .diff import DiffState


def input_token(geometry):
    geometry.validate()
    digest = hashlib.blake2b(digest_size=16)
    _, arrays = DiffState().build(geometry, 0)
    for key, array in sorted(arrays.items()):
        digest.update(key.encode())
        digest.update(str((array.dtype.str, array.shape)).encode())
        digest.update(array.tobytes())
    return digest.hexdigest()


@dataclass
class LiveModifier:
    instance_id: str
    label: str
    graph: object
    path: object
    geometry: object = None
    frame: float = 1.0
    fps: float = 24.0
    token: str = ""
    sent_params: dict = field(default_factory=dict)
    connected: bool = True
    sources: dict = field(default_factory=dict)


def object_feeds(graph, sources):
    return {nid: (input_token(sources[key]) + str(sources[key].detail.get("object_name", "")), sources[key])
            for nid, n in graph.nodes.items() if n.type == "object_input"
            for key in [n.params.get("binding") or nid] if key in sources}


def preview_instances(geo, sources):
    """Realize only the editor preview; Blender still receives native instance points."""
    import numpy as np
    from ..geometry import Geometry
    from ..nodes.copy_to_points import copy_to_points
    from ..nodes.delete import keep_points
    from ..nodes.merge import merge
    names = geo.detail.get("instance_objects", [])
    ids = geo.point.get("instance")
    if not names or ids is None:
        return geo
    ids = np.asarray(ids).reshape(-1)
    parts = [keep_points(geo, ids == 0)]
    estimate = sum(int(np.count_nonzero(ids == i)) * sources[name.lstrip("@")].npoints
                   for i, name in enumerate(names, 1) if name.lstrip("@") in sources)
    if estimate > 2_000_000:
        raise ValueError("Preview exceeds 2 million vertices; reduce scatter count to preview the source meshes")
    for i, name in enumerate(names, 1):
        mask = ids == i
        if not mask.any():
            continue
        source = sources.get(name.lstrip("@"))
        if source is None:
            raise ValueError(f"Choose the Blender object for {name} to preview its geometry")
        points = Geometry.points_only(geo.P[mask])
        for key in ("rot", "scale"):
            if key in geo.point:
                points.point[key] = geo.point[key][mask]
        parts.append(copy_to_points(source, points, True, False))
    return merge(parts)
