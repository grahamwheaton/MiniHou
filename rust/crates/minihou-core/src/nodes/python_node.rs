//! Explicit Python-runtime boundary. This core does not embed or launch Python.
//! `minihou-py` is an extension exposing Rust to Python, not an embedded interpreter.
use super::{InputSpec,Inputs,NodeSpec};
use crate::{error::{Error,Result},geometry::Geometry,params::Params};

const DEFAULT_CODE:&str=r#"# geo, input1, input2, input3: Geometry (or None when nothing is wired). They are read-only: copy before changing.
# Variables from the parameters: a b c d (floats)  i j (ints)  v (x, y, z)  s (text)  flag (bool), and any `name = value`
# lines of the Variables field. Frame, Time, TimeInc give the time (tick "animated" so the node recooks on every frame).
# Set `out` to the result Geometry; leave it out to pass `geo` through.
P = geo.P.astype(np.float64)
P[:, 2] += a * np.sin(P[:, 0] * b)
out = geo.copy()
out.point["P"] = P.astype(np.float32)
"#;

pub const PYTHON:NodeSpec=NodeSpec{name:"python",inputs:&[
    InputSpec{name:"geo",multi:false,optional:true},InputSpec{name:"input1",multi:false,optional:true},
    InputSpec{name:"input2",multi:false,optional:true},InputSpec{name:"input3",multi:false,optional:true}],
    params:crate::params!("code":DEFAULT_CODE;"a":0.;"b":1.;"c":0.;"d":0.;"i":0;"j":0;
        "v":[0.,0.,0.];"s":"";"flag":false;"variables":"";"animated":false),cook:cook};

fn cook(_inputs:&Inputs,_p:&Params)->Result<Geometry>{
    Err(Error::new("python node is unsupported without a Python runtime bridge in the Rust engine; open this graph in the Python engine with Python nodes explicitly enabled to run trusted code"))
}

#[cfg(test)] mod tests {use super::*;
    #[test] fn stub_preserves_python_contract_and_fails_clearly(){
        let params=Params::resolve(PYTHON.params,None).unwrap();
        assert_eq!(params.s("code").unwrap(),DEFAULT_CODE);
        assert_eq!(PYTHON.inputs.iter().map(|v|v.name).collect::<Vec<_>>(),["geo","input1","input2","input3"]);
        assert!(cook(&[vec![],vec![],vec![],vec![]],&params).unwrap_err().to_string().contains("unsupported without a Python runtime"));
    }
}
