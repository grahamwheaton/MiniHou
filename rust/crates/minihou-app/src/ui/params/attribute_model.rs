//! Pure parameter model of the Attribute Create panel: entry defaults, channel counts and legacy upgrades.
use serde_json::{json, Value};

/// Domain choices as (stored value, label). Faces are stored as `primitives`.
pub const DOMAINS: [(&str, &str); 3] = [("points", "Points"), ("edges", "Edges"), ("primitives", "Faces")];
pub const TYPES: [&str; 6] = ["float", "int", "string", "vector2", "vector", "color"];

/// Channels a type has before any alpha choice.
pub fn natural_channels(ty: &str) -> usize {
    match ty { "vector2" => 2, "vector" | "color" => 3, "vector4" => 4, "string" => 0, _ => 1 }
}

/// Value channels of an entry: colour is 3 or 4 (alpha), every other type is fixed by the type.
pub fn channels(entry: &Value) -> usize {
    let ty = entry["type"].as_str().unwrap_or("float");
    if ty == "color" && entry["size"].as_u64() == Some(4) { 4 } else { natural_channels(ty) }
}

/// The entry's domain: the `domain` key, else the older `class` key, else points.
pub fn entry_domain(entry: &Value) -> &str {
    entry["domain"].as_str().or_else(|| entry["class"].as_str()).unwrap_or("points")
}

pub fn domain_label(domain: &str) -> &str {
    DOMAINS.iter().find(|(v, _)| *v == domain).map_or(domain, |(_, l)| l)
}

pub fn blank_entry(index: usize) -> Value {
    json!({"name": format!("attrib{index}"), "domain": "points", "type": "float", "size": 1, "value": [0.0]})
}

fn resized(value: &Value, n: usize, fill: f64) -> Vec<Value> {
    let mut v = value.as_array().cloned().unwrap_or_default();
    v.resize(n, json!(fill));
    v
}

/// Change the type: size and value channels follow it.
pub fn set_type(entry: &mut Value, ty: &str) {
    let keep_alpha = ty == "color" && entry["type"] == "color" && channels(entry) == 4;
    entry["type"] = json!(ty);
    let n = if keep_alpha { 4 } else { natural_channels(ty) };
    if ty == "string" {
        entry["size"] = json!(1);
        if !entry["value"].is_string() { entry["value"] = json!(""); }
    } else {
        entry["size"] = json!(n);
        let value = if entry["value"].is_array() { entry["value"].clone() } else { json!([]) };
        entry["value"] = json!(resized(&value, n, 0.0));
    }
}

/// Colour with or without alpha (4 channels, alpha defaults to opaque).
pub fn set_alpha(entry: &mut Value, alpha: bool) {
    let n = if alpha { 4 } else { 3 };
    let had_alpha = entry["value"].as_array().is_some_and(|a| a.len() >= 4);
    let mut value = resized(&entry["value"], n, 0.0);
    if alpha && !had_alpha { value[3] = json!(1.0); }
    entry["size"] = json!(n);
    entry["value"] = json!(value);
}

/// Entries to show: the stored list, or a single entry built from the pre-list node parameters.
pub fn resolve(params: &Value) -> Vec<Value> {
    if let Some(list) = params["attributes"].as_array() { return list.clone(); }
    let mut entry = json!({
        "name": params["name"].as_str().unwrap_or("attrib1"),
        "domain": params["class"].as_str().filter(|c| *c != "points").or(params["domain"].as_str()).unwrap_or("points"),
        "type": params["type"].as_str().unwrap_or("float"),
        "value": params["value"].clone(),
    });
    let ty = entry["type"].as_str().unwrap_or("float").to_owned();
    entry["type"] = json!("");
    set_type(&mut entry, &ty);
    vec![entry]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn value_channels_follow_the_type() {
        let mut e = blank_entry(1);
        for (ty, n) in [("float", 1), ("int", 1), ("vector2", 2), ("vector", 3), ("color", 3), ("string", 0)] {
            set_type(&mut e, ty);
            assert_eq!(channels(&e), n, "{ty}");
            if ty != "string" { assert_eq!(e["value"].as_array().unwrap().len(), n, "{ty}"); }
        }
        set_type(&mut e, "color");
        set_alpha(&mut e, true);
        assert_eq!((channels(&e), e["value"][3].as_f64()), (4, Some(1.0)));
        set_type(&mut e, "color"); // same type keeps the alpha choice
        assert_eq!(channels(&e), 4);
        set_type(&mut e, "vector");
        assert_eq!(channels(&e), 3);
        set_type(&mut e, "float");
        assert_eq!(e["value"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn old_parameters_still_resolve() {
        // class key, size and default stored by the earlier panel; the count never was a parameter
        let old = json!({"attributes": [{"name":"w","class":"primitives","type":"float","size":1,
            "default":[0,0,0,0],"value":[2,0,0,1]}]});
        let list = resolve(&old);
        assert_eq!(entry_domain(&list[0]), "primitives");
        assert_eq!(domain_label(entry_domain(&list[0])), "Faces");
        assert_eq!(channels(&list[0]), 1);
        let legacy = resolve(&json!({"name":"Cd","domain":"edges","type":"color","value":[1,0.5,0],"numattribs":7}));
        assert_eq!(legacy.len(), 1);
        assert_eq!((legacy[0]["name"].as_str(), entry_domain(&legacy[0]), channels(&legacy[0])), (Some("Cd"), "edges", 3));
        assert_eq!(resolve(&json!({"attributes": []})).len(), 0);
    }
}
