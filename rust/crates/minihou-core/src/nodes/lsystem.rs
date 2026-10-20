//! L-System skeleton source. Advanced parametric rules and tube/leaf stamping fail explicitly.
use std::{collections::BTreeMap,sync::Arc};
use super::{InputSpec,Inputs,NodeSpec,model::{add,mul}};
use crate::{error::{Error,Result},geometry::{Attr,Geometry},params::Params};

pub const LSYSTEM:NodeSpec=NodeSpec{ outputs: &["geometry"], cook_multi: None, cook_sidecar: None,name:"lsystem",inputs:&[
    InputSpec{name:"leaf_j",multi:false,optional:true},InputSpec{name:"leaf_k",multi:false,optional:true},
    InputSpec{name:"leaf_m",multi:false,optional:true},InputSpec{name:"meta_test",multi:false,optional:true}],params:crate::params!(
    "type":"skeleton";"geometry_type":"skeleton";"generations":4.;"start_position":[0.,0.,0.];"random_scale":0.;
    "random_seed":0;"seed":0;"continuous_angles":true;"continuous_length":true;"continuous_width":true;
    "apply_color":false;"image_file":"";"uv_increment_u":0.1;"uv_increment_v":0.1;"point_attributes":false;
    "rows":3;"cols":3;"tension":3.;"branch_blend":0.;"thickness":0.05;"width":0.05;
    "thickness_scale":0.7;"width_scale":0.7;"apply_tube_texture_coordinates":false;"vertical_increment":0.1;
    "step_size":0.1;"step":0.1;"step_size_scale":0.9;"step_scale":0.9;"angle":22.5;"angle_scale":1.;
    "separate_angles":false;"yaw":22.5;"pitch":22.5;"roll":137.5;"initial_orientation":[0.,0.,0.];
    "variable_b":1.;"variable_c":1.;"variable_d":1.;"gravity":0.;"variables":"";"pic_image_file":"";
    "group_prefix":"lsys";"channel_prefix":"lsys";"leaf_param_a":"lsys";"leaf_param_b":"";"leaf_param_c":"";
    "read_rules_from_file":false;"rule_file":"./rules.txt";"context_ignore":"F+-";"context_includes_siblings":false;
    "premise":"F";"rules":"F=FF-[-F+F+F]+[+F-F-F]"),cook:lsystem};

