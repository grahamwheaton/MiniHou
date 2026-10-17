"""Add / Rename / Remove Input operators and their panel rows (add-on 0.3.1; see input_define.py)."""
import bpy
from bpy.props import BoolProperty, EnumProperty, FloatProperty, FloatVectorProperty, IntProperty, StringProperty

from . import input_define

_KIND_ITEMS = [(k, k.capitalize(), f"A {k} input") for k in input_define.KINDS]
_ALL_SUBTYPES = sorted({s for v in input_define.SUBTYPES.values() for s in v})


def _subtype_items(self, context):
    return [("NONE", "None", "")] + [(s, s.capitalize(), "") for s in input_define.SUBTYPES.get(self.kind, ())]


def network(context, modifier, tree):
    """(node group, modifier or None) the operator acts on."""
    if modifier:
        obj = context.object
        mod = obj.modifiers.get(modifier) if obj is not None else None
        if mod is None or mod.node_group is None:
            raise ValueError(f"modifier {modifier!r} not found on the active object")
        return mod.node_group, mod
    ng = bpy.data.node_groups.get(tree)
    if ng is None:
        raise ValueError(f"node group {tree!r} not found")
    return ng, None


class _Target:
    modifier: StringProperty(options={"HIDDEN", "SKIP_SAVE"})
    tree: StringProperty(options={"HIDDEN", "SKIP_SAVE"})

    def run(self, context, msg):
        try:
            ng, mod = network(context, self.modifier, self.tree)
            entry, route = input_define.define(ng, msg, mod=mod)
        except Exception as exc:
            self.report({"ERROR"}, str(exc))
            return {"CANCELLED"}
        where = {"app": "in MiniHou (undo there with Ctrl+Z)",
                 "offline": "in the embedded network (MiniHou applies it when it opens this network)",
                 "offline_old_app": "offline: the connected MiniHou app cannot define inputs (update it)"}[route]
        self.report({"WARNING"} if route == "offline_old_app" else {"INFO"}, f"{entry['name']}: stored {where}")
        return {"FINISHED"}


class MINIHOU_OT_add_input(_Target, bpy.types.Operator):
    bl_idname = "minihou.add_input"
    bl_label = "Add Input"
    bl_description = "Add an input to this network's MiniHou input node (also in MiniHou when connected)"
    bl_options = {"REGISTER", "UNDO"}

    input_name: StringProperty(name="Name", default="Input")
    kind: EnumProperty(name="Type", items=_KIND_ITEMS, default="float")
    float_value: FloatProperty(name="Default")
    int_value: IntProperty(name="Default")
    bool_value: BoolProperty(name="Default")
    vector: FloatVectorProperty(name="Default", size=3)
    color: FloatVectorProperty(name="Default", size=4, subtype="COLOR", min=0.0, default=(1.0, 1.0, 1.0, 1.0))
    string: StringProperty(name="Default")
    id_name: StringProperty(name="Default")
    options: StringProperty(name="Options", description="Menu items, comma separated", default="A, B")
    use_min: BoolProperty(name="Min")
    min_value: FloatProperty(name="Min")
    use_max: BoolProperty(name="Max")
    max_value: FloatProperty(name="Max", default=1.0)
    subtype: EnumProperty(name="Subtype", items=_subtype_items)
    description: StringProperty(name="Tooltip")

    def invoke(self, context, event):
        return context.window_manager.invoke_props_dialog(self, width=360)

    def draw(self, context):
        layout = self.layout
        layout.prop(self, "input_name")
        layout.prop(self, "kind")
        kind = self.kind
        if kind in input_define.ID_KINDS:
            layout.prop_search(self, "id_name", bpy.data, input_define.ID_KINDS[kind], text="Default")
        elif kind in {"float", "int", "bool", "string"}:
            layout.prop(self, {"float": "float_value", "int": "int_value", "bool": "bool_value", "string": "string"}[kind])
        elif kind in {"vector", "rotation"}:
            layout.prop(self, "vector")
        elif kind == "color":
            layout.prop(self, "color")
        elif kind == "menu":
            layout.prop(self, "options")
        else:
            layout.label(text="No default for this type")
        if kind in {"float", "int", "vector", "rotation"}:
            for use, value in (("use_min", "min_value"), ("use_max", "max_value")):
                row = layout.row()
                row.prop(self, use)
                sub = row.row()
                sub.active = getattr(self, use)
                sub.prop(self, value, text="")
        if kind in input_define.SUBTYPES:
            layout.prop(self, "subtype")
        layout.prop(self, "description")
        if kind in {"geometry", "bundle", "closure", "matrix"}:
            layout.label(text="Definable; some engines cannot consume this type yet", icon="INFO")

    def execute(self, context):
        try:
            draft = input_define.build_draft(
                self.kind, self.input_name, float_value=self.float_value, int_value=self.int_value,
                bool_value=self.bool_value, vector=self.vector, color=self.color, string=self.string,
                id_name=self.id_name, options=self.options, use_min=self.use_min, min_value=self.min_value,
                use_max=self.use_max, max_value=self.max_value, subtype=self.subtype, doc=self.description)
        except ValueError as exc:
            self.report({"ERROR"}, str(exc))
            return {"CANCELLED"}
        return self.run(context, input_define.request("add", input=draft))


class MINIHOU_OT_rename_input(_Target, bpy.types.Operator):
    bl_idname = "minihou.rename_input"
    bl_label = "Rename Input"
    bl_description = "Rename a MiniHou input; its stable id keeps values, links and placement sockets"
    bl_options = {"REGISTER", "UNDO"}

    ident: StringProperty(options={"HIDDEN"})
    new_name: StringProperty(name="Name")

    def invoke(self, context, event):
        return context.window_manager.invoke_props_dialog(self)

    def execute(self, context):
        return self.run(context, input_define.request("rename", ident=self.ident, name=self.new_name))


class MINIHOU_OT_remove_input(_Target, bpy.types.Operator):
    bl_idname = "minihou.remove_input"
    bl_label = "Remove Input"
    bl_description = "Remove a MiniHou input (parameters linked to it return to their own values)"
    bl_options = {"REGISTER", "UNDO"}

    ident: StringProperty(options={"HIDDEN"})

    def invoke(self, context, event):
        return context.window_manager.invoke_confirm(self, event)

    def execute(self, context):
        return self.run(context, input_define.request("remove", ident=self.ident))


def draw(layout, ng, *, modifier="", tree=""):
    """Add Input button plus one row per dynamic input (rename / remove)."""
    col = layout.column(align=True)
    op = col.operator("minihou.add_input", icon="ADD")
    op.modifier, op.tree = modifier, tree
    for e in input_define.network_inputs(ng):
        row = col.row(align=True)
        row.label(text=f"{e['name']} ({e['kind']})")
        op = row.operator("minihou.rename_input", text="", icon="GREASEPENCIL")
        op.modifier, op.tree, op.ident, op.new_name = modifier, tree, e["id"], e["name"]
        op = row.operator("minihou.remove_input", text="", icon="X")
        op.modifier, op.tree, op.ident = modifier, tree, e["id"]


_classes = (MINIHOU_OT_add_input, MINIHOU_OT_rename_input, MINIHOU_OT_remove_input)


def register():
    for cls in _classes:
        bpy.utils.register_class(cls)


def unregister():
    for cls in reversed(_classes):
        bpy.utils.unregister_class(cls)
