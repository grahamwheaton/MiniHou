"""Runner for the Rust app's opt-in ``python`` node bridge (see docs/RUST_PYTHON_NODE.md).

    python -B python_node_runner.py <job_dir>

Reads ``<job_dir>/job.mhl``, runs the snippet with the same names in scope as ``minihou.nodes.python_node`` and writes
``<job_dir>/out.mhl``. Both files use the live link's MHL1 framing (JSON header + 8-byte aligned little-endian arrays).

This is NOT a sandbox. The Rust app only starts this script after the user enabled Python nodes and confirmed the snippet.

When the repo's ``minihou`` package imports, its Geometry, groups, timectx and the python node itself are used, so results
match the Python engine exactly. Otherwise a small numpy-only stand-in with the same attribute API is used.
Standard library + numpy only; keep this file self-contained (the Rust core embeds it with include_str!).
"""
from __future__ import annotations

import json
import os
import struct
import sys
import traceback

import numpy as np

WIRE_VERSION = 1
MAGIC = b"MHL1"
_DT = {"<f4": np.float32, "<i4": np.int32, "|b1": np.bool_, "|u1": np.uint8}


def read_frame(path):
    with open(path, "rb") as f:
        data = f.read()
    if data[:4] != MAGIC:
        raise ValueError("bad magic")
    hlen, blen = struct.unpack("<IQ", data[4:16])
    if 16 + hlen + blen != len(data):
        raise ValueError("frame size mismatch")
    header = json.loads(data[16:16 + hlen].decode("utf-8"))
    blob = memoryview(data)[16 + hlen:]
    arrays = {}
    for d in header.get("arrays", []):
        dt = np.dtype(_DT[d["dtype"]])
        a = np.frombuffer(blob[d["offset"]:d["offset"] + d["nbytes"]], dtype=dt).reshape(d["shape"]).copy()
        arrays[d["key"]] = a
    return header, arrays


def write_frame(path, header, arrays):
    blob = bytearray()
    descs = []
    for key, a in arrays.items():
        a = np.ascontiguousarray(a)
        if a.dtype == np.float32:
            dt = "<f4"
        elif a.dtype == np.int32:
            dt = "<i4"
        elif a.dtype == np.bool_:
            dt = "|b1"
        elif a.dtype == np.uint8:
            dt = "|u1"
        else:
            raise TypeError(f"{key}: unsupported dtype {a.dtype}")
        raw = a.astype(a.dtype.newbyteorder("<"), copy=False).tobytes()
        descs.append({"key": key, "dtype": dt, "shape": list(a.shape), "offset": len(blob), "nbytes": len(raw)})
        blob += raw
        blob += b"\0" * ((8 - len(blob) % 8) % 8)
    header = dict(header)
    if descs:
        header["arrays"] = descs
    hb = json.dumps(header, default=_json_default, allow_nan=False).encode("utf-8")
    hb += b" " * ((8 - len(hb) % 8) % 8)
    tmp = path + ".tmp"
    with open(tmp, "wb") as f:
        f.write(MAGIC + struct.pack("<IQ", len(hb), len(blob)) + hb + bytes(blob))
    os.replace(tmp, path)


def _json_default(o):
    if isinstance(o, np.ndarray):
        return o.tolist()
    if isinstance(o, np.generic):
        return o.item()
    if isinstance(o, (set, frozenset, tuple)):
        return list(o)
    raise TypeError(f"detail value of type {type(o).__name__} cannot be stored")


# --- numpy-only stand-ins, used when `import minihou` fails -----------------------------------------------------------
class _Table(dict):
    def __init__(self, size=0, attrs=None):
        super().__init__(attrs or {})
        self.size = size
        self._frozen = False

    def names(self):
        return list(self.keys())

    def __setitem__(self, name, arr):
        if self._frozen:
            raise ValueError("table is frozen (cooked geometry is immutable); call geo.copy() first")
        a = np.asarray(arr)
        a = a.astype(np.float32) if a.dtype.kind == "f" else a.astype(np.int32) if a.dtype.kind in "iu" else a
        if a.shape[:1] != (self.size,):
            raise ValueError(f"attribute {name!r}: expected {self.size} rows, got {a.shape}")
        super().__setitem__(name, np.ascontiguousarray(a))

    def copy(self):
        return _Table(self.size, dict(self))

    def freeze(self):
        self._frozen = True
        for a in self.values():
            a.flags.writeable = False


