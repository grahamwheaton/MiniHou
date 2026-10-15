"""Parameter-boundary smoke test over MiniHou node types.

Tests every geometry-in/geometry-out node type in minihou.sdk.REGISTRY against
boundary parameter values on both a normal source geometry (unit box, grid)
and an empty geometry.
"""
from __future__ import annotations

import time
import unittest
from typing import Any, Dict, List, Optional, Sequence, Tuple

import numpy as np

import minihou  # noqa: F401 - registers all built-in node types
from minihou import Geometry
from minihou.boolean import BooleanError
from minihou.edit import EditError
from minihou.evaluator import GraphError, NodeError
from minihou.groups import GroupError
from minihou.sdk import REGISTRY, NodeType, Param, ParamError
from minihou.wrangle import WrangleError

# ---------------------------------------------------------------------------
# Module-level KNOWN dict
# ---------------------------------------------------------------------------
# Mapping of (node_type, param_name, repr(value)) to a short reason and file:line
# for confirmed bugs; entries are reported but do not fail the test.
KNOWN: Dict[Tuple[str, str, str], str] = {
}

# ---------------------------------------------------------------------------
# Skip rules (reused from sibling test)
# ---------------------------------------------------------------------------
SKIPPED: Dict[str, str] = {
    # Python code node
    "python": "Python code node; executes arbitrary script code",
    # Outside world / File & Image I/O
    "image_file": "reads image files from disk",
    "image_out": "writes image files to disk",
    "usd_import": "reads USD files from disk",
    "usd_export": "writes USD files to disk",
    "blender_image": "reads Blender image data from outside world",
    # Outside world / Blender input & scene objects
    "blender_object": "reads Blender scene objects from outside world",
    "modifier_input": "reads Blender modifier stack from outside world",
    "object_input": "reads Blender object picker from outside world",
    "scene_objects": "publishes Blender scene objects to outside world",
    # RBD / solver / physics
    "solver": "frame-by-frame simulation solver (category time)",
    "rbd_configure": "RBD physics configuration node (category physics)",
    "rbd_solver": "RBD physics solver (category physics)",
    "rbd_constraint": "RBD physics constraint source (category physics)",
    "rbd_constraint_properties": "RBD physics constraint properties (category physics)",
    "rbd_material_fracture": "RBD physics fracture node (category physics)",
    "goal_length": "physics goal length constraint node (category physics)",
    "goal_collision": "physics goal collision constraint node (category physics)",
    "goal_surface": "physics goal surface constraint node (category physics)",
    "goal_gravity": "physics goal gravity constraint node (category physics)",
    "goal_pin": "physics goal pin constraint node (category physics)",
    "goal_solve": "physics goal solver node (category physics)",
    # Control flow nodes
    "loop_begin": "loop control node (category flow; requires paired loop_end)",
    "loop_end": "loop control node (category flow; requires paired loop_begin)",
    "for_each": "for-each control node (category combine; requires graph body)",
    # System boundary nodes
    "input": "system input node (category system; requires enclosing graph)",
}

# ---------------------------------------------------------------------------
# Source geometry builders
# ---------------------------------------------------------------------------
def build_test_sources() -> Dict[str, Tuple[Geometry, Geometry]]:
    """Construct (primary, secondary) source geometries for normal and empty tests."""
    # 1. Closed unit box (8 points, 6 quads) built as in sibling test
    box_p = np.array([
        [-0.5, -0.5, -0.5],
        [ 0.5, -0.5, -0.5],
        [ 0.5,  0.5, -0.5],
        [-0.5,  0.5, -0.5],
        [-0.5, -0.5,  0.5],
        [ 0.5, -0.5,  0.5],
        [ 0.5,  0.5,  0.5],
        [-0.5,  0.5,  0.5],
    ], dtype=np.float32)
    box_quads = np.array([
        [0, 3, 2, 1],  # -Z
        [4, 5, 6, 7],  # +Z
        [0, 1, 5, 4],  # -Y
        [2, 3, 7, 6],  # +Y
        [0, 4, 7, 3],  # -X
        [1, 2, 6, 5],  # +X
    ], dtype=np.int32)
    unit_box = Geometry.from_arrays(
        box_p,
        vertex_point=box_quads.ravel(),
        prim_offsets=np.arange(7, dtype=np.int32) * 4,
    )
    unit_box.validate()
    unit_box.freeze()

    # 2. Planar quad grid for nodes needing a second input
    grid_p = np.array([
        [-0.5, -0.5, 0.0],
        [ 0.5, -0.5, 0.0],
        [ 0.5,  0.5, 0.0],
        [-0.5,  0.5, 0.0],
    ], dtype=np.float32)
    grid_quad = np.array([0, 1, 2, 3], dtype=np.int32)
    grid = Geometry.from_arrays(
        grid_p,
        vertex_point=grid_quad,
        prim_offsets=np.array([0, 4], dtype=np.int32),
    )
    grid.validate()
    grid.freeze()

    # 3. Empty geometry (0 points, 0 primitives)
    empty = Geometry.from_arrays(np.zeros((0, 3), dtype=np.float32))
    empty.validate()
    empty.freeze()

    return {
        "normal": (unit_box, grid),
        "empty": (empty, empty),
    }


