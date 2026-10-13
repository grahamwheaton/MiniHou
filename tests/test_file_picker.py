"""Choosing, dropping and saving files: the folder browser, the native-dialog hook, uploads, workspace-relative paths and the
USD Export node's Save to Disk button."""
import http.client
import json
import os
import shutil
import tempfile
import unittest

from minihou import Graph, filedialog, paths, usd_io
from minihou.link.session import Session
from minihou.web import WebServer


def needs_usd(fn):
    return unittest.skipUnless(usd_io.available(), "usd-core is not installed")(fn)


class PickerTests(unittest.TestCase):
    def setUp(self):
        self.dir = os.path.realpath(tempfile.mkdtemp(prefix="minihou_pick_"))
        self.addCleanup(shutil.rmtree, self.dir, True)
        self.outside = os.path.realpath(tempfile.mkdtemp(prefix="minihou_outside_"))
        self.addCleanup(shutil.rmtree, self.outside, True)
        self.s = Session(Graph.from_dict({"nodes": {"box": {"type": "box"}}, "display": "box"}))
        self.s.start()
        self.web = WebServer(self.s, "127.0.0.1", 0, root=self.dir)
        self.port = self.web.start()
        self.addCleanup(self.s.stop)
        self.addCleanup(self.web.stop)
        self.addCleanup(paths.set_root, None)

    def req(self, method, path, body=None, headers=None, raw=None):
        c = http.client.HTTPConnection("127.0.0.1", self.port, timeout=15)
        h = {"Content-Type": "application/json"} if body is not None else {}
        h.update(headers or {})
        c.request(method, path, body=raw if raw is not None else (json.dumps(body) if body is not None else None), headers=h)
        r = c.getresponse()
        data = r.read()
        c.close()
        try:
            return r.status, json.loads(data)
        except ValueError:
            return r.status, data

    # -- paths -----------------------------------------------------------------------------------------------------------
    def test_relative_paths_start_in_the_workspace_and_come_back_relative(self):
        self.assertEqual(paths.resolve("a/b.usd"), os.path.join(self.dir, "a", "b.usd"))
        self.assertEqual(paths.resolve(""), "")
        self.assertEqual(paths.to_param(os.path.join(self.dir, "a", "b.usd")), "a/b.usd")
        far = os.path.join(self.outside, "x.usd")
        self.assertEqual(paths.to_param(far), far)
        self.assertEqual(paths.resolve(far), far)

    # -- the folder browser ----------------------------------------------------------------------------------------------
    def test_browse_lists_folders_and_only_the_files_asked_for(self):
        os.makedirs(os.path.join(self.dir, "models"))
        for n in ("b.usd", "a.USDA", "notes.txt", ".hidden.usd"):
            with open(os.path.join(self.dir, n), "wb") as f:
                f.write(b"x" * 5)
        st, d = self.req("GET", "/api/browse?exts=.usd,.usda&places=1")
        self.assertEqual(st, 200)
        self.assertEqual(d["path"], self.dir)
        self.assertEqual([(e["name"], e["dir"]) for e in d["entries"]], [("models", True), ("a.USDA", False), ("b.usd", False)])
        self.assertEqual(d["entries"][1]["size"], 5)
        self.assertTrue(d["parent"])
        self.assertTrue(any(p["name"] == "Workspace" for p in d["places"]))
        st, d = self.req("GET", "/api/browse?path=" + self.outside.replace("\\", "/"))
        self.assertEqual((st, d["path"]), (200, self.outside))
        st, d = self.req("GET", "/api/browse?path=" + os.path.join(self.dir, "nope").replace("\\", "/"))
        self.assertEqual(st, 404)

    def test_browse_a_file_lists_its_folder_and_hidden_files_on_request(self):
        with open(os.path.join(self.dir, ".h.usd"), "wb") as f:
            f.write(b"x")
        st, d = self.req("GET", "/api/browse?path=" + os.path.join(self.dir, ".h.usd").replace("\\", "/") + "&hidden=1")
        self.assertEqual(d["path"], self.dir)
        self.assertEqual([e["name"] for e in d["entries"]], [".h.usd"])

    def test_extension_lists_are_cleaned(self):
        self.assertEqual(filedialog.clean_exts(".usd, .USDA ,bad,../x,.toolongextension"), [".usd", ".usda"])
        self.assertEqual(filedialog.clean_exts(None), [])

    # -- the native dialog ------------------------------------------------------------------------------------------------
    def test_pick_file_uses_the_desktop_dialog_and_stores_workspace_paths_relative(self):
        calls = []

        def fake(mode, exts, title, start):
            calls.append((mode, exts, title, start))
            return {"path": os.path.join(self.dir, "models", "car.usdc")}
        self.web.pick_file = fake
        st, d = self.req("POST", "/api/pick_file", {"mode": "open", "exts": [".usd", "bad"], "title": "Pick"})
        self.assertEqual((st, d), (200, {"native": True, "cancelled": False, "path": "models/car.usdc"}))
        self.assertEqual(calls[0], ("open", [".usd"], "Pick", self.dir))
        self.web.pick_file = lambda *a: {"path": os.path.join(self.outside, "far.usd")}
        st, d = self.req("POST", "/api/pick_file", {"mode": "save"})
        self.assertEqual(d["path"], os.path.join(self.outside, "far.usd"))
        self.web.pick_file = lambda *a: {"path": ""}
        st, d = self.req("POST", "/api/pick_file", {})
        self.assertEqual((d["native"], d["cancelled"], d["path"]), (True, True, ""))

    def test_pick_file_says_when_there_is_no_system_dialog(self):
        old = filedialog.system_dialog_available
        filedialog.system_dialog_available = lambda: False
        self.addCleanup(setattr, filedialog, "system_dialog_available", old)
        st, d = self.req("POST", "/api/pick_file", {"mode": "open"})
        self.assertEqual((st, d), (200, {"native": False}))

    # -- uploads ---------------------------------------------------------------------------------------------------------
    def upload(self, name, data, headers=None):
        h = {"Content-Type": "application/octet-stream", "X-MiniHou-Upload": "1"}
        h.update(headers or {})
        return self.req("POST", "/api/upload?name=" + name, raw=data, headers=h)

    def test_a_dropped_file_is_streamed_into_imports_and_never_replaces_another(self):
        data = os.urandom(3 * 2**20 + 17)
        st, d = self.upload("model.usdc", data)
        self.assertEqual((st, d), (200, {"path": "imports/model.usdc", "size": len(data)}))
        with open(os.path.join(self.dir, "imports", "model.usdc"), "rb") as f:
            self.assertEqual(f.read(), data)
        st, d = self.upload("model.usdc", b"second")
        self.assertEqual(d["path"], "imports/model_1.usdc")
        self.assertEqual(sorted(os.listdir(os.path.join(self.dir, "imports"))), ["model.usdc", "model_1.usdc"])

    def test_uploads_are_guarded(self):
        self.assertEqual(self.upload("a.usd", b"x", {"X-MiniHou-Upload": "0"})[0], 403)
        self.assertEqual(self.upload("a.exe", b"x")[0], 400)
        self.assertEqual(self.upload("..%2F..%2Fevil.usd", b"x")[1].get("path"), "imports/evil.usd")     # only the file name counts
        self.assertEqual(self.upload("a.usd", b"")[0], 413)
        self.assertEqual(self.upload("a.usd", b"x", {"Origin": "http://evil.example"})[0], 403)
        self.assertEqual(self.upload("a.usd", b"x", {"Host": "evil.example"})[0], 403)
        self.assertFalse(any(f.endswith(".part") for f in os.listdir(os.path.join(self.dir, "imports"))))


