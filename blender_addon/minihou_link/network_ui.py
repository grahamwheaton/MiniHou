"""Small UI integration kept separate from modifier cooking and simulation code."""
import textwrap

import bpy
from bpy.props import StringProperty

from . import embed, embed_ui, modifiers, runtime
from .network_editor import NetworkEditor

router = NetworkEditor()


def _redraw():
    runtime._tag_redraw()
    return 0.1 if router.snapshot()["state"] == "opening" else None


class MINIHOU_OT_edit_network(bpy.types.Operator):
    bl_idname = "minihou.edit_network"
    bl_label = "Edit Network"
    bl_description = "Edit this modifier live in MiniHou using its actual Blender input mesh"
    modifier_name: StringProperty()

    def execute(self, context):
        mod = context.object.modifiers.get(self.modifier_name) if context.object else None
        if mod is None or not modifiers.is_modifier(mod):
            self.report({"ERROR"}, "MiniHou modifier no longer exists")
            return {"CANCELLED"}
        try:
            server, path = modifiers.edit_live(context.object, mod)
            router.start(server, path, live=True)
        except embed.ConflictPending as exc:
            self.report({"WARNING"}, str(exc))
            return {"FINISHED"}
        except Exception as exc:
            self.report({"ERROR"}, str(exc))
            return {"CANCELLED"}
        if not bpy.app.timers.is_registered(_redraw):
            bpy.app.timers.register(_redraw, first_interval=0.1)
        self.report({"INFO"}, "Opening live modifier; edits update this Blender object")
        return {"FINISHED"}


class MINIHOU_OT_choose_object(bpy.types.Operator):
    bl_idname = "minihou.choose_object"
    bl_label = "Choose MiniHou Object"
    bl_options = {"REGISTER", "UNDO"}
    modifier_name: StringProperty()
    binding: StringProperty()
    object_name: StringProperty(name="Object")

    def invoke(self, context, event):
        return context.window_manager.invoke_props_dialog(self)

    def draw(self, context):
        self.layout.prop_search(self, "object_name", bpy.data, "objects", text="Object")
        self.layout.label(text="Leave empty to clear the input")

    def execute(self, context):
        mod = context.object.modifiers.get(self.modifier_name) if context.object else None
        if mod is None or not modifiers.is_modifier(mod):
            return {"CANCELLED"}
        if self.binding not in modifiers._object_keys(mod) and self.binding != "source":
            self.report({"ERROR"}, "This input no longer exists")
            return {"CANCELLED"}
        obj = bpy.data.objects.get(self.object_name) if self.object_name else None
        try:
            if self.object_name and obj is None:
                raise ValueError("Object no longer exists")
            if obj is not None:
                if obj.type != "MESH":
                    raise ValueError("Choose a mesh object")
                modifiers._validate_source(context.object, obj)
            modifiers.set_object(mod, self.binding, obj)
        except (ValueError, modifiers.SourceBindingError) as exc:
            self.report({"ERROR"}, str(exc))
            return {"CANCELLED"}
        return {"FINISHED"}


def draw_objects(layout, mod):
    from . import proxy
    for key in dict.fromkeys(["source"] + modifiers._object_keys(mod)):
        if key == "source":
            item = modifiers._source_socket(mod)
        else:
            import json
            ident = json.loads(mod.node_group.get("minihou_object_sockets", "{}")).get(key)
            item = next((s for s in proxy._sockets(mod.node_group, "INPUT") if s.identifier == ident), None)
        if item is None:
            continue
        obj = proxy.get_input(mod, item.identifier) if proxy.has_input(mod, item.identifier) else None
        row = layout.row()
        row.label(text=item.name)
        op = row.operator("minihou.choose_object", text=obj.name if obj else "Choose Object...", icon="OBJECT_DATA")
        op.modifier_name, op.binding, op.object_name = mod.name, key, obj.name if obj else ""


def draw(layout, mod):
    draw_objects(layout, mod)
    row = layout.column(align=True)
    row.operator("minihou.edit_network", text="Open in MiniHou", icon="URL").modifier_name = mod.name
    row.operator("minihou.reload_modifier", text="Reload Graph", icon="FILE_REFRESH").modifier_name = mod.name
    layout.label(text="Live editor: this modifier")
    state = router.snapshot()
    if state["path"] == modifiers.graph_path(mod):
        status = layout.column()
        status.alert = state["state"] == "error"
        region = bpy.context.region
        width = max(18, min(72, (region.width - 50) // 7)) if region else 45
        for line in textwrap.wrap(state["detail"], width):
            status.label(text=line)
    embed_ui.draw(layout, mod)


def register():
    bpy.utils.register_class(MINIHOU_OT_edit_network)
    bpy.utils.register_class(MINIHOU_OT_choose_object)
    embed_ui.register()


def unregister():
    embed_ui.unregister()
    if bpy.app.timers.is_registered(_redraw):
        bpy.app.timers.unregister(_redraw)
    bpy.utils.unregister_class(MINIHOU_OT_choose_object)
    bpy.utils.unregister_class(MINIHOU_OT_edit_network)
