bl_info = {
    "name": "MiniHou Link",
    "author": "BlenderHoudini",
    "version": (0, 1, 0),
    "blender": (4, 2, 0),
    "location": "3D Viewport > Sidebar > MiniHou",
    "description": "Live link to the MiniHou procedural geometry engine",
    "category": "Object",
}

import bpy
from bpy.props import IntProperty, StringProperty

from . import runtime
from .protocol import DEFAULT_PORT


class MINIHOU_OT_connect(bpy.types.Operator):
    bl_idname = "minihou.connect"
    bl_label = "Connect"
    bl_description = "Connect to the MiniHou engine and create the proxy objects"

    def execute(self, context):
        wm = context.window_manager
        runtime.get_runtime().connect(wm.minihou_host, wm.minihou_port)
        return {"FINISHED"}


class MINIHOU_OT_disconnect(bpy.types.Operator):
    bl_idname = "minihou.disconnect"
    bl_label = "Disconnect"

    def execute(self, context):
        runtime.get_runtime().disconnect()
        return {"FINISHED"}


class MINIHOU_OT_resync(bpy.types.Operator):
    bl_idname = "minihou.resync"
    bl_label = "Resend Everything"
    bl_description = "Ask the engine for a full geometry update"

    def execute(self, context):
        rt = runtime.get_runtime()
        if rt.client:
            rt.client.resync()
        return {"FINISHED"}


class MINIHOU_OT_select_proxy(bpy.types.Operator):
    bl_idname = "minihou.select_proxy"
    bl_label = "Select Proxy Object"

    def execute(self, context):
        from . import proxy
        _out, obj, _mod = proxy.ensure_objects()
        for o in context.view_layer.objects:
            o.select_set(False)
        obj.select_set(True)
        context.view_layer.objects.active = obj
        return {"FINISHED"}


class MINIHOU_PT_main(bpy.types.Panel):
    bl_label = "MiniHou"
    bl_idname = "MINIHOU_PT_main"
    bl_space_type = "VIEW_3D"
    bl_region_type = "UI"
    bl_category = "MiniHou"

    def draw(self, context):
        from . import proxy
        layout = self.layout
        wm = context.window_manager
        rt = runtime.get_runtime()
        col = layout.column(align=True)
        col.prop(wm, "minihou_host", text="Host")
        col.prop(wm, "minihou_port", text="Port")
        row = layout.row(align=True)
        if rt.client is None:
            row.operator("minihou.connect", icon="LINKED")
        else:
            row.operator("minihou.disconnect", icon="UNLINKED")
            row.operator("minihou.resync", text="", icon="FILE_REFRESH")
        icon = {"connected": "CHECKMARK", "connecting": "SORTTIME"}.get(rt.state, "X")
        layout.label(text=f"{rt.state}" + (f" ({rt.detail})" if rt.detail and rt.state != "connected" else ""), icon=icon)
        if rt.client is not None and rt.applied_count:
            box = layout.box()
            c = rt.counts
            box.label(text=f"{c['point']:,} points  {c['prim']:,} prims")
            t = rt.last_timings
            box.label(text=f"cook {rt.last_cook_ms:.1f} ms   write {t.get('total_ms', 0):.1f} ms")
            box.label(text=f"latency {rt.last_latency_ms:.1f} ms   updates {rt.applied_count}")
        if rt.last_error:
            box = layout.box()
            box.alert = True
            for line in rt.last_error[:300].split("\n")[:4]:
                box.label(text=line, icon="ERROR")
        proxy_obj = bpy.data.objects.get(proxy.PROXY_OBJECT)
        if proxy_obj is not None:
            layout.operator("minihou.select_proxy", icon="RESTRICT_SELECT_OFF")
            mod = proxy_obj.modifiers.get(proxy.MODIFIER)
            if mod is not None and mod.node_group is not None and rt.params:
                m = proxy.mapping(mod.node_group)
                col = layout.column()
                for p in rt.params:
                    ident = m.get(p["name"])
                    if ident and ident in mod.keys():
                        col.prop(mod, f'["{ident}"]', text=p.get("label", p["name"]))


_classes = (MINIHOU_OT_connect, MINIHOU_OT_disconnect, MINIHOU_OT_resync, MINIHOU_OT_select_proxy, MINIHOU_PT_main)


def register():
    bpy.types.WindowManager.minihou_host = StringProperty(name="Host", default="127.0.0.1")
    bpy.types.WindowManager.minihou_port = IntProperty(name="Port", default=DEFAULT_PORT, min=1, max=65535)
    for c in _classes:
        bpy.utils.register_class(c)


def unregister():
    runtime.shutdown()
    for c in reversed(_classes):
        bpy.utils.unregister_class(c)
    del bpy.types.WindowManager.minihou_host
    del bpy.types.WindowManager.minihou_port
