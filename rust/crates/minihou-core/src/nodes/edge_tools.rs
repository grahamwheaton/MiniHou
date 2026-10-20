//! Derived-edge selections and conservative polygon operations from edge_tools.py.
use super::{GEO, InputSpec, Inputs, NodeSpec, groups as node_groups, model::*};
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
mod common;
mod groups;
mod dissolve;
mod bridge;
pub(super) use common::{values, write_values, wildcard, remap};
pub(crate) use common::{edge_mask, rebuild_edges};
pub(super) use dissolve::root;
pub(super) use bridge::chain;
use groups::{crease, edge_group, promote};
use dissolve::dissolve;
use bridge::polybridge;
pub const CREASE: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None, cook_sidecar: None,
    name: "crease",
    inputs: GEO,
    params: crate::params!("domain":"edges";"group":"";"sharpness":2.),
    cook: crease,
};
pub const EDGE_GROUP: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None, cook_sidecar: None,
    name: "edge_group",
    inputs: &[
        InputSpec {
            name: "geo",
            multi: false,
            optional: false,
        },
        InputSpec {
            name: "bounds",
            multi: false,
            optional: true,
        },
    ],
    params: crate::params!("name":"edges";"method":"pattern";"pattern":"*";"combine":"replace"),
    cook: edge_group,
};
pub const EDGE_GROUP_PROMOTE: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None, cook_sidecar: None,
    name: "edge_group_promote",
    inputs: GEO,
    params: crate::params!("pattern":"*";"name":"edge_points";"to":"points"),
    cook: promote,
};
pub const DISSOLVE: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None, cook_sidecar: None,
    name: "dissolve_edges",
    inputs: GEO,
    params: crate::params!("group":"";"angle":0.1;"preserve_seams":true),
    cook: dissolve,
};
pub const POLYBRIDGE: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None, cook_sidecar: None,
    name: "polybridge",
    inputs: GEO,
    params: crate::params!("source":"";"destination":"";"segments":1;"twist":0;"reverse":false;"outgrp_bridge":false;"outgrp_bridge_name":"bridgepolys"),
    cook: polybridge,
};
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn polybridge_uses_two_group_create_edge_loops_on_box() {
        let box_params = Params::resolve(super::super::generate::BOX.params, None).unwrap();
        let box_geo = (super::super::generate::BOX.cook)(&[vec![]], &box_params).unwrap();
        // Open the two caps so the selected loops are bridgeable boundaries.
        let keep = super::super::group::centroids(&box_geo).chunks_exact(3)
            .map(|p| p[2].abs() < 0.25).collect::<Vec<_>>();
        let mut g = super::super::group::keep_prims(&box_geo, &keep, false).unwrap();
        let edges = topo::edges(&g.topology, g.npoints);
        for (name, z) in [("group1", -0.5), ("group2", 0.5)] {
            let pattern = edges.points.chunks_exact(2).filter_map(|pair| {
                let a = pair[0] as usize;
                let b = pair[1] as usize;
                (g.p()[3*a+2] == z && g.p()[3*b+2] == z)
                    .then(|| format!("{}-{}", a, b))
            }).collect::<Vec<_>>().join(" ");
            let params = Params::resolve(super::super::group::GROUP_CREATE.params,
                json!({"name":name,"domain":"edges","pattern":pattern}).as_object()).unwrap();
            g = (super::super::group::GROUP_CREATE.cook)(&[vec![Arc::new(g)],vec![]], &params).unwrap();
            assert_eq!(g.edge[&format!("group_{name}")].as_i32().unwrap().iter().filter(|&&v| v != 0).count(), 4);
        }
        let params = Params::resolve(POLYBRIDGE.params,
            json!({"source":"group1","destination":"group2"}).as_object()).unwrap();
        let out = polybridge(&[vec![Arc::new(g)]], &params).unwrap();
        assert_eq!(out.nprims(), 8);
        out.validate().unwrap();
    }
    #[test]
    fn edge_groups_and_crease_live_on_outgoing_corners() {
        let g = Geometry::from_arrays(
            vec![0., 0., 0., 1., 0., 0., 1., 1., 0., 0., 1., 0.],
            vec![0, 1, 2, 0, 2, 3],
            vec![0, 3, 6],
            None,
        )
        .unwrap();
        let p = Params::resolve(
            CREASE.params,
            json!({"group":"p0-2","sharpness":3}).as_object(),
        )
        .unwrap();
        let out = crease(&[vec![Arc::new(g)]], &p).unwrap();
        assert_eq!(
            values(&out, "creaseweight").unwrap(),
            vec![0., 3., 0., 0., 0.]
        );
        assert_eq!(
            out.vertex["creaseweight"].data,
            AttrData::F32(vec![0., 0., 3., 3., 0., 0.])
        );
    }
    #[test]
    fn dissolve_rejects_material_seams() {
        let mut g = Geometry::from_arrays(
            vec![0., 0., 0., 1., 0., 0., 1., 1., 0., 0., 1., 0.],
            vec![0, 1, 2, 0, 2, 3],
            vec![0, 3, 6],
            None,
        )
        .unwrap();
        let p = Params::resolve(DISSOLVE.params, json!({"group":"p0-2"}).as_object()).unwrap();
        let out = dissolve(&[vec![Arc::new(g.clone())]], &p).unwrap();
        assert_eq!(out.nprims(), 1);
        assert_eq!(out.nvertices(), 4);
        g.prim
            .insert("material_id".into(), Arc::new(Attr::i32(1, vec![1, 2])));
        assert!(dissolve(&[vec![Arc::new(g)]], &p).is_err());
    }
}
