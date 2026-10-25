"""Real-Blender tests for embedded networks (add-on 0.3.0).

From a normal Python (skips cleanly unless BLENDER_EXE names a Blender executable):

    $env:BLENDER_EXE='C:\\Program Files\\Blender Foundation\\Blender 5.2\\blender.exe'
    python -m unittest tests.test_blender_embed -v

That launches separate headless processes only (``blender -b --factory-startup``); it never touches an
interactive Blender. Inside Blender the tests below run directly; files are saved to a temp folder and
reopened in a *fresh* Blender process. The link test needs the Rust ``link_selftest`` example
(MINIHOU_LINK_SELFTEST=<path>, or a build in one of the rust/target_* folders).
"""
import json
import os
import subprocess
import sys
import tempfile
import time
import unittest
import uuid
from pathlib import Path

try:
    import bpy
except ImportError:
    bpy = None

ROOT = Path(__file__).resolve().parent.parent
sys.path[:0] = [str(ROOT), str(ROOT / "blender_addon")]

GRAPH = {"version": 1, "nodes": {
    "input": {"type": "modifier_input"},
    "transform": {"type": "transform", "inputs": {"geo": "input"}, "params": {"translate": [1, 0, 0]}},
    "output": {"type": "output", "inputs": {"geo": "transform"}}},
    "display": "output",
    "exposed": [{"name": "Scale", "node": "transform", "param": "uniform_scale"}]}

CHECK = r'''
import json, sys, hashlib
sys.path[:0] = [{root!r}, {addon!r}]
import bpy
from minihou_link import embed
out = {{}}
for ng in bpy.data.node_groups:
    if embed.network_id(ng):
        text = embed.text_of(ng)
        content = text.as_string() if text else ""
        cache = embed._cache_object(ng)
        out[ng.name] = {{"mode": ng.get(embed.MODE), "text": ng.get(embed.TEXT), "fake": bool(text and text.use_fake_user),
            "hash_ok": hashlib.sha256(content.encode()).hexdigest() == ng.get(embed.HASH),
            "schema": ng.get(embed.ESCHEMA), "addon": ng.get(embed.ADDON), "content": content,
            "cache_points": len(cache.data.vertices) if cache else -1}}
embed.load_all()
for ng in bpy.data.node_groups:
    if embed.network_id(ng):
        out[ng.name]["runtime"] = json.loads(ng.get(embed.GRAPH, "{{}}"))
        out[ng.name]["missing"] = json.loads(ng.get(embed.MISSING, "[]"))
        out[ng.name]["error"] = ng.get(embed.ERROR, "")
out["texts"] = sorted(t.name for t in bpy.data.texts)
with open({result!r}, "w", encoding="utf-8") as f:
    json.dump(out, f)
'''


def blender_exe():
    return os.environ.get("BLENDER_EXE", "")


@unittest.skipIf(bpy is not None or not blender_exe(), "set BLENDER_EXE to run the real-Blender embed tests")
class RunInBlender(unittest.TestCase):
    """Outer launcher: runs this module inside a separate headless Blender."""

    def test_embedded_networks_in_real_blender(self):
        proc = subprocess.run([blender_exe(), "-b", "--factory-startup", "--python-exit-code", "1",
                               "--python", str(Path(__file__).resolve())],
                              capture_output=True, text=True, encoding="utf-8", errors="replace", timeout=900,
                              creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0))
        tail = (proc.stdout + proc.stderr)[-6000:]
        self.assertEqual(proc.returncode, 0, tail)
        self.assertIn("OK", proc.stderr + proc.stdout, tail)


