"""L-systems (Houdini's L-System SOP): string rewriting plus a 3-D turtle, producing branching curves or tubes.

Turtle symbols (as in "The Algorithmic Beauty of Plants" and Houdini's L-System SOP). Most take an optional argument in
brackets that overrides the node's value, e.g. ``F(0.5)``, ``+(30)``, ``!(0.8)``:

* ``F(l,w,s,d)`` move forward drawing; ``H`` the same for half the length; ``f`` / ``h`` move without drawing; ``G`` moves without
  recording a vertex. ``l`` length, ``w`` width, ``s`` cross sections (columns) and ``d`` divisions (rows) of a tube
* ``+`` / ``-`` turn left / right, ``&`` / ``^`` pitch down / up, ``\\`` / ``/`` roll left / right (by the angle), ``|`` turn around,
  ``*`` roll 180 degrees, ``~(a)`` turn, pitch and roll by a random amount up to ``a`` degrees (default 180)
* ``[`` / ``]`` push / pop the turtle (a branch), ``%`` cut off the rest of the branch
* ``!`` / ``?`` multiply / divide the width, ``"`` / ``_`` multiply / divide the step, ``;`` / ``@`` multiply / divide the angle
* ``T(g)`` tropism towards -Z (gravity), ``$(x,y,z)`` orient the turtle (and point it at x, y, z when given)
* ``'`` / ``#`` increment the colour map's U / V, ``g(i)`` start a primitive group, ``a(name,v1,v2,v3)`` set a point attribute
* ``{`` ``.`` ``}`` build a polygon from the points marked with ``.``
* ``J`` / ``K`` / ``M`` ``(s,x,a,b,c)`` stamp the geometry of input 1 / 2 / 3 (leaf J, K, M) at the turtle, scaled by ``s`` and
  stamped with ``a``, ``b``, ``c``. Without a connected input, ``J`` places a leaf point (group ``leaves``) as before
* any other letter is a variable that only takes part in rewriting

Rules are ``[left<]symbol[(vars)][>right][:condition]=replacement[:probability]`` (``->`` works like ``=``), separated by
newlines. A ``;`` also separates rules on one line when what follows is itself a rule (``A=AB;B=A``); use newlines when ``;`` is
the angle-scale command. Several rules for one symbol may carry probabilities (``F=0.6:F[+F]F`` or ``F=F[+F]F:0.6``), chosen per
symbol occurrence from the seed. Symbols carry up to any number of numeric variables: ``A(i,j):i<5=A(i+1,j-1)F(i)``. Expressions
know ``+ - * / %``, comparisons, ``&&``, ``||``, ``!``, and the functions and variables Houdini documents (``sin cos asin acos min max
in pic rand chan``, ``a b c d g i t x y z A L T U V W``).

Branches start new curves that share the branch point, so the result is one connected network of curves with a ``width`` point
attribute ready for ``sweep`` (or set Type to Tube to get polygon tubes directly).
"""
import ast
import math
import os
import re
from types import SimpleNamespace

import numpy as np

from .. import paths
from ..actions import action
from ..sdk import REGISTRY, Action, Bool, Enum, Float, Input, Int, Str, Vec3, node
from . import _build as B

MAX_SYMBOLS = 2_000_000
MAX_LEAF_POINTS = 5_000_000
MAX_TUBE_POINTS = 10_000_000
RESERVED = frozenset("abcdgitxyzALTUVW")      # expression variables a user variable may not replace
_IDENT = re.compile(r"[A-Za-z_]\w*\Z")


# ---------------------------------------------------------------------------------------------------------------------
# Expressions (conditions and symbol arguments)
# ---------------------------------------------------------------------------------------------------------------------
_FUNC_NAMES = frozenset({"acos", "asin", "cos", "sin", "tan", "min", "max", "abs", "sqrt", "floor", "ceil", "pow", "exp",
                         "log", "chan", "inside", "pic", "rand"})
_BINOPS = (ast.Add, ast.Sub, ast.Mult, ast.Div, ast.Mod, ast.FloorDiv)
_CMPOPS = (ast.Eq, ast.NotEq, ast.Lt, ast.LtE, ast.Gt, ast.GtE)
_SAFE_NODES = (ast.Expression, ast.BinOp, ast.UnaryOp, ast.BoolOp, ast.Compare, ast.Name, ast.Load, ast.And, ast.Or, ast.Not,
               ast.USub, ast.UAdd) + _BINOPS + _CMPOPS
_code_cache = {}


def _compile(expr: str):
    """Compile a rule expression after turning Houdini's spelling (``in(``, ``&&``, ``||``, ``!``, ``=``) into Python's."""
    code = _code_cache.get(expr)
    if code is None:
        src = re.sub(r"\bin\s*\(", "inside(", expr.strip())
        src = src.replace("&&", " and ").replace("||", " or ")
        src = re.sub(r"!(?!=)", " not ", src)
        src = re.sub(r"(?<![<>=!])=(?!=)", "==", src).strip()
        try:
            tree = ast.parse(src, mode="eval")
        except SyntaxError:
            raise ValueError(f"cannot read the expression {expr!r}") from None
        for n in ast.walk(tree):
            ok = (isinstance(n, _SAFE_NODES)
                  or (isinstance(n, ast.Constant) and isinstance(n.value, (int, float)) and not isinstance(n.value, complex))
                  or (isinstance(n, ast.Call) and isinstance(n.func, ast.Name) and n.func.id in _FUNC_NAMES and not n.keywords))
            if not ok:
                raise ValueError(f"expression {expr!r} uses something rules do not support")
        code = _code_cache[expr] = compile(tree, "<lsystem>", "eval")
    return code


def _ev(expr: str, env: dict, g: dict):
    try:
        return eval(_compile(expr), g, env)                       # noqa: S307 - the tree was checked against a whitelist
    except NameError as e:
        raise ValueError(f"expression {expr!r}: unknown variable {e.name!r}") from None
    except (ZeroDivisionError, OverflowError, ValueError, TypeError) as e:
        raise ValueError(f"expression {expr!r}: {e}") from None


def _arg(text: str, env: dict, g: dict):
    """A symbol argument: a quoted string (attribute names) or a number."""
    if len(text) >= 2 and text[0] == '"' and text[-1] == '"':
        return text[1:-1]
    return float(_ev(text, env, g))


# ---------------------------------------------------------------------------------------------------------------------
# Rules
# ---------------------------------------------------------------------------------------------------------------------
def _close(s: str, start: int, quoted: bool) -> int:
    depth, inq = 0, False
    for j in range(start, len(s)):
        c = s[j]
        if quoted and c == '"':
            inq = not inq
        elif inq:
            continue
        elif c == "(":
            depth += 1
        elif c == ")":
            depth -= 1
            if depth == 0:
                return j
    raise ValueError(f"unbalanced '(' in {s!r}")


def _split_args(text: str, quoted: bool):
    out, depth, inq, cur = [], 0, False, []
    for c in text:
        if quoted and c == '"':
            inq = not inq
        if not inq and c == "(":
            depth += 1
        elif not inq and c == ")":
            depth -= 1
        if c == "," and depth == 0 and not inq:
            out.append("".join(cur).strip())
            cur = []
        else:
            cur.append(c)
    last = "".join(cur).strip()
    if last or out:
        out.append(last)
    return out


