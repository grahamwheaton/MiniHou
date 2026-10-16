//! Attribute Delete and Attribute Rename (nodes/attribs.py).
use super::{Inputs,NodeSpec,GEO};
use crate::{error::{Error,Result},geometry::Geometry,params::Params};

/// Python fnmatchcase's *, ?, [abc], [!abc], and ranges, with literal unmatched [.
pub(super) fn matches(pattern:&str,name:&str)->bool{
    fn go(p:&[char],s:&[char])->bool{
        if p.is_empty(){return s.is_empty();}
        match p[0]{
            '*'=> (0..=s.len()).any(|i|go(&p[1..],&s[i..])),
            '?'=> !s.is_empty() && go(&p[1..],&s[1..]),
            '['=>{
                let start=1+usize::from(p.get(1)==Some(&'!'));
                let search=start+usize::from(p.get(start)==Some(&']'));
                if let Some(end)=(search..p.len()).find(|&i|p[i]==']'){
                    if s.is_empty(){return false;}let mut hit=false;let mut i=start;
                    while i<end{if i+2<end && p[i+1]=='-'{hit|=p[i]<=s[0] && s[0]<=p[i+2];i+=3;}else{hit|=p[i]==s[0];i+=1;}}
                    if start==2{hit=!hit;}hit && go(&p[end+1..],&s[1..])
                }else{!s.is_empty() && s[0]=='[' && go(&p[1..],&s[1..])}
            }
            c=> !s.is_empty() && c==s[0] && go(&p[1..],&s[1..]),
        }
    }go(&pattern.chars().collect::<Vec<_>>(),&name.chars().collect::<Vec<_>>())
}
pub const ATTRIBUTE_DELETE:NodeSpec=NodeSpec{name:"attribute_delete",inputs:GEO,params:crate::params!("names":""),cook:delete};
fn delete(inputs:&Inputs,p:&Params)->Result<Geometry>{
    let g=ATTRIBUTE_DELETE.one(inputs,0)?;let text=p.s("names")?;let pats:Vec<_>=text.split(|c:char|c==',' || c.is_whitespace()).filter(|s|!s.is_empty()).collect();
    let mut out=(*g).clone();
    for table in [&mut out.point,&mut out.vertex,&mut out.prim]{table.retain(|n,_|n=="P" || !pats.iter().any(|pat|matches(pat,n)));}
    out.detail.retain(|n,_|!pats.iter().any(|pat|matches(pat,n)));Ok(out)
}
pub const ATTRIBUTE_RENAME:NodeSpec=NodeSpec{name:"attribute_rename",inputs:GEO,params:crate::params!("from_name":"";"to_name":""),cook:rename};
fn rename(inputs:&Inputs,p:&Params)->Result<Geometry>{
    let g=ATTRIBUTE_RENAME.one(inputs,0)?;let from=p.s("from_name")?;let to=p.s("to_name")?;
    if from.is_empty() || to.is_empty() || from==to{return Ok((*g).clone());}
    if from=="P" || to=="P"{return Err(Error::new("P cannot be renamed"));}
    let src=[&g.point,&g.vertex,&g.prim].iter().position(|t|t.contains_key(&from)).ok_or_else(||Error::new(format!("no attribute {from:?}")))?;
    let mut out=(*g).clone();let mut tables=[&mut out.point,&mut out.vertex,&mut out.prim];let a=tables[src].remove(&from).unwrap();
    for t in &mut tables{t.remove(&to);}tables[src].insert(to,a);Ok(out)
}
#[cfg(test)]
mod tests{
    use super::*;use crate::geometry::Attr;use std::sync::Arc;
    #[test]
    fn wildcard_and_protected_position(){
        assert!(matches("tmp_[!a-c]?","tmp_z0"));assert!(!matches("tmp_[!a-c]?","tmp_b0"));assert!(matches("[","["));
        let mut g=Geometry::points_only(vec![0.;3]).unwrap();g.point.insert("N".into(),Arc::new(Attr::f32(3,vec![1.;3])));g.detail.insert("P".into(),serde_json::json!(9));
        let out=delete(&[vec![Arc::new(g)]],&Params::resolve(ATTRIBUTE_DELETE.params,serde_json::json!({"names":"*"}).as_object()).unwrap()).unwrap();
        assert_eq!(out.point.len(),1);assert!(out.point.contains_key("P"));assert!(out.detail.is_empty());
    }
    #[test]
    fn rename_replaces_other_domains(){
        let mut g=Geometry::points_only(vec![0.;3]).unwrap();g.point.insert("weight".into(),Arc::new(Attr::f32(1,vec![2.])));g.prim.insert("pscale".into(),Arc::new(Attr::f32(1,vec![])));
        let out=rename(&[vec![Arc::new(g)]],&Params::resolve(ATTRIBUTE_RENAME.params,serde_json::json!({"from_name":"weight","to_name":"pscale"}).as_object()).unwrap()).unwrap();
        assert!(!out.prim.contains_key("pscale"));assert_eq!(out.point["pscale"].as_f32().unwrap(),[2.]);
    }
}
