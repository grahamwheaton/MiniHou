"""Native mesh-sequence bake: preserves per-frame attributes/materials/custom normals.

Each sample is an ordinary mesh object with constant visibility keyframes. The baked
collection needs no MiniHou engine or handlers, including background animation renders.
Deliberately bounded and more memory-hungry than transform-only caches.
"""
import bpy
from . import modifiers


def bake(obj, start, end, *, max_mb=512):
    if obj is None or obj.type != 'MESH' or not any(modifiers.has_rbd(m) for m in obj.modifiers):
        raise ValueError('Select a mesh with a MiniHou RBD Solver modifier')
    start, end = int(start), int(end)
    if not start <= end or end - start >= 1000:
        raise ValueError('Bake range must contain 1..1000 integer frames')
    scene = bpy.context.scene
    if scene is None:
        raise ValueError('No scene')
    old_frame = scene.frame_current
    collection = bpy.data.collections.new(obj.name + ' RBD Bake')
    scene.collection.children.link(collection)
    created, meshes, total = [], [], 0
    try:
        for f in range(start, end + 1):
            modifiers.cook_frame(scene, f)
            depsgraph = bpy.context.evaluated_depsgraph_get()
            evaluated = obj.evaluated_get(depsgraph)
            mesh = bpy.data.meshes.new_from_object(evaluated, preserve_all_data_layers=True, depsgraph=depsgraph)
            meshes.append(mesh)
            # Conservative storage estimate, including generic attributes on every domain.
            total += len(mesh.vertices)*64 + len(mesh.loops)*64 + len(mesh.polygons)*64
            total += sum(len(a.data)*16 for a in mesh.attributes)
            if total > max_mb*1024**2:
                raise ValueError(f'RBD bake exceeds {max_mb} MiB estimate; shorten range or simplify geometry')
            baked = bpy.data.objects.new(f'{obj.name} RBD {f:04d}', mesh)
            created.append(baked)
            collection.objects.link(baked)
            baked.matrix_world = obj.matrix_world.copy()
            for frame, hidden in ((start-1, True), (f, False), (f+1, True)):
                baked.hide_viewport = baked.hide_render = hidden
                baked.keyframe_insert('hide_viewport', frame=frame)
                baked.keyframe_insert('hide_render', frame=frame)
            baked['minihou_baked_frame'] = f
        collection['minihou_bake_start'] = start
        collection['minihou_bake_end'] = end
        collection['minihou_bake_bytes_estimate'] = total
        # Hide source only after a complete successful bake; never delete its graph/modifiers.
        obj.hide_render = True
        obj.hide_set(True)
        return collection
    except Exception:
        for baked in created:
            bpy.data.objects.remove(baked, do_unlink=True)
        for mesh in meshes:
            if mesh.users == 0:
                bpy.data.meshes.remove(mesh)
        bpy.data.collections.remove(collection)
        raise
    finally:
        scene.frame_set(old_frame)
