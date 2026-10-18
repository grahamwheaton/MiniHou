//! Deterministic attribute tools from nodes/attrib_tools.py.
use std::{collections::BTreeMap,sync::Arc};
use super::{InputSpec, Inputs, NodeSpec, GEO};
use crate::{error::{Error,Result},geometry::{Attr,AttrData,AttrTable,Geometry,Topology},params::Params};
fn table_mut<'a>(g:&'a mut Geometry,dom:&str)-> &'a mut AttrTable{match dom{"point"=>&mut g.point,"vertex"=>&mut g.vertex,_=>&mut g.prim}}
fn domain(s:&str)->&str{match s{"points"=>"point","vertices"=>"vertex","detail"=>"detail",_=>"prim"}}
fn size(g:&Geometry,d:&str)->usize{match d{"point"=>g.npoints,"vertex"=>g.nvertices(),_=>g.nprims()}}
fn set(g:&Geometry,dom:&str,name:String,arr:Attr)->Geometry{
    let mut out=g.clone();for d in ["point","vertex","prim"]{if d!=dom{table_mut(&mut out,d).remove(&name);}}
    table_mut(&mut out,dom).insert(name,Arc::new(arr));out
}
fn set_with_group(g:&Geometry,dom:&str,name:&str,arr:Attr,group:&str)->Result<Geometry>{
    super::group::valid_name(name)?;
    let mut out=g.clone();for d in ["point","vertex","prim"]{if d!=dom{table_mut(&mut out,d).remove(name);}}
    let arr=if !group.is_empty() && (dom=="point" || dom=="prim"){
        let sel=super::group::mask(g,dom,group)?;let old=g.table(dom).get(name);let n=size(g,dom);let w=arr.width;
        match &arr.data{
            AttrData::F32(new_v)=>{
                let old_v=old.and_then(|a|if a.width==w{match &a.data{AttrData::F32(v)=>Some(v.as_slice()),_=>None}}else{None});
                let mut v=Vec::with_capacity(n*w);
                for i in 0..n{if sel[i]{v.extend_from_slice(&new_v[i*w..(i+1)*w]);}else if let Some(old_v)=old_v{v.extend_from_slice(&old_v[i*w..(i+1)*w]);}else{v.resize(v.len()+w,0.);}}
                Attr::f32(w,v)
            }
            AttrData::I32(new_v)=>{
                let old_v=old.and_then(|a|if a.width==w{match &a.data{AttrData::I32(v)=>Some(v.as_slice()),_=>None}}else{None});
                let mut v=Vec::with_capacity(n*w);
                for i in 0..n{if sel[i]{v.extend_from_slice(&new_v[i*w..(i+1)*w]);}else if let Some(old_v)=old_v{v.extend_from_slice(&old_v[i*w..(i+1)*w]);}else{v.resize(v.len()+w,0);}}
                Attr::i32(w,v)
            }
        }
    }else{arr};
    table_mut(&mut out,dom).insert(name.to_string(),Arc::new(arr));Ok(out)
}
pub const ATTRIBUTE_CREATE:NodeSpec=NodeSpec{name:"attribute_create",inputs:GEO,params:crate::params!("name":"attrib1";"domain":"points";"type":"float";"value":[0.,0.,0.];"group":""),cook:create};
fn create(inputs:&Inputs,p:&Params)->Result<Geometry>{
    let g=ATTRIBUTE_CREATE.one(inputs,0)?;let name=p.s("name")?;super::group::valid_name(&name)?;
    let dom=p.e("domain",&["points","primitives","vertices","detail"])?;let dom=domain(&dom);
    let kind=p.e("type",&["float","int","vector","vector2","color","vector4"])?;let w=match kind.as_str(){"float"|"int"=>1,"vector2"=>2,"vector4"=>4,_=>3};
    let v=p.v3("value")?;let v=[v[0],v[1],v[2],1.];
    if dom=="detail"{let mut out=(*g).clone();out.detail.insert(name,if w==1{serde_json::json!(v[0])}else{serde_json::json!(&v[..w])});return Ok(out);}
    if name=="P" && (dom!="point" || w!=3){return Err(Error::new("P must be a point vector attribute"));}
    let n=size(&g,dom);let sel=if dom=="point" || dom=="prim"{super::group::mask(&g,dom,&p.s("group")?)?}else{vec![true;n]};
    let arr=if kind=="int"{Attr::i32(w,(0..n).flat_map(|i|(0..w).map({let sel=&sel;move|k|if sel[i]{v[k] as i32}else{0}})).collect())}
        else{Attr::f32(w,(0..n).flat_map(|i|(0..w).map({let sel=&sel;move|k|if sel[i]{v[k] as f32}else{0.}})).collect())};
    Ok(set(&g,dom,name,arr))
}
pub(super) fn piece_labels(g:&Geometry)->(Vec<i32>,Vec<i32>){
    fn root(parent:&mut[usize],mut i:usize)->usize{while parent[i]!=i{parent[i]=parent[parent[i]];i=parent[i];}i}
    let mut parent:Vec<_>=(0..g.npoints).collect();
    for r in g.topology.prim_offsets.windows(2){let ids=&g.topology.vertex_point[r[0] as usize..r[1] as usize];
        for w in ids.windows(2){let a=root(&mut parent,w[0] as usize);let b=root(&mut parent,w[1] as usize);parent[a.max(b)]=a.min(b);}
    }
    let mut roots=Vec::new();for i in 0..g.npoints{roots.push(root(&mut parent,i));}
    let mut keys=roots.clone();keys.sort_unstable();keys.dedup();
    let labels:Vec<_>=roots.iter().map(|v|keys.binary_search(v).unwrap() as i32).collect();
    let prim=g.topology.prim_offsets.windows(2).map(|r|if r[0]<r[1]{labels[g.topology.vertex_point[r[0] as usize] as usize]}else{-1}).collect();(labels,prim)
}
pub const CONNECTIVITY:NodeSpec=NodeSpec{name:"connectivity",inputs:GEO,params:crate::params!("domain":"primitives";"name":"class"),cook:connectivity};
fn connectivity(inputs:&Inputs,p:&Params)->Result<Geometry>{let g=CONNECTIVITY.one(inputs,0)?;let dom=p.e("domain",&["primitives","points"])?;let name=p.s("name")?;
    let(a,b)=piece_labels(&g);Ok(set(&g,domain(&dom),name,Attr::i32(1,if dom=="points"{a}else{b})))
}
pub const ATTRIBUTE_PROMOTE:NodeSpec=NodeSpec{name:"attribute_promote",inputs:GEO,params:crate::params!("name":"Cd";"to":"points";"method":"average";"keep_original":false),cook:promote};
fn promote(inputs:&Inputs,p:&Params)->Result<Geometry>{
    let g=ATTRIBUTE_PROMOTE.one(inputs,0)?;let name=p.s("name")?;let dst=p.e("to",&["points","primitives","vertices","detail"])?;let dst=domain(&dst);
    let method=p.e("method",&["average","min","max","sum","first"])?;let keep=p.b("keep_original")?;
    let mut hits=Vec::new();for src in ["point","prim","vertex"]{for(n,a)in g.table(src){if src!=dst && super::attribs::matches(&name,n) && (n!="P" || keep){hits.push((src,n.clone(),a.clone()));}}}
    if hits.is_empty(){return Err(Error::new(format!("no attribute {name:?} on another domain")));}
    let mut pv=Vec::new();for i in 0..g.nprims(){pv.extend(std::iter::repeat_n(i,(g.topology.prim_offsets[i+1]-g.topology.prim_offsets[i]) as usize));}
    let mut out=(*g).clone();
    for(src,n,a)in hits{
        let mut rows=vec![Vec::<usize>::new();if dst=="detail"{1}else{size(&g,dst)}];
        if dst=="detail"{rows[0].extend(0..a.rows());}else{
            for (v,&prim) in pv.iter().enumerate(){let pt=g.topology.vertex_point[v] as usize;let s=match src{"point"=>pt,"prim"=>prim,_=>v};let d=match dst{"point"=>pt,"prim"=>prim,_=>v};rows[d].push(s);}
        }
        let mut result=a.zeros_like(rows.len());
        match (&a.data,&mut result.data){
            (AttrData::F32(v),AttrData::F32(r))=>for(i,ids)in rows.iter().enumerate(){if ids.is_empty(){continue;}for k in 0..a.width{
                r[i*a.width+k]=match method.as_str(){"first"=>v[ids[0]*a.width+k],"min"=>ids.iter().map(|&j|v[j*a.width+k]).fold(f32::INFINITY,f32::min),"max"=>ids.iter().map(|&j|v[j*a.width+k]).fold(f32::NEG_INFINITY,f32::max),"sum"=>ids.iter().fold(0f32,|s,&j|s+v[j*a.width+k]),_=> (ids.iter().map(|&j|v[j*a.width+k] as f64).sum::<f64>()/ids.len() as f64) as f32};
            }},
            (AttrData::I32(v),AttrData::I32(r))=>for(i,ids)in rows.iter().enumerate(){if ids.is_empty(){continue;}for k in 0..a.width{
                r[i*a.width+k]=match method.as_str(){"min"=>ids.iter().map(|&j|v[j*a.width+k]).min().unwrap(),"max"=>ids.iter().map(|&j|v[j*a.width+k]).max().unwrap(),"sum"=>ids.iter().fold(0i32,|s,&j|s.wrapping_add(v[j*a.width+k])),_=>v[ids[0]*a.width+k]};
            }},_=>unreachable!(),
        }
        if !keep{table_mut(&mut out,src).remove(&n);}
        if dst=="detail"{let vals:Vec<_>=match result.data{AttrData::F32(v)=>v.into_iter().map(|v|serde_json::json!(v)).collect(),AttrData::I32(v)=>v.into_iter().map(|v|serde_json::json!(v)).collect()};out.detail.insert(n,if a.width==1{vals[0].clone()}else{serde_json::json!(vals)});}
        else{table_mut(&mut out,dst).insert(n,Arc::new(result));}
    }Ok(out)
}
pub const MEASURE:NodeSpec=NodeSpec{name:"measure",inputs:GEO,params:crate::params!("measure":"area";"name":""),cook:measure};
fn measure(inputs:&Inputs,p:&Params)->Result<Geometry>{
    let g=MEASURE.one(inputs,0)?;let mode=p.e("measure",&["area","perimeter","curvature","piece_area","piece_volume"])?;let name=p.s("name")?;let name=if name.is_empty(){mode.clone()}else{name};
    let face=crate::normals::prim_normals(g.p(),&g.topology);let norm=|v:&[f64;3]|v.iter().map(|x|x*x).sum::<f64>().sqrt();
    if mode=="area"{return Ok(set(&g,"prim",name,Attr::f32(1,face.iter().map(|v|(norm(v)/2.) as f32).collect())));}
    if mode=="perimeter"{let mut values=Vec::new();for i in 0..g.nprims(){let ids=&g.topology.vertex_point[g.topology.prim_offsets[i] as usize..g.topology.prim_offsets[i+1] as usize];let mut length=0.;
        let n=if g.topology.is_closed(i){ids.len()}else{ids.len().saturating_sub(1)};for j in 0..n{let(a,b)=(ids[j] as usize,ids[(j+1)%ids.len()] as usize);length+=(0..3).map(|k|{let d=g.p()[a*3+k] as f64-g.p()[b*3+k] as f64;d*d}).sum::<f64>().sqrt();}values.push(length as f32);
    }return Ok(set(&g,"prim",name,Attr::f32(1,values)));}
    if mode=="curvature"{
        let e=crate::topo::edges(&g.topology,g.npoints);let mut sums=vec![[0.;3];g.npoints];let mut degree=vec![0usize;g.npoints];let mut l2=vec![0.;g.npoints];let mut acc=vec![[0.;3];g.npoints];let mut poly_count=vec![0usize;e.len()];
        // Python sums first all contributions to lower endpoints, then higher.
        for side in 0..2{for pair in e.points.chunks_exact(2){let(a,b)=(pair[side] as usize,pair[1-side] as usize);degree[a]+=1;let mut dist=0.;for k in 0..3{sums[a][k]+=g.p()[b*3+k] as f64;let d=g.p()[a*3+k] as f64-g.p()[b*3+k] as f64;dist+=d*d;}l2[a]+=dist;}}
        for i in 0..g.nprims(){for v in g.topology.prim_offsets[i] as usize..g.topology.prim_offsets[i+1] as usize{let pt=g.topology.vertex_point[v] as usize;for k in 0..3{acc[pt][k]+=face[i][k];}if g.topology.is_closed(i) && e.of_vertex[v]>=0{poly_count[e.of_vertex[v] as usize]+=1;}}}
        let normals=crate::normals::normalize_to_f32(&acc);let mut boundary=vec![false;g.npoints];for(i,&n)in poly_count.iter().enumerate(){if n==1{boundary[e.points[i*2] as usize]=true;boundary[e.points[i*2+1] as usize]=true;}}
        let values=(0..g.npoints).map(|i|{if boundary[i]{0.}else{let deg=degree[i].max(1) as f64;let h=(0..3).map(|k|(g.p()[i*3+k] as f64-sums[i][k]/deg)*normals[i*3+k] as f64).sum::<f64>();(2.*h/(l2[i]/deg).max(1e-24)) as f32}}).collect();
        return Ok(set(&g,"point",name,Attr::f32(1,values)));
    }
    let(_,labels)=piece_labels(&g);if labels.iter().any(|&i|i<0){return Err(Error::new("piece measures require nonempty primitives"));}
    let mut totals=vec![0.;labels.iter().copied().max().map_or(0,|i|i as usize+1)];let centres=super::group::centroids64(&g);
    for(i,&lab)in labels.iter().enumerate(){totals[lab as usize]+=if mode=="piece_area"{norm(&face[i])/2.}else{(0..3).map(|k|face[i][k]*centres[i*3+k]).sum::<f64>()/6.};}
    Ok(set(&g,"prim",name,Attr::f32(1,labels.into_iter().map(|i|totals[i as usize] as f32).collect())))
}

