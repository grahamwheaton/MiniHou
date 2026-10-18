use std::sync::Arc;
use crate::{nodes::{InputSpec, Inputs, NodeSpec}, error::{Error, Result}, geometry::Geometry, params::Params};
use crate::nodes::curves::{norm, sub};

const CONTROLS: &str = r#"[{"p": [-1, 0, 0], "left": [-1.5, 0, 0], "right": [-0.5, 0, 0], "mode": "AUTO"}, {"p": [0, 0, 1], "left": [-0.5, 0, 1], "right": [0.5, 0, 1], "mode": "AUTO"}, {"p": [1, 0, 0], "left": [0.5, 0, 0], "right": [1.5, 0, 0], "mode": "AUTO"}]"#;
pub(super) const NODE: NodeSpec = NodeSpec { name: "bezier", inputs: &[InputSpec{name:"geo",multi:false,optional:true}],
    params: crate::params!("controls": CONTROLS; "closed": false; "resolution": 24; "as_mesh": false), cook: bezier };
fn bezier(inputs:&Inputs,p:&Params)->Result<Geometry> {
    let value:serde_json::Value=serde_json::from_str(&p.s("controls")?).map_err(|e| Error::new(e.to_string()))?;
    let cs=value.as_array().ok_or_else(|| Error::new("Bezier curves need 2 to 10000 control points"))?;
    if !(2..=10000).contains(&cs.len()) { return Err(Error::new("Bezier curves need 2 to 10000 control points")); }
    let mut xyz=Vec::<[[f64;3];3]>::new(); let mut modes=Vec::new();
    for c in cs {
        let mode=c.get("mode").and_then(|v| v.as_str()).unwrap_or("FREE");
        if !["AUTO","VECTOR","ALIGNED","MIRRORED","FREE"].contains(&mode) { return Err(Error::new("Unknown Bezier handle mode")); }
        let mut row=[[0.;3];3];
        for (j,name) in ["p","left","right"].iter().enumerate() {
            let a=c.get(*name).and_then(|v| v.as_array()).filter(|v| v.len()==3).ok_or_else(|| Error::new("Bezier handles must be finite XYZ vectors"))?;
            for i in 0..3 { row[j][i]=a[i].as_f64().filter(|x| x.is_finite()).ok_or_else(|| Error::new("Bezier handles must be finite XYZ vectors"))?; }
        }
        xyz.push(row); modes.push(mode);
    }
    let closed=p.b("closed")?; let n=xyz.len();
    for i in 0..n {
        let pt=xyz[i][0];
        let prev=if closed || i>0 {xyz[(i+n-1)%n][0]} else {pt};
        let next=if closed || i+1<n {xyz[(i+1)%n][0]} else {pt};
        if modes[i]=="VECTOR" {
            xyz[i][1]=std::array::from_fn(|k| pt[k]+(prev[k]-pt[k])/3.);
            xyz[i][2]=std::array::from_fn(|k| pt[k]+(next[k]-pt[k])/3.);
        } else if modes[i]=="AUTO" {
            let d=sub(next,prev); let ln=norm(d).max(1e-12);
            xyz[i][1]=std::array::from_fn(|k| pt[k]-d[k]/ln*norm(sub(pt,prev))/3.);
            xyz[i][2]=std::array::from_fn(|k| pt[k]+d[k]/ln*norm(sub(next,pt))/3.);
        } else if modes[i]=="ALIGNED" || modes[i]=="MIRRORED" {
            let d=sub(xyz[i][2],pt); let ln=norm(d);
            if ln>1e-12 { let size=if modes[i]=="MIRRORED" {ln} else {norm(sub(xyz[i][1],pt))};
                xyz[i][1]=std::array::from_fn(|k| pt[k]-d[k]*size/ln); }
        }
    }
    let res=p.i("resolution")?; if !(1..=1024).contains(&res) {return Err(Error::new("resolution must be 1..1024"));}
    let seg=if closed {n} else {n-1}; crate::nodes::curves::count(seg*res as usize+usize::from(!closed))?;
    let mut out=Vec::new();
    for i in 0..seg { for j in 0..res {
        let t=j as f64/res as f64; let u=1.-t; let a=xyz[i]; let b=xyz[(i+1)%n];
        for k in 0..3 { out.push((u.powi(3)*a[0][k]+3.*u*u*t*a[2][k]+3.*u*t*t*b[1][k]+t.powi(3)*b[0][k]) as f32); }
    } }
    if !closed { out.extend(xyz[n-1][0].map(|x| x as f32)); }
    let mut out=crate::nodes::curves::curve(out,closed)?;
    if p.b("as_mesh")? { out.prim.remove("__viewport_curve"); }
    if let Some(g)=inputs[0].first() { crate::nodes::merge::merge(&[vec![g.clone(),Arc::new(out)]],&Params::resolve(&[],None)?) } else {Ok(out)}
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
