"""Benchmark the live link's Blender end: socket receive + ``foreach_set`` of an N-point position array.

Run inside Blender (this is what you want on Windows; see README):

    blender --background --factory-startup --python bench/bench_link.py -- --points 1000000

or with the ``bpy`` module:   python bench/bench_link.py --points 1000000

Modes
-----
raw     (default) a minimal sender process pushes a position array over a real loopback socket using
        the real wire protocol. Isolates transport + decode + ``foreach_set`` + ``mesh.update``.
engine  starts ``python -m minihou serve`` on a generated grid+noise graph and measures the whole path for a
        slider-style edit: set_param -> cook -> diff -> socket -> foreach_set.

``--shape points`` is a bare point cloud (no faces); ``--shape mesh`` is a quad grid with ~N points, whose
first frame also carries topology (that frame is reported separately).
"""
import argparse
import json
import math
import os
import re
import statistics
import subprocess
import sys
import tempfile
import time

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
sys.path[:0] = [ROOT, os.path.join(ROOT, "blender_addon")]

import bpy  # noqa: E402

from minihou_link import mesh_writer  # noqa: E402
from minihou_link.client import LinkClient  # noqa: E402
from minihou_link.protocol import Mirror  # noqa: E402
from minihou_link.state import Inbox  # noqa: E402


def script_args():
    argv = sys.argv[sys.argv.index("--") + 1:] if "--" in sys.argv else sys.argv[1:]
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--mode", choices=["raw", "engine"], default="raw")
    ap.add_argument("--shape", choices=["points", "mesh"], default="points")
    ap.add_argument("--points", type=int, default=1_000_000)
    ap.add_argument("--iterations", type=int, default=15)
    ap.add_argument("--python", default=os.environ.get("MINIHOU_ENGINE_PYTHON", sys.executable),
                    help="python used to run the sender/engine subprocess (default: $MINIHOU_ENGINE_PYTHON, else this python)")
    return ap.parse_args(argv)


def spawn(cmd):
    """Start a subprocess and return (process, port) once it prints its "listening ... :PORT" line.
    Startup may print other lines first (e.g. the compute backend), so scan rather than read one line."""
    p = subprocess.Popen(cmd, cwd=ROOT, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True,
                         encoding="utf-8", errors="replace")
    seen = []
    for _ in range(50):
        line = p.stdout.readline()
        if not line:
            break
        seen.append(line)
        m = re.search(r"(?:listening |on 127\.0\.0\.1:)(\d+)", line)
        if m:
            return p, int(m.group(1))
    p.kill()
    raise RuntimeError("subprocess failed to start:\n" + "".join(seen))


class Receiver:
    """Client + inbox + mesh, driven synchronously so each stage can be timed."""

    def __init__(self, port):
        self.inbox, self.mirror = Inbox(), Mirror()
        self.mesh = bpy.data.meshes.new("bench")
        self.client = LinkClient("127.0.0.1", port, on_message=self.inbox.push, client_name="bench", reconnect=False)
        self.client.start()

    def next_update(self, timeout=60.0):
        end = time.monotonic() + timeout
        while time.monotonic() < end:
            _events, geo = self.inbox.take()
            if geo is not None:
                return geo
            time.sleep(0.0005)
        raise TimeoutError("no geometry received")

    def apply(self, u):
        t0 = time.perf_counter()
        self.mirror.apply(u)
        mirror_ms = (time.perf_counter() - t0) * 1000
        t = mesh_writer.apply_update(self.mesh, self.mirror, u)
        t["mirror_ms"] = mirror_ms
        return t

    def stop(self):
        self.client.stop()


def stats(vals):
    return (statistics.median(vals), min(vals), max(vals))


def report(title, rows, mb):
    print(f"\n{title}")
    print(f"  {'stage':34s} {'median':>9s} {'min':>9s} {'max':>9s}   (ms)")
    for name, vals in rows:
        med, lo, hi = stats(vals)
        extra = f"   {mb / (med / 1000):7.0f} MB/s" if name.startswith(("transport", "foreach_set")) and med > 0 else ""
        print(f"  {name:34s} {med:9.2f} {lo:9.2f} {hi:9.2f}{extra}")