pub const ATTRIBUTE_BLUR:NodeSpec=NodeSpec{name:"attribute_blur",inputs:GEO,params:crate::params!("names":"P";"iterations":5;"strength":0.5;"pin_boundary":true;"group":""),cook:blur};
pub const SMOOTH:NodeSpec=NodeSpec{name:"smooth",inputs:GEO,params:crate::params!("iterations":10;"strength":0.5;"method":"taubin";"pin_boundary":true;"group":""),cook:smooth};
fn laplacian(g:&Geometry,names:&str,p:&Params,factors:&[f64])->Result<Geometry>{
    let e=crate::topo::edges(&g.topology,g.npoints);let mut neighbours=vec![Vec::new();g.npoints];let mut boundary=vec![false;g.npoints];
    // Only polygon edges mark polygon boundary; curve ends are pinned by degree.
    let mut polygon_counts=BTreeMap::<(i32,i32),usize>::new();
    for i in 0..g.nprims(){if g.topology.is_closed(i){let ids=&g.topology.vertex_point[g.topology.prim_offsets[i] as usize..g.topology.prim_offsets[i+1] as usize];for j in 0..ids.len(){let(a,b)=(ids[j],ids[(j+1)%ids.len()]);*polygon_counts.entry((a.min(b),a.max(b))).or_default()+=1;}}}
    for((a,b),n)in polygon_counts{if n==1{boundary[a as usize]=true;boundary[b as usize]=true;}}
    for pair in e.points.chunks_exact(2){let(a,b)=(pair[0] as usize,pair[1] as usize);neighbours[a].push(b);neighbours[b].push(a);}
    let mut sel=super::group::mask(g,"point",&p.s("group")?)?;let pin=p.b("pin_boundary")?;
    for i in 0..g.npoints{sel[i]&=!neighbours[i].is_empty() && !(pin && (boundary[i] || neighbours[i].len()==1));}
    let mut out=g.clone();for name in names.split(|c:char|c==',' || c.is_whitespace()).filter(|s|!s.is_empty()){
        let a=g.point.get(name).ok_or_else(||Error::new(format!("no point attribute {name:?}")))?;let src=a.as_f32().ok_or_else(||Error::new("only float attributes can be blurred"))?;
        let mut v:Vec<f64>=src.iter().map(|&x|x as f64).collect();
        for &f in factors{let mut next=v.clone();for i in 0..g.npoints{if sel[i]{for k in 0..a.width{let avg=neighbours[i].iter().map(|&j|v[j*a.width+k]).sum::<f64>()/neighbours[i].len() as f64;next[i*a.width+k]=v[i*a.width+k]+f*(avg-v[i*a.width+k]);}}}v=next;}
        out.point.insert(name.into(),Arc::new(Attr::f32(a.width,v.into_iter().map(|x|x as f32).collect())));
    }Ok(out)
}
fn blur(inputs:&Inputs,p:&Params)->Result<Geometry>{let g=ATTRIBUTE_BLUR.one(inputs,0)?;let n=p.i("iterations")?;if n<0{return Err(Error::new("iterations must be nonnegative"));}laplacian(&g,&p.s("names")?,p,&vec![p.f("strength")?;n as usize])}
fn smooth(inputs:&Inputs,p:&Params)->Result<Geometry>{let g=SMOOTH.one(inputs,0)?;let n=p.i("iterations")?;if n<0{return Err(Error::new("iterations must be nonnegative"));}let f=p.f("strength")?;
    let taubin=p.e("method",&["taubin","laplacian"])?=="taubin";let mut factors=Vec::new();for _ in 0..n{factors.push(f);if taubin{factors.push(1./(0.1-1./f.max(1e-6)));}}
    let mut out=laplacian(&g,"P",p,&factors)?;
    if out.point.contains_key("N") && out.nprims()>0{let spec=super::get("normal").unwrap();out=(spec.cook)(&[vec![Arc::new(out)]],&Params::resolve(spec.params,None)?)?;}Ok(out)
}

