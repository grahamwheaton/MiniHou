import uuid
import bpy
from . import geonodes, modifiers


def _active(context):
    tree = getattr(context.space_data, "edit_tree", None)
    node = tree.nodes.active if tree else None
    if node and geonodes.is_group(node):
        return node
    # Tab inside the group: recover the occurrence from the editor breadcrumb path.
    path = getattr(context.space_data, "path", ())
    if tree and len(path) > 1:
        parent = path[-2].node_tree
        candidates = [n for n in parent.nodes if geonodes.is_group(n) and n.node_tree == tree]
        active = parent.nodes.active
        return active if active in candidates else candidates[0] if len(candidates) == 1 else None
    return None


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


class MINIHOU_OT_duplicate_placement(bpy.types.Operator):
    bl_idname = "minihou.duplicate_placement"
    bl_label = "Duplicate Placement"
    bl_description = "Create an independent duplicate of the active MiniHou placement"
    bl_options = {"REGISTER", "UNDO"}

    @classmethod
    def poll(cls, context):
        return _active(context) is not None

    def execute(self, context):
        try:
            node = _active(context)
            tree = getattr(context.space_data, "edit_tree", None)
            if tree is None:
                for ng in bpy.data.node_groups:
                    if node in list(ng.nodes):
                        tree = ng
                        break
            if tree is None:
                raise ValueError("Active tree not found")
            new_node = tree.nodes.new("GeometryNodeGroup")
            new_node.node_tree = node.node_tree.copy()
            new_node.node_tree.use_fake_user = False
            new_node.node_tree[geonodes.KEY] = uuid.uuid4().hex
            geonodes.proxy.ensure_plumbing(new_node.node_tree)
            geonodes._cache(new_node.node_tree)
            geonodes._object_interface(new_node.node_tree)
            for inp in new_node.inputs:
                old_inp = node.inputs.get(inp.name)
                if old_inp is not None and hasattr(inp, "default_value") and hasattr(old_inp, "default_value") and not old_inp.is_linked:
                    try:
                        inp.default_value = old_inp.default_value
                    except Exception:
                        pass
            new_node.location = (node.location.x + 40, node.location.y - 40)
            for other in tree.nodes:
                other.select = False
            new_node.select = True
            tree.nodes.active = new_node
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
    bl_description = "Edit this placement live in MiniHou using its wired Blender geometry"

    @classmethod
    def poll(cls, context):
        return _active(context) is not None

    def execute(self, context):
        from . import _server, network_ui
        try:
            server, path = geonodes.edit_live(context.object, _active(context))
            network_ui.router.start(server, path, live=True)
            if not bpy.app.timers.is_registered(network_ui._redraw):
                bpy.app.timers.register(network_ui._redraw, first_interval=0.1)
        except Exception as exc:
            self.report({"ERROR"}, str(exc))
            return {"CANCELLED"}
        self.report({"INFO"}, "Opening live group network")
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
            box.operator("minihou.duplicate_placement")
            box.operator("minihou.edit_group_network", icon="URL")
            box.operator("minihou.reload_group")
            box.label(text="Edit Network updates this placement live.")
            from . import network_ui
            state = network_ui.router.snapshot()
            if state["path"] == node.node_tree[modifiers.PATH]:
                import textwrap
                for line in textwrap.wrap(state["detail"], 48):
                    box.label(text=line)


_classes = (MINIHOU_OT_add_group, MINIHOU_OT_duplicate_placement, MINIHOU_OT_reload_group, MINIHOU_OT_edit_group_network, MINIHOU_PT_geonodes)


def _context_menu(self, context):
    if _active(context):
        self.layout.separator()
        self.layout.operator("minihou.duplicate_placement")
        self.layout.operator("minihou.edit_group_network", icon="NODETREE")


def register():
    geonodes.register()
    bpy.types.NODE_MT_context_menu.append(_context_menu)
    for cls in _classes:
        bpy.utils.register_class(cls)


def unregister():
    bpy.types.NODE_MT_context_menu.remove(_context_menu)
    for cls in reversed(_classes):
        bpy.utils.unregister_class(cls)
    geonodes.unregister()
