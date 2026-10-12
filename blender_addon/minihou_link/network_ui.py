"""Small UI integration kept separate from modifier cooking and simulation code."""
import textwrap

import bpy
from bpy.props import StringProperty

from . import modifiers, runtime
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
        except Exception as exc:
            self.report({"ERROR"}, str(exc))
            return {"CANCELLED"}
        if not bpy.app.timers.is_registered(_redraw):
            bpy.app.timers.register(_redraw, first_interval=0.1)
        self.report({"INFO"}, "Opening live modifier; edits update this Blender object")
        return {"FINISHED"}


def draw(layout, mod):
    row = layout.column(align=True)
    row.operator("minihou.edit_network", icon="URL").modifier_name = mod.name
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


def register():
    bpy.utils.register_class(MINIHOU_OT_edit_network)


def unregister():
    if bpy.app.timers.is_registered(_redraw):
        bpy.app.timers.unregister(_redraw)
    bpy.utils.unregister_class(MINIHOU_OT_edit_network)
