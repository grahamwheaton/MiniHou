import bpy, json, sys
F=sys.argv[-1]
bpy.ops.wm.open_mainfile(filepath=F)
out=[]
for ng in bpy.data.node_groups:
    if ng.bl_idname!='GeometryNodeTree': 
        out.append({"name":ng.name,"type":ng.bl_idname}); continue
    ins=[];outs=[]
    for it in ng.interface.items_tree:
        if it.item_type!='SOCKET': continue
        d={"name":it.name,"type":it.socket_type}
        (ins if it.in_out=='INPUT' else outs).append(d)
    nodes={}
    for n in ng.nodes:
        k=n.bl_idname
        if k=='GeometryNodeGroup' and n.node_tree: k='GROUP:'+n.node_tree.name
        nodes[k]=nodes.get(k,0)+1
    a=ng.asset_data
    out.append({"name":ng.name,"asset":bool(a),"desc":(a.description if a else ng.description) or "","catalog":(a.catalog_simple_name if a else ""),
      "tags":[t.name for t in a.tags] if a else [],"in":ins,"out":outs,"nnodes":len(ng.nodes),"nodes":nodes,"modifier":bool(getattr(ng,'is_modifier',False)),"tool":bool(getattr(ng,'is_tool',False))})
json.dump(out, open('groups.json', 'w', encoding='utf-8'), indent=1)
print(len(out),'groups')
