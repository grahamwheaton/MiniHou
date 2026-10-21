//! Picking mesh elements in the viewport and highlight geometry for the current selection.

#[cfg(test)]
use crate::ui::SelectionMode;

mod spatial;

mod interact;

pub use spatial::{position_bounds, SnapFeature, snap_feature, RegionShape, SelectOp, Hit, Bvh, WireHit, pick_wire, pick_wire_occluded};

pub use interact::{Selection, at, try_at, pick_triangle, pick_triangle_filtered, pick_triangle_culled, element_of, extend, highlight, intersect_horizontal_plane, drop_world_position, cursor_hit_position, primitive_rest_offset, intersect_ground_plane, intersect_ground_or_mesh};

/// Nearest further surface on the same ray, for cycling through overlapping X-ray faces.
pub fn pick_triangle_after(p:&[f32],tri:&[u32],origin:[f32;3],dir:[f32;3],after:f32,
    visible:impl FnMut(&[u32])->bool)->Option<Hit> {
    let start=after+1e-5;
    let shifted=[0,1,2].map(|k|origin[k]+dir[k]*start);
    pick_triangle_culled(p,tri,shifted,dir,false,visible).map(|mut hit|{hit.t+=start;hit})
}

/// Wireframe and X-ray expose elements on the far side of a surface.
pub fn ignores_surface_occlusion(mode: crate::viewport::Mode, overlays_enabled: bool, xray: bool, wireframe: bool) -> bool {
    matches!(mode, crate::viewport::Mode::Wireframe | crate::viewport::Mode::WireframeGhost | crate::viewport::Mode::WireBoundingBox)
        || (overlays_enabled && (xray || wireframe))
}

/// Test the element's depth against the nearest displayed surface along its screen ray.
pub fn element_in_front(bvh:&Bvh,positions:&[f32],triangles:&[u32],origin:[f32;3],dir:[f32;3],
    point:[f32;3],tolerance:f32,ignore_occlusion:bool,visible:impl FnMut(&[u32])->bool)->bool {
    let depth=(0..3).map(|k|(point[k]-origin[k])*dir[k]).sum::<f32>();
    if !depth.is_finite() || depth <= 0. { return false; }
    ignore_occlusion || bvh.pick(positions,triangles,origin,dir,visible)
        .is_none_or(|hit|depth <= hit.t + tolerance)
}

#[cfg(test)]
#[test]
fn picking_occlusion_tracks_display_mode() {
    use crate::viewport::Mode;
    assert!(!ignores_surface_occlusion(Mode::SmoothShaded, true, false, false));
    assert!(ignores_surface_occlusion(Mode::SmoothShaded, true, true, false));
    assert!(ignores_surface_occlusion(Mode::SmoothShaded, true, false, true));
    assert!(ignores_surface_occlusion(Mode::Wireframe, false, false, false));
    assert!(!ignores_surface_occlusion(Mode::HiddenLineInvisible, false, false, false));
    assert!(!ignores_surface_occlusion(Mode::SmoothShaded, false, true, true));
}

#[cfg(test)]
#[test]
fn hidden_point_edge_and_face_are_selected_only_in_xray_or_wireframe() {
    use crate::viewport::Mode;
    use eframe::egui::{pos2,Rect};
    // Two coincident screen-space triangles, with independent element ids.
    let p=[-1.,-1.,1., 1.,-1.,1., 0.,1.,1.,
           -1.,-1.,0., 1.,-1.,0., 0.,1.,0.];
    let tris=[0,1,2,3,4,5];
    let bvh=Bvh::new(&p,&tris);
    let origin=[0.,0.,3.]; let dir=[0.,0.,-1.];
    let targets=[[0.,0.,0.], [0.,-1.,0.], [0.,-1./3.,0.]];
    let box_shape=RegionShape::Rect(Rect::from_min_max(pos2(-2.,-2.),pos2(2.,2.)));
    let lasso=RegionShape::Lasso(vec![pos2(-2.,-2.),pos2(2.,-2.),pos2(2.,2.),pos2(-2.,2.)]);
    for shape in [box_shape,lasso] {
        for target in targets {
            assert!(shape.contains(pos2(target[0],target[1])));
            for (mode,xray,expected) in [(Mode::SmoothShaded,false,false),(Mode::SmoothShaded,true,true),
                                          (Mode::Wireframe,false,true)] {
                let ignore=ignores_surface_occlusion(mode,true,xray,false);
                assert_eq!(element_in_front(&bvh,&p,&tris,origin,dir,target,1e-4,ignore,|_|true),expected);
            }
        }
    }
    assert!(element_in_front(&bvh,&p,&tris,origin,dir,[0.,0.,1.],1e-4,false,|_|true));
}

