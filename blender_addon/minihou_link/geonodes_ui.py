"""Geometry Nodes editor controls for staged MiniHou placements."""
import bpy
from . import geonodes, modifiers


def _active(context):
    tree = getattr(context.space_data, "edit_tree", None)
    node = tree.nodes.active if tree else None
    return node if node and geonodes.is_group(node) else None


class MINIHOU_OT_add_group(bpy.types.Operator):
    bl_idname = "minihou.add_group"
    bl_label = "Add MiniHou Group"
    bl_description = "Add selected engine graph; wire Geometry input to process native upstream mesh"
    bl_options = {"REGISTER", "UNDO"}

    @classmethod
    def poll(cls, context):
        tree = getattr(context.space_data, "edit_tree", None)
        return tree is not None and tree.bl_idname == "GeometryNodeTree" and tree.library is None

    def execute(self, context):
        try:
            tree = context.space_data.edit_tree
            node = geonodes.add_group(tree, context.scene.minihou_graph_path)
            for other in tree.nodes:
                other.select = False
            node.location = context.space_data.cursor_location
            node.select = True
            tree.nodes.active = node
        except Exception as exc:
            self.report({"ERROR"}, str(exc))
            return {"CANCELLED"}
        return {"FINISHED"}


class MINIHOU_OT_reload_group(bpy.types.Operator):
    bl_idname = "minihou.reload_group"
    bl_label = "Reload Group Graph"
    bl_description = "Adopt saved graph template for this placement, retaining matching values and links"
    bl_options = {"REGISTER", "UNDO"}

    @classmethod
    def poll(cls, context):
        return _active(context) is not None

    def execute(self, context):
        try:
            geonodes.reload_graph(_active(context))
            modifiers.pump(force=True)
            if modifiers.last_error():
                raise RuntimeError(modifiers.last_error())
        except Exception as exc:
            self.report({"ERROR"}, str(exc))
            return {"CANCELLED"}
        return {"FINISHED"}


class MINIHOU_OT_edit_group_network(bpy.types.Operator):
    bl_idname = "minihou.edit_group_network"
    bl_label = "Edit Network"
    bl_description = "Open this placement's graph template in the shared external editor; save then Reload Group Graph"

    @classmethod
    def poll(cls, context):
        return _active(context) is not None

    def execute(self, context):
        from . import _server, network_ui
        try:
            network_ui.router.start(_server.snapshot(), _active(context).node_tree[modifiers.PATH])
            if not bpy.app.timers.is_registered(network_ui._redraw):
                bpy.app.timers.register(network_ui._redraw, first_interval=0.1)
        except Exception as exc:
            self.report({"ERROR"}, str(exc))
            return {"CANCELLED"}
        self.report({"INFO"}, "Opening graph template; save edits, then Reload Group Graph")
        return {"FINISHED"}


class MINIHOU_PT_geonodes(bpy.types.Panel):
    bl_label = "MiniHou Wired Groups"
    bl_idname = "MINIHOU_PT_geonodes"
    bl_space_type = "NODE_EDITOR"
    bl_region_type = "UI"
    bl_category = "MiniHou"

    @classmethod
    def poll(cls, context):
        return getattr(context.space_data, "tree_type", "") == "GeometryNodeTree"

    def draw(self, context):
        layout = self.layout
        layout.prop(context.scene, "minihou_graph_path")
        layout.operator("minihou.add_group", icon="NODETREE")
        layout.operator("minihou.refresh_modifiers", text="Cook Wired Groups / Stack Now")
        layout.label(text="Wire mesh Geometry input and output.")
        layout.label(text="Staged updates; cook before rendering.")
        node = _active(context)
        if node:
            box = layout.box()
            box.label(text=node.label or node.name)
            text = geonodes.status(node)
            box.alert = text.startswith("Error")
            for offset in range(0, min(len(text), 400), 55):
                box.label(text=text[offset:offset + 55])
            box.operator("minihou.edit_group_network", icon="URL")
            box.operator("minihou.reload_group")
            box.label(text="Template editing; reload to adopt changes.")
            from . import network_ui
            state = network_ui.router.snapshot()
            if state["path"] == node.node_tree[modifiers.PATH]:
                import textwrap
                for line in textwrap.wrap(state["detail"], 48):
                    box.label(text=line)


_classes = (MINIHOU_OT_add_group, MINIHOU_OT_reload_group, MINIHOU_OT_edit_group_network, MINIHOU_PT_geonodes)


def register():
    geonodes.register()
    for cls in _classes:
        bpy.utils.register_class(cls)


def unregister():
    for cls in reversed(_classes):
        bpy.utils.unregister_class(cls)
    geonodes.unregister()
