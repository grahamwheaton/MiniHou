#!/usr/bin/env python3
"""Compare MiniHou's nodes with SideFX's Houdini documentation: inputs, outputs, output groups and parameters.

    python tools/houdini_parity.py              # fetch (cached), compare, write docs/HOUDINI_PARITY.md and docs/houdini_parity_data.json
    python tools/houdini_parity.py --offline    # use the saved data only (no network)
    python tools/houdini_parity.py --refresh    # ignore the page cache

Every MiniHou node is mapped to the Houdini SOP it follows (``MAP``; ``None`` means MiniHou's own node with no Houdini equivalent). The
SOP's documentation page (https://www.sidefx.com/docs/houdini/nodes/sop/<name>.html) lists its inputs, its parameters (with the folder each
sits in) and, when it has several outputs, its outputs. What this prints is a checklist, not a verdict: a Houdini parameter counts as covered
when MiniHou has a parameter whose name matches its internal name or its label (spelling aside), so a differently named equivalent shows up
as missing and needs a human look.
"""
from __future__ import annotations

import argparse
import difflib
import html
import json
import os
import re
import sys
import urllib.request
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
DATA = ROOT / "docs" / "houdini_parity_data.json"
REPORT = ROOT / "docs" / "HOUDINI_PARITY.md"
CACHE = Path(os.environ.get("MINIHOU_HOUDINI_CACHE", ROOT / ".houdini_docs_cache"))
URL = "https://www.sidefx.com/docs/houdini/nodes/sop/{}.html"

# MiniHou node -> Houdini SOP slug(s), the first that exists wins. None: no Houdini equivalent (MiniHou's own node, or Blender / engine plumbing).
MAP = {
    "assemble": "assemble", "attrib_paint": "attribpaint", "attribute_blur": "attribblur", "attribute_create": "attribcreate",
    "attribute_delete": "attribdelete", "attribute_promote": "attribpromote", "attribute_randomize": "attribrandomize",
    "attribute_rename": "attribute", "attribute_transfer": "attribtransfer", "bend": "bend", "bezier": "curve", "blast": "blast",
    "blend": "blendshapes", "blender_image": None, "blender_object": None, "boolean": "boolean", "boolean_fracture": "booleanfracture",
    "bound": "bound", "box": "box", "bridge": "polybridge", "building_rules": None, "carve": "carve", "circle": "circle", "clean": "clean",
    "clip": "clip", "color": "color", "connectivity": "connectivity", "convert_line": "convertline",
    "convex_decomposition": "convexdecomposition", "convex_hull": None, "copy_to_points": "copytopoints",
    "copy_transform": "copyxform", "crease": "crease", "curve": "curve", "delaunay": "triangulate2d", "delete": "delete",
    "dissolve": "dissolve", "dissolve_edges": "dissolve", "divide": "divide", "edge_collapse": "edgecollapse", "edge_cusp": "edgecusp",
    "edge_flip": "edgeflip", "edge_group": "groupcreate", "edge_group_promote": "grouppromote", "exploded_view": "explodedview",
    "facet": "facet", "fillet": "fillet", "for_each": "block_begin", "fuse": "fuse", "glue_cluster": "gluecluster",
    "goal_collision": None, "goal_gravity": None, "goal_length": None, "goal_pin": None, "goal_solve": None, "goal_surface": None,
    "grid": "grid", "group_create": "groupcreate", "group_expand": "groupexpand", "group_promote": "grouppromote",
    "group_range": "grouprange", "heightfield_erode": "heightfield_erode", "helix": None, "image_file": None, "image_out": None,
    "input": None, "inset": "polyextrude", "instance": "instance", "lattice": "lattice", "lattice_grid": "lattice",
    "line": "line", "linear_taper": None, "loop_begin": "block_begin", "loop_end": "block_end", "lsystem": "lsystem",
    "match_size": "matchsize", "measure": "measure", "merge": "merge", "mirror": "mirror", "modifier_input": None, "noise": "attribnoise",
    "normal": "normal", "null": "null", "object_input": None, "output": "output", "outset": "polyextrude", "path_deform": "pathdeform",
    "peak": "peak", "point_deform": "pointdeform", "point_jitter": "pointjitter", "point_relax": "relax",
    "points_to_surface": "particlefluidsurface", "polybevel": "polybevel", "polybridge": "polybridge", "polyexpand2d": "polyexpand2d",
    "polyextrude": "polyextrude", "polyfill": "polyfill", "polypath": "polypath", "polyreduce": "polyreduce", "polysplit": "polysplit",
    "pushpull": None, "python": "python", "ray": "ray", "rbd_configure": "rbdconfigure", "rbd_constraint": "rbdconstraintsfromrules",
    "rbd_constraint_properties": "rbdconstraintproperties", "rbd_material_fracture": "rbdmaterialfracture", "rbd_solver": "rbdbulletsolver",
    "region_identify": None, "region_transform": None, "remesh": "remesh", "resample": "resample", "reverse": "reverse", "revolve": "revolve",
    "point_generate": "pointgenerate", "point_replicate": "pointreplicate", "point_velocity": "pointvelocity", "particle_trail": "particletrail",
    "cluster_points": "clusterpoints", "point_cloud_measure": "pointcloudmeasure", "point_cloud_normal": "pointcloudnormal",
    "point_cloud_reduce": "pointcloudreduce", "point_cloud_surface": "pointcloudsurface", "points_from_volume": "pointsfromvolume",
    "scatter_in_texture_mask": "scatterintexturemask",
    "scatter": "scatter", "scene_objects": None, "sdf_boolean": "vdbcombine", "set_material": "material", "skin": "skin",
    "skin_profiles": "profile", "smooth": "smooth", "soft_peak": "softpeak", "soft_transform": "softxform", "solver": "solver",
    "sort": "sort", "sphere": "sphere", "squash_stretch": None, "subdivide": "subdivide", "sweep": "sweep", "switch": "switch",
    "torus": "torus", "transform": "xform", "tube": "tube", "twist": "twist", "usd_export": "usdexport", "usd_import": "usdimport",
    "uv_flatten": "uvflatten", "uv_project": "uvproject", "voronoi_adjacency": "voronoiadjacency", "voronoi_fracture": "voronoifracture",
    "voronoi_fracture_points": "voronoifracturepoints", "voronoi_split": "voronoisplit", "wrangle": "attribwrangle",
}
for _name in [f"pcg_{n}" for n in ("bounds", "copy_points", "debug", "density_filter", "density_noise", "difference", "distance", "intersection",
                                    "points", "projection", "self_prune", "spawner", "spline_sampler", "surface_sampler", "transform", "union",
                                    "volume_sampler")]:
    MAP[_name] = None                                                       # Houdini has no PCG graph; MiniHou's PCG nodes follow Unreal's

