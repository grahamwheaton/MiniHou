//! Parameter-only dependency evaluation. Channel references never request geometry.
use std::collections::{HashMap, HashSet};
use serde_json::{Map, Value};
use crate::{graph::Graph, nodes::{self, scalar_expr::{Expr, Number}}};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ParamDiagKind { CycleDetected, BadReference, BadExpression }

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParamDiag {
    pub node: String,
    pub param: String,
    pub kind: ParamDiagKind,
    pub message: String,
}

#[derive(Default)]
pub struct ResolvedChannels {
    pub params: HashMap<String, Map<String, Value>>,
    pub diagnostics: Vec<ParamDiag>,
    pub time_dependent: HashSet<String>,
}

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
struct Address { node: String, param: String, component: Option<usize> }
impl Address {
    fn label(&self)->String {format!("{}{}",self.param,self.component.map(|i|format!(".{}",["x","y","z"][i])).unwrap_or_default())}
}

struct Resolver<'a> {
    graph: &'a Graph,
    result: ResolvedChannels,
    // Keep the failure as well as its literal fallback. A later dependent must
    // not mistake a failed channel's memoized literal for a successful link.
    memo: HashMap<Address, (Value, Option<ParamDiagKind>)>,
    active: HashSet<Address>,
}

/// Resolve every channel before geometry cooking. Invalid links leave their saved literals intact.
pub fn resolve_graph(graph: &Graph) -> ResolvedChannels {
    let mut r=Resolver{graph,result:ResolvedChannels::default(),memo:HashMap::new(),active:HashSet::new()};
    for (id,def) in &graph.nodes {
        let Some(spec)=nodes::get(&def.node_type) else {continue};
        let mut values=def.params.clone().unwrap_or_default();
        for p in spec.params {
            let address=Address{node:id.clone(),param:p.name.into(),component:None};
            let value=r.resolve(&address).unwrap_or_else(|_|r.literal(&address).unwrap_or_else(||(p.default)()));
            values.insert(p.name.into(),value);
        }
        if def.node_type == "controller" {
            let saved = graph.to_value();
            for entry in crate::controller::parameters(&saved["nodes"][id]) {
                let name = entry["name"].as_str().unwrap();
                let address=Address{node:id.clone(),param:name.into(),component:None};
                let value=r.resolve(&address).unwrap_or_else(|_|r.literal(&address).unwrap());
                values.insert(name.into(),value);
            }
        }
        if def.node_type == "subnet" {
            for name in graph.promoted_params(id).into_iter().filter_map(|p| p["name"].as_str()) {
                let address=Address{node:id.clone(),param:name.into(),component:None};
                if let Ok(value)=r.resolve(&address) { values.insert(name.into(),value); }
            }
        }
        if let Some(channels)=&def.channels {
            for name in channels.keys() {
                if !valid_address(spec.params,name) && graph.promoted_param(id,split_component(name).0).is_none()
                    && graph.controller_param(id,split_component(name).0).is_none_or(|p| split_component(name).1.is_some() && p["kind"] != "vector") {
                    r.result.diagnostics.push(ParamDiag{node:id.clone(),param:name.clone(),kind:ParamDiagKind::BadReference,message:format!("unknown channel target {name:?}")});
                }
            }
        }
        r.result.params.insert(id.clone(),values);
    }
    r.result
}

