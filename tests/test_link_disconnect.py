"""Disconnect regressions using the shipped add-on modules and owned stub processes.

python -m unittest tests.test_link_disconnect -v
blender --background --factory-startup --python-exit-code 1 --python tests/test_link_disconnect.py
No test starts the desktop app, uses Blender MCP, or touches another Blender.
"""
from __future__ import annotations

import importlib
import os
from pathlib import Path
import queue
import socket
import struct
import subprocess
import sys
import threading
import time
import types
import unittest
from unittest.mock import patch

import numpy as np

ROOT = Path(__file__).resolve().parents[1]
PKG = "_disconnect_addon"
package = types.ModuleType(PKG)
package.__path__ = [str(ROOT / "blender_addon/minihou_link")]
sys.modules[PKG] = package
protocol = importlib.import_module(PKG + ".protocol")
client_module = importlib.import_module(PKG + ".client")
InstanceClient = importlib.import_module(PKG + ".instance_client").InstanceClient
Inbox = importlib.import_module(PKG + ".state").Inbox

WELCOME = {"type": "welcome", "server": "minihou", "protocol": 1,
           "capabilities": ["instances_v1", "live_modifier_v1", "object_inputs_v1"]}
MESH = {"point/P": np.array([[4, 0, 0]], np.float32),
        "topo/vertex_point": np.array([], np.int32),
        "topo/prim_offsets": np.array([0], np.int32)}


def wait_for(predicate, timeout=3):
    deadline = time.monotonic() + timeout
    while not predicate():
        if time.monotonic() >= deadline:
            raise AssertionError("condition did not become true")
        time.sleep(.005)


def stub(mode, port):
    listener = socket.socket()
    listener.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    listener.bind(("127.0.0.1", port))
    listener.listen()
    print(listener.getsockname()[1], flush=True)
    connection, _ = listener.accept()
    connection.settimeout(5)
    protocol.recv_message(connection)
    if mode == "handshake":
        print("PENDING", flush=True)
        threading.Event().wait(20)
        return
    protocol.send_message(connection, WELCOME)
    if mode in ("partial", "rst", "eof"):
        connection.sendall(b"MHL1\x10")  # EOF halfway through a prefix
        print("PENDING", flush=True)
        if mode == "rst":
            connection.setsockopt(socket.SOL_SOCKET, socket.SO_LINGER, struct.pack("hh" if os.name == "nt" else "ii", 1, 0))
            connection.close()
        elif mode == "eof":
            connection.shutdown(socket.SHUT_RDWR)
            connection.close()
        else:
            threading.Event().wait(20)
        return
    if mode == "backpressure":
        connection.setsockopt(socket.SOL_SOCKET, socket.SO_RCVBUF, 1024)
        print("PENDING", flush=True)
        threading.Event().wait(20)
        return
    while True:
        h, _ = protocol.recv_message(connection)
        if h["type"].startswith("instance_"):
            print("PENDING", flush=True)
            if mode == "rpc":
                threading.Event().wait(20)
                return
            reply = dict(type={"instance_cook": "instance_result", "instance_describe": "instance_description",
                               "instance_edit": "instance_synced", "instance_sync": "instance_synced",
                               "instance_sources": "instance_sources_set", "instance_remove": "instance_removed"}[h["type"]],
                         request_id=h["request_id"], instance_id=h.get("instance_id"), interface=[],
                         graph=h.get("graph") or {}, full=True, seq=h["request_id"], topology_changed=True,
                         counts={"point": 1, "vertex": 0, "prim": 0},
                         manifest={"point/P": {"dtype": "<f4", "shape": [1, 3]}}, detail={})
            protocol.send_message(connection, reply, MESH if h["type"] == "instance_cook" else None)
        elif h["type"] == "ping":
            protocol.send_message(connection, {"type": "pong"})


class Host:
    def __init__(self, mode="normal", port=0):
        # Blender's sys.executable is Blender, so use the bundled Python instead.
        executable = sys.executable
        if "blender" in Path(executable).stem.lower():
            import bpy
            executable = str(Path(bpy.app.binary_path).parent / (f"{bpy.app.version[0]}.{bpy.app.version[1]}") /
                             "python/bin/python.exe")
        self.process = subprocess.Popen([executable, "-u", str(Path(__file__).resolve()), "--stub", mode, str(port)],
                                        stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True,
                                        creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0))
        self.lines = queue.Queue()
        def read():
            for line in self.process.stdout:
                self.lines.put(line.strip())
        self.reader = threading.Thread(target=read, daemon=True)
        self.reader.start()
        self.port = int(self.lines.get(timeout=10))

    def pending(self):
        if self.lines.get(timeout=3) != "PENDING":
            raise AssertionError("missing pending operation")

    def close(self):
        if self.process.poll() is None:
            self.process.kill()  # Only the stub process this test created.
        self.process.wait(timeout=3)
        self.reader.join(.5)
        self.process.stdout.close()
        self.process.stderr.close()


