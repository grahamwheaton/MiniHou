"""Embedded MiniHou networks (add-on 0.3.0). See docs/EMBEDDED_GRAPHS.md.

Each MiniHou network (a stack modifier's group or a Geometry Nodes placement tree) can keep its
graph JSON inside the .blend, in a Text datablock named ``MiniHou: <label>.minihou`` with a fake
user. The group holds the text name and metadata as ID properties. Two modes:

* EMBEDDED (default for new networks): the text is authoritative. Edits arriving over the link
  are written to it (debounced), and again in save_pre. load_post reads it back.
* LINKED (every network made before 0.3.0): a .minihou/.json file is authoritative; no text.

The text stores file-path parameters relative to the .blend (``//...``); the runtime copy in the
group's ``minihou_graph`` property holds them resolved, which is what the engine receives.
Opening a network never runs embedded python node code: that is the app's opt-in permission.
"""
from __future__ import annotations

import hashlib
import json
import os
import time

try:
    import bpy
    from bpy.app.handlers import persistent
except ImportError:          # pure helpers are unit tested without Blender
    bpy = None

    def persistent(f):
        return f

EMBEDDED, LINKED = "EMBEDDED", "LINKED"
SCHEMA = 1                   # layout of these ID properties / the graph_push envelope
GRAPH_SCHEMA = 1             # graph "version" this add-on writes and reads
CAPABILITY = "graph_embed_v1"
WARN_BYTES = 2 * 1024 * 1024
MAX_BYTES = 32 * 1024 * 1024  # same bound as the app; larger graphs are refused, never truncated
DEBOUNCE = 0.75
TEXT_PREFIX = "MiniHou: "
TEXT_SUFFIX = ".minihou"

GRAPH = "minihou_graph"      # runtime copy (modifiers.GRAPH)
PATH = "minihou_graph_path"  # modifiers.PATH (the live editor overwrites it with a temp path)
IDS = ("minihou_modifier_id", "minihou_gn_id")
MODE = "minihou_embed_mode"
TEXT = "minihou_embed_text"
HASH = "minihou_embed_hash"
SIZE = "minihou_embed_size"
SYNCED = "minihou_embed_synced"
ESCHEMA = "minihou_embed_schema"
ADDON = "minihou_embed_addon"
FILE = "minihou_linked_file"
PATHS = "minihou_embed_paths"
MISSING = "minihou_embed_missing"
STATE = "minihou_embed_state"
ERROR = "minihou_embed_error"
FREEZE = "minihou_freeze"
OWNER = "minihou_owner"      # on the Text: the network id that owns it
ASSET_OWNER = "minihou_asset_owner"  # original modifier object, retained when an asset is copied
TARGET_CHANGES = "minihou_target_migration"  # exact graph fields rewritten for the asset
MARK = "minihou_embedded_graph"

_pending = {}                # network id -> monotonic time of the last unsaved edit
_swapped = {}                # unfrozen caches emptied for one save: cache name -> mesh name


class EmbedError(ValueError):
    """A clear, user-facing problem; the embedded data is left untouched."""


class ConflictPending(RuntimeError):
    """The app is asking the user how to combine its open work with the pushed network."""


def addon_version():
    try:
        from . import bl_info
        return ".".join(str(n) for n in bl_info["version"])
    except Exception:
        return "0.3.0"


# ----------------------------------------------------------------------------- pure helpers
def canonical(graph) -> str:
    """Deterministic text: key order preserved (node order is meaningful), one-space indent."""
    return json.dumps(graph, indent=1, ensure_ascii=False, allow_nan=False) + "\n"


def digest(text: str) -> str:
    return hashlib.sha256(text.encode("utf-8")).hexdigest()