pub const SORT: NodeSpec = NodeSpec {
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
    let g = SORT.one(inputs, 0)?;
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
            let mut rng = super::np_random::Rng::new(&[(seed as u64) & 0xFFFFFFFF]);
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
                let centres = super::group::centroids64(&g);
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
        let mut out = Geometry { packed: Default::default(),
            npoints: n,
            topology: Arc::new(topo),
            point: super::group::gather_table(&g.point, &order),
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
        let out = Geometry { packed: Default::default(),
            npoints: g.npoints,
            topology: Arc::new(topo),
            point: g.point.clone(),
            vertex: super::group::gather_table(&g.vertex, &verts),
            prim: super::group::gather_table(&g.prim, &order),
            detail: g.detail.clone(),
        };
        Ok(out)
    }
}

pub const ATTRIBUTE_RANDOMIZE: NodeSpec = NodeSpec {
    name: "attribute_randomize",
    inputs: GEO,
    params: crate::params!(
        "name": "Cd";
        "domain": "points";
        "type": "color";
        "min": [0.0, 0.0, 0.0];
        "max": [1.0, 1.0, 1.0];
        "distribution": "uniform";
        "seed": 0;
        "by": "";
        "group": "";
    ),
    cook: randomize,
};

fn randomize(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let g = ATTRIBUTE_RANDOMIZE.one(inputs, 0)?;
    let name = p.s("name")?;
    super::group::valid_name(&name)?;
    let dom = p.e("domain", &["points", "primitives", "vertices"])?;
    let dom = domain(&dom);
    let kind = p.e("type", &["color", "float", "vector", "int"])?;
    let width = if kind == "float" || kind == "int" { 1 } else { 3 };
    let min_val = p.v3("min")?;
    let max_val = p.v3("max")?;
    let dist = p.e("distribution", &["uniform", "normal"])?;
    let is_normal = dist == "normal";
    let seed = p.i("seed")?;
    let by = p.s("by")?;
    let group = p.s("group")?;
    let n = size(&g, dom);

    let (r, inv) = if !by.is_empty() {
        let src = g.table(dom).get(&by).ok_or_else(|| Error::new(format!("no {dom} attribute {by:?} to randomize by")))?;
        let keys: Vec<i64> = match &src.data {
            AttrData::I32(v) => (0..n).map(|i| v[i * src.width] as i64).collect(),
            AttrData::F32(v) => (0..n).map(|i| v[i * src.width] as i64).collect(),
        };
        let mut uniq = keys.clone();
        uniq.sort_unstable();
        uniq.dedup();
        let inv: Vec<usize> = keys.iter().map(|k| uniq.binary_search(k).unwrap()).collect();
        let mut rng = super::np_random::Rng::new(&[(seed as u64) & 0xFFFFFFFF, 7]);
        let mut r = Vec::with_capacity(uniq.len() * width);
        for _ in 0..uniq.len() {
            for _ in 0..width {
                let val = if is_normal {
                    (0.5 + rng.standard_normal() / 6.0).clamp(0.0, 1.0)
                } else {
                    rng.random()
                };
                r.push(val);
            }
        }
        (r, Some(inv))
    } else {
        let mut rng = super::np_random::Rng::new(&[(seed as u64) & 0xFFFFFFFF]);
        let mut r = Vec::with_capacity(n * width);
        for _ in 0..n {
            for _ in 0..width {
                let val = if is_normal {
                    (0.5 + rng.standard_normal() / 6.0).clamp(0.0, 1.0)
                } else {
                    rng.random()
                };
                r.push(val);
            }
        }
        (r, None)
    };

    let arr = if kind == "int" {
        let lo = min_val[0];
        let hi = max_val[0];
        let mut data = Vec::with_capacity(n);
        for i in 0..n {
            let r_idx = inv.as_ref().map_or(i, |inv| inv[i]);
            let r_val = r[r_idx];
            let val = (lo + r_val * (hi - lo + 1.0)).floor().clamp(lo, hi) as i32;
            data.push(val);
        }
        Attr::i32(1, data)
    } else {
        let lo = &min_val[..width];
        let hi = &max_val[..width];
        let mut data = Vec::with_capacity(n * width);
        for i in 0..n {
            let r_idx = inv.as_ref().map_or(i, |inv| inv[i]);
            for k in 0..width {
                let r_val = r[r_idx * width + k];
                let val = (lo[k] + r_val * (hi[k] - lo[k])) as f32;
                data.push(val);
            }
        }
        Attr::f32(width, data)
    };

    set_with_group(&g, dom, &name, arr, &group)
}

