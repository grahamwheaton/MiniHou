//! Faithful polygon operations from minihou/nodes/model.py. Shared builders follow _build.py.
use super::{GEO, Inputs, NodeSpec, groups};
use crate::{
    error::{Error, Result},
    geometry::{Attr, AttrData, AttrTable, Geometry, Topology},
    normals,
    params::Params,
    topo,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

pub(super) fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    (0..3).map(|k| a[k] * b[k]).sum()
}
pub(super) fn add(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    std::array::from_fn(|k| a[k] + b[k])
}
pub(super) fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    std::array::from_fn(|k| a[k] - b[k])
}
pub(super) fn mul(a: [f64; 3], s: f64) -> [f64; 3] {
    a.map(|v| v * s)
}
pub(super) fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
pub(super) fn len(a: [f64; 3]) -> f64 {
    dot(a, a).sqrt()
}
pub(super) fn unit(a: [f64; 3]) -> [f64; 3] {
    let n = len(a);
    mul(a, 1.0 / if n < 1e-20 { 1.0 } else { n })
}
// Python normalize_rows returns float32 even when its input is float64.
pub(super) fn unit32(a: [f64; 3]) -> [f64; 3] {
    unit(a).map(|v| v as f32 as f64)
}
mod build;
pub(super) use build::{positions, range, gather, promote_corner_uv_sets, concat, lerp, mean, make, rebuild, prim_mask, group_name, write_group, outgroup};
pub(crate) use build::{corners, polygons, compact};

pub const DIVIDE: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None, cook_sidecar: None,
    name: "divide",
    inputs: GEO,
    params: crate::params!("group":""),
    cook: divide,
};
pub const POLYFILL: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None, cook_sidecar: None,
    name: "polyfill",
    inputs: GEO,
    params: super::polyfill::PARAMS,
    cook: super::polyfill::polyfill,
};
pub const MIRROR: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None, cook_sidecar: None,
    name: "mirror",
    inputs: GEO,
    params: crate::params!("origin":[0.,0.,0.];"normal":[1.,0.,0.];"keep_original":true;"consolidate_seam":true;"distance":0.001;"outgrp_mirrored":false;"outgrp_mirrored_name":"mirrored";"outgrp_mirroredpoints":false;"outgrp_mirroredpoints_name":"mirroredpoints";"outgrp_seam":false;"outgrp_seam_name":"seampoints";"group":"";"operation":"mirror_all";"offset":0.0;"reverse":false;"cut":false;"unshared_only":false),
    cook: super::mirror::mirror,
};
pub const CLIP: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None, cook_sidecar: None,
    name: "clip",
    inputs: GEO,
    params: crate::params!("origin":[0.,0.,0.];"normal":[0.,0.,1.];"keep":"above";"outgrp_above":true;"outgrp_above_name":"above";"outgrp_below":false;"outgrp_below_name":"below";"outgrp_cutpolys":false;"outgrp_cutpolys_name":"cutpolys";"outgrp_cutpoints":false;"outgrp_cutpoints_name":"cutpoints";"outgrp_cutedges":false;"outgrp_cutedges_name":"cutedges"),
    cook: clip,
};
pub const POLYEXTRUDE: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None, cook_sidecar: None,
    name: "polyextrude",
    inputs: &[super::InputSpec{name:"geo",multi:false,optional:false},super::InputSpec{name:"spine",multi:false,optional:true}],
    params: crate::params!("group":"";"edges":"";"points":"";"distance":0.5;"direction":[0.,0.,0.];"inset":0.;"mode":"connected";"output_back":false;"outgrp_front":true;"outgrp_front_name":"extrudeFront";"outgrp_back":false;"outgrp_back_name":"extrudeBack";"outgrp_side":true;"outgrp_side_name":"extrudeSide";"divide_into":"connected";"split_group":"";"extrusion_mode":"primitive_normal";"twist":0.;"divisions":1;"spine_shape":"straight";"transform_front":true;"output_front":true;"output_side":true;"outgrp_front_boundary":false;"outgrp_front_boundary_name":"extrudeFrontBoundary";"outgrp_back_boundary":false;"outgrp_back_boundary_name":"extrudeBackBoundary";"preserve_groups":true;"copy_edge_groups":true;"copy_point_groups":true;"copy_primitive_groups":true;"limit_insetting":false;"use_common_limit":false;"offset_even":true;"add_vertex_normals":false;"cusp_angle":60.;"generate_sides_uvs":false;"uv_style":"proportional";"uv_scaling":"match_incident";
        "front_magnitude":1.;"back_magnitude":1.;"front_stiffness":1.;"back_stiffness":1.;"interpolation":"rotating_frame";"division_spacing":"uniform";
        "reverse_spine_direction":false;"axial_rotation":0.;"front_blend":0.;"back_blend":0.;
        "thickness_scale":1.;"thickness_ramp":[[0.,0.5],[1.,0.5]];"thickness_attribute":"";
        "twist_range_half_width":1.;"local_twist_ramp":[[0.,0.5],[1.,0.5]];"twist_attribute":"";
        "distance_scale_attribute":"zscale";"inset_scale_attribute":"insetscale";"twist_scale_attribute":"twistscale";"divisions_scale_attribute":"divsscale";
        "local_x_attribute":"localx";"local_z_attribute":"localz";"local_center_attribute":"localctr";"front_translate":[0.,0.,0.]),
    cook: polyextrude,
};
mod cuts;
pub(super) use cuts::ear_clip;
use cuts::{divide, clip};

