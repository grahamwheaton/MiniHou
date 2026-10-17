"""Define MiniHou input-node inputs from Blender (add-on 0.3.1, link message ``input_define_v1``).

Connected and bound (the network is the document open in MiniHou, normally while it is live-edited):
the add-on sends ``input_define`` and MiniHou applies it as an undoable edit through the same code as
its own "Add input" button (rust/crates/minihou-app/src/input_define.rs); the ack carries the new
graph and interface. Otherwise (no app, an older app without the capability, or a different open
document) the definition is applied here with the same rules and stored in the network's embedded
graph text, which is authoritative in Embedded mode; MiniHou sees it when it next opens the graph.
Linked File networks need the app (the file is MiniHou's to write).

The pure helpers mirror the Rust rules and are unit tested without Blender.
"""
from __future__ import annotations

import json

try:
    import bpy
except ImportError:          # pure helpers are tested without Blender
    bpy = None

CAPABILITY = "input_define_v1"
MAX_MESSAGE_BYTES = 64 * 1024
KINDS = ("float", "int", "bool", "vector", "color", "rotation", "matrix", "string", "menu",
         "object", "collection", "material", "image", "texture", "geometry", "bundle", "closure")
ID_KINDS = {"object": "objects", "collection": "collections", "material": "materials",
            "image": "images", "texture": "textures"}
SUBTYPES = {"vector": ("translation", "xyz", "euler", "direction", "velocity", "acceleration"),
            "float": ("angle", "distance", "factor", "percentage", "time"),
            "int": ("factor", "percentage"), "string": ("filepath",)}
RESERVED = ("Geometry", "Source Object")


def default_for(kind):
    return {"float": 0.0, "int": 0, "bool": False, "vector": [0.0, 0.0, 0.0], "rotation": [0.0, 0.0, 0.0],
            "color": [1.0, 1.0, 1.0, 1.0],
            "matrix": [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0]}.get(kind, "")


def _reserved(name):
    return name in RESERVED or name.startswith("Object: ")


def unique_name(taken, wanted):
    """``wanted``, else ``wanted 2``, ``wanted 3``... (same as the app)."""
    base = (wanted or "").strip() or "Input"
    if base.startswith("Object: "):          # the prefix is reserved for object bindings; no suffix frees it
        base = "Object " + base[len("Object: "):]
    taken = set(taken)
    if base not in taken and not _reserved(base):
        return base
    i = 2
    while f"{base} {i}" in taken or _reserved(f"{base} {i}"):
        i += 1
    return f"{base} {i}"


def input_nodes(graph):
    return [k for k, n in (graph.get("nodes") or {}).items() if n.get("type") == "modifier_input"]


def entries(graph):
    """Rust modifier_inputs::entries: each definition with its node and current value."""
    out = []
    for node in input_nodes(graph):
        data = graph["nodes"][node]
        for e in data.get("modifier_inputs") or []:
            item = dict(e, node=node)
            item["value"] = (data.get("input_values") or {}).get(e.get("id"), e.get("default"))
            out.append(item)
    return out


def taken_names(graph):
    return [e["name"] for e in graph.get("exposed") or [] if "name" in e] + [e["name"] for e in entries(graph)]


def _numbers(v, n):
    return isinstance(v, list) and len(v) == n and all(isinstance(c, (int, float)) and not isinstance(c, bool) for c in v)


def validate(inputs):
    """Mirror of modifier_inputs::validate (raises ValueError with the app's wording)."""
    if len(inputs) > 128:
        raise ValueError("modifier_input supports at most 128 inputs")
    ids, names = set(), set()
    for e in inputs:
        ident, name, kind = e.get("id"), e.get("name"), e.get("kind")
        if not isinstance(ident, str) or not ident or len(ident) > 128:
            raise ValueError("input needs a stable id")
        if not isinstance(name, str) or not name.strip() or len(name) > 128:
            raise ValueError("input needs a name")
        if _reserved(name):
            raise ValueError(f"{name!r} is reserved by the Blender group")
        if kind not in KINDS:
            raise ValueError(f"unsupported Blender input type {kind!r}")
        if ident in ids or name in names:
            raise ValueError("duplicate input id or name")
        ids.add(ident); names.add(name)
        if "default" not in e:
            raise ValueError("input needs a default")
        value = e["default"]
        subtype = e.get("subtype")
        if subtype and subtype not in SUBTYPES.get(kind, ()):
            raise ValueError(f"{name}: unsupported {kind} subtype {subtype!r}")
        num = isinstance(value, (int, float)) and not isinstance(value, bool)
        ok = {"float": num, "int": isinstance(value, int) and not isinstance(value, bool),
              "bool": isinstance(value, bool), "vector": _numbers(value, 3), "rotation": _numbers(value, 3),
              "color": _numbers(value, 4), "matrix": _numbers(value, 16)}.get(kind, isinstance(value, str) or value is None)
        if not ok:
            raise ValueError(f"invalid default for {name} ({kind})")
        if kind == "menu":
            options = e.get("options") or []
            if not options or len(set(options)) != len(options) or not all(isinstance(o, str) and o for o in options) \
                    or value not in options:
                raise ValueError(f"{name}: menu default must be one of its options")
        lo, hi = e.get("min"), e.get("max")
        for bound, b in (("min", lo), ("max", hi)):
            if b is not None and (not isinstance(b, (int, float)) or isinstance(b, bool)):
                raise ValueError(f"{name}: {bound} must be numeric")
        if lo is not None and hi is not None and lo > hi:
            raise ValueError(f"{name}: minimum exceeds maximum")
        for c in (value if isinstance(value, list) else [value]):
            if isinstance(c, (int, float)) and not isinstance(c, bool):
                if (lo is not None and c < lo) or (hi is not None and c > hi):
                    raise ValueError(f"{name}: value outside input limits")