def _tokenize(s: str):
    """``F(1,2)A`` -> [("F", ["1", "2"]), ("A", None)]: symbols with the text of their bracketed arguments."""
    out, i, n = [], 0, len(s)
    while i < n:
        ch = s[i]
        if ch.isspace():
            i += 1
        elif i + 1 < n and s[i + 1] == "(" and ch not in "()":
            j = _close(s, i + 1, ch == "a")
            out.append((ch, _split_args(s[i + 2:j], ch == "a")))
            i = j + 1
        else:
            out.append((ch, None))
            i += 1
    return out


class Rule:
    __slots__ = ("sym", "left", "right", "vars", "cond", "p", "rhs", "tmpl", "plain", "text")

    def __init__(self, **kw):
        for k, v in kw.items():
            setattr(self, k, v)


_SEMI = re.compile(r";(?=\s*[^\s;()=<>:]\s*(?:\([^)]*\))?\s*(?:=|->|:[^=;]*=))")
_HEAD = re.compile(r"(?:(?P<left>[^<>]+?)\s*<\s*)?(?P<sym>[^<>(\s])\s*(?:\((?P<vars>[^)]*)\))?\s*(?:>\s*(?P<right>[^<>]+))?\Z")


def _split_rule(line: str):
    depth, pos, ln = 0, -1, 0
    for i, ch in enumerate(line):
        if ch == "(":
            depth += 1
        elif ch == ")":
            depth -= 1
        elif depth == 0:
            if line.startswith("->", i):
                pos, ln = i, 2
            elif ch == "=" and (i == 0 or line[i - 1] not in "<>!=") and line[i + 1:i + 2] != "=":
                pos, ln = i, 1
    if pos < 0:
        raise ValueError(f"rule {line!r} needs '=', e.g. F=F[+F]F")
    return line[:pos].strip(), line[pos + ln:].strip()


def _parse_rule(line: str) -> Rule:
    lhs, rhs = _split_rule(line)
    depth, cut = 0, -1
    for i, ch in enumerate(lhs):
        depth += ch == "("
        depth -= ch == ")"
        if ch == ":" and depth == 0:
            cut = i
            break
    head, cond = (lhs[:cut], lhs[cut + 1:].strip() or None) if cut >= 0 else (lhs, None)
    m = _HEAD.match(head.strip())
    if not m:
        raise ValueError(f"rule {line!r}: the left side must be a single symbol, optionally with context (A<B>C), variables "
                         "(B(i,j)) and a condition (:i<5)")
    names = [v.strip() for v in (m.group("vars") or "").split(",") if v.strip()]
    for v in names:
        if not _IDENT.match(v):
            raise ValueError(f"rule {line!r}: {v!r} is not a variable name")

    def ctx_syms(text):
        return list(re.sub(r"\([^)]*\)|\s+", "", text)) if text else []

    p = 1.0
    if ":" in rhs:
        first, rest = rhs.split(":", 1)
        body, _, last = rhs.rpartition(":")
        try:
            p, rhs = float(first), rest.strip()
        except ValueError:
            try:
                p, rhs = float(last), body.strip()
            except ValueError:
                pass
    if p < 0:
        raise ValueError(f"rule {line!r}: a probability cannot be negative")
    if p > 1:
        p /= 100.0                                           # Houdini's example writes ': 80' for 80 %
    if cond is not None:
        _compile(cond)
    tmpl = _tokenize(rhs)
    for _s, args in tmpl:
        for a in args or ():
            if not (len(a) >= 2 and a[0] == '"' and a[-1] == '"'):
                _compile(a)
    left, right = ctx_syms(m.group("left")), ctx_syms(m.group("right"))
    return Rule(sym=m.group("sym"), left=left, right=right, vars=tuple(names), cond=cond, p=p,
                rhs=re.sub(r"\s+", "", rhs), tmpl=tmpl, text=line,
                plain=not (left or right or names or cond or "(" in rhs))


def parse_rules(text: str):
    """``{symbol: [Rule, ...]}`` from rule lines (see the module docs for the syntax)."""
    rules = {}
    for chunk in text.splitlines():
        for raw in _SEMI.split(chunk):
            line = raw.strip()
            if not line or line.startswith("#"):
                continue
            r = _parse_rule(line)
            rules.setdefault(r.sym, []).append(r)
    return rules


# ---------------------------------------------------------------------------------------------------------------------
# Turtle
# ---------------------------------------------------------------------------------------------------------------------
def _rot(a, b, ang):
    """Rotate the pair of frame vectors (a, b) by ang in their plane."""
    c, s = np.cos(ang), np.sin(ang)
    return a * c + b * s, b * c - a * s


def _rodrigues(v, axis, ang):
    c, s = math.cos(ang), math.sin(ang)
    return v * c + np.cross(axis, v) * s + axis * np.dot(axis, v) * (1 - c)


def _a(args, k, default):
    if args is not None and len(args) > k and not isinstance(args[k], str):
        return args[k]
    return default


_UP_REF = np.array([0.0, -1.0, 0.0])          # the turtle's initial "up" (U), which `$` re-levels the roll against
_MOVES = "FHGfh"