norm = lambda s: re.sub(r"[^a-z0-9]", "", str(s).lower())                    # noqa: E731


def fetch(slug: str, refresh: bool = False):
    """The HTML of a SOP page, or None when SideFX has no such page."""
    CACHE.mkdir(parents=True, exist_ok=True)
    f = CACHE / f"{slug}.html"
    if f.exists() and not refresh:
        return f.read_text(encoding="utf-8", errors="replace") or None
    try:
        req = urllib.request.Request(URL.format(slug), headers={"User-Agent": "minihou-parity/1.0"})
        text = urllib.request.urlopen(req, timeout=40).read().decode("utf-8", "replace")
    except Exception:
        f.write_text("", encoding="utf-8")
        return None
    f.write_text(text, encoding="utf-8")
    return text


def _text(fragment: str) -> str:
    return re.sub(r"\s+", " ", html.unescape(re.sub(r"<[^>]+>", " ", fragment))).strip()


def _section(page: str, sid: str) -> str:
    m = re.search(r'<h2[^>]*id="%s"[^>]*>(.*?)(?=<h2 class="label section"|</main>|$)' % sid, page, flags=re.S)
    return m.group(1) if m else ""


HINT_RE = re.compile(r"[^.]{0,100}\b(?:second|third|fourth|2nd|3rd|4th|right|middle)\s+(?:topology\s+)?(?:input|output)s?\b[^.]{0,100}\.?|[^.]{0,100}\b(?:input|output)\s+[2-4]\b[^.]{0,100}\.?", re.I)


def input_hints(page: str) -> list:
    """Sentences of the page's own text (not the node index around it) that talk about a second/third/fourth input: for pages
    with no Inputs section they are the only trace of the node's inputs."""
    m = re.search(r"<main.*?</main>", page, flags=re.S)
    text = _text(m.group(0)) if m else ""
    seen = []
    for h in HINT_RE.finditer(text):
        s = h.group(0).strip()
        if s not in seen:
            seen.append(s[:200])
    return seen[:4]


