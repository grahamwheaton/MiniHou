//! Deterministic attribute tools from nodes/attrib_tools.py.
use std::{collections::BTreeMap,sync::Arc};
use crate::{error::{Error,Result},geometry::{Attr,AttrData,AttrTable,Geometry},params::Params};
pub(super) fn table_mut<'a>(g:&'a mut Geometry,dom:&str)-> &'a mut AttrTable{match dom{"point"=>&mut g.point,"vertex"=>&mut g.vertex,_=>&mut g.prim}}
pub(super) fn domain(s:&str)->&str{match s{"points"=>"point","vertices"=>"vertex","detail"=>"detail",_=>"prim"}}
pub(super) fn size(g:&Geometry,d:&str)->usize{match d{"point"=>g.npoints,"vertex"=>g.nvertices(),_=>g.nprims()}}
pub(super) fn set(g:&Geometry,dom:&str,name:String,arr:Attr)->Geometry{
    let mut out=g.clone();for d in ["point","vertex","prim"]{if d!=dom{table_mut(&mut out,d).remove(&name);}}
    table_mut(&mut out,dom).insert(name,Arc::new(arr));out
}
pub(super) fn set_with_group(g:&Geometry,dom:&str,name:&str,arr:Attr,group:&str)->Result<Geometry>{
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
pub(super) fn laplacian(g:&Geometry,names:&str,p:&Params,factors:&[f64])->Result<Geometry>{
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
        let out=(crate::nodes::get("attribute_promote").unwrap().cook)(&[vec![Arc::new(g)]],&Params::resolve(crate::nodes::get("attribute_promote").unwrap().params,serde_json::json!({"name":"id","to":"primitives"}).as_object()).unwrap()).unwrap();
        assert_eq!(out.prim["id"].data,AttrData::I32(vec![4,8,3]));assert!(!out.point.contains_key("id"));
    }
    #[test]
    fn create_vector4_and_group_zero_fill(){
        let g=geo();let out=(crate::nodes::get("attribute_create").unwrap().cook)(&[vec![Arc::new(g)]],&Params::resolve(crate::nodes::get("attribute_create").unwrap().params,serde_json::json!({"type":"vector4","value":[1,2,3],"group":"1"}).as_object()).unwrap()).unwrap();
        assert_eq!(&out.point["attrib1"].as_f32().unwrap()[4..8],&[1.,2.,3.,1.]);assert_eq!(out.point["attrib1"].as_f32().unwrap()[0],0.);out.validate().unwrap();
    }
    #[test]
    fn blur_pins_curve_ends(){
        let g=Geometry::from_arrays(vec![0.,0.,0.,1.,2.,0.,2.,0.,0.],vec![0,1,2],vec![0,3],Some(vec![false])).unwrap();
        let out=(crate::nodes::get("attribute_blur").unwrap().cook)(&[vec![Arc::new(g)]],&Params::resolve(crate::nodes::get("attribute_blur").unwrap().params,serde_json::json!({"iterations":1,"strength":1.}).as_object()).unwrap()).unwrap();assert_eq!(out.p(),[0.,0.,0.,1.,0.,0.,2.,0.,0.]);
    }
    #[test]
    fn measure_area_and_open_perimeter(){
        let g=Arc::new(Geometry::from_arrays(vec![0.,0.,0.,1.,0.,0.,1.,1.,0.],vec![0,1,2],vec![0,3],Some(vec![false])).unwrap());
        let out=(crate::nodes::get("measure").unwrap().cook)(&[vec![g.clone()]],&Params::resolve(crate::nodes::get("measure").unwrap().params,None).unwrap()).unwrap();assert_eq!(out.prim["area"].as_f32().unwrap(),[0.5]);
        let p=Params::resolve(crate::nodes::get("measure").unwrap().params,serde_json::json!({"measure":"perimeter"}).as_object()).unwrap();let out=(crate::nodes::get("measure").unwrap().cook)(&[vec![g]],&p).unwrap();assert_eq!(out.prim["perimeter"].as_f32().unwrap(),[2.]);out.validate().unwrap();
    }
    #[test]
    fn test_sort_reverse_and_x(){
        let g = Geometry::from_arrays(vec![2.0, 0.0, 0.0, 1.0, 0.0, 0.0, 3.0, 0.0, 0.0], vec![0, 1, 2], vec![0, 3], None).unwrap();
        let p = Params::resolve(crate::nodes::get("sort").unwrap().params, serde_json::json!({"key": "x"}).as_object()).unwrap();
        let out = (crate::nodes::get("sort").unwrap().cook)(&[vec![Arc::new(g.clone())]], &p).unwrap();
        assert_eq!(out.p(), &[1.0, 0.0, 0.0, 2.0, 0.0, 0.0, 3.0, 0.0, 0.0]);

        let p_rev = Params::resolve(crate::nodes::get("sort").unwrap().params, serde_json::json!({"key": "reverse"}).as_object()).unwrap();
        let out_rev = (crate::nodes::get("sort").unwrap().cook)(&[vec![Arc::new(g)]], &p_rev).unwrap();
        assert_eq!(out_rev.p(), &[3.0, 0.0, 0.0, 1.0, 0.0, 0.0, 2.0, 0.0, 0.0]);
    }
    #[test]
    fn test_attribute_randomize_float_and_int(){
        let g = Geometry::points_only(vec![0.0; 12]).unwrap();
        let p = Params::resolve(crate::nodes::get("attribute_randomize").unwrap().params, serde_json::json!({"name": "val", "type": "float", "min": [2.0, 0.0, 0.0], "max": [5.0, 0.0, 0.0], "seed": 42}).as_object()).unwrap();
        let out = (crate::nodes::get("attribute_randomize").unwrap().cook)(&[vec![Arc::new(g.clone())]], &p).unwrap();
        let vals = out.point["val"].as_f32().unwrap();
        assert_eq!(vals.len(), 4);
        for &v in vals {
            assert!(v >= 2.0 && v <= 5.0);
        }

        let p_int = Params::resolve(crate::nodes::get("attribute_randomize").unwrap().params, serde_json::json!({"name": "tag", "type": "int", "min": [1.0, 0.0, 0.0], "max": [3.0, 0.0, 0.0], "seed": 10}).as_object()).unwrap();
        let out_int = (crate::nodes::get("attribute_randomize").unwrap().cook)(&[vec![Arc::new(g)]], &p_int).unwrap();
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