class Turtle:
    """Interprets tokens ``(symbol, args, born)``. With ``record`` False it only tracks position and state (used while rewriting
    so rules can read x, y, z, A, L, W, U, V); with True it also builds points, curves, polygons, tube segments and leaves."""

    def __init__(self, cfg, rng, record=True, grow=1.0, newest=-1):
        self.cfg, self.rng, self.rec, self.grow, self.newest = cfg, rng, record, grow, newest
        self.pos = np.array(cfg.start, np.float64)
        H, L, U = np.array([0.0, 0, 1]), np.array([-1.0, 0, 0]), np.array([0.0, -1, 0])
        iy, ip, ir = cfg.init_orient
        H, L = _rot(H, L, iy)
        H, U = _rot(H, U, ip)
        L, U = _rot(L, U, ir)
        self.H, self.L, self.U = H, L, U
        self.w, self.st, self.asc = float(cfg.width), float(cfg.step), 1.0
        self.u = self.v = 0.0
        self.arc = self.lage = 0.0
        self.prev_w = None
        self.group, self.group_counter = -1, 0
        self.attrs = {}
        self.stack, self.cut, self.poly = [], -1, None
        self.cur, self.current = 0, None
        self.pts, self.leaf_flags, self.curves, self.curve_group, self.polys, self.leaves, self.segs = [], [], [], [], [], [], []
        self.cols = {k: [] for k in ("width", "N", "u", "v", "arc", "lage", "gen", "segs", "div", "up", "attrs")}
        if record:
            self._point(self.pos, self.w, 0, cfg.cols, cfg.rows, False)

    # -- recording ---------------------------------------------------------------------------------------------------
    def _point(self, pos, width, born, s, d, leaf):
        c = self.cols
        self.pts.append(np.array(pos, np.float64))
        self.leaf_flags.append(leaf)
        c["width"].append(width)
        c["N"].append(self.H)
        c["u"].append(self.u)
        c["v"].append(self.v)
        c["arc"].append(self.arc)
        c["lage"].append(self.lage)
        c["gen"].append(born)
        c["segs"].append(s)
        c["div"].append(d)
        c["up"].append(-self.U)
        c["attrs"].append(self.attrs)
        return len(self.pts) - 1

    def _save(self):
        return (self.pos, self.H, self.L, self.U, self.w, self.st, self.asc, self.cur, self.u, self.v, self.arc, self.lage,
                self.group, self.attrs, self.prev_w)

    def _load(self, s):
        (self.pos, self.H, self.L, self.U, self.w, self.st, self.asc, self.cur, self.u, self.v, self.arc, self.lage,
         self.group, self.attrs, self.prev_w) = s

    # -- one symbol --------------------------------------------------------------------------------------------------
    def step(self, sym, args, born):
        if self.cut >= 0:                                     # skipping the rest of a branch after %
            if sym == "[":
                self.cut += 1
                return
            if sym != "]":
                return
            if self.cut > 0:
                self.cut -= 1
                return
            self.cut = -1
        cfg = self.cfg
        gf = self.grow if born == self.newest else 1.0
        if sym in _MOVES:
            self._move(sym, args, born, gf)
        elif sym in "+-&^\\/":
            base = cfg.yaw if sym in "+-" else cfg.pitch if sym in "&^" else cfg.roll
            a = _a(args, 0, None)
            ang = base * self.asc if a is None else math.radians(a)
            if cfg.cont_angles:
                ang *= gf
            if sym in "-^/":
                ang = -ang
            if sym in "+-":
                self.H, self.L = _rot(self.H, self.L, ang)
            elif sym in "&^":
                self.H, self.U = _rot(self.H, self.U, ang)
            else:
                self.L, self.U = _rot(self.L, self.U, ang)
        elif sym == "|":
            self.H, self.L = -self.H, -self.L
        elif sym == "*":
            self.L, self.U = -self.L, -self.U
        elif sym == "[":
            self.stack.append(self._save())
            self.current = None                               # a branch starts a new curve at this point
            self.w *= cfg.width_scale
        elif sym == "]":
            if not self.stack:
                if self.rec:
                    raise ValueError("']' without a matching '['")
                return
            self._load(self.stack.pop())
            self.current = None                               # continuing after a branch starts a new curve
        elif sym == "!":
            self.w *= _a(args, 0, cfg.width_scale)
        elif sym == "?":
            self.w /= max(_a(args, 0, cfg.width_scale), 1e-12)
        elif sym == '"':
            self.st *= _a(args, 0, cfg.step_scale)
        elif sym == "_":
            self.st /= max(_a(args, 0, cfg.step_scale), 1e-12)
        elif sym == ";":
            self.asc *= _a(args, 0, cfg.angle_scale)
        elif sym == "@":
            self.asc /= max(_a(args, 0, cfg.angle_scale), 1e-12)
        elif sym == "~":
            m = math.radians(_a(args, 0, 180.0))
            ry, rp, rr = (self.rng.random(3) * 2 - 1) * m
            self.H, self.L = _rot(self.H, self.L, ry)
            self.H, self.U = _rot(self.H, self.U, rp)
            self.L, self.U = _rot(self.L, self.U, rr)
        elif sym == "'":
            self.u += _a(args, 0, cfg.uv_inc[0])
        elif sym == "#":
            self.v += _a(args, 0, cfg.uv_inc[1])
        elif sym == "%":
            self.cut = 0
        elif sym == "T":
            self._tropism(_a(args, 0, cfg.gravity))
        elif sym == "$":
            self._orient(args)
        elif sym == "g":
            a = _a(args, 0, None)
            if a is None:
                self.group, self.group_counter = self.group_counter, self.group_counter + 1
            else:
                self.group = int(a) if a >= 0 else -1
            self.current = None
        elif sym == "a":
            self._attribute(args)
        elif sym == "{":
            self.poly = []
        elif sym == ".":
            if self.poly is not None and self.rec:
                if self.cur is not None and np.allclose(self.pts[self.cur], self.pos):
                    p = self.cur
                else:
                    p = self._point(self.pos, self.w, born, cfg.cols, cfg.rows, False)
                    self.cur = p
                self.poly.append(p)
        elif sym == "}":
            if self.poly is not None and len(self.poly) >= 3 and self.rec:
                self.polys.append((self.poly, self.group))
            self.poly = None
        elif sym in "JKM":
            self._leaf(sym, args, born)

    def _move(self, sym, args, born, gf):
        cfg = self.cfg
        half = 0.5 if sym in "Hh" else 1.0
        l = _a(args, 0, self.st) * half
        if cfg.random_scale:
            l *= max(0.0, 1.0 + cfg.random_scale * (self.rng.random() * 2 - 1))
        if cfg.cont_length:
            l *= gf
        wseg = _a(args, 1, self.w)
        if cfg.cont_width:
            wseg *= gf
        newpos = self.pos + self.H * l
        arc1 = self.arc + l
        if sym in "FH":
            lage1 = self.lage + cfg.vinc
            if self.rec:
                s = int(_a(args, 2, cfg.cols))
                d = int(_a(args, 3, cfg.rows))
                if cfg.tube:
                    w0 = self.prev_w if self.prev_w is not None else wseg
                    self.segs.append((self.pos, newpos, self.L, self.U, w0, wseg, self.lage, lage1, self.arc, arc1, born,
                                      self.group, s, d, dict(self.attrs)))
                else:
                    self.arc, self.lage = arc1, lage1
                    p = self._point(newpos, wseg, born, s, d, False)
                    if self.current is None:
                        self.current = [self.cur]
                        self.curves.append(self.current)
                        self.curve_group.append(self.group)
                    self.current.append(p)
                    self.cur = p
            self.prev_w = wseg
            self.pos, self.arc, self.lage = newpos, arc1, lage1
        elif sym in "fh":
            self.pos, self.arc = newpos, arc1
            if self.rec:
                self.cur = self._point(newpos, wseg, born, int(_a(args, 2, cfg.cols)), int(_a(args, 3, cfg.rows)), False)
            self.current = None
            self.prev_w = None
        else:                                                # G: move, record nothing
            self.pos, self.arc = newpos, arc1

    def _tropism(self, e):
        T = np.array([0.0, 0.0, -1.0])
        axis = np.cross(self.H, T)
        n = np.linalg.norm(axis)
        if n < 1e-12 or e == 0:
            return
        ang = e * n
        axis = axis / n
        self.H, self.L, self.U = (_rodrigues(v, axis, ang) for v in (self.H, self.L, self.U))

    def _orient(self, args):
        if args is not None and len(args) >= 3:
            d = np.array([args[0], args[1], args[2]], np.float64) - self.pos
            n = np.linalg.norm(d)
            if n > 1e-12:
                self.H = d / n
        L = np.cross(_UP_REF, self.H)
        n = np.linalg.norm(L)
        if n > 1e-9:
            self.L = L / n
            self.U = np.cross(self.H, self.L)
        else:
            L = self.L - self.H * np.dot(self.L, self.H)
            self.L = L / max(np.linalg.norm(L), 1e-12)
            self.U = np.cross(self.H, self.L)

    def _attribute(self, args):
        if not args or not isinstance(args[0], str):
            raise ValueError('a(name, v1, v2, v3) needs the attribute name in quotes, e.g. a("Cd",1,0,1)')
        name = args[0]
        if not _IDENT.match(name) or name == "P":
            raise ValueError(f"a(): {name!r} is not a usable attribute name")
        vals = tuple(float(v) for v in args[1:4] if not isinstance(v, str))
        if not vals:
            raise ValueError(f"a({name!r}, ...) needs at least one value")
        self.attrs = {**self.attrs, name: vals}

    def _leaf(self, kind, args, born):
        if not self.rec:
            return
        cfg = self.cfg
        if cfg.leaf_geo.get(kind) is None:
            if kind == "J":                                   # no input: a leaf point, as in the original node
                self._point(self.pos, self.w, born, cfg.cols, cfg.rows, True)
            return
        s = _a(args, 0, self.st)
        stamps = tuple(_a(args, 2 + k, None) for k in range(3))
        R = np.stack([-self.L, -self.U, self.H], 1)
        self.leaves.append((kind, self.pos.copy(), R, s, stamps, self.group))

    def check_closed(self):
        if self.stack:
            raise ValueError("'[' without a matching ']'")


