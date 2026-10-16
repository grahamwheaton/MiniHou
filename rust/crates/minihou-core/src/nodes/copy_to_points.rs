//! Realised instancing with Houdini transform precedence (nodes/copy_to_points.py).
use std::sync::Arc;
use super::{InputSpec, Inputs, NodeSpec};
use crate::{error::{Error,Result}, geometry::{Attr,AttrData,Geometry,MAX_ELEMENTS}, math::{self,M3,IDENTITY}, params::Params};
const TRANSFORM:&[&str]=&["P","N","pscale","orient","up","rot","scale"];
/// Same explicitly user-owned file and validation as user_preferences.py.
/// Graph parameters cannot alter the allocation limit.
pub(crate) fn copy_limit_bytes()->u64{
    let path=std::env::var_os("MINIHOU_PREFERENCES_FILE").filter(|v|!v.is_empty()).map(std::path::PathBuf::from).or_else(||{
        std::env::var_os("LOCALAPPDATA").map(std::path::PathBuf::from).or_else(||
            std::env::var_os("USERPROFILE").or_else(||std::env::var_os("HOME")).map(|p|std::path::PathBuf::from(p).join(".config")))
            .map(|p|p.join("MiniHou").join("preferences.json"))
    });
    let value=path.and_then(|p|std::fs::read_to_string(p).ok()).and_then(|s|serde_json::from_str::<serde_json::Value>(&s).ok());
    limit_from_preferences(value.as_ref())
}
fn limit_from_preferences(v:Option<&serde_json::Value>)->u64{
    v.and_then(|v|v.get("copy_memory_limit_mib")).and_then(|v|v.as_u64()).filter(|n|(16..=16384).contains(n)).unwrap_or(256)*1024*1024
}
pub const COPY_TO_POINTS:NodeSpec=NodeSpec{name:"copy_to_points",inputs:&[
    InputSpec{name:"source",multi:false,optional:false},InputSpec{name:"points",multi:false,optional:false}],
    params:crate::params!("orient":true;"use_pscale":true),cook:copy};
