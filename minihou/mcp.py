"""MCP server: lets an AI assistant (Claude Code, Claude Desktop, any MCP client) build MiniHou graphs live.

Run as ``python -m minihou mcp`` (stdio transport, newline-delimited JSON-RPC 2.0; no dependencies beyond the
standard library). It talks to the running engine's HTTP API (:mod:`minihou.web.server`), the same API the browser
editor uses, so whatever the AI does appears immediately in the editor and in Blender, and the user can undo it.
If no engine is running, ``open_graph`` starts one.

Only protocol messages go to stdout; anything else (logs, warnings) goes to stderr.
"""
from __future__ import annotations

import http.client
import json
import logging
import os
import subprocess
import sys
import tempfile
import time
from pathlib import Path
from typing import Any, Dict, List, Optional
from urllib.parse import urlparse

log = logging.getLogger("minihou.mcp")

PROTOCOL_VERSIONS = ("2025-06-18", "2025-03-26", "2024-11-05")
REPO_ROOT = Path(__file__).resolve().parent.parent
DEFAULT_URL = "http://127.0.0.1:8766"

INSTRUCTIONS = """\
MiniHou is a small Houdini-style procedural geometry engine that live-links into Blender. A graph is a DAG of nodes
(grid, noise, scatter, copy_to_points, wrangle, ...); the node with the display flag is what Blender shows.

How to build a system:
1. engine_status. If no engine runs, open_graph(path, create=true) starts one (new graphs go in graphs/).
2. list_node_types once to learn the nodes, their inputs and parameters (names, kinds, defaults, ranges).
3. edit_graph with a batch of ops: add nodes (give readable ids like "terrain", "rocks"), connect them, set params,
   set the display node, expose the parameters an artist should tweak in Blender, add short comments. A batch is
   atomic; the reply tells you whether the cook succeeded, which node failed and why, and point/prim counts.
4. inspect_node to check geometry (counts, bounds, attributes with ranges) at any node, then iterate.
Use wrangle nodes (wrangle_reference) for custom per-point math; for anything more (new topology, several inputs,
loops over geometry) write a node-library file (node_library). A `python` node (Python code inside the graph, with inputs
geo/input1..3 and variables a b c d i j v s flag) exists too, but it only runs when the USER started the engine with
--allow-python; engine_status says whether it is on. Never ask to turn it on yourself, and prefer wrangles or
node-library files. To make something reusable, build it as a *system*: a graph
saved in nodes/ (e.g. nodes/rock_scatter.json) with `input` nodes and exposed parameters becomes a node type that any
graph can use (details in node_library). Houdini-style toolkit: groups (group_create, group_expand; most nodes take a group pattern
like 'top ^side', '0-9' or '@P.z > 0'), modelling (boolean, polyextrude, polybevel, solidify, subdivide, clip, mirror, polyfill, delaunay, convex_hull, dissolve, skin, revolve, bridge;
tube and torus sources; smooth, peak, match_size, bound, copy_transform, switch; voronoi_fracture + exploded_view;
polyreduce; normal with domain 'vertex' keeps hard edges crisp), attributes (attribute_transfer, promote, blur,
measure, color, ray, falloff (0..1 weights), occlusion (AO / thickness); uv_project, uv_flatten, uv_transform), curves (line, helix, resample,
curve_simplify, contour, slice, sweep, path_deform), volumes (remesh, sdf_shape,
points_to_surface, sdf_boolean), instancing of Blender objects (instance), PCG point graphs (pcg_* nodes: samplers,
point booleans pcg_difference/intersection/union, pcg_self_prune, density noise/filter, pcg_spawner), for_each (a
system per piece), time (@Frame, solver; set_frame to check).
To work on something the user made in Blender, start from a blender_object node
(the object's name; its UV map, color attribute, vertex groups and material indices arrive as attributes, its material slot
names as the detail list 'materials', and it recooks live when they edit it); inspect_node on it shows what was read.
Curves, hair curves, point clouds and images also come in (blender_object, blender_image) and go out (scene_objects modes curves/points,
image_out). Give faces a material with set_material (a group and a name): Blender gets the slots by name, USD gets bound materials. Attributes can be painted by hand with an attrib_paint node (paint_assign op; the user can also paint it in the 3-D view). USD files come in with usd_import (a Primitives pattern, purposes, time offset and scale) and go out with usd_export (press_button save; cooking itself never writes). Z is up; units are Blender metres. The user is
watching the editor and Blender live and can undo your edits, so prefer small verified steps over huge batches."""