def migrate(graph):
    """Return (graph, migrated). Older graphs are upgraded; newer ones raise EmbedError untouched."""
    if not isinstance(graph, dict) or not isinstance(graph.get("nodes"), dict):
        raise EmbedError("not a MiniHou network: the JSON has no 'nodes' object")
    version = graph.get("version")
    if version is None:      # schema 0: saved before graphs carried a version; same layout
        out = dict(graph)
        out["version"] = GRAPH_SCHEMA
        return out, True
    if not isinstance(version, int) or isinstance(version, bool):
        raise EmbedError(f"unsupported graph schema {version!r}")
    if version > GRAPH_SCHEMA:
        raise EmbedError(f"this network was saved by a newer MiniHou (graph schema {version}; this add-on reads "
                         f"up to {GRAPH_SCHEMA}). Update the add-on; the embedded data was left untouched")
    if version < 1:
        raise EmbedError(f"unsupported graph schema {version!r}")
    return graph, False


def path_params(graph):
    """(node id, param, value) for every file-path parameter: 'file' or '*_file' strings."""
    for nid, node in (graph.get("nodes") or {}).items():
        params = node.get("params") if isinstance(node, dict) else None
        if isinstance(params, dict):
            for key, value in params.items():
                if (key == "file" or key.endswith("_file")) and isinstance(value, str) and value.strip():
                    yield nid, key, value


def _map_paths(graph, fn):
    out = json.loads(json.dumps(graph))
    for nid, key, value in list(path_params(out)):
        new = fn(nid, key, value)
        if new is not None:
            out["nodes"][nid]["params"][key] = new
    return out


def relativize(graph, to_rel):
    """Paths -> blend-relative where to_rel can; returns (graph, {node/param: original})."""
    originals = {}

    def fn(nid, key, value):
        rel = to_rel(value)
        if rel is not None and rel != value:
            originals[f"{nid}/{key}"] = value
            return rel
        return None
    return _map_paths(graph, fn), originals


def resolve(graph, to_abs, exists, originals=None):
    """Blend-relative -> absolute; returns (graph, missing paths). Falls back to the stored original."""
    missing = []
    originals = originals or {}

    def fn(nid, key, value):
        path = to_abs(value)
        if not exists(path):
            fallback = originals.get(f"{nid}/{key}")
            if fallback and exists(fallback):
                return fallback
            missing.append(f"{nid}.{key}: {path}")
        return path if path != value else None
    return _map_paths(graph, fn), missing


def outside(graph, base_dir):
    """Absolute paths that cannot travel with the .blend (outside its folder)."""
    base = os.path.normcase(os.path.abspath(base_dir)) if base_dir else ""
    found = []
    for nid, key, value in path_params(graph):
        if value.startswith("//") or not os.path.isabs(value):
            continue
        target = os.path.normcase(os.path.abspath(value))
        if not base or os.path.commonpath([base, target]) != base:
            found.append(f"{nid}.{key}: {value}")
    return found


def text_name(label: str, taken=()) -> str:
    base = "".join(c if c not in '\\/:*?"<>|' else "_" for c in label).strip() or "network"
    base = base[:40]
    name = TEXT_PREFIX + base + TEXT_SUFFIX
    n = 2
    while name in taken:
        name = f"{TEXT_PREFIX}{base} {n}{TEXT_SUFFIX}"
        n += 1
    return name


def human_size(n: int) -> str:
    return f"{n} B" if n < 1024 else f"{n / 1024:.1f} KB" if n < 1024 * 1024 else f"{n / 1048576:.2f} MB"


# ----------------------------------------------------------------------------- Blender side
def network_id(ng) -> str:
    return next((str(ng[k]) for k in IDS if k in ng), "")


def networks():
    return [ng for ng in bpy.data.node_groups if network_id(ng)]


def mode(ng) -> str:
    return str(ng.get(MODE, LINKED))


def text_of(ng):
    name = ng.get(TEXT, "")
    return bpy.data.texts.get(name) if name else None


def linked_file(ng) -> str:
    """The network's file: the graph path unless the live editor replaced it with its temp file."""
    legacy = str(ng.get(PATH, ""))
    if legacy.strip() and "minihou-live-" not in os.path.basename(legacy):
        return legacy
    return str(ng.get(FILE, ""))