struct KdNode {
    axis: usize,
    split: f64,
    idx: usize,
    left: Option<usize>,
    right: Option<usize>,
}

struct KdTree {
    pts: Vec<[f64; 3]>,
    nodes: Vec<KdNode>,
}

impl KdTree {
    fn build(pts: Vec<[f64; 3]>) -> Self {
        let mut indices: Vec<usize> = (0..pts.len()).collect();
        let mut nodes = Vec::with_capacity(pts.len());
        let _ = Self::build_rec(&pts, &mut indices, 0, &mut nodes);
        KdTree { pts, nodes }
    }

    fn build_rec(
        pts: &[[f64; 3]],
        indices: &mut [usize],
        depth: usize,
        nodes: &mut Vec<KdNode>,
    ) -> Option<usize> {
        if indices.is_empty() {
            return None;
        }
        let axis = depth % 3;
        let mid = indices.len() / 2;
        indices.select_nth_unstable_by(mid, |&a, &b| {
            pts[a][axis].total_cmp(&pts[b][axis])
        });
        let idx = indices[mid];
        let split = pts[idx][axis];

        let node_idx = nodes.len();
        nodes.push(KdNode {
            axis,
            split,
            idx,
            left: None,
            right: None,
        });

        let (left_slice, rest) = indices.split_at_mut(mid);
        let right_slice = &mut rest[1..];

        let left = Self::build_rec(pts, left_slice, depth + 1, nodes);
        let right = Self::build_rec(pts, right_slice, depth + 1, nodes);

        nodes[node_idx].left = left;
        nodes[node_idx].right = right;

        Some(node_idx)
    }

