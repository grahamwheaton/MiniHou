"""The disk cache: results that survive a restart, and the cases that must never be stored or served."""
import os
import shutil
import tempfile
import unittest

import numpy as np

import minihou  # noqa: F401
from minihou import Evaluator, Geometry, Graph, proxy
from minihou.diskcache import DiskCache
from minihou.evaluator import Cache
from minihou.link.session import Session
from minihou.sdk import Float, Int, Registry, node


def make_registry(calls):
    reg = Registry()

    @node("big", params=[Int("n", 1000, proxy="count"), Float("x", 0.0)], registry=reg)
    def big(n, x):
        calls.append(("big", n, x))
        g = Geometry.from_arrays(np.random.default_rng(3).random((n, 3)).astype(np.float32) + np.float32(x),
                                 np.arange(n - n % 3, dtype=np.int32), np.arange(0, n - n % 3 + 1, 3, dtype=np.int32))
        g.point["w"] = np.arange(n, dtype=np.float32)
        g.vertex["c"] = np.arange(n - n % 3, dtype=np.int32)
        g.prim["tag"] = np.arange(g.nprims, dtype=np.int32)
        g.detail["note"] = "kept"
        g.detail["count"] = n
        return g

    @node("shift", inputs=["geo"], params=[Float("dx", 0.0)], registry=reg)
    def shift(geo, dx):
        calls.append(("shift", dx))
        out = geo.copy()
        out.point["P"] = geo.P + np.float32(dx)
        return out

    @node("tupled", params=[Int("n", 100)], registry=reg)
    def tupled(n):
        calls.append(("tupled", n))
        g = Geometry.from_arrays(np.zeros((n, 3), np.float32))
        g.detail["pair"] = (1, 2)                       # a tuple would come back from JSON as a list: must not be stored
        return g
    return reg


def graph(reg, n=1000, dx=1.0, extra=None):
    nodes = {"b": {"type": "big", "params": {"n": n}}, "s": {"type": "shift", "inputs": {"geo": "b"}, "params": {"dx": dx}}}
    nodes.update(extra or {})
    return Graph.from_dict({"nodes": nodes, "display": "s"}, reg)