OPS_DOC = """\
Each op is an object with "op":
  add        {"op":"add","type":T,"id"?:str,"params"?:{..},"inputs"?:{port:id or [ids]},"pos"?:[x,y],"display"?:true,"comment"?:str}
  insert     {"op":"insert","after":id,"type":"polybevel","params"?:{...}}  (a node fed by `after` that takes over everything which
             read `after`: joins a chain in place, display flag included; "port"? picks its input)
  remove     {"op":"remove","id":id or [ids]}
  rename     {"op":"rename","id":old,"to":new}
  connect    {"op":"connect","from":src,"to":dst,"port"?:name,"index"?:int}  (port defaults to the first free input)
  disconnect {"op":"disconnect","to":dst,"port"?:name,"from"?:src}
  set        {"op":"set","id":id,"params":{name:value}}  (null resets to default; vec3 = [x,y,z])
  paint_assign {"op":"paint_assign","id":paint_node,"indices":[0,5,9],"value":0.5 | "values":[..] | "erase":true,"clear"?:true,"default"?:v}
             write painted values into an attrib_paint node (the viewer's Paint mode, Quick Assign and spreadsheet use this): element numbers are
             point / primitive / derived-edge numbers as inspect_node shows them; erase sends them back to the input's value; clear forgets all
  display    {"op":"display","id":id}  (what Blender shows; usually an "output" node). More results: set an extra "output" node's
             "object" param (e.g. "Trees") and it appears as its own Blender object, live
  expose     {"op":"expose","id":id,"param":name,"name"?:"Label in Blender"}  (float/int/bool/vec3 only)
  unexpose   {"op":"unexpose","name":label} or {"op":"unexpose","id":id,"param":name}
  move       {"op":"move","id":id,"pos":[x,y]} or {"op":"move","positions":{id:[x,y]}}  (layout is automatic if omitted)
  meta       {"op":"meta","id":id,"set":{"comment":"..."}}
  replace    {"op":"replace","graph":{...whole graph JSON...}}
  layout     {"op":"layout"}  (tidy all node positions; put it last in a batch that added many nodes)
  loop       {"op":"loop","from"?:src,"to"?:dst,"port"?:name,"iterations"?:int,"pos"?:[x,y]} (paired Begin/Transform/End)

  box_add    {"op":"box_add","around":[ids],"title"?:"Terrain","color"?:"#5b7fb5"} - a labelled network box behind those nodes
             (or "pos":[x,y],"size":[w,h]); box_set {"op":"box_set","id":"box1","title"?,"color"?,"pos"?,"size"?}; box_remove {"op":"box_remove","id":id}
Nodes flow top to bottom: each node's single output feeds inputs of nodes below it. Ids: letters, digits, _."""

