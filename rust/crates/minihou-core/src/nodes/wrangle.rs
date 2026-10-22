//! Deterministic, bounded VEX-style attribute expressions.
use super::{
    scalar_expr::{Expr, Number},
    InputSpec, Inputs, NodeSpec,
};
use crate::{params::Params, Attr, AttrData, Error, Geometry, Result};
use std::sync::Arc;
pub const WRANGLE: NodeSpec = NodeSpec { outputs: &["geometry"], cook_multi: None, cook_sidecar: None,
    name: "wrangle",
    inputs: &[
        InputSpec {
            name: "geo",
            multi: false,
            optional: false,
        },
        InputSpec {
            name: "opinput1",
            multi: false,
            optional: true,
        },
        InputSpec {
            name: "opinput2",
            multi: false,
            optional: true,
        },
        InputSpec {
            name: "opinput3",
            multi: false,
            optional: true,
        },
    ],
    params: crate::params!("snippet":"@P.z += sin(@P.x)";"run_over":"points";"group":""),
    cook: wrangle,
};

#[derive(Clone, Copy)]
enum Domain {
    Point,
    Vertex,
    Prim,
    Detail,
}
impl Domain {
    fn parse(s: &str) -> Result<Self> {
        match s {
            "points" => Ok(Self::Point),
            "vertices" => Ok(Self::Vertex),
            "primitives" => Ok(Self::Prim),
            "detail" => Ok(Self::Detail),
            _ => Err(Error::new(format!("unsupported Run Over: {s}"))),
        }
    }
    fn count(self, g: &Geometry) -> usize {
        match self {
            Self::Point => g.npoints,
            Self::Vertex => g.nvertices(),
            Self::Prim => g.nprims(),
            Self::Detail => 1,
        }
    }
    fn label(self) -> &'static str {
        match self {
            Self::Point => "point",
            Self::Vertex => "vertex",
            Self::Prim => "primitive",
            Self::Detail => "detail",
        }
    }
    fn table<'a>(self, g: &'a Geometry) -> Option<&'a crate::geometry::AttrTable> {
        match self {
            Self::Point => Some(&g.point),
            Self::Vertex => Some(&g.vertex),
            Self::Prim => Some(&g.prim),
            Self::Detail => None,
        }
    }
    fn table_mut<'a>(self, g: &'a mut Geometry) -> Option<&'a mut crate::geometry::AttrTable> {
        match self {
            Self::Point => Some(&mut g.point),
            Self::Vertex => Some(&mut g.vertex),
            Self::Prim => Some(&mut g.prim),
            Self::Detail => None,
        }
    }
}
fn reference(s: &str) -> Result<(&str, Option<usize>)> {
    let t = s
        .strip_prefix('@')
        .ok_or_else(|| Error::new("attribute references start with @"))?;
    let (n, c) = match t.split_once('.') {
        Some((n, c)) => (
            n,
            Some(match c {
                "x" | "r" => 0,
                "y" | "g" => 1,
                "z" | "b" => 2,
                "w" | "a" => 3,
                _ => return Err(Error::new("unsupported attribute component")),
            }),
        ),
        None => (t, None),
    };
    if n.is_empty()
        || !n.as_bytes()[0].is_ascii_alphabetic() && n.as_bytes()[0] != b'_'
        || !n.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_')
    {
        return Err(Error::new("invalid attribute name"));
    }
    Ok((n, c))
}
fn prim_of_vertex(g: &Geometry, v: usize) -> usize {
    g.topology
        .prim_offsets
        .partition_point(|&x| x as usize <= v)
        .saturating_sub(1)
        .min(g.nprims().saturating_sub(1))
}
fn row(a: &Attr, i: usize) -> Vec<f64> {
    match &a.data {
        AttrData::F32(v) => v[i * a.width..(i + 1) * a.width]
            .iter()
            .map(|&x| x as f64)
            .collect(),
        AttrData::I32(v) => v[i * a.width..(i + 1) * a.width]
            .iter()
            .map(|&x| x as f64)
            .collect(),
    }
}
fn from_table(g: &Geometry, d: Domain, n: &str, i: usize) -> Option<Vec<f64>> {
    d.table(g)?.get(n).map(|a| row(a, i))
}
fn detail(g: &Geometry, n: &str) -> Result<Vec<f64>> {
    let v = g
        .detail
        .get(n)
        .ok_or_else(|| Error::new(format!("unknown attribute @{n}")))?;
    if let Some(a) = v.as_array() {
        if a.is_empty() || a.len() > 4 {
            return Err(Error::new("detail vector needs 1 to 4 components"));
        }
        a.iter()
            .map(|x| {
                x.as_f64()
                    .ok_or_else(|| Error::new("non-numeric detail attribute"))
            })
            .collect()
    } else {
        v.as_f64()
            .map(|x| vec![x])
            .ok_or_else(|| Error::new("non-numeric detail attribute"))
    }
}
fn read(g: &Geometry, d: Domain, i: usize, text: &str, inputs: &Inputs) -> Result<Vec<f64>> {
    if text == "PI" {
        return Ok(vec![std::f64::consts::PI]);
    }
    if text == "E" {
        return Ok(vec![std::f64::consts::E]);
    }
    let (n, c) = reference(text)?;
    if let Some(rest) = n.strip_prefix("opinput") {
        if let Some((slot, attribute)) = rest.split_once('_') {
            if let Ok(slot) = slot.parse::<usize>() {
                if (1..=3).contains(&slot) && !attribute.is_empty() {
                    let other = WRANGLE.one(inputs, slot)?;
                    let value = if i < d.count(&other) {
                        read(&other, d, i, &format!("@{attribute}"), inputs)?
                    } else {
                        // Houdini's virtual binding uses zero when the other
                        // input has no corresponding element.
                        let width = if d.count(&other) > 0 {
                            read(&other, d, 0, &format!("@{attribute}"), inputs)?.len()
                        } else {
                            d.table(&other)
                                .and_then(|t| t.get(attribute))
                                .or_else(|| other.point.get(attribute))
                                .or_else(|| other.prim.get(attribute))
                                .or_else(|| other.vertex.get(attribute))
                                .map(|a| a.width)
                                .ok_or_else(|| Error::new(format!("input {slot} has no attribute @{attribute}")))?
                        };
                        vec![0.; width]
                    };
                    return if let Some(k) = c {
                        value.get(k).copied().map(|x| vec![x])
                            .ok_or_else(|| Error::new(format!("component outside @{n}")))
                    } else {
                        Ok(value)
                    };
                }
            }
        }
    }
    let special = match n {
        "Frame" => Some(crate::timectx::frame() as f64),
        "Time" => Some(crate::timectx::seconds()),
        "TimeInc" => Some(crate::timectx::time_inc()),
        "numpt" => Some(g.npoints as f64),
        "numprim" => Some(g.nprims() as f64),
        "numvtx" => Some(g.nvertices() as f64),
        "ptnum" => match d {
            Domain::Point => Some(i as f64),
            Domain::Vertex => Some(g.topology.vertex_point[i] as f64),
            _ => None,
        },
        "primnum" => match d {
            Domain::Prim => Some(i as f64),
            Domain::Vertex => Some(prim_of_vertex(g, i) as f64),
            _ => None,
        },
        "vtxnum" => match d {
            Domain::Vertex => Some(i as f64),
            _ => None,
        },
        "closed" => match d {
            Domain::Prim => Some(g.topology.is_closed(i) as i32 as f64),
            Domain::Vertex => Some(g.topology.is_closed(prim_of_vertex(g, i)) as i32 as f64),
            _ => None,
        },
        _ => None,
    };
    let value = if let Some(x) = special {
        vec![x]
    } else if let Some(v) = from_table(g, d, n, i) {
        v
    } else if let Domain::Vertex = d {
        from_table(g, Domain::Point, n, g.topology.vertex_point[i] as usize)
            .or_else(|| from_table(g, Domain::Prim, n, prim_of_vertex(g, i)))
            .or_else(|| detail(g, n).ok())
            .ok_or_else(|| Error::new(format!("unknown attribute @{n}")))?
    } else if let Domain::Prim = d {
        let (start, end) = (
            g.topology.prim_offsets[i] as usize,
            g.topology.prim_offsets[i + 1] as usize,
        );
        let source = g
            .point
            .get(n)
            .map(|a| (a, true))
            .or_else(|| g.vertex.get(n).map(|a| (a, false)));
        if let Some((a, point)) = source {
            let mut sum = vec![0.; a.width];
            for v in start..end {
                let r = if point {
                    g.topology.vertex_point[v] as usize
                } else {
                    v
                };
                for (x, y) in sum.iter_mut().zip(row(a, r)) {
                    *x += y;
                }
            }
            for x in &mut sum {
                *x /= (end - start).max(1) as f64;
            }
            sum
        } else {
            detail(g, n)?
        }
    } else {
        detail(g, n)?
    };
    if let Some(k) = c {
        value
            .get(k)
            .map(|&x| vec![x])
            .ok_or_else(|| Error::new(format!("component outside @{n}")))
    } else {
        Ok(value)
    }
}
fn split_top(s: &str, delim: char) -> Option<Vec<&str>> {
    let mut depth = 0i32;
    let mut start = 0;
    let mut parts = Vec::new();
    for (i, c) in s.char_indices() {
        match c {
            '(' | '{' => depth += 1,
            ')' | '}' => depth -= 1,
            _ => {}
        }
        if depth == 0 && c == delim {
            parts.push(s[start..i].trim());
            start = i + 1;
        }
    }
    if parts.is_empty() {
        None
    } else {
        parts.push(s[start..].trim());
        Some(parts)
    }
}
fn eval(s: &str, g: &Geometry, d: Domain, i: usize, inputs: &Inputs) -> Result<Vec<f64>> {
    let s = s.trim();
    if s == "rand()" {
        return eval(&format!("rand({i})"), g, d, i, inputs);
    }
    if s.starts_with("noise(") && s.ends_with(')') {
        let args = &s[6..s.len() - 1];
        if let Ok(v) = eval(args, g, d, i, inputs) {
            if v.len() == 3 {
                return Ok(vec![crate::noise::fbm3_one(
                    [v[0] as f32, v[1] as f32, v[2] as f32],
                    0,
                    4,
                    2.,
                    0.5,
                ) as f64]);
            }
        }
    }
    let vec_inner = if s.starts_with('{') && s.ends_with('}') {
        Some(&s[1..s.len() - 1])
    } else if s.starts_with("set(") && s.ends_with(')') {
        Some(&s[4..s.len() - 1])
    } else {
        None
    };
    if let Some(inner) = vec_inner {
        let parts =
            split_top(inner, ',').ok_or_else(|| Error::new("vector literal needs components"))?;
        if !(2..=4).contains(&parts.len()) {
            return Err(Error::new("vector literal needs 2 to 4 components"));
        }
        return parts
            .into_iter()
            .map(|p| {
                let v = eval(p, g, d, i, inputs)?;
                if v.len() != 1 {
                    Err(Error::new("vector component must be scalar"))
                } else {
                    Ok(v[0])
                }
            })
            .collect();
    }
    if s.starts_with('@') && reference(s).is_ok() {
        return read(g, d, i, s, inputs);
    }
    for op in ['+', '-', '*', '/'] {
        if let Some(parts) = split_top(s, op) {
            if parts.len() == 2 && !parts[0].is_empty() && !parts[1].is_empty() {
                let a = eval(parts[0], g, d, i, inputs)?;
                let b = eval(parts[1], g, d, i, inputs)?;
                if a.len() > 1 || b.len() > 1 {
                    let w = a.len().max(b.len());
                    if a.len() != 1 && a.len() != w || b.len() != 1 && b.len() != w {
                        return Err(Error::new("vector widths differ"));
                    }
                    return (0..w)
                        .map(|k| {
                            let x = a[if a.len() == 1 { 0 } else { k }];
                            let y = b[if b.len() == 1 { 0 } else { k }];
                            Ok(match op {
                                '+' => x + y,
                                '-' => x - y,
                                '*' => x * y,
                                _ => x / y,
                            })
                        })
                        .collect();
                }
            }
        }
    }
    let expr = Expr::parse(s)?;
    let result = expr.eval(&mut |name| {
        let v = read(g, d, i, name, inputs)?;
        if v.len() != 1 {
            return Err(Error::new(format!("select a component of {name}")));
        }
        Ok(Number::array(v[0] as f32))
    })?;
    Ok(vec![result.value])
}
fn mask(g: &Geometry, d: Domain, pattern: &str) -> Result<Vec<bool>> {
    let n = d.count(g);
    let s = pattern.trim();
    if s.is_empty() {
        return Ok(vec![true; n]);
    }
    if let Domain::Point = d {
        return super::groups::point_mask(g, s);
    }
    if s == "*" {
        return Ok(vec![true; n]);
    }
    let mut out = vec![false; n];
    for token in s
        .split(|c: char| c == ',' || c.is_whitespace())
        .filter(|x| !x.is_empty())
    {
        let exclude = token.starts_with('^');
        let name = token.trim_start_matches('^');
        let range = if name.as_bytes().first().is_some_and(u8::is_ascii_digit) {
            let (a, b) = name.split_once('-').unwrap_or((name, name));
            Some((
                a.parse::<usize>()
                    .map_err(|_| Error::new("invalid group range"))?,
                b.parse::<usize>()
                    .map_err(|_| Error::new("invalid group range"))?,
            ))
        } else {
            None
        };
        let group = if range.is_none() {
            let key = format!("group_{name}");
            let table = if let Domain::Vertex = d {
                &g.prim
            } else {
                d.table(g).unwrap_or(&g.prim)
            };
            Some(
                table
                    .get(&key)
                    .ok_or_else(|| Error::new(format!("no group {name:?}")))?,
            )
        } else {
            None
        };
        for (i, dst) in out.iter_mut().enumerate() {
            let r = if let Domain::Vertex = d {
                prim_of_vertex(g, i)
            } else {
                i
            };
            let hit = if let Some((a, b)) = range {
                (a..=b).contains(&i)
            } else {
                row(group.unwrap(), r)[0] != 0.
            };
            if exclude {
                *dst &= !hit
            } else {
                *dst |= hit
            }
        }
    }
    Ok(out)
}
fn assign(g: &mut Geometry, d: Domain, i: usize, left: &str, op: &str, right: &str, inputs: &Inputs) -> Result<()> {
    let (prefix, target) = if left.len() > 1 && left.as_bytes()[1] == b'@' {
        (&left[..1], &left[1..])
    } else {
        ("", left)
    };
    let (name, component) = reference(target)?;
    if [
        "ptnum", "primnum", "vtxnum", "numpt", "numprim", "numvtx", "closed", "Frame", "Time",
        "TimeInc",
    ]
    .contains(&name)
        || name.starts_with("opinput")
        || name.starts_with("near")
    {
        return Err(Error::new("cannot write a read-only attribute"));
    }
    if name == "P" && !matches!(d, Domain::Point) {
        return Err(Error::new("@P can only be written over points"));
    }
    let rhs = eval(right, g, d, i, inputs)?;
    if rhs.iter().any(|x| !x.is_finite()) {
        return Err(Error::new("expression result is not finite"));
    }
    let old = if let Domain::Detail = d {
        g.detail.get(name).map(|v| {
            if let Some(a) = v.as_array() {
                a.iter().filter_map(|x| x.as_f64()).collect()
            } else {
                vec![v.as_f64().unwrap_or(0.)]
            }
        })
    } else {
        d.table(g).unwrap().get(name).map(|a| row(a, i))
    };
    let width = if let Some(ref v) = old {
        v.len()
    } else {
        match prefix {
            "f" | "i" => 1,
            "u" => 2,
            "v" => 3,
            "p" => 4,
            "" => {
                if component.is_some() {
                    return Err(Error::new(
                        "new component needs a declared vector attribute",
                    ));
                } else if rhs.len() == 1 {
                    return Err(Error::new("new scalar attributes require f@name or i@name"));
                } else {
                    rhs.len()
                }
            }
            _ => return Err(Error::new("unsupported attribute prefix")),
        }
    };
    if let Some(c) = component {
        if c >= width {
            return Err(Error::new(
                "destination component is outside attribute width",
            ));
        }
    }
    if prefix == "f" && width != 1
        || prefix == "v" && width != 3
        || prefix == "u" && width != 2
        || prefix == "p" && width != 4
    {
        return Err(Error::new(
            "declared attribute width differs from existing attribute",
        ));
    }
    let expected = if component.is_some() { 1 } else { width };
    if rhs.len() != expected {
        return Err(Error::new(format!(
            "assignment needs {expected} component(s), got {}",
            rhs.len()
        )));
    }
    let integer = d
        .table(g)
        .and_then(|t| t.get(name))
        .map(|a| matches!(a.data, AttrData::I32(_)))
        .unwrap_or(prefix == "i");
    let mut values = old.unwrap_or_else(|| vec![0.; width]);
    for k in 0..expected {
        let c = component.unwrap_or(k);
        let x = values[c];
        let y = rhs[k];
        values[c] = match op {
            "+" => x + y,
            "-" => x - y,
            "*" => x * y,
            "/" => x / y,
            _ => y,
        };
    }
    if values.iter().any(|x| !x.is_finite()) {
        return Err(Error::new("assignment result is not finite"));
    }
    if integer
        && values
            .iter()
            .any(|x| *x < i32::MIN as f64 || *x > i32::MAX as f64)
    {
        return Err(Error::new("integer attribute value out of range"));
    }
    if let Domain::Detail = d {
        g.detail.insert(
            name.into(),
            if width == 1 {
                serde_json::json!(values[0])
            } else {
                serde_json::json!(values)
            },
        );
        return Ok(());
    }
    let n = d.count(g);
    let table = d.table_mut(g).unwrap();
    let attr = table.get(name);
    let updated = if integer {
        let mut data = attr
            .and_then(|a| a.as_i32().map(|x| x.to_vec()))
            .unwrap_or_else(|| vec![0; n * width]);
        for k in 0..width {
            data[i * width + k] = values[k] as i32;
        }
        Attr::i32(width, data)
    } else {
        let mut data = attr
            .and_then(|a| a.as_f32().map(|x| x.to_vec()))
            .unwrap_or_else(|| vec![0.; n * width]);
        for k in 0..width {
            data[i * width + k] = values[k] as f32;
        }
        Attr::f32(width, data)
    };
    table.insert(name.into(), Arc::new(updated));
    Ok(())
}
fn wrangle(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let source = WRANGLE.one(inputs, 0)?;
    let d = Domain::parse(&p.s("run_over")?)?;
    let snippet = p.s("snippet")?;
    let selected = mask(&source, d, &p.s("group")?)?;
    let mut out = (*source).clone();
    for (line, text) in snippet.lines().enumerate() {
        let text = text.split("//").next().unwrap_or("");
        for statement in text.split(';').map(str::trim).filter(|s| !s.is_empty()) {
            let (left, right) = statement
                .split_once('=')
                .ok_or_else(|| Error::new(format!("line {}: expected assignment", line + 1)))?;
            let left = left.trim();
            let (target, op) = match left.as_bytes().last() {
                Some(b'+') => (&left[..left.len() - 1], "+"),
                Some(b'-') => (&left[..left.len() - 1], "-"),
                Some(b'*') => (&left[..left.len() - 1], "*"),
                Some(b'/') => (&left[..left.len() - 1], "/"),
                _ => (left, "="),
            };
            for (i, &choose) in selected.iter().enumerate() {
                if choose {
                    assign(&mut out, d, i, target.trim(), op, right.trim(), inputs).map_err(|e| {
                        Error::new(format!("{} {i}, line {}: {e}", d.label(), line + 1))
                    })?;
                }
            }
        }
    }
    out.validate()?;
    Ok(out)
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn fixture() -> Geometry {
        let mut g = Geometry::from_arrays(
            vec![0., 0., 2., 1., 0., 3., 2., 0., 4.],
            vec![0, 1, 2, 2, 1],
            vec![0, 3, 5],
            None,
        )
        .unwrap();
        g.point
            .insert("weight".into(), Arc::new(Attr::f32(1, vec![1., 2., 3.])));
        g.prim
            .insert("group_first".into(), Arc::new(Attr::i32(1, vec![1, 0])));
        g
    }
    fn cook(g: Geometry, run_over: &str, snippet: &str, group: &str) -> Result<Geometry> {
        let p = Params::resolve(
            WRANGLE.params,
            json!({"snippet":snippet,"run_over":run_over,"group":group}).as_object(),
        )?;
        wrangle(&[vec![Arc::new(g)], vec![], vec![], vec![]], &p)
    }
    #[test]
    fn existing_graph_and_point_attributes() {
        let g = fixture();
        let a = cook(g.clone(), "points", "@P.z += sin(@P.x)", "").unwrap();
        assert!((a.p()[5] - (3. + 1f32.sin())).abs() < 1e-6);
        let b = cook(g, "points", "f@mask = @weight + @ptnum; @weight += 2", "").unwrap();
        assert_eq!(b.point["mask"].as_f32().unwrap(), &[1., 3., 5.]);
        assert_eq!(b.point["weight"].as_f32().unwrap(), &[3., 4., 5.]);
    }
    #[test]
    fn each_run_over_and_groups() {
        let g = fixture();
        let p = cook(g.clone(), "primitives", "f@x = @P.z + @primnum; i@group_copy = @group_first", "first").unwrap();
        assert_eq!(p.prim["x"].as_f32().unwrap(), &[3., 0.]);
        assert_eq!(p.prim["group_copy"].as_i32().unwrap(), &[1, 0]);
        let v = cook(g.clone(), "vertices", "f@x = @ptnum + @primnum; f@w = @weight", "").unwrap();
        assert_eq!(v.vertex["x"].as_f32().unwrap(), &[0., 1., 2., 3., 2.]);
        assert_eq!(v.vertex["w"].as_f32().unwrap(), &[1., 2., 3., 3., 2.]);
        let d = cook(g, "detail", "f@total = @numpt + @numprim", "").unwrap();
        assert_eq!(d.detail["total"], json!(5.));
    }
    #[test]
    fn vectors_functions_and_determinism() {
        let g = fixture();
        let s =
            "v@Cd = set(1, 0.5, @P.z / 4); f@r = rand(@ptnum); f@n = noise(@P.x); @P += {1,0,0}";
        let a = cook(g.clone(), "points", s, "").unwrap();
        let b = cook(g, "points", s, "").unwrap();
        assert_eq!(a.point["Cd"], b.point["Cd"]);
        assert_eq!(a.point["r"], b.point["r"]);
        assert_eq!(a.p()[0], 1.);
    }
    #[test]
    fn errors_name_element_and_line() {
        let e = cook(fixture(), "points", "f@x = 1;\nf@y = @missing", "").unwrap_err();
        assert!(e.0.contains("point 0, line 2"), "{e}");
    }

    #[test]
    fn extra_input_reads_match_element_number_and_zero_pad() {
        let source = fixture();
        let mut other = Geometry::points_only(vec![7., 0., 0., 8., 0., 0.]).unwrap();
        other.point.insert("weight".into(), Arc::new(Attr::f32(1, vec![10., 20.])));
        let p = Params::resolve(WRANGLE.params,
            json!({"run_over": "points", "snippet": "f@copied = @opinput1_weight; v@position2 = @opinput1_P"}).as_object()).unwrap();
        let out = wrangle(&[vec![Arc::new(source.clone())], vec![Arc::new(other)], vec![], vec![]], &p).unwrap();
        assert_eq!(out.point["copied"].as_f32().unwrap(), &[10., 20., 0.]);
        assert_eq!(out.point["position2"].as_f32().unwrap(), &[7., 0., 0., 8., 0., 0., 0., 0., 0.]);

        let err = cook(source, "points", "f@x = @opinput1_weight", "").unwrap_err();
        assert!(err.0.contains("input 'opinput1' is not connected"), "{err}");
    }
}