class _Topology:
    def __init__(self, vertex_point=None, prim_offsets=None, closed=None):
        self.vertex_point = np.zeros(0, np.int32) if vertex_point is None else np.ascontiguousarray(vertex_point, np.int32)
        self.prim_offsets = np.zeros(1, np.int32) if prim_offsets is None else np.ascontiguousarray(prim_offsets, np.int32)
        self.closed = None if closed is None else np.ascontiguousarray(closed, np.bool_)

    nvertices = property(lambda self: int(self.vertex_point.shape[0]))
    nprims = property(lambda self: int(self.prim_offsets.shape[0] - 1))

    def freeze(self):
        for a in (self.vertex_point, self.prim_offsets, self.closed):
            if a is not None:
                a.flags.writeable = False


class _Geometry:
    def __init__(self, npoints=0, topology=None):
        self.topology = topology or _Topology()
        self.point, self.vertex, self.prim = _Table(npoints), _Table(self.topology.nvertices), _Table(self.topology.nprims)
        self.detail = {}

    @classmethod
    def from_arrays(cls, P, vertex_point=None, prim_offsets=None, closed=None):
        P = np.asarray(P, np.float32)
        g = cls(P.shape[0], _Topology(vertex_point, prim_offsets, closed))
        g.point["P"] = P
        return g

    @classmethod
    def points_only(cls, P):
        return cls.from_arrays(P)

    def table(self, domain):
        return getattr(self, domain)

    def set_topology(self, topology):
        self.topology = topology
        self.vertex, self.prim = _Table(topology.nvertices), _Table(topology.nprims)

    npoints = property(lambda self: self.point.size)
    nvertices = property(lambda self: self.topology.nvertices)
    nprims = property(lambda self: self.topology.nprims)
    P = property(lambda self: self.point["P"], lambda self, v: self.point.__setitem__("P", v))

    def copy(self):
        g = _Geometry.__new__(_Geometry)
        g.topology, g.point, g.vertex, g.prim, g.detail = self.topology, self.point.copy(), self.vertex.copy(), self.prim.copy(), dict(self.detail)
        return g

    def freeze(self):
        self.topology.freeze()
        for t in (self.point, self.vertex, self.prim):
            t.freeze()
        return self


def _api():
    """(Geometry, Topology, groups module or None, timectx or None, python_node or None)."""
    try:
        from minihou import groups, timectx
        from minihou.geometry import Geometry, Topology
        from minihou.nodes import python_node
        return Geometry, Topology, groups, timectx, python_node
    except Exception:  # noqa: BLE001 - any import problem means: use the numpy-only stand-ins
        return _Geometry, _Topology, None, None, None


def decode_geometry(arrays, prefix, npoints, detail, Geometry, Topology):
    closed = arrays.get(prefix + "topo/prim_closed")
    topo = Topology(arrays.get(prefix + "topo/vertex_point"), arrays.get(prefix + "topo/prim_offsets"),
                    None if closed is None else closed.astype(np.bool_))
    g = Geometry(npoints, topo)
    for key, a in arrays.items():
        if not key.startswith(prefix):
            continue
        dom, _, name = key[len(prefix):].partition("/")
        if dom in ("point", "vertex", "prim"):
            g.table(dom)[name] = a
    g.detail = detail
    return g


def encode_geometry(g):
    arrays = {}
    for dom in ("point", "vertex", "prim"):
        t = g.table(dom)
        for name in t.names():
            a = np.asarray(t[name])
            if a.ndim > 2:
                a = a.reshape(a.shape[0], -1)
            arrays[f"{dom}/{name}"] = a
    arrays["topo/vertex_point"] = np.asarray(g.topology.vertex_point, np.int32)
    arrays["topo/prim_offsets"] = np.asarray(g.topology.prim_offsets, np.int32)
    closed = getattr(g.topology, "closed", None)
    if closed is not None and not np.all(closed):
        arrays["topo/prim_closed"] = np.asarray(closed, np.bool_).astype(np.uint8)
    return arrays


