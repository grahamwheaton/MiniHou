"""Text geometry generation (Houdini's Font SOP): raster-free true vector glyph outlines, polygon face
triangulation with hole cutting, alignment, tracking, kerning, and attribute tagging.
"""
from __future__ import annotations

import glob
import math
import os
import re
import sys
from typing import Dict, List, Optional, Tuple

import numpy as np
from fontTools.pens.basePen import BasePen
from fontTools.ttLib import TTFont

from .. import paths, timectx
from ..boolean import _area2, _inside_poly, earclip, holes_into
from ..sdk import Bool, Enum, Float, Handle, Param, Str, Vec3, node
from . import _build as B
from ._util import euler_xyz_matrix

# ---------------------------------------------------------------------------------------------------------------------
# Font Discovery and Loading
# ---------------------------------------------------------------------------------------------------------------------
_font_cache: Dict[Tuple[str, int], TTFont] = {}
_font_path_cache: Dict[str, Optional[str]] = {}


def _candidate_font_dirs() -> List[str]:
    dirs = []
    # Houdini bundled fonts
    for p in glob.glob("C:/Program Files/Side Effects Software/*/houdini/fonts"):
        dirs.append(p)
    if sys.platform == "win32":
        windir = os.environ.get("WINDIR", "C:\\Windows")
        dirs.append(os.path.join(windir, "Fonts"))
        local_app = os.environ.get("LOCALAPPDATA")
        if local_app:
            dirs.append(os.path.join(local_app, "Microsoft", "Windows", "Fonts"))
    elif sys.platform == "darwin":
        dirs.extend(["/Library/Fonts", "/System/Library/Fonts", os.path.expanduser("~/Library/Fonts")])
    else:
        dirs.extend(["/usr/share/fonts", "/usr/local/share/fonts", os.path.expanduser("~/.fonts"),
                     os.path.expanduser("~/.local/share/fonts")])
    return [d for d in dirs if os.path.isdir(d)]


def _clean_str(s: str) -> str:
    return re.sub(r"[^a-z0-9]", "", s.lower())


def _find_font(name_or_path: str) -> Optional[str]:
    """Find a font on disk from a file path or font family name."""
    if not name_or_path:
        name_or_path = "Lato Bold"
    cached = _font_path_cache.get(name_or_path)
    if cached is not None:
        return cached

    # 1. Direct path check
    resolved = paths.resolve(name_or_path)
    if resolved and os.path.isfile(resolved):
        _font_path_cache[name_or_path] = resolved
        return resolved
    if os.path.isfile(name_or_path):
        abs_path = os.path.abspath(name_or_path)
        _font_path_cache[name_or_path] = abs_path
        return abs_path

    # 2. Search font directories by file stem
    target = _clean_str(name_or_path)
    dirs = _candidate_font_dirs()
    for d in dirs:
        try:
            for fname in os.listdir(d):
                if not fname.lower().endswith((".ttf", ".otf")):
                    continue
                stem = _clean_str(os.path.splitext(fname)[0])
                if stem == target:
                    full = os.path.join(d, fname)
                    _font_path_cache[name_or_path] = full
                    return full
        except OSError:
            continue

    # 3. Fallback common sans-serif fonts
    fallback_stems = ["arial", "segoeui", "helvetica", "dejavusans", "liberationsans", "tahoma", "verdana"]
    for d in dirs:
        try:
            for fname in os.listdir(d):
                if not fname.lower().endswith((".ttf", ".otf")):
                    continue
                stem = _clean_str(os.path.splitext(fname)[0])
                if any(stem.startswith(fb) for fb in fallback_stems):
                    full = os.path.join(d, fname)
                    _font_path_cache[name_or_path] = full
                    return full
        except OSError:
            continue

    # 4. Any .ttf/.otf in candidate dirs
    for d in dirs:
        try:
            for fname in os.listdir(d):
                if fname.lower().endswith((".ttf", ".otf")):
                    full = os.path.join(d, fname)
                    _font_path_cache[name_or_path] = full
                    return full
        except OSError:
            continue

    return None


