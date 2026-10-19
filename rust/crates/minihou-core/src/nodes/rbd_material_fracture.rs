//! Material fracture composes the existing Rust fracture, assemble and glue nodes.
use std::sync::Arc;
use super::{GEO, Inputs, NodeOutputs, NodeSpec};
use crate::{error::{Error, Result}, geometry::{Attr, AttrData, Geometry}, params::Params};

pub const RBD_MATERIAL_FRACTURE: NodeSpec = NodeSpec { outputs: &["pieces", "constraints"], cook_multi: Some(cook_multi),
    name: "rbd_material_fracture", inputs: GEO,
    params: crate::params!(
        "material":"concrete"; "density":1000.; "friction":0.6; "restitution":0.1; "glue_strength":1.0e7;
        "pieces":24; "fracture_level":1; "sub_pieces":4; "seed":0; "surface_fraction":0.15; "relax":2;
        "gap":0.; "convex_parts":false; "max_concavity":0.05;
        "interior_detail":false; "detail_divisions":4;
        "detail_amplitude":0.02; "detail_frequency":4.;
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
    Ok(cook_multi(inputs, p)?.0.remove(0))
}
fn cook_multi(inputs: &Inputs, p: &Params) -> Result<NodeOutputs> {
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
    let glue_geo = if p.b("glue")? { glue(split.clone(),p,strength,"constraints")? }
        else { Geometry::default() };
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
    if p.b("interior_detail")? {
        out=super::rbd_interior_detail::apply(out,p)?;
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
    Ok(vec![out, glue_geo].into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn material_fracture_returns_pieces_and_constraints() {
        let box_params=Params::resolve(super::super::generate::BOX.params,None).unwrap();
        let box_geo=(super::super::generate::BOX.cook)(&[vec![]],&box_params).unwrap();
        let p=Params::resolve(RBD_MATERIAL_FRACTURE.params,
            serde_json::json!({"pieces":2,"seed":2,"surface_fraction":0,"relax":0}).as_object()).unwrap();
        let outputs=RBD_MATERIAL_FRACTURE.cook_outputs(&[vec![Arc::new(box_geo)]],&p).unwrap();
        assert_eq!(outputs.0.len(),2);
        assert!(outputs[0].nprims()>0);
        assert!(outputs[0].prim.contains_key("rbd_mass"));
        outputs[1].validate().unwrap();
        assert!(outputs[1].nprims()>0);
        assert!(outputs[1].prim.contains_key("rbd_a"));
    }
    #[test]
    fn material_fracture_caches_both_ports_after_one_cook() {
        let graph=crate::graph::Graph::from_value(&serde_json::json!({"nodes":{
            "box":{"type":"box"},
            "fracture":{"type":"rbd_material_fracture","inputs":{"geo":"box"},
                "params":{"pieces":2,"seed":2,"surface_fraction":0,"relax":0}},
            "pieces":{"type":"null","inputs":{"geo":"fracture"}},
            "constraints":{"type":"null","inputs":{"geo":{"node":"fracture","output":1}}}
        }})).unwrap();
        let mut session=crate::session::Session::new(graph);
        session.cook(Some("pieces")).unwrap();
        assert_eq!(session.last_stats().cooked.iter().filter(|id|*id=="fracture").count(),1);
        assert!(session.cook(Some("constraints")).unwrap().nprims()>0);
        assert!(session.last_stats().cached.contains(&"fracture".into()));
        assert!(!session.last_stats().cooked.contains(&"fracture".into()));
    }
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
    #[test]
    fn interior_box_and_material_oracle() {
        let bp=Params::resolve(super::super::generate::BOX.params,None).unwrap();
        let mut g=(super::super::generate::BOX.cook)(&[vec![]],&bp).unwrap();
        g.detail.insert("materials".into(),serde_json::json!(["a","b"]));
        g.detail.insert("material_bindings".into(),serde_json::json!({"version":1,"slots":["a","b"]}));
        g.prim.insert("material_index".into(),Arc::new(Attr::i32(1,vec![0,1,0,1,0,1])));
        g.vertex.insert("uv".into(),Arc::new(Attr::f32(2,(0..g.nvertices()).flat_map(|v|[(v%4) as f32/3.,(v%4) as f32/3.]).collect())));
        let p=Params::resolve(RBD_MATERIAL_FRACTURE.params,serde_json::json!({
            "pieces":2,"seed":2,"surface_fraction":0,"relax":0,"glue":false,
            "interior_detail":true,"detail_divisions":2,"detail_amplitude":0,
            "outgrp_cutpoints":true,"outgrp_cutedges":true}).as_object()).unwrap();
        let out=cook(&[vec![Arc::new(g)]],&p).unwrap();
        let oracle:serde_json::Value=serde_json::from_str(include_str!("test_data/last_nodes_parity.json")).unwrap();
        let expected=&oracle["material_interior"]["two_materials"];
        assert_eq!(out.npoints,expected["points"].as_u64().unwrap() as usize);
        assert_eq!(out.nprims(),expected["prims"].as_u64().unwrap() as usize);
        assert_eq!(out.prim["group_inside"].as_i32().unwrap().iter().sum::<i32>(),expected["inside"].as_i64().unwrap() as i32);
        assert_eq!(out.point["group_cutpoints"].as_i32().unwrap().iter().sum::<i32>(),expected["cutpoints"].as_i64().unwrap() as i32);
        let uv=out.vertex["uv"].as_f32().unwrap();
        let finite=uv.chunks_exact(2).filter(|v|v.iter().all(|x|x.is_finite())).count();
        assert_eq!(finite,expected["uv_finite_corners"].as_u64().unwrap() as usize);
        out.validate().unwrap();
    }
    #[test]
    fn sphere_interior_oracle() {
        let sp=Params::resolve(super::super::generate::SPHERE.params,
            serde_json::json!({"rows":8,"columns":12}).as_object()).unwrap();
        let g=(super::super::generate::SPHERE.cook)(&[vec![]],&sp).unwrap();
        let p=Params::resolve(RBD_MATERIAL_FRACTURE.params,serde_json::json!({
            "pieces":2,"seed":2,"surface_fraction":0,"relax":0,"glue":false,
            "interior_detail":true,"detail_divisions":2,"detail_amplitude":0,
            "outgrp_cutpoints":true,"outgrp_cutedges":true}).as_object()).unwrap();
        let out=cook(&[vec![Arc::new(g)]],&p).unwrap();
        let oracle:serde_json::Value=serde_json::from_str(include_str!("test_data/last_nodes_parity.json")).unwrap();
        let expected=&oracle["material_interior"]["sphere"];
        assert_eq!(out.npoints,expected["points"].as_u64().unwrap() as usize);
        assert_eq!(out.nprims(),expected["prims"].as_u64().unwrap() as usize);
        assert_eq!(out.prim["group_inside"].as_i32().unwrap().iter().sum::<i32>(),expected["inside"].as_i64().unwrap() as i32);
        assert_eq!(out.point["group_cutpoints"].as_i32().unwrap().iter().sum::<i32>(),expected["cutpoints"].as_i64().unwrap() as i32);
    }
    #[test]
    fn concave_interior_and_compound_are_valid() {
        let xy=[[0.,0.],[2.,0.],[2.,1.],[1.,1.],[1.,2.],[0.,2.]];
        let pts:Vec<f32>=[0.,1.].into_iter().flat_map(|z|xy.into_iter().flat_map(move |q|[q[0],q[1],z])).collect();
        let mut faces=vec![(0..6).rev().collect::<Vec<i32>>(),(6..12).collect()];
        for i in 0..6 {faces.push(vec![i,(i+1)%6,(i+1)%6+6,i+6]);}
        let mut vp=Vec::new();let mut offsets=vec![0];for face in faces {vp.extend(face);offsets.push(vp.len() as i32);}
        let g=Geometry::from_arrays(pts,vp,offsets,None).unwrap();
        for compound in [false,true] {
            let p=Params::resolve(RBD_MATERIAL_FRACTURE.params,serde_json::json!({
                "pieces":2,"seed":2,"surface_fraction":0,"relax":0,"glue":false,
                "convex_parts":compound,"max_concavity":0.,"interior_detail":true,
                "detail_divisions":2,"detail_amplitude":0.,"outgrp_cutpoints":true,
                "outgrp_cutedges":true}).as_object()).unwrap();
            let out=cook(&[vec![Arc::new(g.clone())]],&p).unwrap();
            out.validate().unwrap();
            assert!(out.npoints>g.npoints && out.nprims()>g.nprims());
            if !compound {
                let oracle:serde_json::Value=serde_json::from_str(include_str!("test_data/last_nodes_parity.json")).unwrap();
                let expected=&oracle["material_interior"]["l_prism"];
                assert_eq!((out.npoints,out.nprims()),(expected["points"].as_u64().unwrap() as usize,expected["prims"].as_u64().unwrap() as usize));
            }
            if compound {
                assert!(out.prim.contains_key("rbd_hull"));
                println!("L prism compound: {} points, {} faces",out.npoints,out.nprims());
            }
        }
    }
    #[test]
    #[ignore = "production-sized performance sample; run explicitly"]
    fn stress_sphere_24_pieces() {
        let sp=Params::resolve(super::super::generate::SPHERE.params,
            serde_json::json!({"rows":36,"columns":72}).as_object()).unwrap();
        let g=(super::super::generate::SPHERE.cook)(&[vec![]],&sp).unwrap();
        assert!(g.nprims()>2000);
        let p=Params::resolve(RBD_MATERIAL_FRACTURE.params,serde_json::json!({
            "pieces":24,"seed":2,"surface_fraction":0.15,"relax":2,"glue":false,
            "interior_detail":true,"detail_divisions":2,"detail_amplitude":0.02,
            "outgrp_cutpoints":true,"outgrp_cutedges":true}).as_object()).unwrap();
        let start=std::time::Instant::now();
        let out=cook(&[vec![Arc::new(g)]],&p).unwrap();
        println!("rbd stress: {:.3}s, {} points, {} faces, {} pieces",start.elapsed().as_secs_f64(),out.npoints,out.nprims(),
            piece_ids(&out).unwrap().into_iter().collect::<std::collections::BTreeSet<_>>().len());
        out.validate().unwrap();
        assert!(out.prim["piece"].as_i32().unwrap().iter().copied().collect::<std::collections::BTreeSet<_>>().len()>=20);
    }
    #[test]
    fn detail_controls_and_missing_group() {
        let bp=Params::resolve(super::super::generate::BOX.params,None).unwrap();
        let g=(super::super::generate::BOX.cook)(&[vec![]],&bp).unwrap();
        let base=serde_json::json!({"pieces":2,"seed":2,"surface_fraction":0,"relax":0,"glue":false,
            "interior_detail":true,"detail_divisions":3,"detail_amplitude":0.,"detail_frequency":7.,
            "outgrp_inside_name":"caps","outgrp_cutpoints":true,"outgrp_cutpoints_name":"seampts",
            "outgrp_cutedges":true,"outgrp_cutedges_name":"seamedges"});
        let p=Params::resolve(RBD_MATERIAL_FRACTURE.params,base.as_object()).unwrap();
        let flat=cook(&[vec![Arc::new(g.clone())]],&p).unwrap();
        assert!(flat.prim.contains_key("group_caps") && flat.point.contains_key("group_seampts"));
        assert!(flat.vertex.contains_key("edge_group_seamedges"));
        let mut rough=base.clone();rough["detail_amplitude"]=serde_json::json!(0.1);
        let p=Params::resolve(RBD_MATERIAL_FRACTURE.params,rough.as_object()).unwrap();
        let noisy=cook(&[vec![Arc::new(g.clone())]],&p).unwrap();
        assert_eq!(flat.npoints,noisy.npoints);
        let fixed=flat.point["group_seampts"].as_i32().unwrap();
        assert!(flat.p().chunks_exact(3).zip(noisy.p().chunks_exact(3)).zip(fixed)
            .all(|((a,b),&f)|f==0 || a==b));
        assert!(flat.p().iter().zip(noisy.p()).any(|(a,b)|a!=b));
        rough["outgrp_inside"]=serde_json::json!(false);
        let p=Params::resolve(RBD_MATERIAL_FRACTURE.params,rough.as_object()).unwrap();
        assert!(cook(&[vec![Arc::new(g)]],&p).unwrap_err().0.contains("inside faces group"));
    }
}

