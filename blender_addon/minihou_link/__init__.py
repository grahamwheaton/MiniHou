bl_info = {
    "name": "MiniHou Link",
    "author": "MiniHou",
    "version": (0, 3, 0),
    "blender": (4, 2, 0),
    "location": "Properties > Modifiers > MiniHou",
    "description": "Live link to the MiniHou procedural geometry engine",
    "category": "Object",
}

import json

import bpy
from bpy.app.handlers import persistent
from bpy.props import BoolProperty, IntProperty, StringProperty

from . import runtime, modifiers, geonodes_ui, network_ui, scene_objects
from .server_control import ServerController
from .protocol import DEFAULT_PORT

_server = ServerController()


def _save_endpoint(self, context):
    if context.scene:
        context.scene.minihou_link_host = self.minihou_host
        context.scene.minihou_link_port = self.minihou_port


@persistent
def _restore_endpoint(_dummy):
    scene = getattr(bpy.context, "scene", None)
    wm = getattr(bpy.context, "window_manager", None)
    if scene and wm:
        host, port = scene.minihou_link_host, scene.minihou_link_port
        wm.minihou_host, wm.minihou_port = host, port


def _restore_endpoint_timer():
    _restore_endpoint(None)
    return None


def _auto_connect_timer():
    try:
        addon = bpy.context.preferences.addons.get(__package__)
        wm = bpy.context.window_manager
        if addon and wm:
            runtime.get_runtime().poll_auto_connect(addon.preferences.auto_connect_minihou, wm.minihou_port)
    except Exception as exc:
        # Keep polling through file loads and temporarily restricted Blender contexts.
        print(f"[minihou] auto-connect: {exc}")
    return 2.0


class MINIHOU_Preferences(bpy.types.AddonPreferences):
    bl_idname = __package__

    auto_connect_minihou: BoolProperty(name="Auto-connect to MiniHou", default=True,
        description="Connect to the local engine automatically; Disconnect pauses this until Connect is pressed")

    auto_connect: BoolProperty(name="Find MiniHou automatically", default=True,
        update=lambda self, context: modifiers.disconnect())
    manual_host: StringProperty(name="Host", default="127.0.0.1",
        update=lambda self, context: modifiers.disconnect())
    manual_port: IntProperty(name="Link Port", default=8765, min=1, max=65535,
        update=lambda self, context: modifiers.disconnect())
    show_sidebar: BoolProperty(name="Show legacy sidebar tools", default=False)

    external_python: StringProperty(name="External Python / Portable Runtime", subtype="FILE_PATH",
        description="Prepared Python 3.11+ executable with MiniHou and numpy; never Blender's executable")
    project_root: StringProperty(name="MiniHou Project Folder", subtype="DIR_PATH",
        description="Folder containing minihou/__main__.py and the node library")

    def draw(self, context):
        self.layout.prop(self, "auto_connect_minihou")
        self.layout.prop(self, "auto_connect")
        if not self.auto_connect:
            self.layout.prop(self, "manual_host")
            self.layout.prop(self, "manual_port")
            self.layout.prop(context.scene, "minihou_http_port", text="Editor Port")
        self.layout.prop(self, "show_sidebar")
        self.layout.label(text="Advanced: launch an external Python engine from Blender")
        self.layout.prop(self, "external_python")
        self.layout.prop(self, "project_root")
        self.layout.label(text="Prepare dependencies in the external runtime, not Blender.")


def _server_redraw():
    runtime._tag_redraw()
    return 0.25 if _server.snapshot()["state"] in {"starting", "stopping"} else None