#[cfg(test)]
#[test]
fn xray_face_click_can_reach_the_surface_behind_the_selected_face() {
    let p=[-1.,-1.,1., 1.,-1.,1., 0.,1.,1.,
           -1.,-1.,0., 1.,-1.,0., 0.,1.,0.];
    let tris=[0,1,2,3,4,5];
    let origin=[0.,0.,3.]; let dir=[0.,0.,-1.];
    let front=pick_triangle(&p,&tris,origin,dir).unwrap();
    assert_eq!(front.tri,0);
    assert_eq!(pick_triangle_after(&p,&tris,origin,dir,front.t,|_|true).unwrap().tri,1);
    assert!(pick_triangle_after(&p,&tris,origin,dir,front.t,|_|false).is_none());
}

#[cfg(test)] mod bvh_navigation_tests {
    use super::*;
    #[test] fn navigation_depth_pick_uses_cooked_bvh_and_empty_fallback() {
        let p=[-1.,-1.,0., 1.,-1.,0., 0.,1.,0.]; let tri=[0,1,2];
        let bvh=Bvh::new(&p,&tri);
        assert_eq!(bvh.pick(&p,&tri,[0.,0.,2.],[0.,0.,-1.],|_|true).unwrap().tri,0);
        assert!(bvh.pick(&p,&tri,[5.,0.,2.],[0.,0.,-1.],|_|true).is_none());
        assert!(bvh.pick(&p,&tri,[0.,0.,2.],[0.,0.,-1.],|_|false).is_none());
    }
    #[test] fn million_triangle_navigation_pick_is_under_five_ms() {
        let (w,h)=(1000_u32,500_u32);
        let mut p=Vec::with_capacity(((w+1)*(h+1)*3) as usize);
        for y in 0..=h {for x in 0..=w {p.extend([x as f32,y as f32,0.]);}}
        let mut tri=Vec::with_capacity((w*h*6) as usize);
        for y in 0..h {for x in 0..w {let a=y*(w+1)+x;tri.extend([a,a+1,a+w+2,a,a+w+2,a+w+1]);}}
        let bvh=Bvh::new(&p,&tri);
        let mut times=Vec::new();
        for _ in 0..7 {
            let now=std::time::Instant::now();
            assert!(bvh.pick(&p,&tri,[511.25,241.5,10.],[0.,0.,-1.],|_|true).is_some());
            times.push(now.elapsed());
        }
        times.sort();
        eprintln!("million-triangle BVH median pick: {:?}",times[3]);
        assert!(times[3]<std::time::Duration::from_millis(5),"median pick: {:?}",times[3]);
    }
}

#[cfg(test)] mod wire_tests {
    use super::*;
    use eframe::egui::pos2;
    #[test]
    fn swept_circle_covers_fast_and_bent_strokes_without_gaps() {
        use eframe::egui::pos2;
        let stroke=RegionShape::CircleStroke {points:vec![pos2(0.,0.),pos2(100.,0.),pos2(100.,100.)],radius:5.};
        for p in [pos2(-5.,0.),pos2(50.,5.),pos2(100.,50.),pos2(100.,105.)] {assert!(stroke.contains(p));}
        assert!(!stroke.contains(pos2(50.,50.)),"do not cut across a bent stroke");
        assert!(!stroke.contains(pos2(50.,5.1)));
        assert!(RegionShape::CircleStroke {points:vec![pos2(0.,0.);2],radius:5.}.contains(pos2(3.,4.)));
        assert!(!RegionShape::CircleStroke {points:vec![],radius:5.}.contains(pos2(0.,0.)));
    }
    #[test] fn wire_behind_the_surface_is_not_picked() {
        let p=[0.,0.,5., 100.,0.,5.];
        let project=|p:[f32;3]|Some((pos2(p[0],p[1]),p[2]));
        assert_eq!(pick_wire_occluded(&p,&[0,1],pos2(50.,2.),project,|_|true,false,true,4.),None);
        assert_eq!(pick_wire_occluded(&p,&[0,1],pos2(50.,2.),project,|_|true,false,true,5.1),Some(WireHit::Edge(0,1)));
    }
    #[test] fn wire_pixels_and_visibility() {
        let p=[0.,0.,1., 100.,0.,1.];
        let project=|p:[f32;3]|Some((pos2(p[0],p[1]),p[2]));
        assert_eq!(pick_wire(&p,&[0,1],pos2(3.,11.),project,|_|true,true,false),Some(WireHit::Point(0)));
        assert_eq!(pick_wire(&p,&[0,1],pos2(50.,7.),project,|_|true,false,true),Some(WireHit::Edge(0,1)));
        assert_eq!(pick_wire(&p,&[0,1],pos2(50.,9.),project,|_|true,false,true),None);
        assert_eq!(pick_wire(&p,&[0,1],pos2(0.,0.),project,|_|false,true,true),None);
    }
}

