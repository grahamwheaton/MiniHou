"""Minimal sender for the link benchmark: serves N-point position arrays using the real wire protocol,
with no engine/graph involved, so transport + Blender write cost can be measured in isolation.

    python bench/raw_sender.py --points 1000000 --mode points --port 0

Prints ``listening <port>`` once ready. After the client's hello it sends a full update, then (paced
by the client's ``ping`` acks) ``--iterations`` position-only updates.
"""
import argparse
import math
import os
import socket
import sys
import time

import numpy as np

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
from minihou.link.protocol import (PROTOCOL_VERSION, TOPO_PRIM_OFFSETS, TOPO_VERTEX_POINT, recv_message,  # noqa: E402
                                   send_message)


def make_geometry(points: int, mode: str):
    rng = np.random.default_rng(0)
    if mode == "points":
        P = [rng.random((points, 3), dtype=np.float32) for _ in range(2)]
        return P, None, None, points
    side = int(math.isqrt(points))
    n = side * side
    u = np.linspace(0, 1, side, dtype=np.float32)
    uu, vv = np.meshgrid(u, u)
    base = np.stack([uu.ravel(), vv.ravel(), np.zeros(n, np.float32)], axis=1)
    P = [base, base + rng.random((n, 3), dtype=np.float32) * 0.01]
    ii, jj = np.meshgrid(np.arange(side - 1, dtype=np.int32), np.arange(side - 1, dtype=np.int32))
    b = (jj * side + ii).ravel()
    vp = np.stack([b, b + 1, b + side + 1, b + side], axis=1).ravel().astype(np.int32)
    po = (np.arange(b.size + 1, dtype=np.int32) * 4)
    return P, vp, po, n


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--points", type=int, default=1_000_000)
    ap.add_argument("--mode", choices=["points", "mesh"], default="points")
    ap.add_argument("--iterations", type=int, default=10)
    ap.add_argument("--port", type=int, default=0)
    a = ap.parse_args()

    P, vp, po, n = make_geometry(a.points, a.mode)
    nv, npr = (0, 0) if vp is None else (vp.size, po.size - 1)
    srv = socket.socket()
    srv.bind(("127.0.0.1", a.port))
    srv.listen(1)
    print(f"listening {srv.getsockname()[1]}", flush=True)
    conn, _ = srv.accept()
    conn.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
    hello, _ = recv_message(conn)
    assert hello["type"] == "hello"
    send_message(conn, {"type": "welcome", "protocol": PROTOCOL_VERSION})

    def send_geo(seq, full):
        arrays = {"point/P": P[seq % 2]}
        if full and vp is not None:
            arrays[TOPO_VERTEX_POINT], arrays[TOPO_PRIM_OFFSETS] = vp, po
        header = {"type": "geo", "seq": seq, "full": full,
                  "counts": {"point": n, "vertex": nv, "prim": npr}, "topology_changed": full,
                  "manifest": {"point/P": {"dtype": "<f4", "shape": [n, 3]}}, "detail": {}, "cook": {"ms": 0.0},
                  "t_sent": time.time()}
        send_message(conn, header, arrays)

    send_geo(1, True)
    for seq in range(2, a.iterations + 2):
        while True:                                    # wait for the client's ack of the previous frame
            msg, _ = recv_message(conn)
            if msg["type"] == "ping" and msg.get("t") == seq - 1:
                break
        send_geo(seq, False)
    recv_message(conn)                                 # final ack
    conn.close()


if __name__ == "__main__":
    main()