def parse(page: str) -> dict:
    """{title, inputs: [{label, text}], outputs: [...], params: [{id, label, folder}]} of one SOP page."""
    title = _text(re.search(r"<h1[^>]*>(.*?)</h1>", page, flags=re.S).group(1)) if re.search(r"<h1[^>]*>", page) else ""
    out = {"title": title, "inputs": [], "outputs": [], "params": [], "has_inputs_section": 'id="inputs"' in page, "hints": input_hints(page)}
    for key in ("inputs", "outputs"):
        sec = _section(page, key)
        for m in re.finditer(r'<div id="([^"]*)" class="def[^"]*">\s*<p class="label[^"]*">(.*?)</p>\s*<div class="content">(.*?)</div>\s*</div>', sec, flags=re.S):
            out[key].append({"label": _text(m.group(2)), "text": _text(m.group(3))[:240]})
    sec = _section(page, "parameters")
    folder = ""
    # walk in order: titled separators (folders) and parameters
    pos = 0
    tokens = []
    for m in re.finditer(r'<div\s+class="sep\s+titled[^"]*">\s*<(?:div|p) class="label[^"]*">(.*?)</(?:div|p)>|<div id="([^"]*)" class="parameter[^"]*">\s*<p class="label[^"]*">(.*?)</p>', sec, flags=re.S):
        if m.group(1) is not None:
            tokens.append(("folder", _text(m.group(1))))
        else:
            tokens.append(("param", m.group(2), _text(m.group(3))))
    for t in tokens:
        if t[0] == "folder":
            folder = t[1]
        else:
            out["params"].append({"id": t[1], "label": t[2], "folder": folder})
    return out


# Why a difference listed in the report is still there (written by hand: the tool cannot know).
NOTES = {
    "attribute_blur": "Rest geometry (second input) fixes the weights on deforming meshes; MiniHou's blur is topological (uniform weights), so a rest input would change nothing.",
    "attribute_create": "'Variable names ending in 2' read the second input in VEX; the wrangle node has opinput1-3 for that.",
    "attribute_delete": "Reference geometry (second input) names the attributes to delete; not supported (type the names).",
    "bridge": "Spine from a curve on the second input is not supported (straight bridge with segments / twist).",
    "polybridge": "Spine from a curve on the second input is not supported (straight bridge with segments / twist).",
    "polyextrude": "Extrude along a spine curve (second input) is not supported.",
    "inset": "Extrude along a spine curve (second input) is not supported.",
    "outset": "Extrude along a spine curve (second input) is not supported.",
    "divide": "Rest geometry for convexing (second input) is not supported.",
    "skin": "The second input (V cross-sections, bilinear skin) is not supported: skin lofts one set of profiles.",
    "subdivide": "The second input (creases / topology subset) is not supported: use the crease node or the creases parameter.",
    "solver": "The extra inputs passed down into the solver network (2nd-4th) are not supported.",
    "rbd_configure": "A bounding-geometry fourth input is not supported.",
    "rbd_material_fracture": "Cutter geometry and input points on the fourth input are not supported.",
    "loop_end": "The second input (pieces to loop over) is not supported: for_each iterates pieces itself.",
    "usd_import": "No inputs here: MiniHou's importer reads a file path only.",
    "voronoi_fracture": "Houdini's second output (constraint geometry) is not an output here: a node has one output. glue_cluster and rbd_constraint make the constraints from the fractured pieces.",
    "boolean_fracture": "Same: constraint geometry comes from glue_cluster, not from a second output.",
    "voronoi_fracture_points": "Impact points come in on the second input; the optional SDF for depth sampling is not supported (depth is sampled from the geometry itself).",
    "polyreduce": "Rest, view, curve and closed-surface inputs (constraints on what may collapse) are not supported; keep_borders covers the common case.",
    "polysplit": "The projection curve (split along a curve drawn on the surface) is not supported; loops/position split by edge loop.",
    "remesh": "No rest geometry: the voxel remesher has no attributes to carry over.",
    "points_to_surface": "Collision and mask volumes are not supported.",
    "lattice_grid": "MiniHou's lattice is the lattice_grid/lattice pair (rest, deformed), not Houdini's one node with three sources.",
    "lsystem": "Leaf geometry inputs are not supported (the generated tree has no leaf instancing).",
    "rbd_constraint": "A manual link generator (kind, bodies, anchors), not Houdini's rule-based RBD Constraints From Rules.",
}


def load_minihou() -> dict:
    sys.path.insert(0, str(ROOT))
    from minihou import Evaluator
    reg = Evaluator().registry
    nodes = {}
    for name in sorted(reg.names()):
        nt = reg.get(name)
        nodes[name] = {
            "category": nt.category,
            "inputs": [{"name": i.name, "multi": bool(i.multi), "optional": bool(i.optional)} for i in nt.inputs],
            "outputs": [o.name for o in nt.outputs],
            "params": [p.name for p in nt.params],
            "handles": bool(getattr(nt, "handles", None)),
        }
    return nodes