fn pick_f(p:&Params,primary:&str,alias:&str,default:f64)->Result<f64>{let a=p.f(primary)?;let b=p.f(alias)?;Ok(if a!=default{a}else if b!=default{b}else{a})}
fn pick_i(p:&Params,primary:&str,alias:&str)->Result<i64>{let a=p.i(primary)?;let b=p.i(alias)?;Ok(if a!=0{a}else{b})}
fn rotate(a:&mut [f64;3],b:&mut [f64;3],angle:f64){let (s,c)=angle.sin_cos();let old=*a;*a=std::array::from_fn(|k|c*old[k]+s*b[k]);*b=std::array::from_fn(|k|c*b[k]-s*old[k]);}
#[derive(Clone)]
struct State {pos:[f64;3],h:[f64;3],l:[f64;3],u:[f64;3],width:f64,step:f64,angle_scale:f64,cur:usize,arc:f64,lage:f64,group:i32}
#[derive(Clone)]
struct Point {pos:[f64;3],h:[f64;3],u:[f64;3],width:f64,arc:f64,lage:f64,born:i32,leaf:bool}
fn lsystem(inputs:&Inputs,p:&Params)->Result<Geometry>{
    if inputs[..3].iter().any(|v|!v.is_empty()) {return Err(Error::new("lsystem leaf geometry stamping is not supported in Rust yet"));}
    if p.b("read_rules_from_file")? || p.b("apply_color")? || !p.s("variables")?.is_empty() || !p.s("pic_image_file")?.is_empty() {
        return Err(Error::new("lsystem rule files, image functions, and user variables are not supported in Rust yet"));
    }
    let ty=p.s("type")?;let alias=p.s("geometry_type")?;if (if ty!="skeleton"{ty}else{alias})=="tube"{return Err(Error::new("lsystem tube geometry is not supported in Rust yet"));}
    let generations=p.f("generations")?;if !(0.0..=12.0).contains(&generations){return Err(Error::new("generations must be 0..12"));}
    let n=generations.floor() as usize;let frac=generations-n as f64;let partial=frac>1e-9 && (p.b("continuous_angles")?||p.b("continuous_length")?||p.b("continuous_width")?);
    let total=n+usize::from(partial);
    let mut rules=BTreeMap::<char,String>::new();for line in p.s("rules")?.lines(){let line=line.trim();if line.is_empty()||line.starts_with('#'){continue;}
        for part in line.split(';'){let part=part.trim();if part.is_empty(){continue;}let (lhs,rhs)=part.split_once('=').or_else(||part.split_once("->")).ok_or_else(||Error::new("lsystem rule needs '='"))?;
            let lhs=lhs.trim();if lhs.chars().count()!=1 || lhs.contains(['<','>',':','(']) || rhs.contains([':', '(', ')']) {
                return Err(Error::new("lsystem context, conditional and stochastic rules are not supported in Rust yet"));}
            rules.insert(lhs.chars().next().unwrap(),rhs.trim().to_string());}}
    let premise=p.s("premise")?;if premise.contains(['(',')']) {return Err(Error::new("lsystem parametric symbols are not supported in Rust yet"));}
    let mut tokens:Vec<(char,i32)>=premise.chars().filter(|c|!c.is_whitespace()).map(|c|(c,0)).collect();
    for generation in 0..total {let mut next=Vec::new();for (ch,born) in tokens {if let Some(rhs)=rules.get(&ch){next.extend(rhs.chars().filter(|c|!c.is_whitespace()).map(|c|(c,(generation+1) as i32)));}else{next.push((ch,born));}
            if next.len()>2_000_000{return Err(Error::new("the string grew past 2,000,000 symbols; use fewer generations"));}}
        tokens=next;}
    let symbols=tokens.len();
    let start=p.v3("start_position")?;let mut state=State{pos:start,h:[0.,0.,1.],l:[-1.,0.,0.],u:[0.,-1.,0.],width:pick_f(p,"thickness","width",0.05)?,step:pick_f(p,"step_size","step",0.1)?,angle_scale:1.,cur:0,arc:0.,lage:0.,group:-1};
    let init=p.v3("initial_orientation")?;rotate(&mut state.h,&mut state.l,init[0].to_radians());rotate(&mut state.h,&mut state.u,init[1].to_radians());rotate(&mut state.l,&mut state.u,init[2].to_radians());
    let (yaw,pitch,roll)=if p.b("separate_angles")?{(p.f("yaw")?,p.f("pitch")?,p.f("roll")?)}else{let a=p.f("angle")?;(a,a,a)};
    let width_scale=pick_f(p,"thickness_scale","width_scale",0.7)?;let step_scale=pick_f(p,"step_size_scale","step_scale",0.9)?;
    let mut rng=super::np_random::Rng::new(&[(pick_i(p,"random_seed","seed")? as u64)&0xffff_ffff,1]);
    let mut pts=vec![Point{pos:state.pos,h:state.h,u:state.u,width:state.width,arc:0.,lage:0.,born:0,leaf:false}];
    let mut curves=Vec::<(Vec<usize>,i32)>::new();let mut current:Option<usize>=None;let mut stack=Vec::<State>::new();let mut cut:Option<usize>=None;let mut group_counter=0i32;
    for (ch,born) in tokens {if let Some(depth)=cut.as_mut(){if ch=='['{*depth+=1;}else if ch==']'{if *depth==0{cut=None;}else{*depth-=1;}}continue;}
        let grow=if partial&&born==total as i32{frac}else{1.};
        match ch {
            'F'|'H'|'f'|'h'|'G'=>{let mut length=state.step*if ch=='H'||ch=='h'{0.5}else{1.};let random_scale=p.f("random_scale")?;if random_scale>0.{length*=0f64.max(1.+random_scale*(rng.random()*2.-1.));}
                if p.b("continuous_length")?{length*=grow;}let new=add(state.pos,mul(state.h,length));state.arc+=length;
                if ch=='F'||ch=='H'{state.lage+=p.f("vertical_increment")?;let width=state.width*if p.b("continuous_width")?{grow}else{1.};
                    let id=pts.len();pts.push(Point{pos:new,h:state.h,u:state.u,width,arc:state.arc,lage:state.lage,born,leaf:false});
                    if let Some(curve)=current{curves[curve].0.push(id);}else{curves.push((vec![state.cur,id],state.group));current=Some(curves.len()-1);}state.cur=id;
                }else if ch=='f'||ch=='h'{state.cur=pts.len();pts.push(Point{pos:new,h:state.h,u:state.u,width:state.width,arc:state.arc,lage:state.lage,born,leaf:false});current=None;}
                state.pos=new;}
            '+'|'-'|'&'|'^'|'\\'|'/' =>{let base=if ch=='+'||ch=='-'{yaw}else if ch=='&'||ch=='^'{pitch}else{roll};let mut a=base.to_radians()*state.angle_scale;if p.b("continuous_angles")?{a*=grow;}if matches!(ch,'-'|'^'|'/'){a=-a;}
                match ch{'+'|'-'=>rotate(&mut state.h,&mut state.l,a),'&'|'^'=>rotate(&mut state.h,&mut state.u,a),_=>rotate(&mut state.l,&mut state.u,a)};}
            '['=>{stack.push(state.clone());current=None;state.width*=width_scale;}
            ']'=>{state=stack.pop().ok_or_else(||Error::new("']' without a matching '['"))?;current=None;}
            '|'=>{state.h=mul(state.h,-1.);state.l=mul(state.l,-1.);}
            '*'=>{state.l=mul(state.l,-1.);state.u=mul(state.u,-1.);}
            '!'=>state.width*=width_scale,'?'=>state.width/=width_scale.max(1e-12),
            '"'=>state.step*=step_scale,'_'=>state.step/=step_scale.max(1e-12),
            ';'=>state.angle_scale*=p.f("angle_scale")?,'@'=>state.angle_scale/=p.f("angle_scale")?.max(1e-12),
            'g'=>{state.group=group_counter;group_counter+=1;current=None;}
            'J'=>pts.push(Point{pos:state.pos,h:state.h,u:state.u,width:state.width,arc:state.arc,lage:state.lage,born,leaf:true}),
            '%'=>cut=Some(0),
            'K'|'M'|'a'|'{'|'.'|'}'|'T'|'$'|'~'=>return Err(Error::new(format!("lsystem command {ch:?} is not supported in Rust yet"))),
            _=>{}
        }
    }
    if !stack.is_empty(){return Err(Error::new("'[' without a matching ']'"));}
    let mut used=vec![false;pts.len()];for (curve,_) in &curves{if curve.len()>=2{for &i in curve{used[i]=true;}}}for (i,pt) in pts.iter().enumerate(){used[i]|=pt.leaf;}
    let mut remap=vec![0i32;pts.len()];let mut compact=Vec::new();for (i,pt) in pts.iter().enumerate(){if used[i]{remap[i]=compact.len() as i32;compact.push(pt.clone());}}
    let mut vp=Vec::new();let mut offsets=vec![0];let mut groups=Vec::new();for (curve,gid) in &curves{if curve.len()<2{continue;}vp.extend(curve.iter().map(|&i|remap[i]));offsets.push(vp.len() as i32);groups.push(*gid);}
    let mut out=Geometry::from_arrays(compact.iter().flat_map(|pt|pt.pos.map(|x|x as f32)).collect(),vp,offsets,Some(vec![false;groups.len()]))?;
    out.point.insert("width".into(),Arc::new(Attr::f32(1,compact.iter().map(|pt|pt.width as f32).collect())));
    out.point.insert("N".into(),Arc::new(Attr::f32(3,compact.iter().flat_map(|pt|pt.h.map(|x|x as f32)).collect())));
    out.point.insert("group_leaves".into(),Arc::new(Attr::i32(1,compact.iter().map(|pt|pt.leaf as i32).collect())));
    if p.b("point_attributes")? {for (name,attr) in [
        ("segs",Attr::i32(1,vec![p.i("cols")? as i32;compact.len()])),("div",Attr::i32(1,vec![p.i("rows")? as i32;compact.len()])),
        ("lage",Attr::f32(1,compact.iter().map(|pt|pt.lage as f32).collect())),("arc",Attr::f32(1,compact.iter().map(|pt|pt.arc as f32).collect())),
        ("up",Attr::f32(3,compact.iter().flat_map(|pt|mul(pt.u,-1.).map(|x|x as f32)).collect())),
        ("gen",Attr::i32(1,compact.iter().map(|pt|pt.born).collect()))]{out.point.insert(name.into(),Arc::new(attr));}}
    for gid in groups.iter().copied().filter(|&x|x>=0){let key=format!("group_{}{}",p.s("group_prefix")?,gid);out.prim.entry(key).or_insert_with(||Arc::new(Attr::i32(1,groups.iter().map(|&x|(x==gid) as i32).collect())));}
    out.detail.insert("lsystem_symbols".into(),serde_json::json!(symbols));Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn python_branch_golden() {
        // Python lsystem(premise="F[+F]F", rules="", generations=0, step_size=1, thickness=.2, angle=90).
        let p=Params::resolve(LSYSTEM.params,serde_json::json!({"premise":"F[+F]F","rules":"","generations":0.,"step_size":1.,"thickness":0.2,"angle":90.}).as_object()).unwrap();
        let out=lsystem(&[vec![],vec![],vec![],vec![]],&p).unwrap();
        assert_eq!(out.p(), &[0.,0.,0.,0.,0.,1.,-1.,0.,1.,0.,0.,2.]);
        assert_eq!(out.topology.vertex_point,[0,1,1,2,1,3]);
        assert_eq!(out.topology.prim_offsets,[0,2,4,6]);
        assert_eq!(out.point["width"].as_f32().unwrap(), &[0.2,0.2,0.14,0.2]);
        assert_eq!(out.detail["lsystem_symbols"],6);
    }
    #[test]
    fn unsupported_parametric_rule_is_explicit() {
        let p=Params::resolve(LSYSTEM.params,serde_json::json!({"rules":"A(i)=F(i+1)"}).as_object()).unwrap();
        assert!(lsystem(&[vec![],vec![],vec![],vec![]],&p).is_err());
    }
    #[test]
    fn python_default_counts() {
        let p=Params::resolve(LSYSTEM.params,None).unwrap();
        let out=lsystem(&[vec![],vec![],vec![],vec![]],&p).unwrap();
        assert_eq!((out.npoints,out.nprims(),out.nvertices(),out.detail["lsystem_symbols"].as_u64().unwrap()),(4097,1536,5632,11116));
    }

    #[test]
    fn test_lsystem_empty_and_single_element() {
        // Empty premise produces safe empty geometry
        let p_empty = Params::resolve(LSYSTEM.params, serde_json::json!({"premise": "", "rules": ""}).as_object()).unwrap();
        let out_empty = lsystem(&[vec![], vec![], vec![], vec![]], &p_empty).unwrap();
        assert_eq!(out_empty.npoints, 0);
        assert_eq!(out_empty.nprims(), 0);

        // Single step 'F': 2 points, 1 curve
        let p_single = Params::resolve(LSYSTEM.params, serde_json::json!({"premise": "F", "rules": "", "generations": 0.}).as_object()).unwrap();
        let out_single = lsystem(&[vec![], vec![], vec![], vec![]], &p_single).unwrap();
        assert_eq!(out_single.npoints, 2);
        assert_eq!(out_single.nprims(), 1);
        assert_eq!(out_single.topology.vertex_point, &[0, 1]);
    }

    #[test]
    fn test_lsystem_errors_and_unsupported() {
        // generations out of 0..12 range
        let p_neg_gen = Params::resolve(LSYSTEM.params, serde_json::json!({"generations": -1.}).as_object()).unwrap();
        assert!(lsystem(&[vec![], vec![], vec![], vec![]], &p_neg_gen).is_err());
        let p_huge_gen = Params::resolve(LSYSTEM.params, serde_json::json!({"generations": 13.}).as_object()).unwrap();
        assert!(lsystem(&[vec![], vec![], vec![], vec![]], &p_huge_gen).is_err());

        // tube geometry unsupported
        let p_tube = Params::resolve(LSYSTEM.params, serde_json::json!({"type": "tube"}).as_object()).unwrap();
        assert!(lsystem(&[vec![], vec![], vec![], vec![]], &p_tube).is_err());

        // leaf geometry input unsupported
        let dummy = Arc::new(Geometry::points_only(vec![0., 0., 0.]).unwrap());
        let p_def = Params::resolve(LSYSTEM.params, None).unwrap();
        assert!(lsystem(&[vec![dummy], vec![], vec![], vec![]], &p_def).is_err());

        // unmatched brackets
        let p_open = Params::resolve(LSYSTEM.params, serde_json::json!({"premise": "F[F", "rules": "", "generations": 0.}).as_object()).unwrap();
        assert!(lsystem(&[vec![], vec![], vec![], vec![]], &p_open).is_err());
        let p_close = Params::resolve(LSYSTEM.params, serde_json::json!({"premise": "F]F", "rules": "", "generations": 0.}).as_object()).unwrap();
        assert!(lsystem(&[vec![], vec![], vec![], vec![]], &p_close).is_err());

        // invalid rule syntax
        let p_bad_rule = Params::resolve(LSYSTEM.params, serde_json::json!({"rules": "invalid_rule"}).as_object()).unwrap();
        assert!(lsystem(&[vec![], vec![], vec![], vec![]], &p_bad_rule).is_err());
    }

    #[test]
    fn test_lsystem_determinism_and_attributes() {
        let p = Params::resolve(LSYSTEM.params, serde_json::json!({
            "premise": "F+F",
            "rules": "",
            "generations": 0.,
            "random_scale": 0.5,
            "seed": 12345,
            "point_attributes": true
        }).as_object()).unwrap();

        let out1 = lsystem(&[vec![], vec![], vec![], vec![]], &p).unwrap();
        let out2 = lsystem(&[vec![], vec![], vec![], vec![]], &p).unwrap();

        // Determinism with seeded random
        assert_eq!(out1.p(), out2.p());
        assert_eq!(out1.point["width"].data, out2.point["width"].data);
        assert_eq!(out1.point["arc"].data, out2.point["arc"].data);
        assert_eq!(out1.point["up"].data, out2.point["up"].data);
        assert!(out1.point.contains_key("segs"));
        assert!(out1.point.contains_key("div"));
        assert!(out1.point.contains_key("gen"));
    }
}
