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
