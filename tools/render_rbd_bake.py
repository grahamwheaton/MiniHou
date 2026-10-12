"""Render a native RBD bake, including arbitrary frame subsets, with no engine/addon.

blender --background --factory-startup demo.blend --python-exit-code 1
    --python tools/render_rbd_bake.py -- --start 11 --end 13 --output /path/frame_
Optional acceptance comparison: --reference live.png --reference-frame 12.
"""
import argparse
import json
from pathlib import Path
import sys

import bpy
import numpy as np

parser = argparse.ArgumentParser()
parser.add_argument('--start', type=int)
parser.add_argument('--end', type=int)
parser.add_argument('--output', required=True)
parser.add_argument('--reference')
parser.add_argument('--reference-frame', type=int)
args = parser.parse_args(sys.argv[sys.argv.index('--')+1:] if '--' in sys.argv else [])
scene = bpy.context.scene
scene.frame_start = args.start if args.start is not None else scene.frame_start
scene.frame_end = args.end if args.end is not None else scene.frame_end
if scene.frame_end < scene.frame_start:
    raise ValueError('End frame precedes start frame')
scene.render.filepath = str(Path(args.output).resolve())
scene.render.image_settings.file_format = 'PNG'
scene.render.use_motion_blur = False  # discrete per-frame meshes cannot interpolate motion blur
# Required before render depsgraph construction when jumping to a new range start.
scene.frame_set(scene.frame_start)
bpy.context.view_layer.update()
bpy.ops.render.render(animation=True)

if args.reference:
    frame = args.reference_frame if args.reference_frame is not None else scene.frame_start
    actual = bpy.data.images.load(scene.render.frame_path(frame=frame), check_existing=False)
    expected = bpy.data.images.load(str(Path(args.reference).resolve()), check_existing=False)
    a = np.empty(len(actual.pixels), np.float32)
    b = np.empty(len(expected.pixels), np.float32)
    actual.pixels.foreach_get(a)
    expected.pixels.foreach_get(b)
    if a.shape != b.shape:
        raise AssertionError('Live and baked image dimensions differ')
    error = float(np.abs(a-b).mean())
    if error >= .005 or float(a.reshape(-1, 4)[:, :3].std()) < .02:
        raise AssertionError(f'Baked render does not match live output: mean pixel error {error}')
    result = dict(offline_animation=True, start=scene.frame_start, end=scene.frame_end,
                  compared_frame=frame, mean_pixel_error=error, engine_required=False)
    Path(args.output).with_name('offline_render_validation.json').write_text(json.dumps(result, indent=2), encoding='utf8')
    print('RBD_OFFLINE_PIXEL_ACCEPTANCE', json.dumps(result), flush=True)