def _set_file(ng, path):
    ng[FILE] = ng[PATH] = os.path.abspath(path)


def frozen(ng) -> bool:
    return bool(ng.get(FREEZE, True))


def _blend_dir() -> str:
    return os.path.dirname(bpy.data.filepath) if bpy.data.filepath else ""


def _to_rel(path):
    if path.startswith("//") or not os.path.isabs(path) or not bpy.data.filepath:
        return None
    try:
        return bpy.path.relpath(path)
    except ValueError:       # another drive: stays absolute (panel warns)
        return None


def _to_abs(path):
    return os.path.normpath(bpy.path.abspath(path)) if path.startswith("//") else path


def _label(ng) -> str:
    for obj in bpy.data.objects:
        for mod in getattr(obj, "modifiers", ()):
            if getattr(mod, "node_group", None) == ng:
                return f"{obj.name} {mod.name}"
    return ng.name.replace("MiniHou - ", "").replace("MiniHou ", "") or "network"


def runtime_graph(ng):
    return json.loads(ng.get(GRAPH, "{}") or "{}")


def bind_target(graph, owner):
    """Replace only root Blender Object references to the asset's original owner."""
    out = json.loads(json.dumps(graph))
    changed = []
    for nid, node in out.get("nodes", {}).items():
        if (node.get("type") == "blender_object" and not node.get("inputs")
                and node.get("params", {}).get("object") == owner):
            node["params"]["object"] = "@target"
            changed.append(f"nodes.{nid}.params.object: {owner!r} -> '@target'")
    return out, changed


def _asset_owner(ng):
    owner = str(ng.get(ASSET_OWNER, ""))
    if owner or bpy is None or "minihou_modifier_id" not in ng:
        return owner
    users = [obj for obj in bpy.data.objects for mod in getattr(obj, "modifiers", ())
             if getattr(mod, "node_group", None) == ng]
    if len(users) > 1:
        roots = {node.get("params", {}).get("object") for node in runtime_graph(ng).get("nodes", {}).values()
                 if node.get("type") == "blender_object" and not node.get("inputs")}
        matches = {obj.name for obj in users if obj.name in roots}
        if len(matches) == 1:
            owner = matches.pop()
            ng[ASSET_OWNER] = owner
            return owner
    if len(users) == 1:
        owner = users[0].name
        # A legacy asset may arrive without its original owner object. Its copied
        # Source Object socket still names that owner until the instance is split.
        try:
            from . import modifiers
            mod = next(mod for mod in users[0].modifiers if mod.node_group == ng)
            source = modifiers.get_source(mod)
            if source is not None and source.name != owner:
                owner = source.name
        except (ImportError, AttributeError, KeyError, StopIteration):
            pass
        ng[ASSET_OWNER] = owner
        return owner
    return ""


def repair_target(ng):
    """Migrate a known owner's runtime and embedded graph, recording each changed field."""
    owner = _asset_owner(ng)
    if not owner:
        return []
    graph, changed = bind_target(runtime_graph(ng), owner)
    if changed:
        ng[GRAPH] = json.dumps(graph)
        ng[TARGET_CHANGES] = json.dumps(changed)
        if mode(ng) == EMBEDDED:
            write(ng, graph)
        print(f"[minihou] {ng.name}: asset target migration: {'; '.join(changed)}")
    return changed