class MINIHOU_OT_start_server(bpy.types.Operator):
    bl_idname = "minihou.start_server"
    bl_label = "Start Server"
    bl_description = "Start configured external Python, or reuse a compatible engine for this graph"

    def execute(self, context):
        addon = context.preferences.addons.get(__package__)
        prefs = addon.preferences if addon else None
        try:
            _server.start(bpy.path.abspath(prefs.external_python) if prefs and prefs.external_python else "",
                          bpy.path.abspath(prefs.project_root) if prefs and prefs.project_root else "",
                          bpy.path.abspath(context.scene.minihou_graph_path),
                          host=context.window_manager.minihou_host, port=context.window_manager.minihou_port,
                          http_port=context.scene.minihou_http_port)
            if not bpy.app.timers.is_registered(_server_redraw):
                bpy.app.timers.register(_server_redraw, first_interval=0.1)
        except Exception as exc:
            self.report({"ERROR"}, str(exc))
            return {"CANCELLED"}
        return {"FINISHED"}


class MINIHOU_OT_stop_server(bpy.types.Operator):
    bl_idname = "minihou.stop_server"
    bl_label = "Stop Owned Server"
    bl_description = "Stop only the engine process this add-on started"

    def execute(self, context):
        _server.stop()
        if not bpy.app.timers.is_registered(_server_redraw):
            bpy.app.timers.register(_server_redraw, first_interval=0.1)
        return {"FINISHED"}


class MINIHOU_OT_open_editor(bpy.types.Operator):
    bl_idname = "minihou.open_editor"
    bl_label = "Open Editor"
    bl_description = "Open the ready server's graph editor in your external browser"

    def execute(self, context):
        try:
            _server.open_editor()
        except Exception as exc:
            self.report({"ERROR"}, str(exc))
            return {"CANCELLED"}
        return {"FINISHED"}


class MINIHOU_OT_add_modifier(bpy.types.Operator):
    bl_idname = "minihou.add_modifier"
    bl_label = "Add MiniHou Modifier"
    bl_description = "Add a MiniHou modifier to the selected meshes"
    new_graph: BoolProperty(default=False, options={"HIDDEN"})
    bl_options = {"REGISTER", "UNDO"}

    @classmethod
    def poll(cls, context):
        return context.mode == "OBJECT" and any(o.type == "MESH" and o.library is None
                                               for o in context.selected_objects)

    def execute(self, context):
        added = 0
        try:
            for obj in context.selected_objects:
                if obj.type == "MESH" and obj.library is None:
                    modifiers.add_modifier(obj, "" if self.new_graph else context.scene.minihou_graph_path)
                    added += 1
            modifiers.pump(force=True)
        except Exception as exc:
            self.report({"ERROR"}, f"Added {added} modifier(s); {exc}")
            return {"FINISHED"} if added else {"CANCELLED"}
        if modifiers.last_error():
            self.report({"WARNING"}, f"Modifiers added; cook failed: {modifiers.last_error()}")
            return {"FINISHED"}
        self.report({"INFO"}, f"Added {added} independent MiniHou modifier(s)")
        return {"FINISHED"}


class MINIHOU_OT_refresh_modifiers(bpy.types.Operator):
    bl_idname = "minihou.refresh_modifiers"
    bl_label = "Cook Stack Now"
    bl_description = "Synchronously update interactive stack caches before inspecting or rendering a still"

    def execute(self, context):
        modifiers.pump(force=True)
        if modifiers.last_error():
            self.report({"ERROR"}, modifiers.last_error())
            return {"CANCELLED"}
        return {"FINISHED"}


