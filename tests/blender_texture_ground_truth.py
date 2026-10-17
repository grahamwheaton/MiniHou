import bpy
import os
import sys
import struct
import zlib
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent / 'blender_addon'))
from minihou_link import material_export

bpy.ops.object.select_all(action='SELECT')
bpy.ops.object.delete(use_global=False)
bpy.ops.mesh.primitive_plane_add(size=2)
plane = bpy.context.object
print('PLANE_UVS', [(tuple(plane.data.vertices[loop.vertex_index].co), tuple(plane.data.uv_layers.active.data[i].uv)) for i,loop in enumerate(plane.data.loops)])
image = bpy.data.images.new('orientation', width=64, height=64, alpha=True)
pixels = []
for y in range(64):
    for x in range(64):
        rgb = ((1,0,0) if x < 32 else (0,1,0)) if y >= 32 else ((0,0,1) if x < 32 else (1,1,0))
        if (x-12)**2 + (y-51)**2 < 16:
            rgb = (1,1,1)
        pixels.extend((*rgb,1))
image.pixels.foreach_set(pixels)
image.filepath_raw = os.path.abspath('.tmp_blender_orientation_source.png')
image.file_format = 'PNG'
encoded = material_export.image_bytes(image, 'linear')
offset = 8
compressed = bytearray()
while offset < len(encoded):
    size = struct.unpack_from('>I', encoded, offset)[0]
    kind = encoded[offset+4:offset+8]
    if kind == b'IDAT':
        compressed.extend(encoded[offset+8:offset+8+size])
    offset += 12 + size
rows = zlib.decompress(compressed)
def exported_rgb(x, y):
    start = y * (1 + 64*4) + 1 + x*4
    return tuple(rows[start:start+3])
for label,x,y,expected in [('top_left',16,16,(255,0,0)),('top_right',48,16,(0,255,0)),('bottom_left',16,48,(0,0,255)),('bottom_right',48,48,(255,255,0)),('dot',12,12,(255,255,255))]:
    actual = exported_rgb(x,y)
    print('EXPORTED', label, actual)
    assert actual == expected, (label, actual, expected)
image.save()
mat = bpy.data.materials.new('marked quadrants')
mat.use_nodes = True
tex = mat.node_tree.nodes.new('ShaderNodeTexImage')
tex.image = image
mat.node_tree.links.new(tex.outputs['Color'], mat.node_tree.nodes.get('Principled BSDF').inputs['Base Color'])
plane.data.materials.append(mat)
bpy.ops.object.camera_add(location=(0,0,4))
camera = bpy.context.object
camera.data.type = 'ORTHO'
camera.data.ortho_scale = 2.4
bpy.context.scene.camera = camera
bpy.ops.object.light_add(type='AREA', location=(0,0,3))
bpy.context.object.data.energy = 500
bpy.context.object.data.shape = 'DISK'
bpy.context.object.data.size = 5
scene = bpy.context.scene
scene.render.engine = 'BLENDER_EEVEE'
scene.render.resolution_x = 128
scene.render.resolution_y = 128
scene.render.resolution_percentage = 100
scene.view_settings.view_transform = 'Standard'
scene.render.image_settings.file_format = 'PNG'
scene.render.filepath = os.path.abspath('.tmp_blender_orientation_render.png')
bpy.ops.render.render(write_still=True)
result = bpy.data.images.load(bpy.path.abspath(scene.render.filepath), check_existing=False)
for label,x,y,dominant in [('top_left',48,80,0),('top_right',80,80,1),('bottom_left',48,48,2),('bottom_right',80,48,None),('dot',32,96,None)]:
    index=(y*128+x)*4
    rgba = tuple(result.pixels[index:index+4])
    print('SAMPLE',label,tuple(round(v,3) for v in rgba))
    if dominant is not None:
        assert rgba[dominant] > max(rgba[(dominant+1)%3], rgba[(dominant+2)%3]) + .3
    elif label == 'bottom_right':
        assert min(rgba[:2]) > rgba[2] + .3
    else:
        assert min(rgba[:3]) > .95
