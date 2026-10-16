//! Material fracture composes the existing Rust fracture, assemble and glue nodes.
use std::sync::Arc;
use super::{GEO, Inputs, NodeSpec};
use crate::{error::{Error, Result}, geometry::{Attr, AttrData, Geometry}, params::Params};

pub const RBD_MATERIAL_FRACTURE: NodeSpec = NodeSpec {
    name: "rbd_material_fracture", inputs: GEO,
    params: crate::params!(
        "material":"concrete"; "density":1000.; "friction":0.6; "restitution":0.1; "glue_strength":1.0e7;
        "pieces":24; "fracture_level":1; "sub_pieces":4; "seed":0; "surface_fraction":0.15; "relax":2;
        "gap":0.; "convex_parts":false; "max_concavity":0.05;
        "interior_detail":false,unsupported; "detail_divisions":4,unsupported;
        "detail_amplitude":0.02,unsupported; "detail_frequency":4.,unsupported;
        "glue":true; "clusters":0; "strength_scale":1.; "mass_scale":1.; "output":"pieces";
        "outgrp_inside":true; "outgrp_inside_name":"inside"; "outgrp_outside":false; "outgrp_outside_name":"outside";
        "outgrp_cutpoints":false; "outgrp_cutpoints_name":"cutpoints";
        "outgrp_cutedges":false; "outgrp_cutedges_name":"cutedges"
    ), cook: cook,
};