def build_node_args(nt: NodeType, primary: Geometry, secondary: Geometry) -> List[object]:
    """Build positional arguments for the node function.

    - Optional inputs receive None.
    - Multi-inputs receive a list with the appropriate source geometry.
    - Required primary input receives primary (unit box or empty).
    - Required second input receives secondary (grid or empty).
    - Any subsequent required inputs receive primary.
    """
    args: List[object] = []
    for idx, inp in enumerate(nt.inputs):
        if inp.optional:
            args.append(None)
        elif inp.multi:
            args.append([primary if idx == 0 else secondary])
        else:
            args.append(primary if idx == 0 else secondary)
    return args


# ---------------------------------------------------------------------------
# Parameter boundary value generator
# ---------------------------------------------------------------------------
# Keywords indicating parameters that scale memory or work with count/density;
# for these we cap large test values to 1000 to keep the suite fast (<90s).
COUNT_KEYWORDS = {
    "count", "div", "seg", "iter", "res", "point", "col", "row", "layer",
    "step", "sample", "piece", "cluster", "level", "loop", "octave", "storey",
    "substep", "ray", "blur", "depth", "copy", "copies", "side", "smooth",
    "search", "relax", "droplet", "size", "width", "height", "target",
    "generations", "m2", "resolution", "subdivisions",
}


def is_count_param(p: Param) -> bool:
    name = p.name.lower()
    if p.proxy in ("count", "res2", "step3", "step1"):
        return True
    return any(k in name for k in COUNT_KEYWORDS)


def get_boundary_values(node_name: str, p: Param) -> List[Any]:
    """Generate boundary values appropriate for parameter kind, capped for speed."""
    vals: List[Any] = [p.default]

    # lsystem generations parameter grows exponentially (7^N); cap to 5.0 to stay within 90s,
    # and include 13.0 to verify clamp/ParamError rejection.
    if node_name == "lsystem" and p.name == "generations":
        return [0.0, 1.0, 4.0, 5.0, 13.0]

    k = p.kind
    if k == "bool":
        vals.extend([True, False])
    elif k == "enum":
        vals.extend(p.choices)
    elif k == "str":
        vals.extend(["", " ", "x" * 1000, "__nonexistent_attr_or_group__"])
    elif k == "vec3":
        vals.extend([
            (0.0, 0.0, 0.0),
            (1.0, 1.0, 1.0),
            (-1.0, -1.0, -1.0),
            (1e6, 1e6, 1e6),
        ])
    elif k == "int":
        large = 1000 if is_count_param(p) else 1_000_000
        vals.extend([0, 1, -1, large])
        if p.min is not None:
            m = int(p.min)
            vals.extend([m, m - 1, m + 1])
        if p.max is not None:
            m = int(p.max)
            vals.extend([m, m + 1, m - 1])
    elif k == "float":
        large = 1000.0 if is_count_param(p) else 1e6
        vals.extend([0.0, 1.0, -1.0, 1e-9, large])
        if p.min is not None:
            m = float(p.min)
            vals.extend([m, m - 1.0, m + 1.0])
        if p.max is not None:
            m = float(p.max)
            vals.extend([m, m + 1.0, m - 1.0])

    # Deduplicate while preserving insertion order
    dedup: List[Any] = []
    seen = set()
    for v in vals:
        rep = repr(v)
        if rep not in seen:
            seen.add(rep)
            dedup.append(v)
    return dedup


# ---------------------------------------------------------------------------
# Evaluation and failure classification
# ---------------------------------------------------------------------------
RAW_IMPLEMENTATION_ERRORS = (
    IndexError,
    KeyError,
    AttributeError,
    TypeError,
    ZeroDivisionError,
    FloatingPointError,
    AssertionError,
    RecursionError,
    MemoryError,
)

DELIBERATE_ENGINE_ERRORS = (
    ValueError,
    NodeError,
    GraphError,
    ParamError,
    GroupError,
    WrangleError,
    BooleanError,
    EditError,
)

NUMPY_BROADCASTING_PATTERNS = (
    "could not be broadcast",
    "operands could not be broadcast together",
    "shape mismatch",
    "cannot reshape array",
    "zero-size array to reduction operation",
)


