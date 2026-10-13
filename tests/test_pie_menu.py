"""The pie menu's data (pie.js) must only name nodes and parameters this engine has."""
import json
import os
import shutil
import subprocess
import unittest

from minihou.sdk import REGISTRY

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
PIE = os.path.join(ROOT, "minihou", "web", "static", "pie.js")

DUMP = """
global.window = {}; global.document = { addEventListener() {}, createElementNS() { return {}; }, body: {} };
global.innerWidth = 1000; global.innerHeight = 800; global.performance = { now: () => 0 };
require(process.argv[1]);
process.stdout.write(JSON.stringify(window.MiniPie.PAGES));
"""


@unittest.skipUnless(shutil.which("node"), "node is not installed")
class PieMenuData(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        import minihou.nodes  # noqa: F401 - registers the built-in nodes
        out = subprocess.run(["node", "-e", DUMP, PIE], capture_output=True, text=True, encoding="utf-8", check=True).stdout
        cls.pages = json.loads(out)

    def items(self):
        for name, page in self.pages.items():
            for slot, it in enumerate(page["items"]):
                if it:
                    yield name, slot, it

    def test_main_page_has_houdinis_eight_categories_in_order(self):
        self.assertEqual([i["label"] for i in self.pages["main"]["items"]],
                         ["Adjust", "Deform", "Polygons", "Group", "Create", "Build", "Edges", "Modify"])

    def test_every_page_has_at_most_eight_slots_and_every_link_resolves(self):
        for name, page in self.pages.items():
            self.assertLessEqual(len(page["items"]), 8, name)
        for name, slot, it in self.items():
            if "page" in it:
                self.assertIn(it["page"], self.pages, f"{name}[{slot}] {it['label']}")

    def test_the_screenshots_tools_are_all_there(self):
        wanted = {"adjust": ["Transform", "Soft Transform", "Soft Peak", "Smooth", "Sculpt", "Point Weld"],
                  "deform": ["Lattice", "Mountain", "Bend", "Linear Taper", "Squash and Stretch", "Twist"],
                  "polygons": ["PolyBevel", "PolyFill", "PolyExtrude", "PolySplit", "PolyBridge", "Clip", "Boolean"],
                  "group": ["Group Promote", "Group by Range", "Group Paint", "Group Create", "Group Expression"],
                  "create": ["Null", "Circle", "Tube", "Sphere", "Curve", "Grid", "Box"],
                  "build": ["Mirror", "Duplicate", "Combine", "Extract"],
                  "edges": ["Dissolve", "Edge Collapse", "Edge Loop", "Edge Cusp", "Edge Flip"]}
        for page, labels in wanted.items():
            have = [i["label"] for i in self.pages[page]["items"] if i]
            for label in labels:
                self.assertIn(label, have, page)

    def test_tools_name_real_nodes_and_parameters(self):
        for name, slot, it in self.items():
            if "page" in it:
                continue
            where = f"{name}[{slot}] {it['label']}"
            if it.get("disabled"):
                continue
            self.assertTrue(it.get("type"), where)
            nt = REGISTRY.get(it["type"])
            params = {p.name for p in nt.params}
            for key in it.get("params", {}):
                self.assertIn(key, params, where)
            if it.get("center"):
                self.assertIn(it["center"], params, where)
            for mode, spec in (it.get("sel") or {}).items():
                self.assertIn(mode, ("face", "point", "edge"), where)
                spec = {spec: "$"} if isinstance(spec, str) else {k: v for k, v in spec.items() if k != "from"}
                self.assertTrue(spec, where)
                for key in spec:
                    self.assertIn(key, params, f"{where} ({mode})")
            if it.get("source"):
                self.assertTrue(all(port.optional for port in nt.inputs), where)
            else:
                self.assertTrue(nt.inputs or it["type"] == "null", where)

    def test_tools_on_a_sphere_cook_or_explain_what_they_need(self):
        from minihou import Evaluator, Graph, NodeError
        for name, slot, it in self.items():
            if "page" in it or it.get("disabled"):
                continue
            nt = REGISTRY.get(it["type"])
            if len(nt.inputs) > 1 or not nt.inputs and not it.get("source"):
                continue
            nodes = {"t": {"type": it["type"], "params": it.get("params", {})}}
            if nt.inputs:
                nodes["s"] = {"type": "sphere", "params": {}}
                nodes["t"]["inputs"] = {nt.inputs[0].name: ["s"] if nt.inputs[0].multi else "s"}
            try:
                Evaluator().cook(Graph.from_dict({"nodes": nodes, "display": "t"}))
            except NodeError as e:                       # a tool may need more than a sphere (curves, pieces) and say so; it must not crash
                if not isinstance(e.__cause__, ValueError):
                    self.fail(f"{name}[{slot}] {it['label']} ({it['type']}): {e}")


if __name__ == "__main__":
    unittest.main()