# ---------------------------------------------------------------------------------------------------------------------
# Rewriting
# ---------------------------------------------------------------------------------------------------------------------
class _Ctx:
    """What expressions and the turtle need: variables, functions, the pic image and the meta-test region."""

    def __init__(self, cfg):
        self.cfg = cfg
        self.base = {"a": cfg.angle, "b": cfg.b, "c": cfg.c, "d": cfg.d, "T": cfg.gravity, **cfg.user_vars}
        self.g = {"__builtins__": {}, "sin": lambda v: math.sin(math.radians(v)), "cos": lambda v: math.cos(math.radians(v)),
                  "tan": lambda v: math.tan(math.radians(v)), "asin": lambda v: math.degrees(math.asin(max(-1.0, min(1.0, v)))),
                  "acos": lambda v: math.degrees(math.acos(max(-1.0, min(1.0, v)))), "min": min, "max": max, "abs": abs,
                  "sqrt": math.sqrt, "floor": math.floor, "ceil": math.ceil, "pow": math.pow, "exp": math.exp,
                  "log": math.log, "chan": self._chan, "inside": self._inside, "pic": self._pic, "rand": self._rand}

    def _chan(self, idx):
        return float(self.cfg.user_vars.get(f"{self.cfg.channel_prefix}{int(idx)}", 0.0))

    def _inside(self, x, y, z):
        box = self.cfg.meta_box
        if box is None:
            raise ValueError("in(x,y,z) needs geometry connected to the Meta Test input")
        lo, hi = box
        return float(lo[0] <= x <= hi[0] and lo[1] <= y <= hi[1] and lo[2] <= z <= hi[2])

    def _pic(self, x, y, plane=0):
        img = self.cfg.pic_image
        if img is None:
            raise ValueError("pic() needs a Pic Image File")
        h, w = img.shape[:2]
        px = img[int(round(min(max(y, 0.0), 1.0) * (h - 1)))][int(round(min(max(x, 0.0), 1.0) * (w - 1)))]
        plane = int(plane)
        return float(0.299 * px[0] + 0.587 * px[1] + 0.114 * px[2]) if plane == 0 else float(px[min(plane, 3) - 1])

    def _rand(self, seed):
        x = math.sin(float(seed) * 12.9898 + self.cfg.seed * 78.233) * 43758.5453
        return x - math.floor(x)


_TURTLE_VARS = re.compile(r"\b[xyzALWUV]\b|\bin\s*\(")


def _needs_turtle(rules, premise) -> bool:
    texts = [premise]
    for lst in rules.values():
        for r in lst:
            texts.append(r.cond or "")
            texts.extend(a for _s, args in r.tmpl for a in (args or ()))
    return any(_TURTLE_VARS.search(t) for t in texts)


def _partners(tokens):
    partner, st = [-1] * len(tokens), []
    for i, tok in enumerate(tokens):
        if tok[0] == "[":
            st.append(i)
        elif tok[0] == "]" and st:
            j = st.pop()
            partner[i], partner[j] = j, i
    return partner


def _left_ok(tokens, idx, want, ignore, partner, flat):
    j = idx - 1
    for w in reversed(want):
        while j >= 0:
            s = tokens[j][0]
            if s in ignore or (flat and s in "[]"):
                j -= 1
            elif s == "]":
                j = partner[j] - 1 if partner[j] >= 0 else j - 1
            elif s == "[":
                return False
            else:
                break
        if j < 0 or tokens[j][0] != w:
            return False
        j -= 1
    return True


def _right_ok(tokens, idx, want, ignore, partner, flat):
    j, n = idx + 1, len(tokens)
    for w in want:
        while j < n:
            s = tokens[j][0]
            if s in ignore or (flat and s in "[]"):
                j += 1
            elif s == "[":
                j = partner[j] + 1 if partner[j] >= 0 else n
            elif s == "]":
                return False
            else:
                break
        if j >= n or tokens[j][0] != w:
            return False
        j += 1
    return True


def _rewrite_plain(premise, rules, gens, rng) -> str:
    s = premise
    for _ in range(gens):
        out = []
        for ch in s:
            opts = rules.get(ch)
            if not opts:
                out.append(ch)
            elif len(opts) == 1:
                r = opts[0]
                out.append(r.rhs if (r.p >= 1.0 or rng.random() < r.p) else ch)
            else:
                w = np.array([r.p for r in opts], np.float64)
                out.append(opts[int(rng.choice(len(opts), p=w / w.sum()))].rhs if w.sum() > 0 else ch)
        s = "".join(out)
        if len(s) > MAX_SYMBOLS:
            raise ValueError(f"the string grew past {MAX_SYMBOLS:,} symbols; use fewer generations")
    return s


def _bound(args, k):
    return float(args[k]) if args is not None and k < len(args) and not isinstance(args[k], str) else 0.0


def _rewrite_tokens(tokens, rules, gens, rng, ctx):
    cfg = ctx.cfg
    ignore = set(cfg.context_ignore)
    flat = cfg.context_siblings
    contexts = any(r.left or r.right for lst in rules.values() for r in lst)
    track = _needs_turtle(rules, "")
    for t in range(gens):
        partner = _partners(tokens) if contexts else None
        tracker = Turtle(cfg, np.random.default_rng([cfg.seed & 0xFFFFFFFF, 2, t]), record=False) if track else None
        out = []
        for idx, tok in enumerate(tokens):
            sym, args, born = tok
            cand = rules.get(sym)
            chosen = None
            if cand:
                env = dict(ctx.base)
                env.update(i=float(idx), t=float(t), g=float(t - born))
                if tracker is not None:
                    env.update(x=float(tracker.pos[0]), y=float(tracker.pos[1]), z=float(tracker.pos[2]), A=tracker.arc,
                               L=tracker.st, W=tracker.w, U=tracker.u, V=tracker.v)
                matched = []
                for r in cand:
                    if r.left and not _left_ok(tokens, idx, r.left, ignore, partner, flat):
                        continue
                    if r.right and not _right_ok(tokens, idx, r.right, ignore, partner, flat):
                        continue
                    renv = env
                    if r.vars:
                        renv = dict(env)
                        for k, name in enumerate(r.vars):
                            renv[name] = _bound(args, k)
                    if r.cond is not None and not _ev(r.cond, renv, ctx.g):
                        continue
                    matched.append((r, renv))
                if len(matched) == 1:
                    if matched[0][0].p >= 1.0 or rng.random() < matched[0][0].p:
                        chosen = matched[0]
                elif matched:
                    w = np.array([m[0].p for m in matched], np.float64)
                    if w.sum() > 0:
                        chosen = matched[int(rng.choice(len(matched), p=w / w.sum()))]
            if chosen is not None:
                r, renv = chosen
                for s2, a2 in r.tmpl:
                    out.append((s2, None if a2 is None else tuple(_arg(a, renv, ctx.g) for a in a2), t + 1))
            else:
                out.append(tok)
            if tracker is not None:
                tracker.step(sym, args, born)
        tokens = out
        if len(tokens) > MAX_SYMBOLS:
            raise ValueError(f"the string grew past {MAX_SYMBOLS:,} symbols; use fewer generations")
    return tokens