def _target(graph, wanted=None):
    nodes = input_nodes(graph)
    if wanted:
        if wanted not in nodes:
            raise ValueError(f"{wanted!r} is not a MiniHou input node")
        return wanted
    if not nodes:
        raise ValueError("this network has no MiniHou input node (modifier_input); add one in MiniHou first")
    return nodes[0]


def add_entry(graph, node, draft):
    """(inputs, entry) with ``draft`` appended: the app's ids (input_N), suffixes and defaults."""
    current = list(graph["nodes"][node].get("modifier_inputs") or [])
    kind = draft.get("kind")
    if kind not in KINDS:
        raise ValueError(f"unsupported Blender input type {kind!r}")
    used = {e.get("id") for e in current}
    i = 1
    while f"input_{i}" in used:
        i += 1
    options = [o.strip() for o in draft.get("options") or [] if o and o.strip()]
    default = draft.get("default")
    if default is None:
        default = (options[0] if options else None) if kind == "menu" else default_for(kind)
    entry = {"id": f"input_{i}", "name": unique_name(taken_names(graph), draft.get("name", "")),
             "kind": kind, "default": default}
    for bound in ("min", "max"):
        if draft.get(bound) is not None:
            entry[bound] = float(draft[bound])
    if kind == "menu":
        entry["options"] = options
    elif (draft.get("subtype") or "").strip():
        entry["subtype"] = draft["subtype"].strip()
    if (draft.get("doc") or "").strip():
        entry["doc"] = draft["doc"].strip()
    validate(current + [entry])
    return current + [entry], entry


def plan(graph, msg):
    """(node, inputs, entry) for one input_define message; mirror of input_define::plan."""
    if len(json.dumps(msg).encode("utf-8")) > MAX_MESSAGE_BYTES:
        raise ValueError(f"input_define is larger than {MAX_MESSAGE_BYTES} bytes")
    node = _target(graph, msg.get("node"))
    current = list(graph["nodes"][node].get("modifier_inputs") or [])
    op = msg.get("op", "add")

    def find(ident):
        for k, e in enumerate(current):
            if e.get("id") == ident:
                return k
        raise ValueError(f"unknown input id {ident!r}")
    if op == "add":
        inputs, entry = add_entry(graph, node, msg.get("input") or {})
    elif op == "rename":
        k = find(msg.get("id"))
        old = current[k]["name"]
        inputs = [dict(e) for e in current]
        inputs[k]["name"] = unique_name([n for n in taken_names(graph) if n != old], msg.get("name", ""))
        validate(inputs)
        entry = inputs[k]
    elif op == "remove":
        k = find(msg.get("id"))
        inputs = current[:k] + current[k + 1:]
        entry = current[k]
    else:
        raise ValueError(f"unknown input_define op {op!r}")
    return node, inputs, dict(entry, node=node)


def apply_offline(graph, msg):
    """Apply to a graph dict (copy). Removing an input resets parameters linked to it, like the app."""
    graph = json.loads(json.dumps(graph))
    node, inputs, entry = plan(graph, msg)
    data = graph["nodes"][node]
    kept = {e["id"] for e in inputs}
    removed = {e["id"] for e in data.get("modifier_inputs") or []} - kept
    data["modifier_inputs"] = inputs
    values = data.get("input_values") or {}
    for ident in removed:
        values.pop(ident, None)
    if "input_values" in data:
        data["input_values"] = values
    if removed:
        for other in graph["nodes"].values():
            for pname, v in list((other.get("params") or {}).items()):
                if isinstance(v, dict) and (v.get("modifier_input") or {}).get("node") == node \
                        and v["modifier_input"].get("id") in removed:
                    del other["params"][pname]   # back to the node's own default
    return graph, entry


