//! Conditional branch between two inputs (Houdini's Switch-If).
use std::sync::Arc;
use serde_json::Value;

use crate::{
    error::{Error, Result},
    geometry::Geometry,
    nodes::{
        scalar_expr::{Expr, Number},
        InputSpec, Inputs, NodeSpec,
    },
    params::Params,
};

pub const INPUTS: &[InputSpec] = &[
    InputSpec { name: "input0", multi: false, optional: true },
    InputSpec { name: "input1", multi: false, optional: true },
    InputSpec { name: "first", multi: false, optional: true },
    InputSpec { name: "second", multi: false, optional: true },
];

pub(super) const NODE: NodeSpec = NodeSpec {
    outputs: &["geometry"],
    cook_multi: None,
    name: "switchif",
    inputs: INPUTS,
    params: crate::params!(
        "use_second_input_if": "all_conditions_true";
        "mergecondition": "all_conditions_true";
        "test_input": "first_input";
        "testinput": "first_input";
        "number_of_tests": 2;
        "numtests": 2;
        "tests": serde_json::json!([
            { "enable": true, "enable_value": "1", "test_type": "expression" },
            { "enable": true, "enable_value": "1", "test_type": "expression" }
        ]);
        "enable": true;
        "test_type": "expression";
        "type": "expression";
        "enable_value": "1";
        "expr": "1";
    ),
    cook,
};

fn resolve_inputs<'a>(inputs: &'a Inputs) -> (Option<&'a Arc<Geometry>>, Option<&'a Arc<Geometry>>) {
    if inputs.len() == 1 && inputs[0].len() >= 2 {
        return (inputs[0].first(), inputs[0].get(1));
    }
    let first = inputs.get(0).and_then(|s| s.first())
        .or_else(|| inputs.get(2).and_then(|s| s.first()));
    let second = inputs.get(1).and_then(|s| s.first())
        .or_else(|| inputs.get(3).and_then(|s| s.first()));
    (first, second)
}

fn resolve_rule(p: &Params) -> String {
    let default_val = Value::String("all_conditions_true".to_string());
    let val = if p.value("use_second_input_if") != &default_val && !p.value("use_second_input_if").is_null() {
        p.value("use_second_input_if")
    } else if p.value("mergecondition") != &default_val && !p.value("mergecondition").is_null() {
        p.value("mergecondition")
    } else {
        p.value("use_second_input_if")
    };

    match val {
        Value::Number(n) => match n.as_i64() {
            Some(1) => "any_condition_true".to_string(),
            Some(2) => "all_conditions_false".to_string(),
            Some(3) => "any_condition_false".to_string(),
            _ => "all_conditions_true".to_string(),
        },
        Value::String(s) => match s.as_str() {
            "all" | "all_conditions_true" => "all_conditions_true".to_string(),
            "any" | "any_condition_true" => "any_condition_true".to_string(),
            "none" | "all_conditions_false" => "all_conditions_false".to_string(),
            "not_all" | "any_condition_false" => "any_condition_false".to_string(),
            _ => s.clone(),
        },
        _ => "all_conditions_true".to_string(),
    }
}

fn resolve_test_input(p: &Params) -> String {
    let default_val = Value::String("first_input".to_string());
    let val = if p.value("test_input") != &default_val && !p.value("test_input").is_null() {
        p.value("test_input")
    } else if p.value("testinput") != &default_val && !p.value("testinput").is_null() {
        p.value("testinput")
    } else {
        p.value("test_input")
    };

    match val {
        Value::Number(n) => match n.as_i64() {
            Some(1) => "second_input".to_string(),
            _ => "first_input".to_string(),
        },
        Value::String(s) => match s.as_str() {
            "second" | "second_input" => "second_input".to_string(),
            _ => "first_input".to_string(),
        },
        _ => "first_input".to_string(),
    }
}

fn resolve_num_tests(p: &Params) -> usize {
    let default_val = Value::from(2);
    let val = if p.value("number_of_tests") != &default_val && !p.value("number_of_tests").is_null() {
        p.value("number_of_tests")
    } else if p.value("numtests") != &default_val && !p.value("numtests").is_null() {
        p.value("numtests")
    } else {
        p.value("number_of_tests")
    };

    match val {
        Value::Number(n) => n.as_i64().unwrap_or(2).max(0) as usize,
        _ => 2,
    }
}

fn eval_scalar_expr(text: &str, geo: Option<&Geometry>) -> Result<f64> {
    let text = text.trim();
    if let Ok(num) = text.parse::<f64>() {
        return Ok(num);
    }
    let expr = Expr::parse(text)?;
    let num = expr.eval(&mut |var| {
        let name = var.trim_start_matches('@').trim_start_matches('$');
        match name {
            "Frame" | "F" => Ok(Number::scalar(crate::timectx::frame())),
            "Time" | "T" => Ok(Number::scalar(crate::timectx::seconds())),
            "TimeInc" => Ok(Number::scalar(crate::timectx::time_inc())),
            "npoints" | "numpt" | "ptnum" => {
                let n = geo.map_or(0.0, |g| g.npoints as f64);
                Ok(Number::scalar(n))
            }
            "nprims" | "numprim" | "primnum" => {
                let n = geo.map_or(0.0, |g| g.nprims() as f64);
                Ok(Number::scalar(n))
            }
            "nvertices" | "numvtx" | "vtxnum" => {
                let n = geo.map_or(0.0, |g| g.nvertices() as f64);
                Ok(Number::scalar(n))
            }
            "pi" | "PI" => Ok(Number::scalar(std::f64::consts::PI)),
            _ => {
                if let Some(g) = geo {
                    if let Some(val) = g.detail.get(name) {
                        if let Some(num) = val.as_f64() {
                            return Ok(Number::scalar(num));
                        }
                        if let Some(b) = val.as_bool() {
                            return Ok(Number::scalar(if b { 1.0 } else { 0.0 }));
                        }
                    }
                }
                Err(Error::new(format!("unknown variable or attribute '{var}' in expression")))
            }
        }
    })?;
    if !num.value.is_finite() {
        return Err(Error::new("expression is not finite"));
    }
    Ok(num.value)
}