def _premise_tokens(premise, ctx):
    env = dict(ctx.base)
    env.update(t=0.0, g=0.0, x=0.0, y=0.0, z=0.0, A=0.0, L=float(ctx.cfg.step), W=float(ctx.cfg.width), U=0.0, V=0.0)
    out = []
    for i, (sym, args) in enumerate(_tokenize(premise)):
        env["i"] = float(i)
        out.append((sym, None if args is None else tuple(_arg(a, env, ctx.g) for a in args), 0))
    return out


def expand(premise, rules, generations, seed, ctx, need_born=False):
    """The tokens ``(symbol, args, born)`` of the string after ``generations`` rewrites."""
    rng = np.random.default_rng(int(seed) & 0xFFFFFFFF)
    plain = all(r.plain for lst in rules.values() for r in lst) and "(" not in premise and not need_born
    if plain:
        s = _rewrite_plain(re.sub(r"\s+", "", premise), rules, generations, rng)
        return [(c, None, 0) for c in s]
    return _rewrite_tokens(_premise_tokens(premise, ctx), rules, generations, rng, ctx)


def _fmt(tok) -> str:
    sym, args, _born = tok
    if not args:
        return sym
    return sym + "(" + ",".join('"' + a + '"' if isinstance(a, str) else format(a, ".6g") for a in args) + ")"


def rewrite(premise: str, rules, generations: int, seed: int) -> str:
    """The string after ``generations`` rewrites with default settings (the turtle-independent part of the node)."""
    if isinstance(rules, str):
        rules = parse_rules(rules)
    ctx = _Ctx(_config(REGISTRY.get("lsystem").default_params()))
    return "".join(_fmt(t) for t in expand(premise, rules, int(generations), seed, ctx))


# ---------------------------------------------------------------------------------------------------------------------
# Files
# ---------------------------------------------------------------------------------------------------------------------
_image_cache = {}


def _load_image(file, what):
    path = paths.resolve(file)
    if not path or not os.path.isfile(path):
        raise ValueError(f"{what}: no such file {path or file!r}")
    key = (path, os.stat(path).st_mtime_ns)
    img = _image_cache.get(key)
    if img is None:
        try:
            from PIL import Image
            img = np.asarray(Image.open(path).convert("RGB"), np.float32) / 255.0
        except ImportError:
            from ..images import read_png
            pix = np.asarray(read_png(path, srgb=False), np.float32)
            pix = pix[..., None] if pix.ndim == 2 else pix
            img = np.repeat(pix[..., :1], 3, 2) if pix.shape[2] < 3 else pix[..., :3]
        except Exception as e:  # noqa: BLE001
            raise ValueError(f"{what}: cannot read {os.path.basename(path)}: {e}") from None
        _image_cache.clear()
        _image_cache[key] = img
    return img


def _sample_image(img, u, v):
    h, w = img.shape[:2]
    x = np.clip(np.rint((np.asarray(u) % 1.0) * (w - 1)).astype(int), 0, w - 1)
    y = np.clip(np.rint((1.0 - np.asarray(v) % 1.0) * (h - 1)).astype(int), 0, h - 1)
    return img[y, x].astype(np.float32)


def _stamp(path):
    try:
        st = os.stat(path)
        return f"{st.st_mtime_ns}:{st.st_size}"
    except OSError:
        return "missing"


def _key(p) -> str:
    parts = []
    for name, used in (("rule_file", p.get("read_rules_from_file", False)), ("image_file", p.get("apply_color", False)), ("pic_image_file", True)):
        path = paths.resolve(p.get(name, "")) if used else ""
        if path:
            parts.append(_stamp(path))
    return "|".join(parts)


def _rules_text(p) -> str:
    if not p.get("read_rules_from_file", False):
        return p.get("rules", "F=FF-[-F+F+F]+[+F-F-F]")
    path = paths.resolve(p.get("rule_file", "./rules.txt"))
    if not path or not os.path.isfile(path):
        raise ValueError(f"Rule File: no such file {path or p.get('rule_file')!r}")
    with open(path, encoding="utf-8") as f:
        return f.read()


def _user_variables(text: str) -> dict:
    out = {}
    for raw in text.replace(";", "\n").splitlines():
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        if "=" not in line:
            raise ValueError(f"variable {line!r} needs '=', e.g. e=0.5")
        name, value = (x.strip() for x in line.split("=", 1))
        if not _IDENT.match(name) or name in RESERVED:
            raise ValueError(f"variable name {name!r} is invalid or reserved (reserved: {' '.join(sorted(RESERVED))})")
        try:
            out[name] = float(value)
        except ValueError:
            raise ValueError(f"variable {name!r}: {value!r} is not a number") from None
    return out


# ---------------------------------------------------------------------------------------------------------------------
# Building geometry
# ---------------------------------------------------------------------------------------------------------------------
def _config(p, leaf_geo=None, meta=None):
    """Everything the turtle and the expressions read, from the node's parameters."""
    def _pick(primary, alias, default):
        if primary in p and p[primary] != default:
            return p[primary]
        if alias in p and p[alias] != default:
            return p[alias]
        return p.get(primary, p.get(alias, default))

    geo_type = _pick("type", "geometry_type", "skeleton")
    seed = int(_pick("random_seed", "seed", 0))
    step = float(_pick("step_size", "step", 0.1))
    step_scale = float(_pick("step_size_scale", "step_scale", 0.9))
    width = float(_pick("thickness", "width", 0.05))
    width_scale = float(_pick("thickness_scale", "width_scale", 0.7))

    if p.get("separate_angles", False):
        yaw, pitch, roll = (math.radians(p["yaw"]), math.radians(p["pitch"]), math.radians(p["roll"]))
    else:
        yaw = pitch = roll = math.radians(p.get("angle", 22.5))
    box = None
    if meta is not None and meta.npoints:
        box = (meta.P.min(0).astype(np.float64), meta.P.max(0).astype(np.float64))
    return SimpleNamespace(
        tube=geo_type == "tube", yaw=yaw, pitch=pitch, roll=roll, angle=float(p.get("angle", 22.5)),
        angle_scale=float(p.get("angle_scale", 1.0)), step=step, step_scale=step_scale,
        width=width, width_scale=width_scale, gravity=float(p.get("gravity", 0.0)),
        b=float(p.get("variable_b", 1.0)), c=float(p.get("variable_c", 1.0)), d=float(p.get("variable_d", 1.0)),
        start=tuple(float(x) for x in p.get("start_position", (0, 0, 0))),
        init_orient=tuple(math.radians(x) for x in p.get("initial_orientation", (0, 0, 0))),
        random_scale=float(p.get("random_scale", 0.0)), seed=seed,
        cont_angles=bool(p.get("continuous_angles", True)), cont_length=bool(p.get("continuous_length", True)),
        cont_width=bool(p.get("continuous_width", True)), apply_color=bool(p.get("apply_color", False)),
        uv_inc=(float(p.get("uv_increment_u", 0.1)), float(p.get("uv_increment_v", 0.1))),
        point_attrs=bool(p.get("point_attributes", False)),
        rows=int(p.get("rows", 3)), cols=int(p.get("cols", 3)), vinc=float(p.get("vertical_increment", 0.1)),
        tube_uvs=bool(p.get("apply_tube_texture_coordinates", False)), group_prefix=p.get("group_prefix", "lsys"),
        channel_prefix=p.get("channel_prefix", "lsys"),
        leaf_params=(p.get("leaf_param_a", "lsys"), p.get("leaf_param_b", ""), p.get("leaf_param_c", "")),
        context_ignore=p.get("context_ignore", "F+-"),
        context_siblings=bool(p.get("context_includes_siblings", False)),
        user_vars=_user_variables(p.get("variables", "")), leaf_geo=leaf_geo or {}, meta_box=box, color_image=None,
        pic_image=_load_image(p["pic_image_file"], "Pic Image File") if p.get("pic_image_file", "").strip() else None)