class DisconnectTests(unittest.TestCase):
    def host(self, *args):
        host = Host(*args)
        self.addCleanup(host.close)
        return host

    def link(self, host, **kwargs):
        inbox = Inbox()
        states = []
        client = client_module.LinkClient(port=host.port, retry_seconds=.05,
                                          on_message=inbox.push, on_state=lambda s, d: states.append(s), **kwargs)
        self.addCleanup(client.stop)
        client.start()
        wait_for(lambda: "connected" in states)
        return client, inbox, states

    def test_partial_eof_rst_and_process_kill_reconnect(self):
        for mode in ("partial", "rst", "eof"):
            with self.subTest(mode=mode):
                host = self.host(mode)
                client, _, states = self.link(host)
                host.pending()
                host.close()
                wait_for(lambda: "disconnected" in states)
                restarted = self.host("normal", host.port)
                wait_for(lambda: states.count("connected") >= 2)
                client.stop()
                wait_for(lambda: not client._thread.is_alive())
                self.assertFalse(client._writer.is_alive())
                restarted.close()

    def test_idle_link_is_not_a_partial_frame_timeout(self):
        host = self.host()
        client, _, _ = self.link(host, reconnect=False)
        time.sleep(.35)
        self.assertEqual(client.state, "connected")

    def test_timer_send_and_stop_do_not_wait_for_backpressure(self):
        host = self.host("backpressure")
        client, inbox, _ = self.link(host)
        host.pending()
        actual_send = client_module.send_message
        callers = []
        def tracked(*args, **kwargs):
            callers.append(threading.current_thread())
            return actual_send(*args, **kwargs)
        with patch.object(client_module, "send_message", tracked):
            started = time.monotonic()
            client.send({"type": "object_geo"}, {"point/P": np.zeros((1_000_000, 3), np.float32)})
            for _ in range(8):
                inbox.take()
                client.set_param("Size", 2)
                client.send({"type": "set_frame", "frame": 2, "fps": 24})
                client.resync()
            self.assertLess(time.monotonic() - started, .1)
            wait_for(lambda: callers)
            started = time.monotonic()
            client.stop()
            self.assertLess(time.monotonic() - started, .1)
            wait_for(lambda: not client._thread.is_alive(), .75)
            self.assertTrue(all(t is not threading.main_thread() for t in callers))

    def test_handshake_and_pending_rpc_have_one_total_budget(self):
        for mode in ("handshake", "rpc"):
            with self.subTest(mode=mode):
                host = self.host(mode)
                client = InstanceClient(port=host.port, timeout=30)
                self.addCleanup(client.close)
                started = time.monotonic()
                with self.assertRaises(OSError):
                    client.cook("pending", "", {}, MESH)
                elapsed = time.monotonic() - started
                self.assertLess(elapsed, .25)
                self.assertIsNone(client.sock)
                wait_for(lambda: not client._thread.is_alive(), .5)

    def test_kill_during_rpc_then_same_client_can_reconnect(self):
        host = self.host("rpc")
        client = InstanceClient(port=host.port)
        self.addCleanup(client.close)
        client.connect()
        # Older modifier callers override both fields; the UI bound still applies.
        client.timeout = 30
        client.sock.settimeout(30)
        killer = threading.Thread(target=lambda: (host.pending(), host.close()))
        killer.start()
        started = time.monotonic()
        with self.assertRaises(OSError):
            client.cook("pending", "", {}, MESH)
        self.assertLess(time.monotonic() - started, .25)
        killer.join(3)
        self.assertFalse(killer.is_alive())
        restarted = self.host("normal", host.port)
        header, output = client.cook("pending", "", {}, MESH)
        self.assertEqual(header["type"], "instance_result")
        np.testing.assert_array_equal(output["point/P"], MESH["point/P"])
        restarted.close()

    def test_close_interrupts_an_rpc_without_waiting_for_its_lock(self):
        host = self.host("rpc")
        client = InstanceClient(port=host.port)
        self.addCleanup(client.close)
        errors = []
        def cook():
            try:
                client.cook("pending", "", {}, MESH)
            except OSError as exc:
                errors.append(exc)
        worker = threading.Thread(target=cook)
        worker.start()
        host.pending()
        started = time.monotonic()
        client.close()
        self.assertLess(time.monotonic() - started, .1)
        worker.join(.5)
        self.assertFalse(worker.is_alive())
        self.assertTrue(errors)

    def test_close_during_connect_cannot_publish_a_late_socket(self):
        host = self.host()
        client = InstanceClient(port=host.port)
        self.addCleanup(client.close)
        entered, release = threading.Event(), threading.Event()
        actual_connect = socket.create_connection
        errors = []
        def delayed_connect(*args, **kwargs):
            connection = actual_connect(*args, **kwargs)
            entered.set()
            release.wait(.1)
            return connection
        def connect():
            try:
                client.connect()
            except OSError as exc:
                errors.append(exc)
        module = sys.modules[InstanceClient.__module__]
        with patch.object(module.socket, "create_connection", delayed_connect):
            worker = threading.Thread(target=connect)
            worker.start()
            self.assertTrue(entered.wait(.2))
            client.close()
            release.set()
            worker.join(.5)
        self.assertFalse(worker.is_alive())
        self.assertIsNone(client.sock)
        self.assertTrue(errors)

    def test_partial_trickle_has_total_deadline_and_eof_is_terminal(self):
        a, b = socket.socketpair()
        self.addCleanup(a.close)
        self.addCleanup(b.close)
        a.settimeout(.01)
        def trickle():
            try:
                for byte in b"MHL1\x10\0\0\0":
                    b.send(bytes([byte]))
                    time.sleep(.03)
            except OSError:
                pass
        thread = threading.Thread(target=trickle)
        thread.start()
        with self.assertRaises(TimeoutError):
            protocol.recv_message(a, deadline=time.monotonic() + .08)
        thread.join(.5)
        b.close()
        with self.assertRaises(ConnectionError):
            protocol.recv_message(a)