TOOLS: List[dict] = [
    {"name": "engine_status",
     "description": "Is the MiniHou engine running? Shows the open graph file, the latest cook result, whether Blender "
                    "is connected, and the browser editor URL.",
     "inputSchema": {"type": "object", "properties": {}, "additionalProperties": False}},
    {"name": "list_node_types",
     "description": "Describe node types: inputs, parameters (kind, default, range, meaning) and what each node does.",
     "inputSchema": {"type": "object", "properties": {
         "names": {"type": "array", "items": {"type": "string"}, "description": "Only these types (default: all)."}},
         "additionalProperties": False}},
    {"name": "get_graph",
     "description": "The current graph as JSON (nodes with params and inputs, display node, exposed parameters) "
                    "plus which nodes feed the display node.",
     "inputSchema": {"type": "object", "properties": {
         "include_layout": {"type": "boolean", "description": "Include node positions (default false)."}},
         "additionalProperties": False}},
    {"name": "edit_graph",
     "description": "Apply a batch of edit operations atomically (all or nothing), wait for the cook, and report the "
                    "result: created ids, cook success or the failing node and message, point/prim counts.\n" + OPS_DOC,
     "inputSchema": {"type": "object", "required": ["ops"], "additionalProperties": False, "properties": {
         "ops": {"type": "array", "minItems": 1, "description": "Edit operations, applied in order.",
                 "items": {"type": "object", "required": ["op"], "properties": {
                     "op": {"type": "string", "enum": ["add", "insert", "remove", "rename", "connect", "disconnect", "set",
                                                       "display", "expose", "unexpose", "move", "meta", "replace",
                                                       "layout", "loop", "box_add", "box_set", "box_remove"]}}}}}}},
    {"name": "inspect_node",
     "description": "Cook any node and summarise its geometry: point/prim/vertex counts, bounds, and attributes "
                    "(with min/max/mean and first values when detail is true). Use it to verify each step.",
     "inputSchema": {"type": "object", "required": ["node"], "additionalProperties": False, "properties": {
         "node": {"type": "string"}, "detail": {"type": "boolean", "description": "Attribute value ranges (default true)."}}}},
    {"name": "set_frame",
     "description": "Set the current frame (what @Frame / @Time read; Blender's timeline also sets it). Use it to check "
                    "animated systems (wrangles using @Frame, solvers) at different times, then inspect_node.",
     "inputSchema": {"type": "object", "required": ["frame"], "additionalProperties": False, "properties": {
         "frame": {"type": "number"}, "fps": {"type": "number", "exclusiveMinimum": 0}}}},
    {"name": "export_usd",
     "description": "Write the cooked geometry to a USD file (.usd/.usda/.usdc, inside the MiniHou folder) for Houdini, Blender, "
                    "Omniverse or usdview. Always full size, whatever the proxy level. The display output and each extra output "
                    "become prims under /MiniHou (meshes, curves and points, with attributes as primvars and groups as GeomSubsets); "
                    "the graph is stored in the file. Read a USD file back with the usd_import node (or write one from a graph with a usd_export node and press_button save).",
     "inputSchema": {"type": "object", "required": ["path"], "additionalProperties": False, "properties": {
         "path": {"type": "string"}, "node": {"type": "string", "description": "Export this node instead of the display output."},
         "overwrite": {"type": "boolean"}, "graph": {"type": "boolean", "description": "Store the graph in the file (default true)."}}}},
    {"name": "press_button",
     "description": "Press a button of a node, the way the editor's parameter panel does. usd_export: `save` writes its USD file "
                    "(settings are the node's parameters: file, format, root, frames current or range, up_axis, units, materials, "
                    "embed_graph, overwrite; cooking never writes, only this does). usd_import: `reload` reads the file again. "
                    "list_node_types shows each node's buttons.",
     "inputSchema": {"type": "object", "required": ["node", "action"], "additionalProperties": False, "properties": {
         "node": {"type": "string"}, "action": {"type": "string", "description": "e.g. save, reload"}}}},
    {"name": "set_proxy",
     "description": "Work on a very large scene: cook it at a fraction of its size. Node parameters that set the amount of work "
                    "(grid columns/rows, scatter count, sphere bands, voxel size, spacing...) scale with the level, so a 10% "
                    "level cooks about a tenth of the points. auto=true cooks the small version first on slow scenes and the "
                    "real one when edits pause. Remember to set level back to 1 (full) before judging final results. "
                    "link_tris caps the triangles sent to Blender (0 = everything). No arguments: show the settings.",
     "inputSchema": {"type": "object", "additionalProperties": False, "properties": {
         "level": {"type": "number", "minimum": 0.001, "maximum": 1}, "auto": {"type": "boolean"},
         "link_tris": {"type": "integer", "minimum": 0}}}},
    {"name": "undo",
     "description": "Undo the last edit(s) (yours or the user's); redo=true goes forward again.",
     "inputSchema": {"type": "object", "additionalProperties": False, "properties": {
         "steps": {"type": "integer", "minimum": 1, "maximum": 50}, "redo": {"type": "boolean"}}}},
    {"name": "list_graphs",
     "description": "Graph files in the workspace (examples/graphs has worked examples worth reading).",
     "inputSchema": {"type": "object", "properties": {}, "additionalProperties": False}},
    {"name": "open_graph",
     "description": "Open a graph file (path relative to the MiniHou folder), or create a starter graph with "
                    "create=true. Starts the engine if it is not running. New systems usually go in graphs/<name>.json.",
     "inputSchema": {"type": "object", "required": ["path"], "additionalProperties": False, "properties": {
         "path": {"type": "string"}, "create": {"type": "boolean"}}}},
    {"name": "node_library",
     "description": "Node libraries: Python files of @node functions the engine hot-loads as new node types (for "
                    "anything a wrangle can't do: new topology, multi-input logic, simulations). Lists the node "
                    "folder, loaded files, their node types and any load errors (file and line), plus a guide to "
                    "writing a node. Write the file with your own file tools; it is live within a second.",
     "inputSchema": {"type": "object", "properties": {}, "additionalProperties": False}},
    {"name": "wrangle_reference",
     "description": "The wrangle snippet language (VEX-like expressions over points, primitives or vertices): syntax, "
                    "attributes, functions, and which names become native Blender data (uv, Cd, N, material_index).",
     "inputSchema": {"type": "object", "properties": {}, "additionalProperties": False}},
]