fn invoke(spec: &NodeSpec, inputs: &[Vec<Arc<Geometry>>], values: serde_json::Value) -> Result<Geometry> {
    let params = Params::resolve(spec.params, values.as_object())?;
    (spec.cook)(inputs, &params)
}
fn piece_ids(g: &Geometry) -> Result<Vec<i32>> {
    match g.prim.get("piece").map(|a| &a.data) {
        Some(AttrData::I32(v)) => Ok(v.clone()),
        _ => Err(Error::new("material fracture expected primitive piece attribute")),
    }
}
fn volume(g: &Geometry) -> f64 {
    let p = super::model::positions(g);
    let mut sum = 0.;
    for face in 0..g.nprims() {
        let r = super::model::range(g, face);
        if r.len() < 3 { continue; }
        let v = &g.topology.vertex_point;
        let a = p[v[r.start] as usize];
        for k in r.start + 1..r.end - 1 {
            let b = p[v[k] as usize]; let c = p[v[k+1] as usize];
            sum += super::model::dot(a, super::model::cross(b,c)) / 6.;
        }
    }
    sum.abs()
}
fn fracture(g: Geometry, count: i64, seed: i64, surface: f64, relax: i64, gap: f64, p: &Params) -> Result<Geometry> {
    let input = Arc::new(g);
    let seeds = invoke(&super::voronoi_points::VORONOI_FRACTURE_POINTS,
        &[vec![input.clone()], vec![]], serde_json::json!({"count":count,"seed":seed,
            "surface_fraction":surface,"relax":relax}))?;
    let mut opts = serde_json::json!({"gap":gap,"outgrp_inside":p.b("outgrp_inside")?,
        "outgrp_inside_name":p.s("outgrp_inside_name")?,"outgrp_outside":p.b("outgrp_outside")?,
        "outgrp_outside_name":p.s("outgrp_outside_name")?,"outgrp_cutpoints":p.b("outgrp_cutpoints")?,
        "outgrp_cutpoints_name":p.s("outgrp_cutpoints_name")?,"outgrp_cutedges":p.b("outgrp_cutedges")?,
        "outgrp_cutedges_name":p.s("outgrp_cutedges_name")?});
    let cut = invoke(&super::fracture_voronoi::VORONOI_FRACTURE,
        &[vec![input],vec![Arc::new(seeds)]], std::mem::take(&mut opts))?;
    invoke(&super::fracture_tools::ASSEMBLE, &[vec![Arc::new(cut)]],
        serde_json::json!({"source":"attribute","piece_attrib":"piece","output_attrib":"piece",
            "unshare_points":true}))
}
fn glue(g: Geometry, p: &Params, strength: f64, output: &str) -> Result<Geometry> {
    invoke(&super::fracture_tools::GLUE_CLUSTER, &[vec![Arc::new(g)]],
        serde_json::json!({"output":output,"strength":strength,"strength_variation":0.25,
            "seed":p.i("seed")?,"max_gap":p.f("gap")?*1.2,"min_share":0.05,
            "clusters":p.i("clusters")?,"weak":0.5}))
}
fn cook(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let source = RBD_MATERIAL_FRACTURE.one(inputs,0)?;
    let count=p.i("pieces")?; let levels=p.i("fracture_level")?; let sub=p.i("sub_pieces")?;
    if count<2 || sub<2 || !(1..=4).contains(&levels) { return Err(Error::new("invalid material fracture piece counts or level")); }
    let total=(count as u64).saturating_mul((sub as u64).saturating_pow((levels-1) as u32));
    if total>2000 { return Err(Error::new(format!("material fracture would make about {total} pieces (limit 2000)"))); }
    let material=p.e("material", &["concrete","brick","stone","wood","glass","ceramic","ice","metal","custom"])?;
    let (density, friction, restitution, strength)=match material.as_str() {
        "concrete"=>(2400.,0.7,0.05,1.2e7),"brick"=>(1900.,0.6,0.05,6.0e6),
        "stone"=>(2700.,0.7,0.05,2.0e7),"wood"=>(700.,0.5,0.2,3.0e7),
        "glass"=>(2500.,0.4,0.1,6.0e6),"ceramic"=>(2300.,0.45,0.15,8.0e6),
        "ice"=>(920.,0.05,0.05,4.0e6),"metal"=>(7800.,0.35,0.2,8.0e8),
        _=>(p.f("density")?,p.f("friction")?,p.f("restitution")?,p.f("glue_strength")?),
    };
    let gap=p.f("gap")?;
    let mut split=fracture((*source).clone(),count,p.i("seed")?,p.f("surface_fraction")?,p.i("relax")?,gap,p)?;
    for level in 2..=levels {
        let ids=piece_ids(&split)?;
        let n=ids.iter().copied().max().unwrap_or(-1)+1;
        let mut parts=Vec::new(); let mut offset=0i32;
        for k in 0..n {
            let part=super::group::keep_prims(&split,&ids.iter().map(|&id|id==k).collect::<Vec<_>>(),false)?;
            let mut cut=fracture(part,sub,p.i("seed")?+31*level+7*k as i64,p.f("surface_fraction")?,1,gap,p)?;
            let labels=piece_ids(&cut)?;
            let local=labels.iter().copied().max().unwrap_or(-1)+1;
            cut.prim.insert("piece".into(),Arc::new(Attr::i32(1,labels.into_iter().map(|v|v+offset).collect())));
            offset+=local; parts.push(Arc::new(cut));
        }
        let joined=invoke(&super::merge::MERGE,&[parts],serde_json::json!({}))?;
        split=invoke(&super::fracture_tools::ASSEMBLE,&[vec![Arc::new(joined)]],
            serde_json::json!({"source":"attribute","unshare_points":true}))?;
    }
    let ids=piece_ids(&split)?; let n=ids.iter().copied().max().unwrap_or(-1).max(0) as usize+1;
    let mut masses=vec![1e-6f64;n];
    for k in 0..n { let part=super::group::keep_prims(&split,&ids.iter().map(|&id|id==k as i32).collect::<Vec<_>>(),false)?;
        masses[k]=(volume(&part)*density*p.f("mass_scale")?).max(1e-6); }
    let strength=strength*p.f("strength_scale")?;
    if p.s("output")?=="constraints" {
        if !p.b("glue")? {return Err(Error::new("output = constraints needs glue switched on"));}
        return glue(split,p,strength,"constraints");
    }
    let mut out=if p.b("glue")? && p.i("clusters")?>0 {glue(split,p,strength,"pieces")?} else {split};
    let mut compound=false;
    if p.b("convex_parts")? {
        let labels=piece_ids(&out)?;let mut parts=Vec::new();
        for k in 0..n {
            let part=super::group::keep_prims(&out,&labels.iter().map(|&id|id==k as i32).collect::<Vec<_>>(),false)?;
            if part.nprims()==0 {continue;}
            let concavity=super::convex_decomposition::missing(&part)?/volume(&part).max(1e-300);
            if concavity>p.f("max_concavity")? {
                let source_piece=part.prim["piece"].as_i32().unwrap()[0];
                let source_cluster=part.prim.get("cluster").and_then(|a|a.as_i32()).map(|v|v[0]);
                let mut dec=invoke(&super::convex_decomposition::CONVEX_DECOMPOSITION,&[vec![Arc::new(part)]],
                    serde_json::json!({"max_concavity":p.f("max_concavity")?,"max_pieces":16,"min_volume":0.001,
                        "search":8,"piece_attrib":"part","outgrp_inside":p.b("outgrp_inside")?,
                        "outgrp_inside_name":p.s("outgrp_inside_name")?,"outgrp_outside":p.b("outgrp_outside")?,
                        "outgrp_outside_name":p.s("outgrp_outside_name")?,"outgrp_cutpoints":p.b("outgrp_cutpoints")?,
                        "outgrp_cutpoints_name":p.s("outgrp_cutpoints_name")?,"outgrp_cutedges":p.b("outgrp_cutedges")?,
                        "outgrp_cutedges_name":p.s("outgrp_cutedges_name")?}))?;
                dec.prim.insert("piece".into(),Arc::new(Attr::i32(1,vec![source_piece;dec.nprims()])));
                if let Some(cluster)=source_cluster {dec.prim.insert("cluster".into(),Arc::new(Attr::i32(1,vec![cluster;dec.nprims()])));}
                parts.push(Arc::new(dec));compound=true;
            } else {parts.push(Arc::new(part));}
        }
        if !parts.is_empty() {out=invoke(&super::merge::MERGE,&[parts],serde_json::json!({}))?;}
    }
    let ids=piece_ids(&out)?;
    let body=out.prim.get("cluster").and_then(|a|a.as_i32()).map(|v|v.to_vec()).unwrap_or_else(||ids.clone());
    let body_mass:Vec<f64>=ids.iter().map(|&id| {
        if out.prim.contains_key("cluster") {let cluster=body[ids.iter().position(|&v|v==id).unwrap()];
            let mut seen=std::collections::BTreeSet::new();
            ids.iter().zip(&body).filter(|(_, &b)|b==cluster).filter_map(|(&piece,_)|seen.insert(piece).then_some(masses[piece as usize])).sum()}
        else {masses[id as usize]}
    }).collect();
    let np=out.nprims(); let clustered=out.prim.contains_key("cluster");
    let mut put_i=|name:&str,v:Vec<i32>|{out.prim.insert(name.into(),Arc::new(Attr::i32(1,v)));};
    put_i("rbd_id",body);put_i("rbd_active",vec![1;np]);
    if clustered || compound {put_i("rbd_compound",vec![1;np]);}
    if compound {put_i("rbd_hull",vec![1;np]);}
    for (name,v) in [("rbd_mass",body_mass.iter().map(|&v|v as f32).collect()),
        ("rbd_friction",vec![friction as f32;np]),("rbd_restitution",vec![restitution as f32;np]),
        ("rbd_linear_damping",vec![0.05;np]),("rbd_angular_damping",vec![0.1;np])] {
        out.prim.insert(name.into(),Arc::new(Attr::f32(1,v)));
    }
    out.prim.insert("rbd_v".into(),Arc::new(Attr::f32(3,vec![0.;np*3])));
    out.prim.insert("rbd_w".into(),Arc::new(Attr::f32(3,vec![0.;np*3])));
    out.detail.insert("rbd_material".into(),serde_json::json!(material));
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn python_two_piece_box_golden() {
        // Python Evaluator: box -> rbd_material_fracture(pieces=2,seed=2,
        // surface_fraction=0,relax=0,glue=False).
        let box_params=Params::resolve(super::super::generate::BOX.params,None).unwrap();
        let box_geo=(super::super::generate::BOX.cook)(&[vec![]],&box_params).unwrap();
        let params=Params::resolve(RBD_MATERIAL_FRACTURE.params,
            serde_json::json!({"pieces":2,"seed":2,"surface_fraction":0,"relax":0,"glue":false}).as_object()).unwrap();
        let out=cook(&[vec![Arc::new(box_geo)]],&params).unwrap();
        let oracle:serde_json::Value=serde_json::from_str(include_str!("test_data/last_nodes_parity.json")).unwrap();
        let expected=&oracle["material_box"];
        assert_eq!(out.npoints,expected["points"].as_u64().unwrap() as usize);
        assert_eq!(out.nprims(),expected["prims"].as_u64().unwrap() as usize);
        assert_eq!(piece_ids(&out).unwrap(),[vec![0;6],vec![1;6]].concat());
        assert_eq!(out.detail["rbd_material"],"concrete");
        let masses=out.prim["rbd_mass"].as_f32().unwrap();
        assert!((masses[0]-expected["mass"][0].as_f64().unwrap() as f32).abs()<0.1,"{masses:?}");
        assert!((masses[6]-expected["mass"][1].as_f64().unwrap() as f32).abs()<0.1,"{masses:?}");
        assert!(out.prim.contains_key("group_inside"));
    }
}

