//! Seed points for Voronoi fracture, following minihou/nodes/fracture_tools.py.
use std::sync::Arc;
use super::{InputSpec, Inputs, NodeSpec, model::{add,sub,mul,len,cross,positions,outgroup}, np_random::Rng, ray::RayMesh, volume::triangulation};
use crate::{error::{Error,Result}, geometry::{Attr,Geometry}, params::Params};

pub const VORONOI_FRACTURE_POINTS:NodeSpec=NodeSpec{name:"voronoi_fracture_points",inputs:&[
    InputSpec{name:"geo",multi:false,optional:false},InputSpec{name:"points",multi:false,optional:true}],params:crate::params!(
    "count":20;"seed":0;"surface_fraction":0.;"min_distance":0.;"relax":0;"cluster_amount":0.;"clusters":3;
    "cluster_radius":0.25;"piece_attrib":"piece";"source":"scatter";"impact_radius":0.4;"impact_share":0.7;"merge_clusters":0;
    "outgrp_interior":false;"outgrp_interior_name":"interior";"outgrp_surface":false;"outgrp_surface_name":"surface";
    "outgrp_clustered":false;"outgrp_clustered_name":"clustered";"outgrp_extra":false;"outgrp_extra_name":"extrapoints";
    "outgrp_impact":false;"outgrp_impact_name":"impactpoints"),cook:voronoi_points};

