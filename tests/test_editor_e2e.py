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
        self.page.locator("#file-menu .item", has_text="New graph").click()
        self.assertTrue(until(lambda: self.s.path is not None and self.s.path.name == "fresh.json"))
        self.page.wait_for_selector('.node[data-id="grid1"]')
        self.assertTrue(os.path.exists(os.path.join(self.dir, "graphs", "fresh.json")))
        self.page.click("#file-btn")
        self.page.locator("#file-menu .item", has_text="demo.json").click()
        self.page.wait_for_selector('.node[data-id="hills"]')


if __name__ == "__main__":
    unittest.main()
