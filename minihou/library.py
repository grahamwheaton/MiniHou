"""Node libraries: Python files of ``@node`` functions that the engine loads and hot-reloads.

Put files in ``<workspace>/nodes/`` (or pass ``serve --nodes DIR``). Each ``*.py`` file (names starting with ``_`` or
``.`` are skipped) is executed and its ``@node`` functions become node types, available in the editor's Tab menu and to
AI tools like built-in nodes. When a file changes it is re-executed:

* its node types are replaced as a unit (nodes removed from the file disappear);
* every node type carries a hash of its file, which is part of the cache key, so results never go stale;
* if the new code fails (syntax error, exception at import, a clash with an existing node name), the previous version
  stays loaded and the error (with file and line) is reported to the editor and AI tools.

Library files are ordinary Python running inside the engine, with the same trust as any script you run: only put files
you trust in a node folder. Opening a graph never executes code; graphs refer to node types by name.
"""
from __future__ import annotations

import hashlib
import logging
import threading
import traceback
import types
from dataclasses import dataclass, field
from pathlib import Path
from typing import Dict, Iterable, List, Optional

from . import sdk
from .sdk import REGISTRY, Registry

log = logging.getLogger("minihou.library")

_LOAD_LOCK = threading.Lock()              # sdk._LOADING is process-wide

TEMPLATE = '''"""{name}: describe what this family of nodes does."""
import numpy as np

from minihou.sdk import Float, Int, Vec3, node


@node("{name}", inputs=["geo"], category="custom", params=[
    Float("amount", 1.0, soft_min=0.0, soft_max=10.0, doc="How strong the effect is"),
])
def {name}(geo, amount):
    """One line on what the node does (shown in the editor and to AI tools)."""
    out = geo.copy()                                   # inputs are read-only: copy, then replace arrays
    P = geo.P.copy()
    P[:, 2] += amount
    out.point["P"] = P
    return out
'''


AUTHORING_GUIDE = """\
Writing a node: a Python file in the node folder (one file can hold several nodes). It is loaded within a second of
saving and re-loaded on every save; errors are reported with file and line, and the previous version stays active.

    import numpy as np
    from minihou.sdk import node, Float, Int, Bool, Vec3, Enum, Str, Input
    from minihou.geometry import Geometry, Topology

    @node("twist", inputs=["geo"], category="deform", params=[
        Float("angle", 90.0, soft_min=-360, soft_max=360, doc="Degrees of twist over the height"),
        Vec3("axis_center", (0, 0, 0)),
    ])
    def twist(geo, angle, axis_center):
        \"\"\"Twist points around Z, proportional to height.\"\"\"
        out = geo.copy()                      # shallow copy; untouched arrays stay shared (cheap, not re-sent)
        P = geo.P.astype(np.float64)          # (n, 3) float32, read-only: never modify inputs in place
        ...
        out.point["P"] = P.astype(np.float32)
        return out

Rules
* Pure function of inputs and params (the cache depends on it): no time, files, global random state.
* One positional argument per input (in order); a list for Input("geos", multi=True); None for an unconnected
  Input("x", optional=True). One keyword argument per param. Return a new Geometry.
* Vectorise with numpy (no Python loops over points): graphs often carry 100k-1M points.
* Params: Float/Int(name, default, min=, max=, soft_min=, soft_max=, doc=), Bool, Vec3, Enum(name, choices), Str(name,
  default, kind="code"). Float/Int/Bool/Vec3 params can be exposed to Blender.
* Node names: lower_case, unique, not an existing type. Category is free text (shown in the Tab menu).

Geometry API
* geo.P -> (n, 3) float32 positions; geo.npoints, geo.nprims, geo.nvertices
* geo.point / geo.vertex / geo.prim: attribute tables, e.g. geo.point["N"] (n, 3), out.point["Cd"] = arr (float32/int32,
  shape (n,) or (n, k)); "name" in geo.point; geo.point.names(); geo.detail is a plain dict
* geo.topology.vertex_point (vertex -> point), geo.topology.prim_offsets (CSR: prim i owns vertices
  offsets[i]:offsets[i+1]); geo.topology.prim_sizes()
* New geometry: Geometry.from_arrays(P, vertex_point, prim_offsets) or Geometry.points_only(P);
  Topology.from_polys([[0, 1, 2, 3], ...]) for small cases; out.set_topology(t) resets vertex/prim tables
* Faces are counter-clockwise when seen from outside (normals point out). Z is up; units are Blender metres.
The built-in nodes in minihou/nodes/*.py are complete examples (transform.py and noise.py are short).
"""


