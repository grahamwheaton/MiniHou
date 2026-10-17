"""Panel section and operators for embedded networks (add-on 0.3.0)."""
import os

import bpy
from bpy.props import EnumProperty, StringProperty

from . import embed, modifiers


def _network(context, name):
    mod = context.object.modifiers.get(name) if context.object else None
    if mod is None or not modifiers.is_modifier(mod):
        return None, None
    return mod, mod.node_group


def _label(context, mod):
    return f"{context.object.name} {mod.name}"


class _NetworkOp:
    modifier_name: StringProperty()

    def run(self, context, fn):
        mod, ng = _network(context, self.modifier_name)
        if ng is None:
            self.report({"ERROR"}, "MiniHou modifier no longer exists")
            return {"CANCELLED"}
        try:
            message = fn(context, mod, ng)
        except (embed.EmbedError, OSError, ValueError) as exc:
            self.report({"ERROR"}, str(exc))
            return {"CANCELLED"}
        modifiers._digests.pop(modifiers.instance_id(mod), None)   # recook with the adopted graph
        if message:
            self.report({"INFO"}, message)
        return {"FINISHED"}


class MINIHOU_OT_embed_from_file(_NetworkOp, bpy.types.Operator):
    bl_idname = "minihou.embed_from_file"
    bl_label = "Embed from File"
    bl_description = "Copy a .minihou/.json network into this .blend (Embedded mode); the file is remembered"
    bl_options = {"REGISTER", "UNDO"}
    filepath: StringProperty(subtype="FILE_PATH")
    filter_glob: StringProperty(default="*.minihou;*.json", options={"HIDDEN"})

    def invoke(self, context, event):
        mod, ng = _network(context, self.modifier_name)
        if ng is not None and embed.linked_file(ng):
            self.filepath = embed.linked_file(ng)
        context.window_manager.fileselect_add(self)
        return {"RUNNING_MODAL"}

    def execute(self, context):
        def go(context, mod, ng):
            embed.embed_from_file(ng, bpy.path.abspath(self.filepath), label=_label(context, mod))
            return f"Embedded {os.path.basename(self.filepath)} ({embed.human_size(int(ng[embed.SIZE]))})"
        return self.run(context, go)


class MINIHOU_OT_extract_graph(_NetworkOp, bpy.types.Operator):
    bl_idname = "minihou.extract_graph"
    bl_label = "Extract to .minihou File..."
    bl_description = "Write this network to a separate .minihou file for sharing or central storage"
    filepath: StringProperty(subtype="FILE_PATH")
    filter_glob: StringProperty(default="*.minihou;*.json", options={"HIDDEN"})

    def invoke(self, context, event):
        mod, ng = _network(context, self.modifier_name)
        if ng is not None:
            self.filepath = embed.linked_file(ng) or bpy.path.abspath(
                "//" + bpy.path.clean_name(_label(context, mod)) + ".minihou")
        context.window_manager.fileselect_add(self)
        return {"RUNNING_MODAL"}

    def execute(self, context):
        def go(context, mod, ng):
            path = bpy.path.abspath(self.filepath)
            if not path.lower().endswith((".minihou", ".json")):
                path += ".minihou"
            size = embed.extract(ng, path)
            return f"Wrote {os.path.basename(path)} ({embed.human_size(size)})"
        return self.run(context, go)


class MINIHOU_OT_switch_embed_mode(_NetworkOp, bpy.types.Operator):
    bl_idname = "minihou.switch_embed_mode"
    bl_label = "Switch Mode"
    bl_description = "Toggle between Embedded (network stored in this .blend) and Linked File (the file is authoritative)"
    bl_options = {"REGISTER", "UNDO"}

    def execute(self, context):
        return self.run(context, lambda c, mod, ng: "Mode: " + embed.switch_mode(ng, label=_label(c, mod)).title())


class MINIHOU_OT_resolve_embed(_NetworkOp, bpy.types.Operator):
    bl_idname = "minihou.resolve_embed"
    bl_label = "Resolve Embedded / File"
    bl_options = {"REGISTER", "UNDO"}
    choice: EnumProperty(items=(("USE_EMBEDDED", "Use Embedded", "Keep the copy in this .blend"),
                                ("USE_FILE", "Use File", "Replace the embedded copy with the file"),
                                ("MATCH_FILE", "Make File Match Embedded", "Overwrite the file with the embedded copy")))

    @classmethod
    def description(cls, context, properties):
        return {"USE_EMBEDDED": "Keep the network stored in this .blend",
                "USE_FILE": "Replace the embedded network with the linked file",
                "MATCH_FILE": "Overwrite the linked file with the embedded network"}[properties.choice]

    def execute(self, context):
        return self.run(context, lambda c, mod, ng: embed.resolve_choice(ng, self.choice))


