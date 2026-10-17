"""Real headless Blender material/link test. Set BLENDER_EXE and freshly built MINIHOU_LINK_SELFTEST."""
import hashlib
import json
import os
from pathlib import Path
import socket
import subprocess
import sys
import tempfile
import time
import unittest

try:
    import bpy
except ImportError:
    bpy = None

ROOT = Path(__file__).resolve().parent.parent
sys.path[:0] = [str(ROOT), str(ROOT / "blender_addon")]


@unittest.skipIf(bpy is not None or not os.getenv("BLENDER_EXE") or not os.getenv("MINIHOU_LINK_SELFTEST"),
                 "set BLENDER_EXE and MINIHOU_LINK_SELFTEST")
class LaunchBlender(unittest.TestCase):
    def test_material_link(self):
        proc = subprocess.run([os.environ["BLENDER_EXE"], "-b", "--factory-startup", "--python-exit-code", "1",
                               "--python", str(Path(__file__).resolve())], capture_output=True, text=True,
                              encoding="utf-8", errors="replace", timeout=180,
                              creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0))
        self.assertEqual(proc.returncode, 0, (proc.stdout + proc.stderr)[-6000:])
        self.assertIn("TEXTURE_LINK_OK", proc.stdout)


def run_in_blender():
    import numpy as np
    from minihou_link import material_export, mesh_reader
    from minihou_link.protocol import send_message, recv_message
    temp = Path(tempfile.mkdtemp(prefix="minihou-texture-"))
    graph = temp / "graph.json"
    graph.write_text(json.dumps({"version": 1, "nodes": {"box": {"type": "box"}}, "display": "box"}))
    raw = bytes([255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 0, 255])
    png = material_export._png(2, 2, raw)
    path = temp / "external.png"
    path.write_bytes(png)
    mesh = bpy.data.meshes.new("Seam")
    mesh.from_pydata([(0, 0, 0), (1, 0, 0), (1, 1, 0), (0, 1, 0)], [], [(0, 1, 2), (0, 2, 3)])
    uv = mesh.uv_layers.new(name="UVMap")
    for i, loop in enumerate(uv.data):
        loop.uv = ((i % 3) / 2, (i // 3) / 2)
    obj = bpy.data.objects.new("Seam", mesh)
    bpy.context.collection.objects.link(obj)
    images = []
    for kind in ("external", "packed", "generated"):
        if kind == "generated":
            image = bpy.data.images.new(kind, 2, 2, alpha=True)
            image.pixels[:] = [1, 0, 0, 1] * 4
        else:
            image = bpy.data.images.load(str(path), check_existing=False)
            if kind == "packed":
                image.pack()
        images.append(image)
        mat = bpy.data.materials.new(kind)
        mat.use_nodes = True
        nodes = mat.node_tree.nodes
        shader = next(n for n in nodes if n.type == "BSDF_PRINCIPLED")
        tex = nodes.new("ShaderNodeTexImage")
        tex.image = image
        mat.node_tree.links.new(tex.outputs["Color"], shader.inputs["Base Color"])
        mesh.materials.append(mat)
    mesh.polygons[0].material_index = 0
    mesh.polygons[1].material_index = 1
    before = [(image.is_dirty, image.filepath, bool(image.packed_file)) for image in images]
    manifest, payloads = material_export.export_object(obj)
    assert before == [(image.is_dirty, image.filepath, bool(image.packed_file)) for image in images]
    assert len(manifest["descriptors"]) == 3 and len(payloads) >= 2
    assert not manifest["diagnostics"], manifest["diagnostics"]
    external_tex = next(n for n in mesh.materials[0].node_tree.nodes if n.type == "TEX_IMAGE")
    uv_node = mesh.materials[0].node_tree.nodes.new("ShaderNodeUVMap")
    uv_node.uv_map = "UVMap"
    mesh.materials[0].node_tree.links.new(uv_node.outputs["UV"], external_tex.inputs["Vector"])
    with_uv, _ = material_export.export_object(obj)
    assert with_uv["descriptors"]["blender:Seam:0"]["textures"]["base_color"]["uv_set"] == "uv"
    mapping = mesh.materials[0].node_tree.nodes.new("ShaderNodeMapping")
    mapping.inputs["Location"].default_value[0] = .5
    mesh.materials[0].node_tree.links.new(uv_node.outputs["UV"], mapping.inputs["Vector"])
    mesh.materials[0].node_tree.links.new(mapping.outputs["Vector"], external_tex.inputs["Vector"])
    mapped, _ = material_export.export_object(obj)
    assert "Mapping" in mapped["diagnostics"]["blender:Seam:0"]
    mesh.materials[0].node_tree.links.new(uv_node.outputs["UV"], external_tex.inputs["Vector"])
    group = bpy.data.node_groups.new("Simple image group", "ShaderNodeTree")
    group.interface.new_socket(name="Color", in_out="OUTPUT", socket_type="NodeSocketColor")
    group_output = group.nodes.new("NodeGroupOutput")
    group_tex = group.nodes.new("ShaderNodeTexImage")
    group_tex.image = images[0]
    group.links.new(group_tex.outputs["Color"], group_output.inputs["Color"])
    group_node = mesh.materials[0].node_tree.nodes.new("ShaderNodeGroup")
    group_node.node_tree = group
    external_shader = next(n for n in mesh.materials[0].node_tree.nodes if n.type == "BSDF_PRINCIPLED")
    mesh.materials[0].node_tree.links.new(group_node.outputs["Color"], external_shader.inputs["Base Color"])
    grouped, _ = material_export.export_object(obj)
    assert not grouped["diagnostics"], grouped["diagnostics"]
    mesh.materials[0].node_tree.links.new(external_tex.outputs["Color"], external_shader.inputs["Base Color"])
    external_tex.image = None
    missing, _ = material_export.export_object(obj)
    assert missing["descriptors"]["blender:Seam:0"]["textures"]["base_color"]["image_id"].startswith("sha256:")
    assert "no image" in missing["diagnostics"]["blender:Seam:0"]
    external_tex.image = images[0]
    for hash_id, payload in payloads.items():
        assert hash_id == "sha256:" + hashlib.sha256(payload).hexdigest()
    # A procedural Base Color must produce a visible diagnostic and a constant factor.
    mat = mesh.materials[2]
    shader = next(n for n in mat.node_tree.nodes if n.type == "BSDF_PRINCIPLED")
    mat.node_tree.links.new(mat.node_tree.nodes.new("ShaderNodeTexNoise").outputs["Color"], shader.inputs["Base Color"])
    unsupported, _ = material_export.export_object(obj)
    assert unsupported["diagnostics"], unsupported
    tex = next(n for n in mat.node_tree.nodes if n.type == "TEX_IMAGE")
    mat.node_tree.links.new(tex.outputs["Color"], shader.inputs["Base Color"])
    with socket.socket() as probe:
        probe.bind(("127.0.0.1", 0))
        port = probe.getsockname()[1]
    engine = subprocess.Popen([os.environ["MINIHOU_LINK_SELFTEST"], str(graph), f"127.0.0.1:{port}", "30"],
                              stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True,
                              creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0))
    try:
        assert "LINK_READY" in engine.stdout.readline()
        def connect():
            sock = socket.create_connection(("127.0.0.1", port), timeout=3)
            sock.settimeout(3)
            send_message(sock, {"type": "hello", "protocol": 1, "client": "Blender texture test",
                                "capabilities": ["viewport_materials_v1", "image_assets_v1"]})
            assert recv_message(sock)[0]["type"] == "welcome"
            assert recv_message(sock)[0]["type"] == "watch_objects"
            return sock
        def until(sock, kind):
            for _ in range(12):
                h, a = recv_message(sock)
                if h["type"] == kind:
                    return h, a
            raise AssertionError(kind)
        for first in (True, False):
            with connect() as sock:
                send_message(sock, {"type": "material_manifest", "scope": "object:Seam", "revision": 1, **manifest})
                requested = until(sock, "image_request")[0]["hashes"]
                assert len(requested) == len(payloads) if first else not requested
                for hash_id in requested:
                    payload = payloads[hash_id]
                    send_message(sock, {"type": "image_begin", "transfer_id": hash_id, "hash": hash_id, "total": len(payload)})
                    for offset in range(0, len(payload), 1024 * 1024):
                        chunk = payload[offset:offset + 1024 * 1024]
                        send_message(sock, {"type": "image_chunk", "transfer_id": hash_id, "hash": hash_id,
                                            "offset": offset, "total": len(payload)}, {"data": np.frombuffer(chunk, np.uint8)})
                    send_message(sock, {"type": "image_end", "transfer_id": hash_id})
                    assert until(sock, "image_ack")[0]["hash"] == hash_id
                fields, arrays = mesh_reader.read_object(obj, bpy.context.evaluated_depsgraph_get())
                send_message(sock, {"type": "object_geo", "object": "Seam", "manifest_revision": 1, **fields}, arrays)
            time.sleep(.1)
        image = images[2]
        image.pixels[0] = .25
        changed, changed_payloads = material_export.export_object(obj)
        assert changed["images"] != manifest["images"]
        from minihou_link.instance_client import InstanceClient
        client = InstanceClient("127.0.0.1", port, timeout=10)
        try:
            revision = client.sync_materials("seam-instance", list(mesh.materials))
            assert revision == 1
            assert client.sync_materials("seam-instance", list(mesh.materials)) == 1
            instance_graph = {"version": 1, "nodes": {"input": {"type": "modifier_input"},
                              "output": {"type": "output", "inputs": {"geo": "input"}}}, "display": "output"}
            fields, arrays = mesh_reader.read_object(obj, bpy.context.evaluated_depsgraph_get())
            client.set_sources("seam-instance", {"source": arrays}, materials=list(mesh.materials),
                               manifest_revision=revision)
            reply, _ = client.cook("seam-instance", "", {}, arrays, graph=instance_graph,
                                   materials=fields["materials"], manifest_revision=revision)
            assert reply["type"] == "instance_result", reply
            image.pixels[0] = .75
            assert client.sync_materials("seam-instance", list(mesh.materials)) == 2
        finally:
            client.close()
        print("TEXTURE_LINK_OK", flush=True)
    finally:
        engine.terminate()
        try:
            engine.communicate(timeout=3)
        except subprocess.TimeoutExpired:
            engine.kill()


if __name__ == "__main__":
    if bpy is None:
        unittest.main()
    else:
        run_in_blender()