@dataclass
class LibraryFile:
    path: Path
    stamp: Optional[tuple] = None
    code_hash: str = ""
    types: List[str] = field(default_factory=list)
    error: Optional[str] = None


class NodeLibrary:
    def __init__(self, dirs: Iterable, registry: Registry = REGISTRY):
        self.dirs = [Path(d).resolve() for d in dirs]
        self.registry = registry
        self.files: Dict[Path, LibraryFile] = {}

    # -- scanning ----------------------------------------------------------------------------------
    def scan(self) -> List[Path]:
        out = []
        for d in self.dirs:
            if d.is_dir():
                out.extend(sorted(p for p in d.glob("*.py") if not p.name.startswith(("_", "."))))
        return out

    def refresh(self) -> bool:
        """Load new or changed files, drop deleted ones. Returns True if node types or errors changed."""
        changed = False
        current = self.scan()
        for path in [p for p in self.files if p not in current]:
            for name in self.files[path].types:
                self.registry.unregister(name)
            del self.files[path]
            log.info("node library: removed %s", path.name)
            changed = True
        for path in current:
            try:
                st = path.stat()
                stamp = (st.st_mtime_ns, st.st_size)
            except OSError:
                continue
            entry = self.files.get(path)
            if entry is not None and entry.stamp == stamp:
                continue
            try:
                text = path.read_text(encoding="utf-8")
            except (OSError, UnicodeDecodeError) as e:
                entry = self.files.setdefault(path, LibraryFile(path))
                entry.stamp, entry.error = stamp, f"cannot read: {e}"
                changed = True
                continue
            h = hashlib.blake2b(text.encode("utf-8"), digest_size=8).hexdigest()
            if entry is not None and entry.code_hash == h:
                entry.stamp = stamp                     # touched, not changed
                continue
            self._load(path, text, h, stamp)
            changed = True
        return changed

    # -- loading -----------------------------------------------------------------------------------
    def _load(self, path: Path, text: str, h: str, stamp) -> None:
        entry = self.files.setdefault(path, LibraryFile(path))
        old = {n: self.registry.get(n) for n in entry.types if n in self.registry}
        for n in old:
            self.registry.unregister(n)
        src = str(path)
        with _LOAD_LOCK:
            sdk._LOADING = {"file": src, "hash": h}
            try:
                module = types.ModuleType(f"minihou_nodes_{path.stem}")
                module.__file__ = src
                exec(compile(text, src, "exec"), module.__dict__)          # noqa: S102 - user's own node library
                new = [nt.name for nt in self.registry if nt.source == src]
                error = None if new else "defines no @node functions"
            except Exception as e:  # noqa: BLE001 - report, keep the previous version
                for nt in [nt for nt in self.registry if nt.source == src]:
                    self.registry.unregister(nt.name)
                for nt in old.values():
                    self.registry.register(nt)
                new, error = list(old), _describe_error(e, src)
            finally:
                sdk._LOADING = None
        entry.stamp, entry.code_hash, entry.types, entry.error = stamp, h, new, error
        if error:
            log.warning("node library %s: %s", path.name, error)
        else:
            log.info("node library: loaded %s (%s)", path.name, ", ".join(new))

    def status(self) -> dict:
        return {"dirs": [str(d) for d in self.dirs],
                "files": [{"file": str(f.path), "nodes": list(f.types), "error": f.error}
                          for f in sorted(self.files.values(), key=lambda f: str(f.path))]}

    def unload(self) -> None:
        for entry in self.files.values():
            for name in entry.types:
                self.registry.unregister(name)
        self.files.clear()


def _describe_error(e: BaseException, src: str) -> str:
    if isinstance(e, SyntaxError) and e.lineno:
        return f"SyntaxError: {e.msg} (line {e.lineno})"
    line = next((fr.lineno for fr in reversed(traceback.extract_tb(e.__traceback__)) if fr.filename == src), None)
    return f"{type(e).__name__}: {e}" + (f" (line {line})" if line else "")