class DiskCacheTests(unittest.TestCase):
    def setUp(self):
        self.dir = tempfile.mkdtemp(prefix="minihou_dc_")
        self.addCleanup(shutil.rmtree, self.dir, True)
        self.calls = []
        self.reg = make_registry(self.calls)

    def disk(self, **kw):
        kw.setdefault("min_ms", 0)
        kw.setdefault("min_bytes", 0)
        d = DiskCache(self.dir, **kw)
        self.addCleanup(d.close)
        return d

    def evaluator(self, disk):
        return Evaluator(self.reg, Cache(), disk=disk)

    def test_a_new_process_gets_the_result_from_disk_without_cooking_anything(self):
        d = self.disk()
        first = self.evaluator(d).cook(graph(self.reg))
        self.assertTrue(d.flush())
        self.assertEqual([c[0] for c in self.calls], ["big", "shift"])
        self.calls.clear()
        ev2 = self.evaluator(self.disk())                     # fresh memory cache, same folder: a restarted engine
        again = ev2.cook(graph(self.reg))
        self.assertEqual(self.calls, [])                       # the target came from disk: its inputs were never even looked at
        self.assertEqual(ev2.last_stats.disk, ["s"])
        np.testing.assert_array_equal(again.P, first.P)
        self.assertTrue(again.frozen)

    def test_what_comes_back_is_identical(self):
        d = self.disk()
        g = self.evaluator(d).cook(graph(self.reg, n=999))
        d.flush()
        back = self.evaluator(self.disk()).cook(graph(self.reg, n=999))
        self.assertEqual((back.npoints, back.nvertices, back.nprims), (g.npoints, g.nvertices, g.nprims))
        np.testing.assert_array_equal(back.topology.vertex_point, g.topology.vertex_point)
        np.testing.assert_array_equal(back.topology.prim_offsets, g.topology.prim_offsets)
        self.assertEqual(back.topology.has_open, g.topology.has_open)
        for dom in ("point", "vertex", "prim"):
            self.assertEqual(back.table(dom).names(), g.table(dom).names())
            for name in g.table(dom).names():
                np.testing.assert_array_equal(back.table(dom)[name], g.table(dom)[name])
                self.assertEqual(back.table(dom)[name].dtype, g.table(dom)[name].dtype)
        self.assertEqual(back.detail, g.detail)

    def test_open_curves_keep_their_closed_flags(self):
        reg = Registry()

        @node("curvy", registry=reg)
        def curvy():
            return Geometry.from_arrays(np.zeros((6, 3), np.float32), np.arange(6, dtype=np.int32), [0, 3, 6], closed=[True, False])
        g = Graph.from_dict({"nodes": {"c": {"type": "curvy"}}, "display": "c"}, reg)
        d = self.disk()
        Evaluator(reg, Cache(), disk=d).cook(g)
        d.flush()
        back = Evaluator(reg, Cache(), disk=self.disk()).cook(g)
        self.assertEqual(back.topology.closed_mask().tolist(), [True, False])

    def test_a_changed_parameter_or_proxy_level_is_a_different_entry(self):
        d = self.disk()
        ev = self.evaluator(d)
        ev.cook(graph(self.reg, n=1000))
        ev.proxy_level = 0.1
        small = ev.cook(graph(self.reg, n=1000))
        self.assertEqual(small.detail["count"], 100)
        d.flush()
        self.calls.clear()
        ev2 = self.evaluator(self.disk())
        self.assertEqual(ev2.cook(graph(self.reg, n=1000)).detail["count"], 1000)
        ev2.proxy_level = 0.1
        self.assertEqual(ev2.cook(graph(self.reg, n=1000)).detail["count"], 100)
        self.assertEqual(self.calls, [])                       # both levels were on disk
        ev2.proxy_level = None
        ev2.cook(graph(self.reg, n=1000, dx=2.0))              # another parameter: cooked (the big node comes from disk though)
        self.assertEqual([c[0] for c in self.calls], ["shift"])

    def test_only_results_that_are_quicker_to_read_than_to_cook_are_written(self):
        d = DiskCache(self.dir)                                # default rule: at least 100 ms, and reading takes under half the cook time
        self.addCleanup(d.close)
        self.evaluator(d).cook(graph(self.reg))                # these nodes take microseconds
        d.flush()
        self.assertEqual(d.stats()["writes"], 0)
        small = Geometry.from_arrays(np.zeros((1000, 3), np.float32)).freeze()
        big = Geometry.from_arrays(np.zeros((20_000_000, 3), np.float32)).freeze()       # 240 MB: about a second to read back
        self.assertTrue(d.wanted(small, 500.0))                # a slow node with a small result: the best catch
        self.assertFalse(d.wanted(small, 20.0))                # too quick to bother
        self.assertFalse(d.wanted(big, 500.0))                 # cooked faster than it could be read
        self.assertTrue(d.wanted(big, 5000.0))

    def test_results_with_unstable_details_are_not_written(self):
        d = self.disk()
        g = Graph.from_dict({"nodes": {"t": {"type": "tupled"}}, "display": "t"}, self.reg)
        self.evaluator(d).cook(g)
        d.flush()
        self.assertEqual((d.stats()["writes"], d.stats()["skipped"]), (0, 1))

    def test_nothing_that_depends_on_the_outside_world_is_written(self):
        from minihou.inputs import INPUTS
        INPUTS.put("DiskCacheTestObject", Geometry.from_arrays(np.random.default_rng(1).random((500, 3)).astype(np.float32)))
        self.addCleanup(INPUTS.retain, [])
        reg = make_registry(self.calls)
        import minihou.nodes  # noqa: F401
        from minihou.sdk import REGISTRY
        for name in ("blender_object", "transform"):
            reg.register(REGISTRY.get(name))
        g = Graph.from_dict({"nodes": {"o": {"type": "blender_object", "params": {"object": "DiskCacheTestObject"}},
                                       "t": {"type": "transform", "inputs": {"geo": "o"}, "params": {"translate": [1, 0, 0]}}},
                             "display": "t"}, reg)
        d = self.disk()
        Evaluator(reg, Cache(), disk=d).cook(g)
        d.flush()
        self.assertEqual(d.stats()["writes"], 0)                # neither the object nor what is built on it
        self.assertEqual(os.listdir(self.dir), [])

    def test_systems_inputs_are_not_written(self):
        # a node cooked with fed-in geometry has a one-off key
        reg = self.reg
        g = graph(reg)
        ev = self.evaluator(self.disk())
        fed = Geometry.from_arrays(np.zeros((10, 3), np.float32)).freeze()
        ev.cook(g, "s", feeds={"b": ("in:token", fed)})
        ev.disk.flush()
        self.assertEqual(ev.disk.stats()["writes"], 0)

    def test_the_folder_is_capped_and_old_results_go_first(self):
        d = self.disk(max_bytes=60_000)
        ev = self.evaluator(d)
        for i in range(6):
            ev.cook(graph(self.reg, n=1500, dx=float(i)))        # each entry holds about 40 KB
            d.flush()
        self.assertLessEqual(d.stats()["bytes"], 60_000)
        self.assertGreater(d.stats()["bytes"], 0)
        self.assertGreater(d.stats()["writes"], 3)

    def test_a_damaged_entry_is_a_miss_and_is_cooked_again(self):
        d = self.disk()
        g = graph(self.reg)
        self.evaluator(d).cook(g)
        d.flush()
        victim = None
        for root, _dirs, files in os.walk(self.dir):
            for f in files:
                if f == "point_0.npy":
                    victim = os.path.join(root, f)
        with open(victim, "wb") as fh:
            fh.write(b"not an array")
        self.calls.clear()
        ev = self.evaluator(self.disk())
        out = ev.cook(g)
        self.assertEqual(out.npoints, 1000)
        self.assertIn("shift", [c[0] for c in self.calls])      # recooked, no error

    def test_clear_empties_the_folder(self):
        d = self.disk()
        self.evaluator(d).cook(graph(self.reg))
        d.flush()
        self.assertGreater(d.stats()["bytes"], 0)
        d.clear()
        self.assertEqual((d.stats()["bytes"], os.listdir(self.dir)), (0, []))