fn eval_expr_string(s: &str, geo: Option<&Geometry>) -> Result<bool> {
    let s = s.trim();
    if s.is_empty() {
        return Ok(false);
    }
    if s.eq_ignore_ascii_case("true") {
        return Ok(true);
    }
    if s.eq_ignore_ascii_case("false") {
        return Ok(false);
    }
    if let Ok(num) = s.parse::<f64>() {
        return Ok(num != 0.0);
    }
    for op in ["==", "!=", "<=", ">="] {
        if let Some((lhs, rhs)) = s.split_once(op) {
            let l = eval_scalar_expr(lhs.trim(), geo)?;
            let r = eval_scalar_expr(rhs.trim(), geo)?;
            let res = match op {
                "==" => (l - r).abs() < 1e-9,
                "!=" => (l - r).abs() >= 1e-9,
                "<=" => l <= r + 1e-9,
                ">=" => l >= r - 1e-9,
                _ => unreachable!(),
            };
            return Ok(res);
        }
    }
    for op in ["<", ">"] {
        if let Some((lhs, rhs)) = s.split_once(op) {
            let l = eval_scalar_expr(lhs.trim(), geo)?;
            let r = eval_scalar_expr(rhs.trim(), geo)?;
            let res = match op {
                "<" => l < r - 1e-9,
                ">" => l > r + 1e-9,
                _ => unreachable!(),
            };
            return Ok(res);
        }
    }
    let v = eval_scalar_expr(s, geo)?;
    Ok(v.abs() > 1e-9)
}

fn eval_test_value(val: &Value, geo: Option<&Geometry>) -> Result<bool> {
    match val {
        Value::Bool(b) => Ok(*b),
        Value::Number(n) => Ok(n.as_f64().unwrap_or(0.0) != 0.0),
        Value::String(s) => eval_expr_string(s, geo),
        Value::Null => Ok(true),
        _ => Err(Error::new(format!("invalid condition test value: {val}"))),
    }
}

fn cook(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let (first, second) = resolve_inputs(inputs);
    if first.is_none() && second.is_none() {
        return Err(Error::new("connect at least one input"));
    }

    let rule = resolve_rule(p);
    let test_input = resolve_test_input(p);
    let num_tests = resolve_num_tests(p);

    let test_geo = match test_input.as_str() {
        "second_input" => second.map(|g| g.as_ref()),
        _ => first.map(|g| g.as_ref()),
    };

    let default_tests = serde_json::json!([
        { "enable": true, "enable_value": "1", "test_type": "expression" },
        { "enable": true, "enable_value": "1", "test_type": "expression" }
    ]);
    let has_custom_tests = p.value("tests") != &default_tests && p.value("tests").as_array().is_some();

    let default_expr = Value::String("1".to_string());
    let top_val = if p.value("enable_value") != &default_expr && !p.value("enable_value").is_null() {
        p.value("enable_value")
    } else if p.value("expr") != &default_expr && !p.value("expr").is_null() {
        p.value("expr")
    } else {
        p.value("enable_value")
    };
    let top_enable = p.b("enable").unwrap_or(true);
    let top_type = p.s("test_type").or_else(|_| p.s("type")).unwrap_or_else(|_| "expression".to_string());

    let mut tests_list = Vec::new();
    if has_custom_tests {
        if let Some(arr) = p.value("tests").as_array() {
            for (i, item) in arr.iter().enumerate() {
                if i >= num_tests {
                    break;
                }
                let enable = item.get("enable").and_then(Value::as_bool).unwrap_or(true);
                let test_type = item.get("test_type").and_then(Value::as_str)
                    .or_else(|| item.get("type").and_then(Value::as_str))
                    .unwrap_or("expression");
                let val = item.get("enable_value")
                    .or_else(|| item.get("expr"))
                    .or_else(|| item.get("value"))
                    .unwrap_or(&Value::Null);
                tests_list.push((enable, test_type.to_string(), val.clone()));
            }
        }
    }

    while tests_list.len() < num_tests {
        tests_list.push((top_enable, top_type.clone(), top_val.clone()));
    }

    let mut test_results = Vec::new();
    for (enable, _test_type, val) in &tests_list {
        if !enable {
            continue;
        }
        test_results.push(eval_test_value(val, test_geo)?);
    }

    let use_second = match rule.as_str() {
        "all_conditions_true" => test_results.iter().all(|&b| b),
        "any_condition_true" => test_results.iter().any(|&b| b),
        "all_conditions_false" => test_results.iter().all(|&b| !b),
        "any_condition_false" => test_results.iter().any(|&b| !b),
        _ => return Err(Error::new(format!("unknown condition rule '{rule}'"))),
    };

    if use_second {
        let second_geo = second.ok_or_else(|| Error::new("second input is not connected"))?;
        Ok((**second_geo).clone())
    } else {
        let first_geo = first.ok_or_else(|| Error::new("first input is not connected"))?;
        Ok((**first_geo).clone())
    }
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
