"""bpy-free tests for embedded networks (add-on 0.3.0): canonical text, hashing, migration, paths, link fallback."""
import hashlib
import importlib.util
import json
import os
import sys
import tempfile
import types
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
ADDON = ROOT / "blender_addon" / "minihou_link"


def _load(name):
    """Load one add-on module without the bpy-dependent package entry point."""
    pkg = sys.modules.get("_mh_embed_pkg")
    if pkg is None:
        pkg = types.ModuleType("_mh_embed_pkg")
        pkg.__path__ = [str(ADDON)]
        sys.modules["_mh_embed_pkg"] = pkg
    spec = importlib.util.spec_from_file_location(f"_mh_embed_pkg.{name}", ADDON / f"{name}.py")
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


embed = _load("embed")

GRAPH = {"version": 1, "nodes": {"z_in": {"type": "modifier_input"},
                                  "img": {"type": "image_file", "params": {"file": "C:/proj/tex/a.png"}},
                                  "a_out": {"type": "output", "inputs": {"geo": "z_in"}}}, "display": "a_out"}


class CanonicalTextTests(unittest.TestCase):
    def test_canonical_is_deterministic_and_keeps_node_order(self):
        text = embed.canonical(GRAPH)
        self.assertEqual(text, embed.canonical(json.loads(text)))
        self.assertLess(text.index('"z_in"'), text.index('"a_out"'))       # first output stays first
        self.assertTrue(text.endswith("\n"))
        self.assertEqual(embed.digest(text), hashlib.sha256(text.encode("utf-8")).hexdigest())

    def test_non_ascii_and_nan(self):
        g = {"version": 1, "nodes": {"n": {"type": "box", "params": {"label": "Würfel"}}}}
        self.assertIn("Würfel", embed.canonical(g))
        with self.assertRaises(ValueError):
            embed.canonical({"nodes": {}, "x": float("nan")})               # never silently written


class MigrationTests(unittest.TestCase):
    def test_unversioned_graph_is_stamped(self):
        old = {"nodes": {"b": {"type": "box"}}, "display": "b"}
        new, migrated = embed.migrate(old)
        self.assertTrue(migrated)
        self.assertEqual(new["version"], embed.GRAPH_SCHEMA)
        self.assertNotIn("version", old)                                     # input untouched
        self.assertEqual(embed.migrate(new), (new, False))

    def test_newer_and_invalid_graphs_are_refused_with_clear_messages(self):
        with self.assertRaisesRegex(embed.EmbedError, "newer MiniHou.*left untouched"):
            embed.migrate({"version": 99, "nodes": {}})
        with self.assertRaisesRegex(embed.EmbedError, "no 'nodes'"):
            embed.migrate({"version": 1})
        with self.assertRaises(embed.EmbedError):
            embed.migrate({"version": "1", "nodes": {}})


class PathTests(unittest.TestCase):
    def test_path_params_cover_file_and_star_file_strings_only(self):
        g = {"nodes": {"a": {"type": "image_file", "params": {"file": "x.png"}},
                       "b": {"type": "lsystem", "params": {"rule_file": "r.txt", "read_rules_from_file": True,
                                                           "image_file": ""}},
                       "c": {"type": "usd_import", "params": {"file": "s.usd", "prim_path": "/a"}}}}
        self.assertEqual(sorted((n, k) for n, k, _ in embed.path_params(g)),
                         [("a", "file"), ("b", "rule_file"), ("c", "file")])

    def test_relativize_resolve_roundtrip_and_fallback(self):
        rel = {"C:/proj/tex/a.png": "//tex/a.png"}
        stored, originals = embed.relativize(GRAPH, lambda p: rel.get(p))
        self.assertEqual(stored["nodes"]["img"]["params"]["file"], "//tex/a.png")
        self.assertEqual(originals, {"img/file": "C:/proj/tex/a.png"})
        self.assertEqual(GRAPH["nodes"]["img"]["params"]["file"], "C:/proj/tex/a.png")   # not mutated
        back, missing = embed.resolve(stored, lambda p: "D:/moved/tex/a.png" if p.startswith("//") else p,
                                      lambda p: True)
        self.assertEqual((back["nodes"]["img"]["params"]["file"], missing), ("D:/moved/tex/a.png", []))
        _, missing = embed.resolve(stored, lambda p: "D:/gone.png", lambda p: False)
        self.assertEqual(missing, ["img.file: D:/gone.png"])
        fallback, missing = embed.resolve(stored, lambda p: "D:/gone.png", lambda p: p == "C:/proj/tex/a.png",
                                          originals)
        self.assertEqual((fallback["nodes"]["img"]["params"]["file"], missing), ("C:/proj/tex/a.png", []))

    def test_outside_lists_absolute_paths_beyond_the_blend_folder(self):
        with tempfile.TemporaryDirectory() as tmp:
            inside = os.path.join(tmp, "tex", "a.png")
            g = {"nodes": {"a": {"type": "image_file", "params": {"file": inside}},
                           "b": {"type": "image_file", "params": {"file": "//rel.png"}},
                           "c": {"type": "usd_import", "params": {"file": os.path.join(os.path.dirname(tmp), "x.usd")}}}}
            self.assertEqual([m.split(":")[0] for m in embed.outside(g, tmp)], ["c.file"])
            self.assertEqual(len(embed.outside(g, "")), 2)                    # unsaved blend: all absolute

    def test_text_names_are_unique_and_safe(self):
        self.assertEqual(embed.text_name("Cube MiniHou"), "MiniHou: Cube MiniHou.minihou")
        self.assertEqual(embed.text_name("a/b", {"MiniHou: a_b.minihou"}), "MiniHou: a_b 2.minihou")
        self.assertLessEqual(len(embed.text_name("x" * 200)), 63)            # Blender's ID name limit

    def test_human_size(self):
        self.assertEqual(embed.human_size(10), "10 B")
        self.assertEqual(embed.human_size(3 * 1024 * 1024), "3.00 MB")


class ClientFallbackTests(unittest.TestCase):
    def setUp(self):
        _load("protocol")
        _load("shm")
        self.client_mod = _load("instance_client")

    def client(self, capabilities):
        client = self.client_mod.InstanceClient()
        client.connect = lambda: client.welcome
        client.welcome = {"capabilities": capabilities}
        client.sent = []
        client._request = lambda header, arrays=None: (client.sent.append(header) or ({"status": "accepted"}, {}))
        return client

    def test_older_app_without_graph_embed_v1_is_not_sent_graph_messages(self):
        old = self.client(["instances_v1", "live_modifier_v1"])
        self.assertEqual(old.pull_graph("n")["status"], "unsupported")
        with self.assertRaisesRegex(RuntimeError, "predates"):
            old.push_graph("n", name="N", label="L", text="{}", sha256="x")
        self.assertEqual(old.sent, [])

    def test_push_header_carries_size_hash_and_schema(self):
        new = self.client(["instances_v1", "graph_embed_v1"])
        text = '{"nodes":{"ü":{}}}'
        new.push_graph("n", name="N", label="Cube / MiniHou", text=text, sha256="h")
        header = new.sent[0]
        self.assertEqual((header["type"], header["instance_id"], header["schema"], header["size"]),
                         ("graph_push", "n", 1, len(text.encode("utf-8"))))

    def test_embed_push_falls_back_for_older_apps(self):
        class Old:
            def connect(self):
                return {"capabilities": ["instances_v1"]}
        self.assertEqual(embed.push(Old(), object(), "label"), "")


if __name__ == "__main__":
    unittest.main()
