//! Modeling tools, including quad-strip loop cuts, from modeling_tools.py.
use super::{GEO, Inputs, NodeSpec, edge_tools as edges, groups, model::*};
use crate::{
    error::{Error, Result},
    geometry::{Attr, AttrData, Geometry},
    normals,
    params::Params,
    topo,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};
mod mesh;
mod deform;
mod loops;
mod polysplit;

use mesh::{soft_peak, group_range, edge_collapse, edge_flip, edge_cusp};
use deform::{twist, taper, squash};
use loops::edge_slide;
use polysplit::polysplit;
pub use loops::{LOOP_CUT_MAX, loop_cut_fractions, loop_slide_fractions, LoopTopology, LoopRing, edge_slide_preview, loop_cut_preview};
#[cfg(test)]
use loops::key;

pub const SOFT_PEAK: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None, cook_sidecar: None,
    name: "soft_peak",
    inputs: GEO,
    params: crate::params!("distance":0.3;"radius":1.;"falloff":"smooth";"group":""),
    cook: soft_peak,
};
pub const GROUP_RANGE: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None, cook_sidecar: None,
    name: "group_range",
    inputs: GEO,
    params: crate::params!("name":"group1";"domain":"primitives";"start":0;"end":-1;"select":1;"every":1;"offset":0;"invert":false;"combine":"replace"),
    cook: group_range,
};
pub const EDGE_COLLAPSE: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None, cook_sidecar: None,
    name: "edge_collapse",
    inputs: GEO,
    params: crate::params!("group":"";"outgrp_collapsed":false;"outgrp_collapsed_name":"collapsedpoints"),
    cook: edge_collapse,
};
pub const EDGE_FLIP: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None, cook_sidecar: None,
    name: "edge_flip",
    inputs: GEO,
    params: crate::params!("group":"";"outgrp_flipped":false;"outgrp_flipped_name":"flippededges"),
    cook: edge_flip,
};
pub const EDGE_CUSP: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None, cook_sidecar: None,
    name: "edge_cusp",
    inputs: GEO,
    params: crate::params!("group":"";"cusp_angle":180.),
    cook: edge_cusp,
};
pub const POLYSPLIT: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None, cook_sidecar: None,
    name: "polysplit",
    inputs: GEO,
    params: crate::params!("mode":"edge_loop";"split_edges":"";"loops":1;"position":0.5;"factor":0.;"smoothness":0.;"falloff":"inverse_square";"even":false;"flipped":false;"clamp":true;"correct_uvs":true;"split_points":"";"outgrp_points":false;"outgrp_points_name":"splitpoints";"outgrp_edges":false;"outgrp_edges_name":"splitedges"),
    cook: polysplit,
};
// Native keymap calls this tool loop_cut; Python's node type is polysplit.
pub const LOOP_CUT: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None, cook_sidecar: None,
    name: "loop_cut",
    inputs: GEO,
    params: POLYSPLIT.params,
    cook: polysplit,
};
pub const EDGE_SLIDE: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None, cook_sidecar: None,
    name: "edge_slide",
    inputs: GEO,
    params: crate::params!("group":"";"factor":0.;"even":false;"flipped":false;"clamp":true;"mirror_editing":true;"correct_uvs":true;"output_group":false;"output_group_name":"slid_edges"),
    cook: edge_slide,
};
pub(super) fn twins(g: &Geometry) -> Vec<Option<usize>> {
    let (_, nx, _) = corners(g);
    let vp = &g.topology.vertex_point;
    let mut directed = BTreeMap::<(i32, i32), Vec<usize>>::new();
    for v in 0..g.nvertices() {
        directed.entry((vp[v], vp[nx[v]])).or_default().push(v);
    }
    let mut tw = vec![None; g.nvertices()];
    for v in 0..g.nvertices() {
        if directed[&(vp[v], vp[nx[v]])].len() == 1 {
            if let Some(opp) = directed.get(&(vp[nx[v]], vp[v])) {
                if opp.len() == 1 {
                    tw[v] = Some(opp[0]);
                }
            }
        }
    }
    tw
}
pub(super) fn pair_mask(g: &Geometry, pairs: &[(usize, usize)]) -> Vec<bool> {
    let e = topo::edges(&g.topology, g.npoints);
    let pairs: BTreeSet<_> = pairs.iter().map(|&(a, b)| (a.min(b), a.max(b))).collect();
    e.points
        .chunks_exact(2)
        .map(|p| pairs.contains(&(p[0] as usize, p[1] as usize)))
        .collect()
}
pub(super) fn parse_pairs(text: &str, np: usize) -> Result<Vec<(usize, usize)>> {
    let mut pairs = BTreeSet::new();
    for token in text
        .split(|c: char| c == ',' || c.is_whitespace())
        .filter(|s| !s.is_empty())
    {
        let (a, b) = token
            .split_once('-')
            .ok_or_else(|| Error::new("write point pairs like 12-13"))?;
        let a: usize = a.parse().map_err(|_| Error::new("invalid point pair"))?;
        let b: usize = b.parse().map_err(|_| Error::new("invalid point pair"))?;
        if a >= np || b >= np {
            return Err(Error::new("point pair is outside the geometry"));
        }
        if a != b {
            pairs.insert((a.min(b), a.max(b)));
        }
    }
    Ok(pairs.into_iter().collect())
}
pub const TWIST: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None, cook_sidecar: None,
    name: "twist",
    inputs: GEO,
    params: crate::params!("strength":90.;"origin":[0.,0.,0.];"direction":[0.,1.,0.];"group":""),
    cook: twist,
};
pub const LINEAR_TAPER: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None, cook_sidecar: None,
    name: "linear_taper",
    inputs: GEO,
    params: crate::params!("taper_x":0.5;"taper_y":0.5;"length":1.;"dropoff":1.;"origin":[0.,0.,0.];"direction":[0.,1.,0.];"up":[0.,0.,1.];"symmetric":false;"group":""),
    cook: taper,
};
pub const SQUASH_STRETCH: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None, cook_sidecar: None,
    name: "squash_stretch",
    inputs: GEO,
    params: crate::params!("amount":1.5;"preserve_volume":true;"center":[0.,0.,0.];"direction":[0.,1.,0.];"group":""),
    cook: squash,
};
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn quad() -> Geometry {
        Geometry::from_arrays(
            vec![0., 0., 0., 2., 0., 0., 2., 2., 0., 0., 2., 0.],
            vec![0, 1, 2, 3],
            vec![0, 4],
            None,
        )
        .unwrap()
    }
    #[test]
    fn loop_cut_exact_points_and_winding() {
        let p = Params::resolve(
            POLYSPLIT.params,
            json!({"split_edges":"p0-1","outgrp_points":true,"outgrp_edges":true}).as_object(),
        )
        .unwrap();
        let o = polysplit(&[vec![Arc::new(quad())]], &p).unwrap();
        assert_eq!((o.npoints, o.nprims()), (6, 2));
        assert_eq!(&o.p()[12..], &[1., 0., 0., 1., 2., 0.]);
        assert!(
            normals::prim_normals(o.p(), &o.topology)
                .iter()
                .all(|n| n[2] > 0.)
        );
        assert_eq!(
            o.point["group_splitpoints"].data,
            AttrData::I32(vec![0, 0, 0, 0, 1, 1])
        );
    }
    fn cube() -> Geometry {
        let p = vec![
            0., 0., 0., 1., 0., 0., 1., 1., 0., 0., 1., 0., 0., 0., 1., 1., 0., 1., 1., 1., 1., 0., 1., 1.,
        ];
        let f = vec![0, 3, 2, 1, 4, 5, 6, 7, 0, 1, 5, 4, 1, 2, 6, 5, 2, 3, 7, 6, 3, 0, 4, 7];
        Geometry::from_arrays(p, f, vec![0, 4, 8, 12, 16, 20, 24], None).unwrap()
    }
    fn cut(g: Geometry, params: serde_json::Value) -> Geometry {
        let p = Params::resolve(POLYSPLIT.params, params.as_object()).unwrap();
        polysplit(&[vec![Arc::new(g)]], &p).unwrap()
    }
    #[test]
    fn loop_cut_box_ring_is_watertight_for_each_count() {
        for loops in 1..4 {
            let o = cut(cube(), json!({"split_edges":"p0-1","loops":loops}));
            assert_eq!((o.nprims(), o.npoints), (6 + 4 * loops, 8 + 4 * loops));
            assert!((0..o.nprims()).all(|i| range(&o, i).len() == 4));
            let lt = LoopTopology::new(&o);
            assert!(lt.owners.values().all(|f| f.len() == 2), "closed manifold");
        }
    }
    #[test]
    fn loop_cut_position_counts_from_lower_point_and_ignores_pair_direction() {
        let a = cut(cube(), json!({"split_edges":"p0-1","position":0.25}));
        let b = cut(cube(), json!({"split_edges":"p1-0","position":0.25}));
        assert_eq!(a.p(), b.p(), "python sorts p-pairs; direction does not matter");
        assert_eq!(&a.p()[24..27], &[0.25, 0., 0.]);
        // multiple loops ignore position and space evenly
        let c = cut(cube(), json!({"split_edges":"p0-1","loops":3,"position":0.9}));
        assert_eq!(&c.p()[24..33], &[0.25, 0., 0., 0.5, 0., 0., 0.75, 0., 0.]);
    }
    #[test]
    fn loop_preview_lines_are_the_cooked_edges() {
        for (loops, position) in [(1, 0.3), (2, 0.5), (5, 0.5)] {
            let g = cube();
            let ring = LoopTopology::new(&g).ring([(0, 1)]);
            let lines = ring.cut_lines(g.p(), &loop_cut_fractions(loops, position));
            assert_eq!(lines.len(), 4 * loops);
            let o = cut(g, json!({"split_edges":"p0-1","loops":loops,"position":position,"outgrp_edges":true}));
            let pts: Vec<[f32; 3]> = o.p().chunks_exact(3).map(|q| [q[0], q[1], q[2]]).collect();
            let near = |a: [f32; 3], b: [f32; 3]| (0..3).all(|k| (a[k] - b[k]).abs() < 1e-5);
            let id = |x: [f32; 3]| pts.iter().position(|&q| near(q, x)).expect("preview end is a cooked point");
            let lt = LoopTopology::new(&o);
            for (a, b) in lines {
                assert!(lt.owners.contains_key(&key(id(a), id(b))), "preview segment is a cooked edge");
            }
        }
    }
    #[test]
    fn loop_ring_stops_at_triangles_boundaries_and_nonmanifold_edges() {
        // two quads in a row then a triangle: 0-1-2-3, 1-4-5-2, 4-6-5
        let p = vec![0., 0., 0., 1., 0., 0., 1., 1., 0., 0., 1., 0., 2., 0., 0., 2., 1., 0., 3., 0.5, 0.];
        let g = Geometry::from_arrays(p, vec![0, 1, 2, 3, 1, 4, 5, 2, 4, 6, 5], vec![0, 4, 8, 11], None).unwrap();
        let lt = LoopTopology::new(&g);
        let ring = lt.ring([(0, 3)]);
        assert_eq!(ring.quads.len(), 2);
        assert!(ring.dirs.contains_key(&(4, 5)), "the triangle's edge is cut too (no cracks)");
        assert_eq!(lt.ring([(0, 1)]).quads.len(), 1, "boundary stops the strip");
        assert!(lt.ring([(4, 6)]).quads.is_empty(), "a triangle-only edge has no strip");
        assert!(lt.ring([(7, 9)]).dirs.is_empty(), "unknown edges are ignored");
        let o = cut(g, json!({"split_edges":"p0-3"}));
        assert_eq!((o.npoints, o.nprims()), (10, 5));
        assert_eq!(range(&o, 4).len(), 4, "triangle gains the new point");
    }
    #[test]
    fn loop_fractions_match_python() {
        assert_eq!(loop_cut_fractions(1, 0.3), vec![0.3]);
        assert_eq!(loop_cut_fractions(3, 0.3), vec![0.25, 0.5, 0.75]);
    }
    fn slide(g: Geometry, params: serde_json::Value) -> Geometry {
        let p=Params::resolve(EDGE_SLIDE.params,params.as_object()).unwrap();
        edge_slide(&[vec![Arc::new(g)]],&p).unwrap()
    }
    #[test] fn edge_slide_boundary_flip_and_empty_group() {
        let g=quad();
        assert_eq!(slide(g.clone(),json!({})).p(),g.p());
        let positive=slide(g.clone(),json!({"group":"p0-1","factor":0.5}));
        assert_eq!(&positive.p()[..6],&[0.,1.,0.,2.,1.,0.]);
        assert_eq!(slide(g.clone(),json!({"group":"p0-1","factor":-0.5})).p(),g.p());
        assert_eq!(slide(g,json!({"group":"p0-1","factor":-0.5,"flipped":true})).p(),positive.p());
        assert_eq!(positive.topology.vertex_point,quad().topology.vertex_point);
    }
    #[test] fn edge_slide_on_cube_and_polysplit_are_same() {
        let centre=cut(cube(),json!({"split_edges":"p0-1","outgrp_edges":true}));
        let via_slide=slide(centre.clone(),json!({"group":"splitedges","factor":0.4}));
        let direct=cut(cube(),json!({"split_edges":"p0-1","factor":0.4}));
        assert_eq!(via_slide.p(),direct.p());
        assert_eq!(via_slide.topology.vertex_point,direct.topology.vertex_point);
        assert_eq!(slide(centre.clone(),json!({"group":"splitedges","factor":0.4})).p(),via_slide.p());
        let negative=slide(centre,json!({"group":"splitedges","factor":-0.4}));
        assert_ne!(negative.p(),via_slide.p());
    }
    #[test] fn edge_slide_interpolates_point_attributes_and_keeps_uv_seams() {
        let mut g=Geometry::from_arrays(vec![0.,0.,0., 1.,0.,0., 2.,0.,0., 0.,1.,0., 1.,1.,0., 2.,1.,0.],
            vec![0,1,4,3, 1,2,5,4],vec![0,4,8],None).unwrap();
        g.point.insert("weight".into(),Arc::new(Attr::f32(1,vec![0.,1.,2.,0.,1.,2.])));
        g.vertex.insert("uv".into(),Arc::new(Attr::f32(2,vec![0.,0., 1.,0., 1.,1., 0.,1.,
            10.,0., 11.,0., 11.,1., 10.,1.])));
        let o=slide(g.clone(),json!({"group":"p1-4","factor":0.5}));
        assert_eq!(o.topology.vertex_point,g.topology.vertex_point);
        assert_eq!(o.point["weight"].as_f32().unwrap()[1],0.5);
        let uv=o.vertex["uv"].as_f32().unwrap();
        assert_eq!(uv[2],0.5);
        assert_eq!(uv[8],10.);
        let off=slide(g,json!({"group":"p1-4","factor":0.5,"correct_uvs":false}));
        assert_eq!(off.vertex["uv"].as_f32().unwrap()[2],1.);
    }
    #[test] fn edge_slide_even_and_clamp_on_unequal_rails() {
        let g=Geometry::from_arrays(vec![0.,0.,0., 2.,0.,0., 2.,2.,0., 0.,4.,0.],vec![0,1,2,3],vec![0,4],None).unwrap();
        let plain=slide(g.clone(),json!({"group":"p0-1","factor":0.5}));
        let even=slide(g.clone(),json!({"group":"p0-1","factor":0.5,"even":true}));
        assert_eq!(plain.p()[4],1.);
        assert_eq!(even.p()[4],2.);
        let capped=slide(g.clone(),json!({"group":"p0-1","factor":1.,"even":true}));
        let free=slide(g,json!({"group":"p0-1","factor":1.,"even":true,"clamp":false}));
        assert_eq!(capped.p()[4],2.);
        assert_eq!(free.p()[4],4.);
    }
    #[test] fn multiple_loop_cuts_slide_together_and_preview_matches_cook() {
        let g=cube();
        for count in [1,3] { for factor in [-0.5,0.,0.5] {
            let o=cut(g.clone(),json!({"split_edges":"p0-1","loops":count,"factor":factor,"outgrp_edges":true}));
            let lines=loop_cut_preview(&g,(0,1),count,factor,false,false,true).unwrap();
            assert_eq!(lines.len(),4*count);
            let coords: BTreeSet<_>=o.p().chunks_exact(3).map(|q|format!("{:.4},{:.4},{:.4}",q[0],q[1],q[2])).collect();
            for (a,b) in lines { for p in [a,b] { assert!(coords.contains(&format!("{:.4},{:.4},{:.4}",p[0],p[1],p[2]))); } }
        } }
    }
    #[test]
    fn twist_and_singular_taper_normals() {
        let mut g = quad();
        g.point.insert(
            "N".into(),
            Arc::new(Attr::f32(
                3,
                vec![0., 0., 1., 0., 0., 1., 0., 0., 1., 0., 0., 1.],
            )),
        );
        let p = Params::resolve(TWIST.params, None).unwrap();
        let out = twist(&[vec![Arc::new(g.clone())]], &p).unwrap();
        assert!(out.p().iter().all(|x| x.is_finite()));
        let p = Params::resolve(LINEAR_TAPER.params, None).unwrap();
        let out = taper(&[vec![Arc::new(g)]], &p).unwrap();
        out.validate().unwrap();
        assert!(
            out.point["N"]
                .as_f32()
                .unwrap()
                .iter()
                .all(|x| x.is_finite())
        );
    }
    #[test]
    fn group_range_negative_offset_uses_python_modulo() {
        let p = Params::resolve(
            GROUP_RANGE.params,
            json!({"domain":"points","every":3,"offset":-1}).as_object(),
        )
        .unwrap();
        let out = group_range(&[vec![Arc::new(quad())]], &p).unwrap();
        assert_eq!(
            out.point["group_group1"].data,
            AttrData::I32(vec![0, 1, 0, 0])
        );
    }
}
