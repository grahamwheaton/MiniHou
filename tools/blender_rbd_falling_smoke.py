"""Companion Blender acceptance for live Voronoi pieces and the offline native bake."""
from pathlib import Path
import sys
import json
ROOT = Path(__file__).resolve().parents[1]
sys.path[:0] = [str(ROOT), str(ROOT / 'tools')]
import blender_rbd_smoke as smoke
import bpy
import numpy as np
from minihou_link import modifiers, rbd_bake
from tests.test_blender_addon import evaluated_positions

try:
    obj, mod = smoke.setup()
    obj.name = 'RBD Falling Fracture'
    mod.node_group[modifiers.PATH] = str(ROOT / 'examples/rbd_falling_modifier.json')
    modifiers.reload_graph(mod, port=smoke.engine.port)
    scene = bpy.context.scene
    modifiers.cook_frame(scene, 1)
    first = evaluated_positions(obj).copy()
    scene.frame_set(72)
    last = evaluated_positions(obj).copy()
    assert last.shape == first.shape
    assert np.mean(last[:-8, 2]) < np.mean(first[:-8, 2])-1
    assert last[:-8, 2].min() > -.02
    mesh = modifiers._cache(mod).data
    assert mesh.attributes.get('piece') is not None
    assert mesh.attributes.get('group_inside') is not None
    scene.frame_set(1)
    np.testing.assert_array_equal(first, evaluated_positions(obj))
    smoke.camera_lights()
    for frame in (1, 24, 72):
        modifiers.cook_frame(scene, frame)
        scene.render.filepath = str(smoke.DEST / f'falling_live_{frame:04d}.png')
        bpy.ops.render.render(write_still=True)
    rbd_bake.bake(obj, 1, 96)
    smoke.engine.stop()
    modifiers.unregister()
    scene.frame_set(1)
    bpy.ops.wm.save_as_mainfile(filepath=str(smoke.DEST / 'RBD_Falling_Pieces_Baked.blend'))
    (smoke.DEST / 'falling_smoke.json').write_text(json.dumps(dict(
        blender=bpy.app.version_string, live=True, floor=True, pieces=25, groups=True,
        scrub_exact=True, baked_frames=96)), encoding='utf8')
    print('RBD_FALLING_ACCEPTANCE: passed', flush=True)
finally:
    if smoke.engine:
        smoke.engine.stop()
