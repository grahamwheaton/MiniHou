//! Parameter data models, color parsing/serialization, and ramp curve/gradient logic.
use serde_json::{json, Value};

/// Parse a color value into RGBA `[f32; 4]`, number of components (3 or 4), and whether it was a hex string.
pub fn parse_color_value(value: &Value) -> Option<([f32; 4], usize, bool)> {
    if let Some(arr) = value.as_array() {
        if arr.len() == 3 && arr.iter().all(Value::is_number) {
            let r = arr[0].as_f64()? as f32;
            let g = arr[1].as_f64()? as f32;
            let b = arr[2].as_f64()? as f32;
            return Some(([r, g, b, 1.0], 3, false));
        } else if arr.len() == 4 && arr.iter().all(Value::is_number) {
            let r = arr[0].as_f64()? as f32;
            let g = arr[1].as_f64()? as f32;
            let b = arr[2].as_f64()? as f32;
            let a = arr[3].as_f64()? as f32;
            return Some(([r, g, b, a], 4, false));
        }
    } else if let Some(s) = value.as_str() {
        let s = s.trim();
        if let Some(hex_part) = s.strip_prefix('#') {
            if hex_part.len() == 6 {
                if let (Ok(r), Ok(g), Ok(b)) = (
                    u8::from_str_radix(&hex_part[0..2], 16),
                    u8::from_str_radix(&hex_part[2..4], 16),
                    u8::from_str_radix(&hex_part[4..6], 16),
                ) {
                    return Some(([r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, 1.0], 3, true));
                }
            } else if hex_part.len() == 8 {
                if let (Ok(r), Ok(g), Ok(b), Ok(a)) = (
                    u8::from_str_radix(&hex_part[0..2], 16),
                    u8::from_str_radix(&hex_part[2..4], 16),
                    u8::from_str_radix(&hex_part[4..6], 16),
                    u8::from_str_radix(&hex_part[6..8], 16),
                ) {
                    return Some(([r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, a as f32 / 255.0], 4, true));
                }
            } else if hex_part.len() == 3 {
                if let (Ok(r), Ok(g), Ok(b)) = (
                    u8::from_str_radix(&hex_part[0..1], 16),
                    u8::from_str_radix(&hex_part[1..2], 16),
                    u8::from_str_radix(&hex_part[2..3], 16),
                ) {
                    return Some(([
                        (r * 17) as f32 / 255.0,
                        (g * 17) as f32 / 255.0,
                        (b * 17) as f32 / 255.0,
                        1.0,
                    ], 3, true));
                }
            }
        }
    }
    None
}

/// Formats f32 cleanly into JSON without float representation noise.
pub fn clean_float(f: f32) -> Value {
    f.to_string().parse::<f64>().map_or_else(|_| json!(f), |v| json!(v))
}

/// Serialize RGBA back into the original format (array of 3/4 floats or hex string).
pub fn serialize_color_value(rgba: [f32; 4], comps: usize, is_hex: bool) -> Value {
    if is_hex {
        let r = (rgba[0] * 255.0).clamp(0.0, 255.0).round() as u8;
        let g = (rgba[1] * 255.0).clamp(0.0, 255.0).round() as u8;
        let b = (rgba[2] * 255.0).clamp(0.0, 255.0).round() as u8;
        if comps == 4 {
            let a = (rgba[3] * 255.0).clamp(0.0, 255.0).round() as u8;
            json!(format!("#{r:02x}{g:02x}{b:02x}{a:02x}"))
        } else {
            json!(format!("#{r:02x}{g:02x}{b:02x}"))
        }
    } else if comps == 4 {
        json!([clean_float(rgba[0]), clean_float(rgba[1]), clean_float(rgba[2]), clean_float(rgba[3])])
    } else {
        json!([clean_float(rgba[0]), clean_float(rgba[1]), clean_float(rgba[2])])
    }
}

