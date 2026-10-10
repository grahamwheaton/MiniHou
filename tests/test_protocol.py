import socket
import threading
import unittest

import numpy as np

from minihou import Geometry
from minihou.link.diff import DiffState, arrays_equal
from minihou.link.protocol import (GeoUpdate, MAGIC, Mirror, ProtocolError, TOPO_KEYS, decode_payload,
                                   encode_message, merge_updates, recv_message, send_message)
from tests.helpers import quad_geo


def roundtrip(header, arrays=None):
    a, b = socket.socketpair()
    try:
        t = threading.Thread(target=send_message, args=(a, header, arrays))
        t.start()
        out = recv_message(b)
        t.join()
        return out
    finally:
        a.close()
        b.close()


class FramingTests(unittest.TestCase):
    def test_roundtrip_various_dtypes_and_shapes(self):
        arrays = {
            "f": np.arange(12, dtype=np.float32).reshape(4, 3),
            "i": np.arange(5, dtype=np.int32),
            "b": np.array([True, False, True]),
            "d": np.linspace(0, 1, 7),
            "empty": np.zeros((0, 3), np.float32),
            "u": np.arange(3, dtype=np.uint8),
        }
        h, got = roundtrip({"type": "x", "n": 1}, arrays)
        self.assertEqual(h["n"], 1)
        self.assertEqual(set(got), set(arrays))
        for k, v in arrays.items():
            np.testing.assert_array_equal(got[k], v)
            self.assertEqual(got[k].dtype, v.dtype)
            self.assertEqual(got[k].shape, v.shape)

    def test_arrays_are_8_byte_aligned_in_the_frame(self):
        bufs = encode_message({"type": "x"}, {"a": np.zeros(3, np.uint8), "b": np.zeros(2, np.float32)})
        body = b"".join(bytes(b) for b in bufs)
        hlen = int.from_bytes(body[4:8], "little")
        self.assertEqual(hlen % 8, 0)
        h, arrays = decode_payload(bytearray(body[16:]), hlen)
        self.assertEqual([d["offset"] % 8 for d in h["arrays"]], [0, 0])

    def test_non_contiguous_arrays_are_handled(self):
        a = np.arange(24, dtype=np.float32).reshape(6, 4)[:, ::2]
        _, got = roundtrip({"type": "x"}, {"a": a})
        np.testing.assert_array_equal(got["a"], a)

    def test_large_array_one_million_points(self):
        P = np.random.default_rng(0).random((1_000_000, 3), dtype=np.float32)
        _, got = roundtrip({"type": "x"}, {"point/P": P})
        np.testing.assert_array_equal(got["point/P"], P)

    def test_bad_magic_and_truncation(self):
        a, b = socket.socketpair()
        a.sendall(b"NOPE" + b"\0" * 12)
        with self.assertRaises(ProtocolError):
            recv_message(b)
        a.close()
        b.close()
        a, b = socket.socketpair()
        a.sendall(b"MH")
        a.close()
        with self.assertRaises(ConnectionError):
            recv_message(b)
        b.close()

    def test_unsupported_dtype_rejected(self):
        with self.assertRaises(ProtocolError):
            encode_message({"type": "x"}, {"a": np.zeros(2, np.complex64)})

    def test_descriptor_outside_blob_rejected(self):
        bufs = encode_message({"type": "x"}, {"a": np.zeros(4, np.float32)})
        body = bytearray(b"".join(bytes(b) for b in bufs))
        hlen = int.from_bytes(body[4:8], "little")
        payload = body[16:-8]                      # chop the blob
        with self.assertRaises(ProtocolError):
            decode_payload(payload, hlen)

    def test_nan_in_header_rejected(self):
        with self.assertRaises(ValueError):
            encode_message({"type": "x", "v": float("nan")})


