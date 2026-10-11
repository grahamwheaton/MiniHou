"""Real Blender/live socket/timeline/bake/render acceptance. Uses only its own processes.

MINIHOU_ENGINE_PYTHON=<isolated python> blender --background --factory-startup
    --python-exit-code 1 --python tools/blender_rbd_smoke.py
Run without --background and with -- --playback to exercise animation_play as well.
"""
import json
import os
from pathlib import Path
import sys
import time

ROOT = Path(__file__).resolve().parents[1]
sys.path[:0] = [str(ROOT), str(ROOT / 'blender_addon')]
os.environ.setdefault('MINIHOU_ENGINE_PYTHON', str(ROOT / 'build/runtime/python312/python.exe'))
import bpy
import numpy as np
from mathutils import Vector
import minihou_link
from minihou_link import modifiers, rbd_bake
from tests.test_blender_addon import Engine, evaluated_positions

DEST = ROOT / 'dist/rbd_acceptance'
DEST.mkdir(parents=True, exist_ok=True)
engine = None


def setup():
    global engine
    bpy.ops.wm.read_factory_settings(use_empty=True)
    minihou_link.register()
    engine = Engine(ROOT / 'examples/rbd_wall.json')
    bpy.context.window_manager.minihou_port = engine.port
    bpy.context.window_manager.minihou_host = '127.0.0.1'
    mesh = bpy.data.meshes.new('RBD empty input')
    obj = bpy.data.objects.new('RBD Brick Wall', mesh)
    bpy.context.scene.collection.objects.link(obj)
    bpy.context.view_layer.objects.active = obj
    obj.select_set(True)
    for name, color in [('Brick', (.5, .12, .055, 1)), ('Projectile', (.04, .2, .6, 1)), ('Floor', (.2, .22, .25, 1))]:
        mat = bpy.data.materials.new(name)
        mat.diffuse_color = color
        mat.use_nodes = True
        mat.node_tree.nodes['Principled BSDF'].inputs['Base Color'].default_value = color
        mat.node_tree.nodes['Principled BSDF'].inputs['Roughness'].default_value = .65
        obj.data.materials.append(mat)
    mod = modifiers.add_modifier(obj, str(ROOT / 'examples/rbd_wall_modifier.json'), port=engine.port)
    # Add distinct face material indices without changing any checked-in demo.
    graph = json.loads(mod.node_group[modifiers.GRAPH])
    for src, value in [('wall', 0), ('ball', 1), ('floor', 2)]:
        tag = src + '_material'
        graph['nodes'][tag] = dict(type='wrangle', inputs={'geo': src},
                                   params={'run_over': 'primitives', 'snippet': f'i@material_index = {value}; i@sharp_face = {int(src != "ball")}'})
        target = {'wall':'bricks', 'ball':'projectile', 'floor':'ground'}[src]
        graph['nodes'][target]['inputs']['geo'] = tag
    mod.node_group[modifiers.GRAPH] = json.dumps(graph)
    scene = bpy.context.scene
    scene.frame_start, scene.frame_end = 1, 96
    scene.render.fps = 24
    modifiers.cook_frame(scene, 1)
    assert modifiers.status(mod) == 'Ready', modifiers.status(mod)
    return obj, mod


def camera_lights():
    scene = bpy.context.scene
    bpy.ops.object.camera_add(location=(11, -17, 11))
    camera = bpy.context.object
    camera.rotation_euler = (Vector((0, 1.5, 1))-camera.location).to_track_quat('-Z', 'Y').to_euler()
    camera.data.lens = 40
    scene.camera = camera
    bpy.ops.object.light_add(type='AREA', location=(2, -4, 12))
    bpy.context.object.data.energy = 2200
    bpy.context.object.data.shape = 'DISK'
    bpy.context.object.data.size = 8
    scene.world = bpy.data.worlds.new('RBD World')
    scene.world.use_nodes = True
    scene.world.node_tree.nodes['Background'].inputs[0].default_value = (.3, .3, .3, 1)
    scene.world.node_tree.nodes['Background'].inputs[1].default_value = .5
    scene.render.engine = 'BLENDER_EEVEE'
    scene.render.resolution_x, scene.render.resolution_y = 640, 480
    scene.render.resolution_percentage = 100