@unittest.skipIf(bpy is None, "requires real Blender")
class EmbeddedNetworkTests(unittest.TestCase):
    def setUp(self):
        from minihou_link import embed, modifiers, proxy
        self.e, self.m, self.proxy = embed, modifiers, proxy
        self.m.unregister()
        bpy.ops.wm.read_factory_settings(use_empty=True)
        self.m.register()                         # includes embed's save/load handlers
        self.addCleanup(self.m.unregister)
        self.tmp = Path(tempfile.mkdtemp(prefix="minihou-embed-"))

    # -- helpers ---------------------------------------------------------------------------------
    def obj(self, name="Cube"):
        mesh = bpy.data.meshes.new(name)
        mesh.from_pydata([(0, 0, 0), (1, 0, 0), (0, 1, 0)], [], [(0, 1, 2)])
        obj = bpy.data.objects.new(name, mesh)
        bpy.context.scene.collection.objects.link(obj)
        return obj

    def network(self, obj, graph=GRAPH, embed_it=True):
        """What add_modifier builds, without needing a link connection."""
        m = self.m
        mod = obj.modifiers.new("MiniHou", "NODES")
        ng = bpy.data.node_groups.new("MiniHou test", "GeometryNodeTree")
        ng[m.KEY], ng[m.PATH], ng[m.GRAPH], ng[m.PARAMS] = uuid.uuid4().hex, "", json.dumps(graph), "[]"
        mod.node_group = ng
        self.proxy.ensure_nested(ng)
        m._interface(mod, {"interface": []}, initial=True)
        self.proxy.link_passthrough(ng)
        if embed_it:
            self.e.embed(ng, label=f"{obj.name} {mod.name}")
        return mod, ng

    def save(self, name="scene.blend"):
        path = self.tmp / name
        bpy.ops.wm.save_as_mainfile(filepath=str(path), check_existing=False)
        return path

    def reopen(self, path):
        """Open the .blend in a fresh Blender process and report what it finds."""
        result = self.tmp / ("check-" + uuid.uuid4().hex[:6] + ".json")
        script = self.tmp / "check.py"
        script.write_text(CHECK.format(root=str(ROOT), addon=str(ROOT / "blender_addon"), result=str(result)),
                          encoding="utf-8")
        proc = subprocess.run([bpy.app.binary_path, "-b", "--factory-startup", "--python-exit-code", "1",
                               str(path), "--python", str(script)], capture_output=True, text=True, timeout=300,
                              creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0))
        self.assertEqual(proc.returncode, 0, proc.stdout[-3000:] + proc.stderr[-3000:])
        return json.loads(result.read_text(encoding="utf-8"))

    def users(self):
        return (sorted((g.name, g.users) for g in bpy.data.node_groups),
                sorted((t.name, t.users) for t in bpy.data.texts),
                sorted((m.name, m.users) for m in bpy.data.meshes))

    # -- tests -----------------------------------------------------------------------------------
    def test_embed_save_reopen_in_fresh_process(self):
        _mod, ng = self.network(self.obj())
        text = self.e.text_of(ng)
        self.assertTrue(text.name.startswith("MiniHou: Cube MiniHou"))
        self.assertTrue(text.use_fake_user)
        self.assertEqual(ng[self.e.MODE], "EMBEDDED")
        report = self.reopen(self.save())[ng.name]
        self.assertEqual((report["mode"], report["text"], report["fake"], report["hash_ok"]),
                         ("EMBEDDED", text.name, True, True))
        self.assertEqual((report["schema"], report["addon"]), (1, self.e.addon_version()))
        self.assertEqual(report["runtime"], GRAPH)
        self.assertEqual(report["error"], "")

    def test_purge_keeps_embedded_text(self):
        _mod, ng = self.network(self.obj())
        name = self.e.text_of(ng).name
        bpy.data.orphans_purge(do_local_ids=True, do_linked_ids=True, do_recursive=True)
        self.assertIn(name, bpy.data.texts)

    def test_duplicate_object_gets_independent_text(self):
        obj = self.obj()
        mod, ng = self.network(obj)
        copy = obj.copy()                                   # shares the modifier's node group
        bpy.context.scene.collection.objects.link(copy)
        self.m.pump(port=1, force=True)                     # no server: identity repair still runs
        a, b = obj.modifiers[0].node_group, copy.modifiers[0].node_group
        self.assertIsNot(a, b)
        ta, tb = self.e.text_of(a), self.e.text_of(b)
        self.assertIsNot(ta, tb)
        self.assertEqual((ta["minihou_owner"], tb["minihou_owner"]), (self.e.network_id(a), self.e.network_id(b)))
        edited = json.loads(json.dumps(GRAPH))
        edited["nodes"]["transform"]["params"]["translate"] = [5, 0, 0]
        self.e.embed(b, edited)
        self.assertNotIn("5", json.dumps(json.loads(ta.as_string())["nodes"]["transform"]["params"]))
        self.assertEqual(json.loads(tb.as_string())["nodes"]["transform"]["params"]["translate"], [5, 0, 0])
        self.assertEqual(ta.as_string(), self.e.canonical(GRAPH))

    def test_extract_then_embed_from_file_is_byte_identical(self):
        self.save()
        _mod, ng = self.network(self.obj())
        before = self.e.text_of(ng).as_string()
        path = str(self.tmp / "shared.minihou")
        self.e.extract(ng, path)
        self.assertEqual(Path(path).read_text(encoding="utf-8"), before)
        self.e.text_of(ng).from_string("{}")                # make sure the text really is rewritten
        self.e.embed_from_file(ng, path)
        self.assertEqual(self.e.text_of(ng).as_string(), before)
        self.assertEqual(ng[self.e.HASH], self.e.digest(before))
        self.assertEqual(self.e.compare(ng)["newer"], "same")

    def test_save_pre_writes_pending_edits(self):
        _mod, ng = self.network(self.obj())
        edited = json.loads(json.dumps(GRAPH))
        edited["nodes"]["transform"]["params"]["translate"] = [7, 0, 0]
        ng[self.e.GRAPH] = json.dumps(edited)               # an edit that arrived over the link
        self.e.note_graph(ng)
        self.assertEqual(json.loads(self.e.text_of(ng).as_string()), GRAPH, "debounced: not written yet")
        report = self.reopen(self.save())[ng.name]
        self.assertTrue(report["hash_ok"])
        self.assertEqual(report["runtime"], edited)
        self.assertEqual(json.loads(self.e.text_of(ng).as_string()), edited)
        self.assertEqual(self.e._pending, {})

    def test_debounced_flush_writes_once_quiet(self):
        _mod, ng = self.network(self.obj())
        edited = dict(GRAPH, display="input")
        ng[self.e.GRAPH] = json.dumps(edited)
        self.e.note_graph(ng)
        self.assertEqual(self.e.flush(), 0)
        time.sleep(self.e.DEBOUNCE + 0.05)
        self.assertEqual(self.e.flush(), 1)
        self.assertEqual(json.loads(self.e.text_of(ng).as_string())["display"], "input")

    def test_embedded_and_file_conflict_detected_and_resolved(self):
        self.save()
        _mod, ng = self.network(self.obj())
        path = self.tmp / "net.minihou"
        self.e.extract(ng, str(path))
        self.assertEqual(self.e.compare(ng)["newer"], "same")
        changed = dict(GRAPH, display="input")
        time.sleep(0.05)
        path.write_text(self.e.canonical(changed), encoding="utf-8")
        os.utime(path, (time.time() + 5, time.time() + 5))
        self.assertEqual(self.e.compare(ng)["newer"], "file")
        self.e.resolve_choice(ng, "USE_FILE")
        self.assertEqual(self.e.compare(ng)["newer"], "same")
        self.assertEqual(json.loads(ng[self.e.GRAPH])["display"], "input")
        self.e.embed(ng, GRAPH)                             # embedded edited after the file
        os.utime(path, (time.time() - 60, time.time() - 60))
        self.assertEqual(self.e.compare(ng)["newer"], "embedded")
        self.e.resolve_choice(ng, "MATCH_FILE")
        self.assertEqual(self.e.compare(ng)["newer"], "same")
        with self.assertRaisesRegex(self.e.EmbedError, "differs"):
            path.write_text(self.e.canonical(changed), encoding="utf-8")
            self.e.switch_mode(ng)
        self.e.resolve_choice(ng, "MATCH_FILE")
        self.assertEqual(self.e.switch_mode(ng), "LINKED")
        self.assertIsNone(self.e.text_of(ng))
        self.assertEqual(ng[self.e.PATH], str(path))
        self.assertFalse([t for t in bpy.data.texts if t.name.startswith("MiniHou: ")])
        self.assertEqual(self.e.switch_mode(ng), "EMBEDDED")

    def test_paths_are_relative_to_the_blend_and_resolved_on_load(self):
        (self.tmp / "tex").mkdir()
        image = self.tmp / "tex" / "a.png"
        image.write_bytes(b"\x89PNG")
        elsewhere = Path(tempfile.mkdtemp(prefix="minihou-elsewhere-")) / "scene.usd"
        elsewhere.write_text("#usda 1.0", encoding="utf-8")
        graph = json.loads(json.dumps(GRAPH))
        graph["nodes"]["img"] = {"type": "image_file", "params": {"file": str(image)}}
        graph["nodes"]["usd"] = {"type": "usd_import", "params": {"file": str(elsewhere)}}
        self.save()
        _mod, ng = self.network(self.obj(), graph)
        stored = json.loads(self.e.text_of(ng).as_string())
        self.assertEqual(stored["nodes"]["img"]["params"]["file"].replace("\\", "/"), "//tex/a.png")
        self.assertEqual(json.loads(ng[self.e.PATHS])["img/file"], str(image))
        self.assertEqual(json.loads(ng[self.e.GRAPH])["nodes"]["img"]["params"]["file"], str(image))
        info = self.e.compare(ng)
        self.assertEqual([o.split(":")[0] for o in info["outside"]], ["usd.file"])
        extracted = self.tmp / "x.minihou"
        self.e.extract(ng, str(extracted))
        self.assertEqual(json.loads(extracted.read_text(encoding="utf-8"))["nodes"]["img"]["params"]["file"],
                         os.path.normpath(str(image)))
        # Move the project: the relative texture follows the .blend.
        moved = Path(tempfile.mkdtemp(prefix="minihou-moved-"))
        (moved / "tex").mkdir()
        (moved / "tex" / "a.png").write_bytes(b"\x89PNG")
        blend = self.save()
        target = moved / "scene.blend"
        target.write_bytes(blend.read_bytes())
        report = self.reopen(target)[ng.name]
        self.assertEqual(os.path.normcase(report["runtime"]["nodes"]["img"]["params"]["file"]),
                         os.path.normcase(str(moved / "tex" / "a.png")))
        self.assertEqual(report["missing"], [])
        (moved / "tex" / "a.png").unlink()
        image.unlink()                                       # the stored original is gone too
        report = self.reopen(target)[ng.name]
        self.assertEqual(len(report["missing"]), 1)
        self.assertIn("img.file", report["missing"][0])

    def test_older_graph_migrates_and_newer_is_left_untouched(self):
        _mod, ng = self.network(self.obj())
        old = {k: v for k, v in GRAPH.items() if k != "version"}
        self.e.text_of(ng).from_string(json.dumps(old))
        self.e.load_all()
        self.assertEqual(json.loads(ng[self.e.GRAPH])["version"], 1)
        self.assertNotIn(self.e.ERROR, ng)
        runtime = ng[self.e.GRAPH]
        newer = json.dumps(dict(GRAPH, version=99))
        self.e.text_of(ng).from_string(newer)
        self.e.load_all()
        self.assertIn("newer MiniHou", ng[self.e.ERROR])
        self.assertEqual(self.e.text_of(ng).as_string(), newer)
        self.assertEqual(ng[self.e.GRAPH], runtime)
        self.e.save_all()                                    # saving must not clobber it either
        self.assertEqual(self.e.text_of(ng).as_string(), newer)

    def test_deleting_the_modifier_removes_its_text(self):
        obj = self.obj()
        mod, ng = self.network(obj)
        keep_mod, keep = self.network(self.obj("Other"))
        name = self.e.text_of(ng).name
        obj.modifiers.remove(mod)
        self.m._cleanup(set())
        self.assertNotIn(name, bpy.data.texts)
        self.assertIsNotNone(self.e.text_of(keep))
        bpy.data.texts.new("Notes")                          # the user's own texts are never touched
        self.e.repair()
        self.assertIn("Notes", bpy.data.texts)

    def test_lost_text_is_rebuilt_from_the_runtime_copy(self):
        _mod, ng = self.network(self.obj())
        bpy.data.texts.remove(self.e.text_of(ng))
        self.e.repair()
        self.assertEqual(json.loads(self.e.text_of(ng).as_string()), GRAPH)
        self.assertIn("rebuilt", ng[self.e.ERROR])

    def test_repeated_syncs_leak_no_users_or_orphans(self):
        obj = self.obj()
        mod, ng = self.network(obj)
        ident = self.m.instance_id(mod)
        self.m._cleanup({ident})
        before = self.users()
        for k in range(12):
            g = json.loads(json.dumps(GRAPH))
            g["nodes"]["transform"]["params"]["translate"] = [k, 0, 0]
            ng[self.e.GRAPH] = json.dumps(g)
            self.e.note_graph(ng)
            self.e.flush(force=True)
            self.m._cleanup({ident})
            self.assertEqual(self.users(), before, f"sync {k}")
        self.assertEqual(json.loads(self.e.text_of(ng).as_string())["nodes"]["transform"]["params"]["translate"],
                         [11, 0, 0])

    def test_panel_section_draws_status_and_buttons(self):
        from minihou_link import embed_ui
        from tests.test_blender_link_sync import RecordingLayout
        mod, ng = self.network(self.obj())
        ng[self.e.SIZE] = self.e.WARN_BYTES + 1
        ng[self.e.STATE] = "conflict"
        layout = RecordingLayout()
        embed_ui.draw(layout, mod)
        labels = [kw.get("text", "") for name, args, kw in layout.calls if name == "label"]
        self.assertTrue(any(t.startswith("Embedded in .blend") and "#" + ng[self.e.HASH][:8] in t for t in labels), labels)
        self.assertTrue(any(t.startswith("Large network") for t in labels), labels)
        self.assertTrue(any("MiniHou window" in t for t in labels), labels)
        ops = [args[0] for name, args, kw in layout.calls if name == "operator"]
        for op in ("minihou.embed_from_file", "minihou.extract_graph", "minihou.switch_embed_mode"):
            self.assertIn(op, ops)
        self.assertNotIn("minihou.toggle_freeze", ops)
        self.assertFalse(any(t.startswith("Freeze result") for t in labels))

    def test_legacy_path_modifier_stays_linked(self):
        _mod, ng = self.network(self.obj(), embed_it=False)
        ng[self.m.PATH] = str(self.tmp / "legacy.json")
        self.assertEqual(self.e.mode(ng), "LINKED")
        self.e.load_all()
        self.e.save_all()
        self.e.repair()
        self.assertIsNone(self.e.text_of(ng))
        self.assertEqual(ng[self.e.FILE], str(self.tmp / "legacy.json"))

    def test_freeze_result_keeps_or_drops_the_cooked_mesh(self):
        obj = self.obj()
        mod, ng = self.network(obj)
        cache = self.e._cache_object(ng)
        cache.data.from_pydata([(0, 0, 0), (1, 0, 0), (0, 1, 0), (1, 1, 0)], [], [(0, 1, 3, 2)])
        report = self.reopen(self.save("frozen.blend"))[ng.name]
        self.assertEqual(report["cache_points"], 4)
        verts, cost = self.e.frozen_cost(ng)
        self.assertEqual(verts, 4)
        self.assertGreater(cost, 0)
        ng[self.e.FREEZE] = False
        report = self.reopen(self.save("unfrozen.blend"))[ng.name]
        self.assertEqual(report["cache_points"], 0)
        self.assertEqual(len(self.e._cache_object(ng).data.vertices), 4, "the open session keeps its mesh")
        self.assertFalse([m for m in bpy.data.meshes if m.name.startswith("MiniHou Cache (not frozen)")])


