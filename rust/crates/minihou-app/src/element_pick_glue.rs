//! Viewport element picking and selection-range glue.
use crate::{picking, ui, viewport};
use crate::viewport::Scene;
use crate::ui::CameraInfo;
use eframe::egui;
use minihou_app::Camera;

pub(super) fn point_pickable(visibility:viewport::ObjectVisibility,ranges:&[(String,std::ops::Range<u32>)],selected:&std::collections::BTreeSet<String>,id:u32)->bool {
    visibility==viewport::ObjectVisibility::ShowAll || ranges.iter().any(|(owner,range)| selected.contains(owner) && range.contains(&id))
}
pub(super) fn loop_cut_has_marks(g:&minihou_core::Geometry)->bool { g.vertex.get("edge_group_viewport_loop_cut").is_some() }
pub(super) fn loop_cut_selection(g:&minihou_core::Geometry,range:&std::ops::Range<u32>)->picking::Selection {
    let topo_edges=minihou_core::topo::edges(&g.topology,g.npoints);
    let marks=g.vertex.get("edge_group_viewport_loop_cut").and_then(|attr|attr.as_i32());
    let mut selected=vec![false;topo_edges.len()];
    if let Some(marks)=marks { for (vertex,&edge) in topo_edges.of_vertex.iter().enumerate() {
        if edge>=0 && marks.get(vertex).is_some_and(|&mark|mark!=0) { selected[edge as usize]=true; }
    } }
    let edges=topo_edges.points.chunks_exact(2).enumerate()
        .filter(|(i,pair)|selected[*i] && range.contains(&(pair[0] as u32)) && range.contains(&(pair[1] as u32)))
        .map(|(_,pair)|(pair[0] as u32,pair[1] as u32)).collect();
    picking::Selection { mode:ui::SelectionMode::Edges,edges,..Default::default() }
}
pub(super) fn selection_exists_on_scene(selection:&picking::Selection,scene:&Scene)->bool {
    selection.points.iter().all(|&p|(p as usize)<scene.positions().len()/3)
        && selection.tris.iter().all(|&t|(t as usize)<scene.triangle_indices().len()/3)
        && selection.edges.iter().all(|&(a,b)|scene.edge_indices().chunks_exact(2)
            .any(|edge|(edge[0]==a && edge[1]==b)||(edge[0]==b && edge[1]==a)))
}
pub(super) fn selection_in_range(selection:&picking::Selection,scene:&Scene,range:&std::ops::Range<u32>)->picking::Selection {
    let mut result=selection.clone();
    result.points.retain(|id|range.contains(id));
    result.edges.retain(|(a,b)|range.contains(a)&&range.contains(b));
    result.tris.retain(|id|scene.triangle_indices().get(*id as usize*3).is_some_and(|p|range.contains(p)));
    result
}
pub(super) fn pick_displayed(scene:&Scene,o:[f32;3],d:[f32;3],visibility:viewport::ObjectVisibility,ranges:&[(String,std::ops::Range<u32>)],selected:&std::collections::BTreeSet<String>)->Option<picking::Hit> {
    pick_displayed_culled(scene,o,d,visibility,ranges,selected,false)
}
/// Back-face culling is off for selection: the renderer draws both sides, so a visible face with the opposite winding (open meshes, flipped normals) must stay pickable. The nearest hit already hides faces behind it.
pub(super) fn pick_displayed_culled(scene:&Scene,o:[f32;3],d:[f32;3],visibility:viewport::ObjectVisibility,ranges:&[(String,std::ops::Range<u32>)],selected:&std::collections::BTreeSet<String>,cull:bool)->Option<picking::Hit> {
    let base=scene.pick_bvh.pick(scene.positions(),scene.triangle_indices(),o,d,|tri|
        !scene.packed_point(tri[0]) && tri.iter().all(|id|point_pickable(visibility,ranges,selected,*id)));
    let packed=scene.pick_packed(o,d,|range|point_pickable(visibility,ranges,selected,range.start));
    let _=cull;
    match (base,packed) {(Some(a),Some(b))=>Some(if a.t<=b.t {a}else{b}), (a,b)=>a.or(b)}
}
/// When X-ray exposes overlapping faces, another click on the selected front face reaches the next surface.
pub(super) fn pick_displayed_behind(scene:&Scene,o:[f32;3],d:[f32;3],front:picking::Hit,
    visibility:viewport::ObjectVisibility,ranges:&[(String,std::ops::Range<u32>)],selected:&std::collections::BTreeSet<String>)->Option<picking::Hit> {
    let after=front.t+front.t.abs().max(1.)*1e-5;
    let shifted=[0,1,2].map(|k|o[k]+d[k]*after);
    pick_displayed_culled(scene,shifted,d,visibility,ranges,selected,false)
        .map(|mut hit|{hit.t+=after;hit})
}
/// A projected element is visible when no displayed triangle lies in front of it.
pub(super) fn element_visible_at(scene:&Scene,camera:CameraInfo,rect:egui::Rect,point:[f32;3],
    visibility:viewport::ObjectVisibility,ranges:&[(String,std::ops::Range<u32>)],selected:&std::collections::BTreeSet<String>,ignore_occlusion:bool)->bool {
    let Some(screen)=ui::transform_gizmo::project(camera,rect,point) else { return false; };
    let (origin,dir)=ui::transform_gizmo::ray(camera,rect,screen);
    picking::element_in_front(&scene.pick_bvh,scene.positions(),scene.triangle_indices(),origin,dir,point,
        scene.radius.max(1.)*1e-4,ignore_occlusion,
        |tri|tri.iter().all(|id|point_pickable(visibility,ranges,selected,*id)))
}
/// After an extrude cook the original selection stays when its ids still exist (Blender keeps the faces selected).
pub(super) fn keep_extrude_selection(before:&picking::Selection,scene:&Scene)->Option<picking::Selection> {
    (!before.is_empty() && selection_exists_on_scene(before,scene)).then(||before.clone())
}
pub(super) fn pick_displayed_wire(scene:&Scene, camera:CameraInfo, rect:egui::Rect, pointer:egui::Pos2,
    visibility:viewport::ObjectVisibility,ranges:&[(String,std::ops::Range<u32>)],selected:&std::collections::BTreeSet<String>,mode:ui::SelectionMode,xray:bool)->Option<picking::WireHit> {
    // without X-ray, points and edges behind the mesh surface under the pointer are not pickable
    let max_depth=if xray {f32::INFINITY} else {
        let (o,d)=ui::transform_gizmo::ray(camera,rect,pointer);
        pick_displayed(scene,o,d,visibility,ranges,selected).map_or(f32::INFINITY,|hit|{
            let depth=hit.t*(0..3).map(|k|d[k]*camera.ray_forward[k]).sum::<f32>();
            let depth=if camera.ortho {depth.max(0.001)} else {depth};
            // The ray samples the face at the cursor while a point marker is
            // centred a few pixels away. Allow that local surface slope without
            // making points on the far side of the object pickable.
            let half_height=if camera.ortho {
                camera.ray_up.iter().map(|v|v*v).sum::<f32>().sqrt()
            } else { depth*0.4_f32.tan() };
            let pixel_depth=2.0*half_height/rect.height().max(1.0);
            depth*1.002+5.0*pixel_depth+scene.radius*1e-4
        })
    };
    picking::pick_wire_occluded(scene.positions(),scene.line_primitive_indices(),pointer,
        |p| ui::transform_gizmo::project(camera,rect,p).map(|screen| {
            let depth=(0..3).map(|k|(p[k]-camera.ray_origin[k])*camera.ray_forward[k]).sum::<f32>();
            (screen, if camera.ortho { depth.max(0.001) } else { depth })
        }),
        |id| point_pickable(visibility,ranges,selected,id),
        matches!(mode,ui::SelectionMode::Object|ui::SelectionMode::Points),
        matches!(mode,ui::SelectionMode::Object|ui::SelectionMode::Edges),max_depth)
}