class ToolError(Exception):
    """Reported to the model as a tool result with isError=true."""


class EngineDown(ToolError):
    pass


class EngineClient:
    """Minimal HTTP client for the engine API (http.client: never goes through an HTTP proxy)."""

    def __init__(self, url: str = DEFAULT_URL):
        u = urlparse(url if "://" in url else "http://" + url)
        self.host, self.port = u.hostname or "127.0.0.1", u.port or 8766
        self.url = f"http://{self.host}:{self.port}/"

    def request(self, method: str, path: str, body: Optional[dict] = None, timeout: float = 60.0) -> Any:
        conn = http.client.HTTPConnection(self.host, self.port, timeout=timeout)
        try:
            data = json.dumps(body).encode("utf-8") if body is not None else None
            headers = {"Content-Type": "application/json"} if body is not None else {}
            conn.request(method, path, body=data, headers=headers)
            r = conn.getresponse()
            raw = r.read()
        except TimeoutError:
            raise ToolError(f"the engine did not answer within {timeout:.0f} s (busy with a very heavy cook?)") from None
        except OSError as e:
            raise EngineDown(f"the MiniHou engine is not reachable at {self.url} ({e.__class__.__name__}). "
                             "Start it with open_graph(path), or `python -m minihou serve graph.json`.") from None
        finally:
            conn.close()
        try:
            out = json.loads(raw.decode("utf-8"))
        except ValueError:
            raise ToolError(f"engine returned a non-JSON reply (HTTP {r.status})") from None
        if r.status >= 400:
            msg = out.get("error", f"HTTP {r.status}") if isinstance(out, dict) else f"HTTP {r.status}"
            raise ToolError(msg)
        return out

    def get(self, path: str, timeout: float = 60.0) -> Any:
        return self.request("GET", path, timeout=timeout)

    def post(self, path: str, body: dict, timeout: float = 120.0) -> Any:
        return self.request("POST", path, body, timeout=timeout)

    def alive(self) -> bool:
        try:
            self.get("/api/status", timeout=2.0)
            return True
        except ToolError:
            return False


# ---------------------------------------------------------------------------
# formatting helpers
# ---------------------------------------------------------------------------
def _fmt_param(p: dict) -> str:
    rng = []
    for k, label in (("min", "min"), ("max", "max"), ("soft_min", "soft min"), ("soft_max", "soft max")):
        if k in p:
            rng.append(f"{label} {p[k]}")
    s = f"    {p['name']}: {p['kind']} = {json.dumps(p['default'])}"
    if p.get("choices"):
        s += f" (one of {', '.join(p['choices'])})"
    if rng:
        s += f" [{', '.join(rng)}]"
    if p.get("ui") == "code":
        s += " (code)"
    if p.get("doc"):
        s += " - " + " ".join(p["doc"].split())
    return s


def format_node_types(types: List[dict]) -> str:
    out = []
    for t in types:
        ins = ", ".join(i["name"] + (" (any number)" if i.get("multi") else "") + (" (optional)" if i.get("optional") else "")
                        for i in t["inputs"]) or "none (a source)"
        src = f" (from {t['source']})" if t.get("source") else ""
        out.append(f"{t['name']} [{t.get('category') or 'other'}]{src}\n  {' '.join((t.get('doc') or '').split())}\n  inputs: {ins}")
        if t.get("actions"):
            out.append("  buttons (press_button): " + "; ".join(f"{a['name']} - {a['doc'] or a['label']}" for a in t["actions"]))
        if t["params"]:
            out.append("  params:\n" + "\n".join(_fmt_param(p) for p in t["params"] if p.get("ui") != "hidden"))
    return "\n\n".join(out)


def _upstream(graph: dict) -> List[str]:
    nodes = graph.get("nodes", {})
    disp = graph.get("display") or next((k for k, v in nodes.items() if v.get("type") == "output"), None)
    seen, stack = set(), [disp] if disp else []
    while stack:
        nid = stack.pop()
        if nid in seen or nid not in nodes:
            continue
        seen.add(nid)
        for ref in (nodes[nid].get("inputs") or {}).values():
            stack.extend(ref if isinstance(ref, list) else [ref])
    return [n for n in nodes if n in seen]


