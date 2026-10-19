//! The `python` node: runs the snippet in an external Python through [`crate::python_bridge`], only when the cook carries a
//! [`crate::python_bridge::PythonGate`] that is switched on and this exact snippet was confirmed (see docs/RUST_PYTHON_NODE.md).
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

pub const PYTHON:NodeSpec=NodeSpec{ outputs: &["geometry"], cook_multi: None,name:"python",inputs:&[
    InputSpec{name:"geo",multi:false,optional:true},InputSpec{name:"input1",multi:false,optional:true},
    InputSpec{name:"input2",multi:false,optional:true},InputSpec{name:"input3",multi:false,optional:true}],
    params:crate::params!("code":DEFAULT_CODE;"a":0.;"b":1.;"c":0.;"d":0.;"i":0;"j":0;
        "v":[0.,0.,0.];"s":"";"flag":false;"variables":"";"animated":false),cook:cook};

fn cook(inputs:&Inputs,p:&Params)->Result<Geometry>{
    use crate::python_bridge::{self as bridge,Decision};
    let code=p.s("code")?;
    let Some(gate)=bridge::current_gate() else {return Err(Error::new(bridge::DISABLED_MESSAGE))};
    match gate.check(&code) {
        Decision::Disabled=>Err(Error::new(bridge::DISABLED_MESSAGE)),
        Decision::NeedsConfirmation=>Err(Error::new(format!("{} (snippet {})",bridge::CONFIRM_MESSAGE,bridge::code_hash(&code)))),
        Decision::Skipped=>Err(Error::new(bridge::SKIPPED_MESSAGE)),
        Decision::Run=>{
            let slot=|k:usize|inputs.get(k).and_then(|v|v.first()).map(|g|&**g);
            bridge::run(&gate,[slot(0),slot(1),slot(2),slot(3)],p)
        }
    }
}

#[cfg(test)] mod tests {use super::*;
    #[test] fn denied_by_default_preserves_python_contract(){
        let params=Params::resolve(PYTHON.params,None).unwrap();
        assert_eq!(params.s("code").unwrap(),DEFAULT_CODE);
        assert_eq!(PYTHON.inputs.iter().map(|v|v.name).collect::<Vec<_>>(),["geo","input1","input2","input3"]);
        assert_eq!(cook(&[vec![],vec![],vec![],vec![]],&params).unwrap_err().to_string(),crate::python_bridge::DISABLED_MESSAGE);
    }
}
