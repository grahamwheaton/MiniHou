//! Deterministic attribute tools from nodes/attrib_tools.py.
use std::{collections::BTreeMap,sync::Arc};
use crate::{error::{Error,Result},geometry::{Attr,AttrData,AttrTable,Geometry},params::Params};
pub(super) fn table_mut<'a>(g:&'a mut Geometry,dom:&str)-> &'a mut AttrTable{match dom{"point"=>&mut g.point,"vertex"=>&mut g.vertex,"edge"=>&mut g.edge,_=>&mut g.prim}}
pub(super) fn domain(s:&str)->&str{match s{"points"|"point"=>"point","vertices"|"vertex"=>"vertex","edges"|"edge"=>"edge","detail"=>"detail",_=>"prim"}}
pub(super) fn size(g:&Geometry,d:&str)->usize{match d{"point"=>g.npoints,"vertex"=>g.nvertices(),"edge"=>g.nedges(),_=>g.nprims()}}
pub(super) fn set(g:&Geometry,dom:&str,name:String,arr:Attr)->Geometry{
    let mut out=g.clone();for d in ["point","vertex","edge","prim"]{if d!=dom{table_mut(&mut out,d).remove(&name);}}
    table_mut(&mut out,dom).insert(name,Arc::new(arr));out
}
pub(super) fn set_with_group(g:&Geometry,dom:&str,name:&str,arr:Attr,group:&str)->Result<Geometry>{
    super::group::valid_name(name)?;
    let mut out=g.clone();for d in ["point","vertex","edge","prim"]{if d!=dom{table_mut(&mut out,d).remove(name);}}
    let arr=if !group.is_empty() && (dom=="point" || dom=="prim" || dom=="edge"){
        let sel=if dom=="edge" { super::edge_tools::edge_mask(g,group)? } else { super::group::mask(g,dom,group)? };
        let old=g.table(dom).get(name);let n=size(g,dom);let w=arr.width;
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
        let p = Params::resolve(crate::nodes::get("attribute_transfer").unwrap().params, serde_json::json!({"max_distance": 1.0}).as_object()).unwrap();
        let out = (crate::nodes::get("attribute_transfer").unwrap().cook)(&[vec![Arc::new(g)], vec![Arc::new(src)]], &p).unwrap();
        let cd = out.point["Cd"].as_f32().unwrap();
        assert_eq!(&cd[0..3], &[1.0, 0.5, 0.2]);
        assert_eq!(&cd[3..6], &[0.0, 0.0, 0.0]);
    }
    #[test]
    fn test_color_constant_random_ramp_and_pieces(){
        // 1. Constant mode on points
        let g = Geometry::points_only(vec![0.0, 0.0, 0.0, 1.0, 0.0, 0.0]).unwrap();
        let p = Params::resolve(crate::nodes::get("color").unwrap().params, serde_json::json!({"color": [0.2, 0.4, 0.6]}).as_object()).unwrap();
        let out = (crate::nodes::get("color").unwrap().cook)(&[vec![Arc::new(g.clone())]], &p).unwrap();
        let cd = out.point["Cd"].as_f32().unwrap();
        assert_eq!(cd, &[0.2, 0.4, 0.6, 0.2, 0.4, 0.6]);

        // 2. Ramp mode along P.x
        let g_ramp = Geometry::points_only(vec![-2.0, 0.0, 0.0, 2.0, 0.0, 0.0]).unwrap();
        let p_ramp = Params::resolve(crate::nodes::get("color").unwrap().params, serde_json::json!({
            "mode": "ramp",
            "ramp_attrib": "P.x",
            "color": [0.0, 0.0, 0.0],
            "color2": [1.0, 1.0, 1.0]
        }).as_object()).unwrap();
        let out_ramp = (crate::nodes::get("color").unwrap().cook)(&[vec![Arc::new(g_ramp)]], &p_ramp).unwrap();
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
        let p_poly = Params::resolve(crate::nodes::get("color").unwrap().params, serde_json::json!({
            "mode": "random",
            "domain": "primitives",
            "group": "0",
            "seed": 42
        }).as_object()).unwrap();
        let out_poly = (crate::nodes::get("color").unwrap().cook)(&[vec![Arc::new(g_poly)]], &p_poly).unwrap();
        let prim_cd = out_poly.prim["Cd"].as_f32().unwrap();
        assert_ne!(&prim_cd[0..3], &[0.0, 0.0, 0.0]);
        assert_eq!(&prim_cd[3..6], &[0.0, 0.0, 0.0]);

        // 4. Missing attribute error for ramp
        let p_err = Params::resolve(crate::nodes::get("color").unwrap().params, serde_json::json!({
            "mode": "ramp",
            "ramp_attrib": "nope"
        }).as_object()).unwrap();
        assert!((crate::nodes::get("color").unwrap().cook)(&[vec![Arc::new(g.clone())]], &p_err).is_err());
    }
}