class MINIHOU_OT_reload_modifier(bpy.types.Operator):
    bl_idname = "minihou.reload_modifier"
    bl_label = "Reload Graph"
    bl_description = "Adopt this instance's saved graph template, preserving matching parameter overrides"
    bl_options = {"REGISTER", "UNDO"}
    modifier_name: StringProperty()

    def execute(self, context):
        mod = context.object.modifiers.get(self.modifier_name) if context.object else None
        if mod is None or not modifiers.is_modifier(mod):
            return {"CANCELLED"}
        try:
            modifiers.reload_graph(mod)
            modifiers.pump(force=True)
            if modifiers.last_error():
                raise RuntimeError(modifiers.last_error())
        except Exception as exc:
            self.report({"ERROR"}, str(exc))
            return {"CANCELLED"}
        return {"FINISHED"}


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
    bl_label = f"MiniHou V{'.'.join(str(n) for n in bl_info['version'])}"
    bl_idname = "MINIHOU_PT_main"
    bl_space_type = "VIEW_3D"
    bl_region_type = "UI"
    bl_category = "MiniHou"

    @classmethod
    def poll(cls, context):
        addon = context.preferences.addons.get(__package__)
        return bool(addon and addon.preferences.show_sidebar)

    def draw(self, context):
        from . import proxy
        layout = self.layout
        wm = context.window_manager
        rt = runtime.get_runtime()
        box = layout.box()
        box.label(text="External Engine")
        box.prop(wm, "minihou_host", text="Host")
        box.prop(wm, "minihou_port", text="Link Port")
        box.prop(context.scene, "minihou_graph_path", text="Graph")
        box.prop(context.scene, "minihou_http_port", text="Editor Port")
        addon = context.preferences.addons.get(__package__)
        if addon:
            box.prop(addon.preferences, "external_python")
            box.prop(addon.preferences, "project_root")
        server = _server.snapshot()
        row = box.row(align=True)
        row.enabled = server["state"] not in {"starting", "stopping"}
        row.operator("minihou.start_server", icon="PLAY")
        edit = row.row(align=True)
        edit.enabled = server["state"] == "ready"
        edit.operator("minihou.open_editor", icon="URL")
        if server["owned"] or server["state"] == "ready":
            box.operator("minihou.stop_server", icon="PAUSE",
                         text="Stop Owned Server" if server["owned"] else "Detach from Server")
        box.label(text=f"Server: {server['state']}")
        if server["detail"]:
            for line in server["detail"][:400].splitlines()[:4]:
                box.label(text=line)
        stack = layout.box()
        stack.label(text="Mesh Stack Instances")
        stack.operator("minihou.add_modifier", icon="MODIFIER")
        stack.operator("minihou.refresh_modifiers", icon="FILE_REFRESH")
        stack.label(text="Interactive caches; cook before still renders.")
        if context.object:
            for mod in context.object.modifiers:
                if not modifiers.is_modifier(mod):
                    continue
                item = stack.box()
                item.label(text=mod.name)
                item.label(text=modifiers.status(mod)[:160])
                source_owner = modifiers.source_owner(mod)
                if source_owner is not None:
                    network_ui.draw_objects(item, mod)
                network_ui.draw(item, mod)
                mapping = proxy.mapping(mod.node_group)
                for p in modifiers.parameters(mod):
                    owner = proxy.input_owner(mod, mapping.get(p["name"], ""))
                    if owner is not None:
                        item.prop(owner[0], owner[1], text=p.get("label", p["name"]))
        layout.label(text="Legacy Proxy Link")
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
        if rt.client is not None and rt.watched:
            box = layout.box()
            box.label(text="Inputs read from Blender", icon="IMPORT")
            for name in rt.watched:
                box.label(text=f"{name}: {rt.object_info.get(name, 'waiting')}", icon="OBJECT_DATA")
        if rt.last_error:
            box = layout.box()
            box.alert = True
            for line in rt.last_error[:300].split("\n")[:4]:
                box.label(text=line, icon="ERROR")
        n = scene_objects.counts()
        if n["total"]:                                          # objects made by scene_objects nodes
            box = layout.box()
            box.label(text=f"Scene objects: {n['total']} ({n['hidden']} parked)", icon="OUTLINER_OB_GROUP_INSTANCE")
            row = box.row()
            row.operator("minihou.cleanup_scene_objects", text="Delete parked").everything = False
            row.operator("minihou.cleanup_scene_objects", text="Delete all").everything = True
        proxy_obj = bpy.data.objects.get(proxy.PROXY_OBJECT)
        if proxy_obj is not None:
            layout.operator("minihou.select_proxy", icon="RESTRICT_SELECT_OFF")
            mod = proxy_obj.modifiers.get(proxy.MODIFIER)
            if mod is not None and mod.node_group is not None and rt.params:
                m = proxy.mapping(mod.node_group)
                col = layout.column()
                for p in rt.params:
                    owner = proxy.input_owner(mod, m.get(p["name"], ""))
                    if owner is not None:
                        col.prop(owner[0], owner[1], text=p.get("label", p["name"]))


