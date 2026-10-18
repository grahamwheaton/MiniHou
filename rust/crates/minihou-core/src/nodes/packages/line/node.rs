use crate::{nodes::{Inputs, NodeSpec}, error::{Error, Result}, geometry::Geometry, params::Params};
pub(super) const NODE: NodeSpec = NodeSpec {
    name: "line", inputs: &[],
    params: crate::params!("origin": [0.0, 0.0, 0.0]; "direction": [0.0, 0.0, 1.0]; "length": 1.0; "points": 10;),
    cook: line,
};

fn line(_inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let n = p.i("points")?;
    if !(2..=1_000_000).contains(&n) {
        return Err(Error::new("line points must be between 2 and 1000000"));
    }
    let n = n as usize;
    let origin = p.v3("origin")?;
    let mut d = p.v3("direction")?;
    let len = d.iter().map(|x| x * x).sum::<f64>().sqrt().max(1e-12);
    for x in &mut d { *x /= len; }
    let mut pos = Vec::with_capacity(n * 3);
    for t in crate::nodes::linspace(0.0, p.f("length")?, n) {
        pos.extend((0..3).map(|k| (origin[k] + t * d[k]) as f32));
    }
    Geometry::from_arrays(pos, (0..n as i32).collect(), vec![0, n as i32], Some(vec![false]))
}


#[cfg(test)]
#[path = "tests.rs"]
mod tests;