def write(ng, graph=None, *, label=None):
    """Store graph (default: the runtime copy) in the network's text. Returns the hash."""
    graph = runtime_graph(ng) if graph is None else graph
    graph, _ = migrate(graph)
    stored, originals = relativize(graph, _to_rel)
    content = canonical(stored)
    size = len(content.encode("utf-8"))
    if size > MAX_BYTES:
        raise EmbedError(f"network is {human_size(size)}; the embed limit is {human_size(MAX_BYTES)}. "
                         "Extract it to a .minihou file and use Linked File mode")
    text = text_of(ng)
    if text is None:
        text = bpy.data.texts.new(text_name(label or _label(ng), {t.name for t in bpy.data.texts}))
    if text.as_string() != content:
        text.from_string(content)
    text.use_fake_user = True
    text[OWNER] = network_id(ng)
    text[MARK] = SCHEMA
    ng[TEXT] = text.name
    ng[HASH] = digest(content)
    ng[SIZE] = size
    ng[SYNCED] = time.time()
    ng[ESCHEMA] = SCHEMA
    ng[ADDON] = addon_version()
    ng[PATHS] = json.dumps(originals)
    ng[MISSING] = json.dumps(resolve(stored, _to_abs, os.path.exists, originals)[1])
    if ERROR in ng:
        del ng[ERROR]
    if ng.get(STATE) == "unreadable":
        del ng[STATE]
    _pending.pop(network_id(ng), None)
    return ng[HASH]


def read(ng):
    """The embedded graph resolved for this machine: (graph, content). Raises EmbedError."""
    text = text_of(ng)
    if text is None:
        raise EmbedError("the embedded network text is missing")
    content = text.as_string()
    try:
        stored = json.loads(content)
    except ValueError as exc:
        raise EmbedError(f"the embedded network text is not valid JSON ({exc}); it was left untouched") from None
    stored, _ = migrate(stored)
    graph, missing = resolve(stored, _to_abs, os.path.exists, json.loads(ng.get(PATHS, "{}") or "{}"))
    ng[MISSING] = json.dumps(missing)
    return graph, content


def embed(ng, graph=None, *, label=None, file=None):
    """Switch to EMBEDDED keeping (or replacing) the network. Existing linked file is remembered."""
    if file:
        _set_file(ng, file)
    elif not ng.get(FILE) and linked_file(ng):
        ng[FILE] = linked_file(ng)
    if graph is not None:
        graph, _ = migrate(graph)
        ng[GRAPH] = json.dumps(graph)
    repair_target(ng)
    h = write(ng, label=label)
    ng[MODE] = EMBEDDED
    return h


def load_file(path):
    with open(path, encoding="utf-8") as f:
        text = f.read()
    try:
        graph = json.loads(text)
    except ValueError as exc:
        raise EmbedError(f"{os.path.basename(path)} is not valid JSON: {exc}") from None
    return migrate(graph)[0]


def embed_from_file(ng, path, *, label=None):
    return embed(ng, load_file(path), label=label, file=path)


def extract(ng, path):
    """Write the network to a standalone file (absolute paths, so it works anywhere). Returns bytes written."""
    if mode(ng) == EMBEDDED and text_of(ng) is not None:
        graph = read(ng)[0]
    else:
        graph = migrate(runtime_graph(ng))[0]
    content = canonical(graph)
    tmp = path + ".tmp"
    with open(tmp, "w", encoding="utf-8", newline="\n") as f:
        f.write(content)
    os.replace(tmp, path)
    _set_file(ng, path)
    return len(content.encode("utf-8"))


def _file_digest(path):
    """Hash the file the way it would be embedded, so equal networks compare equal."""
    stored, _ = relativize(load_file(path), _to_rel)
    return digest(canonical(stored))


def compare(ng):
    """Status for the panel: mode, size, hash, which copy is newer, path warnings."""
    info = {"mode": mode(ng), "size": int(ng.get(SIZE, 0)), "hash": str(ng.get(HASH, "")),
            "file": linked_file(ng), "newer": "", "error": str(ng.get(ERROR, "")),
            "missing": json.loads(ng.get(MISSING, "[]") or "[]"), "state": str(ng.get(STATE, ""))}
    try:
        info["outside"] = outside(runtime_graph(ng), _blend_dir())
    except ValueError:
        info["outside"] = []
    path = info["file"]
    if info["mode"] != EMBEDDED or not path:
        return info
    if not os.path.exists(path):
        info["newer"] = "file missing"
        return info
    try:
        same = _file_digest(path) == info["hash"]
    except (OSError, ValueError) as exc:
        info["newer"] = f"file unreadable: {exc}"
        return info
    if same:
        info["newer"] = "same"
    else:
        info["newer"] = "file" if os.path.getmtime(path) > float(ng.get(SYNCED, 0)) else "embedded"
    return info