def format_status(st: dict, graph: Optional[dict] = None) -> str:
    if st.get("ok") is False:
        err = st.get("error") or {}
        where = f" at node '{err['node']}'" if err.get("node") else ""
        s = f"COOK ERROR{where}: {err.get('message')}"
    elif st.get("ok"):
        cook = st.get("cook") or {}
        s = (f"Cook OK: {st.get('points', 0):,} points, {st.get('prims', 0):,} prims"
             + (f", {cook.get('ms', 0):.1f} ms" if cook else ""))
    else:
        s = "Not cooked yet."
    px = (st.get("proxy") or {})
    if px.get("cooked", 1.0) < 1.0:
        s += f" [PROXY: cooked at {px['cooked']:.1%} of full size]"
    if st.get("busy"):
        s += " (still cooking newer edits)"
    if graph is not None:
        disp = graph.get("display")
        live = set(_upstream(graph))
        unused = [n for n in graph.get("nodes", {}) if n not in live]
        s += f"\nDisplay node: {disp or '(none: first output node)'}"
        if unused:
            s += f"\nNodes not feeding the display node: {', '.join(unused)}"
    return s


def _strip_layout(graph: dict) -> dict:
    g = json.loads(json.dumps(graph))
    for n in g.get("nodes", {}).values():
        n.pop("pos", None)
    return g


# ---------------------------------------------------------------------------
# the server
# ---------------------------------------------------------------------------
class JsonRpcError(Exception):
    def __init__(self, code: int, message: str):
        super().__init__(message)
        self.code, self.message = code, message