def _empty_part():
    return dict(P=np.zeros((0, 3), np.float32), vp=np.zeros(0, np.int32), sizes=np.zeros(0, np.int32), closed=np.zeros(0, bool),
                point={}, vertex={}, prim={}, group=np.zeros(0, np.int32))


def _custom_attrs(snapshots, keep):
    names = {}
    for s in {id(s): s for s in snapshots}.values():
        for k, v in s.items():
            names[k] = max(names.get(k, 0), len(v))
    out = {}
    for name, n in names.items():
        a = np.zeros((len(snapshots), n), np.float32)
        for i, s in enumerate(snapshots):
            v = s.get(name)
            if v:
                a[i, :len(v)] = v
        out[name] = (a[:, 0] if n == 1 else a)[keep]
    return out


def _skeleton_part(t, cfg):
    part = _empty_part()
    curves = [c for c in t.curves if len(c) >= 2]
    cgroup = [g for c, g in zip(t.curves, t.curve_group) if len(c) >= 2]
    polys = t.polys
    n = len(t.pts)
    used = np.zeros(n, bool)
    for c in curves:
        used[c] = True
    for poly, _g in polys:
        used[poly] = True
    keep = used | np.asarray(t.leaf_flags, bool)                    # drop points nothing refers to (e.g. after f)
    remap = np.cumsum(keep) - 1
    rings = curves + [poly for poly, _g in polys]
    part["P"] = np.asarray(t.pts, np.float32).reshape(-1, 3)[keep]
    part["vp"] = (np.concatenate([remap[np.asarray(c)] for c in rings]).astype(np.int32) if rings else np.zeros(0, np.int32))
    part["sizes"] = np.asarray([len(c) for c in rings], np.int32)
    part["closed"] = np.asarray([False] * len(curves) + [True] * len(polys), bool)
    part["group"] = np.asarray(cgroup + [g for _p, g in polys], np.int32)
    c = t.cols
    pt = part["point"]
    pt["width"] = np.asarray(c["width"], np.float32)[keep]
    pt["N"] = np.asarray(c["N"], np.float32).reshape(-1, 3)[keep]
    pt["group_leaves"] = np.asarray(t.leaf_flags, np.int32)[keep]
    if cfg.point_attrs:
        pt["segs"] = np.asarray(c["segs"], np.int32)[keep]
        pt["div"] = np.asarray(c["div"], np.int32)[keep]
        pt["lage"] = np.asarray(c["lage"], np.float32)[keep]
        pt["arc"] = np.asarray(c["arc"], np.float32)[keep]
        pt["up"] = np.asarray(c["up"], np.float32).reshape(-1, 3)[keep]
        pt["gen"] = np.asarray(c["gen"], np.int32)[keep]
    if cfg.apply_color:
        pt["Cd"] = _sample_image(cfg.color_image, np.asarray(c["u"])[keep], np.asarray(c["v"])[keep])
    pt.update(_custom_attrs(c["attrs"], keep))
    return part


def _tube_parts(t, cfg):
    groups = {}
    for i, s in enumerate(t.segs):
        rows, cols = max(3, min(int(s[13]), 256)), max(1, min(int(s[12]), 256))
        groups.setdefault((rows, cols), []).append(i)
    parts = []
    total = 0
    for (R, C), ids in groups.items():
        S = len(ids)
        total += S * (C + 1) * R
        if total > MAX_TUBE_POINTS:
            raise ValueError(f"the tubes would need over {MAX_TUBE_POINTS:,} points; use fewer generations, Rows or Cols")
        seg = [t.segs[i] for i in ids]

        def col(k):
            return np.asarray([s[k] for s in seg], np.float64)

        p0, p1, L, U = col(0), col(1), col(2), col(3)
        w0, w1, v0, v1 = col(4), col(5), col(6), col(7)
        grp = np.asarray([s[11] for s in seg], np.int32)
        th = 2 * np.pi * np.arange(R) / R
        tt = np.arange(C + 1) / C
        center = p0[:, None, :] + (p1 - p0)[:, None, :] * tt[None, :, None]
        rad = 0.5 * (w0[:, None] + (w1 - w0)[:, None] * tt[None, :])
        direc = np.cos(th)[None, :, None] * L[:, None, :] + np.sin(th)[None, :, None] * U[:, None, :]
        pos = center[:, :, None, :] + rad[:, :, None, None] * direc[:, None, :, :]
        s_i, k_i, j_i = np.meshgrid(np.arange(S), np.arange(C), np.arange(R), indexing="ij")
        j2 = (j_i + 1) % R

        def idx(k, j):
            return (s_i * (C + 1) + k) * R + j

        vp = np.stack([idx(k_i, j_i), idx(k_i, j2), idx(k_i + 1, j2), idx(k_i + 1, j_i)], -1).reshape(-1)
        part = _empty_part()
        part["P"] = pos.reshape(-1, 3).astype(np.float32)
        part["vp"] = vp.astype(np.int32)
        part["sizes"] = np.full(S * C * R, 4, np.int32)
        part["closed"] = np.ones(S * C * R, bool)
        part["group"] = np.repeat(grp, C * R)
        part["point"]["width"] = np.broadcast_to(2 * rad[:, :, None], (S, C + 1, R)).reshape(-1).astype(np.float32)
        part["point"]["N"] = np.broadcast_to(direc[:, None, :, :], (S, C + 1, R, 3)).reshape(-1, 3).astype(np.float32)
        part["point"]["group_leaves"] = np.zeros(S * (C + 1) * R, np.int32)
        if cfg.tube_uvs:
            u = np.stack([j_i / R, (j_i + 1) / R, (j_i + 1) / R, j_i / R], -1)
            vk = v0[s_i] + (v1 - v0)[s_i] * k_i / C
            vk1 = v0[s_i] + (v1 - v0)[s_i] * (k_i + 1) / C
            v = np.stack([vk, vk, vk1, vk1], -1)
            part["vertex"]["uv"] = np.stack([u, v], -1).reshape(-1, 2).astype(np.float32)
        attrs = [s[14] for s in seg]
        if any(attrs):
            for name in {k for a in attrs for k in a}:
                n = max(len(a[name]) for a in attrs if name in a)
                arr = np.zeros((S, n), np.float32)
                for i, a in enumerate(attrs):
                    if name in a:
                        arr[i, :len(a[name])] = a[name]
                full = np.repeat(arr, (C + 1) * R, axis=0)
                part["point"][name] = full[:, 0] if n == 1 else full
        parts.append(part)
    return parts


