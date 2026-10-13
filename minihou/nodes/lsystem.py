"""L-systems (Houdini's L-System SOP): string rewriting plus a 3-D turtle, producing branching curves.

Symbols (as in "The Algorithmic Beauty of Plants"):

* ``F`` move forward drawing, ``f`` move forward without drawing (both by the step size)
* ``+`` / ``-`` turn left / right, ``&`` / ``^`` pitch down / up, ``\\`` / ``/`` roll left / right (by the angle),
  ``|`` turn around
* ``[`` / ``]`` push / pop the turtle (a branch)
* ``!`` multiply the width by the width scale, ``"`` multiply the step by the step scale
* ``J`` place a leaf point (group ``leaves``, N along the heading)
* any other letter is a variable that only takes part in rewriting

Rules are ``A=replacement``, separated by newlines or ``;``. Several rules for one symbol may carry probabilities
(``F=0.6:F[+F]F``), chosen per symbol occurrence from the seed. Branches start new curves that share the branch
point, so the result is one connected network of curves with a ``width`` point attribute ready for ``sweep``.
"""
import numpy as np

from ..sdk import Bool, Float, Int, Str, Vec3, node
from . import _build as B

MAX_SYMBOLS = 2_000_000


def parse_rules(text: str):
    rules = {}
    for raw in text.replace(";", "\n").splitlines():
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        if "=" not in line:
            raise ValueError(f"rule {line!r} needs '=', e.g. F=F[+F]F")
        lhs, rhs = (x.strip() for x in line.split("=", 1))
        if len(lhs) != 1:
            raise ValueError(f"rule {line!r}: the left side must be a single symbol")
        p = 1.0
        if ":" in rhs:
            head, tail = rhs.split(":", 1)
            try:
                p = float(head)
                rhs = tail.strip()
            except ValueError:
                pass
        rules.setdefault(lhs, []).append((p, rhs))
    return rules


def rewrite(premise: str, rules, generations: int, seed: int) -> str:
    rng = np.random.default_rng(int(seed) & 0xFFFFFFFF)
    s = premise
    for _ in range(generations):
        out = []
        for ch in s:
            opts = rules.get(ch)
            if not opts:
                out.append(ch)
            elif len(opts) == 1:
                out.append(opts[0][1])
            else:
                w = np.array([p for p, _ in opts], np.float64)
                out.append(opts[int(rng.choice(len(opts), p=w / w.sum()))][1])
        s = "".join(out)
        if len(s) > MAX_SYMBOLS:
            raise ValueError(f"the string grew past {MAX_SYMBOLS:,} symbols; use fewer generations")
    return s


def _rot(a, b, ang):
    """Rotate the pair of frame vectors (a, b) by ang in their plane."""
    c, s = np.cos(ang), np.sin(ang)
    return a * c + b * s, b * c - a * s