def resolve_choice(ng, choice):
    """Panel buttons when embedded and file differ."""
    path = linked_file(ng)
    if choice == "USE_EMBEDDED":
        graph = read(ng)[0]
        ng[GRAPH] = json.dumps(graph)
        ng[SYNCED] = time.time()
    elif choice == "USE_FILE":
        embed_from_file(ng, path)
    elif choice == "MATCH_FILE":
        extract(ng, path)
        ng[SYNCED] = time.time()
    else:
        raise EmbedError(f"unknown choice {choice}")


def switch_mode(ng, *, label=None):
    if mode(ng) == EMBEDDED:
        path = linked_file(ng)
        if not path or not os.path.exists(path):
            raise EmbedError("Extract to a .minihou file first; Linked File mode needs a file")
        if compare(ng)["newer"] != "same":
            raise EmbedError("the file differs from the embedded network; use 'Make file match embedded' or "
                             "'Use file' first so nothing is lost")
        ng[GRAPH] = json.dumps(load_file(path))
        ng[PATH] = path
        ng[MODE] = LINKED
        release(ng)
        return LINKED
    embed(ng, label=label)
    return EMBEDDED


def release(ng):
    """Drop the network's text (deleted modifier or Linked File mode)."""
    text = text_of(ng)
    _pending.pop(network_id(ng), None)
    if text is not None and str(text.get(OWNER, "")) in ("", network_id(ng)):
        if not any(o is not ng and o.get(TEXT) == text.name for o in bpy.data.node_groups):
            bpy.data.texts.remove(text)
    if TEXT in ng:
        del ng[TEXT]


def note_graph(ng, *, pending=False):
    """The runtime graph changed (link edit). Debounced write; never during a pending conflict."""
    if mode(ng) == EMBEDDED and not pending and ng.get(STATE) not in ("conflict", "unreadable"):
        _pending[network_id(ng)] = time.monotonic()


def flush(force=False):
    if not _pending:
        return 0
    by_id = {network_id(ng): ng for ng in networks()}
    now, written = time.monotonic(), 0
    for ident, when in list(_pending.items()):
        ng = by_id.get(ident)
        if ng is None:
            _pending.pop(ident, None)
            continue
        if force or now - when >= DEBOUNCE:
            try:
                write(ng)
                written += 1
            except (EmbedError, ValueError) as exc:
                ng[ERROR] = str(exc)
                _pending.pop(ident, None)
    return written


def repair():
    """Duplicates get independent texts; lost texts are rebuilt; orphaned texts are removed."""
    owners = {}
    for ng in networks():
        if mode(ng) != EMBEDDED or ng.users == 0 or ng.get(STATE) == "unreadable":
            continue
        text = text_of(ng)
        if text is None:
            try:
                write(ng)
                ng[ERROR] = "embedded text was missing; rebuilt from the modifier's runtime copy"
            except (EmbedError, ValueError) as exc:
                ng[ERROR] = str(exc)
            continue
        owners.setdefault(text.name, []).append(ng)
    for name, groups in owners.items():
        text = bpy.data.texts[name]
        owner = str(text.get(OWNER, ""))
        keeper = next((g for g in groups if network_id(g) == owner), groups[0])
        text[OWNER] = network_id(keeper)
        for ng in groups:
            if ng is keeper:
                continue
            copy = text.copy()
            copy.name = text_name(_label(ng), {t.name for t in bpy.data.texts})
            copy.use_fake_user = True
            copy[OWNER] = network_id(ng)
            ng[TEXT] = copy.name
    for ng in networks():
        if ng.users and ng.get(STATE) != "unreadable":
            try:
                repair_target(ng)
            except (EmbedError, ValueError) as exc:
                ng[ERROR] = str(exc)
    live = {network_id(ng) for ng in networks() if mode(ng) == EMBEDDED}
    used = {ng.get(TEXT) for ng in networks()}
    for text in list(bpy.data.texts):
        if MARK in text and (str(text.get(OWNER, "")) not in live or text.name not in used):
            bpy.data.texts.remove(text)