    fn nearest(&self, q: [f64; 3]) -> (usize, f64) {
        if self.nodes.is_empty() {
            return (0, f64::INFINITY);
        }
        let mut best_idx = 0;
        let mut best_dist_sq = f64::INFINITY;
        self.search(0, q, &mut best_idx, &mut best_dist_sq);
        (best_idx, best_dist_sq.sqrt())
    }

    fn search(&self, node_idx: usize, q: [f64; 3], best_idx: &mut usize, best_dist_sq: &mut f64) {
        let node = &self.nodes[node_idx];
        let p = self.pts[node.idx];
        let d2 = (q[0] - p[0]).powi(2) + (q[1] - p[1]).powi(2) + (q[2] - p[2]).powi(2);
        if d2 < *best_dist_sq {
            *best_dist_sq = d2;
            *best_idx = node.idx;
        }

        let diff = q[node.axis] - node.split;
        let (first, second) = if diff <= 0.0 {
            (node.left, node.right)
        } else {
            (node.right, node.left)
        };

        if let Some(child) = first {
            self.search(child, q, best_idx, best_dist_sq);
        }
        if diff * diff < *best_dist_sq {
            if let Some(child) = second {
                self.search(child, q, best_idx, best_dist_sq);
            }
        }
    }
}

pub const ATTRIBUTE_TRANSFER: NodeSpec = NodeSpec {
    name: "attribute_transfer",
    inputs: &[
        InputSpec { name: "geo", multi: false, optional: false },
        InputSpec { name: "source", multi: false, optional: false },
    ],
    params: crate::params!(
        "names": "Cd";
        "max_distance": 1e9;
        "blend_width": 0.0;
        "group": "";
    ),
    cook: transfer,
};

fn transfer(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let geo = ATTRIBUTE_TRANSFER.one(inputs, 0)?;
    let source = ATTRIBUTE_TRANSFER.one(inputs, 1)?;
    let names_str = p.s("names")?;
    let pats: Vec<&str> = names_str.split(|c: char| c == ',' || c.is_whitespace()).filter(|s| !s.is_empty()).collect();
    let mut want: Vec<String> = Vec::new();
    for (name, _) in &source.point {
        if name != "P" && pats.iter().any(|pat| super::attribs::matches(pat, name)) {
            want.push(name.clone());
        }
    }
    if want.is_empty() {
        return Err(Error::new(format!("the source has no point attribute matching {names_str:?}")));
    }
    let max_dist = p.f("max_distance")?;
    let blend_width = p.f("blend_width")?;
    let group = p.s("group")?;
    let sel = super::group::mask(&geo, "point", &group)?;

    let src_p = source.p();
    let src_pts: Vec<[f64; 3]> = (0..source.npoints)
        .map(|i| [src_p[i * 3] as f64, src_p[i * 3 + 1] as f64, src_p[i * 3 + 2] as f64])
        .collect();
    let tree = KdTree::build(src_pts);

    let geo_p = geo.p();
    let n = geo.npoints;
    let mut nearest_idx = Vec::with_capacity(n);
    let mut weights = Vec::with_capacity(n);

    for i in 0..n {
        let q = [geo_p[i * 3] as f64, geo_p[i * 3 + 1] as f64, geo_p[i * 3 + 2] as f64];
        let (idx, dist) = tree.nearest(q);
        nearest_idx.push(idx);

        let mut w = if dist <= max_dist { 1.0 } else { 0.0 };
        if blend_width > 0.0 {
            let fade = 1.0 - (dist - (max_dist - blend_width)) / blend_width;
            w = fade.clamp(0.0, 1.0);
        }
        if !sel[i] {
            w = 0.0;
        }
        weights.push(w);
    }

    let mut out = (*geo).clone();
    for nm in want {
        let src_attr = &source.point[&nm];
        let w = src_attr.width;
        let own_attr = geo.point.get(&nm);
        let own_matches = own_attr.map_or(false, |a| a.width == w && a.dtype() == src_attr.dtype());

        let res_attr = match &src_attr.data {
            AttrData::F32(src_v) => {
                let own_v = if own_matches {
                    own_attr.unwrap().as_f32().unwrap()
                } else {
                    &[]
                };
                let mut data = Vec::with_capacity(n * w);
                for i in 0..n {
                    let s_idx = nearest_idx[i];
                    let weight = weights[i] as f32;
                    for k in 0..w {
                        let s_val = src_v[s_idx * w + k];
                        let o_val = if own_matches { own_v[i * w + k] } else { 0.0 };
                        data.push(o_val * (1.0 - weight) + s_val * weight);
                    }
                }
                Attr::f32(w, data)
            }
            AttrData::I32(src_v) => {
                let own_v = if own_matches {
                    match &own_attr.unwrap().data {
                        AttrData::I32(v) => v.as_slice(),
                        _ => &[],
                    }
                } else {
                    &[]
                };
                let mut data = Vec::with_capacity(n * w);
                for i in 0..n {
                    let s_idx = nearest_idx[i];
                    let use_src = weights[i] >= 0.5;
                    for k in 0..w {
                        let s_val = src_v[s_idx * w + k];
                        let o_val = if own_matches { own_v[i * w + k] } else { 0 };
                        data.push(if use_src { s_val } else { o_val });
                    }
                }
                Attr::i32(w, data)
            }
        };

        out.prim.remove(&nm);
        out.vertex.remove(&nm);
        out.point.insert(nm, Arc::new(res_attr));
    }
    Ok(out)
}

pub const COLOR: NodeSpec = NodeSpec {
    name: "color",
    inputs: GEO,
    params: crate::params!(
        "mode": "constant";
        "domain": "points";
        "color": [1.0, 0.5, 0.2];
        "color2": [0.1, 0.3, 1.0];
        "ramp_attrib": "P.z";
        "seed": 0;
        "group": ""
    ),
    cook: color_node,
};

