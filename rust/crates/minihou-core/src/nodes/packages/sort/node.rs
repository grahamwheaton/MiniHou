use std::sync::Arc;
use crate::nodes::{Inputs, NodeSpec, GEO};
use crate::error::{Error, Result};
use crate::geometry::{AttrData, Geometry, Topology};
use crate::params::{Params};
use crate::nodes::attrib_tools::{domain, size};
pub(super) const NODE: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None, cook_sidecar: None,
    name: "sort",
    inputs: GEO,
    params: crate::params!(
        "domain": "points";
        "key": "random";
        "attribute": "";
        "seed": 0;
    ),
    cook: sort,
};

fn sort(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let g = NODE.one(inputs, 0)?;
    let dom = p.e("domain", &["points", "primitives"])?;
    let dom = domain(&dom);
    let key = p.e("key", &["random", "x", "y", "z", "attribute", "reverse"])?;
    let seed = p.i("seed")?;
    let attribute = p.s("attribute")?;
    let n = size(&g, dom);
    if n == 0 {
        return Ok((*g).clone());
    }

    let order = match key.as_str() {
        "random" => {
            let mut rng = crate::nodes::np_random::Rng::new(&[(seed as u64) & 0xFFFFFFFF]);
            rng.permutation(n)
        }
        "reverse" => (0..n).rev().collect(),
        "x" | "y" | "z" => {
            let axis = match key.as_str() {
                "x" => 0,
                "y" => 1,
                _ => 2,
            };
            let mut order: Vec<usize> = (0..n).collect();
            if dom == "point" {
                let p_slice = g.p();
                order.sort_by(|&a, &b| p_slice[a * 3 + axis].total_cmp(&p_slice[b * 3 + axis]));
            } else {
                let centres = crate::nodes::group::centroids64(&g);
                order.sort_by(|&a, &b| centres[a * 3 + axis].total_cmp(&centres[b * 3 + axis]));
            }
            order
        }
        "attribute" => {
            let a = g.table(dom).get(&attribute)
                .ok_or_else(|| Error::new(format!("no {dom} attribute {attribute:?}")))?;
            let mut order: Vec<usize> = (0..n).collect();
            match &a.data {
                AttrData::F32(v) => order.sort_by(|&i, &j| v[i * a.width].total_cmp(&v[j * a.width])),
                AttrData::I32(v) => order.sort_by(|&i, &j| v[i * a.width].cmp(&v[j * a.width])),
            }
            order
        }
        _ => unreachable!(),
    };

    if dom == "point" {
        let mut inv = vec![0i32; n];
        for (new_i, &old_i) in order.iter().enumerate() {
            inv[old_i] = new_i as i32;
        }
        let old_p = g.p();
        let mut new_p = vec![0f32; n * 3];
        for (new_i, &old_i) in order.iter().enumerate() {
            new_p[new_i * 3..new_i * 3 + 3].copy_from_slice(&old_p[old_i * 3..old_i * 3 + 3]);
        }
        let new_vp: Vec<i32> = g.topology.vertex_point.iter().map(|&pt| inv[pt as usize]).collect();
        let topo = Topology {
            vertex_point: new_vp,
            prim_offsets: g.topology.prim_offsets.clone(),
            closed: g.topology.closed.clone(),
        };
        let mut out = Geometry { packed: Default::default(), edge: Default::default(),
            npoints: n,
            topology: Arc::new(topo),
            point: crate::nodes::group::gather_table(&g.point, &order),
            vertex: g.vertex.clone(),
            prim: g.prim.clone(),
            detail: g.detail.clone(),
        };
        out.set_p(new_p);
        Ok(out)
    } else {
        let t = &g.topology;
        let mut new_offsets = Vec::with_capacity(n + 1);
        new_offsets.push(0i32);
        let mut verts = Vec::with_capacity(t.vertex_point.len());
        let mut cur_off = 0i32;
        for &old_prim in &order {
            let start = t.prim_offsets[old_prim] as usize;
            let end = t.prim_offsets[old_prim + 1] as usize;
            for v in start..end {
                verts.push(v);
            }
            cur_off += (end - start) as i32;
            new_offsets.push(cur_off);
        }
        let new_vp: Vec<i32> = verts.iter().map(|&v| t.vertex_point[v]).collect();
        let closed = t.closed.as_ref().map(|c| order.iter().map(|&p| c[p]).collect());
        let topo = Topology {
            vertex_point: new_vp,
            prim_offsets: new_offsets,
            closed,
        };
        let out = Geometry { packed: Default::default(), edge: Default::default(),
            npoints: g.npoints,
            topology: Arc::new(topo),
            point: g.point.clone(),
            vertex: crate::nodes::group::gather_table(&g.vertex, &verts),
            prim: crate::nodes::group::gather_table(&g.prim, &order),
            detail: g.detail.clone(),
        };
        Ok(out)
    }
}


#[cfg(test)]
#[path = "tests.rs"]
mod tests;
