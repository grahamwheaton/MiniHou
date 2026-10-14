"""The Add Node menu: every node has a group, searches find nodes by the names people know them by, and the menu works."""
import json
import os
import shutil
import subprocess
import unittest

import minihou.nodes  # noqa: F401 - registers the built-in nodes
from minihou import menu
from minihou.sdk import REGISTRY

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
JS = os.path.join(ROOT, "minihou", "web", "static", "nodemenu.js")


class TaxonomyTests(unittest.TestCase):
    def test_every_builtin_node_is_listed_exactly_once_and_nothing_stale(self):
        listed = [n for _g, _d, names in menu.GROUPS for n in names]
        self.assertEqual(len(listed), len(set(listed)))
        have = set(REGISTRY.names())
        self.assertEqual(sorted(have - set(listed)), [], "nodes missing from minihou/menu.py")
        self.assertEqual(sorted(set(listed) - have - {"loop_block"}), [], "menu.py names nodes that do not exist")
        self.assertEqual(sorted(set(menu.ALIASES) - have - {"loop_block"}), [])

    def test_describe_carries_the_menu_fields(self):
        d = REGISTRY.get("polyextrude").describe()
        self.assertEqual(d["menu"], "Polygon Modelling")
        self.assertEqual(d["label"], "PolyExtrude")
        self.assertIn("extrude", d["keywords"])
        self.assertLess(d["menu_order"], REGISTRY.get("clean").describe()["menu_order"])             # common tools first
        self.assertEqual(REGISTRY.get("voronoi_fracture").describe()["menu"], "Fracture & RBD")
        self.assertEqual(REGISTRY.get("edge_collapse").describe()["label"], "Edge Collapse")

    def test_groups_are_in_order_and_unknown_nodes_fall_back(self):
        names = [g["name"] for g in menu.groups()]
        self.assertEqual(names[0], "Create")
        self.assertEqual(names[-1], menu.LIBRARY)
        self.assertEqual(menu.group_of("my_node", "deform"), "Deform")
        self.assertEqual(menu.group_of("my_node", "custom"), menu.LIBRARY)
        self.assertEqual(menu.group_of("my_system", "system"), menu.LIBRARY)
        self.assertEqual(menu.group_of("my_node", "something odd"), menu.LIBRARY)
        self.assertEqual(menu.label("my_node"), "my_node")                       # a name someone chose is shown as it is

    def test_pseudo_entries(self):
        p = menu.pseudo()[0]
        self.assertEqual((p["name"], p["menu"]), ("loop_block", "Flow & Time"))


@unittest.skipUnless(shutil.which("node"), "node is not installed")
class SearchTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.types = [nt.describe() for nt in REGISTRY] + menu.pseudo()

    def run_js(self, code):
        script = ("global.window = {}; require(process.argv[1]); const types = JSON.parse(require('fs').readFileSync(0, 'utf8'));"
                  "const M = window.NodeMenu; process.stdout.write(JSON.stringify((function(){" + code + "})()));")
        out = subprocess.run(["node", "-e", script, JS], input=json.dumps(self.types), capture_output=True, text=True,
                             encoding="utf-8", check=True)
        return json.loads(out.stdout)

    def test_search_finds_nodes_by_the_names_people_know(self):
        first = lambda q: self.run_js(f"return M.search(types, {json.dumps(q)}).slice(0, 3).map(t => t.name);")
        self.assertEqual(first("extrude")[0], "polyextrude")
        self.assertEqual(first("bevel")[0], "polybevel")
        self.assertIn("native_extrude", first("native extrude"))
        self.assertIn("native_solidify", first("blender solidify"))
        self.assertEqual(first("mountain")[0], "noise")
        self.assertEqual(first("duplicate")[0], "copy_transform")
        self.assertIn("rbd_material_fracture", first("shatter"))
        self.assertIn("voronoi_fracture", first("shatter"))
        self.assertEqual(first("weld")[0], "fuse")
        self.assertEqual(first("polyextrude")[0], "polyextrude")                  # an exact name wins
        self.assertEqual(first("loop_block")[0], "loop_block")
        self.assertIn("edge_collapse", self.run_js("return M.search(types, 'edge coll').map(t => t.name);"))

    def test_every_word_must_match_and_nothing_matches_nonsense(self):
        self.assertEqual(self.run_js("return M.search(types, 'qqzzxx').length;"), 0)
        both = self.run_js("return M.search(types, 'voronoi points').map(t => t.name);")
        self.assertIn("voronoi_fracture_points", both)
        self.assertNotIn("box", both)
        self.assertEqual(self.run_js("return M.search(types, '   ').length;"), len(self.types))

    def test_rows_without_a_query_are_favourites_recent_then_groups_folded_by_the_open_set(self):
        rows = self.run_js("""return M.rows(types, '', {favs: new Set(['box']), recent: ['sphere', 'box', 'transform'],
                             open: new Set(['Create'])}).map(r => r.kind === 'head' ? '#' + r.name + (r.open ? '+' : '') : r.type.name);""")
        self.assertEqual(rows[0:3], ["#Favourites+", "box", "#Recent+"])
        self.assertEqual(rows[3:5], ["sphere", "transform"])                          # a favourite is not repeated under Recent
        self.assertIn("#Create+", rows)
        at = rows.index("#Create+")
        self.assertEqual(rows[at + 1], "box")                                         # the open group lists its nodes in menu order
        self.assertIn("#Polygon Modelling", rows)                                     # closed: just its header
        after = rows[rows.index("#Polygon Modelling") + 1]
        self.assertTrue(after.startswith("#"))
        heads = [r for r in rows if r.startswith("#")]
        self.assertEqual(heads[-1], "#Utility & I/O")                                 # Library is empty, so it is left out

    def test_highlight_marks_the_first_match(self):
        self.assertEqual(self.run_js("return M.highlight('PolyExtrude', 'ext');"), [["Poly", False], ["Ext", True], ["rude", False]])
        self.assertEqual(self.run_js("return M.highlight('Box', 'zzz');"), [["Box", False]])