@unittest.skipIf(bpy is None, "requires real Blender")
class EmbeddedLinkTests(unittest.TestCase):
    """graph_push / graph_state against the Rust link_selftest server."""

    @classmethod
    def setUpClass(cls):
        from tests.test_blender_rust_placements import RustEngine, TRANSFORM
        # An older link_selftest build (e.g. rust/target_chain2) lacks graph_embed_v1: the add-on then
        # correctly falls back to a plain live edit and no conflict can be reported. Use a current build.
        exe = next((p for p in [os.environ.get("MINIHOU_LINK_SELFTEST", "")] +
                    [str(ROOT / "rust" / t / "debug" / "examples" / "link_selftest.exe")
                     for t in ("target_embed", "target_chain3", "target_chain2", "target_place", "target_inst")]
                    if p and os.path.isfile(p) and b"graph_embed_v1" in Path(p).read_bytes()), None)
        if exe is None:
            raise unittest.SkipTest("no link_selftest build with graph_embed_v1; run cargo build -p minihou-app "
                                    "--example link_selftest and set MINIHOU_LINK_SELFTEST")
        os.environ["MINIHOU_LINK_SELFTEST"] = exe
        cls.engine = RustEngine(TRANSFORM)
        cls.command = 100

    @classmethod
    def tearDownClass(cls):
        cls.engine.stop()

    def setUp(self):
        from minihou_link import embed, geonodes, modifiers
        self.e, self.m, self.g = embed, modifiers, geonodes
        self.g.unregister()
        self.m.unregister()
        self.m.disconnect()
        bpy.ops.wm.read_factory_settings(use_empty=True)
        self.addCleanup(self.m.disconnect)

    def control(self, **command):
        type(self).command += 1
        command["id"] = self.command
        tmp = self.engine.control.with_suffix(".tmp")
        tmp.write_text(json.dumps(command), encoding="utf-8")
        for _ in range(25):
            try:
                tmp.replace(self.engine.control)
                break
            except PermissionError:
                time.sleep(0.05)
        else:
            tmp.replace(self.engine.control)
        time.sleep(0.3)

    def retry(self, fn):
        for _ in range(8):
            try:
                return fn()
            except TimeoutError:
                self.m.disconnect()
                time.sleep(0.2)
        return fn()

    def pump_until(self, predicate):
        for _ in range(40):
            self.m.pump(port=self.engine.port, force=True)
            if predicate():
                return
            time.sleep(0.1)
        self.engine.log.flush()
        log = (Path(self.engine.temp.name) / "server.log").read_text(encoding="utf-8", errors="replace")
        self.fail(f"condition not reached; last error: {self.m.last_error()}; server log tail: {log[-1500:]}")

    def test_push_edit_in_app_and_conflict_resolution(self):
        bpy.ops.mesh.primitive_plane_add()
        obj = bpy.context.object
        mod = self.retry(lambda: self.m.add_modifier(obj, " ", port=self.engine.port))
        ng = mod.node_group
        self.assertEqual(self.e.mode(ng), "EMBEDDED")
        self.m._endpoint = ("127.0.0.1", self.engine.port)
        self.retry(lambda: self.m.edit_live(obj, mod))       # graph_push (accepted) + live activation
        self.assertIn(self.m.instance_id(mod), self.m._live_ids)
        # The user edits the bound document in the app; the edit lands in the embedded text.
        edited = json.loads(ng[self.m.GRAPH])
        edited["nodes"]["transform"].setdefault("params", {})["translate"] = [3, 0, 0]
        self.control(op="replace", graph=edited)
        self.pump_until(lambda: json.loads(ng[self.m.GRAPH])["nodes"]["transform"]["params"].get("translate") == [3, 0, 0])
        self.e.flush(force=True)
        stored = json.loads(self.e.text_of(ng).as_string())
        self.assertEqual(stored["nodes"]["transform"]["params"]["translate"], [3, 0, 0])
        self.assertEqual(ng[self.e.HASH], self.e.digest(self.e.text_of(ng).as_string()))
        # Both sides change: Blender's embedded copy (from a file) and the app's document.
        app_version = json.loads(json.dumps(edited))
        app_version["nodes"]["transform"]["params"]["translate"] = [9, 0, 0]
        self.control(op="replace", graph=app_version)
        blender_version = json.loads(json.dumps(edited))
        blender_version["nodes"]["transform"]["params"]["translate"] = [-4, 0, 0]
        self.e.embed(ng, blender_version)
        with self.assertRaises(self.e.ConflictPending):
            self.retry(lambda: self.m.edit_live(obj, mod))
        self.assertEqual(ng[self.e.STATE], "conflict")
        self.m.pump(port=self.engine.port, force=True)       # still pending: the text is not overwritten
        self.e.save_all()
        self.assertEqual(json.loads(self.e.text_of(ng).as_string())["nodes"]["transform"]["params"]["translate"],
                         [-4, 0, 0])
        self.control(op="resolve", choice="use_embedded")
        self.pump_until(lambda: self.e.STATE not in ng)
        self.e.flush(force=True)
        self.assertEqual(json.loads(self.e.text_of(ng).as_string())["nodes"]["transform"]["params"]["translate"],
                         [-4, 0, 0])
        # Keep my version: the app's graph replaces the embedded copy.
        self.control(op="replace", graph=app_version)
        self.e.embed(ng, edited)
        with self.assertRaises(self.e.ConflictPending):
            self.retry(lambda: self.m.edit_live(obj, mod))
        self.control(op="resolve", choice="keep_mine")
        self.pump_until(lambda: self.e.STATE not in ng)
        self.assertEqual(json.loads(self.e.text_of(ng).as_string())["nodes"]["transform"]["params"]["translate"],
                         [9, 0, 0])


if __name__ == "__main__":
    result = unittest.main(argv=[sys.argv[0]], verbosity=2, exit=False).result
    sys.stdout.flush()
    sys.exit(0 if result.wasSuccessful() else 1)