def merged_interface(old_params, graph):
    """Exposed parameters keep their engine-described entries; modifier inputs follow the graph."""
    nodes = set(input_nodes(graph))
    keep = [p for p in old_params if not (p.get("id") and p.get("node") in nodes)]
    names = {p["name"] for p in keep}
    return keep + [dict(e, label=e["name"]) for e in entries(graph) if e["name"] not in names]


def build_draft(kind, name, *, float_value=0.0, int_value=0, bool_value=False, vector=(0.0, 0.0, 0.0),
                color=(1.0, 1.0, 1.0, 1.0), string="", id_name="", options="", use_min=False, min_value=0.0,
                use_max=False, max_value=1.0, subtype="", doc=""):
    """The Add Input dialog's fields -> an input_define ``input`` (only what the type uses)."""
    if kind not in KINDS:
        raise ValueError(f"unsupported Blender input type {kind!r}")
    default = {"float": float(float_value), "int": int(int_value), "bool": bool(bool_value),
               "vector": [float(c) for c in vector], "rotation": [float(c) for c in vector],
               "color": [float(c) for c in color], "string": string, "matrix": None,
               "menu": None}.get(kind, id_name if kind in ID_KINDS else "")
    draft = {"name": name, "kind": kind, "default": default}
    if kind in {"float", "int", "vector", "rotation"}:
        if use_min:
            draft["min"] = float(min_value)
        if use_max:
            draft["max"] = float(max_value)
    if kind == "menu":
        draft["options"] = [o.strip() for o in options.split(",") if o.strip()]
    elif subtype and subtype != "NONE" and kind in SUBTYPES:
        draft["subtype"] = subtype
    if doc.strip():
        draft["doc"] = doc.strip()
    return draft


def request(op="add", *, instance_id="", path="", node=None, input=None, ident=None, name=None):
    msg = {"type": "input_define", "op": op, "instance_id": instance_id, "path": path}
    if node:
        msg["node"] = node
    if input is not None:
        msg["input"] = input
    if ident is not None:
        msg["id"] = ident
    if name is not None:
        msg["name"] = name
    return msg


# --- Blender side -------------------------------------------------------------------------------

def _apply_interface(ng, params, mod=None):
    from . import geonodes, modifiers, proxy
    if mod is not None and modifiers.is_modifier(mod):
        modifiers._interface(mod, {"interface": params})
        modifiers._digests.pop(modifiers.instance_id(mod), None)
    else:
        proxy.sync_interface(ng, geonodes._cache(ng), params, json.dumps(params))
        ng[modifiers.PARAMS] = json.dumps(params)
        geonodes._object_interface(ng)
        geonodes._digests.pop(ng.get(geonodes.KEY), None)


def _client():
    from . import modifiers
    client = modifiers._client
    if client is None or client.sock is None:
        try:
            client = modifiers._connection()
        except Exception:
            return None
    return client


def define(ng, msg, *, mod=None, client=None, connect=True):
    """Apply ``msg`` to network ``ng``. Returns (entry, route); route is "app", "offline" or "offline_old_app".

    Raises ValueError for definitions the rules refuse (in the app's words).
    """
    from . import embed, modifiers
    graph = embed.runtime_graph(ng)
    plan(graph, msg)                                  # refuse early with the same wording, before any I/O
    msg = dict(msg, instance_id=str(next((ng[k] for k in embed.IDS if k in ng), "")),
               path=str(ng.get(modifiers.PATH, "")))
    client = client if client is not None else (_client() if connect else None)
    route = "offline"
    if client is not None:
        if client.has_capability(CAPABILITY):
            reply = client.define_input(msg)
            if reply.get("status") == "ok":
                ng[modifiers.GRAPH] = json.dumps(reply["graph"])
                _apply_interface(ng, reply["interface"], mod)
                embed.note_graph(ng)
                return reply["input"], "app"
            if reply.get("code") != "not_bound":
                raise ValueError(reply.get("message", "MiniHou refused the input"))
        else:
            route = "offline_old_app"
    if embed.mode(ng) != embed.EMBEDDED:
        raise ValueError("Linked File network: open it in MiniHou (Edit Network) to add inputs, "
                         "or switch it to Embedded" + (" (the connected MiniHou app cannot define inputs; update it)"
                                                       if route == "offline_old_app" else ""))
    graph, entry = apply_offline(graph, msg)
    ng[modifiers.GRAPH] = json.dumps(graph)
    embed.write(ng)
    _apply_interface(ng, merged_interface(json.loads(ng.get(modifiers.PARAMS, "[]")), graph), mod)
    return entry, route


def network_inputs(ng):
    """The network's dynamic inputs (definitions with node/value), for the panel."""
    from . import embed
    try:
        return entries(embed.runtime_graph(ng))
    except (ValueError, AttributeError):
        return []