fn row(g:&Geometry,name:&str,i:usize,width:usize)->Option<Vec<f64>>{
    let a=g.point.get(name)?;if a.width!=width{return None;}
    Some(match &a.data{AttrData::F32(v)=>v[i*width..(i+1)*width].iter().map(|&x|x as f64).collect(),AttrData::I32(v)=>v[i*width..(i+1)*width].iter().map(|&x|x as f64).collect()})
}
fn dot(a:[f64;3],b:[f64;3])->f64{(0..3).map(|i|a[i]*b[i]).sum()}
fn cross(a:[f64;3],b:[f64;3])->[f64;3]{[a[1]*b[2]-a[2]*b[1],a[2]*b[0]-a[0]*b[2],a[0]*b[1]-a[1]*b[0]]}
fn normal(v:[f64;3])->[f64;3]{let l=dot(v,v).sqrt();let l=if l<1e-20{1.}else{l};v.map(|x|(x/l) as f32 as f64)}
fn z_rotation(n:[f64;3])->M3{
    let [nx,ny,c]=n;if c< -1.+1e-9{return [[1.,0.,0.],[0.,-1.,0.],[0.,0.,-1.]];}
    let(x,y,s)=(-ny,nx,1./(1.+c));
    [[1.-s*y*y,s*x*y,y],[s*x*y,1.-s*x*x,-x],[-y,x,1.-s*(x*x+y*y)]]
}
fn rotation(g:&Geometry,i:usize,orient:bool)->Result<M3>{
    if !orient{return Ok(IDENTITY);}
    if let Some(q)=row(g,"orient",i,4){let l=q.iter().map(|x|x*x).sum::<f64>().sqrt().max(1e-30);let(x,y,z,w)=(q[0]/l,q[1]/l,q[2]/l,q[3]/l);
        return Ok([[1.-2.*(y*y+z*z),2.*(x*y-z*w),2.*(x*z+y*w)],[2.*(x*y+z*w),1.-2.*(x*x+z*z),2.*(y*z-x*w)],[2.*(x*z-y*w),2.*(y*z+x*w),1.-2.*(x*x+y*y)]]);
    }
    if let Some(e)=row(g,"rot",i,3){let(x,y,z)=(e[0],e[1],e[2]);let(sx,cx)=x.sin_cos();let(sy,cy)=y.sin_cos();let(sz,cz)=z.sin_cos();
        return Ok([[cy*cz,sx*sy*cz-cx*sz,cx*sy*cz+sx*sz],[cy*sz,sx*sy*sz+cx*cz,cx*sy*sz-sx*cz],[-sy,sx*cy,cx*cy]]);
    }
    if g.point.contains_key("N"){
        let n=row(g,"N",i,3).ok_or_else(||Error::new("N must have three components"))?;
        let z=if g.point.contains_key("up"){normal([n[0],n[1],n[2]])}else{
            let q=[n[0] as f32,n[1] as f32,n[2] as f32];let ln=(q[0]*q[0]+q[1]*q[1]+q[2]*q[2]).sqrt();let ln=if ln<1e-20{1.}else{ln};q.map(|x|(x/ln) as f64)
        };let r=z_rotation(z);
        if g.point.contains_key("up") {let u=row(g,"up",i,3).ok_or_else(||Error::new("up must have three components"))?;
            let x=cross([u[0],u[1],u[2]],z);let l=dot(x,x).sqrt();let x=if l>1e-9{x.map(|v|v/l)}else{[r[0][0],r[1][0],r[2][0]]};let y=cross(z,x);
            return Ok(std::array::from_fn(|k|[x[k],y[k],z[k]]));
        }return Ok(r);
    }Ok(IDENTITY)
}
fn copy(inputs:&Inputs,p:&Params)->Result<Geometry>{
    let src=COPY_TO_POINTS.one(inputs,0)?;let targets=COPY_TO_POINTS.one(inputs,1)?;
    let(s,t)=(src.npoints,targets.npoints);let orient=p.b("orient")?;let use_pscale=p.b("use_pscale")?;
    if s==0 || t==0{return Geometry::points_only(Vec::new());}
    for n in [s,src.nvertices(),src.nprims()] {if n.checked_mul(t).is_none_or(|v|v>MAX_ELEMENTS){return Err(Error::new("Copy to Points exceeds the topology index limit"));}}
    // Match the Python byte estimate before allocating temporary position arrays.
    let bytes=|g:&Geometry|g.point.values().chain(g.vertex.values()).chain(g.prim.values()).map(|a|a.rows()*a.width*4).sum::<usize>();
    let estimate=(t as u128)*(bytes(&src)+src.p().len()*4+src.nvertices()*4+(src.nprims()+1)*4) as u128+(t as u128)*(s as u128)*72+
        (s as u128)*targets.point.iter().filter(|(n,_)|!TRANSFORM.contains(&n.as_str())).map(|(_,a)|a.rows()*a.width*4).sum::<usize>() as u128;
    let limit=copy_limit_bytes();
    if estimate>limit as u128{return Err(Error::new(format!("Copy to Points would require approximately {:.2} GiB; the safety limit is {} MiB. Reduce the point count or adjust the user Copy memory limit in Preferences.",estimate as f64/1024f64.powi(3),limit/1024/1024)));}
    let rich=["orient","rot","up","scale"].iter().any(|n|targets.point.contains_key(*n));
    let mut positions=Vec::with_capacity(s*t*3);let mut normals=Vec::new();let mut matrices=Vec::with_capacity(t);
    for i in 0..t{
        let r=rotation(&targets,i,orient)?;let mut scale=row(&targets,"scale",i,3).unwrap_or_else(||vec![1.;3]);
        let ps=if use_pscale {targets.point.get("pscale").map(|a|match &a.data{AttrData::F32(v)=>v[i*a.width] as f64,AttrData::I32(v)=>v[i*a.width] as f64}).unwrap_or(1.)}else{1.};
        for v in &mut scale{*v*=ps;}
        for q in src.p().chunks_exact(3){
            let out:[f32;3]=if rich {let v=math::apply(&r,std::array::from_fn(|k|q[k] as f64*scale[k]));std::array::from_fn(|k|(v[k]+targets.p()[i*3+k] as f64) as f32)}else{
                // numpy's fast path scales and multiplies in float32, then adds P.
                let v:[f32;3]=std::array::from_fn(|k|q[k]*ps as f32);
                let rotated:[f32;3]=if orient && targets.point.contains_key("N"){std::array::from_fn(|k| r[k][0] as f32*v[0]+r[k][1] as f32*v[1]+r[k][2] as f32*v[2])}else{v};
                std::array::from_fn(|k|rotated[k]+targets.p()[i*3+k])
            };positions.extend(out);
        }matrices.push(r);
    }
    let mut vp=Vec::with_capacity(src.nvertices()*t);let mut offs=Vec::with_capacity(src.nprims()*t+1);let mut closed=Vec::new();
    for i in 0..t{vp.extend(src.topology.vertex_point.iter().map(|&v|v+(i*s) as i32));offs.extend(src.topology.prim_offsets[..src.nprims()].iter().map(|&v|v+(i*src.nvertices()) as i32));closed.extend(src.topology.closed_mask());}offs.push(vp.len() as i32);
    let mut out=Geometry::from_arrays(positions,vp,offs,src.topology.has_open().then_some(closed))?;
    let src_rows:Vec<_>=(0..t).flat_map(|_|0..s).collect();let target_rows:Vec<_>=(0..t).flat_map(|i|std::iter::repeat_n(i,s)).collect();
    for (g,rows) in [(&*src,&src_rows),(&*targets,&target_rows)] {for(name,a)in &g.point{if !TRANSFORM.contains(&name.as_str()){out.point.insert(name.clone(),Arc::new(super::group::gather(a,rows)));}}}
    if src.point.contains_key("N") {for r in &matrices{for i in 0..s {let n=row(&src,"N",i,3).ok_or_else(||Error::new("source N must have three components"))?;normals.extend(normal(math::apply(r,[n[0],n[1],n[2]])).map(|x|x as f32));}}
        out.point.insert("N".into(),Arc::new(Attr::f32(3,normals)));
    }
    out.vertex=super::group::gather_table(&src.vertex,&(0..t).flat_map(|_|0..src.nvertices()).collect::<Vec<_>>());
    out.prim=super::group::gather_table(&src.prim,&(0..t).flat_map(|_|0..src.nprims()).collect::<Vec<_>>());out.detail=src.detail.clone();Ok(out)
}
#[cfg(test)]
mod tests{
    use super::*;
    #[test]
    fn memory_preference_matches_python_validation(){
        assert_eq!(limit_from_preferences(None),256*1024*1024);
        assert_eq!(limit_from_preferences(Some(&serde_json::json!({"copy_memory_limit_mib":16}))),16*1024*1024);
        for value in [serde_json::json!(true),serde_json::json!(256.0),serde_json::json!(0),serde_json::json!(16385)]{
            assert_eq!(limit_from_preferences(Some(&serde_json::json!({"copy_memory_limit_mib":value}))),256*1024*1024);
        }
    }
    #[test]
    fn euler_scale_target_override_and_open_topology(){
        let mut src=Geometry::from_arrays(vec![1.,0.,0.,2.,0.,0.],vec![0,1],vec![0,2],Some(vec![false])).unwrap();
        src.point.insert("id".into(),Arc::new(Attr::i32(1,vec![1,2])));
        let mut pts=Geometry::points_only(vec![0.,0.,0.]).unwrap();
        pts.point.insert("rot".into(),Arc::new(Attr::f32(3,vec![0.,0.,std::f32::consts::FRAC_PI_2])));
        pts.point.insert("scale".into(),Arc::new(Attr::f32(3,vec![2.,1.,1.])));pts.point.insert("pscale".into(),Arc::new(Attr::f32(1,vec![3.])));
        pts.point.insert("id".into(),Arc::new(Attr::i32(1,vec![9])));
        let out=copy(&[vec![Arc::new(src)],vec![Arc::new(pts)]],&Params::resolve(COPY_TO_POINTS.params,None).unwrap()).unwrap();
        assert!(out.p()[0].abs()<1e-5);assert!((out.p()[1]-6.).abs()<1e-5);assert!((out.p()[4]-12.).abs()<1e-5);
        assert_eq!(out.point["id"].data,AttrData::I32(vec![9,9]));assert!(out.topology.has_open());out.validate().unwrap();
    }
    #[test]
    fn quaternion_precedes_euler_and_n_up_frame(){
        let mut pts=Geometry::points_only(vec![0.;3]).unwrap();pts.point.insert("orient".into(),Arc::new(Attr::f32(4,vec![0.,0.,1.,1.])));
        pts.point.insert("rot".into(),Arc::new(Attr::f32(3,vec![0.;3])));
        let r=rotation(&pts,0,true).unwrap();let v=math::apply(&r,[1.,0.,0.]);assert!(v[0].abs()<1e-12);assert!((v[1]-1.).abs()<1e-12);
        pts.point.remove("orient");pts.point.remove("rot");pts.point.insert("N".into(),Arc::new(Attr::f32(3,vec![1.,0.,0.])));pts.point.insert("up".into(),Arc::new(Attr::f32(3,vec![0.,0.,1.])));
        assert_eq!(math::apply(&rotation(&pts,0,true).unwrap(),[1.,0.,0.]),[0.,1.,0.]);
    }
}