class McpServer:
    def __init__(self, engine: EngineClient, workspace: Path = REPO_ROOT, link_port: int = 8765):
        self.engine = engine
        self.link_port = link_port
        self.workspace = Path(workspace).resolve()
        self.initialized = False
        self.engine_process: Optional[subprocess.Popen] = None

    # -- JSON-RPC ------------------------------------------------------------
    def handle(self, msg: Any) -> Optional[dict]:
        if not isinstance(msg, dict) or msg.get("jsonrpc") != "2.0":
            return _error(msg.get("id") if isinstance(msg, dict) else None, -32600, "invalid request")
        if "method" not in msg:
            return None                                  # a response to something we never send
        mid, method = msg.get("id"), msg["method"]
        notification = "id" not in msg
        params = msg.get("params") or {}
        try:
            if method == "initialize":
                result = self._initialize(params)
            elif method == "ping":
                result = {}
            elif method == "tools/list":
                result = {"tools": TOOLS}
            elif method == "tools/call":
                result = self.call_tool(params.get("name"), params.get("arguments") or {})
            elif method.startswith("notifications/"):
                if method == "notifications/initialized":
                    self.initialized = True
                return None
            else:
                raise JsonRpcError(-32601, f"method not found: {method}")
        except JsonRpcError as e:
            return None if notification else _error(mid, e.code, e.message)
        except Exception as e:  # noqa: BLE001 - never crash the server on one bad request
            log.exception("request %s failed", method)
            return None if notification else _error(mid, -32603, f"internal error: {e!r}")
        return None if notification else {"jsonrpc": "2.0", "id": mid, "result": result}

    def _initialize(self, params: dict) -> dict:
        asked = params.get("protocolVersion")
        version = asked if asked in PROTOCOL_VERSIONS else PROTOCOL_VERSIONS[0]
        from . import __version__
        return {"protocolVersion": version, "capabilities": {"tools": {"listChanged": False}},
                "serverInfo": {"name": "minihou", "title": "MiniHou procedural geometry", "version": __version__},
                "instructions": INSTRUCTIONS}

    # -- tools -----------------------------------------------------------------
    def call_tool(self, name: Any, args: dict) -> dict:
        fn = getattr(self, f"tool_{name}", None) if isinstance(name, str) else None
        if fn is None or not any(t["name"] == name for t in TOOLS):
            raise JsonRpcError(-32602, f"unknown tool: {name!r}")
        if not isinstance(args, dict):
            raise JsonRpcError(-32602, "arguments must be an object")
        try:
            text = fn(**args)
            return {"content": [{"type": "text", "text": text}], "isError": False}
        except TypeError as e:
            return {"content": [{"type": "text", "text": f"bad arguments for {name}: {e}"}], "isError": True}
        except ToolError as e:
            return {"content": [{"type": "text", "text": str(e)}], "isError": True}

    def tool_engine_status(self) -> str:
        try:
            st = self.engine.get("/api/state")
        except EngineDown as e:
            return (f"Engine not running ({self.engine.url}). Call open_graph(path) to start it on a graph file, "
                    f"e.g. open_graph('graphs/my_system.json', create=true).\n{e}")
        clients = st["link"]["clients"]
        lib = st.get("library") or {}
        bad = [f for f in lib.get("files", []) if f["error"]]
        lines = [f"Engine running (pid {st['engine']['pid']}, minihou {st['engine']['version']}).",
                 f"Graph file: {st.get('path') or '(none)'}  (rev {st['rev']}, {len(st['graph']['nodes'])} nodes)",
                 format_status(st["status"], st["graph"]),
                 f"Blender: {'connected (' + ', '.join(clients) + ')' if clients else 'not connected'}",
                 f"Editor (for the user): {self.engine.url}",
                 "Python nodes: " + ("enabled (the user allowed them)" if st["engine"].get("python_nodes")
                                    else "off (the engine was not started with --allow-python)")]
        dc = (st.get("status") or {}).get("disk_cache")
        if dc:
            lines.append(f"Disk cache: {dc['bytes'] / 1e6:,.0f} MB of {dc['max_bytes'] / 1e9:g} GB in {dc['folder']}; {dc['hits']} hits, "
                         f"{dc['writes']} results written")
        if lib:
            lines.append(f"Node library: {sum(len(f['nodes']) for f in lib['files'])} custom node type(s) from "
                         f"{len(lib['files'])} file(s)" + "".join(f"\n  ERROR in {f['file']}: {f['error']}" for f in bad))
        return "\n".join(lines)

    def tool_list_node_types(self, names: Optional[List[str]] = None) -> str:
        try:
            types = self.engine.get("/api/nodes")
        except EngineDown:
            from . import REGISTRY                          # the engine isn't running: describe the local build
            types = [nt.describe() for nt in REGISTRY]
        if names:
            known = {t["name"] for t in types}
            missing = [n for n in names if n not in known]
            if missing:
                raise ToolError(f"unknown node type(s) {missing}; known: {sorted(known)}")
            types = [t for t in types if t["name"] in names]
        return format_node_types(types)

    def tool_get_graph(self, include_layout: bool = False) -> str:
        snap = self.engine.get("/api/graph")
        st = self.engine.get("/api/status")
        graph = snap["graph"] if include_layout else _strip_layout(snap["graph"])
        return (f"File: {snap.get('path')}  rev {snap['rev']}\n{format_status(st, snap['graph'])}\n\n"
                + json.dumps(graph, indent=1))

    def tool_edit_graph(self, ops: List[dict]) -> str:
        if not isinstance(ops, list) or not ops:
            raise ToolError("ops must be a non-empty list of edit operations")
        r = self.engine.post("/api/edit", {"ops": ops, "wait": True, "timeout": 60, "origin": "mcp"})
        created = [res["id"] for res, op in zip(r["results"], ops) if op.get("op") in ("add", "rename") and "id" in res]
        exposed = [res["name"] for res, op in zip(r["results"], ops) if op.get("op") == "expose"]
        head = f"Applied {len(ops)} op(s), graph rev {r['rev']}." if r["changed"] else "No change (graph already like that)."
        if created:
            head += f" Nodes: {', '.join(created)}."
        if exposed:
            head += f" Exposed to Blender as: {', '.join(exposed)}."
        tail = ""
        if r["status"].get("ok") is False:
            tail = "\nThe edit was kept; fix it with another edit_graph call, or call undo."
        return head + "\n" + format_status(r["status"], r["graph"]["graph"]) + tail

    def tool_inspect_node(self, node: str, detail: bool = True) -> str:
        r = self.engine.post("/api/inspect", {"node": node, "detail": detail})
        if not r.get("ok"):
            err = r.get("error") or {}
            where = f" (failing node: {err['node']})" if err.get("node") and err.get("node") != node else ""
            raise ToolError(f"cannot cook '{node}'{where}: {err.get('message')}")
        r = {k: v for k, v in r.items() if k not in ("ok", "rev", "cooked", "cached")}
        return json.dumps(r, indent=1)

    def tool_undo(self, steps: int = 1, redo: bool = False) -> str:
        done = 0
        for _ in range(max(1, min(int(steps), 50))):
            r = self.engine.post("/api/redo" if redo else "/api/undo", {"origin": "mcp"})
            if not r["ok"]:
                break
            done += 1
        st = self.engine.get("/api/status?wait=1&timeout=30")
        word = "Redid" if redo else "Undid"
        return f"{word} {done} step(s)." + (" Nothing (more) to do." if done < steps else "") + "\n" + format_status(st)

    def tool_export_usd(self, path: str, node: Optional[str] = None, overwrite: bool = False, graph: bool = True) -> str:
        body = {"path": path, "overwrite": overwrite, "graph": graph}
        if node:
            body["node"] = node
        r = self.engine.post("/api/export_usd", body)
        lines = [f"Wrote {r['path']}"]
        for name, o in r["outputs"].items():
            d = o["dropped"]
            lines.append(f"  {o['prim']}: {o['faces']:,} faces, {o['curves']:,} curves, {o['points']:,} loose points"
                         + (f", {o['instances']:,} instances" if o.get("instances") else "")
                         + (f" (left out: {d['faces']} faces with under 3 corners, {d['curves']} one-point curves)" if d["faces"] or d["curves"] else ""))
        return "\n".join(lines)

    def tool_press_button(self, node: str, action: str) -> str:
        r = self.engine.post("/api/node_action", {"node": node, "action": action})
        return r.get("message") or f"Pressed {action} on {node}."

    def tool_set_proxy(self, level: Optional[float] = None, auto: Optional[bool] = None, link_tris: Optional[int] = None) -> str:
        body = {k: v for k, v in (("level", level), ("auto", auto), ("link_tris", link_tris)) if v is not None}
        info = self.engine.post("/api/proxy", body) if body else self.engine.get("/api/proxy")
        if body:
            self.engine.get("/api/status?wait=1")
            info = self.engine.get("/api/proxy")
        what = "full resolution" if info["level"] >= 1 and not info["auto"] else (
            f"auto (small version at {info['level'] if info['level'] < 1 else 0.1:.0%} first, full when edits pause)" if info["auto"]
            else f"{info['level']:.1%} of full size")
        return (f"Proxy: {what}; latest result cooked at {info['cooked']:.1%}. Blender triangle cap: "
                f"{info['link_tris'] or 'none'}.")

    def tool_set_frame(self, frame: float, fps: Optional[float] = None) -> str:
        body = {"frame": frame}
        if fps is not None:
            body["fps"] = fps
        r = self.engine.post("/api/frame", body)
        st = self.engine.get("/api/status?wait=1")
        return f"Frame {r['frame']:g} at {r['fps']:g} fps.\n" + format_status(st)

    def tool_list_graphs(self) -> str:
        try:
            r = self.engine.get("/api/files")
            files, current = r["files"], r.get("current")
            root = Path(r["root"])
        except EngineDown:
            root, current = self.workspace, None
            files = sorted(p.relative_to(root).as_posix() for d in ("examples/graphs", "graphs")
                           for p in (root / d).glob("*.json") if (root / d).is_dir())
        cur = None
        if current:
            try:
                cur = Path(current).resolve().relative_to(root.resolve()).as_posix()
            except ValueError:
                cur = current
        lines = [f"Workspace: {root}"] + [f"{'* ' if f == cur else '  '}{f}" for f in files]
        return "\n".join(lines) + ("\n(* = open)" if cur else "")

    def tool_open_graph(self, path: str, create: bool = False) -> str:
        if not isinstance(path, str) or not path.strip():
            raise ToolError("path must be a file path like graphs/my_system.json")
        if not path.lower().endswith(".json"):
            path += ".json"
        p = Path(path)
        target = (p if p.is_absolute() else self.workspace / p).resolve()
        if not self.engine.alive():
            if not target.exists():
                if not create:
                    raise ToolError(f"no such graph file: {target} (pass create=true to make a new one)")
                from .link.session import NEW_GRAPH
                target.parent.mkdir(parents=True, exist_ok=True)
                target.write_text(json.dumps(NEW_GRAPH, indent=2) + "\n", encoding="utf-8")
            started = self._start_engine(target)
            return started + "\n" + self.tool_engine_status()
        self.engine.post("/api/open", {"path": str(target), "create": bool(create)})
        self.engine.get("/api/status?wait=1&timeout=30")
        return f"Opened {target}.\n" + self.tool_engine_status()

    def tool_node_library(self) -> str:
        from .library import AUTHORING_GUIDE
        try:
            lib = self.engine.get("/api/state").get("library")
        except EngineDown:
            lib = None
        if lib:
            lines = [f"Node folder(s): {', '.join(lib['dirs'])}"]
            for f in lib["files"]:
                lines.append(f"  {f['file']}: " + (f"ERROR {f['error']}" if f["error"] else ", ".join(f["nodes"])))
            if not lib["files"]:
                lines.append("  (no node files yet)")
        else:
            lines = [f"Engine not running; its node folder will be {self.workspace / 'nodes'}"]
        return "\n".join(lines) + "\n\n" + AUTHORING_GUIDE

    def tool_wrangle_reference(self) -> str:
        from .wrangle import reference
        return reference()

    # -- engine process --------------------------------------------------------
    def _start_engine(self, graph: Path) -> str:
        logf = Path(tempfile.gettempdir()) / "minihou_engine.log"
        cmd = [sys.executable, "-m", "minihou", "serve", str(graph), "--http-port", str(self.engine.port),
               "--port", str(self.link_port), "--root", str(self.workspace)]
        kw: Dict[str, Any] = {"cwd": str(REPO_ROOT), "stdin": subprocess.DEVNULL, "close_fds": True}
        if os.name == "nt":
            # its own console window: the user sees the engine and can close it with Ctrl+C
            kw["creationflags"] = subprocess.CREATE_NEW_CONSOLE | subprocess.CREATE_NEW_PROCESS_GROUP
        else:
            kw["start_new_session"] = True
            kw["stdout"] = kw["stderr"] = open(logf, "a", encoding="utf-8")
        env = dict(os.environ)
        env["PYTHONPATH"] = str(REPO_ROOT) + os.pathsep + env.get("PYTHONPATH", "")
        try:
            self.engine_process = subprocess.Popen(cmd, env=env, **kw)
        finally:
            if "stdout" in kw:
                kw["stdout"].close()
        end = time.monotonic() + 30
        while time.monotonic() < end:
            code = self.engine_process.poll()
            if code is not None:
                tail = ""
                if os.name != "nt":
                    try:
                        tail = "\n" + "".join(logf.read_text(encoding="utf-8", errors="replace").splitlines(True)[-6:])
                    except OSError:
                        pass
                raise ToolError(f"the engine exited at once (code {code}); is another engine using port "
                                f"{self.link_port} or {self.engine.port}?{tail}")
            if self.engine.alive():
                self.engine.get("/api/status?wait=1&timeout=30")
                where = "in its own console window" if os.name == "nt" else f"in the background (log: {logf})"
                return f"Started the MiniHou engine on {graph} {where}."
            time.sleep(0.25)
        raise ToolError(f"the engine did not start within 30 s; command: {' '.join(cmd)}"
                        + (f"; see {logf}" if os.name != "nt" else ""))


