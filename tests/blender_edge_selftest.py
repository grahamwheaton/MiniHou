"""Run with Blender --background --python tests/blender_edge_selftest.py."""
import sys
from pathlib import Path

import bpy
import numpy as np

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "blender_addon"))
from minihou_link import mesh_reader, mesh_writer
from minihou_link.protocol import GeoUpdate, Mirror

source = bpy.data.meshes.new("edge-source")
source.from_pydata([(0, 0, 0), (1, 0, 0), (1, 1, 0), (0, 1, 0)], [], [(0, 1, 2, 3)])
source.attributes.new("crease_edge", "FLOAT", "EDGE")
source.attributes.new("bevel_weight_edge", "FLOAT", "EDGE")
source.attributes.new("sharp_edge", "BOOLEAN", "EDGE")
source.attributes.new("custom_int", "INT", "EDGE")
source.attributes.new("custom_int2", "INT32_2D", "EDGE")
expected = {}
for edge in source.edges:
    pair = tuple(sorted(edge.vertices))
    expected[pair] = (float(edge.index + 1) / 10, edge.index % 2, edge.index + 10)
    source.attributes["crease_edge"].data[edge.index].value = expected[pair][0]
    source.attributes["sharp_edge"].data[edge.index].value = bool(expected[pair][1])
    source.attributes["custom_int"].data[edge.index].value = expected[pair][2]
    source.attributes["custom_int2"].data[edge.index].value = (edge.index + 10, edge.index + 20)
    source.attributes["bevel_weight_edge"].data[edge.index].value = expected[pair][0] / 2
    edge.use_seam = bool(expected[pair][1])
obj = bpy.data.objects.new("edge-source", source)
bpy.context.collection.objects.link(obj)
_, arrays = mesh_reader.read_object(obj, bpy.context.evaluated_depsgraph_get())
assert len(arrays["topo/edge_pairs"]) == 4
assert "edge/seam" in arrays

# Force a different wire row order to verify the writer uses pairs.
order = [2, 0, 3, 1]
arrays["topo/edge_pairs"] = arrays["topo/edge_pairs"][order]
for key in list(arrays):
    if key.startswith("edge/"):
        arrays[key] = arrays[key][order]
mirror = Mirror()
mirror.arrays = arrays
mirror.counts = {"point": 4, "vertex": 4, "prim": 1}
target = bpy.data.meshes.new("edge-target")
u = GeoUpdate(full=True, topology_changed=True, arrays=arrays)
result = mesh_writer.apply_update(target, mirror, u)
assert not result["warnings"], result["warnings"]
for edge in target.edges:
    pair = tuple(sorted(edge.vertices))
    crease, sharp, integer = expected[pair]
    assert abs(target.attributes["crease_edge"].data[edge.index].value - crease) < 1e-6
    assert target.attributes["sharp_edge"].data[edge.index].value == bool(sharp)
    assert target.attributes["custom_int"].data[edge.index].value == integer
    assert tuple(target.attributes["custom_int2"].data[edge.index].value) == (integer, integer + 10)
    assert abs(target.attributes["bevel_weight_edge"].data[edge.index].value - crease / 2) < 1e-6
    assert edge.use_seam == bool(sharp)
print("MINIHOU_EDGE_SELFTEST_OK", bpy.app.version_string)