def _leaf_parts(t, cfg):
    parts = []
    for kind in "JKM":
        inst = [x for x in t.leaves if x[0] == kind]
        geo = cfg.leaf_geo.get(kind)
        if not inst or geo is None or not geo.npoints:
            continue
        T, S = len(inst), geo.npoints
        if T * S > MAX_LEAF_POINTS:
            raise ValueError(f"{T:,} copies of {S:,} points is over the {MAX_LEAF_POINTS:,} point limit; use fewer generations")
        pos = np.array([x[1] for x in inst])
        R = np.array([x[2] for x in inst])
        sc = np.array([x[3] for x in inst], np.float64)
        topo = geo.topology
        nv, npr = geo.nvertices, geo.nprims
        part = _empty_part()
        world = np.einsum("tij,sj->tsi", R, geo.P.astype(np.float64)) * sc[:, None, None]
        part["P"] = (world + pos[:, None, :]).reshape(T * S, 3).astype(np.float32)
        part["vp"] = (np.tile(topo.vertex_point, T) + np.repeat(np.arange(T, dtype=np.int32) * np.int32(S), nv)).astype(np.int32)
        part["sizes"] = np.tile(topo.prim_sizes(), T).astype(np.int32)
        part["closed"] = np.tile(topo.closed_mask(), T)
        part["group"] = np.repeat(np.array([x[5] for x in inst], np.int32), npr)
        for name, arr in geo.point.items():
            if name == "P":
                continue
            if name == "N":
                n = np.einsum("tij,sj->tsi", R, arr.astype(np.float64)).reshape(T * S, 3)
                part["point"]["N"] = (n / np.maximum(np.linalg.norm(n, axis=1, keepdims=True), 1e-30)).astype(np.float32)
            else:
                part["point"][name] = np.tile(arr, (T,) + (1,) * (arr.ndim - 1))
        for k, name in enumerate(cfg.leaf_params):
            if name.strip() and any(x[4][k] is not None for x in inst):
                vals = np.array([0.0 if x[4][k] is None else x[4][k] for x in inst], np.float32)
                part["point"][name.strip()] = np.repeat(vals, S)
        for name, arr in geo.vertex.items():
            part["vertex"][name] = np.tile(arr, (T,) + (1,) * (arr.ndim - 1))
        for name, arr in geo.prim.items():
            part["prim"][name] = np.tile(arr, (T,) + (1,) * (arr.ndim - 1))
        parts.append(part)
    return parts


def _union(parts, domain, counts):
    template = {}
    for p in parts:
        for name, arr in p[domain].items():
            template.setdefault(name, arr)
    out = {}
    for name, tmpl in template.items():
        out[name] = np.concatenate([p[domain][name] if name in p[domain]
                                    else np.zeros((n,) + tmpl.shape[1:], tmpl.dtype) for p, n in zip(parts, counts)])
    return out


def _merge_parts(parts, cfg):
    parts = [p for p in parts if len(p["P"]) or len(p["sizes"])] or [_empty_part()]
    npts = [len(p["P"]) for p in parts]
    offs = np.concatenate([[0], np.cumsum(npts)[:-1]]).astype(np.int64)
    P = np.concatenate([p["P"] for p in parts])
    vp = np.concatenate([p["vp"].astype(np.int64) + o for p, o in zip(parts, offs)]).astype(np.int32)
    sizes = np.concatenate([p["sizes"] for p in parts])
    closed = np.concatenate([p["closed"] for p in parts])
    point = _union(parts, "point", npts)
    vertex = _union(parts, "vertex", [len(p["vp"]) for p in parts])
    prim = _union(parts, "prim", [len(p["sizes"]) for p in parts])
    group = np.concatenate([p["group"] for p in parts])
    for gid in np.unique(group[group >= 0]):
        prim[f"group_{cfg.group_prefix}{int(gid)}"] = (group == gid).astype(np.int32)
    return B.make(P, vp, B.offsets_from_sizes(sizes), point=point, vertex=vertex, prim=prim,
                  closed=None if closed.all() else closed)


_IMG = "file:open:.png,.jpg,.jpeg,.tga,.bmp,.tif,.tiff"