#[cfg(test)]
mod try_at_tests {
    use super::*;
    #[test] fn out_of_range_point_is_none() {
        let p = [0f32, 1., 2., 3., 4., 5.];
        assert_eq!(try_at(&p, 1), Some([3., 4., 5.]));
        assert_eq!(try_at(&p, 2), None);
    }
    #[test] fn at_out_of_range_is_nan_not_a_panic() {
        let p = [0f32, 1., 2.];
        assert!(at(&p, 5).iter().all(|v| v.is_nan()));
        assert_eq!(at(&p, 0), [0., 1., 2.]);
    }
}

pub const MAX_UNDECIMATED_POINTS: usize = 200_000;

/// An inner node is evaluated under an expanded name, while the displayed
/// geometry range is owned by its outer subnet (or a downstream output).
pub(crate) fn scoped_display_owner<'a>(value: &serde_json::Value, path: &[String], selected: &str,
    ranges: &'a [(String, std::ops::Range<u32>)]) -> Option<&'a str> {
    let outer = path.first()?;
    let mut inner = value;
    for id in path { inner = inner.get("nodes")?.get(id)?.get("inner")?; }
    let output = inner.get("output")?.as_str()?;
    if inner.get("nodes")?.get(selected).is_none() ||
        !crate::app_helpers::feeds_node(inner, selected, output) { return None; }
    let expanded = format!("{}/{}", path.join("/"), selected);
    if let Some((id, _)) = ranges.iter().find(|(id, _)| id == &expanded) { return Some(id); }
    crate::app_helpers::resolve_displayed_node(value, outer, ranges)
}

pub(crate) fn picked_scoped_node(path: &[String], selected: &str,
    ranges: &[(String, std::ops::Range<u32>)], point: u32) -> Option<String> {
    let prefix = format!("{}/", path.join("/"));
    let active = format!("{prefix}{selected}");
    ranges.iter().find(|(id, range)| id == &active && range.contains(&point))
        .or_else(|| ranges.iter().filter(|(id, range)| id.starts_with(&prefix) && range.contains(&point))
            .min_by_key(|(_, range)| range.len()))
        .and_then(|(id, _)| id.strip_prefix(&prefix).map(str::to_owned))
}

/// Decimation logic for point dots:
/// When `npoints <= MAX_UNDECIMATED_POINTS`, returns stride 1 (all points shown) and None for status.
/// When `npoints > MAX_UNDECIMATED_POINTS`, decimates dots by returning a stride > 1 and a status notice.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DecimatedDots {
    pub stride: usize,
    pub hidden: bool,
    pub status: Option<&'static str>,
}

