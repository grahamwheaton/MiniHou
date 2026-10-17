"""End-to-end texture transport through a real Blender MiniHou modifier and app test host."""
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
    def test_clip_modifier_transport(self):
        run = subprocess.run([os.environ["BLENDER_EXE"], "-b", "--factory-startup", "--python-exit-code", "1",
                              "--python", str(Path(__file__).resolve())], capture_output=True, text=True,
                             encoding="utf-8", errors="replace", timeout=180,
                             creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0))
        self.assertEqual(run.returncode, 0, (run.stdout + run.stderr)[-8000:])
        self.assertIn("TEXTURE_MODIFIER_OK", run.stdout)


def run_in_blender():
    from minihou_link import material_export, modifiers
    temp = Path(tempfile.mkdtemp(prefix="minihou-clip-"))
    graph = temp / "graph.json"
    graph.write_text(json.dumps({"version": 1, "nodes": {"input": {"type": "modifier_input"},
        "output": {"type": "output", "inputs": {"geo": "input"}}}, "display": "output"}))
    png = material_export._png(2, 2, bytes([255, 0, 0, 0, 0, 255, 0, 255,
                                             0, 0, 255, 128, 255, 255, 0, 255]))
    path = temp / "screenshot.png"
    path.write_bytes(png)
    with socket.socket() as probe:
        probe.bind(("127.0.0.1", 0))
        port = probe.getsockname()[1]
    control = temp / "control.json"
    engine = subprocess.Popen([os.environ["MINIHOU_LINK_SELFTEST"], str(graph), f"127.0.0.1:{port}",
                               "90", str(control)], stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True,
                              creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0))
    try:
        assert "LINK_READY" in engine.stdout.readline()
        for kind in ("external", "packed"):
            name = "imagepaste-261017-142014" + ("" if kind == "external" else "-packed")
            mesh = bpy.data.meshes.new(name)
            mesh.from_pydata([(0, 0, 0), (1, 0, 0), (1, 1, 0), (0, 1, 0)], [],
                             [(0, 1, 2), (0, 2, 3)])
            uv = mesh.uv_layers.new(name="UVMap")
            for i, loop in enumerate(uv.data):
                loop.uv = mesh.vertices[mesh.loops[i].vertex_index].co[:2]
            obj = bpy.data.objects.new(name, mesh)
            bpy.context.collection.objects.link(obj)
            image = bpy.data.images.load(str(path), check_existing=False)
            if kind == "packed":
                image.pack()
            mat = bpy.data.materials.new(name)
            mat.use_nodes = True
            nodes = mat.node_tree.nodes
            shader = next(n for n in nodes if n.type == "BSDF_PRINCIPLED")
            tex = nodes.new("ShaderNodeTexImage")
            tex.image, tex.extension, tex.interpolation = image, "CLIP", "Linear"
            mat.node_tree.links.new(tex.outputs["Color"], shader.inputs["Base Color"])
            mesh.materials.append(mat)
            # Match the reported three material users without adding geometry to the link.
            for i in range(2):
                other = bpy.data.meshes.new(name + "-user-" + str(i))
                other.materials.append(mat)
            assert mat.users == 3, mat.users
            mod = modifiers.add_modifier(obj, str(graph), port=port)
            arrays, materials = modifiers.read_prefix(obj, mod, with_materials=True)
            assert materials == (mat,), [(m.name, m.as_pointer(), m.node_tree.nodes.get("Principled BSDF").inputs["Alpha"].default_value)
                                         for m in materials if m]
            manifest, payloads = material_export.export_materials(materials, modifiers.instance_id(mod), "UVMap")
            assert not manifest["diagnostics"], manifest["diagnostics"]
            descriptor = manifest["descriptors"][manifest["bindings"][0]]
            assert descriptor["textures"]["base_color"]["sampler"]["wrap_u"] == "clamp"
            assert len(payloads) == 1
            modifiers.pump(port=port, force=True)
            assert modifiers.last_error() == "", modifiers.last_error()
            client = modifiers._connection(port=port)
            revision = client.sync_materials(modifiers.instance_id(mod), list(materials), "UVMap")
            cooked, _ = client.cook(modifiers.instance_id(mod), str(graph), {}, arrays,
                                    graph=json.loads(mod.node_group[modifiers.GRAPH]),
                                    materials=[mat.name], manifest_revision=revision)
            assert cooked["type"] == "instance_result", cooked
            assert cooked["detail"]["materials"] == [name], cooked.get("detail")
            assert cooked["detail"]["material_bindings"]["slots"][0].endswith(
                "blender:" + modifiers.instance_id(mod) + ":0"), cooked.get("detail")
            result = temp / (kind + "-result.json")
            control.write_text(json.dumps({"id": 1 if kind == "external" else 2, "op": "texture_probe",
                              "slot_suffix": "blender:" + modifiers.instance_id(mod) + ":0", "output": str(result)}))
            for _ in range(100):
                if result.exists():
                    break
                time.sleep(.05)
            assert result.exists(), "app texture probe did not respond"
            report = json.loads(result.read_text())
            assert report["state"] == "Ok" and report["image"] == [2, 2], report
            assert report["image_id"] and report["slot"].endswith("blender:" + modifiers.instance_id(mod) + ":0"), report
            shader.inputs["Alpha"].default_value = .5
            rejected, _ = material_export.export_materials([mat], modifiers.instance_id(mod), "UVMap")
            reason = rejected["diagnostics"][rejected["bindings"][0]]
            assert mat.name in reason and "Alpha=0.500" in reason, reason
            shader.inputs["Alpha"].default_value = 1
            mat.node_tree.links.new(tex.outputs["Alpha"], shader.inputs["Alpha"])
            rejected, _ = material_export.export_materials([mat], modifiers.instance_id(mod), "UVMap")
            assert "socket is linked" in rejected["diagnostics"][rejected["bindings"][0]]
        print("TEXTURE_MODIFIER_OK", flush=True)
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
