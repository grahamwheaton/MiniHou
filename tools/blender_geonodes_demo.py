"""Create a wired MiniHou example; run with Blender --python (see docs/GEONODES.md)."""
import argparse
from pathlib import Path
import sys

import bpy

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "blender_addon"))
import minihou_link
from minihou_link import geonodes, modifiers


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--host", default="127.0.0.1")
    parser.add_argument("--port", type=int, default=8765)
    parser.add_argument("--save", type=Path)
    args = parser.parse_args(sys.argv[sys.argv.index("--") + 1:] if "--" in sys.argv else [])
    if not hasattr(bpy.types.Scene, "minihou_graph_path"):
        minihou_link.register()
    wm = bpy.context.window_manager
    wm.minihou_host, wm.minihou_port = args.host, args.port
    graph = str(ROOT / "examples" / "modifiers" / "transform.json")
    bpy.context.scene.minihou_graph_path = graph

    # Additive: keep existing user objects and create a fresh, uniquely named example.
    bpy.ops.mesh.primitive_cube_add(size=2, location=(0, 0, 0))
    obj = bpy.context.active_object
    obj.name = "MiniHou Wired Demo"
    material = bpy.data.materials.new("MiniHou Demo Copper")
    material.diffuse_color = (0.7, 0.22, 0.055, 1)
    obj.data.materials.append(material)
    weight = obj.data.attributes.new("demo_weight", "FLOAT", "POINT")
    for i, item in enumerate(weight.data):
        item.value = i / max(1, len(weight.data) - 1)

    tree = bpy.data.node_groups.new("MiniHou Wired Demo Network", "GeometryNodeTree")
    tree.interface.new_socket("Geometry", in_out="INPUT", socket_type="NodeSocketGeometry")
    tree.interface.new_socket("Geometry", in_out="OUTPUT", socket_type="NodeSocketGeometry")
    obj.modifiers.new("MiniHou Wired Network", "NODES").node_group = tree
    gin = tree.nodes.new("NodeGroupInput")
    gout = tree.nodes.new("NodeGroupOutput")
    pre = tree.nodes.new("GeometryNodeTransform")
    pre.label = "Native upstream scale"
    pre.inputs["Scale"].default_value = (1.5, 0.7, 1)
    a = geonodes.add_group(tree, graph, host=args.host, port=args.port)
    a.name, a.label = "MiniHou Scale", "MiniHou A: independent scale"
    a.inputs["Scale"].default_value = 1.4
    middle = tree.nodes.new("GeometryNodeTransform")
    middle.label = "Native operation between groups"
    middle.inputs["Translation"].default_value = (0, 0, 0.5)
    b = geonodes.add_group(tree, graph, host=args.host, port=args.port)
    b.name, b.label = "MiniHou Translate", "MiniHou B: independent translation"
    b.inputs["Translate"].default_value = (0, 0, 1.5)
    post = tree.nodes.new("GeometryNodeTransform")
    post.label = "Native downstream rotation"
    post.inputs["Rotation"].default_value = (0, 0, 0.35)
    nodes = [gin, pre, a, middle, b, post, gout]
    for i, node in enumerate(nodes):
        node.location = (i * 290, 0)
    for left, right in zip(nodes, nodes[1:]):
        tree.links.new(left.outputs["Geometry"], right.inputs["Geometry"])
    for node in nodes:
        node.select = False
    b.select = True
    tree.nodes.active = b
    bpy.context.view_layer.update()
    cooked = modifiers.pump(host=args.host, port=args.port, force=True)
    if modifiers.last_error():
        raise RuntimeError(modifiers.last_error())
    if not all(geonodes.status(node).startswith("Ready") for node in (a, b)):
        raise RuntimeError("Demo placements not ready: " + str([geonodes.status(n) for n in (a, b)]))
    evaluated = obj.evaluated_get(bpy.context.evaluated_depsgraph_get())
    mesh = evaluated.to_mesh(preserve_all_data_layers=True, depsgraph=bpy.context.evaluated_depsgraph_get())
    try:
        assert len(mesh.vertices) == 8 and len(mesh.polygons) == 6
        assert "demo_weight" in mesh.attributes
        assert len(mesh.materials) == 1
    finally:
        evaluated.to_mesh_clear()
    if args.save:
        path = args.save.expanduser().resolve()
        path.parent.mkdir(parents=True, exist_ok=True)
        bpy.ops.wm.save_as_mainfile(filepath=str(path))
    print("MINIHOU_GEONODES_DEMO_OK", "cooks=", cooked, "object=", obj.name)
    print("Edit native scale, MiniHou A Scale, or MiniHou B Translate; use Cook Wired Groups / Stack Now.")
    # Keep registration and timer alive for interactive Blender; background exits normally.


if __name__ == "__main__":
    main()