@node("lsystem", category="source", key_extra=_key, inputs=[
    Input("leaf_j", optional=True, doc="Geometry stamped by the J command"),
    Input("leaf_k", optional=True, doc="Geometry stamped by the K command"),
    Input("leaf_m", optional=True, doc="Geometry stamped by the M command"),
    Input("meta_test", optional=True, doc="Region for in(x,y,z): the bounding box of this geometry")], actions=[
    Action("write_rules", "Write Rule Parameters to File", doc="Write the rules to the Rule File, one per line")], params=[
    Enum("type", ["skeleton", "tube"], label="Type", doc="skeleton: polyline curves; tube: polygon tubes around them"),
    Enum("geometry_type", ["skeleton", "tube"], label="Type", ui="hidden",
         doc="Alias for type; kept for backwards compatibility"),
    Float("generations", 4.0, min=0.0, max=12.0, doc="Rewrites to apply. A fraction grows the last rewrite partway when the "
                                                      "Continuous options are on"),
    Vec3("start_position", (0, 0, 0), doc="Where the turtle starts"),
    Float("random_scale", 0.0, min=0.0, max=1.0, doc="Randomly scales the lengths of F and the other moves by up to this fraction"),
    Int("random_seed", 0, label="Random Seed", doc="For stochastic rules, ~ and Random Scale"),
    Int("seed", 0, label="Random Seed", ui="hidden", doc="Alias for random_seed; kept for backwards compatibility"),
    Bool("continuous_angles", True, doc="Scale the angles turned by the last rewrite by the fractional part of Generations"),
    Bool("continuous_length", True, doc="Scale the lengths moved by the last rewrite by the fractional part of Generations"),
    Bool("continuous_width", True, doc="Scale the widths made by the last rewrite by the fractional part of Generations"),
    Bool("apply_color", False, doc="Give every point a colour Cd looked up in the Image File at the turtle's U and V"),
    Str("image_file", "", kind=_IMG, doc="Image for Apply Color"),
    Float("uv_increment_u", 0.1, label="UV Increment U", doc="What the ' command adds to U"),
    Float("uv_increment_v", 0.1, label="UV Increment V", doc="What the # command adds to V"),
    Bool("point_attributes", False, doc="Skeleton only: also write segs, div, lage, arc, up and gen on the points"),
    Int("rows", 3, min=3, max=256, label="Tube Rows", doc="Sides of a tube (3: swept triangles)"),
    Int("cols", 3, min=1, max=256, label="Tube Cols", doc="Cross sections one F is cut into"),
    Float("tension", 3.0, min=0.0, label="Tube Tension",
          doc="Accepted for Houdini parity. Tubes are straight per F, so it has no effect yet"),
    Float("branch_blend", 0.0, min=0.0, max=1.0, label="Tube Branch Blend",
          doc="Accepted for Houdini parity. Tubes do not blend into their parent yet"),
    Float("thickness", 0.05, min=0.0, soft_max=1.0, label="Thickness",
          doc="Starting width (point attribute 'width', used by sweep; a tube's diameter)"),
    Float("width", 0.05, min=0.0, soft_max=1.0, label="Thickness", ui="hidden",
          doc="Alias for thickness; kept for backwards compatibility"),
    Float("thickness_scale", 0.7, min=0.0, max=1.0, label="Thickness Scale", doc="Factor applied by !, and by each [ branch"),
    Float("width_scale", 0.7, min=0.0, max=1.0, label="Thickness Scale", ui="hidden",
          doc="Alias for thickness_scale; kept for backwards compatibility"),
    Bool("apply_tube_texture_coordinates", False, doc="Tube only: write vertex uv (around the tube, along its length)"),
    Float("vertical_increment", 0.1, min=0.0, soft_max=1.0, label="Tube Vertical Increment",
          doc="What lage and the tube's V coordinate grow by per F"),
    Float("step_size", 0.1, min=0.0, soft_max=2.0, label="Step Size", doc="Length of F and f"),
    Float("step", 0.1, min=0.0, soft_max=2.0, label="Step Size", ui="hidden",
          doc="Alias for step_size; kept for backwards compatibility"),
    Float("step_size_scale", 0.9, min=0.0, soft_max=2.0, label="Step Size Scale", doc='Factor applied by " (and divided by _)'),
    Float("step_scale", 0.9, min=0.0, soft_max=2.0, label="Step Size Scale", ui="hidden",
          doc="Alias for step_size_scale; kept for backwards compatibility"),
    Float("angle", 22.5, soft_min=0.0, soft_max=90.0, doc="Degrees for + - & ^ \\ /; also the variable a"),
    Float("angle_scale", 1.0, soft_min=0.0, soft_max=2.0, doc="Factor applied by ; (and divided by @)"),
    Bool("separate_angles", False, doc="Use independent yaw, pitch and roll angles instead of Angle"),
    Float("yaw", 22.5, doc="Degrees for + (left) and - (right), in the turtle frame"),
    Float("pitch", 22.5, doc="Degrees for & (down) and ^ (up), in the turtle frame"),
    Float("roll", 137.5, doc="Degrees for backslash and /, around the current heading"),
    Vec3("initial_orientation", (0, 0, 0), doc="Initial turtle yaw, pitch, roll in degrees, applied in that order; zero starts along +Z"),
    Float("variable_b", 1.0, label="Variable b", doc="The expression variable b"),
    Float("variable_c", 1.0, label="Variable c", doc="The expression variable c"),
    Float("variable_d", 1.0, label="Variable d", doc="The expression variable d"),
    Float("gravity", 0.0, soft_min=-1.0, soft_max=1.0, doc="Tropism of T towards -Z; also the variable T"),
    Str("variables", "", kind="code", label="Variables",
        doc="More expression variables, one 'name=value' per line (a name is letters, digits and _; chan(i) reads <Channel Prefix>i)"),
    Str("pic_image_file", "", kind=_IMG, doc="Image read by the pic() function"),
    Str("group_prefix", "lsys", doc="The g command makes primitive groups named <prefix><number>"),
    Str("channel_prefix", "lsys", doc="chan(i) reads the variable named <prefix><i>"),
    Str("leaf_param_a", "lsys", label="Leaf Param A", doc="Point attribute that receives the 3rd value of J/K/M(s,x,a,b,c)"),
    Str("leaf_param_b", "", label="Leaf Param B", doc="Point attribute that receives the 4th value of J/K/M(s,x,a,b,c)"),
    Str("leaf_param_c", "", label="Leaf Param C", doc="Point attribute that receives the 5th value of J/K/M(s,x,a,b,c)"),
    Bool("read_rules_from_file", False, doc="Read the rules from the Rule File instead of the Rules box"),
    Str("rule_file", "./rules.txt", kind="file:open:.txt", doc="One rule per line; blank lines and lines starting with # are ignored"),
    Str("context_ignore", "F+-", doc="Symbols skipped when matching the context of a rule"),
    Bool("context_includes_siblings", False, doc="Let rule contexts see across branches ([A]Q[B])"),
    Str("premise", "F", doc="The starting string"),
    Str("rules", "F=FF-[-F+F+F]+[+F-F-F]", kind="code",
        doc="Rules, one per line: [left<]A[>right][:condition]=replacement[:probability]"),
])
def lsystem(leaf_j, leaf_k, leaf_m, meta_test, **p):
    """Grow plants and fractals from rewriting rules (Houdini's L-System). Output: curves along the turtle's path (+Z is up)
    with a width attribute, or polygon tubes; leaf points (group 'leaves') where J appears with no Leaf J input, and copies of the
    Leaf J/K/M inputs at J/K/M. + / - yaw, & / ^ pitch, backslash / slash roll in the moving turtle frame. Enable Separate Angles
    to control each axis independently. Branches restore the entire frame.

    Differs from Houdini in these ways: + turns left and - right (the node's original convention), Rules is one multi-line box
    instead of numbered fields, the meta test is the bounding box of its input rather than a metaball field, and tubes are
    straight per F (Tension and Branch Blend are accepted but not used yet)."""
    cfg = _config(p, {"J": leaf_j, "K": leaf_k, "M": leaf_m}, meta_test)
    if cfg.apply_color:
        img_f = p.get("image_file", "")
        if not img_f.strip():
            raise ValueError("Apply Color needs an Image File")
        cfg.color_image = _load_image(img_f, "Image File")
    rules = parse_rules(_rules_text(p))
    gen = float(p.get("generations", 4.0))
    n = int(math.floor(gen + 1e-9))
    frac = gen - n if gen - n > 1e-9 else 0.0
    if frac and not (cfg.cont_angles or cfg.cont_length or cfg.cont_width):
        frac = 0.0
    total = n + (1 if frac else 0)
    ctx = _Ctx(cfg)
    tokens = expand(p.get("premise", "F"), rules, total, cfg.seed, ctx, need_born=bool(frac) or cfg.point_attrs)
    turtle = Turtle(cfg, np.random.default_rng([cfg.seed & 0xFFFFFFFF, 1]), record=True,
                    grow=frac if frac else 1.0, newest=total if frac else -1)
    for sym, args, born in tokens:
        turtle.step(sym, args, born)
    turtle.check_closed()
    parts = _tube_parts(turtle, cfg) if cfg.tube else []
    parts.append(_skeleton_part(turtle, cfg))
    parts.extend(_leaf_parts(turtle, cfg))
    out = _merge_parts(parts, cfg)
    out.detail["lsystem_symbols"] = len(tokens)
    return out


@action("lsystem", "write_rules")
def _write_rules(session, node_id, p):
    path = paths.resolve(p.get("rule_file", ""))
    if not path:
        raise ValueError("set 'rule_file' to the file to write")
    lines = [ln.strip() for ln in p.get("rules", "").splitlines() if ln.strip()]
    os.makedirs(os.path.dirname(path) or ".", exist_ok=True)
    with open(path, "w", encoding="utf-8") as f:
        f.write("\n".join(lines) + "\n")
    return {"ok": True, "message": f"Wrote {len(lines)} rule(s) to {p.get('rule_file', path)}"}
