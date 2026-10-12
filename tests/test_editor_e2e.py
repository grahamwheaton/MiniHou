"""The web editor driven in a real (headless) Chromium: clicks, drags and keys must reach the engine.

Needs ``pip install playwright`` and a Chromium for it (``python -m playwright install chromium``); skipped otherwise.
"""
import os
import shutil
import tempfile
import time
import unittest

from minihou import Graph
from minihou.link.session import Session
from minihou.web import WebServer

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))

try:
    from playwright.sync_api import sync_playwright
except ImportError:                                  # pragma: no cover - optional dependency
    sync_playwright = None


def until(pred, timeout=5.0):
    end = time.monotonic() + timeout
    while time.monotonic() < end:
        if pred():
            return True
        time.sleep(0.03)
    return False


@unittest.skipIf(sync_playwright is None, "playwright not installed")
class EditorE2E(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        try:
            cls.pw = sync_playwright().start()
            cls.browser = cls.pw.chromium.launch()
        except Exception as e:  # noqa: BLE001 - no browser available
            if getattr(cls, "pw", None):
                cls.pw.stop()
            raise unittest.SkipTest(f"cannot launch Chromium: {e}")

    @classmethod
    def tearDownClass(cls):
        cls.browser.close()
        cls.pw.stop()

    def setUp(self):
        self.dir = tempfile.mkdtemp(prefix="minihou_e2e_")
        self.addCleanup(shutil.rmtree, self.dir, True)
        self.path = os.path.join(self.dir, "demo.json")
        shutil.copyfile(os.path.join(ROOT, "examples", "graphs", "demo.json"), self.path)
        self.s = Session(Graph.load(self.path, check=False))
        self.s.start()
        self.s.watch_file(self.path, interval=0.05)
        from minihou.library import NodeLibrary
        self.lib = NodeLibrary([os.path.join(self.dir, "nodes")])
        self.addCleanup(self.lib.unload)
        self.s.attach_library(self.lib, interval=0.05)
        self.web = WebServer(self.s, "127.0.0.1", 0, root=self.dir)
        self.web.start()
        self.addCleanup(self.s.stop)
        self.addCleanup(self.web.stop)
        self.page = self.browser.new_page(viewport={"width": 1400, "height": 850})
        self.addCleanup(self.page.close)
        self.errors = []
        self.page.on("pageerror", lambda e: self.errors.append(str(e)))
        self.page.on("console", lambda m: m.type == "error" and self.errors.append(m.text))
        self.page.goto(self.web.url)
        self.page.wait_for_selector('.node[data-id="out"]')

    def tearDown(self):
        self.assertEqual(self.errors, [], "browser errors")

    # -- helpers ---------------------------------------------------------------------
    def read_file(self):
        with open(self.path, encoding="utf-8") as f:
            return f.read()

    def node(self, nid):
        return self.page.locator(f'.node[data-id="{nid}"]')

    def center(self, locator):
        locator.wait_for(state="visible")
        b = locator.bounding_box()
        return b["x"] + b["width"] / 2, b["y"] + b["height"] / 2

    def drag(self, src, dst):
        self.page.mouse.move(*self.center(src))
        self.page.mouse.down()
        x, y = self.center(dst)
        self.page.mouse.move(x - 20, y - 20, steps=5)
        self.page.mouse.move(x, y, steps=5)
        self.page.mouse.up()

    def empty_spot(self):
        b = self.page.locator("#canvas").bounding_box()
        return b["x"] + 60, b["y"] + b["height"] - 60

    # -- tests -------------------------------------------------------------------------
    def test_repeat_block_insert_edit_duplicate_rename_reload_and_undo(self):
        self.s.edit([{"op": "replace", "graph": {"nodes": {"box": {"type": "box", "pos": [0, 0]}}, "display": "box"}}])
        self.page.wait_for_selector('.node[data-id="box"]')
        self.page.locator("#canvas").focus()
        self.page.keyboard.press("f")
        self.node("box").locator(".title").click()
        self.page.mouse.move(*self.empty_spot())
        self.page.keyboard.press("Tab")
        self.page.keyboard.type("loop_block")
        self.page.keyboard.press("Enter")
        self.page.wait_for_selector('.node[data-id="loop_end1"].selected')
        self.assertEqual(self.s.graph.nodes["loop_end1"].inputs, {"geo": "transform1", "begin": "loop_begin1"})
        self.page.locator("#params button", has_text="Display Loop End").click()
        count = self.page.locator('.prow[data-param="iterations"] input[type=number]')
        count.fill("4")
        count.press("Enter")
        self.assertTrue(until(lambda: self.s.graph.get_param("loop_end1", "iterations") == 4))
        self.page.locator("#canvas").focus()
        self.page.keyboard.press("f")
        self.node("transform1").locator(".title").click()
        number = self.page.locator('.prow[data-param="translate"] input[type=number]').first
        number.fill("0.5")
        number.press("Enter")
        self.assertTrue(until(lambda: self.s.graph.get_param("transform1", "translate")[0] == .5))
        self.page.wait_for_function("document.querySelector('#cook-status').classList.contains('ok')")
        self.assertEqual(self.page.locator(".loop-region").count(), 1)
        self.assertEqual(self.page.locator("path.loop-pair").count(), 1)
        from minihou import Evaluator
        import numpy as np
        ev = Evaluator()
        np.testing.assert_allclose(ev.cook(self.s.graph).P, ev.cook(self.s.graph, "box").P + [2, 0, 0])

        self.node("loop_end1").locator(".title").click()
        self.page.locator("#params button", has_text="Duplicate").click()
        self.page.wait_for_selector('.node[data-id="loop_end_1"]')
        self.assertEqual(len(self.s.graph.nodes), 7)
        self.assertEqual(self.s.graph.nodes["loop_end_1"].inputs["begin"], "loop_begin_1")
        self.assertEqual(self.s.graph.nodes["transform_1"].inputs["geo"], "loop_begin_1")
        self.s.graph.check()
        self.page.locator("#canvas").focus()
        self.page.keyboard.press("Control+z")
        self.assertTrue(until(lambda: len(self.s.graph.nodes) == 4))
        self.page.keyboard.press("Control+Shift+z")
        self.assertTrue(until(lambda: len(self.s.graph.nodes) == 7))
        self.page.keyboard.press("f")
        self.node("loop_begin_1").locator(".title").click()
        self.page.fill("#params input.nid", "copy_begin")
        self.page.press("#params input.nid", "Enter")
        self.assertTrue(until(lambda: "copy_begin" in self.s.graph.nodes))
        self.assertEqual(self.s.graph.nodes["loop_end_1"].inputs["begin"], "copy_begin")
        self.assertTrue(until(lambda: '"copy_begin"' in self.read_file()))
        disk = Graph.load(self.path)
        self.assertEqual(disk.nodes["loop_end_1"].inputs["begin"], "copy_begin")
        self.page.reload()
        self.page.wait_for_selector('.node[data-id="copy_begin"]')
        self.assertEqual(self.page.locator(".loop-region").count(), 2)
        self.assertEqual(self.page.locator("path.loop-pair").count(), 2)

    def test_repeat_nested_duplicate_and_unpaired_error_recovery(self):
        self.s.edit([{"op": "replace", "graph": {"nodes": {"box": {"type": "box", "pos": [0, 0]}}, "display": "box"}},
                     {"op": "loop", "from": "box"},
                     {"op": "loop", "from": "transform1", "to": "loop_end1", "port": "geo", "pos": [200, 120]},
                     {"op": "display", "id": "loop_end1"}])
        self.page.wait_for_selector('.node[data-id="loop_end2"]')
        self.page.locator("#canvas").focus()
        self.page.keyboard.press("f")
        self.node("loop_end1").locator(".title").click()
        self.page.locator("#params button", has_text="Duplicate").click()
        self.assertTrue(until(lambda: len(self.s.graph.nodes) == 13))
        self.s.graph.check()
        copied = [n for nid, n in self.s.graph.nodes.items() if n.type == "loop_end" and "_" in nid[8:]]
        self.assertEqual(len(copied), 2)
        for n in copied:
            self.assertNotIn(n.inputs["begin"], {"loop_begin1", "loop_begin2"})
        self.s.edit([{"op": "disconnect", "to": "loop_end1", "port": "begin"}])
        self.page.wait_for_function("document.querySelector('#cook-status').classList.contains('err')")
        self.assertTrue(self.page.locator('.node[data-id="loop_end1"].error').count())
        self.s.edit([{"op": "connect", "from": "loop_begin1", "to": "loop_end1", "port": "begin"}])
        self.page.wait_for_function("document.querySelector('#cook-status').classList.contains('ok')")

    def test_delayed_slider_edit_is_recovered_after_external_graph_switch(self):
        import json
        target = os.path.join(self.dir, "other.json")
        Graph.load(self.path).save(target)  # same node IDs: a stale edit could silently hit the wrong graph
        self.node("hills").locator(".title").click()
        held = []
        self.page.route("**/api/edit", lambda route: held.append(route))
        number = self.page.locator('.prow[data-param="frequency"] input[type=number]')
        number.fill("0.73")
        number.press("Enter")
        self.page.wait_for_timeout(100)
        self.assertEqual(len(held), 1)
        self.s.open_file(target)
        self.page.wait_for_function("window.minihou.S.path.endsWith('other.json')")
        held[0].continue_()
        self.page.wait_for_selector("#unsent-edits")
        self.assertNotEqual(self.s.graph.get_param("hills", "frequency"), .73)
        with self.page.expect_download() as downloading:
            self.page.click("#unsent-edits")
        saved = os.path.join(self.dir, "recovery.json")
        downloading.value.save_as(saved)
        with open(saved, encoding="utf-8") as f:
            recovery = json.load(f)
        self.assertEqual(recovery[-1]["path"], self.path)
        self.assertEqual(recovery[-1]["ops"][0]["params"]["frequency"], .73)
        self.errors[:] = [e for e in self.errors if "400 (Bad Request)" not in e]

    def test_renders_graph_status_and_inspector(self):
        self.assertEqual(self.page.locator(".node").count(), 9)
        self.page.wait_for_function("document.querySelector('#cook-status').classList.contains('ok')")
        self.assertIn("pts", self.page.inner_text("#cook-status"))
        self.assertIn("display", self.node("out").get_attribute("class"))
        self.page.wait_for_function("document.querySelector('#info-body').innerText.includes('Points')")

    def test_copy_and_paste_keeps_wires_between_copied_nodes(self):
        self.node("hills").locator(".title").click()
        self.page.keyboard.down("Shift")
        self.node("ripple").locator(".title").click()
        self.page.keyboard.up("Shift")
        self.page.locator("#canvas").focus()
        self.page.mouse.move(*self.empty_spot())
        self.page.keyboard.press("Control+c")
        self.page.keyboard.press("Control+v")
        self.assertTrue(until(lambda: "hills1" in self.s.graph.nodes and "ripple1" in self.s.graph.nodes))
        g = self.s.graph.nodes
        self.assertEqual(g["ripple1"].inputs, {"geo": "hills1"})              # the wire between the copies
        self.assertNotIn("geo", g["hills1"].inputs)                           # the wire to 'ground' is not copied
        self.assertEqual(g["hills1"].params, g["hills"].params)
        self.assertEqual(g["ripple1"].params, g["ripple"].params)
        self.assertEqual(g["ripple"].inputs, {"geo": "hills"})                # originals untouched
        self.page.wait_for_selector('.node[data-id="ripple1"].selected')

    def test_cut_removes_and_paste_restores_into_another_graph(self):
        self.node("spike_up").locator(".title").click()
        self.page.locator("#canvas").focus()
        self.page.keyboard.press("Control+x")
        self.assertTrue(until(lambda: "spike_up" not in self.s.graph.nodes))
        clip = self.page.evaluate("localStorage.getItem('minihou.clip')")
        self.assertIn('"minihou":"nodes"', clip)
        other = os.path.join(self.dir, "other.json")
        with open(other, "w", encoding="utf-8") as f:
            f.write('{"version": 1, "nodes": {"b": {"type": "box"}}}')
        self.page.evaluate("fetch('/api/open', {method: 'POST', headers: {'Content-Type': 'application/json'}, "
                           "body: JSON.stringify({path: 'other.json'})})")
        self.page.wait_for_selector('.node[data-id="b"]')
        self.page.locator("#canvas").focus()
        self.page.keyboard.press("Control+v")
        self.assertTrue(until(lambda: any(n.type == "transform" for n in self.s.graph.nodes.values())))

    def test_pasting_junk_or_unknown_types_changes_nothing(self):
        before = dict(self.s.graph.nodes)
        for text in ('hello', '{"minihou": "nodes", "nodes": []}', '{"minihou": "nodes", "nodes": {"x": {"type": "nope"}}}'):
            ok = self.page.evaluate("t => minihou.pasteNodes(t)", text)
            self.assertIn(ok, (True, False))
        time.sleep(0.2)
        self.assertEqual(set(self.s.graph.nodes), set(before))
        self.errors[:] = [e for e in self.errors if "Failed to load resource" not in e]

    def test_network_box_moves_with_its_nodes_resizes_renames_and_deletes(self):
        self.node("hills").locator(".title").click()
        self.page.keyboard.down("Shift")
        self.node("ripple").locator(".title").click()
        self.page.keyboard.up("Shift")
        self.page.locator("#canvas").focus()
        self.page.keyboard.press("o")
        self.assertTrue(until(lambda: len(self.s.graph.boxes) == 1))
        box = self.page.locator(".netbox")
        box.wait_for(state="visible")
        before = {k: list(self.s.graph.nodes[k].meta["pos"]) for k in ("hills", "ripple", "points")}
        b0 = dict(self.s.graph.boxes[0])
        hdr = box.locator(".hdr")
        x, y = self.center(hdr.locator(".title"))
        self.page.mouse.move(x, y)
        self.page.mouse.down()
        self.page.mouse.move(x + 60, y + 40, steps=6)
        self.page.mouse.up()
        self.assertTrue(until(lambda: self.s.graph.boxes[0]["pos"] != b0["pos"]))
        db = [self.s.graph.boxes[0]["pos"][i] - b0["pos"][i] for i in (0, 1)]
        for k in ("hills", "ripple"):                                            # members moved by the same amount
            self.assertEqual([self.s.graph.nodes[k].meta["pos"][i] - before[k][i] for i in (0, 1)], db)
        self.assertEqual(self.s.graph.nodes["points"].meta["pos"], before["points"])    # outsiders stay put
        s0 = list(self.s.graph.boxes[0]["size"])
        gx, gy = self.center(box.locator(".grip"))
        self.page.mouse.move(gx, gy)
        self.page.mouse.down()
        self.page.mouse.move(gx + 50, gy + 30, steps=5)
        self.page.mouse.up()
        self.assertTrue(until(lambda: self.s.graph.boxes[0]["size"][0] > s0[0] and self.s.graph.boxes[0]["size"][1] > s0[1]))
        self.page.once("dialog", lambda d: d.accept("Terrain"))
        self.page.locator(".netbox .title").dblclick()
        self.assertTrue(until(lambda: self.s.graph.boxes[0]["title"] == "Terrain"))
        self.page.wait_for_function("document.querySelector('.netbox .title').textContent === 'Terrain'")
        c0 = self.s.graph.boxes[0]["color"]
        self.page.locator(".netbox .dot").click()
        self.assertTrue(until(lambda: self.s.graph.boxes[0]["color"] != c0))
        self.page.locator(".netbox .x").click()
        self.assertTrue(until(lambda: self.s.graph.boxes == []))
        self.page.locator("#canvas").focus()
        self.page.keyboard.press("Control+z")
        self.assertTrue(until(lambda: len(self.s.graph.boxes) == 1))

    def test_box_button_makes_an_empty_box_and_boxes_are_saved(self):
        self.page.click("#box-btn")
        self.assertTrue(until(lambda: len(self.s.graph.boxes) == 1))
        self.assertTrue(until(lambda: '"boxes"' in self.read_file()))
        self.page.wait_for_selector(".netbox")

    def pixels_vary(self, selector="#pv-canvas"):
        """True when the element's screenshot has more than a couple of distinct colours (something was drawn)."""
        import io
        import struct
        import zlib
        png = self.page.locator(selector).screenshot()
        pos, chunks = 8, {}
        while pos < len(png):
            (n,) = struct.unpack(">I", png[pos:pos + 4])
            chunks.setdefault(png[pos + 4:pos + 8], []).append(png[pos + 8:pos + 8 + n])
            pos += 12 + n
        w, h, depth, ctype = struct.unpack(">IIBB", chunks[b"IHDR"][0][:10])
        raw = zlib.decompress(b"".join(chunks[b"IDAT"]))
        bpp = {2: 3, 6: 4}[ctype]
        stride = w * bpp
        colours, prev = set(), bytearray(stride)
        for y in range(h):
            row = bytearray(raw[y * (stride + 1) + 1:(y + 1) * (stride + 1)])
            ft = raw[y * (stride + 1)]
            for i in range(stride):                         # undo PNG row filters
                a = row[i - bpp] if i >= bpp else 0
                b = prev[i]
                c = prev[i - bpp] if i >= bpp else 0
                if ft == 1:
                    row[i] = (row[i] + a) & 255
                elif ft == 2:
                    row[i] = (row[i] + b) & 255
                elif ft == 3:
                    row[i] = (row[i] + ((a + b) >> 1)) & 255
                elif ft == 4:
                    p = a + b - c
                    pa, pb, pc = abs(p - a), abs(p - b), abs(p - c)
                    row[i] = (row[i] + (a if pa <= pb and pa <= pc else b if pb <= pc else c)) & 255
            prev = row
            colours.update(bytes(row[x:x + bpp]) for x in range(0, stride, bpp * 3))
            if len(colours) > 20:
                return True
        return len(colours) > 3

    def test_3d_preview_draws_the_selected_node_and_reports_errors(self):
        if not self.page.evaluate("!!document.createElement('canvas').getContext('webgl2')"):
            self.skipTest("this Chromium has no WebGL2")
        self.page.wait_for_function("window.minihou.preview && window.minihou.preview.ok && window.minihou.preview.tris > 0")
        self.assertIn("out", self.page.inner_text("#pv-node"))                          # the display node by default
        self.assertTrue(until(lambda: self.pixels_vary(), 5))
        self.node("points").locator(".title").click()                                    # points: scatter output, no faces
        self.page.wait_for_function("window.minihou.preview.tris === 0 && window.minihou.preview.points > 100")
        self.assertIn("points", self.page.inner_text("#pv-node"))
        self.assertTrue(until(lambda: self.pixels_vary(), 5))
        self.page.click("#pv-edges")                                                     # toggles redraw without errors
        self.page.click("#pv-shaded")
        self.page.click("#pv-frame")
        box = self.page.locator("#pv-canvas").bounding_box()
        self.page.mouse.move(box["x"] + 60, box["y"] + 60)                               # orbit and zoom
        self.page.mouse.down()
        self.page.mouse.move(box["x"] + 120, box["y"] + 90, steps=4)
        self.page.mouse.up()
        self.page.mouse.wheel(0, 200)
        self.page.mouse.dblclick(box["x"] + 100, box["y"] + 100)
        # a node that cannot cook shows its error instead of stale geometry
        self.s.edit([{"op": "add", "type": "wrangle", "id": "bad", "inputs": {"geo": "ground"},
                      "params": {"snippet": "@P.z = @nope"}}])
        self.node("bad").locator(".title").click()
        self.page.wait_for_function("document.querySelector('#pv-msg').textContent.includes('nope')")

    def test_inspector_lists_what_a_node_changed(self):
        self.node("ripple").locator(".title").click()
        self.page.wait_for_function("document.querySelector('#info-body').innerText.toLowerCase().includes('changes from inputs')")
        text = self.page.inner_text("#info-body")
        self.assertIn("reads @P", text)                                          # the wrangle's snippet reads @P
        self.assertRegex(text, r"~\s*P")                                          # and rewrites the positions
        self.assertTrue(self.page.locator("#info-body .flow.mod").count() >= 1)

    # -- selecting in the 3-D view and turning the selection into nodes ------------------------------------------
    def box_scene(self, **params):
        """Replace the graph by a single box into an output, select the box node and wait for the 3-D view to show it."""
        if not self.page.evaluate("!!document.createElement('canvas').getContext('webgl2')"):
            self.skipTest("this Chromium has no WebGL2")
        graph = {"version": 1, "nodes": {"b": {"type": "box", "params": dict({"size": [2, 2, 2]}, **params), "pos": [0, 0]},
                                         "o": {"type": "output", "inputs": {"geo": "b"}, "pos": [0, 110]}}, "display": "o"}
        self.s.edit([{"op": "replace", "graph": graph}])
        self.page.wait_for_selector('.node[data-id="b"]')
        self.node("b").locator(".title").click()
        self.page.wait_for_function("window.minihou.preview.tris === 12 && document.querySelector('#pv-node').innerText.includes('b')")
        from minihou import Evaluator
        self.box = Evaluator().cook(self.s.graph, "b")
        vp, offs = self.box.topology.vertex_point.astype(int), self.box.topology.prim_offsets
        centres = [self.box.P[vp[offs[p]:offs[p + 1]]].mean(0) for p in range(self.box.nprims)]
        self.top = int(max(range(len(centres)), key=lambda p: centres[p][2]))               # the face looking up at the camera
        v = vp[offs[self.top]:offs[self.top + 1]]
        self.top_edges = [(min(a, b), max(a, b)) for a, b in zip(v.tolist(), v.tolist()[1:] + v.tolist()[:1])]
        self.top_points = sorted(v.tolist())

    def view_xy(self, kind, ident):
        """Page coordinates of an element of the 3-D view."""
        box = self.page.locator("#pv-canvas").bounding_box()
        sx, sy, _ = self.page.evaluate("([k, i]) => minihou.preview.screenOf(k, i)", [kind, ident])
        return box["x"] + sx, box["y"] + sy

    def selection(self):
        return self.page.evaluate("minihou.preview.selection()")

    def test_pick_edges_with_shift_and_bevel_inserts_a_node_in_the_chain(self):
        self.box_scene()
        self.page.click('#preview [data-mode="edge"]')
        for n, e in enumerate(self.top_edges):
            self.page.keyboard.down("Shift") if n else None
            self.page.mouse.click(*self.view_xy("edge", list(e)))
            self.page.keyboard.up("Shift") if n else None
        self.assertEqual(sorted(map(tuple, self.selection()["edges"])), sorted(self.top_edges))
        self.assertIn("4 edges selected", self.page.inner_text("#pv-selinfo"))
        self.page.click('#pv-actions button[data-action="Bevel"]')
        self.assertTrue(until(lambda: "polybevel1" in self.s.graph.nodes))
        g = self.s.graph.nodes
        self.assertEqual(g["polybevel1"].inputs, {"geo": "b"})
        self.assertEqual(g["o"].inputs, {"geo": "polybevel1"})                              # inserted in the chain
        self.assertEqual(g["polybevel1"].params["edges"], " ".join(f"{a}-{b}" for a, b in sorted(self.top_edges)))
        self.page.wait_for_selector('.node[data-id="polybevel1"].selected')
        self.page.wait_for_function("document.querySelector('#pv-node').innerText.includes('polybevel1')")
        self.assertEqual(self.selection()["edges"], [])                                      # the selection is spent
        self.assertTrue(until(lambda: self.s.status()["ok"], 5))
        self.page.wait_for_function("window.minihou.preview.tris > 12")                      # the bevelled box is shown

    def test_bevel_from_a_selection_then_scrub_its_distance_with_the_arrow(self):
        self.box_scene()
        self.page.click('#preview [data-mode="edge"]')
        for n, e in enumerate(self.top_edges):
            self.page.keyboard.down("Shift") if n else None
            self.page.mouse.click(*self.view_xy("edge", list(e)))
            self.page.keyboard.up("Shift") if n else None
        self.page.click('#pv-actions button[data-action="Bevel"]')
        self.assertTrue(until(lambda: "polybevel1" in self.s.graph.nodes))
        self.page.wait_for_function("minihou.preview.handleTips().some(t => t.id === 'distance')")   # the new node has its arrow
        tip = self.drag_handle("distance", 30)
        self.assertTrue(until(lambda: self.s.graph.nodes["polybevel1"].params.get("distance", 0.1) != 0.1))
        d = self.s.graph.nodes["polybevel1"].params["distance"]
        self.assertAlmostEqual(d, 0.1 + 30 / tip["ppu"], delta=0.1)
        self.assertGreaterEqual(d, 0.0)
        self.assertTrue(until(lambda: self.s.status()["ok"], 5))

    def test_a_lone_edge_cannot_be_bevelled_and_the_error_says_why(self):
        self.box_scene()
        self.page.click('#preview [data-mode="edge"]')
        self.page.mouse.click(*self.view_xy("edge", list(self.top_edges[0])))
        self.assertEqual(len(self.selection()["edges"]), 1)
        self.page.keyboard.press("b")                                                        # the shortcut, with the mouse over the view
        self.assertTrue(until(lambda: "polybevel1" in self.s.graph.nodes))
        self.page.wait_for_selector("#errorbar:not(.hidden)")
        self.assertIn("cannot end inside the surface", self.page.inner_text("#errorbar"))

    def test_pick_a_face_and_box_select_faces_then_extrude(self):
        self.box_scene()
        self.page.click('#preview [data-mode="face"]')
        self.page.mouse.click(*self.view_xy("face", self.top))
        self.assertEqual(self.selection()["faces"], [self.top])
        canvas = self.page.locator("#pv-canvas").bounding_box()
        self.page.mouse.move(canvas["x"] + 4, canvas["y"] + 4)                                # a drag over everything: all faces
        self.page.mouse.down()
        self.page.mouse.move(canvas["x"] + canvas["width"] - 4, canvas["y"] + canvas["height"] - 4, steps=6)
        self.page.mouse.up()
        self.assertEqual(self.selection()["faces"], list(range(6)))
        self.page.keyboard.down("Control")                                                    # Ctrl-click takes one away
        self.page.mouse.click(*self.view_xy("face", self.top))
        self.page.keyboard.up("Control")
        self.assertEqual(len(self.selection()["faces"]), 5)
        self.page.click('#pv-actions button[data-action="Extrude"]')
        self.assertTrue(until(lambda: "polyextrude1" in self.s.graph.nodes))
        n = self.s.graph.nodes["polyextrude1"]
        self.assertEqual(n.inputs, {"geo": "b"})
        rest = [f for f in range(6) if f != self.top]
        expected = {0: "1-5", 5: "0-4"}.get(self.top, f"0-{self.top - 1} {self.top + 1}-5" if self.top > 1 else f"0 {self.top + 1}-5")
        self.assertEqual(n.params["group"], expected, rest)

    def test_points_select_group_and_delete_and_keys_switch_modes(self):
        self.box_scene()
        self.page.mouse.move(*self.view_xy("point", self.top_points[0]))
        self.page.keyboard.press("1")
        self.assertEqual(self.page.evaluate("minihou.preview.mode"), "point")
        self.page.mouse.click(*self.view_xy("point", self.top_points[0]))
        self.assertEqual(self.selection()["points"], [self.top_points[0]])
        self.page.keyboard.press("Escape")                                                    # clears the selection first...
        self.assertEqual(self.selection()["points"], [])
        self.assertEqual(self.page.evaluate("minihou.preview.mode"), "point")
        self.page.keyboard.press("Escape")                                                    # ...then leaves the mode
        self.assertEqual(self.page.evaluate("minihou.preview.mode"), "view")
        self.page.keyboard.press("2")
        self.assertEqual(self.page.evaluate("minihou.preview.mode"), "edge")
        self.page.keyboard.press("1")
        self.page.keyboard.press("a")                                                         # select all points
        self.assertEqual(len(self.selection()["points"]), 8)
        self.page.click('#pv-actions button[data-action="Group"]')
        self.assertTrue(until(lambda: "group_create1" in self.s.graph.nodes))
        self.assertEqual(self.s.graph.nodes["group_create1"].params["pattern"], "0-7")
        self.assertEqual(self.s.graph.nodes["group_create1"].params["domain"], "points")

    def test_orbit_still_works_with_alt_or_middle_mouse_in_a_selection_mode(self):
        self.box_scene()
        self.page.click('#preview [data-mode="face"]')
        before = self.page.evaluate("minihou.preview.view.yaw")
        box = self.page.locator("#pv-canvas").bounding_box()
        self.page.mouse.move(box["x"] + 50, box["y"] + 50)
        self.page.keyboard.down("Alt")
        self.page.mouse.down()
        self.page.mouse.move(box["x"] + 110, box["y"] + 60, steps=4)
        self.page.mouse.up()
        self.page.keyboard.up("Alt")
        self.assertNotEqual(self.page.evaluate("minihou.preview.view.yaw"), before)
        self.assertEqual(self.selection()["faces"], [])                                       # an orbit selects nothing

    def handle_scene(self, node_type, params, select):
        if not self.page.evaluate("!!document.createElement('canvas').getContext('webgl2')"):
            self.skipTest("this Chromium has no WebGL2")
        graph = {"version": 1, "nodes": {"b": {"type": "box", "params": {"size": [2, 2, 2]}, "pos": [0, 0]},
                                         "h": {"type": node_type, "inputs": {"geo": "b"}, "params": params, "pos": [0, 110]},
                                         "o": {"type": "output", "inputs": {"geo": "h"}, "pos": [0, 220]}}, "display": "o"}
        self.s.edit([{"op": "replace", "graph": graph}])
        self.page.wait_for_selector('.node[data-id="h"]')
        self.node(select).locator(".title").click()
        self.page.wait_for_function("minihou.preview.handleTips().length > 0")

    def drag_handle(self, hid, pixels):
        tip = next(t for t in self.page.evaluate("minihou.preview.handleTips()") if t["id"] == hid)
        box = self.page.locator("#pv-canvas").bounding_box()
        x, y = box["x"] + tip["tip"][0], box["y"] + tip["tip"][1]
        self.page.mouse.move(x, y)
        self.page.mouse.down()
        self.page.mouse.move(x + tip["u"][0] * pixels, y + tip["u"][1] * pixels, steps=8)
        self.page.mouse.up()
        return tip

    def test_dragging_a_transform_arrow_moves_the_node_and_keeps_the_other_components(self):
        self.handle_scene("transform", {"translate": [0.5, 1.0, 2.0]}, "h")
        self.assertEqual(sorted(t["id"] for t in self.page.evaluate("minihou.preview.handleTips()")),
                         ["translate_x", "translate_y", "translate_z"])
        tip = self.drag_handle("translate_x", 60)
        self.assertTrue(until(lambda: abs(self.s.graph.nodes["h"].params.get("translate", [0, 0, 0])[0] - (0.5 + 60 / tip["ppu"])) < 0.15))   # 60 px along the arrow, in world units
        t = self.s.graph.nodes["h"].params["translate"]
        self.assertEqual((t[1], t[2]), (1.0, 2.0))                                          # the other components are untouched
        self.assertTrue(until(lambda: self.s.status()["ok"]))
        self.page.wait_for_function("minihou.preview.handleTips().length === 3")             # the arrows follow the moved result

    def test_dragging_an_extrude_arrow_clamps_and_works_for_plain_numbers(self):
        from minihou import Evaluator
        graph = Evaluator().cook(self.s.graph) if False else None
        self.handle_scene("polyextrude", {"group": "0", "distance": 0.2}, "h")
        tip = self.drag_handle("distance", 50)
        self.assertTrue(until(lambda: abs(self.s.graph.nodes["h"].params.get("distance", 0.2) - 0.2) > 0.05))
        d = self.s.graph.nodes["h"].params["distance"]
        self.assertAlmostEqual(abs(d - 0.2), 50 / tip["ppu"], delta=0.2)
        self.assertIsInstance(d, float)

    def test_transform_tools_rotate_ring_scale_cubes_and_uniform_square(self):
        import math
        self.handle_scene("transform", {"translate": [0.5, 1.0, 2.0], "rotate": [0, 0, 10], "scale": [1, 2, 3]}, "h")
        self.assertFalse(self.page.locator("#pv-xform").evaluate("e => e.classList.contains('hidden')"))
        self.page.click('#pv-xform [data-tool="rotate"]')                                   # the tool buttons swap which handles show
        self.assertEqual(sorted(t["id"] for t in self.page.evaluate("minihou.preview.handleTips()")), ["rotate_x", "rotate_y", "rotate_z"])
        box = self.page.locator("#pv-canvas").bounding_box()
        a = self.page.evaluate("minihou.preview.ringScreen('rotate_z', 0.6)")
        b = self.page.evaluate("minihou.preview.ringScreen('rotate_z', 1.1)")                # half a radian further round the ring
        self.page.mouse.move(box["x"] + a[0], box["y"] + a[1])
        self.page.mouse.down()
        self.page.mouse.move(box["x"] + b[0], box["y"] + b[1], steps=8)
        self.page.mouse.up()
        self.assertTrue(until(lambda: abs(self.s.graph.nodes["h"].params.get("rotate", [0, 0, 10])[2] - (10 + math.degrees(0.5))) < 1.0))   # the angle followed the pointer around the ring
        r = self.s.graph.nodes["h"].params["rotate"]
        self.assertEqual((r[0], r[1]), (0.0, 0.0))
        # scale cubes: a pixel drag multiplies that component only
        self.page.keyboard.press("Escape")
        self.page.mouse.move(box["x"] + 20, box["y"] + 20)
        self.page.keyboard.press("r")                                                        # R: scale tool (nothing is selected in the view)
        ids = sorted(t["id"] for t in self.page.evaluate("minihou.preview.handleTips()"))
        self.assertEqual(ids, ["scale_x", "scale_y", "scale_z", "uniform_scale"])
        self.drag_handle("scale_y", 64)                                                      # +64 px: times e
        self.assertTrue(until(lambda: abs(self.s.graph.nodes["h"].params.get("scale", [1, 2, 3])[1] - 2 * math.e) < 0.3))   # (the last value arrives after the drag)
        sc = self.s.graph.nodes["h"].params["scale"]
        self.assertEqual((sc[0], sc[2]), (1.0, 3.0))
        self.assertTrue(until(lambda: self.s.status()["ok"]))
        self.drag_handle("uniform_scale", 60)                                                # the square: drag up and to the right
        self.assertTrue(until(lambda: self.s.graph.nodes["h"].params.get("uniform_scale", 1.0) > 1.2))
        self.page.keyboard.press("w")
        self.assertEqual(len(self.page.evaluate("minihou.preview.handleTips()")), 3)         # W: back to the move arrows

    def test_tool_buttons_are_hidden_for_nodes_without_transform_handles(self):
        self.handle_scene("polyextrude", {"group": "0", "distance": 0.2}, "h")
        self.assertTrue(self.page.locator("#pv-xform").evaluate("e => e.classList.contains('hidden')"))
        self.assertEqual(len(self.page.evaluate("minihou.preview.handleTips()")), 1)         # its arrow is always shown

    # ---- the Blender bar: add-on zip to drag, install, open Blender
    def stub_blenders(self, found, install=None, launch=None):
        from minihou import blender_setup
        orig = (blender_setup.list_blenders, blender_setup.install_addon, blender_setup.launch_blender)
        blender_setup.list_blenders = lambda *a, **k: found
        if install:
            blender_setup.install_addon = install
        if launch:
            blender_setup.launch_blender = launch
        self.addCleanup(lambda: (setattr(blender_setup, "list_blenders", orig[0]), setattr(blender_setup, "install_addon", orig[1]),
                                 setattr(blender_setup, "launch_blender", orig[2])))

    def test_blender_bar_offers_a_draggable_zip_when_nothing_is_connected(self):
        self.stub_blenders([])
        self.page.reload()
        self.page.wait_for_selector("#blender-bar:not(.hidden)")
        chip = self.page.locator("#bb-zip")
        self.assertEqual(chip.get_attribute("href"), "/addon/minihou_link.zip")
        self.assertEqual(chip.get_attribute("draggable"), "true")
        got = self.page.evaluate("""() => {
            const dt = new DataTransfer();
            document.querySelector('#bb-zip').dispatchEvent(new DragEvent('dragstart', { dataTransfer: dt, bubbles: true, cancelable: true }));
            return [dt.getData('DownloadURL'), dt.getData('text/uri-list')];
        }""")
        self.assertTrue(got[0].startswith("application/zip:minihou_link.zip:http://127.0.0.1:"), got)
        self.assertTrue(got[0].endswith("/addon/minihou_link.zip"))
        self.page.wait_for_function("document.querySelector('#bb-text').textContent.includes('No Blender found')")
        self.assertIn("No Blender found", self.page.inner_text("#bb-text"))
        self.assertTrue(self.page.locator("#bb-install").is_disabled())                        # nothing to install into: the zip is the way
        self.assertTrue(self.page.locator("#bb-open").is_disabled())
        with self.page.expect_download() as dl:
            chip.click()                                                                       # a click downloads it
        self.assertEqual(dl.value.suggested_filename, "minihou_link.zip")

    def test_blender_bar_installs_and_launches_and_can_be_hidden_and_recalled(self):
        import types
        self.web.link = types.SimpleNamespace(port=4321, clients=[])                           # the harness runs without a live link
        calls = []
        self.stub_blenders([{"path": "/b/5.1/blender", "version": "5.1", "supported": True},
                            {"path": "/b/4.2/blender", "version": "4.2", "supported": True}],
                           install=lambda exe, **k: (calls.append(("install", exe)), {"ok": True, "message": "Installed and enabled."})[1],
                           launch=lambda exe, port, host="127.0.0.1": calls.append(("launch", exe, port)))
        self.page.reload()
        self.page.wait_for_selector("#blender-bar:not(.hidden)")
        self.page.wait_for_selector("#bb-pick:not(.hidden)")                                   # two installs: pick one
        self.page.select_option("#bb-pick", "1")
        self.page.click("#bb-install")
        self.page.wait_for_function("document.querySelector('#bb-msg').innerText.includes('Installed')")
        self.page.click("#bb-open")
        self.assertTrue(until(lambda: len(calls) == 2))
        self.assertEqual(calls[0], ("install", "/b/4.2/blender"))
        self.assertEqual(calls[1], ("launch", "/b/4.2/blender", 4321))
        # the page content is not pushed below the bar: the graph pane starts under it
        self.assertGreaterEqual(self.page.locator("#canvas").bounding_box()["y"], self.page.locator("#blender-bar").bounding_box()["y"] + 30)
        self.page.click("#bb-close")
        self.assertTrue(self.page.locator("#blender-bar").is_hidden())
        self.page.reload()
        self.page.wait_for_selector('.node[data-id="out"]')
        self.assertTrue(self.page.locator("#blender-bar").is_hidden())                         # stays hidden after a reload
        self.page.click("#blender-status")                                                     # the Blender indicator brings it back
        self.assertFalse(self.page.locator("#blender-bar").is_hidden())

    def test_a_library_node_can_declare_a_handle_and_dragging_it_edits_the_parameter(self):
        if not self.page.evaluate("!!document.createElement('canvas').getContext('webgl2')"):
            self.skipTest("this Chromium has no WebGL2")
        name = f"e2e_pole_{os.getpid()}"
        os.makedirs(os.path.join(self.dir, "nodes"))
        with open(os.path.join(self.dir, "nodes", "pole.py"), "w", encoding="utf-8") as f:
            f.write("import numpy as np\nfrom minihou.geometry import Geometry\n"
                    "from minihou.sdk import Float, Handle, Int, node\n\n"
                    f"@node({name!r}, params=[Float('height', 2.0, min=0.0), Int('count', 3, min=1)],\n"
                    "      handles=[Handle('height', axis='z', at='origin', reach=True, min=0.0), Handle('count', axis='x', at='origin', scale=10.0)])\n"
                    f"def {name}(height, count):\n"
                    "    P = np.array([[0, 0, 0], [0, 0, height]], np.float32)\n"
                    "    return Geometry.points_only(P)\n")
        self.page.wait_for_function(f"window.minihou.S.types[{name!r}] !== undefined")
        graph = {"version": 1, "nodes": {"p": {"type": name, "pos": [0, 0]}, "o": {"type": "output", "inputs": {"geo": "p"}, "pos": [0, 110]}},
                 "display": "o"}
        self.s.edit([{"op": "replace", "graph": graph}])
        self.page.wait_for_selector('.node[data-id="p"]')
        self.node("p").locator(".title").click()
        self.page.wait_for_function("minihou.preview.handleTips().length === 2")
        ppu = next(t for t in self.page.evaluate("minihou.preview.handleTips()") if t["id"] == "height")["ppu"]
        self.drag_handle("height", int(0.5 * ppu))                                           # half a world unit up the arrow
        self.assertTrue(until(lambda: abs(self.s.graph.nodes["p"].params.get("height", 2.0) - 2.5) < 0.1))   # (the last value arrives after the drag)
        ppu = next(t for t in self.page.evaluate("minihou.preview.handleTips()") if t["id"] == "count")["ppu"]
        self.drag_handle("count", int(0.68 * ppu))                                           # an integer parameter moves in whole steps
        self.assertTrue(until(lambda: self.s.graph.nodes["p"].params.get("count", 3) != 3))
        count = self.s.graph.nodes["p"].params["count"]
        self.assertIsInstance(count, int)
        self.assertGreater(count, 3)

    def test_normals_overlay_toggle_draws_each_faces_normal_from_its_winding(self):
        self.box_scene()
        self.assertFalse(self.page.locator("#pv-normals").is_checked())
        self.page.check("#pv-normals")
        self.assertTrue(self.page.evaluate("minihou.preview.opt.normals"))
        self.page.wait_for_function("minihou.preview.normalCount === 6")
        seg = self.page.evaluate("minihou.preview.normalSegments()")
        self.assertEqual(len(seg), 36)
        outward = 0
        for i in range(6):
            c, t = seg[6 * i:6 * i + 3], seg[6 * i + 3:6 * i + 6]
            d = [t[k] - c[k] for k in range(3)]
            length = sum(x * x for x in d) ** 0.5
            self.assertAlmostEqual(length, 0.04 * (12 ** 0.5), delta=1e-3)                  # 4% of the bounds' diagonal
            outward += sum(c[k] * d[k] for k in range(3)) > 0                                # centre of a box face . normal > 0: pointing out
        self.assertEqual(outward, 6)
        self.page.uncheck("#pv-normals")
        self.assertFalse(self.page.evaluate("minihou.preview.opt.normals"))

    def test_normals_overlay_shows_flipped_faces_pointing_inwards(self):
        graph = {"version": 1, "nodes": {"b": {"type": "box", "params": {"size": [2, 2, 2]}, "pos": [0, 0]},
                                         "r": {"type": "reverse", "inputs": {"geo": "b"}, "pos": [0, 110]},
                                         "o": {"type": "output", "inputs": {"geo": "r"}, "pos": [0, 220]}}, "display": "o"}
        if not self.page.evaluate("!!document.createElement('canvas').getContext('webgl2')"):
            self.skipTest("this Chromium has no WebGL2")
        self.s.edit([{"op": "replace", "graph": graph}])
        self.page.wait_for_selector('.node[data-id="r"]')
        self.node("r").locator(".title").click()
        self.page.wait_for_function("minihou.preview.tris === 12")
        self.page.check("#pv-normals")
        self.page.wait_for_function("minihou.preview.normalCount === 6")
        seg = self.page.evaluate("minihou.preview.normalSegments()")
        inward = sum(sum(seg[6 * i + k] * (seg[6 * i + 3 + k] - seg[6 * i + k]) for k in range(3)) < 0 for i in range(6))
        self.assertEqual(inward, 6)

    # ---- the timeline under the 3-D view
    def test_timeline_steps_scrubs_and_jumps_within_the_playback_range(self):
        self.page.fill("#tl-end", "10")
        self.page.press("#tl-end", "Tab")
        self.page.click("#tl-next")
        self.assertTrue(until(lambda: self.s.frame == 2.0))
        self.page.click("#tl-last")
        self.assertTrue(until(lambda: self.s.frame == 10.0))
        self.page.click("#tl-next")                                                              # stays inside the range
        self.page.click("#tl-back")
        self.assertTrue(until(lambda: self.s.frame == 9.0))
        self.page.fill("#tl-slider", "4")
        self.assertTrue(until(lambda: self.s.frame == 4.0))
        self.page.click("#tl-first")
        self.assertTrue(until(lambda: self.s.frame == 1.0))
        self.page.fill("#tl-frame", "7")
        self.page.press("#tl-frame", "Enter")
        self.assertTrue(until(lambda: self.s.frame == 7.0))
        self.assertEqual(self.page.input_value("#frame-input"), "7")                              # the top bar field follows

    def test_play_runs_in_real_time_loops_and_pauses(self):
        self.page.fill("#tl-start", "1"); self.page.press("#tl-start", "Tab")
        self.page.fill("#tl-end", "6"); self.page.press("#tl-end", "Tab")
        self.page.fill("#tl-fps", "60"); self.page.press("#tl-fps", "Tab")
        self.assertTrue(until(lambda: self.s.fps == 60.0))                                       # the engine's @Time follows the fps
        seen = []
        self.page.click("#tl-play")
        self.assertEqual(self.page.inner_text("#tl-play"), "⏸")
        end = time.monotonic() + 6
        while time.monotonic() < end and not (6.0 in seen and seen[-1] < 6.0 and seen.index(6.0) < len(seen) - 1):
            if not seen or self.s.frame != seen[-1]:
                seen.append(self.s.frame)
            time.sleep(0.005)
        self.assertIn(6.0, seen, seen)                                                           # reached the end...
        self.assertLess(seen[-1], 6.0, seen)                                                     # ...and looped back
        self.assertTrue(all(1.0 <= f <= 6.0 for f in seen), seen)
        self.page.click("#tl-play")                                                              # pause
        self.assertEqual(self.page.inner_text("#tl-play"), "▶")
        stopped = self.page.input_value("#tl-frame")
        time.sleep(0.4)
        self.assertEqual(self.page.input_value("#tl-frame"), stopped)
        self.assertEqual(float(stopped), self.s.frame)

    def test_play_without_loop_stops_at_the_end_and_space_toggles_over_the_view(self):
        self.page.fill("#tl-end", "5"); self.page.press("#tl-end", "Tab")
        self.page.fill("#tl-fps", "120"); self.page.press("#tl-fps", "Tab")
        self.page.click("#tl-loop")                                                              # loop off
        self.assertFalse(self.page.evaluate("document.querySelector('#tl-loop').classList.contains('on')"))
        box = self.page.locator("#pv-canvas").bounding_box()
        self.page.mouse.move(box["x"] + 30, box["y"] + 30)
        self.page.keyboard.press("Space")
        self.assertTrue(until(lambda: self.s.frame == 5.0, timeout=6))
        self.page.wait_for_function("document.querySelector('#tl-play').textContent === '▶'")   # it stopped by itself
        time.sleep(0.3)
        self.assertEqual(self.s.frame, 5.0)
        self.page.keyboard.press("ArrowLeft")
        self.assertTrue(until(lambda: self.s.frame == 4.0))
        self.page.keyboard.press("Shift+ArrowLeft")
        self.assertTrue(until(lambda: self.s.frame == 1.0))

    def test_timeline_range_is_remembered_and_the_preview_follows_the_frame(self):
        self.page.fill("#tl-end", "48"); self.page.press("#tl-end", "Tab")
        self.page.reload()
        self.page.wait_for_selector('.node[data-id="out"]')
        self.assertEqual(self.page.input_value("#tl-end"), "48")
        # a time-dependent graph changes the geometry the viewer shows from frame to frame
        graph = {"version": 1, "nodes": {"b": {"type": "box", "params": {"size": [2, 2, 2]}, "pos": [0, 0]},
                                         "w": {"type": "wrangle", "inputs": {"geo": "b"}, "pos": [0, 110],
                                               "params": {"snippet": "@P.z += @Frame * 0.1;", "run_over": "points"}},
                                         "o": {"type": "output", "inputs": {"geo": "w"}, "pos": [0, 220]}}, "display": "o"}
        if not self.page.evaluate("!!document.createElement('canvas').getContext('webgl2')"):
            self.skipTest("this Chromium has no WebGL2")
        self.s.edit([{"op": "replace", "graph": graph}])
        self.page.wait_for_selector('.node[data-id="w"]')
        self.node("w").locator(".title").click()
        self.page.wait_for_function("minihou.preview.tris === 12")
        self.page.click("#tl-last")
        self.assertTrue(until(lambda: self.s.frame == 48.0))
        self.page.wait_for_function("Math.abs(minihou.preview.bounds()[1][2] - 5.8) < 0.01")      # frame 48: the box rose by 4.8
        self.page.click("#tl-first")
        self.page.wait_for_function("Math.abs(minihou.preview.bounds()[1][2] - 1.1) < 0.01")     # frame 1: by 0.1

    # ---- piece selection and the auto-inserted transform
    def two_boxes(self):
        if not self.page.evaluate("!!document.createElement('canvas').getContext('webgl2')"):
            self.skipTest("this Chromium has no WebGL2")
        graph = {"version": 1, "nodes": {"a": {"type": "box", "params": {"size": [1, 1, 1], "center": [-2, 0, 0]}, "pos": [0, 0]},
                                         "b": {"type": "box", "params": {"size": [1, 1, 1], "center": [2, 0, 0]}, "pos": [150, 0]},
                                         "m": {"type": "merge", "inputs": {"geos": ["a", "b"]}, "pos": [60, 110]},
                                         "o": {"type": "output", "inputs": {"geo": "m"}, "pos": [60, 220]}}, "display": "o"}
        self.s.edit([{"op": "replace", "graph": graph}])
        self.page.wait_for_selector('.node[data-id="m"]')
        self.node("m").locator(".title").click()
        self.page.wait_for_function("minihou.preview.tris === 24")
        self.page.wait_for_function("minihou.preview.points === 16")
        from minihou import Evaluator
        self.merged = Evaluator().cook(self.s.graph, "m")

    def second_box_top(self):
        g = self.merged
        vp, offs = g.topology.vertex_point.astype(int), g.topology.prim_offsets
        return max(range(6, 12), key=lambda p: g.P[vp[offs[p]:offs[p + 1]]].mean(0)[2])

    def test_piece_mode_selects_a_whole_object_and_the_gizmo_adds_a_transform_when_dragged(self):
        import math
        from minihou import Evaluator
        self.two_boxes()
        self.page.click('#preview [data-mode="piece"]')
        face = self.second_box_top()
        self.page.mouse.click(*self.view_xy("face", face))
        sel = self.selection()
        self.assertEqual(sel["mode"], "piece")
        self.assertEqual(sorted(sel["faces"]), list(range(6, 12)))                              # all six faces of that box, nothing of the other
        self.assertEqual(self.page.locator("#pv-selinfo").inner_text(), "6 faces in the selected pieces")
        info = self.page.evaluate("minihou.preview.selectionInfo()")
        self.assertEqual(info["points"], list(range(8, 16)))
        self.assertEqual([round(v, 3) for v in info["center"]], [2.0, 0.0, 0.0])
        # a drag on the move gizmo adds the transform: group = the selected points, pivot = their centre
        ids = sorted(t["id"] for t in self.page.evaluate("minihou.preview.handleTips()"))
        self.assertEqual(ids, ["translate_x", "translate_y", "translate_z"])
        ppu = next(t for t in self.page.evaluate("minihou.preview.handleTips()") if t["id"] == "translate_z")["ppu"]
        self.assertFalse([n for n in self.s.graph.nodes.values() if n.type == "transform"])
        self.drag_handle("translate_z", int(0.75 * ppu))
        self.assertTrue(until(lambda: [n for n in self.s.graph.nodes.values() if n.type == "transform"]))
        tid = next(i for i, n in self.s.graph.nodes.items() if n.type == "transform")
        n = self.s.graph.nodes[tid]
        self.assertEqual(n.inputs, {"geo": "m"})
        self.assertEqual(n.params["group"], "8-15")
        self.assertEqual([round(v, 3) for v in n.params["pivot"]], [2.0, 0.0, 0.0])
        self.assertTrue(until(lambda: abs(self.s.graph.nodes[tid].params.get("translate", [0, 0, 0])[2] - 0.75) < 0.1))
        self.assertEqual(self.s.graph.display, "o")
        self.assertEqual(self.s.graph.nodes["o"].inputs["geo"], tid)                            # spliced in before the output
        out = Evaluator().cook(self.s.graph, tid)
        self.assertAlmostEqual(float(out.P[8:, 2].mean()), 0.75, delta=0.1)                     # the second box moved up...
        self.assertAlmostEqual(float(out.P[:8, 2].mean()), 0.0, places=5)                       # ...the first one did not
        # the new node is now the viewed one and shows its own handles at the moved points; selecting again edits the same node
        self.page.wait_for_function(f"document.querySelector('#pv-node').innerText.includes({tid!r})")
        self.page.click('#preview [data-mode="piece"]')
        self.page.mouse.click(*self.view_xy("face", face))
        self.assertEqual(sorted(self.selection()["faces"]), list(range(6, 12)))
        before = self.s.graph.nodes[tid].params["translate"][2]
        ppu = next(t for t in self.page.evaluate("minihou.preview.handleTips()") if t["id"] == "translate_z")["ppu"]
        self.drag_handle("translate_z", int(0.5 * ppu))
        self.assertTrue(until(lambda: self.s.graph.nodes[tid].params["translate"][2] > before + 0.3))
        self.assertEqual(len([x for x in self.s.graph.nodes.values() if x.type == "transform"]), 1)   # no second transform

    def test_rotate_and_scale_tools_of_a_selection_create_the_transform_too(self):
        import math
        from minihou import Evaluator
        self.two_boxes()
        self.page.click('#preview [data-mode="piece"]')
        self.page.mouse.click(*self.view_xy("face", self.second_box_top()))
        self.page.click('#pv-xform [data-tool="rotate"]')
        self.assertEqual(sorted(t["id"] for t in self.page.evaluate("minihou.preview.handleTips()")), ["rotate_x", "rotate_y", "rotate_z"])
        box = self.page.locator("#pv-canvas").bounding_box()
        a = self.page.evaluate("minihou.preview.ringScreen('rotate_z', 0.6)")
        b = self.page.evaluate("minihou.preview.ringScreen('rotate_z', 1.1)")
        self.page.mouse.move(box["x"] + a[0], box["y"] + a[1])
        self.page.mouse.down()
        self.page.mouse.move(box["x"] + b[0], box["y"] + b[1], steps=8)
        self.page.mouse.up()
        self.assertTrue(until(lambda: any(n.type == "transform" for n in self.s.graph.nodes.values())))
        tid = next(i for i, n in self.s.graph.nodes.items() if n.type == "transform")
        self.assertTrue(until(lambda: abs(self.s.graph.nodes[tid].params.get("rotate", [0, 0, 0])[2] - math.degrees(0.5)) < 1.0))
        self.assertEqual(self.s.graph.nodes[tid].params["group"], "8-15")
        out = Evaluator().cook(self.s.graph, tid)
        np_center = out.P[8:].astype("float64").mean(0)
        self.assertAlmostEqual(float(np_center[0]), 2.0, delta=1e-4)                            # turned about its own centre, so it stays put
        self.assertGreater(abs(float(out.P[8, 0]) - float(self.merged.P[8, 0])) + abs(float(out.P[8, 1]) - float(self.merged.P[8, 1])), 0.05)

    def test_a_point_selection_also_gets_the_gizmo_and_piece_box_select_takes_whole_pieces(self):
        self.two_boxes()
        self.page.click('#preview [data-mode="piece"]')
        box = self.page.locator("#pv-canvas").bounding_box()
        x0, y0 = self.view_xy("point", 8)
        x1, y1 = self.view_xy("point", 15)
        self.page.mouse.move(box["x"] + box["width"] / 2 + 5, box["y"] + 15)                    # a box over the right half only
        self.page.mouse.down()
        self.page.mouse.move(box["x"] + box["width"] - 15, box["y"] + box["height"] - 15, steps=6)
        self.page.mouse.up()
        self.assertEqual(sorted(self.selection()["faces"]), list(range(6, 12)))                 # whole piece, not just the faces inside the box
        self.page.click('#preview [data-mode="point"]')
        self.page.mouse.click(*self.view_xy("point", 8))
        self.assertEqual(self.selection()["points"], [8])
        info = self.page.evaluate("minihou.preview.selectionInfo()")
        self.assertEqual(info["points"], [8])
        self.assertEqual(len(self.page.evaluate("minihou.preview.handleTips()")), 3)            # move arrows at the point
        self.drag_handle("translate_x", 40)
        self.assertTrue(until(lambda: any(n.type == "transform" and n.params.get("group") == "8" for n in self.s.graph.nodes.values())))

    # -- the "C" pie menu ---------------------------------------------------------------
    def pie_slot_xy(self, slot, radius=104):
        import math
        st = self.page.evaluate("MiniPie.state()")
        a = math.radians(slot * 45)
        return st["x"] + radius * math.sin(a), st["y"] - radius * math.cos(a)

    def pie_labels(self):
        return self.page.eval_on_selector_all("#pie g[data-slot] text.pie-label", "els => els.map(e => e.textContent.trim())")

    def test_pie_menu_hold_flick_release_inserts_a_tool_after_the_selected_node(self):
        self.s.edit([{"op": "replace", "graph": {"version": 1, "nodes": {"b": {"type": "box", "pos": [0, 0]},
                      "o": {"type": "output", "inputs": {"geo": "b"}, "pos": [0, 110]}}, "display": "o"}}])
        self.page.wait_for_selector('.node[data-id="b"]')
        self.node("b").locator(".title").click()
        self.page.mouse.move(*self.empty_spot())
        self.page.keyboard.down("c")
        self.page.wait_for_selector("#pie:not(.hidden)")
        self.assertEqual(self.pie_labels(), ["Adjust", "Deform", "Polygons", "Group", "Create", "Build", "Edges", "Modify"])
        time.sleep(0.35)
        self.page.mouse.move(*self.pie_slot_xy(1), steps=4)                                    # up and to the right: Deform
        self.page.keyboard.up("c")
        self.page.wait_for_function("MiniPie.state() && MiniPie.state().page === 'deform'")
        self.assertEqual(self.pie_labels(), ["Lattice", "Mountain", "Bend", "Linear Taper", "Squash and Stretch", "Twist", "»"])
        self.page.mouse.click(*self.pie_slot_xy(5))                                              # lower left: Twist
        self.page.wait_for_selector("#pie", state="hidden")
        self.assertTrue(until(lambda: any(n.type == "twist" for n in self.s.graph.nodes.values())))
        tid = next(i for i, n in self.s.graph.nodes.items() if n.type == "twist")
        self.assertEqual(self.s.graph.nodes[tid].inputs, {"geo": "b"})
        self.assertEqual(self.s.graph.nodes["o"].inputs["geo"], tid)
        self.page.wait_for_selector(f'.node[data-id="{tid}"].selected')

    def test_pie_menu_pages_digits_back_escape_and_disabled_tools(self):
        self.page.mouse.move(*self.empty_spot())
        self.page.keyboard.press("c")
        self.page.wait_for_selector("#pie:not(.hidden)")
        self.page.keyboard.press("5")                                                           # slot 5 = Create
        self.page.wait_for_function("MiniPie.state() && MiniPie.state().page === 'create'")
        self.assertEqual(self.pie_labels(), ["Null", "Circle", "Tube", "Sphere", "Curve", "Grid", "Box", "»"])
        self.page.mouse.click(*self.pie_slot_xy(7))                                              # the » arrow: a second page
        self.page.wait_for_function("MiniPie.state().page === 'create2'")
        self.assertIn("Torus", self.pie_labels())
        st = self.page.evaluate("MiniPie.state()")
        self.page.mouse.click(st["x"], st["y"])                                                  # the middle goes back to the main pie
        self.page.wait_for_function("MiniPie.state().page === 'main'")
        self.page.mouse.click(*self.pie_slot_xy(0))                                              # Adjust
        self.page.wait_for_function("MiniPie.state().page === 'adjust'")
        before = len(self.s.graph.nodes)
        self.page.mouse.click(*self.pie_slot_xy(4))                                              # Sculpt: not available, says so
        self.page.wait_for_selector("#pie", state="hidden")
        self.assertIn("Sculpting", self.page.locator("#toast").inner_text())
        self.assertEqual(len(self.s.graph.nodes), before)
        self.page.keyboard.press("c")
        self.page.wait_for_selector("#pie:not(.hidden)")
        self.page.keyboard.press("Escape")
        self.page.wait_for_selector("#pie", state="hidden")
        self.page.keyboard.press("Tab")                                                          # typing a c in a text field is just typing
        self.page.keyboard.type("c")
        self.assertTrue(self.page.locator("#pie").is_hidden())
        self.page.keyboard.press("Escape")

    def test_pie_menu_adds_a_source_unwired_and_fills_a_tool_from_the_3d_selection(self):
        self.two_boxes()
        self.page.mouse.move(*self.empty_spot())
        self.page.keyboard.press("c")
        self.page.keyboard.press("5")
        self.page.mouse.click(*self.pie_slot_xy(5))                                              # Grid
        self.assertTrue(until(lambda: any(n.type == "grid" for n in self.s.graph.nodes.values())))
        gid = next(i for i, n in self.s.graph.nodes.items() if n.type == "grid")
        self.assertEqual(self.s.graph.nodes[gid].inputs, {})
        self.page.wait_for_selector(f'.node[data-id="{gid}"].selected')
        self.node("m").locator(".title").click()
        self.page.wait_for_function("minihou.preview.tris === 24")
        self.page.click('#preview [data-mode="face"]')
        face = self.second_box_top()
        self.page.mouse.click(*self.view_xy("face", face))
        self.assertEqual(self.selection()["faces"], [face])
        box = self.page.locator("#pv-canvas").bounding_box()
        self.page.mouse.move(box["x"] + 40, box["y"] + 60)
        self.page.keyboard.press("c")
        self.page.keyboard.press("3")                                                           # Polygons
        self.page.mouse.click(*self.pie_slot_xy(2))                                              # PolyExtrude
        self.assertTrue(until(lambda: any(n.type == "polyextrude" for n in self.s.graph.nodes.values())))
        n = next(n for n in self.s.graph.nodes.values() if n.type == "polyextrude")
        self.assertEqual(n.inputs, {"geo": "m"})
        self.assertEqual(n.params["group"], str(face))
        self.assertEqual(n.params["distance"], 0.3)

    # -- the Add Node menu -----------------------------------------------------------------
    def open_menu(self):
        self.page.mouse.move(*self.empty_spot())
        self.page.keyboard.press("Tab")
        self.page.wait_for_selector("#tabmenu:not(.hidden)")
        self.page.wait_for_function("document.activeElement && document.activeElement.id === 'tab-search'")

    def menu_heads(self):
        return self.page.eval_on_selector_all("#tab-list .cat.head", "els => els.map(e => e.dataset.group)")

    def test_add_menu_lists_groups_folded_and_opens_them_with_enter_or_arrows(self):
        self.page.evaluate("localStorage.clear()")
        self.page.reload()
        self.page.wait_for_selector('.node[data-id="out"]')
        self.open_menu()
        heads = self.menu_heads()
        self.assertEqual(heads[0], "Create")
        self.assertEqual(heads[-1], "Utility & I/O")
        self.assertIn("Fracture & RBD", heads)
        self.assertEqual(self.page.locator("#tab-list .it").first.get_attribute("data-name"), "box")       # Create starts open, in menu order
        self.assertEqual(self.page.locator("#tab-list .cat.head.active").count(), 0)
        names = self.page.eval_on_selector_all("#tab-list .it", "els => els.map(e => e.dataset.name)")
        self.assertNotIn("voronoi_fracture", names)                                                            # a closed group lists nothing
        # click a closed group's header: it opens and the choice is remembered
        self.page.locator('#tab-list .cat.head[data-group="Fracture & RBD"]').dispatch_event("mousedown")
        self.page.wait_for_selector('#tab-list .it[data-name="voronoi_fracture"]')
        self.page.keyboard.press("Escape")
        self.open_menu()
        self.page.wait_for_selector('#tab-list .it[data-name="voronoi_fracture"]')
        # arrow keys reach the headers; Left folds the group of the highlighted node
        self.page.keyboard.press("ArrowLeft")
        self.page.wait_for_function("!document.querySelector('#tab-list .it[data-name=\"box\"]')")
        self.page.keyboard.press("ArrowRight")
        self.page.wait_for_selector('#tab-list .it[data-name="box"]')
        self.page.keyboard.press("Escape")

    def test_add_menu_search_ranks_by_known_names_and_shows_the_description(self):
        self.page.evaluate("localStorage.clear()")
        self.page.reload()
        self.page.wait_for_selector('.node[data-id="out"]')
        self.open_menu()
        self.page.keyboard.type("mountain")
        self.page.wait_for_selector('#tab-list .it.active[data-name="noise"]')
        self.assertIn("Deform", self.page.locator("#tab-list .it.active small").inner_text())             # the group shows beside a result
        self.assertIn("Noise", self.page.inner_text("#tab-doc"))
        self.assertIn("in:", self.page.inner_text("#tab-doc"))
        self.page.keyboard.press("Control+a")
        self.page.keyboard.type("qqzzxx")
        self.page.wait_for_selector("#tab-list .cat:has-text('No node matches')")
        self.page.keyboard.press("Control+a")
        self.page.keyboard.type("extrude")
        self.page.wait_for_selector('#tab-list .it.active[data-name="polyextrude"] mark')
        self.page.keyboard.press("Enter")
        self.assertTrue(until(lambda: any(n.type == "polyextrude" for n in self.s.graph.nodes.values())))

    def test_add_menu_remembers_recent_and_favourite_nodes(self):
        self.page.evaluate("localStorage.clear()")
        self.page.reload()
        self.page.wait_for_selector('.node[data-id="out"]')
        self.open_menu()
        self.page.locator('#tab-list .it[data-name="box"] .star').dispatch_event("mousedown")
        self.page.wait_for_selector('#tab-list .cat.special:has-text("Favourites")')
        self.assertEqual(self.page.locator("#tab-list .it").first.get_attribute("data-name"), "box")
        self.page.keyboard.type("polybevel")
        self.page.keyboard.press("Enter")
        self.assertTrue(until(lambda: any(n.type == "polybevel" for n in self.s.graph.nodes.values())))
        self.page.reload()                                                                                      # all of it survives a reload
        self.page.wait_for_selector('.node[data-id="out"]')
        self.open_menu()
        self.page.wait_for_selector('#tab-list .cat.special:has-text("Recent")')
        self.assertEqual(self.page.eval_on_selector_all("#tab-list .cat.special", "els => els.map(e => e.dataset.group)"),
                         ["Favourites", "Recent"])
        recent = self.page.eval_on_selector_all("#tab-list .it", "els => els.map(e => e.dataset.name)")
        self.assertEqual(recent[:2], ["box", "polybevel"])
        self.page.keyboard.press("Escape")

    def test_output_groups_section_checkbox_and_name_per_group(self):
        graph = {"version": 1, "nodes": {"b": {"type": "box", "params": {"size": [2, 2, 2]}, "pos": [0, 0]},
                                         "bv": {"type": "polybevel", "inputs": {"geo": "b"}, "params": {"distance": 0.2}, "pos": [0, 110]},
                                         "o": {"type": "output", "inputs": {"geo": "bv"}, "pos": [0, 220]}}, "display": "o"}
        self.s.edit([{"op": "replace", "graph": graph}])
        self.page.wait_for_selector('.node[data-id="bv"]')
        self.node("bv").locator(".title").click()
        self.page.wait_for_selector("#params .ogheader")
        self.assertEqual(self.page.locator("#params .ogheader").inner_text(), "Output Groups")
        pairs = self.page.locator("#params .ogpair")
        self.assertEqual(pairs.count(), 6)                                                       # the six PolyBevel groups
        self.assertEqual([t.strip() for t in pairs.locator(".prow.bool .pl").all_inner_texts()],
                         ["Point Fillet Polygons", "Point Fillet Edges", "Edge Fillet Polygons", "Offset Edges", "Offset Points", "Merged Points"])
        names = [pairs.nth(i).locator("input[type=text]").input_value() for i in range(6)]
        self.assertEqual(names, ["pointfilletpolys", "pointfilletedges", "edgefilletpolys", "offsetedges", "offsetpoints", "mergedpoints"])
        self.assertTrue(all("off" in (pairs.nth(i).get_attribute("class") or "") for i in range(6)))     # all off by default
        row = pairs.nth(2)
        row.locator("input[type=checkbox]").check()
        self.assertTrue(until(lambda: self.s.graph.nodes["bv"].params.get("outgrp_edgefilletpolys") is True))
        self.assertNotIn("off", row.get_attribute("class"))
        row.locator("input[type=text]").fill("strips")
        row.locator("input[type=text]").press("Enter")
        self.assertTrue(until(lambda: self.s.graph.nodes["bv"].params.get("outgrp_edgefilletpolys_name") == "strips"))
        self.page.wait_for_function("document.querySelector('#info-body') && document.querySelector('#info-body').innerText.includes('group_strips')")
        self.assertEqual(self.page.locator("#params .ogpair .pbtn.expose").first.is_visible(), False)    # these are not Blender modifier inputs

    def test_sphere_radius_arrow_clamps_at_zero(self):
        graph = {"version": 1, "nodes": {"s": {"type": "sphere", "params": {"radius": 0.5}}, "o": {"type": "output", "inputs": {"geo": "s"}}},
                 "display": "o"}
        if not self.page.evaluate("!!document.createElement('canvas').getContext('webgl2')"):
            self.skipTest("this Chromium has no WebGL2")
        self.s.edit([{"op": "replace", "graph": graph}])
        self.page.wait_for_selector('.node[data-id="s"]')
        self.node("s").locator(".title").click()
        self.page.wait_for_function("minihou.preview.handleTips().length === 1")
        self.drag_handle("radius", -400)                                                    # far inwards: the radius cannot go negative
        self.assertTrue(until(lambda: self.s.graph.nodes["s"].params.get("radius", 0.5) < 0.5))
        self.assertGreaterEqual(self.s.graph.nodes["s"].params["radius"], 0.0)

    def test_frame_slider_scrubs_and_follows_the_frame(self):
        self.page.fill("#frame-slider", "40")                         # fires input events like a drag
        end = time.monotonic() + 5
        while self.s.frame != 40.0 and time.monotonic() < end:
            time.sleep(0.05)
        self.assertEqual(self.s.frame, 40.0)
        self.assertEqual(self.page.input_value("#frame-input"), "40")
        self.page.fill("#frame-input", "600")                         # a later frame widens the slider's range
        self.page.press("#frame-input", "Enter")
        self.page.wait_for_function("Number(document.querySelector('#frame-slider').max) >= 600")
        self.assertEqual(self.page.input_value("#frame-slider"), "600")

    def test_frame_field_sets_the_engine_frame(self):
        self.page.fill("#frame-input", "7")
        self.page.press("#frame-input", "Enter")
        self.page.click("#frame-next")
        end = time.monotonic() + 5
        while self.s.frame != 8.0 and time.monotonic() < end:
            time.sleep(0.02)
        self.assertEqual(self.s.frame, 8.0)
        self.assertEqual(self.errors, [])

    def test_select_and_edit_a_parameter(self):
        self.node("hills").locator(".title").click()
        self.assertEqual(self.page.input_value("#params input.nid"), "hills")
        num = self.page.locator('.prow[data-param="frequency"] input[type=number]')
        num.fill("0.5")
        num.press("Enter")
        self.assertTrue(until(lambda: self.s.graph.nodes["hills"].params.get("frequency") == 0.5))
        self.assertTrue(until(lambda: '"frequency": 0.5' in self.read_file()))   # saved

    def test_tab_menu_adds_wired_node_and_undo(self):
        self.node("hills").locator(".title").click()
        self.page.mouse.move(*self.empty_spot())
        self.page.keyboard.press("Tab")
        self.page.wait_for_selector("#tabmenu:not(.hidden)")
        self.page.keyboard.type("transf")
        self.page.keyboard.press("Enter")
        self.assertTrue(until(lambda: "transform1" in self.s.graph.nodes))
        self.assertEqual(self.s.graph.nodes["transform1"].inputs, {"geo": "hills"})   # wired to the selection
        self.page.wait_for_selector('.node[data-id="transform1"].selected')
        self.page.locator("#canvas").focus()
        self.page.keyboard.press("Control+z")
        self.assertTrue(until(lambda: "transform1" not in self.s.graph.nodes))
        self.page.keyboard.press("Control+Shift+z")
        self.assertTrue(until(lambda: "transform1" in self.s.graph.nodes))

    def test_wire_by_dragging_ports_and_delete_a_wire(self):
        self.page.mouse.dblclick(*self.empty_spot())
        self.page.wait_for_selector("#tabmenu:not(.hidden)")
        self.page.keyboard.type("box")
        self.page.keyboard.press("Enter")
        self.assertTrue(until(lambda: "box1" in self.s.graph.nodes))
        self.page.wait_for_selector('.node[data-id="box1"]')
        self.drag(self.node("box1").locator(".port.out"), self.node("all").locator(".port.in"))
        self.assertTrue(until(lambda: "box1" in self.s.graph.nodes["all"].inputs.get("geos", [])))
        # pick the wire up from the input and drop it on empty space: disconnect
        self.drag(self.node("out").locator(".port.in"), self.page.locator("#canvas"))
        self.page.mouse.move(*self.empty_spot())
        self.assertTrue(until(lambda: "geo" not in self.s.graph.nodes["out"].inputs))
        self.page.wait_for_selector("#errorbar:not(.hidden)")              # the output is now unwired

    def test_display_flag_rename_and_delete(self):
        self.node("points").locator(".flag").click()
        self.assertTrue(until(lambda: self.s.graph.display == "points"))
        self.node("spike").locator(".title").click()
        nid = self.page.locator("#params input.nid")
        nid.fill("spike_src")
        nid.press("Enter")
        self.assertTrue(until(lambda: "spike_src" in self.s.graph.nodes))
        self.assertEqual(self.s.graph.nodes["spike_up"].inputs["geo"], "spike_src")
        self.page.locator("#canvas").focus()
        self.node("spike_src").locator(".title").click()
        self.page.keyboard.press("Delete")
        self.assertTrue(until(lambda: "spike_src" not in self.s.graph.nodes))

    def test_wrangle_error_is_shown_on_the_node(self):
        self.node("ripple").locator(".title").click()
        ta = self.page.locator('.prow[data-param="snippet"] textarea')
        ta.fill("@P.z = nope")
        ta.press("Control+Enter")
        self.page.wait_for_selector('.node[data-id="ripple"].error')
        self.assertIn("nope", self.page.inner_text("#errorbar"))
        self.assertIn("nope", self.page.inner_text("#params .perr"))
        ta.fill("@P.z += 0.1")
        ta.press("Control+Enter")
        self.page.wait_for_selector("#errorbar.hidden", state="attached")

    def test_expose_toggle(self):
        self.node("ripple").locator(".title").click()
        self.node("spike_up").locator(".title").click()
        self.page.locator('.prow[data-param="scale"] .expose').click()
        self.assertTrue(until(lambda: any(e.node == "spike_up" and e.param == "scale" for e in self.s.graph.exposed)))

    def test_edits_from_elsewhere_show_up_live(self):
        """What the MCP server / Blender / a text editor do: the open editor follows without reloading."""
        self.s.edit([{"op": "add", "type": "sphere", "id": "from_ai", "comment": "added by AI"}], origin="mcp")
        self.page.wait_for_selector('.node[data-id="from_ai"]')
        self.assertIn("added by AI", self.node("from_ai").inner_text())
        self.node("hills").locator(".title").click()
        self.s.set_param("Hill Frequency", 0.9)
        self.page.wait_for_function("document.querySelector('.prow[data-param=\"frequency\"] input[type=number]').value == '0.9'")

    def test_new_library_node_appears_live_in_the_tab_menu(self):
        name = f"e2e_lift_{os.getpid()}"
        os.makedirs(os.path.join(self.dir, "nodes"))
        with open(os.path.join(self.dir, "nodes", "lift.py"), "w", encoding="utf-8") as f:
            f.write(f"from minihou.sdk import node\n\n@node({name!r}, inputs=['geo'], category='mine')\n"
                    f"def {name}(geo):\n    return geo.copy()\n")
        self.page.wait_for_function(f"window.minihou.S.types[{name!r}] !== undefined")
        self.page.mouse.dblclick(*self.empty_spot())
        self.page.keyboard.type("e2e_lift")
        self.page.wait_for_selector(f"#tab-list .it:has-text({name!r})")
        self.page.keyboard.press("Enter")
        self.assertTrue(until(lambda: any(n.type == name for n in self.s.graph.nodes.values())))
        with open(os.path.join(self.dir, "nodes", "lift.py"), "a", encoding="utf-8") as f:
            f.write("\nbroken(\n")
        self.page.wait_for_selector("#toast.err")
        self.assertIn("SyntaxError", self.page.inner_text("#toast"))

    def test_edit_system_opens_the_system_graph(self):
        import json as _json
        name = f"e2e_sys_{os.getpid()}"
        os.makedirs(os.path.join(self.dir, "nodes"))
        system = {"version": 1, "doc": "Test system.", "nodes": {
            "inp": {"type": "input", "params": {"name": "src"}},
            "out": {"type": "output", "inputs": {"geo": "inp"}}}, "display": "out"}
        with open(os.path.join(self.dir, "nodes", f"{name}.json"), "w", encoding="utf-8") as f:
            _json.dump(system, f)
        self.page.wait_for_function(f"window.minihou.S.types[{name!r}] !== undefined")
        self.s.edit([{"op": "add", "type": name, "id": "sys1", "inputs": {"src": "ground"}}])
        self.page.wait_for_selector('.node[data-id="sys1"]')
        self.node("sys1").locator(".title").click()
        self.assertIn("Test system.", self.page.inner_text("#params .pdoc"))
        self.page.locator("#params button", has_text="Edit system").click()
        self.assertTrue(until(lambda: self.s.path is not None and self.s.path.name == f"{name}.json"))
        self.page.wait_for_selector('.node[data-id="inp"]')

    def test_double_click_opens_nested_groups_with_a_breadcrumb_path_back(self):
        import json as _json
        a, b = f"e2e_inner_{os.getpid()}", f"e2e_outer_{os.getpid()}"
        os.makedirs(os.path.join(self.dir, "nodes"))
        inner = {"version": 1, "doc": "Inner.", "nodes": {"inp": {"type": "input", "params": {"name": "src"}},
                                                           "out": {"type": "output", "inputs": {"geo": "inp"}}}, "display": "out"}
        outer = {"version": 1, "doc": "Outer.", "nodes": {"inp": {"type": "input", "params": {"name": "src"}},
                                                           "deep": {"type": a, "inputs": {"src": "inp"}},
                                                           "out": {"type": "output", "inputs": {"geo": "deep"}}}, "display": "out"}
        for n, g in ((a, inner), (b, outer)):
            with open(os.path.join(self.dir, "nodes", f"{n}.json"), "w", encoding="utf-8") as f:
                _json.dump(g, f)
        self.page.wait_for_function(f"window.minihou.S.types[{b!r}] !== undefined && window.minihou.S.types[{a!r}] !== undefined")
        self.s.edit([{"op": "add", "type": b, "id": "grp", "inputs": {"src": "ground"}}])
        self.page.wait_for_selector('.node[data-id="grp"].group')                              # a group is marked as one
        self.assertEqual(self.page.locator("#crumbs .crumb").count(), 1)                       # the root graph alone
        self.node("grp").locator(".title").dblclick()
        self.page.wait_for_selector('.node[data-id="deep"]')
        self.assertEqual(self.page.locator("#crumbs .crumb").all_inner_texts(), ["demo", b])
        self.assertTrue(self.page.locator("#crumbs .crumb.cur").inner_text() == b)
        self.node("deep").locator(".title").dblclick()                                         # a group inside a group
        self.page.wait_for_selector('.node[data-id="inp"]')
        self.assertTrue(until(lambda: self.s.path is not None and self.s.path.name == f"{a}.json"))
        self.assertEqual(self.page.locator("#crumbs .crumb").all_inner_texts(), ["demo", b, a])
        self.page.locator('#crumbs .crumb[data-index="1"]').click()                            # back out one level through the crumb
        self.assertTrue(until(lambda: self.s.path is not None and self.s.path.name == f"{b}.json"))
        self.page.wait_for_selector('.node[data-id="deep"]')
        self.assertEqual(self.page.locator("#crumbs .crumb").all_inner_texts(), ["demo", b])
        self.page.keyboard.press("Alt+ArrowUp")                                                # and out to the root with the keyboard
        self.assertTrue(until(lambda: self.s.path is not None and self.s.path.name == "demo.json"))
        self.page.wait_for_selector('.node[data-id="grp"]')
        self.assertEqual(self.page.locator("#crumbs .crumb").all_inner_texts(), ["demo"])
        self.page.reload()                                                                     # the path survives a reload of the tab
        self.page.wait_for_selector('.node[data-id="grp"]')
        self.node("grp").locator(".title").dblclick()
        self.page.wait_for_selector('.node[data-id="deep"]')
        self.page.reload()
        self.page.wait_for_selector('.node[data-id="deep"]')
        self.assertEqual(self.page.locator("#crumbs .crumb").all_inner_texts(), ["demo", b])
        self.page.click("#file-btn")                                                           # opening a file from the menu starts a new path
        self.page.locator('#file-menu .frow[data-path="demo.json"]').click()
        self.page.wait_for_selector('.node[data-id="grp"]')
        self.assertEqual(self.page.locator("#crumbs .crumb").all_inner_texts(), ["demo"])

    def test_graph_text_is_never_html(self):
        payload = '<img src=x onerror="window.__xss=1"><b>bold</b>'
        self.s.edit([{"op": "meta", "id": "hills", "set": {"comment": payload}},
                     {"op": "expose", "id": "ground", "param": "rows", "name": payload}])
        self.page.wait_for_function(f"document.querySelector('.node[data-id=hills] .comment')?.textContent === {payload!r}")
        self.page.locator("#canvas").click(position={"x": 30, "y": 30})       # graph panel lists exposed names
        self.page.wait_for_timeout(300)
        self.assertIsNone(self.page.evaluate("window.__xss"))
        self.assertEqual(self.page.locator("#nodes img, #params img").count(), 0)

    def test_new_file_from_the_file_menu(self):
        self.page.on("dialog", lambda d: d.accept("graphs/fresh.json"))
        self.page.click("#file-btn")
        self.page.locator("#file-menu .fm-foot button", has_text="New graph").click()
        self.assertTrue(until(lambda: self.s.path is not None and self.s.path.name == "fresh.json"))
        self.page.wait_for_selector('.node[data-id="grid1"]')
        self.assertTrue(os.path.exists(os.path.join(self.dir, "graphs", "fresh.json")))
        self.page.click("#file-btn")
        self.page.locator('#file-menu .frow[data-path="demo.json"]').click()
        self.page.wait_for_selector('.node[data-id="hills"]')


    # -- the graph & library browser ------------------------------------------------------
    def open_files(self):
        self.page.click("#file-btn")
        self.page.wait_for_selector("#file-menu:not(.hidden) .fm-list")

    def test_file_browser_searches_lists_folders_recent_and_the_current_graph(self):
        os.makedirs(os.path.join(self.dir, "graphs"), exist_ok=True)
        for name in ("alpha_rock.json", "beta_wall.json"):
            shutil.copyfile(self.path, os.path.join(self.dir, "graphs", name))
        self.open_files()
        self.assertIn("demo.json", self.page.inner_text(".fm-cur"))
        heads = self.page.eval_on_selector_all("#file-menu .fm-list .cat", "els => els.map(e => e.textContent)")
        self.assertIn("Graphs", heads)
        self.assertIn("graphs/", heads)                                                # a folder gets its own heading
        rows = self.page.eval_on_selector_all("#file-menu .frow", "els => els.map(e => e.dataset.path)")
        self.assertEqual(sorted(rows), ["demo.json", "graphs/alpha_rock.json", "graphs/beta_wall.json"])
        self.page.keyboard.type("wall")
        self.page.wait_for_function("document.querySelectorAll('#file-menu .frow').length === 1")
        self.assertEqual(self.page.locator("#file-menu .frow").get_attribute("data-path"), "graphs/beta_wall.json")
        self.assertIn("mark", self.page.locator("#file-menu .frow .nm").inner_html())
        self.page.keyboard.press("Enter")                                                       # Enter opens the highlighted file
        self.assertTrue(until(lambda: self.s.path is not None and self.s.path.name == "beta_wall.json"))
        self.page.wait_for_selector('.node[data-id="hills"]')
        self.open_files()
        recent = self.page.eval_on_selector_all("#file-menu .fm-list .cat", "els => els.map(e => e.textContent)")
        self.assertEqual(recent[0], "Recent")                                                   # opened files come back at the top
        self.page.keyboard.type("zzzz")
        self.page.wait_for_selector("#file-menu .cat:has-text('No graph or library node matches')")
        self.page.keyboard.press("Escape")
        self.page.wait_for_selector("#file-menu", state="hidden")

    def test_save_as_duplicate_rename_delete_and_insert_from_the_browser(self):
        self.page.on("dialog", lambda d: d.accept(self._answer))
        self._answer = "graphs/saved_copy.json"
        self.open_files()
        self.page.locator(".fm-cur button", has_text="Save As").click()
        self.assertTrue(until(lambda: self.s.path is not None and self.s.path.name == "saved_copy.json"))
        self.assertTrue(os.path.exists(os.path.join(self.dir, "graphs", "saved_copy.json")))
        # duplicate demo.json from its row
        self.open_files()
        row = self.page.locator('#file-menu .frow[data-path="demo.json"]')
        row.hover()
        row.locator('button[title="Duplicate"]').click()
        self.page.wait_for_selector('#file-menu .frow[data-path="demo_copy.json"]')
        # rename it, then delete it
        self._answer = "demo_renamed.json"
        row = self.page.locator('#file-menu .frow[data-path="demo_copy.json"]')
        row.hover()
        row.locator('button[title="Rename"]').click()
        self.page.wait_for_selector('#file-menu .frow[data-path="demo_renamed.json"]')
        self.assertFalse(os.path.exists(os.path.join(self.dir, "demo_copy.json")))
        row = self.page.locator('#file-menu .frow[data-path="demo_renamed.json"]')
        row.hover()
        row.locator('button[title="Delete"]').click()
        self.page.wait_for_function("!document.querySelector('#file-menu .frow[data-path=\"demo_renamed.json\"]')")
        self.assertFalse(os.path.exists(os.path.join(self.dir, "demo_renamed.json")))
        # the open graph has no rename / delete buttons
        cur = self.page.locator('#file-menu .frow[data-path="graphs/saved_copy.json"]')
        cur.hover()
        self.assertEqual(cur.locator('button[title="Delete"]').count(), 0)
        # insert another graph's nodes into this one
        before = len(self.s.graph.nodes)
        row = self.page.locator('#file-menu .frow[data-path="demo.json"]')
        row.hover()
        row.locator('button[title^="Insert"]').click()
        self.assertTrue(until(lambda: len(self.s.graph.nodes) > before))

    def test_ctrl_g_saves_the_selection_as_a_library_node_and_swaps_it_in(self):
        self.page.on("dialog", lambda d: d.accept("rough_ground"))
        self.node("ground").locator(".title").click()
        self.page.keyboard.down("Shift")
        self.node("hills").locator(".title").click()
        self.page.keyboard.up("Shift")
        total = len(self.s.graph.nodes)
        self.page.keyboard.press("Control+g")
        self.assertTrue(until(lambda: "rough_ground" in self.s.evaluator.registry))
        self.assertTrue(until(lambda: any(n.type == "rough_ground" for n in self.s.graph.nodes.values())))
        self.assertTrue(os.path.exists(os.path.join(self.dir, "nodes", "rough_ground.json")))
        self.assertEqual(len(self.s.graph.nodes), total - 1)                                   # two nodes became one
        self.page.wait_for_selector(".node.selected")
        # the new node shows up in the file browser (as a library node) and in the Add Node menu under Library
        self.open_files()
        self.page.wait_for_selector('#file-menu .frow[data-path="nodes/rough_ground.json"]')
        heads = self.page.eval_on_selector_all("#file-menu .fm-list .cat", "els => els.map(e => e.textContent)")
        self.assertIn("Library nodes ◆", heads)
        self.page.keyboard.press("Escape")
        self.page.wait_for_selector("#file-menu", state="hidden")
        self.page.mouse.click(*self.empty_spot())                                                # nothing selected: a node without inputs can be added too
        self.open_menu()
        self.page.keyboard.type("rough")
        self.page.wait_for_selector('#tab-list .it[data-name="rough_ground"]')
        self.assertIn("Library", self.page.locator('#tab-list .it[data-name="rough_ground"] small').inner_text())
        self.page.keyboard.press("Escape")
        self.page.keyboard.press("Control+z")                                                    # one undo brings the nodes back
        self.assertTrue(until(lambda: len(self.s.graph.nodes) == total))

    # ---- layout: resizable panes and navigation of the 3-D view
    def width(self, sel):
        return self.page.locator(sel).bounding_box()["width"]

    def test_layout_presets_swap_stack_and_hide_the_panel(self):
        self.box_scene()
        self.page.click('#topbar [data-layout="view"]')
        self.assertGreater(self.width("#preview"), 2 * self.width("#canvas"))
        self.page.click('#topbar [data-layout="graph"]')
        self.assertGreater(self.width("#canvas"), 2 * self.width("#preview"))
        self.page.click("#lay-swap")
        self.assertLess(self.page.locator("#canvas").bounding_box()["x"], self.page.locator("#preview").bounding_box()["x"] + 1e-6 + 10_000)
        self.assertLess(self.page.locator("#preview").bounding_box()["x"], self.page.locator("#canvas").bounding_box()["x"])
        self.page.click("#lay-dir")                                                            # stacked: the 3-D view is above the graph
        a, b = self.page.locator("#preview").bounding_box(), self.page.locator("#canvas").bounding_box()
        self.assertLess(a["y"], b["y"])
        self.assertAlmostEqual(a["x"], b["x"], delta=2)
        self.page.click("#lay-side")
        self.assertFalse(self.page.locator("#side").is_visible())
        self.page.click("#lay-side")
        self.assertTrue(self.page.locator("#side").is_visible())

    def test_dividers_resize_and_the_layout_is_remembered(self):
        self.box_scene()
        self.page.click('#topbar [data-layout="split"]')
        w0 = self.width("#preview")
        sp = self.page.locator("#work-split").bounding_box()
        self.page.mouse.move(sp["x"] + 2, sp["y"] + 200)
        self.page.mouse.down()
        self.page.mouse.move(sp["x"] - 150, sp["y"] + 200, steps=5)
        self.page.mouse.up()
        w1 = self.width("#preview")
        self.assertGreater(w1, w0 + 100)
        side0 = self.width("#side")
        sp = self.page.locator("#splitter").bounding_box()
        self.page.mouse.move(sp["x"] + 2, sp["y"] + 200)
        self.page.mouse.down()
        self.page.mouse.move(sp["x"] - 80, sp["y"] + 200, steps=5)
        self.page.mouse.up()
        self.assertGreater(self.width("#side"), side0 + 50)
        params0 = self.page.locator("#params").bounding_box()["height"]
        sp = self.page.locator("#side-split").bounding_box()
        self.page.mouse.move(sp["x"] + 40, sp["y"] + 2)
        self.page.mouse.down()
        self.page.mouse.move(sp["x"] + 40, sp["y"] - 60, steps=5)
        self.page.mouse.up()
        self.assertLess(self.page.locator("#params").bounding_box()["height"], params0 - 30)
        w2 = self.width("#preview")
        self.page.reload()
        self.page.wait_for_selector("#pv-canvas")
        self.assertAlmostEqual(self.width("#preview"), w2, delta=6)
        self.assertAlmostEqual(self.width("#side"), side0 + 80, delta=6)

    def test_ctrl_space_fills_the_window_with_the_pane_under_the_pointer(self):
        self.box_scene()
        box = self.page.locator("#pv-canvas").bounding_box()
        self.page.mouse.move(box["x"] + 40, box["y"] + 40)
        self.page.keyboard.press("Control+Space")
        self.assertFalse(self.page.locator("#canvas").is_visible())
        self.assertGreater(self.width("#preview"), 0.6 * self.page.viewport_size["width"])
        self.page.keyboard.press("Control+Space")
        self.assertTrue(self.page.locator("#canvas").is_visible())
        c = self.page.locator("#canvas").bounding_box()
        self.page.mouse.move(c["x"] + 40, c["y"] + 40)
        self.page.keyboard.press("Control+Space")
        self.assertFalse(self.page.locator("#preview").is_visible())
        self.page.click("#topbar [data-layout=split]")                                         # a preset leaves the maximised state
        self.assertTrue(self.page.locator("#preview").is_visible())

    def view_angles(self):
        return self.page.evaluate("[minihou.preview.view.yaw, minihou.preview.view.pitch]")

    def test_axis_views_ortho_gizmo_and_frame_selection(self):
        import math
        self.box_scene()
        self.page.click('#preview [data-view="front"]')
        self.page.wait_for_function("Math.abs(minihou.preview.view.yaw + Math.PI / 2) < 1e-3")
        self.assertAlmostEqual(self.view_angles()[1], 0, places=3)
        self.page.click('#preview [data-view="top"]')
        self.page.wait_for_function("minihou.preview.view.pitch > 1.57")
        box = self.page.locator("#pv-canvas").bounding_box()
        self.page.mouse.move(box["x"] + 60, box["y"] + 300)
        self.page.keyboard.press("Control+Numpad1")                                            # the back view
        self.page.wait_for_function("Math.abs(minihou.preview.view.yaw - Math.PI / 2) < 1e-3 && Math.abs(minihou.preview.view.pitch) < 1e-3")
        self.page.keyboard.press("Numpad3")
        self.page.wait_for_function("Math.abs(minihou.preview.view.yaw) < 1e-3")
        self.page.locator('.pv-gizmo [data-axis="+Z"]').click()                                # the gizmo's Z ball looks from above
        self.page.wait_for_function("minihou.preview.view.pitch > 1.57")
        # orthographic: parallel edges stay parallel, so the box's top face is a rectangle on screen
        self.page.keyboard.press("Numpad5")
        self.assertEqual(self.page.locator("#pv-ortho").inner_text(), "Ortho")
        self.assertTrue(self.page.evaluate("minihou.preview.view.ortho"))
        pts = [self.page.evaluate("([k, i]) => minihou.preview.screenOf(k, i)", ["point", i]) for i in self.top_points]
        xs, ys = sorted({round(p[0]) for p in pts}), sorted({round(p[1]) for p in pts})
        self.assertEqual((len(xs), len(ys)), (2, 2))
        self.page.keyboard.press("Numpad5")
        self.assertFalse(self.page.evaluate("minihou.preview.view.ortho"))
        # frame selection zooms to the selected points, Home to everything
        self.page.click('#preview [data-mode="point"]')
        self.page.mouse.click(*self.view_xy("point", self.top_points[0]))
        d_all = self.page.evaluate("minihou.preview.view.dist")
        self.page.keyboard.press("NumpadDecimal")
        self.assertLess(self.page.evaluate("minihou.preview.view.dist"), d_all)
        self.page.keyboard.press("Home")
        self.page.click('#preview [data-mode="view"]')
        self.page.keyboard.press("Home")

    def test_framing_fits_a_narrow_view(self):
        self.box_scene()
        self.page.click('#topbar [data-layout="graph"]')                                       # a narrow 3-D view
        self.page.click("#pv-frame")
        box = self.page.locator("#pv-canvas").bounding_box()
        for i in range(8):
            sx, sy, _ = self.page.evaluate("([k, i]) => minihou.preview.screenOf(k, i)", ["point", i])
            self.assertTrue(0 <= sx <= box["width"] and 0 <= sy <= box["height"], (i, sx, sy, box))


if __name__ == "__main__":
    unittest.main()