@needs_usd
class ExportNodeTests(unittest.TestCase):
    def setUp(self):
        self.dir = os.path.realpath(tempfile.mkdtemp(prefix="minihou_exp_"))
        self.addCleanup(shutil.rmtree, self.dir, True)
        g = Graph.from_dict({"nodes": {"box": {"type": "box", "params": {"size": [2, 2, 2]}},
                                       "out": {"type": "usd_export", "inputs": {"geo": "box"},
                                               "params": {"file": "exports/box.usdc", "name": "crate", "root": "World"}}}, "display": "out"})
        self.s = Session(g)
        self.s.start()
        self.web = WebServer(self.s, "127.0.0.1", 0, root=self.dir)
        self.port = self.web.start()
        self.addCleanup(self.s.stop)
        self.addCleanup(self.web.stop)
        self.addCleanup(paths.set_root, None)

    def press(self, node="out", action="save"):
        c = http.client.HTTPConnection("127.0.0.1", self.port, timeout=30)
        c.request("POST", "/api/node_action", body=json.dumps({"node": node, "action": action}), headers={"Content-Type": "application/json"})
        r = c.getresponse()
        data = json.loads(r.read())
        c.close()
        return r.status, data

    def test_cooking_writes_nothing_and_save_to_disk_writes_the_file(self):
        self.s.wait_idle(10)
        self.assertFalse(os.path.exists(os.path.join(self.dir, "exports")))
        st, d = self.press()
        self.assertEqual(st, 200, d)
        self.assertEqual(d["path"], "exports/box.usdc")
        self.assertIn("6 faces", d["message"])
        target = os.path.join(self.dir, "exports", "box.usdc")
        geo, rep = usd_io.read_geometry(target)
        self.assertEqual(geo.nprims, 6)
        from pxr import Usd
        stage = Usd.Stage.Open(target)
        self.assertEqual(stage.GetDefaultPrim().GetName(), "World")
        self.assertTrue(stage.GetPrimAtPath("/World/crate"))

    def test_settings_up_axis_units_format_and_materials(self):
        self.s.edit([{"op": "set", "id": "out", "params": dict(up_axis="y", units="centimeters", format="usda", materials=False, file="exports/box.usd")}])
        st, d = self.press()
        self.assertEqual(st, 200, d)
        self.assertEqual(d["path"], "exports/box.usda")
        target = os.path.join(self.dir, "exports", "box.usda")
        from pxr import Usd, UsdGeom
        stage = Usd.Stage.Open(target)
        self.assertEqual(UsdGeom.GetStageUpAxis(stage), "Y")
        self.assertAlmostEqual(UsdGeom.GetStageMetersPerUnit(stage), 0.01)
        mesh = UsdGeom.Mesh(stage.GetPrimAtPath("/World/crate/mesh"))
        pts = mesh.GetPointsAttr().Get()
        self.assertAlmostEqual(max(abs(p[0]) for p in pts), 100.0, places=3)                 # a 2 m box, in centimetres
        geo, rep = usd_io.read_geometry(target)                                               # reads back as metres, Z up
        self.assertAlmostEqual(float(abs(geo.P).max()), 1.0, places=4)
        self.assertEqual(rep.up_axis, "Y")
        ref = Graph.from_dict({"nodes": {"b": {"type": "box", "params": {"size": [2, 2, 2]}}}, "display": "b"})
        from minihou import Evaluator
        import numpy as np
        want = Evaluator().cook(ref).P
        self.assertTrue(np.allclose(np.sort(want, axis=0), np.sort(geo.P, axis=0), atol=1e-4))

    def test_overwrite_off_refuses_and_a_bad_button_is_an_error(self):
        self.assertEqual(self.press()[0], 200)
        self.s.edit([{"op": "set", "id": "out", "params": {"overwrite": False}}])
        st, d = self.press()
        self.assertEqual(st, 409)
        self.assertIn("overwrite", d["error"])
        self.assertEqual(self.press(action="nope")[0], 400)
        self.assertEqual(self.press(node="missing")[0], 400)


if __name__ == "__main__":
    unittest.main()
