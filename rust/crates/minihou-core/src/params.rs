//! Node parameters, read from graph JSON the way `NodeType.coerce_params` reads them (numbers may be ints or floats; a vec3
//! is a list of three numbers). Every node declares all of its Python parameters with their defaults: the parity tests
//! compare these with `REGISTRY.get(type).coerce_params({})`, so the two can not drift apart silently.

use serde_json::{Map, Value};

use crate::error::{Error, Result};

/// One parameter: its name, its Python default and whether this backend implements it. A parameter that is not implemented
/// is refused when a graph sets it to anything but the default (never silently ignored).
pub struct ParamSpec {
    pub name: &'static str,
    pub default: fn() -> Value,
    pub supported: bool,
}

pub struct Params {
    values: Map<String, Value>,
}

impl Params {
    pub fn resolve_node(node_type: &str, specs: &[ParamSpec], given: Option<&Map<String, Value>>) -> Result<Params> {
        if node_type == "controller" { return Ok(Params { values: given.cloned().unwrap_or_default() }); }
        if node_type == "split" {
            if let Some(g) = given.filter(|g| g.contains_key("extract_elements")) {
                // Older graphs stored this checkbox; the group domain now determines extraction.
                let mut g = g.clone();
                g.remove("extract_elements");
                return Self::resolve(specs, Some(&g));
            }
        }
        if node_type == "boundary_select" {
            if let Some(g) = given.filter(|g| g.contains_key("output_group")) {
                // Legacy alias: the removed "output_group" feeds "output_group_name" when that is still at its default.
                let mut g = g.clone();
                let legacy = g.remove("output_group").and_then(|v| v.as_str().map(|s| s.trim().to_string())).unwrap_or_default();
                let at_default = g.get("output_group_name").and_then(|v| v.as_str()).map_or(true, |n| n.trim().is_empty() || n == "boundary");
                if !legacy.is_empty() && at_default {
                    g.insert("output_group_name".into(), Value::String(legacy));
                }
                return Self::resolve(specs, Some(&g));
            }
        }
        if node_type == "switch" {
            if let Some(g) = given.filter(|g| g.contains_key("select")) {
                // Legacy: the removed "select" slider is ignored so older graphs still load.
                let mut g = g.clone();
                g.remove("select");
                return Self::resolve(specs, Some(&g));
            }
        }
        Self::resolve(specs, given)
    }
    /// The node's defaults overlaid with the graph's values; unknown names and unsupported non-defaults are errors.
    pub fn resolve(specs: &[ParamSpec], given: Option<&Map<String, Value>>) -> Result<Params> {
        let mut values = Map::new();
        for s in specs {
            values.insert(s.name.to_string(), (s.default)());
        }
        if let Some(given) = given {
            for (k, v) in given {
                let Some(spec) = specs.iter().find(|s| s.name == k) else {
                    return Err(Error::new(format!("unknown parameter {k:?}")));
                };
                if !spec.supported && !same_value(v, &(spec.default)()) {
                    return Err(Error::new(format!(
                        "parameter {k:?} = {v} is not supported by the Rust backend yet (only the default {})",
                        (spec.default)()
                    )));
                }
                values.insert(k.clone(), v.clone());
            }
        }
        Ok(Params { values })
    }

    fn get(&self, name: &str) -> &Value {
        self.values.get(name).unwrap_or(&Value::Null)
    }

    pub fn value(&self, name: &str) -> &Value { self.get(name) }

    pub fn f(&self, name: &str) -> Result<f64> {
        let v = self.get(name);
        match v {
            Value::Bool(b) => Ok(*b as i64 as f64),
            _ => v.as_f64().ok_or_else(|| Error::new(format!("parameter {name:?}: expected a number, got {v}"))),
        }
    }

    pub fn i(&self, name: &str) -> Result<i64> {
        let x = self.f(name)?;
        if x.fract() != 0.0 {
            return Err(Error::new(format!("parameter {name:?}: expected an integer, got {x}")));
        }
        Ok(x as i64)
    }

    pub fn b(&self, name: &str) -> Result<bool> {
        match self.get(name) {
            Value::Bool(b) => Ok(*b),
            v => v.as_f64().map(|x| x != 0.0).ok_or_else(|| Error::new(format!("parameter {name:?}: expected true or false"))),
        }
    }

    pub fn s(&self, name: &str) -> Result<String> {
        self.get(name).as_str().map(str::to_string).ok_or_else(|| Error::new(format!("parameter {name:?}: expected a string")))
    }

    pub fn v3(&self, name: &str) -> Result<[f64; 3]> {
        let bad = || Error::new(format!("parameter {name:?}: expected 3 numbers"));
        let a = self.get(name).as_array().ok_or_else(bad)?;
        if a.len() != 3 {
            return Err(bad());
        }
        let mut out = [0.0; 3];
        for (o, x) in out.iter_mut().zip(a) {
            *o = x.as_f64().ok_or_else(bad)?;
        }
        Ok(out)
    }

    /// An enum parameter: one of `choices`.
    pub fn e(&self, name: &str, choices: &[&str]) -> Result<String> {
        let s = self.s(name)?;
        if !choices.contains(&s.as_str()) {
            return Err(Error::new(format!("parameter {name:?}: expected one of {choices:?}, got {s:?}")));
        }
        Ok(s)
    }
}

/// JSON equality with numbers compared by value (2 == 2.0), as Python compares them.
pub fn same_value(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => x.as_f64() == y.as_f64(),
        (Value::Bool(x), Value::Number(y)) | (Value::Number(y), Value::Bool(x)) => Some(*x as i64 as f64) == y.as_f64(),
        (Value::Array(x), Value::Array(y)) => x.len() == y.len() && x.iter().zip(y).all(|(p, q)| same_value(p, q)),
        _ => a == b,
    }
}

/// Shorthand for building a [`ParamSpec`] list.
#[macro_export]
macro_rules! params {
    ($($name:literal : $default:expr $(, $flag:ident)?);* $(;)?) => {
        &[$($crate::params::ParamSpec { name: $name, default: || serde_json::json!($default),
                                         supported: $crate::params!(@sup $($flag)?) }),*]
    };
    (@sup) => { true };
    (@sup unsupported) => { false };
}