def run_raw(a):
    sender, port = spawn([a.python, "-u", os.path.join(HERE, "raw_sender.py"), "--points", str(a.points),
                          "--mode", a.shape, "--iterations", str(a.iterations)])
    rx = Receiver(port)
    try:
        first = rx.next_update()
        t = rx.apply(first)
        n = rx.mirror.counts["point"]
        print(f"Blender {bpy.app.version_string}, {n:,} points, {rx.mirror.counts['prim']:,} faces, shape={a.shape}")
        print(f"first (full) frame: {first.nbytes / 1e6:.1f} MB on the wire, transport {(first.t_recv - first.t_sent) * 1000:.1f} ms; "
              f"topology {t['topology_ms']:.1f} ms, foreach_set(co) {t['positions_ms']:.1f} ms, "
              f"mesh.update {t['update_ms']:.1f} ms, total {t['topology_ms'] + t['positions_ms'] + t['attributes_ms'] + t['update_ms']:.1f} ms")
        rx.client.send({"type": "ping", "t": first.seq})
        series = {k: [] for k in ("transport", "decode", "foreach", "update", "total")}
        for _ in range(a.iterations):
            u = rx.next_update()
            t = rx.apply(u)
            assert not t["rebuilt"], "expected a positions-only update"
            series["transport"].append((u.t_recv - u.t_sent) * 1000)
            series["decode"].append(t["mirror_ms"])
            series["foreach"].append(t["positions_ms"])
            series["update"].append(t["update_ms"])
            series["total"].append(t["mirror_ms"] + t["positions_ms"] + t["attributes_ms"] + t["update_ms"])
            rx.client.send({"type": "ping", "t": u.seq})
        mb = n * 12 / 1e6
        report(f"positions-only updates ({mb:.1f} MB each), {a.iterations} frames",
               [("transport (send->received)", series["transport"]), ("mirror/decode", series["decode"]),
                ("foreach_set(co)", series["foreach"]), ("mesh.update()", series["update"]),
                ("Blender-side apply total", series["total"])], mb)
        verdict(statistics.median(series["total"]) + statistics.median(series["transport"]))
    finally:
        rx.stop()
        sender.wait(10)


def run_engine(a):
    side = max(2, int(math.isqrt(a.points)))
    graph = {"version": 1, "nodes": {
        "g": {"type": "grid", "params": {"columns": side, "rows": side, "size_x": 10, "size_y": 10}},
        "n": {"type": "noise", "params": {"amplitude": [0, 0, 1], "frequency": 0.5}, "inputs": {"geo": "g"}},
        "o": {"type": "output", "inputs": {"geo": "n"}}},
        "display": "o", "exposed": [{"name": "Frequency", "node": "n", "param": "frequency"}]}
    with tempfile.TemporaryDirectory() as d:
        path = os.path.join(d, "bench_graph.json")
        with open(path, "w", encoding="utf-8") as f:
            json.dump(graph, f)
        eng, port = spawn([a.python, "-u", "-m", "minihou", "serve", path, "--port", "0", "--no-watch"])
        rx = Receiver(port)
        try:
            first = rx.next_update(120)
            t = rx.apply(first)
            n = rx.mirror.counts["point"]
            print(f"Blender {bpy.app.version_string}, engine graph grid {side}x{side} -> noise, {n:,} points")
            print(f"first (full) frame: {first.nbytes / 1e6:.1f} MB, cook {first.cook.get('ms', 0):.0f} ms, transport "
                  f"{(first.t_recv - first.t_sent) * 1000:.0f} ms, Blender apply "
                  f"{t['topology_ms'] + t['positions_ms'] + t['attributes_ms'] + t['update_ms']:.0f} ms")
            series = {k: [] for k in ("e2e", "cook", "transport", "apply")}
            for i in range(a.iterations):
                t0 = time.perf_counter()
                rx.client.set_param("Frequency", 0.5 + 0.01 * (i + 1))
                u = rx.next_update(60)
                t = rx.apply(u)
                e2e = (time.perf_counter() - t0) * 1000
                assert not u.topology_changed and sorted(u.arrays) == ["point/P"], sorted(u.arrays)
                series["e2e"].append(e2e)
                series["cook"].append(float(u.cook.get("ms", 0)))
                series["transport"].append((u.t_recv - u.t_sent) * 1000)
                series["apply"].append(t["mirror_ms"] + t["positions_ms"] + t["attributes_ms"] + t["update_ms"])
            mb = n * 12 / 1e6
            report(f"slider edit -> mesh updated ({mb:.1f} MB of positions each), {a.iterations} edits",
                   [("end-to-end (set_param -> applied)", series["e2e"]), ("engine cook (noise only; grid cached)", series["cook"]),
                    ("transport (send->received)", series["transport"]), ("Blender-side apply", series["apply"])], mb)
            verdict(statistics.median(series["e2e"]))
        finally:
            rx.stop()
            eng.terminate()
            eng.wait(10)


def verdict(ms):
    fps = 1000.0 / ms if ms > 0 else float("inf")
    tag = "interactive (>= 30 fps)" if ms <= 33.4 else "usable (>= 10 fps)" if ms <= 100 else "laggy (< 10 fps)"
    print(f"\n=> median full path {ms:.1f} ms  (~{fps:.0f} updates/s): {tag}")


def main():
    a = script_args()
    {"raw": run_raw, "engine": run_engine}[a.mode](a)
    if bpy.app.background:
        sys.stdout.flush()


if __name__ == "__main__":
    main()