mod extrude;
pub(super) use extrude::extrude;
use extrude::polyextrude;

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    pub(super) fn quad() -> Geometry {
        Geometry::from_arrays(
            vec![0., 0., 0., 2., 0., 0., 2., 2., 0., 0., 2., 0.],
            vec![0, 1, 2, 3],
            vec![0, 4],
            None,
        )
        .unwrap()
    }
    fn cook(s: &NodeSpec, g: Geometry, v: serde_json::Value) -> Geometry {
        let p = Params::resolve(s.params, v.as_object()).unwrap();
        (s.cook)(&[vec![Arc::new(g)]], &p).unwrap()
    }
    #[test]
    fn reverse_and_divide_keep_corner_rows() {
        let mut g = quad();
        g.vertex
            .insert("id".into(), Arc::new(Attr::i32(1, vec![10, 11, 12, 13])));
        let r = cook(crate::nodes::registry().iter().find(|spec| spec.name == "reverse").unwrap(), g.clone(), json!({}));
        assert_eq!(r.topology.vertex_point, vec![3, 2, 1, 0]);
        assert_eq!(r.vertex["id"].data, AttrData::I32(vec![13, 12, 11, 10]));
        let d = cook(&DIVIDE, g, json!({}));
        assert_eq!(d.topology.vertex_point, vec![0, 1, 2, 0, 2, 3]);
        d.validate().unwrap();
    }
    #[test]
    fn extrusion_closed_shell_and_zero_noop() {
        let g = quad();
        let out = cook(
            &POLYEXTRUDE,
            g.clone(),
            json!({"distance":2.,"output_back":true}),
        );
        assert_eq!((out.npoints, out.nprims()), (8, 6));
        assert!(
            topo::edges(&out.topology, out.npoints)
                .count
                .iter()
                .all(|&c| c == 2)
        );
        assert_eq!(
            out.prim["group_extrudeSide"].data,
            AttrData::I32(vec![0, 1, 1, 1, 1, 0])
        );
        let z = cook(&POLYEXTRUDE, g.clone(), json!({"distance":0.}));
        assert_eq!(z.p(), g.p());
        assert_eq!(z.topology, g.topology);
    }
    #[test]
    fn polyextrude_axis_direction_changes_preview_geometry() {
        let g=quad();
        let normal=cook(&POLYEXTRUDE,g.clone(),json!({"distance":1.}));
        let axis=cook(&POLYEXTRUDE,g,json!({"distance":1.,"direction":[1.,0.,0.]}));
        assert_eq!(normal.nprims(),axis.nprims());
        assert!(axis.p().chunks_exact(3).any(|p|p[0]>2.9));
        assert_ne!(normal.p(),axis.p());
    }
    #[test]
    fn polyextrude_point_can_continue_along_its_new_open_segment() {
        let first=cook(&POLYEXTRUDE,quad(),json!({"points":"0","distance":0.05}));
        let second=cook(&POLYEXTRUDE,first.clone(),json!({"points":"4","distance":0.05}));
        assert_eq!(second.npoints,6);assert_eq!(second.nprims(),3);
        let a=&first.p()[12..15];let b=&second.p()[15..18];
        assert!((b[2]-a[2]-0.05).abs()<1e-6);
        assert_eq!(&second.topology.closed.as_ref().unwrap()[..],&[true,false,false]);
        second.validate().unwrap();
    }
    #[test]
    fn polyextrude_edge_and_point_previews_keep_original_geometry() {
        let g=quad();
        let edge=cook(&POLYEXTRUDE,g.clone(),json!({"edges":"0-1","distance":1.}));
        assert_eq!(edge.nprims(),g.nprims()+1);
        assert_eq!(edge.npoints,g.npoints+2);
        assert_eq!(edge.topology.vertex_point[0..4],g.topology.vertex_point[0..4]);
        assert!(edge.p().chunks_exact(3).any(|p|p[2]>0.9));
        let point=cook(&POLYEXTRUDE,g.clone(),json!({"points":"0","distance":1.}));
        assert_eq!(point.nprims(),g.nprims()+1);
        assert_eq!(point.npoints,g.npoints+1);
        assert_eq!(point.topology.closed.as_ref().unwrap(),&vec![true,false]);
        point.validate().unwrap();
    }
    #[test]
    fn polyextrude_houdini_parity_extrusion_options() {
        let g = quad();
        // 1. Divisions
        let div = cook(&POLYEXTRUDE, g.clone(), json!({"distance": 1.0, "divisions": 3}));
        // Quad has 4 boundary edges * 3 divisions = 12 side quads + 1 front = 13 prims
        assert_eq!(div.nprims(), 13);
        div.validate().unwrap();

        // 2. Output toggles
        let no_front = cook(&POLYEXTRUDE, g.clone(), json!({"distance": 1.0, "output_front": false}));
        assert_eq!(no_front.nprims(), 4);
        assert!(!no_front.prim.contains_key("group_extrudeFront") || no_front.prim["group_extrudeFront"].data == AttrData::I32(vec![0, 0, 0, 0]));

        let no_side = cook(&POLYEXTRUDE, g.clone(), json!({"distance": 1.0, "output_side": false}));
        assert_eq!(no_side.nprims(), 1);

        // 3. Twist
        let straight = cook(&POLYEXTRUDE, g.clone(), json!({"distance": 1.0}));
        let twisted = cook(&POLYEXTRUDE, g.clone(), json!({"distance": 1.0, "twist": 45.0}));
        assert_ne!(straight.p(), twisted.p());

        // 4. Vertex normals
        let with_n = cook(&POLYEXTRUDE, g.clone(), json!({"distance": 1.0, "add_vertex_normals": true, "cusp_angle": 60.0}));
        assert!(with_n.vertex.contains_key("N"));
        assert_eq!(with_n.vertex["N"].width, 3);

        // 5. Generate sides UVs
        let with_uv = cook(&POLYEXTRUDE, g.clone(), json!({"distance": 1.0, "generate_sides_uvs": true}));
        assert!(with_uv.vertex.contains_key("uv"));
        assert_eq!(with_uv.vertex["uv"].width, 2);

        // 6. Boundary edge groups
        let with_bounds = cook(&POLYEXTRUDE, g.clone(), json!({
            "distance": 1.0,
            "outgrp_front_boundary": true,
            "outgrp_front_boundary_name": "front_edges"
        }));
        assert!(with_bounds.vertex.contains_key("edge_group_front_edges") || with_bounds.edge.contains_key("group_front_edges"));
    }
    #[test]
    fn polyextrude_spine_controls_change_bridge_and_follow_second_input() {
        let g=quad();
        let baseline=cook(&POLYEXTRUDE,g.clone(),json!({"distance":1.0,"divisions":4}));
        let curved=cook(&POLYEXTRUDE,g.clone(),json!({"distance":1.0,"divisions":4,"spine_shape":"curved","front_magnitude":2.0,"back_stiffness":0.5}));
        assert_ne!(curved.p(),baseline.p());
        let curved_translated=json!({"distance":1.0,"divisions":4,"spine_shape":"curved","front_magnitude":2.0,"front_translate":[0.3,0.0,0.0]});
        let uniform=cook(&POLYEXTRUDE,g.clone(),curved_translated.clone());
        let mut spaced_params=curved_translated;spaced_params["division_spacing"]=json!("curvature_sensitive");
        let spaced=cook(&POLYEXTRUDE,g.clone(),spaced_params);
        assert_ne!(spaced.p(),uniform.p());
        let mut spine=Geometry::from_arrays(vec![0.,0.,0.,0.5,0.,0.5,0.,0.,1.],vec![0,1,2],vec![0,3],Some(vec![false])).unwrap();
        spine.point.insert("thick".into(),Arc::new(Attr::f32(1,vec![0.,0.1,0.])));
        spine.point.insert("bend".into(),Arc::new(Attr::f32(1,vec![0.,0.2,0.])));
        let p=Params::resolve(POLYEXTRUDE.params,json!({"distance":1.0,"divisions":4,"spine_shape":"curve_from_second_input"}).as_object()).unwrap();
        let along=(POLYEXTRUDE.cook)(&[vec![Arc::new(g.clone())],vec![Arc::new(spine.clone())]],&p).unwrap();
        assert_ne!(along.p(),baseline.p());
        let attributes=Params::resolve(POLYEXTRUDE.params,json!({"distance":1.0,"divisions":4,"spine_shape":"curve_from_second_input","thickness_attribute":"thick","twist_attribute":"bend","twist_range_half_width":90.0}).as_object()).unwrap();
        let attributed=(POLYEXTRUDE.cook)(&[vec![Arc::new(g.clone())],vec![Arc::new(spine.clone())]],&attributes).unwrap();
        assert_ne!(attributed.p(),along.p());
        let reversed=Params::resolve(POLYEXTRUDE.params,json!({"distance":1.0,"divisions":4,"spine_shape":"curve_from_second_input","reverse_spine_direction":true,"axial_rotation":30.0,"front_blend":0.5,"back_blend":0.5}).as_object()).unwrap();
        let other=(POLYEXTRUDE.cook)(&[vec![Arc::new(g)],vec![Arc::new(spine)]],&reversed).unwrap();
        assert_ne!(along.p(),other.p());
    }
    #[test]
    fn polyextrude_thickness_and_twist_ramps_affect_bridge() {
        let g=quad();
        let base=cook(&POLYEXTRUDE,g.clone(),json!({"distance":1.0,"divisions":3}));
        let thick=cook(&POLYEXTRUDE,g.clone(),json!({"distance":1.0,"divisions":3,"thickness_scale":2.0,"thickness_ramp":[[0.0,0.5],[1.0,0.75]]}));
        assert_ne!(base.p(),thick.p());
        let twisted=cook(&POLYEXTRUDE,g,json!({"distance":1.0,"divisions":3,"twist_range_half_width":90.0,"local_twist_ramp":[[0.0,0.5],[1.0,1.0]]}));
        assert_ne!(base.p(),twisted.p());
    }
    #[test]
    fn polyextrude_local_control_attributes_and_frames() {
        let mut g=quad();
        g.prim.insert("zscale".into(),Arc::new(Attr::f32(1,vec![2.])));
        g.prim.insert("insetscale".into(),Arc::new(Attr::f32(1,vec![0.5])));
        g.prim.insert("twistscale".into(),Arc::new(Attr::f32(1,vec![0.])));
        g.prim.insert("divsscale".into(),Arc::new(Attr::f32(1,vec![2.])));
        g.prim.insert("localz".into(),Arc::new(Attr::f32(3,vec![1.,0.,0.])));
        g.prim.insert("localx".into(),Arc::new(Attr::f32(3,vec![0.,1.,0.])));
        g.prim.insert("localctr".into(),Arc::new(Attr::f32(3,vec![5.,0.,0.])));
        let out=cook(&POLYEXTRUDE,g.clone(),json!({"distance":1.0,"inset":0.1,"twist":45.0,"divisions":2,"front_translate":[1.,0.,0.]}));
        assert_eq!(out.nprims(),17); // 4 edges * (2 divisions * divsscale 2) + front
        let unscaled=cook(&POLYEXTRUDE,g.clone(),json!({"distance":1.0,"inset":0.1,"twist":45.0,"divisions":2,"front_translate":[1.,0.,0.],"distance_scale_attribute":"missing","divisions_scale_attribute":"missing"}));
        assert_ne!(out.p(),unscaled.p());
        let no_frame=cook(&POLYEXTRUDE,g,json!({"distance":1.0,"inset":0.1,"twist":45.0,"divisions":2,"front_translate":[1.,0.,0.],"local_x_attribute":"missing","local_z_attribute":"missing","local_center_attribute":"missing"}));
        assert_ne!(out.p(),no_frame.p());
    }
    #[test]
    fn polyextrude_offset_even_and_houdini_output_group_options() {
        let mut g=quad();
        g.point.insert("group_seed".into(),Arc::new(Attr::i32(1,vec![1,0,0,0])));
        g.prim.insert("group_source".into(),Arc::new(Attr::i32(1,vec![1])));
        g.vertex.insert("edge_group_source".into(),Arc::new(Attr::i32(1,vec![1,0,0,0])));
        let even=cook(&POLYEXTRUDE,g.clone(),json!({"distance":1.0,"inset":0.15,"offset_even":true}));
        let uneven=cook(&POLYEXTRUDE,g.clone(),json!({"distance":1.0,"inset":0.15,"offset_even":false}));
        assert_ne!(even.p(),uneven.p());
        let out=cook(&POLYEXTRUDE,g,json!({"distance":1.0,"copy_point_groups":false,"copy_primitive_groups":false,"copy_edge_groups":false,
            "outgrp_front_name":"HoudiniFront","outgrp_side_name":"HoudiniSide","outgrp_back":true,"output_back":true}));
        assert!(out.prim.contains_key("group_HoudiniFront") && out.prim.contains_key("group_HoudiniSide") && out.prim.contains_key("group_extrudeBack"));
        assert!(out.point["group_seed"].as_i32().unwrap().iter().skip(4).all(|v|*v==0));
        assert!(out.prim["group_source"].as_i32().unwrap().iter().take(5).all(|v|*v==0));
        assert!(out.vertex["edge_group_source"].as_i32().unwrap().iter().take(20).all(|v|*v==0));
    }
    #[test]
    fn polyextrude_offset_even_compensates_axis_extrusion_on_tilted_nonuniform_face() {
        let g = Geometry::from_arrays(
            vec![0., 0., 0., 3., 0., 3., 2., 2., 2., 0., 1., 0.],
            vec![0, 1, 2, 3], vec![0, 4], None,
        ).unwrap();
        let even = cook(&POLYEXTRUDE, g.clone(), json!({"distance":1.,"direction":[0.,0.,1.],"offset_even":true}));
        let uneven = cook(&POLYEXTRUDE, g.clone(), json!({"distance":1.,"direction":[0.,0.,1.],"offset_even":false}));
        let normal = unit(normals::prim_normals(g.p(), &g.topology)[0]);
        for i in 0..4 {
            let height = |out: &Geometry| (0..3).map(|k| (out.p()[3*(i+4)+k]-g.p()[3*i+k]) as f64 * normal[k]).sum::<f64>();
            assert!((height(&even)-1.).abs()<1e-5);
            assert!((height(&uneven)-1.).abs()>0.1);
        }
        assert_ne!(even.p(), uneven.p());
    }
    #[test]
    fn clip_shared_intersections_and_int_sources() {
        let mut g = quad();
        g.point
            .insert("id".into(), Arc::new(Attr::i32(1, vec![7, 8, 9, 10])));
        let out = cook(
            &CLIP,
            g,
            json!({"origin":[1.,0.,0.],"normal":[1.,0.,0.],"keep":"both"}),
        );
        assert_eq!((out.npoints, out.nprims()), (6, 2));
        assert_eq!(out.point["id"].data, AttrData::I32(vec![7, 8, 9, 10, 7, 9]));
        out.validate().unwrap();
    }
    #[test]
    fn concave_divide_area() {
        let g = Geometry::from_arrays(
            vec![
                0., 0., 0., 2., 0., 0., 2., 1., 0., 1., 1., 0., 1., 2., 0., 0., 2., 0.,
            ],
            vec![0, 1, 2, 3, 4, 5],
            vec![0, 6],
            None,
        )
        .unwrap();
        let out = cook(&DIVIDE, g, json!({}));
        assert_eq!(out.nprims(), 4);
        let n = normals::prim_normals(out.p(), &out.topology);
        assert!((n.iter().map(|v| len(*v)).sum::<f64>() / 2. - 3.).abs() < 1e-6);
        assert!(n.iter().all(|v| v[2] > 0.));
    }
}
/// Eigenvectors (columns) of a symmetric 3x3 matrix. Used for SVD-equivalent least squares on local Jacobians.
pub(super) fn eigen(mut a: [[f64; 3]; 3]) -> ([f64; 3], [[f64; 3]; 3]) {
    let mut v = [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]];
    for _ in 0..64 {
        let mut pair = (0, 1);
        for (i, j) in [(0, 2), (1, 2)] {
            if a[i][j].abs() > a[pair.0][pair.1].abs() {
                pair = (i, j);
            }
        }
        let (p, q) = pair;
        if a[p][q].abs() <= 1e-16 * (a[0][0].abs() + a[1][1].abs() + a[2][2].abs()).max(1e-300) {
            break;
        }
        let tau = (a[q][q] - a[p][p]) / (2. * a[p][q]);
        let t = if tau >= 0. {
            1. / (tau + (1. + tau * tau).sqrt())
        } else {
            -1. / (-tau + (1. + tau * tau).sqrt())
        };
        let c = 1. / (1. + t * t).sqrt();
        let s = t * c;
        let ap = a[p][p];
        let aq = a[q][q];
        let off = a[p][q];
        a[p][p] = ap - t * off;
        a[q][q] = aq + t * off;
        a[p][q] = 0.;
        a[q][p] = 0.;
        for k in 0..3 {
            if k != p && k != q {
                let x = a[k][p];
                let y = a[k][q];
                a[k][p] = c * x - s * y;
                a[p][k] = a[k][p];
                a[k][q] = s * x + c * y;
                a[q][k] = a[k][q];
            }
            let x = v[k][p];
            let y = v[k][q];
            v[k][p] = c * x - s * y;
            v[k][q] = s * x + c * y;
        }
    }
    ([a[0][0], a[1][1], a[2][2]], v)
}
#[cfg(test)]
mod python_oracles {
    use super::*;
    use serde_json::{Map, Value};