def covered(hparam: dict, ours: list) -> str:
    """The MiniHou parameter that stands for a Houdini one (by internal name or label), or ''."""
    keys = {norm(hparam["id"]), norm(hparam["label"])}
    byn = {norm(p): p for p in ours}
    for k in keys:
        if k in byn:
            return byn[k]
    for k in keys:
        close = difflib.get_close_matches(k, list(byn), n=1, cutoff=0.86)
        if close:
            return byn[close[0]]
    for p, n in byn.items():                                                # a MiniHou name that contains the Houdini one (e.g. outgrp_pointfilletpolys)
        if len(norm(hparam["id"])) >= 6 and norm(hparam["id"]) in p:
            return n
    return ""


def compare(mh: dict, hou: dict, slug: str) -> dict:
    ours = mh["params"]
    hp = hou["params"]
    matched, missing = [], []
    for p in hp:
        c = covered(p, ours)
        (matched if c else missing).append({**p, "ours": c})
    groups_h = [p for p in hp if re.fullmatch(r"output groups?", p["folder"].strip().lower())]
    if len(groups_h) > 12:                  # the folder swallowed the parameters after it (tabs are not separators): keep what reads like a group
        groups_h = [p for p in groups_h if re.search(r"group|seam|inside|outside", p["label"], flags=re.I)]
    groups_ours = [p for p in ours if p.startswith("outgrp_") and not p.endswith("_name")]
    n_in_h = len(hou["inputs"])
    n_in_ours = sum(2 if i["multi"] else 1 for i in mh["inputs"])
    return {
        "slug": slug, "title": hou["title"], "inputs_houdini": hou["inputs"], "inputs_ours": mh["inputs"],
        "has_inputs_section": hou.get("has_inputs_section", True), "hints": hou.get("hints", []),
        "inputs_ok": n_in_ours >= n_in_h or (n_in_h <= 1 and n_in_ours >= 1),
        "outputs_houdini": hou["outputs"], "outputs_ours": mh["outputs"],
        "outputs_ok": len(mh["outputs"]) >= max(1, len(hou["outputs"])),
        "output_groups_houdini": [p["label"] for p in groups_h], "output_groups_ours": groups_ours,
        "params_total": len(hp), "params_matched": len(matched), "params_missing": [{"id": p["id"], "label": p["label"], "folder": p["folder"]} for p in missing],
    }


def run(offline: bool, refresh: bool) -> dict:
    mh = load_minihou()
    if offline:
        return json.loads(DATA.read_text(encoding="utf-8"))
    slugs = {}
    for name in mh:
        v = MAP.get(name, "?")
        slugs[name] = v if isinstance(v, (list, tuple)) else [v]
    wanted = sorted({s for v in slugs.values() for s in v if s and s != "?"})
    with ThreadPoolExecutor(8) as pool:
        pages = dict(zip(wanted, pool.map(lambda s: fetch(s, refresh), wanted)))
    result = {"nodes": {}, "unmapped": [n for n in mh if n not in MAP]}
    for name, spec in mh.items():
        cands = [s for s in slugs[name] if s and s != "?"]
        hit = next((s for s in cands if pages.get(s)), None)
        if name not in MAP:
            result["nodes"][name] = {"status": "unmapped"}
        elif not cands:
            result["nodes"][name] = {"status": "own", "category": spec["category"], "handles": spec["handles"]}
        elif not hit:
            result["nodes"][name] = {"status": "no-page", "tried": cands, "category": spec["category"], "handles": spec["handles"]}
        else:
            info = compare(spec, parse(pages[hit]), hit)
            info.update(status="ok", category=spec["category"], handles=spec["handles"])
            result["nodes"][name] = info
    DATA.write_text(json.dumps(result, indent=1, sort_keys=True), encoding="utf-8")
    return result