def run():
    obj, mod = setup()
    scene = bpy.context.scene
    first = evaluated_positions(obj).copy()
    # No explicit pump: test the actual registered frame-change handler.
    scene.frame_set(30)
    impact = evaluated_positions(obj).copy()
    assert not np.allclose(first, impact), 'timeline did not update live output'
    scene.frame_set(72)
    fallen = evaluated_positions(obj).copy()
    centers0 = first[:384].reshape(48, 8, 3).mean(1)
    centers = fallen[:384].reshape(48, 8, 3).mean(1)
    moved = int(np.count_nonzero(np.linalg.norm(centers-centers0, axis=1) > .3))
    assert moved > 8, f'only {moved} bricks moved'
    scene.frame_set(1)
    np.testing.assert_array_equal(first, evaluated_positions(obj))
    scene.frame_set(72)
    np.testing.assert_array_equal(fallen, evaluated_positions(obj))
    assert len(modifiers._cache(mod).data.materials) == 3
    assert {p.material_index for p in modifiers._cache(mod).data.polygons} == {0, 1, 2}
    # Duplicate modifier ownership, same embedded graph and different exposed velocity.
    second = obj.copy(); second.data = obj.data.copy()
    bpy.context.scene.collection.objects.link(second)
    second.name = 'Independent RBD'
    modifiers.pump(port=engine.port, force=True)
    m2 = next(m for m in second.modifiers if modifiers.is_modifier(m))
    assert modifiers.instance_id(mod) != modifiers.instance_id(m2)
    assert modifiers._cache(mod) != modifiers._cache(m2)
    bpy.data.objects.remove(second, do_unlink=True)
    modifiers.pump(port=engine.port, force=True)
    camera_lights()
    for frame in (1, 12, 30, 72):
        modifiers.cook_frame(scene, frame)
        scene.render.filepath = str(DEST / f'wall_live_{frame:04d}.png')
        bpy.ops.render.render(write_still=True)
    bpy.context.view_layer.objects.active = obj
    collection = rbd_bake.bake(obj, 1, 96)
    assert len(collection.objects) == 96
    baked = next(o for o in collection.objects if o.get('minihou_baked_frame') == 72)
    P = np.empty(len(baked.data.vertices)*3, np.float32)
    baked.data.vertices.foreach_get('co', P)
    np.testing.assert_array_equal(P.reshape(-1, 3), fallen)
    assert {p.material_index for p in baked.data.polygons} == {0, 1, 2}
    # Offline bake must work with the engine disconnected and addon handlers removed.
    engine.stop()
    modifiers.unregister()
    scene.frame_set(72)
    visible = [o for o in collection.objects if max(abs(v) for v in o.scale) > 0]
    assert len(visible) == 1 and visible[0] == baked
    scene.render.filepath = str(DEST / 'wall_baked_0072.png')
    bpy.ops.render.render(write_still=True)
    scene.frame_set(1)
    bpy.ops.wm.save_as_mainfile(filepath=str(DEST / 'RBD_Brick_Wall_Baked.blend'))
    report = dict(blender=bpy.app.version_string, frames=96, moved_bricks=moved,
                  live_timeline=True, scrub_exact=True, independent_modifiers=True,
                  materials=True, offline_bake=True, still_renders=True)
    (DEST / 'blender_smoke.json').write_text(json.dumps(report, indent=2), encoding='utf8')
    print('RBD_BLENDER_ACCEPTANCE', json.dumps(report), flush=True)


def playback():
    obj, mod = setup()
    scene = bpy.context.scene
    scene.sync_mode = 'NONE'  # play every evaluated frame, never wall-clock skip
    scene.frame_set(1)
    initial = evaluated_positions(obj).copy()
    started = time.monotonic()
    observed = []
    def animation_op(op, **kwargs):
        window = bpy.context.window_manager.windows[0]
        area = next(a for a in window.screen.areas if a.type == 'VIEW_3D')
        with bpy.context.temp_override(window=window, area=area):
            op(**kwargs)
    def check():
        try:
            observed.append(scene.frame_current)
            if scene.frame_current >= 30:
                animation_op(bpy.ops.screen.animation_cancel, restore_frame=False)
                assert not np.allclose(initial, evaluated_positions(obj))
                assert modifiers.status(mod) == 'Ready', modifiers.status(mod)
                (DEST / 'timeline_playback.json').write_text(json.dumps(dict(
                    actual_animation_play=True, final_frame=scene.frame_current,
                    observed_frames=observed, seconds=time.monotonic()-started)), encoding='utf8')
                engine.stop()
                bpy.ops.wm.quit_blender()
                return None
            if time.monotonic()-started > 45:
                raise RuntimeError('timeline playback timed out')
            return .1
        except Exception:
            import traceback
            traceback.print_exc()
            engine.stop()
            os._exit(1)
    def start_play():
        animation_op(bpy.ops.screen.animation_play)
        bpy.app.timers.register(check, first_interval=.1)
        return None
    bpy.app.timers.register(start_play, first_interval=1.)


if __name__ == '__main__':
    if '--playback' in sys.argv:
        playback()
    else:
        try:
            run()
        finally:
            if engine:
                engine.stop()
