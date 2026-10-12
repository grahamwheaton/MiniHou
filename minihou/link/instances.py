"""Connection-owned modifier graph instances, isolated from the browser/proxy session."""
from __future__ import annotations

import hashlib
import json
import math

import numpy as np
import threading
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
        self.lock = threading.RLock()

    def clear(self):
        with self.lock:
            for evaluator in self.instances.values():
                evaluator.close()
            self.instances.clear()

    def remove(self, instance_id):
        with self.lock:
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
            token = digest.hexdigest()
            feeds = {nid: (token, geometry.freeze()) for nid, node in graph.nodes.items()
                     if node.type == "modifier_input"}
            with without_object_inputs():
                result = evaluator.cook(graph, feeds=feeds, frame=frame, fps=fps)
                scene, scene_errors = self._scene(graph, evaluator, feeds, frame, fps)
            result.validate()
            if not np.isfinite(result.P).all():
                raise ValueError("cooked point positions must be finite")
            header, arrays = DiffState().build(result, 1)
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
        out, errors = [], []
        for nid in graph.scene_sinks():
            try:
                geo = evaluator.cook(graph, nid, feeds=feeds, frame=frame, fps=fps)
                out.append(build_manifest(nid, graph.effective_params(nid), geo, evaluator.last_stats.key))
            except (NodeError, ValueError) as e:                     # a bad sink must not take the modifier's own result down
                errors.append({"node": nid, "message": str(e)})
        return out, errors