fn read_domain_attr(g: &Geometry, dom: &str, name: &str) -> Option<(Vec<f64>, usize)> {
    if let Some(attr) = g.table(dom).get(name) {
        let width = attr.width;
        let vals: Vec<f64> = match &attr.data {
            AttrData::F32(v) => v.iter().map(|&x| x as f64).collect(),
            AttrData::I32(v) => v.iter().map(|&x| x as f64).collect(),
        };
        return Some((vals, width));
    }
    if dom == "vertex" {
        if let Some(attr) = g.point.get(name) {
            let width = attr.width;
            let vals: Vec<f64> = match &attr.data {
                AttrData::F32(v) => v.iter().map(|&x| x as f64).collect(),
                AttrData::I32(v) => v.iter().map(|&x| x as f64).collect(),
            };
            let mut out = Vec::with_capacity(g.nvertices() * width);
            for &pt in &g.topology.vertex_point {
                let off = (pt as usize) * width;
                out.extend_from_slice(&vals[off..off + width]);
            }
            return Some((out, width));
        }
        if let Some(attr) = g.prim.get(name) {
            let width = attr.width;
            let vals: Vec<f64> = match &attr.data {
                AttrData::F32(v) => v.iter().map(|&x| x as f64).collect(),
                AttrData::I32(v) => v.iter().map(|&x| x as f64).collect(),
            };
            let mut out = Vec::with_capacity(g.nvertices() * width);
            for prim_idx in 0..g.nprims() {
                let count = (g.topology.prim_offsets[prim_idx + 1] - g.topology.prim_offsets[prim_idx]) as usize;
                let off = prim_idx * width;
                let prim_slice = &vals[off..off + width];
                for _ in 0..count {
                    out.extend_from_slice(prim_slice);
                }
            }
            return Some((out, width));
        }
    } else if dom == "prim" {
        if let Some(attr) = g.point.get(name) {
            let width = attr.width;
            let vals: Vec<f64> = match &attr.data {
                AttrData::F32(v) => v.iter().map(|&x| x as f64).collect(),
                AttrData::I32(v) => v.iter().map(|&x| x as f64).collect(),
            };
            let mut out = Vec::with_capacity(g.nprims() * width);
            for prim_idx in 0..g.nprims() {
                let start = g.topology.prim_offsets[prim_idx] as usize;
                let end = g.topology.prim_offsets[prim_idx + 1] as usize;
                let count = end - start;
                if count == 0 {
                    out.resize(out.len() + width, 0.0);
                } else {
                    for c in 0..width {
                        let sum: f64 = (start..end)
                            .map(|v| {
                                let pt = g.topology.vertex_point[v] as usize;
                                vals[pt * width + c]
                            })
                            .sum();
                        out.push(sum / count as f64);
                    }
                }
            }
            return Some((out, width));
        }
        if let Some(attr) = g.vertex.get(name) {
            let width = attr.width;
            let vals: Vec<f64> = match &attr.data {
                AttrData::F32(v) => v.iter().map(|&x| x as f64).collect(),
                AttrData::I32(v) => v.iter().map(|&x| x as f64).collect(),
            };
            let mut out = Vec::with_capacity(g.nprims() * width);
            for prim_idx in 0..g.nprims() {
                let start = g.topology.prim_offsets[prim_idx] as usize;
                let end = g.topology.prim_offsets[prim_idx + 1] as usize;
                let count = end - start;
                if count == 0 {
                    out.resize(out.len() + width, 0.0);
                } else {
                    for c in 0..width {
                        let sum: f64 = (start..end)
                            .map(|v| vals[v * width + c])
                            .sum();
                        out.push(sum / count as f64);
                    }
                }
            }
            return Some((out, width));
        }
    }
    if let Some(val) = g.detail.get(name) {
        let n = size(g, dom);
        if let Some(num) = val.as_f64() {
            return Some((vec![num; n], 1));
        }
        if let Some(arr) = val.as_array() {
            if arr.len() <= 4 && arr.iter().all(|x| x.is_number()) {
                let width = arr.len();
                let nums: Vec<f64> = arr.iter().map(|x| x.as_f64().unwrap()).collect();
                let mut out = Vec::with_capacity(n * width);
                for _ in 0..n {
                    out.extend_from_slice(&nums);
                }
                return Some((out, width));
            }
        }
    }
    None
}