def _load_font(path: str) -> TTFont:
    try:
        st = os.stat(path)
        key = (path, st.st_mtime_ns)
    except OSError:
        key = (path, 0)
    font = _font_cache.get(key)
    if font is None:
        font = TTFont(path)
        _font_cache[key] = font
    return font


# ---------------------------------------------------------------------------------------------------------------------
# Glyph Outline Pen
# ---------------------------------------------------------------------------------------------------------------------
class OutlinePen(BasePen):
    """Subclass of fontTools BasePen to sample quadratic and cubic Bezier curves into 2-D linear contours."""

    def __init__(self, glyph_set=None, lod: float = 1.0):
        super().__init__(glyph_set)
        self.lod = max(0.01, float(lod))
        self.contours: List[List[Tuple[float, float]]] = []
        self.current_contour: List[Tuple[float, float]] = []

    def _moveTo(self, pt: Tuple[float, float]):
        if self.current_contour:
            self._closePath()
        self.current_contour = [(float(pt[0]), float(pt[1]))]

    def _lineTo(self, pt: Tuple[float, float]):
        self.current_contour.append((float(pt[0]), float(pt[1])))

    def _qCurveToOne(self, pt1: Tuple[float, float], pt2: Tuple[float, float]):
        p0 = self._getCurrentPoint()
        steps = max(2, int(round(4 * self.lod)))
        x0, y0 = p0
        x1, y1 = pt1
        x2, y2 = pt2
        for i in range(1, steps + 1):
            t = i / steps
            om = 1.0 - t
            x = om * om * x0 + 2.0 * om * t * x1 + t * t * x2
            y = om * om * y0 + 2.0 * om * t * y1 + t * t * y2
            self.current_contour.append((x, y))

    def _curveToOne(self, pt1: Tuple[float, float], pt2: Tuple[float, float], pt3: Tuple[float, float]):
        p0 = self._getCurrentPoint()
        steps = max(3, int(round(6 * self.lod)))
        x0, y0 = p0
        x1, y1 = pt1
        x2, y2 = pt2
        x3, y3 = pt3
        for i in range(1, steps + 1):
            t = i / steps
            om = 1.0 - t
            x = om * om * om * x0 + 3.0 * om * om * t * x1 + 3.0 * om * t * t * x2 + t * t * t * x3
            y = om * om * om * y0 + 3.0 * om * om * t * y1 + 3.0 * om * t * t * y2 + t * t * t * y3
            self.current_contour.append((x, y))

    def _closePath(self):
        if self.current_contour:
            # Drop duplicate closing point
            if len(self.current_contour) > 1:
                p0 = self.current_contour[0]
                pn = self.current_contour[-1]
                if math.hypot(p0[0] - pn[0], p0[1] - pn[1]) < 1e-6:
                    self.current_contour.pop()
            if len(self.current_contour) >= 3:
                self.contours.append(self.current_contour)
            self.current_contour = []

    def _endPath(self):
        self._closePath()


# ---------------------------------------------------------------------------------------------------------------------
# Variable Expansion & Kerning
# ---------------------------------------------------------------------------------------------------------------------
def _expand_text(text: str, frame: int, fps: float) -> str:
    """Expand Houdini variables ($F, $T, $FPS, backticks) in the text string."""
    def _eval_backtick(match):
        expr = match.group(1).strip()
        try:
            val = eval(expr, {"__builtins__": {}}, {"F": frame, "T": (frame - 1.0) / fps, "FPS": fps})
            return str(val)
        except Exception:
            return match.group(0)

    res = re.sub(r"`([^`]+)`", _eval_backtick, text)

    def _repl_f(match):
        pad = match.group(1)
        if pad:
            return f"{frame:0{int(pad)}d}"
        return str(frame)

    res = re.sub(r"\$F(\d+)?\b", _repl_f, res)
    t = (frame - 1.0) / fps
    res = re.sub(r"\$T\b", f"{t:.4g}", res)
    res = re.sub(r"\$FPS\b", f"{fps:g}", res)
    return res


