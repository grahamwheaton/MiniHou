"""Run in a normal Blender window: actual sidebar and modifier Properties panel draws.

MINIHOU_ENGINE_PYTHON must name the prepared external runtime. Pass -- --out result.json.
The test closes only its own Blender process; no OS input or user preferences are changed.
"""
import argparse
import json
from pathlib import Path
import sys
import time
import traceback

import bpy

ROOT = Path(__file__).resolve().parents[1]
sys.path[:0] = [str(ROOT), str(ROOT / "blender_addon")]
parser = argparse.ArgumentParser()
parser.add_argument("--out", required=True)
args = parser.parse_args(sys.argv[sys.argv.index("--") + 1:])
output = Path(args.out).resolve()
output.parent.mkdir(parents=True, exist_ok=True)
result = {"blender": bpy.app.version_string, "sidebar_draws": 0, "modifier_draws": 0,
          "errors": [], "ok": False}
import minihou_link
from minihou_link import modifiers
from tests.test_blender_addon import Engine


def counted(draw, key):
    def wrapped(self, context):
        result[key] += 1
        try:
            return draw(self, context)
        except Exception:
            result["errors"].append(traceback.format_exc())
            raise
    return wrapped


minihou_link.MINIHOU_PT_main.draw = counted(minihou_link.MINIHOU_PT_main.draw, "sidebar_draws")
minihou_link.MINIHOU_PT_modifier_sources.draw = counted(
    minihou_link.MINIHOU_PT_modifier_sources.draw, "modifier_draws")
minihou_link.register()
bpy.context.preferences.view.show_splash = False
engine = Engine(ROOT / "examples" / "modifiers" / "scatter_object.json")
bpy.context.window_manager.minihou_port = engine.port
bpy.context.scene.minihou_graph_path = str(ROOT / "examples" / "modifiers" / "scatter_object.json")
bpy.ops.object.select_all(action="SELECT")
bpy.ops.object.delete(use_global=False)
bpy.ops.mesh.primitive_cube_add(size=0.5, location=(4, 0, 0))
source = bpy.context.object
source.name = "Editable Cube Source"
bpy.ops.mesh.primitive_plane_add(size=4)
plane = bpy.context.object
plane.name = "Scatter Plane"
mod = modifiers.add_modifier(plane, bpy.context.scene.minihou_graph_path, port=engine.port)
modifiers.set_source(mod, source)
modifiers.pump(force=True)
assert not modifiers.last_error(), modifiers.last_error()
start = time.monotonic()
probe = None


def finish():
    try:
        bpy.context.view_layer.update()
        instances = [item for item in bpy.context.evaluated_depsgraph_get().object_instances
                     if item.is_instance and item.parent and item.parent.original == plane]
        result["instances"] = len(instances)
        result["source"] = modifiers.get_source(mod).name
        result["ok"] = (not result["errors"] and result["sidebar_draws"] > 0
                        and result["modifier_draws"] > 0 and len(instances) == 24)
        shot = output.with_suffix(".png")
        bpy.ops.screen.screenshot(filepath=str(shot), check_existing=False)
        result["screenshot"] = str(shot)
    except Exception:
        result["errors"].append(traceback.format_exc())
        result["ok"] = False
    output.write_text(json.dumps(result, indent=2), encoding="utf-8")
    print("SOURCE_UI_RESULT " + json.dumps(result), flush=True)
    engine.stop()
    # Do not unregister panels while their UI regions are drawing.
    bpy.ops.wm.quit_blender()


def tick():
    global probe
    try:
        for window in bpy.context.window_manager.windows:
            for area in window.screen.areas:
                if area.type == "PROPERTIES":
                    area.spaces.active.context = "MODIFIER"
                    area.tag_redraw()
                if area.type == "VIEW_3D":
                    area.spaces.active.show_region_ui = True
                    area.spaces.active.region_3d.view_distance = 9
                    ui = next((r for r in area.regions if r.type == "UI"), None)
                    category = getattr(ui, "active_panel_category", "") if ui else ""
                    if probe is None and category and category != "UNSUPPORTED":
                        probe = type("MINIHOU_PT_source_ui_probe", (bpy.types.Panel,), {
                            "bl_idname": "MINIHOU_PT_source_ui_probe", "bl_label": "MiniHou source UI test",
                            "bl_space_type": "VIEW_3D", "bl_region_type": "UI", "bl_category": category,
                            "bl_order": -100, "draw": minihou_link.MINIHOU_PT_main.draw})
                        bpy.utils.register_class(probe)
                    area.tag_redraw()
        if (result["sidebar_draws"] > 2 and result["modifier_draws"] > 2
                and time.monotonic() - start > 3) or time.monotonic() - start > 20:
            finish()
            return None
    except Exception:
        result["errors"].append(traceback.format_exc())
        finish()
        return None
    return 0.2


bpy.app.timers.register(tick, first_interval=0.2)