def evaluate_node_param(
    nt: NodeType, args: List[object], param_name: str, value: Any
) -> Optional[str]:
    """Evaluate nt.fn with param_name overridden to value.

    Returns None on acceptable outcome, or failure description string.
    """
    t0 = time.perf_counter()
    try:
        params = nt.coerce_params({param_name: value})
        out = nt.fn(*args, **params)
        elapsed = time.perf_counter() - t0
    except Exception as exc:
        elapsed = time.perf_counter() - t0
        exc_type = type(exc).__name__
        msg = str(exc)[:120]

        if elapsed > 10.0:
            return f"Timeout ({elapsed:.2f}s > 10.0s) and {exc_type}: {msg}"

        if isinstance(exc, RAW_IMPLEMENTATION_ERRORS):
            return f"{exc_type}: {msg}"

        if isinstance(exc, ValueError):
            msg_lower = msg.lower()
            if any(pat in msg_lower for pat in NUMPY_BROADCASTING_PATTERNS):
                return f"{exc_type} (numpy broadcasting): {msg}"
            # Deliberate engine ValueError
            return None

        if isinstance(exc, DELIBERATE_ENGINE_ERRORS):
            return None

        # Any other unhandled error
        return f"{exc_type}: {msg}"

    if elapsed > 10.0:
        return f"Timeout: call took {elapsed:.2f}s (> 10.0s)"

    if not isinstance(out, Geometry):
        return f"Returned {type(out).__name__}, expected Geometry"

    if out.npoints > 20_000_000:
        return f"unbounded: point count {out.npoints} > 20,000,000"

    try:
        out.validate()
    except Exception as val_exc:
        val_msg = str(val_exc)[:120]
        return f"validate() failed: {type(val_exc).__name__}: {val_msg}"

    if out.npoints > 0:
        P = out.point["P"]
        if not np.all(np.isfinite(P)):
            return "P contains NaN or Inf"

    return None


def format_failure_table(failures: List[Tuple[str, str, str, str]]) -> str:
    """Format failures into an aligned table truncated to 140 chars per row."""
    if not failures:
        return ""
    col1_w = max(len("node"), max(len(f[0]) for f in failures))
    col2_w = max(len("param=value"), max(len(f[1]) for f in failures))
    col3_w = max(len("source"), max(len(f[2]) for f in failures))
    header = f"{'node':<{col1_w}} | {'param=value':<{col2_w}} | {'source':<{col3_w}} | failure"
    divider = f"{'-' * col1_w}-+-{'-' * col2_w}-+-{'-' * col3_w}-+-{'-' * 40}"
    rows = [header, divider]
    for node_name, pv, src, err in failures:
        row = f"{node_name:<{col1_w}} | {pv:<{col2_w}} | {src:<{col3_w}} | {err}"
        if len(row) > 140:
            row = row[:137] + "..."
        rows.append(row)
    return "\n".join(rows)


# ---------------------------------------------------------------------------
# Test Case
# ---------------------------------------------------------------------------
class TestNodesParamBoundaries(unittest.TestCase):
    """Parameter-boundary smoke test exercising all registered node types."""

    def test_nodes_param_boundaries(self):
        sources = build_test_sources()
        failures: List[Tuple[str, str, str, str]] = []
        known_reported: List[Tuple[str, str, str, str, str]] = []

        for node_name in sorted(REGISTRY.names()):
            if node_name in SKIPPED:
                continue

            nt = REGISTRY.get(node_name)
            for p in nt.params:
                bvals = get_boundary_values(node_name, p)
                for val in bvals:
                    val_repr = repr(val)
                    for src_name, (prim, sec) in sources.items():
                        args = build_node_args(nt, prim, sec)
                        with self.subTest(node=node_name, param=p.name, value=val):
                            err = evaluate_node_param(nt, args, p.name, val)
                            if err is not None:
                                key = (node_name, p.name, val_repr)
                                pv_str = f"{p.name}={val_repr}"
                                if key in KNOWN:
                                    known_reported.append(
                                        (node_name, pv_str, src_name, err, KNOWN[key])
                                    )
                                else:
                                    failures.append((node_name, pv_str, src_name, err))
                                    self.fail(err)

        if known_reported:
            print(f"\n[INFO] {len(known_reported)} confirmed bug(s) in KNOWN reported:")
            for n, pv, s, err, reason in known_reported:
                print(f"  {n} | {pv} | {s} | {reason} ({err})")

        if failures:
            table = format_failure_table(failures)
            self.fail(
                f"\nParameter boundary test failed for {len(failures)} call(s):\n\n"
                f"{table}\n"
            )


if __name__ == "__main__":
    unittest.main()
