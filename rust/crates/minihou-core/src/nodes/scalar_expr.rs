//! Closed scalar expression AST. No host evaluation, indexing, attribute access or arbitrary calls.
use crate::{Error, Result};
#[derive(Clone,Debug)]
pub(crate) enum Expr { Number(f64), String(String), Name(String), Unary(bool,Box<Expr>), Binary(&'static str,Box<Expr>,Box<Expr>), Call(String,Vec<Expr>) }
#[derive(Clone,Copy)]
pub(crate) struct Number { pub value: f64, pub array: bool }
impl Number { pub fn scalar(value:f64)->Self{Self{value,array:false}} pub fn array(value:f32)->Self{Self{value:value as f64,array:true}} }
impl Expr {
    pub fn parse(text:&str)->Result<Self>{
        if text.len()>16384{return Err(Error::new("expression exceeds 16384 bytes"));}
        let mut p=Parser{text:text.as_bytes(),pos:0,depth:0,nodes:0};let e=p.sum()?;p.space();
        if p.pos!=p.text.len(){return Err(Error::new("unsupported expression syntax"));}Ok(e)
    }
    pub fn names(&self,out:&mut Vec<String>){match self{
        Self::Name(s)=>out.push(s.clone()),Self::Unary(_,e)=>e.names(out),Self::Binary(_,a,b)=>{a.names(out);b.names(out);},
        Self::Call(_,args)=>for a in args{a.names(out);},_=>{}
    }}
    pub(crate) fn eval_channel(&self, read:&mut dyn FnMut(&str)->Result<Number>, channel:&mut dyn FnMut(&str,&str)->Result<serde_json::Value>)->Result<serde_json::Value>{
        if let Self::Call(kind,args)=self {
            if ["ch","chs","chv"].contains(&kind.as_str()) {
                let [Self::String(path)] = args.as_slice() else { return Err(Error::new(format!("{kind} expects one quoted path"))); };
                let value=channel(kind,path)?;
                let valid=match kind.as_str(){"ch"=>value.is_number()||value.is_boolean(),"chs"=>value.is_string(),_=>value.as_array().is_some_and(|a|a.len()==3&&a.iter().all(serde_json::Value::is_number))};
                if !valid {return Err(Error::new(format!("{kind} reference has the wrong type")));}
                return Ok(if kind=="ch" {match value {serde_json::Value::Bool(b)=>serde_json::json!(if b{1.0}else{0.0}),v=>v}}else{value});
            }
        }
        let n=self.eval_with(read,channel)?;
        if !n.value.is_finite(){return Err(Error::new("channel expression is not finite"));}
        Ok(serde_json::json!(n.value))
    }
    /// array=true follows float32 NumPy ufunc operations. Scalar weights stay float64.
    pub fn eval(&self,read:&mut dyn FnMut(&str)->Result<Number>)->Result<Number>{
        self.eval_with(read,&mut |_,_|Err(Error::new("channel reference is unavailable here")))
    }
    fn eval_with(&self,read:&mut dyn FnMut(&str)->Result<Number>,channel:&mut dyn FnMut(&str,&str)->Result<serde_json::Value>)->Result<Number>{
        Ok(match self{
            Self::Number(v)=>Number::scalar(*v),Self::Name(n)=>read(n)?,
            Self::String(_)=>return Err(Error::new("string literal is only valid in a channel call")),
            Self::Unary(neg,e)=>{let mut v=e.eval_with(read,channel)?;if *neg{v.value=-v.value;}v},
            Self::Binary(op,a,b)=>{
                let a=a.eval_with(read,channel)?;let b=b.eval_with(read,channel)?;let array=a.array||b.array;
                let value=if array{let x=a.value as f32;let y=b.value as f32;match *op{
                    "+"=>(x+y) as f64,"-"=>(x-y) as f64,"*"=>(x*y) as f64,"/"=>(x/y) as f64,
                    "%"=>{let r=x%y;(if r==0.{0f32.copysign(y)}else if r.is_sign_negative()!=y.is_sign_negative(){r+y}else{r}) as f64},"**"=>x.powf(y) as f64,_=>unreachable!()
                }}else{let x=a.value;let y=b.value;match *op{
                    "+"=>x+y,"-"=>x-y,"*"=>x*y,"/"=>{if y==0.{return Err(Error::new("division by zero"));}x/y},
                    "%"=>{if y==0.{return Err(Error::new("modulo by zero"));}let r=x%y;if r==0.{0f64.copysign(y)}else if r.is_sign_negative()!=y.is_sign_negative(){r+y}else{r}},"**"=>x.powf(y),_=>unreachable!()
                }};Number{value,array}
            },
            Self::Call(name,args)=>{
                if name=="ch" {let [Self::String(path)]=args.as_slice() else{return Err(Error::new("ch expects one quoted path"));};
                    let v=channel(name,path)?;return match v {serde_json::Value::Bool(b)=>Ok(Number::scalar(if b{1.}else{0.})),serde_json::Value::Number(n)=>Ok(Number::scalar(n.as_f64().unwrap())),_=>Err(Error::new("ch reference must be numeric or bool"))};}
                let a=args.iter().map(|e|e.eval_with(read,channel)).collect::<Result<Vec<_>>>()?;
                let array=a.iter().any(|v|v.array);let v:Vec<f64>=a.iter().map(|v|v.value).collect();
                let value=match (name.as_str(),v.as_slice()){
                    ("sin",[x])=>if array{(*x as f32).sin() as f64}else{x.sin()},
                    ("cos",[x])=>if array{(*x as f32).cos() as f64}else{x.cos()},
                    ("atan2",[y,x])=>if array{(*y as f32).atan2(*x as f32) as f64}else{y.atan2(*x)},
                    ("abs",[x])=>x.abs(),("round",[x])=>x.round(),("sqrt",[x])=>if array{(*x as f32).sqrt() as f64}else{x.sqrt()},
                    ("min",xs) if xs.len()>=2=>xs.iter().copied().reduce(|x,y|if y<x{y}else{x}).unwrap(),
                    ("max",xs) if xs.len()>=2=>xs.iter().copied().reduce(|x,y|if y>x{y}else{x}).unwrap(),
                    ("clamp",[x,lo,hi])=>lo.max(hi.min(*x)),
                    ("rand",[x])=>{let mut h=(*x as f32).to_bits().wrapping_add(0x9e3779b9);h^=h>>16;h=h.wrapping_mul(0x7feb352d);h^=h>>15;h=h.wrapping_mul(0x846ca68b);h^=h>>16;(h as f64)/(u32::MAX as f64)},
                    ("noise",[x])=>crate::noise::fbm3_one([*x as f32,0.,0.],0,4,2.,0.5) as f64,
                    ("smoothstep",[lo,hi,x])=>{let t=if hi==lo{0.}else{((x-lo)/(hi-lo)).clamp(0.,1.)};t*t*(3.-2.*t)},
                    ("fit",[x,a,b,c,d])=>c+(d-c)*if b==a{0.}else{(x-a)/(b-a)},
                    _=>return Err(Error::new(format!("unsupported function or argument count: {name}")))
                };Number{value:if array{value as f32 as f64}else{value},array}
            }
        })
    }
}
struct Parser<'a>{text:&'a [u8],pos:usize,depth:usize,nodes:usize}
impl Parser<'_>{
    fn space(&mut self){while self.text.get(self.pos).is_some_and(u8::is_ascii_whitespace){self.pos+=1;}}
    fn take(&mut self,s:&[u8])->bool{self.space();if self.text[self.pos..].starts_with(s){self.pos+=s.len();true}else{false}}
    fn sum(&mut self)->Result<Expr>{let mut e=self.product()?;loop{let op=if self.take(b"+"){"+"}else if self.take(b"-"){"-"}else{break;};e=Expr::Binary(op,Box::new(e),Box::new(self.product()?));}Ok(e)}
    fn product(&mut self)->Result<Expr>{let mut e=self.unary()?;loop{self.space();if self.text[self.pos..].starts_with(b"**"){break;}
        let op=if self.take(b"*"){"*"}else if self.take(b"/"){"/"}else if self.take(b"%"){"%"}else{break;};e=Expr::Binary(op,Box::new(e),Box::new(self.unary()?));}Ok(e)}
    fn unary(&mut self)->Result<Expr>{
        self.nodes+=1;if self.nodes>256{return Err(Error::new("expression exceeds 256 terms"));}
        self.depth+=1;if self.depth>64{return Err(Error::new("expression nesting exceeds 64"));}
        let e=if self.take(b"+"){Expr::Unary(false,Box::new(self.unary()?))}else if self.take(b"-"){Expr::Unary(true,Box::new(self.unary()?))}else{
            let a=self.atom()?;if self.take(b"**"){Expr::Binary("**",Box::new(a),Box::new(self.unary()?))}else{a}
        };self.depth-=1;Ok(e)
    }
    fn atom(&mut self)->Result<Expr>{
        if self.take(b"("){let e=self.sum()?;if !self.take(b")"){return Err(Error::new("expected ')'"));}return Ok(e);}
        self.space();let start=self.pos;
        if self.text.get(start)==Some(&b'\'') {
            self.pos+=1;let mut bytes=Vec::new();
            while self.pos<self.text.len() {
                let c=self.text[self.pos];self.pos+=1;
                if c==b'\'' {return Ok(Expr::String(String::from_utf8(bytes).map_err(|_|Error::new("invalid string literal"))?));}
                if c==b'\\' {let Some(&next)=self.text.get(self.pos) else{return Err(Error::new("unterminated string literal"));};self.pos+=1;bytes.push(next);}
                else {bytes.push(c);}
            }
            return Err(Error::new("unterminated string literal"));
        }
        if self.text.get(start)==Some(&b'"') {self.pos+=1;while self.pos<self.text.len(){if self.text[self.pos]==b'\\'{self.pos+=2;}else if self.text[self.pos]==b'"'{self.pos+=1;let s=serde_json::from_slice(&self.text[start..self.pos]).map_err(|_|Error::new("invalid string literal"))?;return Ok(Expr::String(s));}else{self.pos+=1;}}return Err(Error::new("unterminated string literal"));}
        if self.text.get(start).is_some_and(|c|c.is_ascii_digit()||*c==b'.'){
            while self.text.get(self.pos).is_some_and(|c|c.is_ascii_digit()||*c==b'.'){self.pos+=1;}
            if self.text.get(self.pos).is_some_and(|c|*c==b'e'||*c==b'E'){
                self.pos+=1;if self.text.get(self.pos).is_some_and(|c|*c==b'+'||*c==b'-'){self.pos+=1;}
                while self.text.get(self.pos).is_some_and(u8::is_ascii_digit){self.pos+=1;}
            }
            let s=std::str::from_utf8(&self.text[start..self.pos]).unwrap();
            return Ok(Expr::Number(s.parse().map_err(|_|Error::new("invalid number"))?));
        }
        let attr=self.take(b"@");let dollar=if attr{false}else{self.take(b"$")};let start=self.pos;
        if !self.text.get(start).is_some_and(|c|c.is_ascii_alphabetic()||*c==b'_'){return Err(Error::new("expected a number, name or expression"));}
        while self.text.get(self.pos).is_some_and(|c|c.is_ascii_alphanumeric()||*c==b'_'){self.pos+=1;}
        if attr&&self.text.get(self.pos)==Some(&b'.'){self.pos+=1;while self.text.get(self.pos).is_some_and(u8::is_ascii_alphabetic){self.pos+=1;}}
        let name=std::str::from_utf8(&self.text[start..self.pos]).unwrap().to_owned();
        if !attr&&self.take(b"("){
            if !["sin","cos","atan2","abs","round","sqrt","min","max","clamp","smoothstep","fit","rand","noise","ch","chs","chv"].contains(&name.as_str()){return Err(Error::new(format!("unsupported function {name}")));}
            let mut args=Vec::new();if !self.take(b")"){loop{args.push(self.sum()?);if self.take(b")"){break;}if !self.take(b","){return Err(Error::new("expected ',' or ')'"));}}}
            Ok(Expr::Call(name,args))
        }else{Ok(Expr::Name(if attr{format!("@{name}")}else if dollar{format!("${name}")}else{name}))}
    }
}

#[cfg(test)]mod tests{
    use super::*;
    #[test]fn precedence_and_rejections(){for (s,v) in [("-2**2",-4.),("2**-2",0.25),("1+2*3",7.),("smoothstep(0,1,0.5)",0.5)]{assert_eq!(Expr::parse(s).unwrap().eval(&mut |_|Err(Error::new("name"))).unwrap().value,v);}
        for s in ["__import__('os')","a[0]","x.y","lambda:1"]{assert!(Expr::parse(s).is_err());}
    }
}