fn color_node(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let g = COLOR.one(inputs, 0)?;
    let mode = p.e("mode", &["constant", "random", "random_by_piece", "ramp"])?;
    let dom = p.e("domain", &["points", "primitives", "vertices"])?;
    let dom = domain(&dom);
    let color = p.v3("color")?;
    let color2 = p.v3("color2")?;
    let c1: [f32; 3] = [color[0] as f32, color[1] as f32, color[2] as f32];
    let ramp_attrib = p.s("ramp_attrib")?;
    let seed = p.i("seed")?;
    let group = p.s("group")?;
    let n = size(&g, dom);

    let cd = match mode.as_str() {
        "constant" => {
            let mut cd = Vec::with_capacity(n * 3);
            for _ in 0..n {
                cd.extend_from_slice(&c1);
            }
            cd
        }
        "random" => {
            let mut rng = super::np_random::Rng::new(&[(seed as u64) & 0xFFFFFFFF]);
            let mut cd = Vec::with_capacity(n * 3);
            for _ in 0..n {
                for _ in 0..3 {
                    cd.push(rng.random() as f32);
                }
            }
            cd
        }
        "random_by_piece" => {
            let mut rng = super::np_random::Rng::new(&[(seed as u64) & 0xFFFFFFFF]);
            let (pts, prim) = piece_labels(&g);
            let lab: Vec<i32> = match dom {
                "point" => pts,
                "prim" => prim,
                "vertex" => {
                    let mut pv = Vec::with_capacity(g.nvertices());
                    for i in 0..g.nprims() {
                        let count = (g.topology.prim_offsets[i + 1] - g.topology.prim_offsets[i]) as usize;
                        pv.resize(pv.len() + count, prim[i]);
                    }
                    pv
                }
                _ => unreachable!(),
            };
            let max_lab = lab.iter().copied().max();
            let pal_count = match max_lab {
                Some(m) if m >= 0 => (m + 1) as usize,
                _ => 0,
            };
            let mut pal = Vec::with_capacity(pal_count * 3);
            for _ in 0..pal_count {
                for _ in 0..3 {
                    pal.push(rng.random() as f32);
                }
            }
            let mut cd = Vec::with_capacity(lab.len() * 3);
            for &l in &lab {
                if l >= 0 && (l as usize) < pal_count {
                    let off = (l as usize) * 3;
                    cd.extend_from_slice(&pal[off..off + 3]);
                } else if l < 0 && pal_count > 0 {
                    let wrapped = ((l % (pal_count as i32)) + (pal_count as i32)) % (pal_count as i32);
                    let off = (wrapped as usize) * 3;
                    cd.extend_from_slice(&pal[off..off + 3]);
                } else {
                    cd.extend_from_slice(&[0.0, 0.0, 0.0]);
                }
            }
            cd
        }
        "ramp" => {
            let (nm, comp) = match ramp_attrib.split_once('.') {
                Some((nm, comp)) => (nm, comp),
                None => (ramp_attrib.as_str(), ""),
            };
            let (a, width) = read_domain_attr(&g, dom, nm)
                .ok_or_else(|| Error::new(format!("no attribute {nm:?} to ramp by")))?;
            let v: Vec<f64> = if !comp.is_empty() {
                let comp_idx = match comp {
                    "x" => 0,
                    "y" => 1,
                    "z" => 2,
                    "w" => 3,
                    _ => return Err(Error::new(format!("unknown component {comp:?} in {ramp_attrib:?}"))),
                };
                if comp_idx >= width {
                    return Err(Error::new(format!("component index {comp_idx} out of range for attribute {nm:?} with width {width}")));
                }
                (0..n).map(|i| a[i * width + comp_idx]).collect()
            } else if width == 1 {
                a
            } else {
                (0..n).map(|i| {
                    let row = &a[i * width..(i + 1) * width];
                    row.iter().map(|&x| x * x).sum::<f64>().sqrt()
                }).collect()
            };
            if v.is_empty() {
                Vec::new()
            } else {
                let mut v_min = f64::INFINITY;
                let mut v_max = f64::NEG_INFINITY;
                for &x in &v {
                    if x < v_min { v_min = x; }
                    if x > v_max { v_max = x; }
                }
                let span = (v_max - v_min).max(1e-12);
                let mut cd = Vec::with_capacity(n * 3);
                for x in v {
                    let t = (x - v_min) / span;
                    for c in 0..3 {
                        let val = (color[c] as f64) * (1.0 - t) + (color2[c] as f64) * t;
                        cd.push(val as f32);
                    }
                }
                cd
            }
        }
        _ => unreachable!(),
    };

    set_with_group(&g, dom, "Cd", Attr::f32(3, cd), &group)
}

