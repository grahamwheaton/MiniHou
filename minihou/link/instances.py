"""Connection-owned modifier graph instances, isolated from the browser/proxy session."""
from __future__ import annotations

import hashlib
import json
import math

import numpy as np
import threading
import uuid
from .live import LiveModifier, input_token, object_feeds
from pathlib import Path

from ..evaluator import Cache, Evaluator
from ..graph import Graph
from ..inputs import without_object_inputs
from .diff import DiffState


class InstanceManager:
    """One manager per connection: a lost Blender connection releases every cached mesh.

    Each request supplies all parameter overrides, so graph defaults never inherit values
    from another modifier (or an earlier request with a removed override).
    """
    def __init__(self, session):
        self.session = session
        self.instances = {}
        self.documents = {}
        self.sources = {}
        self.owner_id = uuid.uuid4().hex
        self.lock = threading.RLock()

    def clear(self):
        with self.lock:
            for evaluator in self.instances.values():
                evaluator.close()
            self.instances.clear()
            for doc in self.documents.values():
                self.session.disconnect_modifier(doc)
            self.documents.clear()
            self.sources.clear()

    def remove(self, instance_id):
        with self.lock:
            self.sources.pop(instance_id, None)
            doc = self.documents.pop(instance_id, None)
            if doc is not None:
                self.session.disconnect_modifier(doc)
            evaluator = self.instances.pop(instance_id, None)
            if evaluator is not None:
                evaluator.close()

    def _graph(self, h):
        data = h.get("graph")
        if data is None:
            path = h.get("graph_path")
            if not path:
                raise ValueError("choose a graph file or supply a saved graph")
            path = Path(path).resolve()
            if path.suffix.lower() != ".json":
                raise ValueError("modifier graph must be a .json file")
            # Browser edits may not yet have reached the debounced disk writer.
            if self.session.path and path == self.session.path.resolve():
                data = self.session.snapshot()["graph"]
            else:
                data = json.loads(path.read_text(encoding="utf-8"))
        graph = Graph.from_dict(data, registry=self.session.evaluator.registry)
        # Named Blender objects can feed evaluated output back into this stack.
        # Fail explicitly, instead of consulting the global legacy INPUTS store.
        def check(value):
            if isinstance(value, dict):
                if value.get("type") == "blender_object":
                    raise ValueError("modifier graphs cannot read blender_object; use modifier_input to prevent feedback loops")
                for item in value.values():
                    check(item)
            elif isinstance(value, list):
                for item in value:
                    check(item)
        check(data)
        if not any(graph.nodes[nid].type == "modifier_input"
                   for nid in graph.upstream_order(graph.output_node())):
            raise ValueError("modifier graphs require a top-level modifier_input node; replacement-only generators are not stack modifiers")
        return graph

    def set_sources(self, ident, sources):
        if not isinstance(ident, str) or not ident or len(ident) > 256:
            raise ValueError("invalid modifier identity")
        if len(sources) > 64 or (ident not in self.sources and len(self.sources) >= 256):
            raise ValueError("too many object inputs")
        for geo in sources.values():
            geo.validate()
            if not np.isfinite(geo.P).all():
                raise ValueError("source point positions must be finite")
        with self.lock, self.session.lock:
            self.sources[ident] = {k: g.freeze() for k, g in sources.items()}
            doc = self.documents.get(ident)
            if doc is not None:
                doc.sources = self.sources[ident]
                if self.session.live_modifier is doc:
                    self.session._rev += 1
                    self.session._wake.set()
        return {"type": "instance_sources_set"}

    def edit_live(self, h, geometry):
        ident = h.get("instance_id")
        if not isinstance(ident, str) or not ident or len(ident) > 256:
            raise ValueError("invalid modifier identity")
        frame, fps = float(h.get("frame", 1)), float(h.get("fps", 24))
        if not math.isfinite(frame) or not math.isfinite(fps) or fps <= 0:
            raise ValueError("frame and fps must be finite, and fps must be positive")
        graph = self._graph(h)
        for name, value in (h.get("params") or {}).items():
            graph.set_exposed(name, value)
        if not np.isfinite(geometry.P).all():
            raise ValueError("input point positions must be finite")
        with self.lock:
            doc = self.documents.get(ident)
            if doc is None:
                if len(self.documents) >= 256:
                    raise ValueError("maximum 256 live modifier documents per connection")
                root = getattr(self.session, "editor_root", None)
                if root is None:
                    raise ValueError("Start the HTTP editor before editing a modifier")
                key = hashlib.sha256((self.owner_id + ident).encode()).hexdigest()[:24]
                doc = LiveModifier(ident, str(h.get("label") or "Blender modifier")[:160], graph,
                                   root / "graphs" / ("Blender-" + key + ".json"))
                self.documents[ident] = doc
            self.session.modifier_input(doc, geometry, input_token(geometry), frame, fps)
            doc.sources = self.sources.get(ident, {})
            self.session.bind_modifier(doc)
            return self.sync_live(h)

    def sync_live(self, h):
        with self.lock, self.session.lock:
            doc = self.documents.get(h.get("instance_id"))
            if doc is None:
                return {"type": "instance_synced", "live": False}
            # Only explicitly changed Blender controls override browser edits.
            incoming = h.get("params") or {}
            graph = doc.graph.copy()
            changed = False
            for name, value in incoming.items():
                if name in doc.sent_params and not _same_parameter(value, doc.sent_params[name]):
                    if any(p["name"] == name for p in graph.interface()):
                        graph.set_exposed(name, value)
                        changed = True
            if changed:
                doc.graph = graph
                if self.session.live_modifier is doc:
                    self.session.replace_graph(graph, origin="modifier", save=True)
            interface = doc.graph.interface()
            doc.sent_params = {p["name"]: p["value"] for p in interface}
            return {"type": "instance_synced", "live": True, "graph": doc.graph.to_dict(),
                    "interface": interface, "path": str(doc.path)}

    def describe(self, h):
        graph = self._graph(h)
        return {"type": "instance_description", "interface": graph.interface(), "graph": graph.to_dict()}

    def cook(self, h, geometry):
        instance_id = h.get("instance_id")
        if not isinstance(instance_id, str) or not instance_id or len(instance_id) > 256:
            raise ValueError("instance_id must be a non-empty string up to 256 characters")
        frame, fps = float(h.get("frame", 1)), float(h.get("fps", 24))
        if not math.isfinite(frame) or not math.isfinite(fps) or fps <= 0:
            raise ValueError("frame and fps must be finite, and fps must be positive")
        geometry.validate()
        if not np.isfinite(geometry.P).all():
            raise ValueError("input point positions must be finite")
        with self.lock:
            doc = self.documents.get(instance_id)
            if doc is not None:
                self.session.modifier_input(doc, geometry, input_token(geometry), frame, fps)
            graph = self._graph(h)
            params = h.get("params") or {}
            if not isinstance(params, dict):
                raise ValueError("params must be an object of exposed parameter values")
            for name, value in params.items():
                graph.set_exposed(name, value)
            evaluator = self.instances.get(instance_id)
            if evaluator is None:
                if len(self.instances) >= 256:
                    raise ValueError("maximum 256 modifier instances per connection")
                evaluator = Evaluator(graph.registry, Cache(16 * 1024 * 1024))
                self.instances[instance_id] = evaluator
            # Content hash keeps stable input cache entries reusable while topology and
            # every attribute participate in invalidation, not just point positions.
            digest = hashlib.blake2b(digest_size=16)
            _, input_arrays = DiffState().build(geometry, 0)
            for key, array in sorted(input_arrays.items()):
                digest.update(key.encode())
                digest.update(str((array.dtype.str, array.shape)).encode())
                digest.update(array.tobytes())
            digest.update(repr(geometry.detail.get("materials")).encode())    # same faces, other material names: another input
            token = digest.hexdigest()
            feeds = {nid: (token, geometry.freeze()) for nid, node in graph.nodes.items()
                     if node.type == "modifier_input"}
            feeds.update(object_feeds(graph, self.sources.get(instance_id, {})))
            with without_object_inputs():
                result = evaluator.cook(graph, graph.render_node() if h.get("render") else None, feeds=feeds, frame=frame, fps=fps)
                scene, scene_errors, scene_arrays = self._scene(graph, evaluator, feeds, frame, fps)
            result.validate()
            if not np.isfinite(result.P).all():
                raise ValueError("cooked point positions must be finite")
            header, arrays = DiffState().build(result, 1)
            arrays.update(scene_arrays)
            header.update(type="instance_result", interface=graph.interface())
            if scene or scene_errors:
                header["scene"] = scene                            # real objects for the add-on to realise (see scene_objects)
                header["scene_errors"] = scene_errors
            return header, arrays

    @staticmethod
    def _scene(graph, evaluator, feeds, frame, fps):
        """Manifests of the graph's ``scene_objects`` nodes, cooked with the same inputs as the modifier's result."""
        from ..evaluator import NodeError
        from ..nodes.scene_objects import build_manifest
        out, errors, arrays = [], [], {}
        for nid in graph.scene_sinks():
            try:
                geo = evaluator.cook(graph, nid, feeds=feeds, frame=frame, fps=fps)
                header, extra = build_manifest(nid, graph.effective_params(nid), geo, evaluator.last_stats.key)
                out.append(header)
                arrays.update({f"scene:{nid}:{k}": v for k, v in extra.items()})     # prefixed: the reply's own arrays are the mesh
            except (NodeError, ValueError) as e:                     # a bad sink must not take the modifier's own result down
                errors.append({"node": nid, "message": str(e)})
        return out, errors, arrays

def _same_parameter(a, b):
    """Blender stores scalar/vector controls as float32; ignore transport rounding."""
    import struct
    if isinstance(a, (list, tuple)) and isinstance(b, (list, tuple)):
        return len(a) == len(b) and all(_same_parameter(x, y) for x, y in zip(a, b))
    if isinstance(a, float) or isinstance(b, float):
        try:
            return struct.pack("f", a) == struct.pack("f", b)
        except (OverflowError, TypeError):
            return a == b
    return a == b