class SessionDiskCacheTests(unittest.TestCase):
    def test_a_restarted_session_reopens_from_disk_and_says_so(self):
        d = tempfile.mkdtemp(prefix="minihou_dc_")
        self.addCleanup(shutil.rmtree, d, True)
        calls = []
        reg = make_registry(calls)
        g = graph(reg, n=3000)
        s1 = Session(Graph.from_dict(g.to_dict(), reg), registry=reg, disk_cache=DiskCache(d, min_ms=0, min_bytes=0))
        s1.start()
        self.assertTrue(s1.wait_idle(10))
        self.assertTrue(s1.disk_cache.flush())
        self.assertGreater(s1.status()["disk_cache"]["writes"], 0)
        s1.stop()
        calls.clear()
        s2 = Session(Graph.from_dict(g.to_dict(), reg), registry=reg, disk_cache=DiskCache(d, min_ms=0, min_bytes=0))
        s2.start()
        self.addCleanup(s2.stop)
        self.assertTrue(s2.wait_idle(10))
        self.assertEqual(calls, [])
        st = s2.status()
        self.assertEqual(st["points"], 3000)
        self.assertGreater(st["disk_cache"]["hits"], 0)
        self.assertEqual(st["cook"]["disk"], ["s"])


class CommandLineTests(unittest.TestCase):
    def test_the_flag_and_the_default_folder(self):
        import argparse
        from minihou.__main__ import _disk_cache
        self.assertIsNone(_disk_cache(argparse.Namespace(disk_cache=None, root="x")))
        root = tempfile.mkdtemp(prefix="minihou_dc_")
        self.addCleanup(shutil.rmtree, root, True)
        d = _disk_cache(argparse.Namespace(disk_cache="auto", root=root, disk_cache_gb=0.5))
        self.addCleanup(d.close)
        self.assertEqual(str(d.folder), os.path.join(root, ".minihou_cache"))
        self.assertEqual(d.max_bytes, int(0.5 * 1024 ** 3))


if __name__ == "__main__":
    unittest.main()