class MINIHOU_OT_cleanup_scene_objects(bpy.types.Operator):
    bl_idname = "minihou.cleanup_scene_objects"
    bl_label = "Clean Up Scene Objects"
    bl_description = ("Delete the objects MiniHou's scene_objects nodes made. Objects the graph no longer asks for are only hidden "
                      "(they may carry your modifiers); this removes them for good")
    bl_options = {"REGISTER", "UNDO"}
    everything: bpy.props.BoolProperty(name="Everything", default=False, description="Delete the visible ones too, not only the parked ones")

    def execute(self, context):
        gone = scene_objects.cleanup(self.everything)
        self.report({"INFO"}, f"Deleted {gone} scene object(s)")
        return {"FINISHED"}


class MINIHOU_OT_bake_rbd(bpy.types.Operator):
    bl_idname = 'minihou.bake_rbd'
    bl_label = 'Bake RBD Frame Range'
    bl_description = 'Bake scene frame range to native mesh samples for reliable offline animation rendering (512 MiB limit)'
    bl_options = {'REGISTER', 'UNDO'}

    def execute(self, context):
        from .rbd_bake import bake
        try:
            col = bake(context.object, context.scene.frame_start, context.scene.frame_end)
            self.report({'INFO'}, f'Baked {len(col.objects)} frames to {col.name}; source retained and hidden')
            return {'FINISHED'}
        except Exception as exc:
            self.report({'ERROR'}, str(exc))
            return {'CANCELLED'}


ADDON_VERSION = ".".join(str(n) for n in bl_info["version"])


class MINIHOU_PT_modifier_sources(bpy.types.Panel):
    bl_label = f"MiniHou V{ADDON_VERSION}"
    bl_idname = "MINIHOU_PT_modifier_sources"
    bl_space_type = "PROPERTIES"
    bl_region_type = "WINDOW"
    bl_context = "modifier"

    @classmethod
    def poll(cls, context):
        return context.object is not None and context.object.type == "MESH"

    def draw(self, context):
        self.layout.operator("minihou.add_modifier", icon="ADD").new_graph = True
        self.layout.label(text="Open MiniHou, then edit a modifier's network")
        if any(modifiers.has_rbd(m) for m in context.object.modifiers):
            self.layout.operator('minihou.bake_rbd', icon='REC')
            self.layout.label(text='RBD: synchronous timeline; bake before animation render')
        for mod in context.object.modifiers:
            if not modifiers.is_modifier(mod):
                continue
            box = self.layout.box()
            box.label(text=mod.name, icon="MODIFIER")
            network_ui.draw(box, mod)
            box.label(text=modifiers.status(mod)[:160])
        # Every placement of a Geometry Nodes modifier gets its own collapsible section with its own values (its node
        # sockets; a socket wired to the modifier's Group Input shows that modifier input). Rename a section by
        # editing its Label field (the node label).
        from . import geonodes
        for mod in context.object.modifiers:
            if mod.type != "NODES" or modifiers.is_modifier(mod):
                continue
            try:
                sections = geonodes.placement_controls(mod)
            except (ReferenceError, ValueError, KeyError):
                continue
            for section in sections:
                draw_placement(self.layout, mod, section)