fn nearest(p:[f64;3],centres:&[[f64;3]])->usize {centres.iter().enumerate().min_by(|(_,a),(_,b)|len(sub(p,**a)).total_cmp(&len(sub(p,**b)))).map(|(i,_)|i).unwrap_or(0)}
fn sample(mesh:&RayMesh,rng:&mut Rng,count:usize,lo:[f64;3],ext:[f64;3],scale:f64,centre:Option<([f64;3],f64)>)->Vec<[f64;3]> {
    let mut found=Vec::new();let mut frac:f64=0.5;
    for _ in 0..48 {if found.len()>=count{break;}
        let n=(((count-found.len()) as f64/frac.max(0.02)*1.3).max(256.) as usize).min(400_000);
        let mut cand=Vec::with_capacity(n);
        if let Some((c,sigma))=centre {for _ in 0..n{cand.push(add(c,[rng.standard_normal()*sigma,rng.standard_normal()*sigma,rng.standard_normal()*sigma]));}}
        else {for _ in 0..n{cand.push(std::array::from_fn(|k|lo[k]+rng.random()*ext[k]));}}
        let mut accepted=0;for pt in cand{if mesh.contains(pt,scale){found.push(pt);accepted+=1;}}
        frac=(accepted as f64/n as f64).max(1e-3);
    }found.truncate(count);found
}
fn voronoi_points(inputs:&Inputs,p:&Params)->Result<Geometry> {
    let geo=VORONOI_FRACTURE_POINTS.one(inputs,0)?;
    super::model::polygons(&geo,"voronoi_fracture_points")?;
    if geo.nprims()==0{return Err(Error::new("voronoi_fracture_points needs a closed mesh to scatter inside"));}
    let count=p.i("count")?.max(0) as usize;let source=p.e("source", &["scatter","impacts"])?;
    let mut rng=Rng::new(&[(p.i("seed")? as u64)&0xffff_ffff,29]);
    let pos=positions(&geo);let mut lo=[f64::INFINITY;3];let mut hi=[f64::NEG_INFINITY;3];
    for pt in &pos{for k in 0..3{lo[k]=lo[k].min(pt[k]);hi[k]=hi[k].max(pt[k]);}}
    let ext=std::array::from_fn(|k|(hi[k]-lo[k]).max(1e-12));let scale=ext.iter().copied().fold(0.,f64::max);
    let ns=(count as f64*p.f("surface_fraction")?).round().clamp(0.,count as f64) as usize;
    let nc=((count-ns) as f64*p.f("cluster_amount")?).round().clamp(0.,(count-ns) as f64) as usize;
    let ni=count-ns-nc;
    if ni+nc>0 {let edges=crate::topo::edges(&geo.topology,geo.npoints);if edges.count.iter().any(|&c|c!=2){return Err(Error::new("voronoi_fracture_points needs a closed mesh to scatter inside (set surface_fraction to 1 for an open one)"));}}
    let mesh=RayMesh::new(&geo);
    let mut inner=sample(&mesh,&mut rng,ni,lo,ext,scale,None);
    let mut clustered=Vec::new();if nc>0 {let centres=sample(&mesh,&mut rng,p.i("clusters")?.max(1) as usize,lo,ext,scale,None);
        for (i,c) in centres.iter().enumerate(){let amount=nc/centres.len()+usize::from(i<nc%centres.len());clustered.extend(sample(&mesh,&mut rng,amount,lo,ext,scale,Some((*c,p.f("cluster_radius")?*scale))));}}
    for _ in 0..p.i("relax")?.max(0) {if inner.len()<2{break;}let cloud=sample(&mesh,&mut rng,(inner.len()*60).max(2000),lo,ext,scale,None);
        let mut sum=vec![[0.;3];inner.len()];let mut cnt=vec![0usize;inner.len()];for pt in cloud{let i=nearest(pt,&inner);sum[i]=add(sum[i],pt);cnt[i]+=1;}
        for i in 0..inner.len(){if cnt[i]>0{inner[i]=mul(sum[i],1./cnt[i] as f64);}}}
    let (tv,_)=triangulation(&geo);let triangles:Vec<_>=tv.iter().map(|t|t.map(|v|pos[geo.topology.vertex_point[v] as usize])).collect();
    let area:Vec<_>=triangles.iter().map(|t|len(cross(sub(t[1],t[0]),sub(t[2],t[0])))*0.5).collect();
    let mut surface=Vec::new();let total:f64=area.iter().sum();if total>0. && ns>0 {
        let picks:(Vec<f64>,Vec<f64>,Vec<f64>)=((0..ns).map(|_|rng.random()).collect(),(0..ns).map(|_|rng.random()).collect(),(0..ns).map(|_|rng.random()).collect());
        for i in 0..ns {let mut target=picks.0[i]*total;let mut tri=triangles.len()-1;for (j,a) in area.iter().enumerate(){target-=a;if target<0.{tri=j;break;}}
            let (mut u,mut v)=(picks.1[i],picks.2[i]);if u+v>1.{u=1.-u;v=1.-v;}let t=triangles[tri];surface.push(add(add(t[0],mul(sub(t[1],t[0]),u)),mul(sub(t[2],t[0]),v)));}}
    let extra=if source=="scatter"{inputs[1].first().map(|g|positions(g)).unwrap_or_default()}else{Vec::new()};
    let mut impacts=Vec::new();let mut impact_ids=Vec::new();if source=="impacts" {
        let other=inputs[1].first().filter(|g|g.npoints>0).ok_or_else(||Error::new("source = impacts needs the impact points on the second input"))?;
        let centres=positions(other);let nimp=(inner.len() as f64*p.f("impact_share")?).round() as usize;
        let weights:Vec<_>=if let Some(a)=other.point.get("impulse"){a.as_f32().ok_or_else(||Error::new("impulse must be float"))?.iter().map(|&x|(x as f64).max(0.)+1e-12).collect()}else{vec![1.;centres.len()]};
        let total:f64=weights.iter().sum();let mut share:Vec<usize>=weights.iter().map(|w|(w/total*nimp as f64).floor() as usize).collect();
        let mut order:Vec<_>=(0..weights.len()).collect();order.sort_by(|&a,&b|weights[b].total_cmp(&weights[a]).then(a.cmp(&b)));
        let remainder=nimp.saturating_sub(share.iter().sum());for &i in order.iter().take(remainder){share[i]+=1;}
        for (i,&c) in centres.iter().enumerate(){let pts=sample(&mesh,&mut rng,share[i],lo,ext,scale,Some((c,p.f("impact_radius")?*scale)));impact_ids.extend(vec![i as i32;pts.len()]);impacts.extend(pts);}
        inner.truncate(inner.len().saturating_sub(nimp));
    }
    let mut all=Vec::new();let mut kinds=Vec::new();for (k,part) in [inner,surface,clustered,extra,impacts].into_iter().enumerate(){kinds.extend(vec![k;part.len()]);all.extend(part);}
    let min=p.f("min_distance")?;let mut keep=vec![true;all.len()];if min>0. {for i in 0..all.len(){for j in i+1..all.len(){if keep[i]&&keep[j]&&len(sub(all[i],all[j]))<=min{keep[j]=false;}}}}
    for (i,&k) in kinds.iter().enumerate(){if k==3{keep[i]=true;}}
    let mut impact_cursor=0;let impact:Vec<i32>=kinds.iter().enumerate().filter_map(|(i,k)|{let id=if *k==4{let v=impact_ids[impact_cursor];impact_cursor+=1;v}else{-1};keep[i].then_some(id)}).collect();
    let selected:Vec<_>=all.into_iter().zip(kinds).zip(keep).filter_map(|((pt,k),ok)|ok.then_some((pt,k))).collect();
    let mut out=Geometry::points_only(selected.iter().flat_map(|(pt,_)|pt.map(|v|v as f32)).collect())?;
    let piece=p.s("piece_attrib")?;if !piece.is_empty(){out.point.insert(piece,Arc::new(Attr::i32(1,(0..out.npoints as i32).collect())));}
    if source=="impacts"{out.point.insert("impact".into(),Arc::new(Attr::i32(1,impact)));}
    let mut cluster=vec![-1;out.npoints];let merge=p.i("merge_clusters")?.max(0) as usize;if merge>0 && out.npoints>0 {
        let k=merge.min(out.npoints);let perm=rng.permutation(out.npoints);let pts:Vec<_>=selected.iter().map(|(pt,_)|pt.map(|v|v as f32 as f64)).collect();let mut seeds:Vec<_>=perm[..k].iter().map(|&i|pts[i]).collect();
        for _ in 0..5{let owner:Vec<_>=pts.iter().map(|&x|nearest(x,&seeds)).collect();for c in 0..k{let members:Vec<_>=pts.iter().zip(&owner).filter(|(_,o)|**o==c).map(|(pt,_)|*pt).collect();if !members.is_empty(){seeds[c]=mul(members.into_iter().fold([0.;3],add),1./owner.iter().filter(|&&o|o==c).count() as f64);}}}
        cluster=pts.iter().map(|&x|nearest(x,&seeds) as i32).collect();
    }out.point.insert("cluster".into(),Arc::new(Attr::i32(1,cluster)));
    for (k,key) in ["interior","surface","clustered","extra","impact"].iter().enumerate(){let mask:Vec<_>=selected.iter().map(|(_,kind)|*kind==k).collect();outgroup(&mut out,p,key,"point",&mask)?;}
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn python_box_seeded_golden() {
        // Python: voronoi_fracture_points(box(), None, 3, 2, 0, 0, 0, 0, 3, .25, "piece").
        let box_p=Params::resolve(super::super::generate::BOX.params,None).unwrap();
        let cube=(super::super::generate::BOX.cook)(&[vec![]],&box_p).unwrap();
        let p=Params::resolve(VORONOI_FRACTURE_POINTS.params,serde_json::json!({"count":3,"seed":2}).as_object()).unwrap();
        let out=voronoi_points(&[vec![Arc::new(cube)],vec![]],&p).unwrap();
        let golden=[-0.31007904,0.09771932,-0.48407987,0.49289390,0.07433863,0.23289038,0.43997028,-0.16314250,-0.21036334];
        assert_eq!(out.npoints,3);
        for (got,want) in out.p().iter().zip(golden){assert!((got-want).abs()<1e-6,"{:?}",out.p());}
        assert_eq!(out.point["cluster"].data,crate::geometry::AttrData::I32(vec![-1;3]));
    }
    #[test]
    fn python_surface_point_order() {
        let box_p=Params::resolve(super::super::generate::BOX.params,None).unwrap();
        let cube=(super::super::generate::BOX.cook)(&[vec![]],&box_p).unwrap();
        let p=Params::resolve(VORONOI_FRACTURE_POINTS.params,serde_json::json!({"count":3,"seed":2,"surface_fraction":1.0}).as_object()).unwrap();
        let out=voronoi_points(&[vec![Arc::new(cube)],vec![]],&p).unwrap();
        let golden=[-0.43997028,-0.43286419,-0.5,-0.5,0.41119614,0.07433863,0.47747296,0.21036334,0.5];
        for (got,want) in out.p().iter().zip(golden){assert!((got-want).abs()<1e-6,"{:?}",out.p());}
    }

    #[test]
    fn test_voronoi_points_empty_and_single_element() {
        // Empty geometry (0 prims) gives clear error
        let empty = Geometry::points_only(vec![]).unwrap();
        let p = Params::resolve(VORONOI_FRACTURE_POINTS.params, None).unwrap();
        assert!(voronoi_points(&[vec![Arc::new(empty)], vec![]], &p).is_err());

        // Box with count: 0 returns empty geometry
        let box_p = Params::resolve(super::super::generate::BOX.params, None).unwrap();
        let cube = (super::super::generate::BOX.cook)(&[vec![]], &box_p).unwrap();
        let p_zero = Params::resolve(VORONOI_FRACTURE_POINTS.params, serde_json::json!({"count": 0}).as_object()).unwrap();
        let out_zero = voronoi_points(&[vec![Arc::new(cube.clone())], vec![]], &p_zero).unwrap();
        assert_eq!(out_zero.npoints, 0);

        // Open mesh with surface_fraction=1.0 works (surface scatter doesn't need closed mesh)
        let tri = Geometry::from_arrays(
            vec![0., 0., 0., 1., 0., 0., 0., 1., 0.],
            vec![0, 1, 2],
            vec![0, 3],
            None,
        ).unwrap();
        let p_surf = Params::resolve(VORONOI_FRACTURE_POINTS.params, serde_json::json!({"count": 5, "surface_fraction": 1.0}).as_object()).unwrap();
        let out_surf = voronoi_points(&[vec![Arc::new(tri)], vec![]], &p_surf).unwrap();
        assert_eq!(out_surf.npoints, 5);
        assert_eq!(out_surf.point["piece"].rows(), 5);
    }

    #[test]
    fn test_voronoi_points_errors_and_limits() {
        let box_p = Params::resolve(super::super::generate::BOX.params, None).unwrap();
        let cube = (super::super::generate::BOX.cook)(&[vec![]], &box_p).unwrap();

        // Invalid source enum
        let p_bad_source = Params::resolve(VORONOI_FRACTURE_POINTS.params, serde_json::json!({"source": "invalid_src"}).as_object()).unwrap();
        assert!(voronoi_points(&[vec![Arc::new(cube.clone())], vec![]], &p_bad_source).is_err());

        // source = impacts without second input points
        let p_impacts = Params::resolve(VORONOI_FRACTURE_POINTS.params, serde_json::json!({"source": "impacts"}).as_object()).unwrap();
        assert!(voronoi_points(&[vec![Arc::new(cube.clone())], vec![]], &p_impacts).is_err());

        // Open mesh with interior scatter (surface_fraction < 1) gives clear error
        let open_grid = Geometry::from_arrays(
            vec![0., 0., 0., 1., 0., 0., 1., 1., 0., 0., 1., 0.],
            vec![0, 1, 2, 3],
            vec![0, 4],
            None,
        ).unwrap();
        let p_int = Params::resolve(VORONOI_FRACTURE_POINTS.params, serde_json::json!({"count": 5, "surface_fraction": 0.0}).as_object()).unwrap();
        let err = voronoi_points(&[vec![Arc::new(open_grid)], vec![]], &p_int).unwrap_err().to_string();
        assert!(err.contains("needs a closed mesh"), "{err}");
    }

    #[test]
    fn test_voronoi_points_determinism_and_clusters() {
        let box_p = Params::resolve(super::super::generate::BOX.params, None).unwrap();
        let cube = (super::super::generate::BOX.cook)(&[vec![]], &box_p).unwrap();
        let p = Params::resolve(VORONOI_FRACTURE_POINTS.params, serde_json::json!({
            "count": 10,
            "seed": 777,
            "surface_fraction": 0.5,
            "merge_clusters": 3,
            "piece_attrib": "piece"
        }).as_object()).unwrap();

        let out1 = voronoi_points(&[vec![Arc::new(cube.clone())], vec![]], &p).unwrap();
        let out2 = voronoi_points(&[vec![Arc::new(cube)], vec![]], &p).unwrap();

        // Deterministic points
        assert_eq!(out1.p(), out2.p());
        assert_eq!(out1.point["piece"].data, out2.point["piece"].data);
        assert_eq!(out1.point["cluster"].data, out2.point["cluster"].data);
    }
}
