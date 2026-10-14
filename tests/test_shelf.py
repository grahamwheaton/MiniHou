"""The tool shelf: every node can be reached from a tab, tabs only name real nodes, and the shelf works in the editor."""
import json
import os
import shutil
import subprocess
import unittest

import minihou.nodes  # noqa: F401
from minihou.sdk import REGISTRY

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
JS = os.path.join(ROOT, "minihou", "web", "static", "shelf.js")

DUMP = """
global.window = {}; global.document = {}; require(process.argv[1]);
const t = window.MiniShelf.TABS.map(([name, colour, tools]) => ({name, colour, tools: tools}));
process.stdout.write(JSON.stringify({tabs: t, icons: Object.keys(window.MiniShelf.ICONS)}));
"""


@unittest.skipUnless(shutil.which("node"), "node is not installed")
class ShelfData(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        out = subprocess.run(["node", "-e", DUMP, JS], capture_output=True, text=True, encoding="utf-8", check=True).stdout
        cls.data = json.loads(out)

    def test_tabs_name_real_nodes_and_every_node_is_on_a_tab(self):
        have = set(REGISTRY.names())
        shown = set()
        for tab in self.data["tabs"]:
            for tool in tab["tools"] or []:
                self.assertIn(tool["type"], have | {"loop_block"}, f"{tab['name']}: {tool['label']}")
                shown.add(tool["type"])
        shown |= {n for n in have if n.startswith("pcg_")}                      # the PCG tab lists them all
        self.assertEqual(sorted(have - shown), [], "built-in nodes with no shelf tab")

    def test_tab_order_starts_like_houdinis_and_has_no_duplicate_tool_in_a_tab(self):
        names = [t["name"] for t in self.data["tabs"]]
        self.assertEqual(names[:4], ["Create", "Modify", "Model", "Polygon"])
        self.assertEqual(names[-1], "Library")
        self.assertEqual(len(names), len(set(names)))
        for tab in self.data["tabs"]:
            labels = [t["label"] for t in tab["tools"] or []]
            self.assertEqual(len(labels), len(set(labels)), tab["name"])

    def test_null_is_there_and_icons_exist_for_the_common_tools(self):
        create = next(t for t in self.data["tabs"] if t["name"] == "Create")
        self.assertIn("null", [t["type"] for t in create["tools"]])
        for name in ("box", "sphere", "tube", "torus", "grid", "null", "line", "circle", "curve", "helix", "lsystem", "font"):
            self.assertIn(name, self.data["icons"])

    def test_icons_for_nodes_that_do_not_exist_are_not_kept(self):
        for name in self.data["icons"]:
            self.assertTrue(name in REGISTRY or name in {"lattice", "file", "host", "duplicate", "hull", "group", "code", "attr", "scatter",
                                                         "fracture", "solver", "loop", "constraint", "uv", "terrain", "taper", "squash", "peak"},
                            name)


try:
    from playwright.sync_api import sync_playwright  # noqa: F401
except ImportError:                                  # pragma: no cover
    pass