def _get_kerning(font: TTFont, left_glyph: str, right_glyph: str) -> float:
    """Look up pair kerning in font's 'kern' table."""
    if "kern" not in font:
        return 0.0
    for sub in font["kern"].kernTables:
        table = getattr(sub, "kernTable", None)
        if table:
            pair = (left_glyph, right_glyph)
            if pair in table:
                return float(table[pair])
    return 0.0


# ---------------------------------------------------------------------------------------------------------------------
# Node Cache Key
# ---------------------------------------------------------------------------------------------------------------------
def _key(p) -> str:
    parts = []
    text = p.get("text", "")
    if re.search(r"\$(F|T|FPS)|`", text):
        parts.append(timectx.key())
    font_path = _find_font(p.get("font", "Lato Bold"))
    if font_path and os.path.isfile(font_path):
        try:
            st = os.stat(font_path)
            parts.append(f"{font_path}:{st.st_mtime_ns}:{st.st_size}")
        except OSError:
            pass
    return "|".join(parts)


# ---------------------------------------------------------------------------------------------------------------------
# Node Declaration
# ---------------------------------------------------------------------------------------------------------------------
@node("font", category="source", key_extra=_key, inputs=[], params=[
    Enum("primitive_type", ["polygons", "bezier"], label="Primitive Type", default="polygons",
         doc="Type of geometry to create: 'polygons' (2-D faces with holes) or 'bezier' (polygon curve outlines)"),
    Str("font", "Lato Bold", kind="file:open:.ttf,.otf,.woff,.woff2", label="Font",
        doc="Font family name (e.g. 'Lato Bold', 'Arial') or path to a .ttf/.otf font file"),
    Str("text", "Frame: $F", kind="code", label="Text",
        doc="Text to be generated. Supports $F (current frame), $T (time), and backtick expressions"),
    Enum("halign", ["left", "center", "right", "justified"], label="Horizontal Alignment", default="center",
         doc="How the text is aligned horizontally around the origin"),
    Enum("valign", ["first_line", "top", "middle", "bottom"], label="Vertical Alignment", default="middle",
         doc="How the text is aligned vertically around the origin"),
    Bool("first_line", False, ui="hidden", label="First Line", doc="Alias for valign='first_line'"),
    Bool("top", False, ui="hidden", label="Top", doc="Alias for valign='top'"),
    Bool("middle", False, ui="hidden", label="Middle", doc="Alias for valign='middle'"),
    Bool("bottom", False, ui="hidden", label="Bottom", doc="Alias for valign='bottom'"),
    Bool("use_descender", False, label="Include Descender in Vertical Alignment",
         doc="If enabled, the alignment baseline is set to the descender rather than the font baseline"),
    Vec3("origin", (0.0, 0.0, 0.0), label="Origin", doc="Origin translation along X, Y, Z"),
    Vec3("rotate", (0.0, 0.0, 0.0), label="Rotate", doc="Euler rotation around X, Y, Z in degrees"),
    Vec3("scale_xy", (1.0, 1.0, 1.0), label="Scale XY", doc="Scale factors along X and Y"),
    Vec3("scale", (1.0, 1.0, 1.0), ui="hidden", label="Scale", doc="Alias for scale_xy"),
    Float("font_size", 1.0, min=0.0001, soft_max=10.0, label="Font Size", doc="Overall font size scale factor"),
    Float("fontsize", 1.0, min=0.0001, soft_max=10.0, ui="hidden", label="Font Size", doc="Alias for font_size"),
    Vec3("tracking_xy", (0.0, 0.0, 0.0), label="Tracking XY",
         doc="Tracking along X (extra letter spacing) and Y (leading/line spacing adjustment)"),
    Vec3("tracking", (0.0, 0.0, 0.0), ui="hidden", label="Tracking", doc="Alias for tracking_xy"),
    Float("oblique", 0.0, soft_min=-45.0, soft_max=45.0, label="Oblique Angle", doc="Amount of slant in degrees"),
    Float("lod", 1.0, min=0.01, soft_max=10.0, label="Level of Detail",
          doc="Controls the number of points in polygonal fonts"),
    Bool("automatic_kerning", True, label="Automatic Kerning",
         doc="Automatically apply font kerning pairs between characters"),
    Bool("hole_faces", True, label="Hole Faces",
         doc="Generates holes in polygons and Bezier faces"),
    Bool("add_attributes", False, label="Add Attributes",
         doc="Create textsymbol (Unicode codepoint) and textindex (character index) primitive attributes"),
    Bool("add_text_attributes", False, ui="hidden", label="Add Text Attributes", doc="Alias for add_attributes"),
], handles=[
    Handle("origin", axis="x", comp=0, tool="move"),
    Handle("origin", axis="y", comp=1, tool="move"),
    Handle("origin", axis="z", comp=2, tool="move")
])
def font(**p):
    """Generate 2-D text geometry from TrueType/OpenType font outlines (Houdini's Font SOP).

    Produces planar polygon faces with holes cleanly cut out, or closed curve outlines. Supports
    alignment, letter and line tracking, kerning, oblique slant, arbitrary scaling and rotation,
    and textsymbol/textindex primitive attributes.
    """
    def _pick(primary, alias, default):
        if primary in p and p[primary] != default:
            return p[primary]
        if alias in p and p[alias] != default:
            return p[alias]
        return p.get(primary, p.get(alias, default))

    prim_type = p.get("primitive_type", "polygons")
    font_name = p.get("font", "Lato Bold")
    raw_text = p.get("text", "Frame: $F")
    halign = p.get("halign", "center")

    # Resolve vertical alignment choices and aliases
    if p.get("first_line"):
        valign = "first_line"
    elif p.get("top"):
        valign = "top"
    elif p.get("bottom"):
        valign = "bottom"
    elif p.get("middle"):
        valign = "middle"
    else:
        valign = p.get("valign", "middle")

    use_descender = bool(p.get("use_descender", False))
    origin = tuple(float(x) for x in p.get("origin", (0.0, 0.0, 0.0)))
    rotate = tuple(float(x) for x in p.get("rotate", (0.0, 0.0, 0.0)))
    scale_xy = _pick("scale_xy", "scale", (1.0, 1.0, 1.0))
    scale_xy = (float(scale_xy[0]), float(scale_xy[1]))
    font_size = float(_pick("font_size", "fontsize", 1.0))
    tracking_xy = _pick("tracking_xy", "tracking", (0.0, 0.0, 0.0))
    track_x = float(tracking_xy[0])
    track_y = float(tracking_xy[1])
    oblique = float(p.get("oblique", 0.0))
    lod = float(p.get("lod", 1.0))
    auto_kern = bool(p.get("automatic_kerning", True))
    hole_faces = bool(p.get("hole_faces", True))
    add_attrs = bool(p.get("add_attributes", False) or p.get("add_text_attributes", False))

    # Expand variables ($F, $T, $FPS, backticks)
    frame_val = int(round(timectx.frame()))
    fps_val = float(timectx.fps())
    expanded_text = _expand_text(raw_text, frame_val, fps_val)

    if not expanded_text:
        return B.make(np.zeros((0, 3), np.float32), np.zeros(0, np.int32), np.zeros(1, np.int32),
                      point={"N": np.zeros((0, 3), np.float32)})

    # Load font
    font_path = _find_font(font_name)
    if not font_path or not os.path.isfile(font_path):
        raise ValueError(f"Font: cannot find font {font_name!r}")
    ttfont = _load_font(font_path)

    head = ttfont["head"]
    units_per_em = float(head.unitsPerEm)
    hhea = ttfont["hhea"]
    ascender = float(hhea.ascender)
    descender = float(hhea.descender)
    line_gap = float(getattr(hhea, "lineGap", 0))
    default_line_height = ascender - descender + line_gap
    line_advance = default_line_height + (track_y * units_per_em)

    cmap = ttfont.getBestCmap() or {}
    gset = ttfont.getGlyphSet()

    # Split text into lines
    lines = expanded_text.split("\n")
    num_lines = len(lines)

    # First pass: measure lines and extract glyph outlines
    line_widths = []
    line_glyph_data = []  # list of lines, each list of (char_idx, code, gname, glyph, width, kern_before, contours)
    global_char_idx = 0

    for line in lines:
        gdata = []
        cur_w = 0.0
        prev_gname = None
        for char in line:
            code = ord(char)
            gname = cmap.get(code)
            glyph = gset[gname] if gname and gname in gset else None
            kern = _get_kerning(ttfont, prev_gname, gname) if (auto_kern and prev_gname and gname) else 0.0
            
            contours = []
            width = 0.0
            if glyph is not None:
                width = float(glyph.width)
                if char not in " \t\r":
                    pen = OutlinePen(gset, lod=lod)
                    glyph.draw(pen)
                    pen._closePath()
                    contours = pen.contours

            gdata.append((global_char_idx, code, gname, width, kern, contours))
            cur_w += width + kern + (track_x * units_per_em)
            prev_gname = gname
            global_char_idx += 1

        # Account for trailing tracking removal on line width
        if line:
            cur_w -= (track_x * units_per_em)
        line_widths.append(max(0.0, cur_w))
        line_glyph_data.append(gdata)
        global_char_idx += 1  # count the newline

    max_line_w = max(line_widths) if line_widths else 0.0

    os2 = ttfont.get("OS/2")
    cap_height = float(getattr(os2, "sCapHeight", 0)) if os2 else 0.0
    top_y = cap_height if cap_height > 0 else ascender
    bot_y = -(num_lines - 1) * line_advance + (descender if use_descender else 0.0)

    if valign == "first_line":
        y_offset = 0.0
    elif valign == "top":
        y_offset = -top_y
    elif valign == "bottom":
        y_offset = -bot_y
    else:  # middle
        y_offset = -(top_y + bot_y) * 0.5

    # 3D Transform calculations
    em_scale = font_size / units_per_em
    tan_oblique = math.tan(math.radians(oblique))

    # Rotation matrix (Euler XYZ)
    R = euler_xyz_matrix(rotate)

    init_norm = np.array([0.0, 0.0, 1.0], dtype=np.float64)
    world_norm = (init_norm @ R.T).astype(np.float32)
    norm_len = np.linalg.norm(world_norm)
    if norm_len > 1e-12:
        world_norm /= norm_len

    tx, ty, tz = origin

    pts_list: List[Tuple[float, float, float]] = []
    vertex_points: List[int] = []
    prim_sizes: List[int] = []
    prim_symbol: List[int] = []
    prim_index: List[int] = []

    def _transform_pt(lx: float, ly: float) -> Tuple[float, float, float]:
        # Oblique slant
        ox = lx + ly * tan_oblique
        oy = ly
        # Scale
        sx_pt = ox * scale_xy[0] * em_scale
        sy_pt = oy * scale_xy[1] * em_scale
        # Rotate & Translate
        wx = sx_pt * R[0, 0] + sy_pt * R[0, 1] + tx
        wy = sx_pt * R[1, 0] + sy_pt * R[1, 1] + ty
        wz = sx_pt * R[2, 0] + sy_pt * R[2, 1] + tz
        return (wx, wy, wz)

    # Layout lines
    for line_idx, (gdata, cur_w) in enumerate(zip(line_glyph_data, line_widths)):
        baseline_y = y_offset - line_idx * line_advance

        # Horizontal alignment
        if halign == "left":
            pen_x = 0.0
            extra_space_per_gap = 0.0
        elif halign == "center":
            pen_x = -cur_w * 0.5
            extra_space_per_gap = 0.0
        elif halign == "right":
            pen_x = -cur_w
            extra_space_per_gap = 0.0
        else:  # justified
            space_count = sum(1 for item in gdata if chr(item[1]) == " ")
            if space_count > 0 and line_idx < num_lines - 1:
                pen_x = 0.0
                extra_space_per_gap = (max_line_w - cur_w) / space_count
            else:
                pen_x = 0.0
                extra_space_per_gap = 0.0

        for char_idx, code, gname, width, kern, contours in gdata:
            pen_x += kern

            if contours:
                if prim_type == "bezier":
                    # Emit closed polyline curves
                    for contour in contours:
                        base = len(pts_list)
                        for pt in contour:
                            pts_list.append(_transform_pt(pt[0] + pen_x, pt[1] + baseline_y))
                        n_c = len(contour)
                        vertex_points.extend(range(base, base + n_c))
                        prim_sizes.append(n_c)
                        prim_symbol.append(code)
                        prim_index.append(char_idx)
                else:
                    # Polygons: classify outer vs hole contours
                    raw_contours = []
                    for c in contours:
                        pts = np.array(c, dtype=np.float64)
                        area = _area2(list(range(len(pts))), pts)
                        if abs(area) > 1e-9:
                            raw_contours.append((pts, area))

                    if raw_contours:
                        raw_contours.sort(key=lambda item: -abs(item[1]))
                        outers: List[Tuple[np.ndarray, List[np.ndarray]]] = []

                        if hole_faces:
                            for pts, area in raw_contours:
                                is_hole = False
                                center = pts.mean(0)
                                for outer_pts, holes in outers:
                                    if _inside_poly(center, list(range(len(outer_pts))), outer_pts):
                                        h_area = _area2(list(range(len(pts))), pts)
                                        if h_area < 0:
                                            pts = pts[::-1]
                                        holes.append(pts)
                                        is_hole = True
                                        break
                                if not is_hole:
                                    o_area = _area2(list(range(len(pts))), pts)
                                    if o_area < 0:
                                        pts = pts[::-1]
                                    outers.append((pts, []))
                        else:
                            for pts, area in raw_contours:
                                o_area = _area2(list(range(len(pts))), pts)
                                if o_area < 0:
                                    pts = pts[::-1]
                                outers.append((pts, []))

                        # Triangulate
                        for outer_pts, holes in outers:
                            base = len(pts_list)
                            if not holes:
                                for pt in outer_pts:
                                    pts_list.append(_transform_pt(pt[0] + pen_x, pt[1] + baseline_y))
                                tris = earclip(list(range(len(outer_pts))), outer_pts)
                                for t in tris:
                                    vertex_points.extend([base + t[0], base + t[1], base + t[2]])
                                    prim_sizes.append(3)
                                    prim_symbol.append(code)
                                    prim_index.append(char_idx)
                            else:
                                all_pts = [outer_pts] + holes
                                offsets = np.cumsum([0] + [len(p) for p in all_pts])
                                xy = np.concatenate(all_pts, 0)
                                for pt in xy:
                                    pts_list.append(_transform_pt(pt[0] + pen_x, pt[1] + baseline_y))
                                poly = list(range(offsets[0], offsets[1]))
                                h_lists = [list(range(offsets[k], offsets[k + 1])) for k in range(1, len(all_pts))]
                                try:
                                    pieces = holes_into(poly, h_lists, xy)
                                    for p in pieces:
                                        for t in earclip(p, xy):
                                            vertex_points.extend([base + t[0], base + t[1], base + t[2]])
                                            prim_sizes.append(3)
                                            prim_symbol.append(code)
                                            prim_index.append(char_idx)
                                except Exception:
                                    for t in earclip(poly, xy):
                                        vertex_points.extend([base + t[0], base + t[1], base + t[2]])
                                        prim_sizes.append(3)
                                        prim_symbol.append(code)
                                        prim_index.append(char_idx)

            pen_x += width + (track_x * units_per_em)
            if chr(code) == " ":
                pen_x += extra_space_per_gap

    if not pts_list or not vertex_points:
        return B.make(np.zeros((0, 3), np.float32), np.zeros(0, np.int32), np.zeros(1, np.int32),
                      point={"N": np.zeros((0, 3), np.float32)})

    P = np.array(pts_list, dtype=np.float32)
    vp = np.array(vertex_points, dtype=np.int32)
    offsets = B.offsets_from_sizes(prim_sizes)
    N = np.tile(world_norm, (len(P), 1))
    point_dict = {"N": N}

    prim_dict = {}
    if add_attrs:
        prim_dict["textsymbol"] = np.array(prim_symbol, dtype=np.int32)
        prim_dict["textindex"] = np.array(prim_index, dtype=np.int32)

    closed = np.ones(len(prim_sizes), bool) if prim_type == "bezier" else None
    return B.make(P, vp, offsets, point=point_dict, prim=prim_dict, closed=closed)