def load_all():
    """load_post: the embedded copy becomes the runtime copy. Errors leave the data untouched."""
    for ng in networks():
        if mode(ng) == LINKED:
            if not ng.get(FILE) and linked_file(ng):
                ng[FILE] = linked_file(ng)       # remember it before a live edit overwrites the path
            continue
        if STATE in ng:
            del ng[STATE]                         # a conflict belongs to a session, not the file
        if text_of(ng) is None:
            continue                              # repair() rebuilds it from the runtime copy
        try:
            graph, content = read(ng)
        except EmbedError as exc:
            ng[ERROR] = str(exc)
            ng[STATE] = "unreadable"      # never overwritten by save/link until the user replaces it
            continue
        data = json.dumps(graph)
        if ng.get(GRAPH) != data:
            ng[GRAPH] = data
        if digest(content) != ng.get(HASH):
            ng[HASH] = digest(content)            # edited in Blender's text editor: adopt it
            ng[SIZE] = len(content.encode("utf-8"))
            ng[SYNCED] = time.time()
        if ERROR in ng:
            del ng[ERROR]


def save_all():
    """save_pre: pending link edits and any runtime change reach the text before the file is written."""
    flush(force=True)
    for ng in networks():
        if mode(ng) != EMBEDDED or ng.get(STATE) in ("conflict", "unreadable"):
            continue
        try:
            stored, _ = relativize(migrate(runtime_graph(ng))[0], _to_rel)
            text = text_of(ng)
            if text is None or text.as_string() != canonical(stored):
                write(ng)
        except (EmbedError, ValueError) as exc:
            ng[ERROR] = str(exc)


# ----------------------------------------------------------------------------- link (graph_embed_v1)
def push(client, ng, label):
    """Send the embedded graph before opening it in MiniHou. Returns the ack status, or '' when the
    app predates graph_embed_v1 (the caller then uses the ordinary live edit, as before)."""
    welcome = client.connect()
    if CAPABILITY not in welcome.get("capabilities", []):
        return ""
    text = text_of(ng)
    if text is None:
        write(ng, label=label)
        text = text_of(ng)
    graph = read(ng)[0]              # the app gets resolved paths, like the engine
    content = json.dumps(graph, ensure_ascii=False, separators=(",", ":"))
    data = content.encode("utf-8")
    if len(data) > MAX_BYTES:
        raise EmbedError(f"network is {human_size(len(data))}; the link limit is {human_size(MAX_BYTES)}")
    ack = client.push_graph(network_id(ng), name=text.name[len(TEXT_PREFIX):-len(TEXT_SUFFIX)] or label,
                            label=label, text=content, sha256=hashlib.sha256(data).hexdigest(), schema=SCHEMA)
    status = ack.get("status", "")
    if status == "conflict":
        ng[STATE] = "conflict"
    elif status == "app_newer":
        adopt(ng, ack)
    return status


def adopt(ng, state):
    """Take the app's graph from a graph_state/graph_ack (hash verified) into runtime copy and text."""
    content = state.get("json")
    if not isinstance(content, str):
        return False
    if hashlib.sha256(content.encode("utf-8")).hexdigest() != state.get("hash"):
        raise EmbedError("graph from MiniHou failed its integrity check; the embedded copy was kept")
    graph = migrate(json.loads(content))[0]
    ng[GRAPH] = json.dumps(graph)
    if mode(ng) == EMBEDDED:
        write(ng)
    return True


