"""Unit tests for shared memory zero-copy transport."""
import unittest
import numpy as np

from minihou.link.shm import (
    SHM_THRESHOLD,
    ShmReader,
    ShmRing,
    ShmSlot,
    calculate_arrays_size,
    read_arrays_from_buffer,
    write_arrays_to_buffer,
)


class ShmBufferTests(unittest.TestCase):
    def test_calculate_size(self):
        arrays = {
            "a": np.zeros((100, 3), dtype=np.float32),  # 1200 bytes, pad 0
            "b": np.zeros(5, dtype=np.uint8),           # 5 bytes, pad 3 -> 8
        }
        size = calculate_arrays_size(arrays)
        self.assertEqual(size, 1208)

    def test_write_and_read_various_dtypes(self):
        arrays = {
            "pos": np.arange(15, dtype=np.float32).reshape(5, 3),
            "ids": np.arange(5, dtype=np.int32),
            "flag": np.array([True, False, True, False, True]),
            "weights": np.linspace(0, 1, 5, dtype=np.float64),
            "empty": np.zeros((0, 3), dtype=np.float32),
        }
        total_size = calculate_arrays_size(arrays)
        raw_buf = bytearray(total_size)
        descs, written = write_arrays_to_buffer(memoryview(raw_buf), arrays)
        self.assertEqual(written, total_size)

        read_back = read_arrays_from_buffer(memoryview(raw_buf), descs)
        self.assertEqual(set(read_back.keys()), set(arrays.keys()))
        for k, v in arrays.items():
            np.testing.assert_array_equal(read_back[k], v)
            self.assertEqual(read_back[k].dtype, v.dtype)
            self.assertEqual(read_back[k].shape, v.shape)

    def test_non_contiguous_array(self):
        a = np.arange(40, dtype=np.float32).reshape(10, 4)[:, ::2]
        self.assertFalse(a.flags.c_contiguous)
        arrays = {"sliced": a}
        sz = calculate_arrays_size(arrays)
        buf = bytearray(sz)
        descs, _ = write_arrays_to_buffer(memoryview(buf), arrays)
        got = read_arrays_from_buffer(memoryview(buf), descs)
        np.testing.assert_array_equal(got["sliced"], a)