#[cfg(test)]
mod tests{
    use super::*;
    fn geo()->Geometry{Geometry::from_arrays(vec![0.;15],vec![0,1,1,2,3,4],vec![0,2,4,6],Some(vec![false;3])).unwrap()}
    #[test]
    fn labels_and_integer_average_uses_first(){
        let mut g=geo();let(a,b)=piece_labels(&g);assert_eq!(a,[0,0,0,1,1]);assert_eq!(b,[0,0,1]);
        g.point.insert("id".into(),Arc::new(Attr::i32(1,vec![4,8,9,3,2])));
        let out=promote(&[vec![Arc::new(g)]],&Params::resolve(ATTRIBUTE_PROMOTE.params,serde_json::json!({"name":"id","to":"primitives"}).as_object()).unwrap()).unwrap();
        assert_eq!(out.prim["id"].data,AttrData::I32(vec![4,8,3]));assert!(!out.point.contains_key("id"));
    }
    #[test]
    fn create_vector4_and_group_zero_fill(){
        let g=geo();let out=create(&[vec![Arc::new(g)]],&Params::resolve(ATTRIBUTE_CREATE.params,serde_json::json!({"type":"vector4","value":[1,2,3],"group":"1"}).as_object()).unwrap()).unwrap();
        assert_eq!(&out.point["attrib1"].as_f32().unwrap()[4..8],&[1.,2.,3.,1.]);assert_eq!(out.point["attrib1"].as_f32().unwrap()[0],0.);out.validate().unwrap();
    }
    #[test]
    fn blur_pins_curve_ends(){
        let g=Geometry::from_arrays(vec![0.,0.,0.,1.,2.,0.,2.,0.,0.],vec![0,1,2],vec![0,3],Some(vec![false])).unwrap();
        let out=blur(&[vec![Arc::new(g)]],&Params::resolve(ATTRIBUTE_BLUR.params,serde_json::json!({"iterations":1,"strength":1.}).as_object()).unwrap()).unwrap();assert_eq!(out.p(),[0.,0.,0.,1.,0.,0.,2.,0.,0.]);
    }
    #[test]
    fn measure_area_and_open_perimeter(){
        let g=Arc::new(Geometry::from_arrays(vec![0.,0.,0.,1.,0.,0.,1.,1.,0.],vec![0,1,2],vec![0,3],Some(vec![false])).unwrap());
        let out=measure(&[vec![g.clone()]],&Params::resolve(MEASURE.params,None).unwrap()).unwrap();assert_eq!(out.prim["area"].as_f32().unwrap(),[0.5]);
        let p=Params::resolve(MEASURE.params,serde_json::json!({"measure":"perimeter"}).as_object()).unwrap();let out=measure(&[vec![g]],&p).unwrap();assert_eq!(out.prim["perimeter"].as_f32().unwrap(),[2.]);out.validate().unwrap();
    }
    #[test]
    fn test_sort_reverse_and_x(){
        let g = Geometry::from_arrays(vec![2.0, 0.0, 0.0, 1.0, 0.0, 0.0, 3.0, 0.0, 0.0], vec![0, 1, 2], vec![0, 3], None).unwrap();
        let p = Params::resolve(SORT.params, serde_json::json!({"key": "x"}).as_object()).unwrap();
        let out = sort(&[vec![Arc::new(g.clone())]], &p).unwrap();
        assert_eq!(out.p(), &[1.0, 0.0, 0.0, 2.0, 0.0, 0.0, 3.0, 0.0, 0.0]);

        let p_rev = Params::resolve(SORT.params, serde_json::json!({"key": "reverse"}).as_object()).unwrap();
        let out_rev = sort(&[vec![Arc::new(g)]], &p_rev).unwrap();
        assert_eq!(out_rev.p(), &[3.0, 0.0, 0.0, 1.0, 0.0, 0.0, 2.0, 0.0, 0.0]);
    }
    #[test]
    fn test_attribute_randomize_float_and_int(){
        let g = Geometry::points_only(vec![0.0; 12]).unwrap();
        let p = Params::resolve(ATTRIBUTE_RANDOMIZE.params, serde_json::json!({"name": "val", "type": "float", "min": [2.0, 0.0, 0.0], "max": [5.0, 0.0, 0.0], "seed": 42}).as_object()).unwrap();
        let out = randomize(&[vec![Arc::new(g.clone())]], &p).unwrap();
        let vals = out.point["val"].as_f32().unwrap();
        assert_eq!(vals.len(), 4);
        for &v in vals {
            assert!(v >= 2.0 && v <= 5.0);
        }

        let p_int = Params::resolve(ATTRIBUTE_RANDOMIZE.params, serde_json::json!({"name": "tag", "type": "int", "min": [1.0, 0.0, 0.0], "max": [3.0, 0.0, 0.0], "seed": 10}).as_object()).unwrap();
        let out_int = randomize(&[vec![Arc::new(g)]], &p_int).unwrap();
        match &out_int.point["tag"].data {
            AttrData::I32(vals) => {
                for &v in vals {
                    assert!(v >= 1 && v <= 3);
                }
            }
            _ => panic!("expected i32"),
        }
    }
    #[test]
    fn test_attribute_transfer_nearest(){
        let g = Geometry::points_only(vec![0.0, 0.0, 0.0, 5.0, 0.0, 0.0]).unwrap();
        let mut src = Geometry::points_only(vec![0.1, 0.0, 0.0]).unwrap();
        src.point.insert("Cd".into(), Arc::new(Attr::f32(3, vec![1.0, 0.5, 0.2])));
        let p = Params::resolve(ATTRIBUTE_TRANSFER.params, serde_json::json!({"max_distance": 1.0}).as_object()).unwrap();
        let out = transfer(&[vec![Arc::new(g)], vec![Arc::new(src)]], &p).unwrap();
        let cd = out.point["Cd"].as_f32().unwrap();
        assert_eq!(&cd[0..3], &[1.0, 0.5, 0.2]);
        assert_eq!(&cd[3..6], &[0.0, 0.0, 0.0]);
    }
    #[test]
    fn test_color_constant_random_ramp_and_pieces(){
        // 1. Constant mode on points
        let g = Geometry::points_only(vec![0.0, 0.0, 0.0, 1.0, 0.0, 0.0]).unwrap();
        let p = Params::resolve(COLOR.params, serde_json::json!({"color": [0.2, 0.4, 0.6]}).as_object()).unwrap();
        let out = color_node(&[vec![Arc::new(g.clone())]], &p).unwrap();
        let cd = out.point["Cd"].as_f32().unwrap();
        assert_eq!(cd, &[0.2, 0.4, 0.6, 0.2, 0.4, 0.6]);

        // 2. Ramp mode along P.x
        let g_ramp = Geometry::points_only(vec![-2.0, 0.0, 0.0, 2.0, 0.0, 0.0]).unwrap();
        let p_ramp = Params::resolve(COLOR.params, serde_json::json!({
            "mode": "ramp",
            "ramp_attrib": "P.x",
            "color": [0.0, 0.0, 0.0],
            "color2": [1.0, 1.0, 1.0]
        }).as_object()).unwrap();
        let out_ramp = color_node(&[vec![Arc::new(g_ramp)]], &p_ramp).unwrap();
        let cd_ramp = out_ramp.point["Cd"].as_f32().unwrap();
        assert_eq!(&cd_ramp[0..3], &[0.0, 0.0, 0.0]);
        assert_eq!(&cd_ramp[3..6], &[1.0, 1.0, 1.0]);

        // 3. Random mode with group restriction on primitives
        let g_poly = Geometry::from_arrays(
            vec![0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 1.0, 0.0, 0.0, 1.0, 0.0,
                 2.0, 0.0, 0.0, 3.0, 0.0, 0.0, 3.0, 1.0, 0.0, 2.0, 1.0, 0.0],
            vec![0, 1, 2, 3, 4, 5, 6, 7],
            vec![0, 4, 8],
            None,
        ).unwrap();
        let p_poly = Params::resolve(COLOR.params, serde_json::json!({
            "mode": "random",
            "domain": "primitives",
            "group": "0",
            "seed": 42
        }).as_object()).unwrap();
        let out_poly = color_node(&[vec![Arc::new(g_poly)]], &p_poly).unwrap();
        let prim_cd = out_poly.prim["Cd"].as_f32().unwrap();
        assert_ne!(&prim_cd[0..3], &[0.0, 0.0, 0.0]);
        assert_eq!(&prim_cd[3..6], &[0.0, 0.0, 0.0]);

        // 4. Missing attribute error for ramp
        let p_err = Params::resolve(COLOR.params, serde_json::json!({
            "mode": "ramp",
            "ramp_attrib": "nope"
        }).as_object()).unwrap();
        assert!(color_node(&[vec![Arc::new(g.clone())]], &p_err).is_err());
    }
}