def poll(client, ng):
    """While a conflict is open in the app: returns its resolution once answered, else None."""
    state = client.pull_graph(network_id(ng))
    if state.get("status") != "resolved":
        return None
    del ng[STATE]
    resolution = state.get("resolution", "")
    if state.get("json"):
        adopt(ng, state)
    elif resolution == "keep_mine":
        ng[ERROR] = "MiniHou kept its open document; click Open in MiniHou again when ready"
    return resolution


# ----------------------------------------------------------------------------- freeze result
def _cache_object(ng):
    ident = ng.get("minihou_modifier_id")
    if not ident:
        return None
    return next((o for o in bpy.data.objects if o.get("minihou_cache_id") == ident and o.type == "MESH"), None)


def frozen_cost(ng):
    obj = _cache_object(ng)
    if obj is None or obj.data is None:
        return 0, 0
    m = obj.data
    approx = len(m.vertices) * 12 + len(m.loops) * 8 + len(m.polygons) * 12 + len(m.edges) * 8
    for attr in m.attributes:
        if not attr.name.startswith(".") and attr.name not in {"position"}:
            approx += len(attr.data) * 4
    return len(m.vertices), approx


def _strip_unfrozen():
    """save_pre: an unfrozen network saves without its cooked mesh (restored in save_post)."""
    for ng in networks():
        if frozen(ng):
            continue
        obj = _cache_object(ng)
        if obj is None or obj.data is None or not len(obj.data.vertices):
            continue
        empty = bpy.data.meshes.new("MiniHou Cache (not frozen)")
        _swapped[obj.name] = obj.data.name
        obj.data.use_fake_user = False
        obj.data = empty


def _restore_unfrozen():
    for obj_name, mesh_name in list(_swapped.items()):
        obj, mesh = bpy.data.objects.get(obj_name), bpy.data.meshes.get(mesh_name)
        if obj is not None and mesh is not None:
            empty = obj.data
            obj.data = mesh
            if empty is not None and empty.users == 0:
                bpy.data.meshes.remove(empty)
    _swapped.clear()


def _passthrough_unfrozen():
    """load_post: an unfrozen network shows its upstream geometry until MiniHou cooks it."""
    from . import proxy
    for ng in networks():
        obj = _cache_object(ng)
        if not frozen(ng) and obj is not None and obj.data is not None and not len(obj.data.vertices):
            try:
                proxy.link_passthrough(ng)
            except (StopIteration, KeyError, AttributeError):
                pass


# ----------------------------------------------------------------------------- handlers
@persistent
def _save_pre(*_args):
    try:
        save_all()
        _strip_unfrozen()
    except Exception as exc:     # never block saving the user's file
        print(f"[minihou] embedding before save failed: {exc}")


@persistent
def _save_post(*_args):
    _restore_unfrozen()


@persistent
def _load_post(*_args):
    _pending.clear()
    _swapped.clear()
    try:
        load_all()
        repair()
        _passthrough_unfrozen()
    except Exception as exc:
        print(f"[minihou] reading embedded networks failed: {exc}")


def _timer():
    try:
        flush()
    except Exception as exc:
        print(f"[minihou] embedded write failed: {exc}")
    return 0.5


def register():
    for handlers, fn in ((bpy.app.handlers.save_pre, _save_pre), (bpy.app.handlers.save_post, _save_post),
                         (bpy.app.handlers.load_post, _load_post)):
        if fn not in handlers:
            handlers.append(fn)
    if not bpy.app.timers.is_registered(_timer):
        bpy.app.timers.register(_timer, first_interval=0.5, persistent=True)


def unregister():
    for handlers, fn in ((bpy.app.handlers.save_pre, _save_pre), (bpy.app.handlers.save_post, _save_post),
                         (bpy.app.handlers.load_post, _load_post)):
        if fn in handlers:
            handlers.remove(fn)
    if bpy.app.timers.is_registered(_timer):
        bpy.app.timers.unregister(_timer)