fn valid_address(specs:&[crate::params::ParamSpec],name:&str)->bool {
    let (base,component)=split_component(name);
    specs.iter().any(|s|s.name==base && (component.is_none() || (s.default)().as_array().is_some_and(|a|a.len()==3)))
}
pub(crate) fn split_component(name:&str)->(&str,Option<usize>) {
    for (suffix,index) in [(".x",0),(".y",1),(".z",2)] {if let Some(base)=name.strip_suffix(suffix){return (base,Some(index));}}
    (name,None)
}
impl Resolver<'_> {
    fn literal(&self,a:&Address)->Option<Value> {
        let def=self.graph.node(&a.node)?;
        let default=nodes::get(&def.node_type)?.params.iter().find(|s|s.name==a.param).map(|s|(s.default)())
            .or_else(||self.graph.controller_param(&a.node,&a.param).map(|p|p["default"].clone()))
            .or_else(||self.graph.promoted_param(&a.node,&a.param).map(|p|p["default"].clone()))?;
        let value=def.params.as_ref().and_then(|m|m.get(&a.param)).cloned().unwrap_or(default);
        match a.component {Some(i)=>value.as_array()?.get(i).cloned(),None=>Some(value)}
    }
    fn expression(&self,a:&Address)->Option<&str> {
        self.graph.node(&a.node)?.channels.as_ref()?.get(&a.label())?.as_str()
    }
    fn diag(&mut self,a:&Address,kind:ParamDiagKind,message:String){
        self.result.diagnostics.push(ParamDiag{node:a.node.clone(),param:a.label(),kind,message});
    }
    fn resolve(&mut self,a:&Address)->std::result::Result<Value,ParamDiagKind> {
        if let Some((value, failure))=self.memo.get(a) {
            return match failure { Some(kind)=>Err(kind.clone()), None=>Ok(value.clone()) };
        }
        if !self.active.insert(a.clone()) {return Err(ParamDiagKind::CycleDetected);}
        let fallback=match self.literal(a){Some(v)=>v,None=>{self.active.remove(a);return Err(ParamDiagKind::BadReference)}};
        let expected=self.graph.node(&a.node).and_then(|d|nodes::get(&d.node_type)).and_then(|s|s.params.iter().find(|p|p.name==a.param)).map(|p|(p.default)())
            .or_else(||self.graph.controller_param(&a.node,&a.param).map(|p|p["default"].clone()))
            .or_else(||self.graph.promoted_param(&a.node,&a.param).map(|p|p["default"].clone())).unwrap_or(Value::Null);
        let expr=self.expression(a).map(str::to_owned);
        let has_expr=expr.is_some();
        let result=if let Some(expr)=expr {
            self.evaluate(a,&expr).and_then(|v|{
                let valid=if a.component.is_some(){v.is_number()}else{match &expected {
                    Value::Array(xs)=>v.as_array().is_some_and(|a|a.len()==xs.len()&&a.iter().all(Value::is_number)),
                    Value::String(_)=>v.is_string(),
                    Value::Bool(_)=>v.is_boolean()||v.is_number(),
                    Value::Number(n) if n.is_i64()||n.is_u64()=>v.as_f64().is_some_and(|x|x.is_finite()&&x.fract()==0.),
                    Value::Number(_)=>v.is_number(),
                    _=>false
                }};
                if valid {Ok(v)} else {Err((ParamDiagKind::BadExpression,"channel result has the wrong parameter type".into()))}
            })
        } else if a.component.is_some() {
            let whole=Address{node:a.node.clone(),param:a.param.clone(),component:None};
            if self.expression(&whole).is_some(){
                self.resolve(&whole).and_then(|v|v.as_array().and_then(|xs|xs.get(a.component.unwrap())).cloned().ok_or(ParamDiagKind::BadReference))
                    .map_err(|k|(k,"whole-vector channel is invalid".into()))
            } else {Ok(fallback.clone())}
        } else if fallback.as_array().is_some_and(|xs|xs.len()==3) {
            let mut xs=Vec::new();
            for i in 0..3 {
                let component=Address{node:a.node.clone(),param:a.param.clone(),component:Some(i)};
                match self.resolve(&component) {Ok(v)=>xs.push(v),Err(_)=>xs.push(fallback[i].clone())}
            }
            Ok(Value::Array(xs))
        } else {Ok(fallback.clone())};
        self.active.remove(a);
        match result {
            Ok(mut v)=>{
                if has_expr && a.component.is_none() && v.as_array().is_some_and(|xs|xs.len()==3) {
                    self.memo.insert(a.clone(),(v.clone(),None));
                    for i in 0..3 {
                        let component=Address{node:a.node.clone(),param:a.param.clone(),component:Some(i)};
                        if self.expression(&component).is_some(){
                            v[i]=self.resolve(&component).unwrap_or_else(|_|fallback[i].clone());
                        }
                    }
                }
                self.memo.insert(a.clone(),(v.clone(),None));Ok(v)
            }
            Err((kind,message))=>{self.diag(a,kind.clone(),message);self.memo.insert(a.clone(),(fallback,Some(kind.clone())));Err(kind)}
        }
    }
    fn evaluate(&mut self,a:&Address,source:&str)->std::result::Result<Value,(ParamDiagKind,String)> {
        let expr=Expr::parse(source).map_err(|e|(ParamDiagKind::BadExpression,e.to_string()))?;
        let mut names=Vec::new();expr.names(&mut names);
        if names.iter().any(|n|matches!(n.as_str(),"@Frame"|"@Time"|"@TimeInc"|"$F"|"$T")){self.result.time_dependent.insert(a.node.clone());}
        let mut failure=None;
        let value=expr.eval_channel(&mut |n| {
            let v=match n {"@Frame"|"$F"=>crate::timectx::frame(),"@Time"|"$T"=>crate::timectx::seconds(),"@TimeInc"=>crate::timectx::time_inc(),_=>return Err(crate::Error::new(format!("unknown channel variable {n}")))};
            Ok(Number::scalar(v))
        },&mut |kind,path| {
            let target=match self.parse_path(a,path) {Some(x)=>x,None=>{failure=Some(ParamDiagKind::BadReference);return Err(crate::Error::new(format!("missing channel path {path:?}")))}};
            let Some(v)=self.literal(&target) else {failure=Some(ParamDiagKind::BadReference);return Err(crate::Error::new(format!("missing channel path {path:?}")))};
            let valid=match kind {"ch"=>v.is_number()||v.is_boolean(),"chs"=>v.is_string(),"chv"=>v.as_array().is_some_and(|xs|xs.len()==3&&xs.iter().all(Value::is_number)),_=>false};
            if !valid {failure=Some(ParamDiagKind::BadReference);return Err(crate::Error::new(format!("wrong channel reference type at {path:?}")))}
            match self.resolve(&target) {
                Ok(v)=>{if self.result.time_dependent.contains(&target.node){self.result.time_dependent.insert(a.node.clone());}Ok(v)},
                Err(k)=>{failure=Some(k);Err(crate::Error::new(format!("channel dependency failed at {path:?}")))}
            }
        }).map_err(|e|(failure.unwrap_or(ParamDiagKind::BadExpression),e.to_string()))?;
        Ok(value)
    }
    fn parse_path(&self,owner:&Address,path:&str)->Option<Address>{
        let mut parts:Vec<&str>=if path.starts_with('/') {Vec::new()}else{owner.node.split('/').collect()};
        let segments:Vec<&str>=path.split('/').collect();
        for segment in &segments[..segments.len().checked_sub(1)?] {
            match *segment {""|"."=>{},".."=>{parts.pop()?;},s=>parts.push(s)}
        }
        let leaf=*segments.last()?;
        let (param,component)=split_component(leaf);
        if param.is_empty()||parts.is_empty(){return None}
        let target=Address{node:parts.join("/"),param:param.into(),component};
        let def=self.graph.node(&target.node)?;
        let spec=nodes::get(&def.node_type)?;
        (valid_address(spec.params,&target.label()) || self.graph.promoted_param(&target.node,&target.param).is_some()
            || self.graph.controller_param(&target.node,&target.param).is_some_and(|p| component.is_none() || p["kind"] == "vector")).then_some(target)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn resolve(v:Value)->ResolvedChannels {resolve_graph(&Graph::from_value(&v).unwrap())}
    #[test]
    fn channel_relative_absolute_components_and_cross_stream_chains() {
        let r=resolve(json!({"nodes":{
            "driver":{"type":"box","params":{"size":[2,4,6]}},
            "text":{"type":"output","params":{"object":"Cube"}},
            "middle":{"type":"box","params":{"size":[1,1,1]},"channels":{"size.y":"ch('../driver/size.y') + @Frame","size.z":"ch('/driver/size.z')"}},
            "target":{"type":"transform","params":{"translate":[0,0,0]},"channels":{"translate.x":"ch('../driver/size.x')","translate.y":"ch('../middle/size.y') * 2"}},
            "subnet/inner":{"type":"box","params":{"size":[1,1,1]},"channels":{"size.x":"ch('../../driver/size.x')"}},
            "vector":{"type":"box","params":{"size":[1,1,1]},"channels":{"size":"chv('../driver/size')","size.y":"ch('./size.x') * 3"}},
            "self":{"type":"box","params":{"size":[3,1,1]},"channels":{"size.y":"ch('./size.x') * 2","size.z":"ch('../driver/consolidate')"}},
            "string":{"type":"output","params":{"object":"fallback"},"channels":{"object":"chs('../text/object')"}}
        }}));
        assert_eq!(r.params["target"]["translate"][0].as_f64(),Some(2.));
        assert_eq!(r.params["target"]["translate"][1].as_f64(),Some(10.));
        assert_eq!(r.params["middle"]["size"].as_array().unwrap().iter().map(|v|v.as_f64().unwrap()).collect::<Vec<_>>(),vec![1.,5.,6.]);
        assert_eq!(r.params["vector"]["size"].as_array().unwrap().iter().map(|v|v.as_f64().unwrap()).collect::<Vec<_>>(),vec![2.,6.,6.]);
        assert_eq!(r.params["subnet/inner"]["size"][0].as_f64(),Some(2.));
        assert_eq!(r.params["self"]["size"].as_array().unwrap().iter().map(|v|v.as_f64().unwrap()).collect::<Vec<_>>(),vec![3.,6.,1.]);
        assert_eq!(r.params["string"]["object"],json!("Cube"));
        assert!(r.time_dependent.contains("target"));
        assert!(r.diagnostics.is_empty(),"{:?}",r.diagnostics);
    }
    #[test]
    fn channel_cycles_and_missing_paths_keep_literals() {
        let r=resolve(json!({"nodes":{
            "a":{"type":"box","params":{"size":[3,4,5]},"channels":{"size.x":"ch('../b/size.x')","size.y":"ch('../gone/size.y')"}},
            "b":{"type":"box","params":{"size":[6,7,8]},"channels":{"size.x":"ch('../a/size.x')"}}
        }}));
        assert_eq!(r.params["a"]["size"][0],json!(3));
        assert_eq!(r.params["a"]["size"][1],json!(4));
        assert_eq!(r.params["b"]["size"][0],json!(6));
        assert!(r.diagnostics.iter().any(|d|d.kind==ParamDiagKind::BadReference&&d.node=="a"&&d.param=="size.y"));
        assert!(r.diagnostics.iter().any(|d|d.kind==ParamDiagKind::CycleDetected&&d.node=="a"&&d.param=="size.x"));
        assert!(r.diagnostics.iter().any(|d|d.kind==ParamDiagKind::CycleDetected&&d.node=="b"&&d.param=="size.x"));
    }
    #[test]
    fn previously_resolved_broken_link_does_not_drive_a_dependent() {
        // JSON object order resolves "a" first. Its failed channel is memoized
        // before "z" reads it; "z" must still get a failed dependency.
        let r=resolve(json!({"nodes":{
            "a":{"type":"box","params":{"size":[3,1,1]},"channels":{"size.x":"ch('../missing/size.x')"}},
            "z":{"type":"box","params":{"size":[9,1,1]},"channels":{"size.x":"ch('../a/size.x') + 1"}}
        }}));
        assert_eq!(r.params["a"]["size"][0],json!(3));
        assert_eq!(r.params["z"]["size"][0],json!(9));
        assert!(r.diagnostics.iter().any(|d|d.node=="a"&&d.param=="size.x"&&d.kind==ParamDiagKind::BadReference));
        assert!(r.diagnostics.iter().any(|d|d.node=="z"&&d.param=="size.x"&&d.kind==ParamDiagKind::BadReference));
    }
    #[test]
    fn channel_time_aliases_and_bad_expression() {
        let graph=Graph::from_value(&json!({"nodes":{
            "a":{"type":"box","params":{"size":[2,3,4]},"channels":{"size.x":"$F + @Time + $T + @TimeInc","size.y":"sqrt(-1)"}}
        }})).unwrap();
        let r=crate::timectx::at(Some(25.),Some(24.),||resolve_graph(&graph));
        assert!((r.params["a"]["size"][0].as_f64().unwrap()-(25.+1.+1.+1./24.)).abs()<1e-12,"{:?} {:?}",r.params["a"],r.diagnostics);
        assert_eq!(r.params["a"]["size"][1],json!(3));
        assert!(r.diagnostics.iter().any(|d|d.kind==ParamDiagKind::BadExpression&&d.param=="size.y"));
    }
}