def draw_placement(layout, mod, section):
    node = section["node"]
    title = f"{mod.name} / {section['title']}"
    body = None
    if hasattr(layout, "panel"):                           # Blender 4.1+: real collapsible sub-panel
        header, body = layout.panel(f"minihou_{mod.name}_{section['key']}"[:60], default_closed=False)
        header.label(text=title, icon="NODETREE")
    else:
        body = layout.box()
        body.label(text=title, icon="NODETREE")
    if body is None:
        return
    body.prop(node, "label", text="Label")
    from . import geonodes
    body.label(text=geonodes.status(node)[:160])
    for label, data, prop in section["rows"]:
        if data is None:
            body.label(text=f"{label}: {prop}")
        else:
            body.prop(data, prop, text=label)


_classes = (MINIHOU_Preferences, MINIHOU_OT_start_server, MINIHOU_OT_stop_server, MINIHOU_OT_open_editor,
            MINIHOU_OT_add_modifier, MINIHOU_OT_refresh_modifiers, MINIHOU_OT_reload_modifier,
            MINIHOU_OT_connect, MINIHOU_OT_disconnect, MINIHOU_OT_resync, MINIHOU_OT_select_proxy,
            MINIHOU_PT_main, MINIHOU_PT_modifier_sources, MINIHOU_OT_bake_rbd, MINIHOU_OT_cleanup_scene_objects)


def register():
    bpy.types.Scene.minihou_link_host = StringProperty(name="Host", default="127.0.0.1")
    bpy.types.Scene.minihou_link_port = IntProperty(name="Port", default=DEFAULT_PORT, min=1, max=65535)
    bpy.types.WindowManager.minihou_host = StringProperty(name="Host", default="127.0.0.1", update=_save_endpoint)
    bpy.types.WindowManager.minihou_port = IntProperty(name="Port", default=DEFAULT_PORT, min=1, max=65535,
                                                     update=_save_endpoint)
    bpy.types.Scene.minihou_graph_path = StringProperty(name="Graph", subtype="FILE_PATH")
    bpy.types.Scene.minihou_http_port = IntProperty(name="Editor Port", default=8766, min=1, max=65535)
    for c in _classes:
        bpy.utils.register_class(c)
    if _restore_endpoint not in bpy.app.handlers.load_post:
        bpy.app.handlers.load_post.append(_restore_endpoint)
    # Blender restricts context access while enabling an installed add-on.
    if not bpy.app.timers.is_registered(_restore_endpoint_timer):
        bpy.app.timers.register(_restore_endpoint_timer, first_interval=0.0)
    modifiers.register()
    geonodes_ui.register()
    network_ui.register()
    if not bpy.app.timers.is_registered(_auto_connect_timer):
        bpy.app.timers.register(_auto_connect_timer, first_interval=2.0, persistent=True)


def unregister():
    if bpy.app.timers.is_registered(_auto_connect_timer):
        bpy.app.timers.unregister(_auto_connect_timer)
    network_ui.unregister()
    geonodes_ui.unregister()
    modifiers.unregister()
    _server.stop(wait=False)  # Owned-process cleanup stays on its worker during unregister.
    if bpy.app.timers.is_registered(_server_redraw):
        bpy.app.timers.unregister(_server_redraw)
    runtime.shutdown()
    if _restore_endpoint in bpy.app.handlers.load_post:
        bpy.app.handlers.load_post.remove(_restore_endpoint)
    if bpy.app.timers.is_registered(_restore_endpoint_timer):
        bpy.app.timers.unregister(_restore_endpoint_timer)
    for c in reversed(_classes):
        bpy.utils.unregister_class(c)
    del bpy.types.WindowManager.minihou_host
    del bpy.types.WindowManager.minihou_port
    del bpy.types.Scene.minihou_graph_path
    del bpy.types.Scene.minihou_http_port
    del bpy.types.Scene.minihou_link_host
    del bpy.types.Scene.minihou_link_port
