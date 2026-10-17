"""Run with BLENDER_EXE --background --factory-startup --python this_file."""
import bpy
import bmesh
import json
vertices=[]
faces=[]
for z, tilt in [(0.,0.),(0.,0.),(0.00005,0.),(1.,0.),(0.,0.001),(0.,1.)]:
    start=len(vertices)
    vertices.extend([(0.,0.,z),(1.,0.,z),(1.,1.,z+tilt),(0.,1.,z+tilt)])
    faces.append(tuple(range(start,start+4)))
mesh=bpy.data.meshes.new("Coplanar parity fixture")
mesh.from_pydata(vertices,[],faces)
obj=bpy.data.objects.new("Coplanar parity fixture",mesh)
bpy.context.collection.objects.link(obj)
bpy.context.view_layer.objects.active=obj
obj.select_set(True)
bpy.ops.object.mode_set(mode="EDIT")
bpy.ops.mesh.select_mode(type="FACE")
bpy.ops.mesh.select_all(action="DESELECT")
bm=bmesh.from_edit_mesh(mesh)
bm.faces.ensure_lookup_table()
bm.faces[0].select_set(True)
bmesh.update_edit_mesh(mesh)
bpy.ops.mesh.select_similar(type="FACE_COPLANAR",threshold=0.0001)
selected=[f.index for f in bm.faces if f.select]
assert selected==[0,1,2], selected
print("MINIHOU_COPLANAR_PARITY "+json.dumps({"selected":selected,"blender":bpy.app.version_string}))
