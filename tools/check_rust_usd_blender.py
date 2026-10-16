"""Blender background interoperability check (never touches an interactive scene).

blender --background --factory-startup --python tools/check_rust_usd_blender.py -- rust/target_usd/interop
"""
from pathlib import Path
import json
import sys
import bpy

folder = Path(sys.argv[sys.argv.index("--") + 1]).resolve()
reports = []
for name in ("triangle.usda", "triangle.usdc", "triangle.usd", "triangle.usdz", "animated.usdc"):
    bpy.ops.wm.read_factory_settings(use_empty=True)
    result = bpy.ops.wm.usd_import(filepath=str(folder / name))
    assert result == {"FINISHED"}, (name, result)
    meshes = [obj for obj in bpy.context.scene.objects if obj.type == "MESH"]
    assert len(meshes) == 1, (name, [o.name for o in bpy.context.scene.objects])
    obj = meshes[0]
    assert len(obj.data.vertices) == 3 and len(obj.data.polygons) == 1, name
    assert obj.data.uv_layers, (name, "UVs missing")
    assert obj.data.materials and obj.data.materials[0].name.startswith("RustRed"), (name, "material missing")
    shader = next(n for n in obj.data.materials[0].node_tree.nodes if n.type == "BSDF_PRINCIPLED")
    assert all(abs(a - b) < 1e-5 for a, b in zip(shader.inputs["Base Color"].default_value[:3], (0.8, 0.2, 0.1))), name
    if name == "animated.usdc":
        bpy.context.scene.frame_set(2)
        evaluated = obj.evaluated_get(bpy.context.evaluated_depsgraph_get())
        assert abs(evaluated.data.vertices[0].co.x - 2) < 1e-5
    reports.append({"file": name, "vertices": 3, "polygons": 1, "uv": True, "material": True})

# A foreign binary fixture, authored by Blender's official USD exporter.
bpy.ops.wm.read_factory_settings(use_empty=True)
mesh = bpy.data.meshes.new("BlenderTriangle")
mesh.from_pydata([(0, 0, 0), (1, 0, 0), (0, 1, 0)], [], [(0, 1, 2)])
obj = bpy.data.objects.new("BlenderTriangle", mesh)
bpy.context.collection.objects.link(obj)
obj.location = (3, 4, 5)
bpy.ops.wm.usd_export(filepath=str(folder / "blender.usdc"), export_animation=False)
(folder / "blender-report.json").write_text(json.dumps(reports, indent=2), encoding="utf8")
print("RUST_USD_BLENDER_OK " + json.dumps(reports))