    fn table(v: &Value) -> AttrTable {
        v.as_object()
            .unwrap()
            .iter()
            .map(|(name, a)| {
                let width = a["width"].as_u64().unwrap() as usize;
                let rows = a["data"].as_array().unwrap();
                let attr = if a["dtype"] == "float32" {
                    Attr::f32(
                        width,
                        rows.iter().map(|x| x.as_f64().unwrap() as f32).collect(),
                    )
                } else {
                    Attr::i32(
                        width,
                        rows.iter().map(|x| x.as_i64().unwrap() as i32).collect(),
                    )
                };
                (name.clone(), Arc::new(attr))
            })
            .collect()
    }
    fn geometry(v: &Value) -> Geometry {
        let point = table(&v["point"]);
        let closed = v["closed"]
            .as_array()
            .unwrap()
            .iter()
            .map(|x| x.as_bool().unwrap())
            .collect();
        let topo = Topology {
            vertex_point: v["vp"]
                .as_array()
                .unwrap()
                .iter()
                .map(|x| x.as_i64().unwrap() as i32)
                .collect(),
            prim_offsets: v["offsets"]
                .as_array()
                .unwrap()
                .iter()
                .map(|x| x.as_i64().unwrap() as i32)
                .collect(),
            closed: Some(closed),
        ..Default::default()};
        let g = Geometry { packed: Default::default(), edge: Default::default(),
            npoints: point["P"].rows(),
            topology: Arc::new(topo),
            point,
            vertex: table(&v["vertex"]),
            prim: table(&v["prim"]),
            detail: v["detail"].as_object().unwrap().clone(),
        };
        g.validate().unwrap();
        g
    }
    fn compare(name: &str, a: &Geometry, b: &Geometry) {
        assert_eq!(a.npoints, b.npoints, "{name}: point count");
        assert_eq!(
            a.topology.vertex_point, b.topology.vertex_point,
            "{name}: corner points"
        );
        assert_eq!(
            a.topology.prim_offsets, b.topology.prim_offsets,
            "{name}: offsets"
        );
        assert_eq!(
            a.topology.closed_mask(),
            b.topology.closed_mask(),
            "{name}: closed flags"
        );
        // Python lacks the new neutral slot and binding metadata; its detail keys still match exactly.
        super::super::oracle_material_detail::compare(name, a, b);
        let (original_a, original_b) = (a, b);
        for domain in ["point", "vertex", "prim"] {
            let (a, b) = (a.table(domain), b.table(domain));
            let neutral_index = domain == "prim" && super::super::oracle_material_detail::neutral_index_added(original_a, original_b);
            // `point_group_knife` is a Rust-only group written by the knife node; Python has no such attribute.
            let rust_only = |key: &str| key == "point_group_knife";
            assert_eq!(
                a.keys().filter(|key| (!neutral_index || key.as_str() != "material_index") && !rust_only(key)).collect::<Vec<_>>(),
                b.keys().collect::<Vec<_>>(),
                "{name}: {domain} attribute names"
            );
            for (key, a) in a {
                if neutral_index && key == "material_index" { continue; }
                if rust_only(key) { continue; }
                let b = &b[key];
                let invalid_uv = if domain == "vertex" {
                    super::super::oracle_material_detail::invalid_uv_rows(
                        original_a, original_b, key, &(0..a.rows()).collect::<Vec<_>>())
                } else { vec![false; a.rows()] };
                assert_eq!(a.width, b.width, "{name}: {domain} {key} width");
                match (&a.data, &b.data) {
                    (AttrData::F32(a), AttrData::F32(b)) => {
                        assert_eq!(a.len(), b.len(), "{name}: {domain} {key} length");
                        for (i, (&a, &b)) in a.iter().zip(b).enumerate() {
                            if invalid_uv[i / original_a.table(domain)[key].width] && a.is_nan() && b == 0. { continue; }
                            assert!(
                                (a - b).abs() <= 1e-5 + 1e-5 * b.abs(),
                                "{name}: {domain} {key}[{i}]: Rust {a}, Python {b}"
                            );
                        }
                    }
                    (AttrData::I32(a), AttrData::I32(b)) => {
                        assert_eq!(a, b, "{name}: {domain} {key}")
                    }
                    _ => panic!("{name}: {domain} {key} dtype"),
                }
            }
        }
    }
    #[test]
    fn modeling_matches_python_topology_attributes_and_defaults() {
        // Regenerate only with tools/dump_rust_modeling_fixtures.py; Python is the oracle.
        // New source-less faces may add a neutral slot and NaN UV where Python stored zero.
        let cases: Value =
            serde_json::from_str(include_str!("test_data/modeling_parity.json")).unwrap();
        for c in cases.as_array().unwrap() {
            let name = c["name"].as_str().unwrap();
            let spec = super::super::get(c["type"].as_str().unwrap()).unwrap();
            let defaults: Map<String, Value> = spec
                .params
                .iter()
                .map(|p| (p.name.into(), (p.default)()))
                .collect();
            let expected_defaults = c["defaults"].as_object().unwrap();
            // Rust may add Rust-only params; every Python param must exist with the same default.
            for (key, expected) in expected_defaults {
                assert!(
                    defaults
                        .get(key)
                        .is_some_and(|value| crate::params::same_value(value, expected)),
                    "{name}: parameter {key} default"
                );
            }
            let p = Params::resolve(spec.params, c["params"].as_object()).unwrap();
            let mut inputs: Vec<Vec<Arc<Geometry>>> = c["inputs"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| vec![Arc::new(geometry(v))])
                .collect();
            inputs.resize_with(spec.inputs.len(), Vec::new);
            let result = (spec.cook)(&inputs, &p);
            if c.get("error").is_some() {
                assert!(result.is_err(), "{name}: expected rejection");
            } else {
                let actual = result.unwrap_or_else(|e| panic!("{name}: {e}"));
                actual.validate().unwrap();
                let mut expected = geometry(&c["expected"]);
                if name == "61_knife_quad" {
                    // The fixture reference documents the intended fix; Knife preserves the input detail.
                    expected.detail.insert("reference".into(), c["inputs"][0]["detail"]["reference"].clone());
                }
                compare(name, &actual, &expected);
            }
        }
    }
    #[test]
    fn loop_cut_is_polysplit() {
        let alias = super::super::get("loop_cut").unwrap();
        let canonical = super::super::get("polysplit").unwrap();
        assert_eq!(alias.params.len(), canonical.params.len());
        let input = super::tests::quad();
        let given = serde_json::json!({"split_edges":"p0-1"});
        let p = Params::resolve(alias.params, given.as_object()).unwrap();
        let inputs = [vec![Arc::new(input)]];
        compare(
            "loop_cut alias",
            &(alias.cook)(&inputs, &p).unwrap(),
            &(canonical.cook)(&inputs, &p).unwrap(),
        );
    }
}
/// Small least-squares solve with a one-sided Jacobi SVD. Avoid forming A^T A:
/// squaring the condition number loses the null-space behavior of numpy.pinv/lstsq.
pub(super) fn least_squares<const N: usize>(
    rows: &[[f64; N]],
    rhs: &[f64],
    rcond: f64,
) -> [f64; N] {
    let mut a = rows.to_vec();
    let mut v = [[0.; N]; N];
    for i in 0..N {
        v[i][i] = 1.;
    }
    for _ in 0..64 {
        let mut changed = false;
        for p in 0..N {
            for q in p + 1..N {
                let aa = a.iter().map(|r| r[p] * r[p]).sum::<f64>();
                let bb = a.iter().map(|r| r[q] * r[q]).sum::<f64>();
                let ab = a.iter().map(|r| r[p] * r[q]).sum::<f64>();
                if ab.abs() <= f64::EPSILON * (aa * bb).sqrt() || ab == 0. {
                    continue;
                }
                let tau = (bb - aa) / (2. * ab);
                let t = if tau >= 0. {
                    1. / (tau + tau.hypot(1.))
                } else {
                    -1. / (-tau + tau.hypot(1.))
                };
                let c = 1. / (1. + t * t).sqrt();
                let s = t * c;
                for r in &mut a {
                    let x = r[p];
                    let y = r[q];
                    r[p] = c * x - s * y;
                    r[q] = s * x + c * y;
                }
                for r in &mut v {
                    let x = r[p];
                    let y = r[q];
                    r[p] = c * x - s * y;
                    r[q] = s * x + c * y;
                }
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    let norm: [f64; N] = std::array::from_fn(|k| a.iter().map(|r| r[k] * r[k]).sum::<f64>().sqrt());
    let limit = norm.iter().copied().fold(0., f64::max) * rcond;
    let mut result = [0.; N];
    for k in 0..N {
        if norm[k] > limit && norm[k] > 0. {
            let coeff =
                a.iter().zip(rhs).map(|(r, &b)| r[k] * b).sum::<f64>() / (norm[k] * norm[k]);
            for i in 0..N {
                result[i] += v[i][k] * coeff;
            }
        }
    }
    result
}
#[cfg(test)]
mod least_squares_tests {
    use super::*;
    #[test]
    fn singular_solve_returns_minimum_norm_solution() {
        let rows = [[1., 2., 0.], [2., 4., 0.], [0., 0., 0.]];
        let result = least_squares(&rows, &[3., 6., 1.], 1e-15);
        assert!(len(sub(result, [0.6, 1.2, 0.])) < 1e-12);
    }
    #[test]
    fn full_rank_solve_recovers_an_affine_field() {
        let rows = [
            [1., 0., 0., 1.],
            [0., 1., 0., 1.],
            [0., 0., 1., 1.],
            [1., 1., 1., 1.],
            [2., -1., 3., 1.],
        ];
        let exact = [2., -3., 4., 0.5];
        let rhs: Vec<_> = rows
            .iter()
            .map(|r| (0..4).map(|k| r[k] * exact[k]).sum())
            .collect();
        let result = least_squares(&rows, &rhs, 5. * f64::EPSILON);
        for i in 0..4 {
            assert!((result[i] - exact[i]).abs() < 1e-11);
        }
    }
}