class DiffTests(unittest.TestCase):
    def geo(self):
        g = quad_geo()
        g.point["Cd"] = np.ones((4, 3), np.float32)
        return g.freeze()

    def test_first_build_is_full(self):
        d = DiffState()
        h, arrays = d.build(self.geo(), 1)
        self.assertTrue(h["full"] and h["topology_changed"])
        self.assertEqual(set(arrays), {"point/P", "point/Cd", *TOPO_KEYS})

    def test_identical_geometry_sends_nothing(self):
        d = DiffState()
        g = self.geo()
        d.build(g, 1)
        h, arrays = d.build(g, 2)
        self.assertEqual(arrays, {})
        self.assertFalse(h["full"] or h["topology_changed"])

    def test_equal_content_in_new_arrays_is_detected_as_unchanged(self):
        d = DiffState()
        d.build(self.geo(), 1)
        h, arrays = d.build(self.geo(), 2)                       # rebuilt from scratch, same bytes
        self.assertEqual(arrays, {})

    def test_only_changed_attribute_is_sent(self):
        d = DiffState()
        g = self.geo()
        d.build(g, 1)
        g2 = g.copy()
        g2.point["P"] = g.P + 1
        g2.freeze()
        h, arrays = d.build(g2, 2)
        self.assertEqual(set(arrays), {"point/P"})
        self.assertFalse(h["topology_changed"])
        self.assertEqual(set(h["manifest"]), {"point/P", "point/Cd"})

    def test_topology_change_sends_both_topology_arrays(self):
        d = DiffState()
        g = self.geo()
        d.build(g, 1)
        g2 = g.copy()
        from minihou import Topology
        g2.set_topology(Topology.from_polys([[0, 1, 2], [0, 2, 3]]))
        g2.freeze()
        h, arrays = d.build(g2, 2)
        self.assertTrue(h["topology_changed"])
        self.assertTrue(set(TOPO_KEYS) <= set(arrays))
        self.assertNotIn("point/P", arrays)                      # points themselves didn't change

    def test_removed_attribute_leaves_manifest(self):
        d = DiffState()
        g = self.geo()
        d.build(g, 1)
        g2 = g.copy()
        del g2.point["Cd"]
        h, arrays = d.build(g2.freeze(), 2)
        self.assertNotIn("point/Cd", h["manifest"])
        self.assertEqual(arrays, {})

    def test_reset_forces_full(self):
        d = DiffState()
        g = self.geo()
        d.build(g, 1)
        d.reset()
        h, arrays = d.build(g, 2)
        self.assertTrue(h["full"])
        self.assertEqual(len(arrays), 2 + 2)

    def test_arrays_equal_semantics(self):
        a = np.arange(100000, dtype=np.float32)
        self.assertTrue(arrays_equal(a, a.copy()))
        b = a.copy()
        b[-1] += 1
        self.assertFalse(arrays_equal(a, b))                     # difference only at the tail
        b = a.copy()
        b[0] += 1
        self.assertFalse(arrays_equal(a, b))                     # difference in the head (early out)
        self.assertFalse(arrays_equal(a, a.astype(np.float64)))
        self.assertFalse(arrays_equal(a, a[:-1]))
        n = np.array([np.nan], np.float32)
        self.assertTrue(arrays_equal(n, n.copy()))               # NaN == NaN bitwise, no endless resends


class MirrorAndMergeTests(unittest.TestCase):
    def updates(self):
        d = DiffState()
        g1 = quad_geo().freeze()
        g2 = g1.copy()
        g2.point["P"] = g1.P + 1
        g2.point["Cd"] = np.ones((4, 3), np.float32)
        g2.freeze()
        g3 = g2.copy()
        g3.point["P"] = g2.P * 2
        del g3.point["Cd"]
        g3.freeze()
        out = []
        for i, g in enumerate((g1, g2, g3), 1):
            h, arrays = d.build(g, i)
            # go through the wire so arrays are real decoded views
            h2, a2 = roundtrip(h, arrays)
            out.append((g, GeoUpdate.from_message(h2, a2)))
        return out

    def test_mirror_tracks_geometry_exactly(self):
        m = Mirror()
        for g, u in self.updates():
            m.apply(u)
            np.testing.assert_array_equal(m.P, g.P)
            np.testing.assert_array_equal(m.vertex_point, g.topology.vertex_point)
            self.assertEqual(sorted(n for _, n, _ in m.attributes()), sorted(g.point.names()))

    def test_merged_update_equals_sequential_updates(self):
        ups = [u for _, u in self.updates()]
        seq = Mirror()
        for u in ups:
            seq.apply(u)
        merged = ups[0]
        for u in ups[1:]:
            merged = merge_updates(merged, u)
        m2 = Mirror()
        m2.apply(merged)
        self.assertEqual(set(seq.arrays), set(m2.arrays))
        for k in seq.arrays:
            np.testing.assert_array_equal(seq.arrays[k], m2.arrays[k])
        self.assertTrue(merged.full)
        self.assertEqual(merged.seq, 3)

    def test_merging_keeps_topology_flag_from_either_side(self):
        ups = [u for _, u in self.updates()]
        a, b = ups[1], ups[2]                                    # neither changes topology
        self.assertFalse(merge_updates(a, b).topology_changed)
        self.assertTrue(merge_updates(ups[0], b).topology_changed)

    def test_mirror_detects_a_missing_array(self):
        _, u = self.updates()[1]
        u.arrays.pop("point/P")
        with self.assertRaises(ProtocolError):
            Mirror().apply(u)


if __name__ == "__main__":
    unittest.main()
