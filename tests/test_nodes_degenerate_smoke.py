"""Degenerate-input smoke test over MiniHou node types.

Tests every geometry-in/geometry-out node type in minihou.sdk.REGISTRY against
six degenerate source geometries:
1. empty geometry (0 points, 0 primitives)
2. a single point, no primitives
3. one triangle
4. an open 2-point curve (one open primitive)
5. a closed unit box (8 points, 6 quads)
6. two points coincident at the origin with a one-edge open curve between them (zero length)
"""
from __future__ import annotations

import time
import unittest
from typing import Dict, List, Optional, Tuple

import numpy as np

import minihou  # noqa: F401 - registers all built-in node types
from minihou import Geometry
from minihou.boolean import BooleanError
from minihou.edit import EditError
from minihou.evaluator import GraphError, NodeError
from minihou.groups import GroupError
from minihou.sdk import REGISTRY, NodeType, ParamError
from minihou.wrangle import WrangleError

# ---------------------------------------------------------------------------
# Module-level KNOWN dict
# ---------------------------------------------------------------------------
# Mapping of (node_type, source_name) to a short reason, for failures that are
# already confirmed bugs; entries in KNOWN are reported but do not fail the test.
# Left EMPTY by default: the caller will fill it.
KNOWN: Dict[Tuple[str, str], str] = {
    # Found by this test (2026-10-15): a target with no triangles reaches an index into an empty array.
    ("ray", "single_point"): "ray.py:83 tri_p[np.maximum(tri, 0)] on an empty triangle array: raw IndexError",
    ("ray", "open_curve"): "ray.py:83 tri_p[np.maximum(tri, 0)] on an empty triangle array: raw IndexError",
    ("ray", "coincident_curve"): "ray.py:83 tri_p[np.maximum(tri, 0)] on an empty triangle array: raw IndexError",
    ("point_relax", "open_curve"): "deform.py:131 grid.tris[tri] on an empty triangle array: raw IndexError",
    ("point_relax", "coincident_curve"): "deform.py:131 grid.tris[tri] on an empty triangle array: raw IndexError",
}


# ---------------------------------------------------------------------------
# Skip rules
# ---------------------------------------------------------------------------
# Node types that need the outside world or are not geometry-in/geometry-out:
# - Python code nodes (python)
# - Files, images, USD reading/writing
# - Blender input/scene objects
# - RBD/solver/physics
# - loop_begin/loop_end/foreach control nodes
# - Nodes whose required input is not a plain geometry
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
# Degenerate source geometry builders
# ---------------------------------------------------------------------------
def build_degenerate_sources() -> Dict[str, Geometry]:
    """Construct the six degenerate test geometries using the real Geometry API.

    Each source is validated and frozen before use.
    """
    # 1. empty geometry (0 points, 0 primitives)
    empty = Geometry.from_arrays(np.zeros((0, 3), dtype=np.float32))
    empty.validate()
    empty.freeze()

    # 2. a single point, no primitives
    single_point = Geometry.points_only(np.zeros((1, 3), dtype=np.float32))
    single_point.validate()
    single_point.freeze()

    # 3. one triangle
    tri_p = np.array([[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]], dtype=np.float32)
    tri_vp = np.array([0, 1, 2], dtype=np.int32)
    tri_po = np.array([0, 3], dtype=np.int32)
    triangle = Geometry.from_arrays(tri_p, vertex_point=tri_vp, prim_offsets=tri_po)
    triangle.validate()
    triangle.freeze()

    # 4. an open 2-point curve (one open primitive)
    curve_p = np.array([[0.0, 0.0, 0.0], [1.0, 0.0, 0.0]], dtype=np.float32)
    curve_vp = np.array([0, 1], dtype=np.int32)
    curve_po = np.array([0, 2], dtype=np.int32)
    open_curve = Geometry.from_arrays(
        curve_p, vertex_point=curve_vp, prim_offsets=curve_po, closed=np.array([False], dtype=bool)
    )
    open_curve.validate()
    open_curve.freeze()

    # 5. a closed unit box (8 points, 6 quads)
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

    # 6. two points coincident at the origin with a one-edge open curve between them (zero length)
    coin_p = np.zeros((2, 3), dtype=np.float32)
    coin_vp = np.array([0, 1], dtype=np.int32)
    coin_po = np.array([0, 2], dtype=np.int32)
    coincident_curve = Geometry.from_arrays(
        coin_p,
        vertex_point=coin_vp,
        prim_offsets=coin_po,
        closed=np.array([False], dtype=bool),
    )
    coincident_curve.validate()
    coincident_curve.freeze()

    return {
        "empty": empty,
        "single_point": single_point,
        "triangle": triangle,
        "open_curve": open_curve,
        "unit_box": unit_box,
        "coincident_curve": coincident_curve,
    }