@node("lsystem", category="source", params=[
    Str("premise", "F", doc="The starting string"),
    Str("rules", "F=FF-[-F+F+F]+[+F-F-F]", kind="code", doc="Rules 'A=...' separated by newlines or ';'"),
    Int("generations", 4, min=0, max=12),
    Float("angle", 22.5, soft_min=0.0, soft_max=90.0, doc="Degrees for + - & ^ \\ /"),
    Bool("separate_angles", False, doc="Use independent yaw, pitch and roll angles instead of Angle"),
    Float("yaw", 22.5, doc="Degrees for + (left) and - (right), in the turtle frame"),
    Float("pitch", 22.5, doc="Degrees for & (down) and ^ (up), in the turtle frame"),
    Float("roll", 137.5, doc="Degrees for backslash and /, around the current heading"),
    Vec3("initial_orientation", (0, 0, 0), doc="Initial turtle yaw, pitch, roll in degrees, applied in that order; zero starts along +Z"),
    Float("step", 0.1, min=0.0, soft_max=2.0, doc="Length of F and f"),
    Float("step_scale", 0.9, min=0.0, soft_max=2.0, doc='Factor applied by "'),
    Float("width", 0.05, min=0.0, soft_max=1.0, doc="Starting width (point attribute 'width', used by sweep)"),
    Float("width_scale", 0.7, min=0.0, max=1.0, doc="Factor applied by !, and by each [ branch"),
    Int("seed", 0, doc="For stochastic rules"),
])
def lsystem(premise, rules, generations, angle, step, step_scale, width, width_scale, seed, separate_angles=False, yaw=22.5, pitch=22.5, roll=137.5, initial_orientation=(0, 0, 0)):
    """Grow plants and fractals from rewriting rules (Houdini's L-System). Output: curves along the turtle's path
    (+Z is up) with a width attribute, and leaf points (group 'leaves') where J appears.
    + / - yaw, & / ^ pitch, backslash / slash roll in the moving turtle frame.
    Enable Separate Angles to control each axis independently. Branches restore the entire frame."""
    s = rewrite(premise, parse_rules(rules), generations, seed)
    yaw_angle, pitch_angle, roll_angle = np.radians((yaw, pitch, roll) if separate_angles else (angle, angle, angle))
    pos = np.zeros(3)
    H, L, U = np.array([0.0, 0, 1]), np.array([-1.0, 0, 0]), np.array([0.0, -1, 0])   # heading up (+Z)
    iy, ip, ir = np.radians(initial_orientation)
    H, L = _rot(H, L, iy)
    H, U = _rot(H, U, ip)
    L, U = _rot(L, U, ir)
    w, st = float(width), float(step)
    pts, widths, leaf = [pos.copy()], [w], [False]
    normals = [H.copy()]
    cur = 0                         # index of the turtle's current point
    curves, current = [], None      # lists of point indices
    stack = []
    for ch in s:
        if ch == "F":
            pos = pos + H * st
            pts.append(pos.copy())
            widths.append(w)
            leaf.append(False)
            normals.append(H.copy())
            if current is None:
                current = [cur]
                curves.append(current)
            current.append(len(pts) - 1)
            cur = len(pts) - 1
        elif ch == "f":
            pos = pos + H * st
            pts.append(pos.copy())
            widths.append(w)
            leaf.append(False)
            normals.append(H.copy())
            cur = len(pts) - 1
            current = None
        elif ch in "+-":
            H, L = _rot(H, L, yaw_angle if ch == "+" else -yaw_angle)
        elif ch in "&^":
            H, U = _rot(H, U, pitch_angle if ch == "&" else -pitch_angle)
        elif ch in "\\/":
            L, U = _rot(L, U, roll_angle if ch == "\\" else -roll_angle)
        elif ch == "|":
            H, L = -H, -L
        elif ch == "[":
            stack.append((pos.copy(), H, L, U, w, st, cur, current))
            current = None                                    # a branch starts a new curve at this point
            w *= width_scale
        elif ch == "]":
            if not stack:
                raise ValueError("']' without a matching '['")
            pos, H, L, U, w, st, cur, current = stack.pop()
            current = None                                    # continuing after a branch starts a new curve
        elif ch == "!":
            w *= width_scale
        elif ch == '"':
            st *= step_scale
        elif ch == "J":
            pts.append(pos.copy())
            widths.append(w)
            leaf.append(True)
            normals.append(H.copy())
    if stack:
        raise ValueError("'[' without a matching ']'")
    curves = [c for c in curves if len(c) >= 2]
    used = np.zeros(len(pts), bool)
    for c in curves:
        used[c] = True
    keep = used | np.asarray(leaf)                           # drop points nothing refers to (e.g. after f)
    remap = np.cumsum(keep) - 1
    P = np.asarray(pts, np.float32)[keep]
    vp = np.concatenate([remap[np.asarray(c)] for c in curves]) if curves else np.zeros(0, np.int64)
    out = B.make(P, vp, B.offsets_from_sizes([len(c) for c in curves]), closed=np.zeros(len(curves), bool))
    out.point["width"] = np.asarray(widths, np.float32)[keep]
    out.point["N"] = np.asarray(normals, np.float32)[keep]
    out.point["group_leaves"] = np.asarray(leaf, np.int32)[keep]
    out.detail["lsystem_symbols"] = len(s)
    return out