def _error(mid, code: int, message: str) -> dict:
    return {"jsonrpc": "2.0", "id": mid, "error": {"code": code, "message": message}}


def serve_stdio(server: McpServer, stdin=None, stdout=None) -> None:
    """Newline-delimited JSON-RPC over stdin/stdout (UTF-8, whatever the console code page is)."""
    stdin = stdin or sys.stdin.buffer
    stdout = stdout or sys.stdout.buffer

    def write(obj):
        stdout.write(json.dumps(obj, ensure_ascii=False, separators=(",", ":")).encode("utf-8") + b"\n")
        stdout.flush()

    for raw in stdin:
        raw = raw.strip()
        if not raw:
            continue
        try:
            msg = json.loads(raw.decode("utf-8"))
        except (ValueError, UnicodeDecodeError) as e:
            write(_error(None, -32700, f"parse error: {e}"))
            continue
        if isinstance(msg, list):                      # JSON-RPC batch (older protocol revisions)
            out = [r for r in (server.handle(m) for m in msg) if r is not None]
            if out:
                write(out)
        else:
            r = server.handle(msg)
            if r is not None:
                write(r)


def main(argv=None) -> int:
    import argparse
    ap = argparse.ArgumentParser(prog="minihou mcp", description="MCP server (stdio) for MiniHou")
    ap.add_argument("--url", default=os.environ.get("MINIHOU_URL", DEFAULT_URL), help="engine HTTP API")
    ap.add_argument("--link-port", type=int, default=int(os.environ.get("MINIHOU_LINK_PORT", 8765)),
                    help="Blender link port for an engine started by open_graph")
    ap.add_argument("--workspace", default=os.environ.get("MINIHOU_WORKSPACE", str(REPO_ROOT)),
                    help="folder graph paths are relative to (default: the MiniHou folder)")
    args = ap.parse_args(argv)
    logging.basicConfig(level=logging.WARNING, stream=sys.stderr, format="minihou-mcp %(levelname)s %(message)s")
    out = sys.stdout.buffer
    sys.stdout = sys.stderr                            # stray prints must never corrupt the protocol stream
    serve_stdio(McpServer(EngineClient(args.url), Path(args.workspace), args.link_port), sys.stdin.buffer, out)
    return 0