class ShmRingTests(unittest.TestCase):
    def test_ring_double_buffering_and_ack(self):
        ring = ShmRing(prefix="test_shm_ring")
        self.addCleanup(ring.close)

        # Frame 1
        slot0_id, name0, buf0 = ring.get_slot_for_write(1024, seq=1)
        self.assertEqual(slot0_id, 1)  # 1 % 2 = 1
        self.assertTrue(ring.slots[1].busy)

        # Frame 2 while frame 1 is busy
        slot1_id, name1, buf1 = ring.get_slot_for_write(1024, seq=2)
        self.assertEqual(slot1_id, 0)  # 2 % 2 = 0
        self.assertTrue(ring.slots[0].busy)

        # Blender ACKs frame 1
        ring.on_ack(seq=1, slot_id=1)
        self.assertFalse(ring.slots[1].busy)
        self.assertTrue(ring.slots[0].busy)

        # Frame 3 reuses slot 1
        slot3_id, name3, buf3 = ring.get_slot_for_write(1024, seq=3)
        self.assertEqual(slot3_id, 1)

    def test_high_water_mark_resizing(self):
        slot = ShmSlot(0, "test_resize_slot", initial_size=1024)
        self.addCleanup(slot.close)
        self.assertEqual(slot.size, 1024)

        # Needs 2000 bytes -> 1.5x of 2000 = 3000 bytes, rounded up to at least 64KB
        name, buf = slot.prepare(70000, seq=1)
        self.assertGreaterEqual(slot.size, 70000 * 3 // 2)
        self.assertEqual(len(buf), slot.size)

    def test_ring_reader_zero_copy(self):
        ring = ShmRing(prefix="test_shm_reader")
        self.addCleanup(ring.close)
        reader = ShmReader()
        self.addCleanup(reader.close)

        arrays = {"point/P": np.arange(300, dtype=np.float32).reshape(100, 3)}
        nbytes = calculate_arrays_size(arrays)
        slot_id, name, buf = ring.get_slot_for_write(nbytes, seq=10)
        descs, _ = write_arrays_to_buffer(buf, arrays)

        # Reader attaches and reads zero-copy views
        rbuf = reader.get_buffer(name)
        got = read_arrays_from_buffer(rbuf, descs)
        np.testing.assert_array_equal(got["point/P"], arrays["point/P"])

        # Modify through original buffer to verify zero-copy view
        buf[0:4] = b"\0\0\0\0"
        self.assertEqual(float(got["point/P"][0, 0]), 0.0)

    def test_send_recv_socket_roundtrip_hybrid_threshold(self):
        import socket
        import threading
        from minihou.link.shm import send_shm_or_tcp_message, recv_shm_or_tcp_message

        ring = ShmRing(prefix="test_shm_sock")
        self.addCleanup(ring.close)
        reader = ShmReader()
        self.addCleanup(reader.close)

        a, b = socket.socketpair()
        self.addCleanup(a.close)
        self.addCleanup(b.close)

        # 1. Under 64KB -> TCP transport
        small_arr = {"pos": np.arange(30, dtype=np.float32).reshape(10, 3)}
        send_shm_or_tcp_message(a, {"type": "geo", "seq": 1}, small_arr, shm_ring=ring, seq=1)
        h1, got1 = recv_shm_or_tcp_message(b, shm_reader=reader)
        self.assertEqual(h1["transport"], "tcp")
        np.testing.assert_array_equal(got1["pos"], small_arr["pos"])

        # 2. Over 64KB (e.g. 25,000 points = 300,000 bytes) -> SHM transport
        large_arr = {"point/P": np.arange(75000, dtype=np.float32).reshape(25000, 3)}
        bytes_sent = send_shm_or_tcp_message(a, {"type": "geo", "seq": 2}, large_arr, shm_ring=ring, seq=2)
        # Socket transfer is tiny (just header, < 1024 bytes) even though data is 300KB!
        self.assertLess(bytes_sent, 1024)

        h2, got2 = recv_shm_or_tcp_message(b, shm_reader=reader)
        self.assertEqual(h2["transport"], "shm")
        self.assertEqual(h2["shm_slot"], 0)
        np.testing.assert_array_equal(got2["point/P"], large_arr["point/P"])

        # 3. ACK release
        ring.on_ack(seq=2, slot_id=h2["shm_slot"])
        self.assertFalse(ring.slots[0].busy)


class ShmIntegrationTests(unittest.TestCase):
    def test_live_link_server_and_client_shm_delivery_and_ack(self):
        import queue
        import time
        from minihou import Graph
        from minihou.link.client import LinkClient
        from minihou.link.server import LinkServer
        from minihou.link.session import Session

        # Create graph with 200x200 grid -> 40,000 points (~480KB arrays > 64KB threshold)
        graph = Graph.from_dict({
            "nodes": {
                "g": {"type": "grid", "params": {"columns": 200, "rows": 200}},
                "o": {"type": "output", "inputs": {"geo": "g"}},
            },
            "display": "o",
        })
        session = Session(graph)
        server = LinkServer(session, "127.0.0.1", 0)
        port = server.start()
        session.start()
        self.addCleanup(session.stop)
        self.addCleanup(server.stop)

        q = queue.Queue()
        client = LinkClient("127.0.0.1", port, on_message=lambda h, a: q.put((h, a)))
        client.start()
        self.addCleanup(client.stop)

        # Wait for geo message
        geo_msg, geo_arrays = None, None
        deadline = time.time() + 5.0
        while time.time() < deadline:
            try:
                h, a = q.get(timeout=1.0)
                if h.get("type") == "geo":
                    geo_msg, geo_arrays = h, a
                    break
            except queue.Empty:
                pass

        self.assertIsNotNone(geo_msg, "Timed out waiting for geo message")
        self.assertEqual(geo_msg.get("transport"), "shm")
        self.assertIn("shm_name", geo_msg)
        self.assertIn("point/P", geo_arrays)
        self.assertEqual(geo_arrays["point/P"].shape, (40000, 3))

        # Check server connection shm_ring slot is busy
        conn = server.clients[0]
        self.assertIsNotNone(conn.shm_ring)
        slot_id = geo_msg["shm_slot"]
        self.assertTrue(conn.shm_ring.slots[slot_id].busy)

        # Send ACK from client
        client.send({"type": "geo_ack", "seq": geo_msg["seq"], "slot": slot_id})

        # Wait for slot to be released
        deadline = time.time() + 2.0
        while time.time() < deadline and conn.shm_ring.slots[slot_id].busy:
            time.sleep(0.02)
        self.assertFalse(conn.shm_ring.slots[slot_id].busy)

    def test_instance_client_shm_delivery_and_ack(self):
        import time
        from minihou import Graph
        from minihou.link.instance_client import InstanceClient
        from minihou.link.server import LinkServer
        from minihou.link.session import Session

        # Input mesh that is large enough to trigger SHM (>64KB)
        # 10,000 points * 12 bytes = 120KB
        n_pts = 10000
        big_mesh = {
            "point/P": np.zeros((n_pts, 3), dtype=np.float32),
            "topo/vertex_point": np.arange(n_pts, dtype=np.int32),
            "topo/prim_offsets": np.array([0, n_pts], dtype=np.int32),
        }

        graph = {
            "nodes": {
                "in": {"type": "modifier_input"},
                "out": {"type": "output", "inputs": {"geo": "in"}},
            },
            "display": "out",
        }

        session = Session(Graph.from_dict({"nodes": {"g": {"type": "grid"}}, "display": "g"}))
        server = LinkServer(session, port=0)
        server.start()
        self.addCleanup(server.stop)

        client = InstanceClient(port=server.port)
        self.addCleanup(client.close)

        h, arrays = client.cook("inst-1", "", {}, big_mesh, graph=graph)
        self.assertEqual(h.get("transport"), "shm")
        self.assertEqual(arrays["point/P"].shape, (n_pts, 3))

        conn = server.clients[0]
        slot_id = h["shm_slot"]
        self.assertTrue(conn.shm_ring.slots[slot_id].busy)

        client.ack(h["seq"], slot_id)

        deadline = time.time() + 2.0
        while time.time() < deadline and conn.shm_ring.slots[slot_id].busy:
            time.sleep(0.02)
        self.assertFalse(conn.shm_ring.slots[slot_id].busy)


if __name__ == "__main__":
    unittest.main()



class ShmLifetimeTests(unittest.TestCase):
    def test_views_outlive_reader_close_without_unraisable_errors(self):
        import gc
        import sys
        from unittest.mock import patch
        from minihou.link.shm import unpack_shm_payload
        ring = ShmRing()
        self.addCleanup(ring.close)
        _, name, buf = ring.get_slot_for_write(1024, 1)
        desc, _ = write_arrays_to_buffer(buf, {"a": np.arange(20, dtype=np.float32)})
        errors = []
        with patch.object(sys, "unraisablehook", errors.append):
            reader = ShmReader()
            arrays = unpack_shm_payload({"shm_name": name, "arrays": desc}, reader)
            alias = np.asarray(arrays["a"])[2:8]
            reader.close()
            del arrays, reader
            gc.collect()
            np.testing.assert_array_equal(alias, np.arange(2, 8))
            del alias
            gc.collect()
        self.assertEqual(errors, [])

    def test_readerless_unpack_owns_its_mapping(self):
        import gc
        import sys
        from unittest.mock import patch
        from minihou.link.shm import unpack_shm_payload
        ring = ShmRing()
        self.addCleanup(ring.close)
        _, name, buf = ring.get_slot_for_write(1024, 1)
        desc, _ = write_arrays_to_buffer(buf, {"a": np.arange(20, dtype=np.float32)})
        errors = []
        with patch.object(sys, "unraisablehook", errors.append):
            arrays = unpack_shm_payload({"shm_name": name, "arrays": desc})
            gc.collect()
            np.testing.assert_array_equal(arrays["a"], np.arange(20))
            del arrays
            gc.collect()
        self.assertEqual(errors, [])

    def test_full_ring_falls_back_without_overwriting_pending_frames(self):
        import socket
        import threading
        from minihou.link.shm import send_shm_or_tcp_message, recv_shm_or_tcp_message
        ring = ShmRing(); self.addCleanup(ring.close)
        reader = ShmReader(); self.addCleanup(reader.close)
        sender, receiver = socket.socketpair()
        self.addCleanup(sender.close);self.addCleanup(receiver.close)
        received = []
        def read():
            for _ in range(3):received.append(recv_shm_or_tcp_message(receiver, reader))
        thread = threading.Thread(target=read);thread.start()
        for seq in range(1,4):
            send_shm_or_tcp_message(sender, {"type":"geo","seq":seq}, {"a":np.full(20000,seq,np.float32)},ring,seq)
        thread.join(5);self.assertFalse(thread.is_alive())
        self.assertEqual([h['transport'] for h,a in received],['shm','shm','tcp'])
        for seq,(h,arrays) in enumerate(received,1):np.testing.assert_array_equal(arrays['a'],seq)