pub(super) fn wire_selection(hit:picking::WireHit,mode:ui::SelectionMode)->picking::Selection {
    let mut out=picking::Selection {mode,..Default::default()};
    match (mode,hit) {
        (ui::SelectionMode::Points,picking::WireHit::Point(id)) => out.points.push(id),
        (ui::SelectionMode::Edges,picking::WireHit::Edge(a,b)) => out.edges.push((a,b)),
        _ => {}
    }
    out
}


pub(super) fn active_range<'a>(active:&str,ranges:&'a [(String,std::ops::Range<u32>)])->Option<&'a std::ops::Range<u32>> {
    ranges.iter().find(|(id,_)|id==active).map(|(_,range)|range)
}

pub(super) fn navigation_point(camera:&Camera,scene:Option<&Scene>,hidden:bool,visibility:viewport::ObjectVisibility,
    ranges:&[(String,std::ops::Range<u32>)],selected:&std::collections::BTreeSet<String>,rect:egui::Rect,pos:egui::Pos2)->Option<[f32;3]> {
    let nx=(pos.x-rect.left())/rect.width()*2.-1.; let ny=1.-(pos.y-rect.top())/rect.height()*2.;
    let (origin,dir)=camera.ray(nx,ny,rect.width()/rect.height());
    if hidden {return None;}
    let scene=scene?;
    let hit=pick_displayed(scene,origin,dir,visibility,ranges,selected)?;
    Some([0,1,2].map(|k|origin[k]+dir[k]*hit.t))
}