def report(res: dict) -> str:
    nodes = res["nodes"]
    ok = {n: v for n, v in nodes.items() if v["status"] == "ok"}
    own = [n for n, v in nodes.items() if v["status"] == "own"]
    nopage = {n: v for n, v in nodes.items() if v["status"] == "no-page"}
    lines = ["# Houdini parity: inputs, outputs and parameters", "",
             "Generated by `tools/houdini_parity.py` from SideFX's documentation (https://www.sidefx.com/docs/houdini/nodes/sop/). A Houdini parameter counts as",
             "covered when MiniHou has a parameter of the same (or nearly the same) internal name or label, so a differently named equivalent appears as missing:",
             "read the lists as a checklist (the percentage is a floor: it counts differently named equivalents, and Houdini's many options for features MiniHou does not have, as missing).",
             "A `?` for Houdini's inputs means the page has no Inputs section (SideFX documents it only in the overview), so nothing could be compared automatically.", "",
             f"{len(ok)} nodes follow a Houdini SOP; {len(own)} are MiniHou's own (no Houdini equivalent); {len(nopage)} point at a page SideFX does not have.", ""]
    tot = sum(v["params_total"] for v in ok.values())
    got = sum(v["params_matched"] for v in ok.values())
    lines += [f"Parameters: {got} of {tot} Houdini parameters covered ({round(100 * got / max(tot, 1))}%).", "",
              "## Summary", "", "| MiniHou node | Houdini SOP | inputs (ours / Houdini) | outputs (ours / Houdini) | output groups (ours / Houdini) | parameters | handles |",
              "|---|---|---|---|---|---|---|"]
    for n, v in sorted(ok.items()):
        ni_o = sum(1 for _ in v["inputs_ours"])
        ni_h = len(v["inputs_houdini"]) if v.get("has_inputs_section", True) else "?"
        flag = lambda b: "" if b else " (!)"                                      # noqa: E731
        lines.append(f"| `{n}` | [{v['title'] or v['slug']}](https://www.sidefx.com/docs/houdini/nodes/sop/{v['slug']}.html) | {ni_o} / {ni_h}{flag(v['inputs_ok'])} | "
                     f"{len(v['outputs_ours'])} / {max(len(v['outputs_houdini']), 1)}{flag(v['outputs_ok'])} | {len(v['output_groups_ours'])} / {len(v['output_groups_houdini'])} | "
                     f"{v['params_matched']} / {v['params_total']} | {'yes' if v['handles'] else ''} |")
    lines += ["", "## Inputs and outputs that differ", ""]
    for n, v in sorted(ok.items()):
        bad = []
        if not v["inputs_ok"]:
            bad.append("inputs: Houdini has " + "; ".join(f"**{i['label']}**" for i in v["inputs_houdini"]) + " - MiniHou has " + (", ".join(f"`{i['name']}`{'*' if i['multi'] else ''}" for i in v["inputs_ours"]) or "none"))
        if not v["outputs_ok"]:
            bad.append("outputs: Houdini has " + "; ".join(f"**{o['label']}**" for o in v["outputs_houdini"]) + " - MiniHou has " + ", ".join(f"`{o}`" for o in v["outputs_ours"]))
        hg, og = v["output_groups_houdini"], v["output_groups_ours"]
        if len(hg) > len(og):
            bad.append(f"output groups: Houdini has {len(hg)} ({', '.join(hg)}); MiniHou {len(og)}")
        if bad:
            lines.append(f"* `{n}` ({v['title']}): " + " | ".join(bad) + (f" **Note:** {NOTES[n]}" if n in NOTES else ""))
    lines += ["", "## Inputs only the overview text mentions", "",
              "Pages with no Inputs section, whose text talks about a second, third or fourth input. Compare with the MiniHou inputs by hand; a `Note` says what is known.", ""]
    for n, v in sorted(ok.items()):
        if v.get("hints") and not v.get("has_inputs_section", True):
            ours = ", ".join(f"`{i['name']}`{'*' if i['multi'] else ''}" for i in v["inputs_ours"]) or "none"
            lines.append(f"* `{n}` (MiniHou inputs: {ours}): " + " / ".join(f"\"{h}\"" for h in v["hints"][:2]) + (f" **Note:** {NOTES[n]}" if n in NOTES else ""))
    lines += ["", "## Parameters not found in MiniHou", ""]
    for n, v in sorted(ok.items(), key=lambda kv: -len(kv[1]["params_missing"])):
        if v["params_missing"]:
            miss = [f"{p['label'] or p['id']}" + (f" ({p['folder']})" if p["folder"] else "") for p in v["params_missing"]]
            lines.append(f"* `{n}` ({v['params_matched']}/{v['params_total']}): " + "; ".join(miss))
    lines += ["", "## MiniHou's own nodes (no Houdini SOP)", "", ", ".join(f"`{n}`" for n in sorted(own)) or "none", ""]
    if nopage:
        lines += ["## Mapped to a page SideFX does not have", ""] + [f"* `{n}`: tried {', '.join(v['tried'])}" for n, v in sorted(nopage.items())] + [""]
    return "\n".join(lines)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--offline", action="store_true")
    ap.add_argument("--refresh", action="store_true")
    a = ap.parse_args()
    res = run(a.offline, a.refresh)
    REPORT.write_text(report(res), encoding="utf-8")
    print(f"wrote {REPORT} and {DATA}")


if __name__ == "__main__":
    main()