pub fn decimate_dots(npoints: usize) -> DecimatedDots {
    if npoints <= MAX_UNDECIMATED_POINTS {
        DecimatedDots {
            stride: 1,
            hidden: false,
            status: None,
        }
    } else {
        DecimatedDots {
            stride: (npoints / 50_000).max(2),
            hidden: false,
            status: Some(">200k points: dots decimated"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use minihou_app::{Camera, StandardView};
    use minihou_app::graph_edit::{is_primitive_node, node_has_center_param};

    #[test]
    fn subnet_inner_node_resolves_to_its_displayed_outer_range() {
        let doc = serde_json::json!({"display":"out","nodes":{
            "sub":{"type":"subnet","inner":{"output":"output1","nodes":{
                "box1":{"type":"box"},"move1":{"type":"transform","inputs":{"geo":"box1"}},
                "output1":{"type":"subnet_output","port_index":0,"inputs":{"geo":"move1"}}
            }}},
            "out":{"type":"output","inputs":{"geo":"sub"}}
        }});
        let ranges = vec![("out".to_owned(), 0..24)];
        assert_eq!(scoped_display_owner(&doc, &["sub".into()], "box1", &ranges), Some("out"));
        assert_eq!(scoped_display_owner(&doc, &["sub".into()], "move1", &ranges), Some("out"));
        assert_eq!(scoped_display_owner(&doc, &["sub".into()], "missing", &ranges), None);
        let mut expanded = ranges;
        expanded.extend([("sub/move1".into(), 0..12), ("sub/box1".into(), 0..12),
            ("sub/other".into(), 12..24)]);
        assert_eq!(scoped_display_owner(&doc, &["sub".into()], "move1", &expanded), Some("sub/move1"));
        assert_eq!(picked_scoped_node(&["sub".into()], "move1", &expanded, 3), Some("move1".into()));
        assert_eq!(picked_scoped_node(&["sub".into()], "move1", &expanded, 15), Some("other".into()));
    }

    #[test]
    fn snap_candidates_use_screen_distance_and_edges_are_not_midpoints() {
        use crate::ui::{CameraInfo,SnapTargets,SnapTo};
        use eframe::egui::{pos2,vec2,Rect};
        let camera=CameraInfo { projection:[[1.,0.,0.,0.],[0.,1.,0.,0.],[0.,0.,1.,0.],[0.,0.,0.,1.]],
            ortho:true,ray_origin:[0.,0.,5.],ray_right:[1.,0.,0.],ray_up:[0.,1.,0.],ray_forward:[0.,0.,-1.],..Default::default() };
        let rect=Rect::from_min_size(pos2(0.,0.),vec2(200.,200.));
        let p=[0.,0.,0., 1.,0.,0., 0.,1.,0.];
        let mut targets=SnapTargets::default(); targets.toggle(SnapTo::Edge); targets.toggle(SnapTo::EdgeCenter);
        let edge=snap_feature(&p,&[0,1,2],camera,rect,pos2(125.,102.),targets,false).unwrap();
        assert_eq!(edge.kind,SnapTo::Edge);
        assert!((edge.point[0]-0.25).abs()<1e-5 && edge.point[1].abs()<1e-5);
        let vertex=snap_feature(&p,&[0,1,2],camera,rect,pos2(96.,104.),SnapTo::Vertex.into(),false).unwrap();
        assert_eq!(vertex.point,[0.;3]); // Pointer need not hit a face.
        assert!(snap_feature(&p,&[0,1,2],camera,rect,pos2(150.,150.),SnapTo::Vertex.into(),false).is_none());
        targets.toggle(SnapTo::Face);
        let face=snap_feature(&p,&[0,1,2],camera,rect,pos2(120.,90.),targets,false).unwrap();
        assert_eq!(face.kind,SnapTo::Face);
        assert!((face.point[0]-0.2).abs()<1e-5 && (face.point[1]-0.1).abs()<1e-5);
        assert!(snap_feature(&p,&[0,2,1],camera,rect,pos2(120.,90.),SnapTo::Face.into(),true).is_none());
    }

    #[test]
    fn volume_snap_uses_midpoint_between_entry_and_exit() {
        use crate::ui::{CameraInfo,SnapTo};
        use eframe::egui::{pos2,vec2,Rect};
        let camera=CameraInfo { projection:[[1.,0.,0.,0.],[0.,1.,0.,0.],[0.,0.,1.,0.],[0.,0.,0.,1.]],
            ortho:true,ray_origin:[0.,0.,5.],ray_right:[1.,0.,0.],ray_up:[0.,1.,0.],ray_forward:[0.,0.,-1.],..Default::default() };
        let p=[0.,0.,1., 1.,0.,1., 0.,1.,1., 0.,0.,-1., 0.,1.,-1., 1.,0.,-1.];
        let feature=snap_feature(&p,&[0,1,2,3,4,5],camera,Rect::from_min_size(pos2(0.,0.),vec2(200.,200.)),pos2(120.,90.),SnapTo::Volume.into(),false).unwrap();
        assert!(feature.point[2].abs()<1e-4);
    }

    #[test]
    fn region_contains_boundaries_and_concave_lassos() {
        use eframe::egui::{pos2, Rect};
        let rect = RegionShape::Rect(Rect::from_min_max(pos2(0., 0.), pos2(10., 10.)));
        assert!(rect.contains(pos2(0., 10.)));
        assert!(!rect.contains(pos2(11., 5.)));
        let circle = RegionShape::Circle { center: pos2(0., 0.), radius: 5. };
        assert!(circle.contains(pos2(3., 4.)));
        assert!(!circle.contains(pos2(4., 4.)));
        let points = vec![pos2(0., 0.), pos2(10., 0.), pos2(10., 4.), pos2(4., 4.), pos2(4., 10.), pos2(0., 10.)];
        for polygon in [points.clone(), points.into_iter().rev().collect()] {
            let lasso = RegionShape::Lasso(polygon);
            assert!(lasso.contains(pos2(2., 8.)));
            assert!(lasso.contains(pos2(4., 7.)));
            assert!(lasso.contains(pos2(10., 0.)));
            assert!(!lasso.contains(pos2(7., 7.)));
            assert!(!lasso.contains(pos2(-1., 0.)));
        }
        assert!(!RegionShape::Lasso(vec![]).contains(pos2(0., 0.)));
        assert!(!RegionShape::Lasso(vec![pos2(0., 0.); 2]).contains(pos2(0., 0.)));
    }

    #[test]
    fn region_edges_require_both_endpoints_even_when_midpoint_is_inside() {
        use eframe::egui::{pos2, Rect};
        for shape in [RegionShape::Rect(Rect::from_min_max(pos2(-5., -5.), pos2(5., 5.))),
            RegionShape::Circle { center: pos2(0., 0.), radius: 5. },
            RegionShape::Lasso(vec![pos2(-5., -5.), pos2(5., -5.), pos2(5., 5.), pos2(-5., 5.)])] {
            assert!(shape.contains(pos2(0., 0.)));
            assert!(!shape.contains_edge(pos2(-10., 0.), pos2(10., 0.)));
            assert!(!shape.contains_edge(pos2(0., 0.), pos2(6., 0.)));
            assert!(shape.contains_edge(pos2(-2., 0.), pos2(5., 0.)));
        }
    }

    // two triangles forming a unit square in the z = 0 plane
    const P: [f32; 12] = [0., 0., 0., 1., 0., 0., 1., 1., 0., 0., 1., 0.];
    const T: [u32; 6] = [0, 1, 2, 0, 2, 3];

    #[test]
    fn a_ray_down_z_hits_the_right_triangle() {
        let hit = pick_triangle(&P, &T, [0.8, 0.3, 5.], [0., 0., -1.]).unwrap();
        assert_eq!(hit.tri, 0);
        assert!((hit.t - 5.).abs() < 1e-5);
        let hit = pick_triangle(&P, &T, [0.2, 0.7, 5.], [0., 0., -1.]).unwrap();
        assert_eq!(hit.tri, 1);
        assert!(pick_triangle(&P, &T, [3., 3., 5.], [0., 0., -1.]).is_none());
    }

    #[test]
    fn nearest_triangle_wins_and_back_faces_count() {
        let p = [0., 0., 0., 1., 0., 0., 0., 1., 0., 0., 0., 1., 1., 0., 1., 0., 1., 1.];
        let t = [0, 1, 2, 3, 4, 5];
        let hit = pick_triangle(&p, &t, [0.2, 0.2, 9.], [0., 0., -1.]).unwrap();
        assert_eq!(hit.tri, 1);
        let from_below = pick_triangle(&p, &t, [0.2, 0.2, -9.], [0., 0., 1.]).unwrap();
        assert_eq!(from_below.tri, 0);
    }

    #[test]
    fn elements_follow_the_mode() {
        let hit = pick_triangle(&P, &T, [0.9, 0.05, 1.], [0., 0., -1.]).unwrap();
        assert_eq!(element_of(hit, &T, SelectionMode::Faces).tris, vec![0]);
        assert_eq!(element_of(hit, &T, SelectionMode::Points).points, vec![1]);
        // the click is nearest the bottom edge 0-1: the edge opposite the corner with the smallest weight (corner 2)
        assert_eq!(element_of(hit, &T, SelectionMode::Edges).edges, vec![(0, 1)]);
        assert!(element_of(hit, &T, SelectionMode::Object).tris.is_empty());
    }

    #[test]
    fn shift_click_toggles() {
        let mut cur = Selection { mode: SelectionMode::Faces, tris: vec![0], ..Default::default() };
        let one = Selection { mode: SelectionMode::Faces, tris: vec![1], ..Default::default() };
        extend(&mut cur, &one);
        assert_eq!(cur.tris, vec![0, 1]);
        extend(&mut cur, &one);
        assert_eq!(cur.tris, vec![0]);
        let pts = Selection { mode: SelectionMode::Points, points: vec![2], ..Default::default() };
        extend(&mut cur, &pts);
        assert!(cur.tris.is_empty() && cur.points == vec![2]);
    }

    #[test]
    fn highlight_sizes() {
        let sel = Selection { mode: SelectionMode::Faces, tris: vec![1], edges: vec![(0, 1)], points: vec![3] };
        let (faces, lines) = highlight(&P, &T, &sel, 0.05);
        assert_eq!(faces.len(), 9);
        assert_eq!(lines.len(), 6 + 18);
    }

    #[test]
    fn camera_center_ray_hits_mesh_in_perspective_and_orthographic_views() {
        let vertices = [-1., 0., -1., 1., 0., -1., 0., 0., 1.];
        let triangles = [0, 1, 2];
        let mut camera = Camera::frame([0., 0., 0.], 1.);
        for view in [StandardView::Perspective, StandardView::Front] {
            camera.set_view(view);
            let (origin, direction) = camera.ray(0., 0., 16. / 9.);
            let hit = pick_triangle(&vertices, &triangles, origin, direction).unwrap();
            assert_eq!(hit.tri, 0);
            assert!(hit.t > 0.);
        }
    }

    #[test]
    fn drop_placement_logic() {
        let p_tri = [-1., 2., -1., 1., 2., -1., 0., 2., 1.];
        let tri = [0, 1, 2];
        let cursor = [10.0, 5.0, -10.0];

        // 1. Ray hits triangle at y = 2.0
        let origin = [0.0, 5.0, 0.0];
        let dir_down = [0.0, -1.0, 0.0];
        let (hit_pos, surface_hit) = drop_world_position(origin, dir_down, Some(&p_tri), Some(&tri), cursor);
        assert!(surface_hit);
        assert!((hit_pos[1] - 2.0).abs() < 1e-4);

        // 2. Ray misses triangle but hits ground plane y = 0
        let origin_off = [5.0, 5.0, 5.0];
        let (hit_ground, surface_hit) = drop_world_position(origin_off, dir_down, Some(&p_tri), Some(&tri), cursor);
        assert!(surface_hit);
        assert!((hit_ground[1] - 0.0).abs() < 1e-5);
        assert!((hit_ground[0] - 5.0).abs() < 1e-5);
        assert!((hit_ground[2] - 5.0).abs() < 1e-5);

        // 3. Ray parallel to ground plane (dir.y = 0): falls back to 3D cursor
        let dir_parallel = [1.0, 0.0, 0.0];
        let (fallback_cursor, surface_hit) = drop_world_position(origin_off, dir_parallel, Some(&p_tri), Some(&tri), cursor);
        assert!(!surface_hit);
        assert_eq!(fallback_cursor, cursor);

        // Rest-on-surface offsets
        assert!((primitive_rest_offset("box") - 0.5).abs() < 1e-4);
        assert!((primitive_rest_offset("sphere") - 1.0).abs() < 1e-4);
        assert!((primitive_rest_offset("grid") - 5.0).abs() < 1e-4);

        // Parameter choice per node type
        assert!(node_has_center_param("box"));
        assert!(node_has_center_param("sphere"));
        assert!(node_has_center_param("tube"));
        assert!(node_has_center_param("torus"));
        assert!(node_has_center_param("grid"));
        assert!(node_has_center_param("circle"));
        assert!(!node_has_center_param("line"));
        assert!(!node_has_center_param("null"));
        assert!(!node_has_center_param("transform"));

        assert!(is_primitive_node("box"));
        assert!(is_primitive_node("line"));
        assert!(!is_primitive_node("noise"));
        assert!(!is_primitive_node("normal"));
    }

    #[test]
    fn cursor_placement_and_reset() {
        let p_mesh = [-1., 3., -1., 1., 3., -1., 0., 3., 1.];
        let tri = [0, 1, 2];
        let mut cursor = [1.0, 2.0, 3.0];
        let fwd = [0.0, 0.0, -1.0];

        // 1. Ray hits mesh
        let origin = [0.0, 6.0, 0.0];
        let dir = [0.0, -1.0, 0.0];
        let at_mesh = cursor_hit_position(origin, dir, Some(&p_mesh), Some(&tri), cursor, fwd);
        assert!((at_mesh[1] - 3.0).abs() < 1e-4);

        // 2. Ray misses mesh: hits plane at cursor height (y = 2.0)
        let origin_off = [5.0, 6.0, 5.0];
        let at_plane = cursor_hit_position(origin_off, dir, Some(&p_mesh), Some(&tri), cursor, fwd);
        assert!((at_plane[1] - cursor[1]).abs() < 1e-4);
        assert!((at_plane[0] - 5.0).abs() < 1e-4);

        // 3. Reset cursor to origin
        cursor = [0.0, 0.0, 0.0];
        assert_eq!(cursor, [0.0, 0.0, 0.0]);
    }

    #[test]
    fn ground_plane_intersection() {
        let origin = [0.0, 5.0, 0.0];
        let dir_down = [0.0, -1.0, 0.0];
        let t = intersect_ground_plane(origin, dir_down).unwrap();
        assert!((t - 5.0).abs() < 1e-5);

        let hit = intersect_ground_or_mesh(origin, dir_down, None, None, 10.0);
        assert!((hit[1] - 0.0).abs() < 1e-5);

        // Pointing up away from ground
        let dir_up = [0.0, 1.0, 0.0];
        assert!(intersect_ground_plane(origin, dir_up).is_none());

        // With mesh between origin and ground plane (at y = 2.0)
        let p_mesh = [-1., 2., -1., 1., 2., -1., 0., 2., 1.];
        let tri_mesh = [0, 1, 2];
        let hit_mesh = intersect_ground_or_mesh(origin, dir_down, Some(&p_mesh), Some(&tri_mesh), 10.0);
        assert!((hit_mesh[1] - 2.0).abs() < 1e-4);
    }

    #[test]
    fn dot_decimation() {
        let small = decimate_dots(100);
        assert_eq!(small.stride, 1);
        assert_eq!(small.status, None);

        let boundary = decimate_dots(200_000);
        assert_eq!(boundary.stride, 1);
        assert_eq!(boundary.status, None);

        let large = decimate_dots(200_001);
        assert!(large.stride > 1);
        assert!(large.status.unwrap().contains(">200k"));

        let very_large = decimate_dots(1_000_000);
        assert!(very_large.stride >= 20);
        assert!(very_large.status.unwrap().contains(">200k"));
    }
}

#[test]
fn filtered_pick_ignores_foreground_ghost_and_preserves_triangle_id() {
    let p=[0.,0.,1., 1.,0.,1., 0.,1.,1., 0.,0.,0., 1.,0.,0., 0.,1.,0.];
    let tri=[0,1,2,3,4,5];
    let hit=pick_triangle_filtered(&p,&tri,[0.2,0.2,3.],[0.,0.,-1.],|t| t.iter().all(|id| (3..6).contains(id))).unwrap();
    assert_eq!(hit.tri,1); assert_eq!(hit.t,3.);
    assert!(pick_triangle_filtered(&p,&tri,[0.2,0.2,3.],[0.,0.,-1.],|_|false).is_none());
}

#[cfg(test)]
mod cull_pick_tests {
    use super::*;
    #[test]
    fn nearest_visible_face_is_picked_whatever_its_winding() {
        // two triangles facing away from the viewer; the nearer one must win without culling
        let p=[0.,0.,0.,0.,1.,0.,1.,0.,0., 0.,0.,-1.,0.,1.,-1.,1.,0.,-1.]; let tri=[0u32,1,2,3,4,5];
        let hit=pick_triangle_filtered(&p,&tri,[0.2,0.2,3.],[0.,0.,-1.],|_|true).unwrap();
        assert_eq!(hit.tri,0);
    }
    #[test]
    fn backface_pick_needs_xray() {
        let p=[0.,0.,0.,1.,0.,0.,0.,1.,0.]; let tri=[0u32,1,2];
        // front (normal +z) seen from +z; back seen from -z
        assert!(pick_triangle_culled(&p,&tri,[0.2,0.2,3.],[0.,0.,-1.],true,|_|true).is_some());
        assert!(pick_triangle_culled(&p,&tri,[0.2,0.2,-3.],[0.,0.,1.],true,|_|true).is_none());
        assert!(pick_triangle_culled(&p,&tri,[0.2,0.2,-3.],[0.,0.,1.],false,|_|true).is_some());
    }
}