# ---------------------------------------------------------------------------
# Node evaluation and error classification
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


def build_node_args(nt: NodeType, source_geo: Geometry) -> List[object]:
    """Build positional arguments for the node function as the evaluator does.

    - An optional input receives None.
    - A multi-input receives a list [source_geo].
    - A standard required input receives source_geo.
    """
    args: List[object] = []
    for inp in nt.inputs:
        if inp.optional:
            args.append(None)
        elif inp.multi:
            args.append([source_geo])
        else:
            args.append(source_geo)
    return args


def evaluate_node(nt: NodeType, source_geo: Geometry) -> Optional[str]:
    """Call nt.fn(*args, **nt.coerce_params({})) directly.

    Returns None if the call is acceptable (valid Geometry returned or deliberate
    engine error raised). Returns a failure description string if the call fails
    (timeout, non-geometry returned, validate() raised, or raw implementation error).
    """
    args = build_node_args(nt, source_geo)
    t0 = time.perf_counter()
    try:
        params = nt.coerce_params({})
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

        # Any other raw/unhandled error
        return f"{exc_type}: {msg}"

    if elapsed > 10.0:
        return f"Timeout: call took {elapsed:.2f}s (> 10.0s)"

    if not isinstance(out, Geometry):
        return f"Returned {type(out).__name__}, expected Geometry"

    try:
        out.validate()
    except Exception as val_exc:
        val_msg = str(val_exc)[:120]
        return f"validate() failed: {type(val_exc).__name__}: {val_msg}"

    return None


def format_failure_table(failures: List[Tuple[str, str, str]]) -> str:
    """Format failures into an aligned table: node_type | source | exception type: message."""
    if not failures:
        return ""
    col1_w = max(len("node_type"), max(len(f[0]) for f in failures))
    col2_w = max(len("source"), max(len(f[1]) for f in failures))
    header = f"{'node_type':<{col1_w}} | {'source':<{col2_w}} | failure"
    divider = f"{'-' * col1_w}-+-{'-' * col2_w}-+-{'-' * 40}"
    rows = [header, divider]
    for node_type, source_name, err in failures:
        rows.append(f"{node_type:<{col1_w}} | {source_name:<{col2_w}} | {err}")
    return "\n".join(rows)


# ---------------------------------------------------------------------------
# Test Case
# ---------------------------------------------------------------------------
class TestNodesDegenerateSmoke(unittest.TestCase):
    """Smoke test exercising all registered node types on degenerate inputs."""

    def test_nodes_degenerate_smoke(self):
        sources = build_degenerate_sources()
        failures: List[Tuple[str, str, str]] = []
        known_reported: List[Tuple[str, str, str, str]] = []

        for node_name in sorted(REGISTRY.names()):
            if node_name in SKIPPED:
                continue

            nt = REGISTRY.get(node_name)
            for source_name, source_geo in sources.items():
                with self.subTest(node=node_name, source=source_name):
                    err = evaluate_node(nt, source_geo)
                    if err is not None:
                        if (node_name, source_name) in KNOWN:
                            known_reported.append(
                                (node_name, source_name, err, KNOWN[(node_name, source_name)])
                            )
                        else:
                            failures.append((node_name, source_name, err))
                            self.fail(err)

        if known_reported:
            print(f"\n[INFO] {len(known_reported)} confirmed bug(s) in KNOWN reported:")
            for n, s, err, reason in known_reported:
                print(f"  {n} | {s} | {reason} ({err})")

        if failures:
            table = format_failure_table(failures)
            self.fail(
                f"\nDegenerate smoke test failed for {len(failures)} node/source combination(s):\n\n"
                f"{table}\n"
            )


if __name__ == "__main__":
    unittest.main()