try:
    import bpy
except ImportError:
    bpy = None


@unittest.skipIf(bpy is None, "requires headless Blender")
class BlenderDisconnectTests(DisconnectTests):
    # Inherit the same real-socket cases under Blender's bundled Python as well.
    def test_real_runtime_timer_and_manual_disconnect(self):
        sys.path.insert(0, str(ROOT / "blender_addon"))
        from minihou_link import runtime
        host = self.host("backpressure")
        rt = runtime.get_runtime()
        self.addCleanup(runtime.shutdown)
        rt.connect(port=host.port)
        wait_for(lambda: rt.state == "connected")
        host.pending()
        rt.client.send({"type": "object_geo"}, {"point/P": np.zeros((1_000_000, 3), np.float32)})
        host.close()
        started = time.monotonic()
        for _ in range(5):
            rt.frame_dirty = True
            rt._timer()
        self.assertLess(time.monotonic() - started, .1)
        wait_for(lambda: rt.state == "disconnected")
        started = time.monotonic()
        rt.disconnect()
        rt.poll_auto_connect(True, host.port)
        self.assertLess(time.monotonic() - started, .1)
        self.assertIsNone(rt.client)
        self.assertTrue(rt.auto_connect_paused)

    def test_modifier_pending_rpc_retains_last_good_mesh(self):
        sys.path.insert(0, str(ROOT / "blender_addon"))
        from minihou_link import modifiers
        self.addCleanup(modifiers.disconnect)
        host = self.host("normal")
        mesh = bpy.data.meshes.new("Disconnect source")
        mesh.from_pydata([(1, 0, 0)], [], [])
        obj = bpy.data.objects.new("Disconnect source", mesh)
        bpy.context.scene.collection.objects.link(obj)
        mod = modifiers.add_modifier(obj, "", port=host.port)
        self.assertEqual(modifiers.pump(port=host.port, force=True, _only=(obj, mod)), 1, modifiers.last_error())
        cache = modifiers._cache(mod)
        before = [tuple(v.co) for v in cache.data.vertices]
        self.assertEqual(before, [(4.0, 0.0, 0.0)])
        host.close()
        # Force a fresh connection to a server that accepts the cook but never replies.
        modifiers.disconnect()
        stalled = self.host("rpc", host.port)
        client = modifiers._connection(port=stalled.port)
        started = time.monotonic()
        self.assertEqual(modifiers.pump(port=stalled.port, force=True, _only=(obj, mod)), 0)
        self.assertLess(time.monotonic() - started, .25)
        self.assertEqual([tuple(v.co) for v in cache.data.vertices], before)
        self.assertIn("last cache retained", modifiers.status(mod))
        wait_for(lambda: not client._thread.is_alive(), .5)
        bpy.context.scene.frame_set(2)  # Depsgraph/timeline evaluation still returns.

    def test_unregister_does_not_join_server_cleanup_on_main_thread(self):
        sys.path.insert(0, str(ROOT / "blender_addon"))
        import minihou_link
        minihou_link.register()
        with patch.object(minihou_link._server, "stop") as stop:
            started = time.monotonic()
            minihou_link.unregister()
            self.assertLess(time.monotonic() - started, .1)
            stop.assert_called_once_with(wait=False)


if __name__ == "__main__":
    if "--stub" in sys.argv:
        index = sys.argv.index("--stub")
        stub(sys.argv[index + 1], int(sys.argv[index + 2]))
    else:
        result = unittest.main(argv=[sys.argv[0]], exit=False)
        if not result.result.wasSuccessful():
            raise SystemExit(1)