/// Check if a parameter represents a colour.
pub fn is_color_param(param_name: &str, meta: Option<&Value>, value: &Value) -> bool {
    if meta.and_then(|m| m["kind"].as_str()) == Some("color") {
        return true;
    }
    let name_is_color = param_name == "color"
        || param_name == "color2"
        || param_name == "colour"
        || param_name.ends_with("_color")
        || param_name.ends_with("_colour")
        || param_name == "tint"
        || param_name == "diffuse"
        || param_name == "specular";
    if name_is_color && parse_color_value(value).is_some() {
        return true;
    }
    if param_name.is_empty() && value.as_str().is_some_and(|s| s.starts_with('#')) && parse_color_value(value).is_some() {
        return true;
    }
    false
}

/// Check if a parameter represents a file path.
pub fn is_file_param(param_name: &str, meta: Option<&Value>) -> bool {
    if let Some(m) = meta {
        if m["kind"].as_str() == Some("file") {
            return true;
        }
        if m["ui"].as_str().is_some_and(|ui| ui.starts_with("file:")) {
            return true;
        }
    }
    param_name == "file"
        || param_name == "path"
        || param_name == "rule_file"
        || param_name == "image_file"
        || param_name.ends_with("_file")
        || param_name.ends_with("_path")
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RampInterpolation {
    Linear,
    Constant,
    Smooth,
}

impl RampInterpolation {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Linear => "linear",
            Self::Constant => "constant",
            Self::Smooth => "smooth",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "constant" | "step" | "hold" => Self::Constant,
            "smooth" | "catmull-rom" | "catmullrom" | "monotone_cubic" | "ease" | "bezier" => Self::Smooth,
            _ => Self::Linear,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum RampValue {
    Float(f32),
    Color([f32; 3]),
    ColorAlpha([f32; 4]),
}

impl RampValue {
    pub fn as_float(&self) -> f32 {
        match self {
            Self::Float(f) => *f,
            Self::Color(c) => (c[0] + c[1] + c[2]) / 3.0,
            Self::ColorAlpha(c) => (c[0] + c[1] + c[2]) / 3.0,
        }
    }

    pub fn as_rgba(&self) -> [f32; 4] {
        match self {
            Self::Color(c) => [c[0], c[1], c[2], 1.0],
            Self::ColorAlpha(c) => *c,
            Self::Float(f) => [*f, *f, *f, 1.0],
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct RampStop {
    pub pos: f32,
    pub value: RampValue,
    pub interp: Option<RampInterpolation>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RampFormat {
    StopsObject,       // {"stops": [...], "interpolation": "..."}
    KeysValuesObject,  // {"keys": [...], "values": [...], "basis": "..."}
    ArrayOfPairs,      // [[pos, val], ...]
    ArrayOfStops,      // [{"pos": ..., "value"/"color": ...}, ...]
}

#[derive(Clone, Debug, PartialEq)]
pub struct RampData {
    pub is_color: bool,
    pub stops: Vec<RampStop>,
    pub interpolation: RampInterpolation,
    pub format: RampFormat,
}

impl RampData {
    pub fn is_ramp_value(value: &Value) -> bool {
        if let Some(obj) = value.as_object() {
            if obj.contains_key("stops") || (obj.contains_key("keys") && obj.contains_key("values")) {
                return true;
            }
        } else if let Some(arr) = value.as_array() {
            if !arr.is_empty() {
                if let Some(first) = arr.first() {
                    if first.as_array().is_some_and(|pair| pair.len() == 2 && pair[0].is_number()) {
                        return true;
                    }
                    if first.as_object().is_some_and(|item| item.contains_key("pos") || item.contains_key("position")) {
                        return true;
                    }
                }
            }
        }
        false
    }

    pub fn from_value(value: &Value, is_color_hint: bool) -> Self {
        let mut is_color = is_color_hint;
        let mut stops = Vec::new();
        let mut interpolation = RampInterpolation::Linear;
        let mut format = RampFormat::StopsObject;

        if let Some(obj) = value.as_object() {
            if let Some(interp) = obj.get("interpolation").or_else(|| obj.get("basis")).and_then(Value::as_str) {
                interpolation = RampInterpolation::from_str(interp);
            }
            if let Some(stops_arr) = obj.get("stops").and_then(Value::as_array) {
                format = RampFormat::StopsObject;
                for item in stops_arr {
                    if let Some(s) = Self::parse_stop_object(item, &mut is_color) {
                        stops.push(s);
                    }
                }
            } else if let (Some(keys), Some(values)) = (obj.get("keys").and_then(Value::as_array), obj.get("values").and_then(Value::as_array)) {
                format = RampFormat::KeysValuesObject;
                for (k, v) in keys.iter().zip(values.iter()) {
                    if let Some(pos) = k.as_f64() {
                        let val = Self::parse_ramp_value(v, &mut is_color);
                        stops.push(RampStop { pos: pos as f32, value: val, interp: None });
                    }
                }
            }
        } else if let Some(arr) = value.as_array() {
            if let Some(first) = arr.first() {
                if first.as_array().is_some() {
                    format = RampFormat::ArrayOfPairs;
                    for pair in arr {
                        if let Some(p) = pair.as_array() {
                            if p.len() >= 2 {
                                let pos = p[0].as_f64().unwrap_or(0.) as f32;
                                let val = Self::parse_ramp_value(&p[1], &mut is_color);
                                stops.push(RampStop { pos, value: val, interp: None });
                            }
                        }
                    }
                } else if first.as_object().is_some() {
                    format = RampFormat::ArrayOfStops;
                    for item in arr {
                        if let Some(s) = Self::parse_stop_object(item, &mut is_color) {
                            stops.push(s);
                        }
                    }
                }
            }
        }

        let mut data = Self {
            is_color,
            stops,
            interpolation,
            format,
        };
        data.ensure_valid_defaults();
        data.sort_stops();
        data
    }

    fn parse_ramp_value(v: &Value, is_color: &mut bool) -> RampValue {
        if let Some(arr) = v.as_array() {
            if arr.len() == 4 && arr.iter().all(Value::is_number) {
                *is_color = true;
                return RampValue::ColorAlpha([
                    arr[0].as_f64().unwrap_or(0.) as f32,
                    arr[1].as_f64().unwrap_or(0.) as f32,
                    arr[2].as_f64().unwrap_or(0.) as f32,
                    arr[3].as_f64().unwrap_or(1.) as f32,
                ]);
            } else if arr.len() == 3 && arr.iter().all(Value::is_number) {
                *is_color = true;
                return RampValue::Color([
                    arr[0].as_f64().unwrap_or(0.) as f32,
                    arr[1].as_f64().unwrap_or(0.) as f32,
                    arr[2].as_f64().unwrap_or(0.) as f32,
                ]);
            }
        } else if let Some(n) = v.as_f64() {
            return RampValue::Float(n as f32);
        } else if let Some(s) = v.as_str() {
            if let Some((rgba, _, _)) = parse_color_value(&Value::String(s.to_owned())) {
                *is_color = true;
                return RampValue::Color([rgba[0], rgba[1], rgba[2]]);
            }
        }
        if *is_color {
            RampValue::Color([0.0, 0.0, 0.0])
        } else {
            RampValue::Float(0.0)
        }
    }

    fn parse_stop_object(item: &Value, is_color: &mut bool) -> Option<RampStop> {
        let obj = item.as_object()?;
        let pos = obj.get("pos")
            .or_else(|| obj.get("position"))
            .or_else(|| obj.get("t"))?
            .as_f64()? as f32;
        let interp = obj.get("interpolation")
            .or_else(|| obj.get("interp"))
            .and_then(Value::as_str)
            .map(RampInterpolation::from_str);
        let value = if let Some(c) = obj.get("color").or_else(|| obj.get("colour")) {
            Self::parse_ramp_value(c, is_color)
        } else if let Some(v) = obj.get("value").or_else(|| obj.get("val")) {
            Self::parse_ramp_value(v, is_color)
        } else {
            Self::parse_ramp_value(&Value::Null, is_color)
        };
        Some(RampStop { pos: pos.clamp(0.0, 1.0), value, interp })
    }

    fn ensure_valid_defaults(&mut self) {
        if self.stops.is_empty() {
            if self.is_color {
                self.stops = vec![
                    RampStop { pos: 0.0, value: RampValue::Color([0.0, 0.0, 0.0]), interp: None },
                    RampStop { pos: 1.0, value: RampValue::Color([1.0, 1.0, 1.0]), interp: None },
                ];
            } else {
                self.stops = vec![
                    RampStop { pos: 0.0, value: RampValue::Float(0.0), interp: None },
                    RampStop { pos: 1.0, value: RampValue::Float(1.0), interp: None },
                ];
            }
        }
    }

    pub fn to_value(&self) -> Value {
        match self.format {
            RampFormat::ArrayOfPairs => {
                let pairs: Vec<Value> = self.stops.iter().map(|s| {
                    let v = match &s.value {
                        RampValue::Float(f) => clean_float(*f),
                        RampValue::Color(c) => json!([clean_float(c[0]), clean_float(c[1]), clean_float(c[2])]),
                        RampValue::ColorAlpha(c) => json!([clean_float(c[0]), clean_float(c[1]), clean_float(c[2]), clean_float(c[3])]),
                    };
                    json!([clean_float(s.pos), v])
                }).collect();
                json!(pairs)
            }
            RampFormat::ArrayOfStops => {
                let stops: Vec<Value> = self.stops.iter().map(|s| {
                    let mut map = serde_json::Map::new();
                    map.insert("pos".into(), clean_float(s.pos));
                    match &s.value {
                        RampValue::Float(f) => { map.insert("value".into(), clean_float(*f)); }
                        RampValue::Color(c) => { map.insert("color".into(), json!([clean_float(c[0]), clean_float(c[1]), clean_float(c[2])])); }
                        RampValue::ColorAlpha(c) => { map.insert("color".into(), json!([clean_float(c[0]), clean_float(c[1]), clean_float(c[2]), clean_float(c[3])])); }
                    }
                    if let Some(interp) = s.interp {
                        map.insert("interpolation".into(), json!(interp.as_str()));
                    }
                    Value::Object(map)
                }).collect();
                json!(stops)
            }
            RampFormat::KeysValuesObject => {
                let keys: Vec<Value> = self.stops.iter().map(|s| clean_float(s.pos)).collect();
                let values: Vec<Value> = self.stops.iter().map(|s| {
                    match &s.value {
                        RampValue::Float(f) => clean_float(*f),
                        RampValue::Color(c) => json!([clean_float(c[0]), clean_float(c[1]), clean_float(c[2])]),
                        RampValue::ColorAlpha(c) => json!([clean_float(c[0]), clean_float(c[1]), clean_float(c[2]), clean_float(c[3])]),
                    }
                }).collect();
                json!({
                    "keys": keys,
                    "values": values,
                    "basis": self.interpolation.as_str()
                })
            }
            RampFormat::StopsObject => {
                let stops: Vec<Value> = self.stops.iter().map(|s| {
                    let mut map = serde_json::Map::new();
                    map.insert("pos".into(), clean_float(s.pos));
                    match &s.value {
                        RampValue::Float(f) => { map.insert("value".into(), clean_float(*f)); }
                        RampValue::Color(c) => { map.insert("color".into(), json!([clean_float(c[0]), clean_float(c[1]), clean_float(c[2])])); }
                        RampValue::ColorAlpha(c) => { map.insert("color".into(), json!([clean_float(c[0]), clean_float(c[1]), clean_float(c[2]), clean_float(c[3])])); }
                    }
                    if let Some(interp) = s.interp {
                        map.insert("interpolation".into(), json!(interp.as_str()));
                    }
                    Value::Object(map)
                }).collect();
                json!({
                    "stops": stops,
                    "interpolation": self.interpolation.as_str()
                })
            }
        }
    }

    pub fn sort_stops(&mut self) {
        self.stops.sort_by(|a, b| a.pos.partial_cmp(&b.pos).unwrap_or(std::cmp::Ordering::Equal));
    }

    pub fn eval_float(&self, t: f32) -> f32 {
        if self.stops.is_empty() { return 0.0; }
        if self.stops.len() == 1 || t <= self.stops[0].pos {
            return self.stops[0].value.as_float();
        }
        let last = self.stops.last().unwrap();
        if t >= last.pos {
            return last.value.as_float();
        }
        for i in 0..self.stops.len() - 1 {
            let s0 = &self.stops[i];
            let s1 = &self.stops[i + 1];
            if t >= s0.pos && t <= s1.pos {
                let u = ((t - s0.pos) / (s1.pos - s0.pos).max(1e-6)).clamp(0.0, 1.0);
                let interp = s0.interp.unwrap_or(self.interpolation);
                let v0 = s0.value.as_float();
                let v1 = s1.value.as_float();
                return match interp {
                    RampInterpolation::Constant => v0,
                    RampInterpolation::Linear => (1.0 - u) * v0 + u * v1,
                    RampInterpolation::Smooth => {
                        let s = u * u * (3.0 - 2.0 * u);
                        (1.0 - s) * v0 + s * v1
                    }
                };
            }
        }
        0.0
    }

    pub fn eval_color(&self, t: f32) -> [f32; 4] {
        if self.stops.is_empty() { return [0.0, 0.0, 0.0, 1.0]; }
        if self.stops.len() == 1 || t <= self.stops[0].pos {
            return self.stops[0].value.as_rgba();
        }
        let last = self.stops.last().unwrap();
        if t >= last.pos {
            return last.value.as_rgba();
        }
        for i in 0..self.stops.len() - 1 {
            let s0 = &self.stops[i];
            let s1 = &self.stops[i + 1];
            if t >= s0.pos && t <= s1.pos {
                let u = ((t - s0.pos) / (s1.pos - s0.pos).max(1e-6)).clamp(0.0, 1.0);
                let interp = s0.interp.unwrap_or(self.interpolation);
                let c0 = s0.value.as_rgba();
                let c1 = s1.value.as_rgba();
                let factor = match interp {
                    RampInterpolation::Constant => 0.0,
                    RampInterpolation::Linear => u,
                    RampInterpolation::Smooth => u * u * (3.0 - 2.0 * u),
                };
                return [
                    (1.0 - factor) * c0[0] + factor * c1[0],
                    (1.0 - factor) * c0[1] + factor * c1[1],
                    (1.0 - factor) * c0[2] + factor * c1[2],
                    (1.0 - factor) * c0[3] + factor * c1[3],
                ];
            }
        }
        [0.0, 0.0, 0.0, 1.0]
    }

    pub fn add_stop(&mut self, pos: f32) -> usize {
        let pos = pos.clamp(0.0, 1.0);
        let val = if self.is_color {
            let c = self.eval_color(pos);
            RampValue::Color([c[0], c[1], c[2]])
        } else {
            RampValue::Float(self.eval_float(pos))
        };
        let new_stop = RampStop { pos, value: val, interp: None };
        self.stops.push(new_stop);
        self.sort_stops();
        self.stops.iter().position(|s| (s.pos - pos).abs() < 1e-4).unwrap_or(0)
    }

    pub fn delete_stop(&mut self, index: usize) -> bool {
        if self.stops.len() <= 1 || index >= self.stops.len() {
            return false;
        }
        self.stops.remove(index);
        true
    }

    pub fn move_stop(&mut self, index: usize, new_pos: f32) -> usize {
        if index >= self.stops.len() { return index; }
        let new_pos = new_pos.clamp(0.0, 1.0);
        self.stops[index].pos = new_pos;
        let target_stop = self.stops[index].clone();
        self.sort_stops();
        self.stops.iter().position(|s| s.pos == target_stop.pos && s.value == target_stop.value).unwrap_or(0)
    }
}

pub fn is_ramp_param(param_name: &str, meta: Option<&Value>, value: &Value) -> bool {
    if let Some(m) = meta {
        if let Some(k) = m["kind"].as_str() {
            if k == "float_ramp" || k == "color_ramp" || k == "ramp" {
                return true;
            }
        }
    }
    if param_name == "ramp"
        || param_name == "float_ramp"
        || param_name == "color_ramp"
        || param_name.ends_with("_ramp")
    {
        return true;
    }
    RampData::is_ramp_value(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn color_deserialization_and_serialization() {
        // 3-component RGB array
        let rgb_val = json!([0.2, 0.4, 0.6]);
        let (rgba, comps, is_hex) = parse_color_value(&rgb_val).expect("parse rgb");
        assert_eq!(comps, 3);
        assert!(!is_hex);
        assert!((rgba[0] - 0.2).abs() < 1e-4);
        assert!((rgba[1] - 0.4).abs() < 1e-4);
        assert!((rgba[2] - 0.6).abs() < 1e-4);
        assert_eq!(rgba[3], 1.0);
        let ser_rgb = serialize_color_value(rgba, comps, is_hex);
        assert_eq!(ser_rgb, rgb_val);

        // 4-component RGBA array
        let rgba_val = json!([0.1, 0.2, 0.3, 0.8]);
        let (rgba, comps, is_hex) = parse_color_value(&rgba_val).expect("parse rgba");
        assert_eq!(comps, 4);
        assert!(!is_hex);
        assert!((rgba[0] - 0.1).abs() < 1e-4);
        assert!((rgba[3] - 0.8).abs() < 1e-4);
        let ser_rgba = serialize_color_value(rgba, comps, is_hex);
        assert_eq!(ser_rgba, rgba_val);

        // 6-digit hex string
        let hex_val = json!("#336699");
        let (rgba, comps, is_hex) = parse_color_value(&hex_val).expect("parse hex");
        assert_eq!(comps, 3);
        assert!(is_hex);
        assert_eq!((rgba[0] * 255.0).round() as u8, 0x33);
        assert_eq!((rgba[1] * 255.0).round() as u8, 0x66);
        assert_eq!((rgba[2] * 255.0).round() as u8, 0x99);
        let ser_hex = serialize_color_value(rgba, comps, is_hex);
        assert_eq!(ser_hex, hex_val);

        // 8-digit hex string
        let hex_alpha_val = json!("#336699cc");
        let (rgba, comps, is_hex) = parse_color_value(&hex_alpha_val).expect("parse hex alpha");
        assert_eq!(comps, 4);
        assert!(is_hex);
        assert_eq!((rgba[3] * 255.0).round() as u8, 0xcc);
        let ser_hex_alpha = serialize_color_value(rgba, comps, is_hex);
        assert_eq!(ser_hex_alpha, hex_alpha_val);

        // 3-digit shorthand hex string
        let hex3_val = json!("#369");
        let (rgba, comps, is_hex) = parse_color_value(&hex3_val).expect("parse hex3");
        assert_eq!(comps, 3);
        assert!(is_hex);
        assert_eq!((rgba[0] * 255.0).round() as u8, 0x33);
        assert_eq!((rgba[1] * 255.0).round() as u8, 0x66);
        assert_eq!((rgba[2] * 255.0).round() as u8, 0x99);
        let ser_hex3 = serialize_color_value(rgba, comps, is_hex);
        assert_eq!(ser_hex3, json!("#336699"));

        // Invalid values
        assert!(parse_color_value(&json!("not-a-color")).is_none());
        assert!(parse_color_value(&json!([1.0, 2.0])).is_none());
        assert!(parse_color_value(&json!(123)).is_none());
    }

    #[test]
    fn ramp_deserialization_and_serialization() {
        // Format 1: StopsObject {"stops": [...], "interpolation": "..."}
        let stops_obj = json!({
            "stops": [
                {"pos": 0.0, "value": 0.2},
                {"pos": 1.0, "value": 0.8}
            ],
            "interpolation": "smooth"
        });
        let ramp1 = RampData::from_value(&stops_obj, false);
        assert_eq!(ramp1.format, RampFormat::StopsObject);
        assert_eq!(ramp1.stops.len(), 2);
        assert_eq!(ramp1.interpolation, RampInterpolation::Smooth);
        assert_eq!(ramp1.stops[0].value, RampValue::Float(0.2));
        assert_eq!(ramp1.stops[1].value, RampValue::Float(0.8));
        let ser1 = ramp1.to_value();
        assert_eq!(ser1["stops"].as_array().unwrap().len(), 2);
        assert_eq!(ser1["interpolation"], "smooth");

        // Format 2: KeysValuesObject {"keys": [...], "values": [...], "basis": "..."}
        let keys_obj = json!({
            "keys": [0.0, 0.5, 1.0],
            "values": [0.0, 0.4, 1.0],
            "basis": "linear"
        });
        let ramp2 = RampData::from_value(&keys_obj, false);
        assert_eq!(ramp2.format, RampFormat::KeysValuesObject);
        assert_eq!(ramp2.stops.len(), 3);
        assert_eq!(ramp2.interpolation, RampInterpolation::Linear);
        let ser2 = ramp2.to_value();
        assert_eq!(ser2["keys"].as_array().unwrap().len(), 3);
        assert_eq!(ser2["values"].as_array().unwrap().len(), 3);
        assert_eq!(ser2["basis"], "linear");

        // Format 3: ArrayOfPairs [[pos, val], ...]
        let pairs = json!([
            [0.0, 0.1],
            [1.0, 0.9]
        ]);
        let ramp3 = RampData::from_value(&pairs, false);
        assert_eq!(ramp3.format, RampFormat::ArrayOfPairs);
        assert_eq!(ramp3.stops.len(), 2);
        let ser3 = ramp3.to_value();
        assert_eq!(ser3, pairs);

        // Format 4: ArrayOfStops [{"pos": 0.0, "color": [...]}, ...]
        let stops_arr = json!([
            {"pos": 0.0, "color": [1.0, 0.0, 0.0]},
            {"pos": 1.0, "color": [0.0, 0.0, 1.0]}
        ]);
        let ramp4 = RampData::from_value(&stops_arr, true);
        assert_eq!(ramp4.format, RampFormat::ArrayOfStops);
        assert!(ramp4.is_color);
        assert_eq!(ramp4.stops.len(), 2);
        let ser4 = ramp4.to_value();
        assert_eq!(ser4[0]["pos"], 0.0);
        assert_eq!(ser4[0]["color"], json!([1.0, 0.0, 0.0]));
        assert_eq!(ser4[1]["pos"], 1.0);
        assert_eq!(ser4[1]["color"], json!([0.0, 0.0, 1.0]));
    }

    #[test]
    fn ramp_stop_add_delete_sort_and_eval() {
        let mut ramp = RampData {
            is_color: false,
            stops: vec![
                RampStop { pos: 0.0, value: RampValue::Float(0.0), interp: None },
                RampStop { pos: 1.0, value: RampValue::Float(1.0), interp: None },
            ],
            interpolation: RampInterpolation::Linear,
            format: RampFormat::ArrayOfPairs,
        };

        // Linear evaluation
        assert!((ramp.eval_float(0.0) - 0.0).abs() < 1e-4);
        assert!((ramp.eval_float(0.5) - 0.5).abs() < 1e-4);
        assert!((ramp.eval_float(1.0) - 1.0).abs() < 1e-4);

        // Constant evaluation
        ramp.interpolation = RampInterpolation::Constant;
        assert!((ramp.eval_float(0.49) - 0.0).abs() < 1e-4);
        assert!((ramp.eval_float(0.99) - 0.0).abs() < 1e-4);

        // Smooth evaluation
        ramp.interpolation = RampInterpolation::Smooth;
        assert!((ramp.eval_float(0.5) - 0.5).abs() < 1e-4);

        // Add stop at 0.5
        let idx = ramp.add_stop(0.5);
        assert_eq!(ramp.stops.len(), 3);
        assert_eq!(idx, 1);
        assert_eq!(ramp.stops[1].pos, 0.5);

        // Move stop 1 to 0.2 (verify sorted order)
        let new_idx = ramp.move_stop(1, 0.2);
        assert_eq!(new_idx, 1);
        assert_eq!(ramp.stops[1].pos, 0.2);

        // Move stop 1 to 0.8 (verify re-sort)
        let moved_idx = ramp.move_stop(1, 0.8);
        assert_eq!(moved_idx, 1);
        assert_eq!(ramp.stops[1].pos, 0.8);

        // Delete stop
        assert!(ramp.delete_stop(1));
        assert_eq!(ramp.stops.len(), 2);

        // Cannot delete below 2 stops
        assert!(ramp.delete_stop(0));
        assert_eq!(ramp.stops.len(), 1);
        assert!(!ramp.delete_stop(0)); // Cannot delete the last stop
        assert_eq!(ramp.stops.len(), 1);

        // Color eval test
        let color_ramp = RampData {
            is_color: true,
            stops: vec![
                RampStop { pos: 0.0, value: RampValue::Color([0.0, 0.2, 0.4]), interp: None },
                RampStop { pos: 1.0, value: RampValue::Color([1.0, 0.8, 0.6]), interp: None },
            ],
            interpolation: RampInterpolation::Linear,
            format: RampFormat::ArrayOfPairs,
        };
        let mid = color_ramp.eval_color(0.5);
        assert!((mid[0] - 0.5).abs() < 1e-3);
        assert!((mid[1] - 0.5).abs() < 1e-3);
        assert!((mid[2] - 0.5).abs() < 1e-3);
        assert_eq!(mid[3], 1.0);
    }

    #[test]
    fn param_type_detection() {
        // Color
        assert!(is_color_param("color", None, &json!([1.0, 1.0, 1.0])));
        assert!(is_color_param("tint", None, &json!("#ff0000")));
        assert!(is_color_param("diffuse", None, &json!([0.5, 0.5, 0.5])));
        assert!(is_color_param("custom", Some(&json!({"kind": "color"})), &json!(null)));
        assert!(!is_color_param("translate", None, &json!([1.0, 2.0, 3.0])));

        // File
        assert!(is_file_param("file", None));
        assert!(is_file_param("path", None));
        assert!(is_file_param("rule_file", None));
        assert!(is_file_param("custom", Some(&json!({"kind": "file"}))));
        assert!(is_file_param("custom", Some(&json!({"ui": "file:open"}))));
        assert!(!is_file_param("scale", None));

        // Ramp
        assert!(is_ramp_param("float_ramp", None, &json!(null)));
        assert!(is_ramp_param("color_ramp", None, &json!(null)));
        assert!(is_ramp_param("custom", Some(&json!({"kind": "float_ramp"})), &json!(null)));
        assert!(is_ramp_param("custom", Some(&json!({"kind": "color_ramp"})), &json!(null)));
        assert!(is_ramp_param("my_ramp", None, &json!(null)));
        assert!(is_ramp_param("custom", None, &json!({"stops": []})));
        assert!(!is_ramp_param("scale", None, &json!(1.0)));
    }
}
