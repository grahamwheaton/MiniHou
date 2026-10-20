use super::*;

/// `Cd` as one RGB per point: point Cd as is; vertex and primitive Cd averaged onto the points they touch.
pub(super) fn point_colors(g:&Geometry) -> Option<Vec<f32>> {
    let t=&g.topology; let n=g.npoints;
    let rgb=|a:&minihou_core::geometry::Attr,i:usize| -> Option<[f32;3]> {
        let v=a.as_f32()?; let w=a.width; if w<3 { return None; }
        let s=v.get(i*w..i*w+3)?; Some([s[0],s[1],s[2]])
    };
    if let Some(a)=g.point.get("Cd") { if a.as_f32().is_some() && a.width>=3 && a.rows()>=n {
        return Some((0..n).flat_map(|i|rgb(a,i).unwrap_or([0.72;3])).collect());
    } }
    let (mut sum,mut cnt)=(vec![0f32;n*3],vec![0u32;n]);
    let mut add=|pt:i32,c:[f32;3]| { let k=pt as usize; if k<n { for j in 0..3 { sum[k*3+j]+=c[j]; } cnt[k]+=1; } };
    if let Some(a)=g.vertex.get("Cd") { for (i,&pt) in t.vertex_point.iter().enumerate() { if let Some(c)=rgb(a,i) { add(pt,c); } } }
    else if let Some(a)=g.prim.get("Cd") { for prim in 0..t.nprims() {
        if let Some(c)=rgb(a,prim) { for &pt in &t.vertex_point[t.prim_offsets[prim] as usize..t.prim_offsets[prim+1] as usize] { add(pt,c); } }
    } }
    else { return None; }
    Some((0..n).flat_map(|i|{ let c=cnt[i].max(1) as f32; if cnt[i]==0 {[0.72;3]} else {[sum[i*3]/c,sum[i*3+1]/c,sum[i*3+2]/c]} }).collect())
}