def draw(layout, mod):
    ng = mod.node_group
    info = embed.compare(ng)
    box = layout.box()
    if info["mode"] == embed.EMBEDDED:
        short = info["hash"][:8] or "--------"
        box.label(text=f"Embedded in .blend  {embed.human_size(info['size'])}  #{short}", icon="PACKAGE")
    else:
        name = os.path.basename(info["file"]) if info["file"] else "graph stored on the modifier only"
        box.label(text=f"Linked file: {name}", icon="LINKED")
    if info["size"] > embed.WARN_BYTES:
        warn = box.row()
        warn.alert = True
        warn.label(text=f"Large network ({embed.human_size(info['size'])}): consider Linked File mode", icon="ERROR")
    newer = info["newer"]
    if newer and newer != "same":
        row = box.row()
        row.alert = newer in {"file", "embedded"}
        row.label(text={"file": "Linked file is newer than the embedded copy",
                        "embedded": "Embedded copy is newer than the linked file",
                        "file missing": "Linked file not found: " + info["file"]}.get(newer, newer), icon="INFO")
        if newer in {"file", "embedded"}:
            col = box.column(align=True)
            for choice, text in (("USE_EMBEDDED", "Use embedded"), ("USE_FILE", "Use file"),
                                 ("MATCH_FILE", "Make file match embedded")):
                op = col.operator("minihou.resolve_embed", text=text)
                op.modifier_name, op.choice = mod.name, choice
    elif newer == "same":
        box.label(text="Linked file matches the embedded copy", icon="CHECKMARK")
    if info["state"] == "conflict":
        row = box.row()
        row.alert = True
        row.label(text="Waiting for your answer in the MiniHou window", icon="QUESTION")
    for line, icon in ([(info["error"], "ERROR")] if info["error"] else []) + \
            [("Missing file: " + m, "ERROR") for m in info["missing"][:4]] + \
            [("Outside the .blend folder: " + m, "ERROR") for m in info["outside"][:4]]:
        row = box.row()
        row.alert = icon == "ERROR"
        row.label(text=line[:160], icon=icon)
    col = box.column(align=True)
    col.operator("minihou.embed_from_file", icon="IMPORT").modifier_name = mod.name
    col.operator("minihou.extract_graph", icon="EXPORT").modifier_name = mod.name
    col.operator("minihou.switch_embed_mode", icon="ARROW_LEFTRIGHT",
                 text="Switch to Linked File" if info["mode"] == embed.EMBEDDED else "Switch to Embedded"
                 ).modifier_name = mod.name
    on = embed.frozen(ng)
    box.operator("minihou.toggle_freeze", text=f"Freeze result: {'On' if on else 'Off'}", icon="FREEZE",
                 depress=on).modifier_name = mod.name
    verts, cost = embed.frozen_cost(ng)
    if verts and on:
        box.label(text=f"Saved cooked mesh: {verts:,} points, ~{embed.human_size(cost)}")


class MINIHOU_OT_toggle_freeze(_NetworkOp, bpy.types.Operator):
    bl_idname = "minihou.toggle_freeze"
    bl_label = "Toggle Freeze Result"
    bl_description = ("Freeze result: the .blend keeps the last cooked mesh so it opens correctly without MiniHou. "
                      "Off: the cooked mesh is not saved (smaller file); the object shows its input until MiniHou cooks")
    bl_options = {"REGISTER", "UNDO"}

    def execute(self, context):
        def go(context, mod, ng):
            ng[embed.FREEZE] = not embed.frozen(ng)
            return "Freeze result " + ("on" if ng[embed.FREEZE] else "off")
        return self.run(context, go)


_classes = (MINIHOU_OT_embed_from_file, MINIHOU_OT_extract_graph, MINIHOU_OT_switch_embed_mode,
            MINIHOU_OT_resolve_embed, MINIHOU_OT_toggle_freeze)


def register():
    for c in _classes:
        bpy.utils.register_class(c)


def unregister():
    for c in reversed(_classes):
        bpy.utils.unregister_class(c)