def run(job_dir):
    header, arrays = read_frame(os.path.join(job_dir, "job.mhl"))
    if header.get("type") != "python_job" or header.get("version") != WIRE_VERSION:
        raise ValueError(f"unsupported job (type {header.get('type')!r}, version {header.get('version')!r}); "
                         f"this runner speaks version {WIRE_VERSION}")
    Geometry, Topology, groups, timectx, python_node = _api()
    ins = []
    for k, desc in enumerate(header["inputs"]):
        if desc is None:
            ins.append(None)
        else:
            ins.append(decode_geometry(arrays, f"in{k}/", desc["npoints"], desc.get("detail") or {}, Geometry, Topology).freeze())
    p = header["params"]
    t = header["time"]
    kw = dict(code=p["code"], a=p["a"], b=p["b"], c=p["c"], d=p["d"], i=p["i"], j=p["j"], v=tuple(p["v"]), s=p["s"],
              flag=p["flag"], variables=p["variables"], animated=p["animated"])
    if python_node is not None:
        python_node.set_allowed(True)  # the Rust app already decided; this process exists only because the user said yes
        with timectx.at(t["frame"], t["fps"]), timectx.step(t["time_inc"]):
            out = python_node.python(*ins, **kw)
    else:
        out = _run_standalone(ins, kw, t, Geometry)
    if not hasattr(out, "topology"):
        raise ValueError("python node: the result is not a Geometry")
    write_frame(os.path.join(job_dir, "out.mhl"),
                {"type": "python_result", "version": WIRE_VERSION, "npoints": int(out.npoints), "detail": dict(out.detail),
                 "engine": "minihou" if python_node is not None else "standalone"}, encode_geometry(out))


def _run_standalone(ins, kw, t, Geometry):
    import ast
    geo = ins[0]
    ns = {"np": np, "Geometry": Geometry, "groups": None, "geo": geo, "input1": ins[1], "input2": ins[2], "input3": ins[3],
          "inputs": list(ins), "a": kw["a"], "b": kw["b"], "c": kw["c"], "d": kw["d"], "i": kw["i"], "j": kw["j"],
          "v": kw["v"], "s": kw["s"], "flag": kw["flag"], "Frame": t["frame"], "Time": (t["frame"] - 1.0) / t["fps"],
          "TimeInc": t["time_inc"], "out": None}
    for stmt in ast.parse(kw["variables"] or "").body:
        if not (isinstance(stmt, ast.Assign) and len(stmt.targets) == 1 and isinstance(stmt.targets[0], ast.Name)):
            raise ValueError(f"Variables line {stmt.lineno}: write 'name = value'")
        ns[stmt.targets[0].id] = ast.literal_eval(stmt.value)
    exec(compile(kw["code"], "<python node>", "exec"), ns)  # noqa: S102 - the point of the node
    result = ns.get("out")
    if result is None:
        if geo is None:
            raise ValueError("python node: nothing wired in and `out` was not set")
        return geo
    if hasattr(result, "topology"):
        return result
    arr = np.asarray(result, np.float32)
    if arr.ndim == 2 and arr.shape[1] == 3:
        return Geometry.points_only(arr)
    raise ValueError(f"python node: `out` must be a Geometry or an (n, 3) array of points, not {type(result).__name__}")


def main(argv):
    if len(argv) != 2:
        print("usage: python_node_runner.py <job_dir>", file=sys.stderr)
        return 64
    try:
        run(argv[1])
    except BaseException as e:  # noqa: BLE001 - report anything, with the traceback, then the one-line summary last
        traceback.print_exc(file=sys.stderr)
        print(str(e) if str(e).startswith("python node") else f"{type(e).__name__}: {e}", file=sys.stderr)
        return 2
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
