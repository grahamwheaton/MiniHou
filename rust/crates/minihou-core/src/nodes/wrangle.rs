//! Safe initial wrangle subset: one float point/component assignment, arithmetic,
//! sin/cos/abs/sqrt. Unsupported syntax is refused before any geometry is changed.
use std::sync::Arc;
use crate::{Attr, Error, Geometry, Result, params::Params};
use super::{InputSpec, Inputs, NodeSpec, scalar_expr::{Expr,Number}};
pub const WRANGLE:NodeSpec=NodeSpec{name:"wrangle",inputs:&[
    InputSpec{name:"geo",multi:false,optional:false},
    InputSpec{name:"opinput1",multi:false,optional:true},
    InputSpec{name:"opinput2",multi:false,optional:true},
    InputSpec{name:"opinput3",multi:false,optional:true},
],params:crate::params!("snippet":"@P.z += sin(@P.x)";"run_over":"points",unsupported;"group":""),cook:wrangle};

fn reference(s:&str)->Result<(&str,Option<usize>)>{
    let text=s.strip_prefix('@').ok_or_else(||Error::new("wrangle subset requires attribute references starting with @"))?;
    let (name,component)=match text.split_once('.'){
        Some((name,c))=>(name,Some(match c{"x"|"r"=>0,"y"|"g"=>1,"z"|"b"=>2,"w"|"a"=>3,_=>return Err(Error::new("unsupported attribute component"))})),
        None=>(text,None)
    };
    if name.is_empty()||!name.as_bytes()[0].is_ascii_alphabetic()&&name.as_bytes()[0]!=b'_'
        ||!name.bytes().all(|c|c.is_ascii_alphanumeric()||c==b'_'){return Err(Error::new("invalid attribute name"));}
    Ok((name,component))
}
fn check_expr(e:&Expr)->Result<()>{match e{
    Expr::Number(v) if v.is_finite()&&v.abs()<9_007_199_254_740_992.=>Ok(()),
    Expr::Name(_)=>Ok(()),Expr::Unary(_,a)=>check_expr(a),
    Expr::Binary(op,a,b) if ["+","-","*","/"].contains(op)=>{
        check_expr(a)?;check_expr(b)?;
        let mut names=Vec::new();e.names(&mut names);
        if names.is_empty(){let value=e.eval(&mut |_|Err(Error::new("unknown constant")))?.value;
            if !value.is_finite()||value.abs()>=9_007_199_254_740_992.{return Err(Error::new("constant arithmetic exceeds the exact numeric subset"));}
        }Ok(())
    },
    Expr::Call(n,args) if ["sin","cos","abs","sqrt"].contains(&n.as_str())&&args.len()==1=>{
        // NumPy functions on Python constants return strong float64 scalars; the
        // subset deliberately keeps functions on float32 attributes/constants.
        let mut names=Vec::new();args[0].names(&mut names);
        if names.is_empty(){return Err(Error::new("wrangle subset requires an attribute or PI/E/time in function arguments"));}
        check_expr(&args[0])
    },
    _=>Err(Error::new("wrangle subset supports + - * / and sin/cos/abs/sqrt only"))
}}
fn read(g:&Geometry,text:&str,row:Option<usize>,created:Option<&str>)->Result<Number>{
    if text=="PI"{return Ok(Number::array(std::f32::consts::PI));}
    if text=="E"{return Ok(Number::array(std::f32::consts::E));}
    let (name,component)=reference(text)?;
    if component.is_none(){match name{
        "Frame"=>return Ok(Number::array(crate::timectx::frame() as f32)),
        "Time"=>return Ok(Number::array(crate::timectx::seconds() as f32)),
        "TimeInc"=>return Ok(Number::array(crate::timectx::time_inc() as f32)),_=>{}
    }}
    let Some(a)=g.point.get(name) else{
        if created==Some(name)&&component.is_none(){return Ok(Number::array(0.));}
        return Err(Error::new(format!("wrangle subset requires a float point attribute: {text}")));
    };
    let values=a.as_f32().ok_or_else(||Error::new(format!("wrangle subset does not read integer attributes: {text}")))?;
    let c=component.unwrap_or(0);
    if component.is_none()&&a.width!=1{return Err(Error::new(format!("select a component of {text}")));}
    if c>=a.width{return Err(Error::new(format!("component outside {text}")));}
    Ok(Number::array(row.map(|i|values[i*a.width+c]).unwrap_or(0.)))
}
fn wrangle(inputs:&Inputs,p:&Params)->Result<Geometry>{
    let g=WRANGLE.one(inputs,0)?;
    let snippet=p.s("snippet")?;
    let text=snippet.trim().trim_end_matches(';').trim();
    if text.is_empty(){return Ok((*g).clone());}
    if text.contains(';')||text.contains('\n'){return Err(Error::new("wrangle subset supports one assignment per snippet"));}
    let (left,right)=text.split_once('=').ok_or_else(||Error::new("wrangle subset requires one assignment"))?;
    let left=left.trim();let (left,op)=match left.as_bytes().last(){
        Some(b'+')=>(&left[..left.len()-1],"+"),Some(b'-')=>(&left[..left.len()-1],"-"),
        Some(b'*')=>(&left[..left.len()-1],"*"),Some(b'/')=>(&left[..left.len()-1],"/"),_=>(left, "=")
    };
    let left=left.trim();let declared=left.starts_with("f@");
    let target=if declared{&left[1..]}else{left};let (name,component)=reference(target)?;
    if ["ptnum","numpt","primnum","numprim","vtxnum","numvtx","closed","Frame","Time","TimeInc"].contains(&name)
        ||name.starts_with("opinput")||name.starts_with("near"){return Err(Error::new("cannot write a read-only attribute"));}
    let old=g.point.get(name);
    if old.is_none()&&(!declared||component.is_some()){return Err(Error::new("new scalar attributes require f@name"));}
    let width=old.map_or(1,|a|a.width);
    if declared&&width!=1{return Err(Error::new("f@ requires a scalar attribute"));}
    if component.is_none()&&width!=1{return Err(Error::new("wrangle subset requires a scalar or vector component destination"));}
    if component.is_some_and(|c|c>=width){return Err(Error::new("destination component is outside attribute width"));}
    let mut values=match old{Some(a)=>a.as_f32().ok_or_else(||Error::new("wrangle subset writes float attributes only"))?.to_vec(),None=>vec![0.;g.npoints]};
    let expr=Expr::parse(right)?;check_expr(&expr)?;
    let mut names=Vec::new();expr.names(&mut names);
    let created=if old.is_none(){Some(name)}else{None};
    for text in &names{read(&g,text,None,created)?;}
    // Validate functions even when the input has no points.
    expr.eval(&mut |text|read(&g,text,None,created))?;
    let selected=super::groups::point_mask(&g,&p.s("group")?)?;
    for (i,select) in selected.into_iter().enumerate(){
        let v=expr.eval(&mut |text|read(&g,text,Some(i),created))?;
        let slot=i*width+component.unwrap_or(0);
        let x=values[slot];let y=v.value as f32;
        let value=match op{"+"=>x+y,"-"=>x-y,"*"=>x*y,"/"=>x/y,_=>y};
        if select{values[slot]=value;}
    }
    let mut out=(*g).clone();out.point.insert(name.into(),Arc::new(Attr::f32(width,values)));
    out.vertex.remove(name);out.prim.remove(name);Ok(out)
}

#[cfg(test)]mod tests{
    use super::*;use serde_json::json;
    fn cook(snippet:&str)->Result<Geometry>{let g=Geometry::points_only(vec![0.,0.,2.,1.,0.,3.]).unwrap();let p=Params::resolve(WRANGLE.params,json!({"snippet":snippet}).as_object())?;wrangle(&[vec![Arc::new(g)],vec![],vec![],vec![]],&p)}
    #[test]fn default_float_component(){let out=cook("@P.z += sin(@P.x)").unwrap();assert_eq!(out.p()[2],2.);assert!((out.p()[5]-(3.+1f32.sin())).abs()<1e-6);}
    #[test]fn scalar_creation(){let out=cook("f@mask = @P.z / 2").unwrap();assert_eq!(out.point["mask"].as_f32().unwrap(),&[1.,1.5]);}
    #[test]fn rejects_host_access_and_unsupported_forms(){for s in ["@P.x = __import__('os')","@P = set(1,2,3)","i@x=1","@P.x=1;@P.y=2","@P.x=@opinput1_P.x"]{assert!(cook(s).is_err(),"{s}");}}
}
