//! Pure units module: internal units, display units, conversions, formatting, parsing, and length parameter metadata.

/// Supported length units for display.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum LengthUnit {
    #[default]
    Mm,
    Cm,
    M,
    Km,
    In,
    Ft,
}

impl LengthUnit {
    pub const ALL: [Self; 6] = [
        Self::Mm,
        Self::Cm,
        Self::M,
        Self::Km,
        Self::In,
        Self::Ft,
    ];

    pub fn symbol(self) -> &'static str {
        match self {
            Self::Mm => "mm",
            Self::Cm => "cm",
            Self::M => "m",
            Self::Km => "km",
            Self::In => "in",
            Self::Ft => "ft",
        }
    }

    /// Factor in metres for one unit.
    pub fn metres(self) -> f64 {
        match self {
            Self::Mm => 0.001,
            Self::Cm => 0.01,
            Self::M => 1.0,
            Self::Km => 1000.0,
            Self::In => 0.0254,
            Self::Ft => 0.3048,
        }
    }

    pub fn parse_symbol(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "mm" => Some(Self::Mm),
            "cm" => Some(Self::Cm),
            "m" => Some(Self::M),
            "km" => Some(Self::Km),
            "in" => Some(Self::In),
            "ft" => Some(Self::Ft),
            _ => None,
        }
    }
}

/// Unit settings: display unit and internal unit scale (metres per internal unit).
/// Default is mm display with scale 1.0 (1 internal unit = 1 metre = 1000 mm).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UnitSettings {
    pub display: LengthUnit,
    pub unit_scale: f64,
}

impl Default for UnitSettings {
    fn default() -> Self {
        Self {
            display: LengthUnit::Mm,
            unit_scale: 1.0,
        }
    }
}

impl UnitSettings {
    pub fn new(display: LengthUnit, unit_scale: f64) -> Self {
        Self {
            display,
            unit_scale: if unit_scale > 0.0 && unit_scale.is_finite() { unit_scale } else { 1.0 },
        }
    }

    /// Internal units per metre (reciprocal of unit_scale).
    pub fn internal_per_metre(&self) -> f64 {
        1.0 / self.unit_scale
    }

    /// Convert an internal value to the current display unit.
    pub fn display_value(&self, value_internal: f64) -> f64 {
        display(value_internal, *self)
    }

    /// Format an internal value using the current settings.
    pub fn format(&self, value_internal: f64) -> String {
        format(value_internal, *self)
    }

    /// Parse typed text into internal coordinates.
    pub fn parse(&self, text: &str) -> Option<f64> {
        parse(text, *self)
    }
}

/// Convert an internal value to display value.
/// Formula: v * scale / metres[unit]
pub fn display(value_internal: f64, settings: UnitSettings) -> f64 {
    value_internal * settings.unit_scale / settings.display.metres()
}

/// Format an internal value into a display string with unit suffix (e.g. "1000 mm").
/// Follows the old web editor: Number(api.display(v).toFixed(4)) + ' ' + unit.
pub fn format(value_internal: f64, settings: UnitSettings) -> String {
    let disp = display(value_internal, settings);
    format_display(disp, settings.display)
}

/// Format a value already in display units with the given unit symbol.
pub fn format_display(display_val: f64, unit: LengthUnit) -> String {
    let mut s = std::format!("{:.4}", display_val);
    if s == "-0.0000" {
        s = "0.0000".to_string();
    }
    if s.contains('.') {
        let trimmed = s.trim_end_matches('0');
        let trimmed = trimmed.strip_suffix('.').unwrap_or(trimmed);
        std::format!("{trimmed} {}", unit.symbol())
    } else {
        std::format!("{s} {}", unit.symbol())
    }
}

/// Parse text into internal coordinates.
/// Matches a number followed optionally by a unit suffix (mm, cm, km, m, in, ft).
/// If no unit suffix is given, the bare number is interpreted in `settings.display` units.
pub fn parse(text: &str, settings: UnitSettings) -> Option<f64> {
    if !settings.unit_scale.is_finite() || settings.unit_scale <= 0.0 {
        return None;
    }
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    let lower = trimmed.to_ascii_lowercase();

    // Check unit suffixes, matching longer suffixes before "m"
    const UNIT_CANDIDATES: [(LengthUnit, &str); 6] = [
        (LengthUnit::Mm, "mm"),
        (LengthUnit::Cm, "cm"),
        (LengthUnit::Km, "km"),
        (LengthUnit::In, "in"),
        (LengthUnit::Ft, "ft"),
        (LengthUnit::M, "m"),
    ];

    let mut matched_unit = None;
    let mut num_str = trimmed;

    for (unit, sym) in UNIT_CANDIDATES {
        if lower.ends_with(sym) {
            let prefix_len = trimmed.len() - sym.len();
            let prefix = trimmed[..prefix_len].trim();
            // Prefix must not be empty
            if !prefix.is_empty() {
                // Check if the prefix can parse as a float
                if let Ok(num) = prefix.parse::<f64>() {
                    if num.is_finite() {
                        matched_unit = Some(unit);
                        num_str = prefix;
                        break;
                    }
                }
            }
        }
    }

    let unit = matched_unit.unwrap_or(settings.display);
    let num = num_str.parse::<f64>().ok()?;
    if !num.is_finite() {
        return None;
    }

    let internal = num * unit.metres() / settings.unit_scale;
    if internal.is_finite() {
        Some(internal)
    } else {
        None
    }
}

/// Checks if a parameter of a given node type is a length parameter.
/// Ported from the old `minihou/web/static/length-params.js`.
pub fn is_length_param(node_type: &str, param_name: &str) -> bool {
    matches!(
        (node_type, param_name),
        ("point", "center")
            | ("box", "center" | "size")
            | ("sphere", "center" | "radius")
            | ("grid", "center" | "size_x" | "size_y")
            | ("tube", "center" | "radius_bottom" | "radius_top" | "height")
            | ("torus", "center" | "radius" | "thickness")
            | ("circle", "center" | "radius")
            | ("line", "origin" | "length")
            | ("helix", "radius" | "radius_end" | "height")
            | ("transform", "translate" | "pivot")
            | ("soft_transform", "translate" | "center" | "radius")
            | ("polybevel", "distance")
            | ("polyextrude", "distance" | "inset")
            | ("inset", "amount")
            | ("outset", "amount")
            | ("pushpull", "distance")
            | ("match_size", "size" | "center")
            | ("lattice_grid", "size" | "center")
            | ("mirror", "origin" | "distance")
            | ("clip", "origin")
            | ("fuse", "distance")
            | ("resample", "length")
            | ("fillet", "radius")
            | ("sweep", "radius")
            | ("curve_simplify", "tolerance")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_settings() {
        let s = UnitSettings::default();
        assert_eq!(s.display, LengthUnit::Mm);
        assert_eq!(s.unit_scale, 1.0);
        assert_eq!(s.internal_per_metre(), 1.0);
    }

    #[test]
    fn format_defaults_to_mm() {
        let s = UnitSettings::default();
        // 1.0 internal = 1.0 metre = 1000 mm
        assert_eq!(format(1.0, s), "1000 mm");
        assert_eq!(format(0.001, s), "1 mm");
        assert_eq!(format(0.0, s), "0 mm");
        assert_eq!(format(-1.5, s), "-1500 mm");
        assert_eq!(format(0.0005, s), "0.5 mm");
        assert_eq!(format(0.00001, s), "0.01 mm");
    }

    #[test]
    fn format_other_units() {
        let s_m = UnitSettings { display: LengthUnit::M, unit_scale: 1.0 };
        assert_eq!(format(1.0, s_m), "1 m");
        assert_eq!(format(2.5, s_m), "2.5 m");

        let s_cm = UnitSettings { display: LengthUnit::Cm, unit_scale: 1.0 };
        assert_eq!(format(1.0, s_cm), "100 cm");

        let s_in = UnitSettings { display: LengthUnit::In, unit_scale: 1.0 };
        // 0.0254 m = 1 in
        assert_eq!(format(0.0254, s_in), "1 in");

        let s_ft = UnitSettings { display: LengthUnit::Ft, unit_scale: 1.0 };
        // 0.3048 m = 1 ft
        assert_eq!(format(0.3048, s_ft), "1 ft");

        let s_km = UnitSettings { display: LengthUnit::Km, unit_scale: 1.0 };
        assert_eq!(format(1000.0, s_km), "1 km");
    }

    #[test]
    fn parse_bare_numbers_use_display_unit() {
        let s = UnitSettings::default(); // mm, scale 1.0
        // Bare 1000 = 1000 mm = 1.0 internal
        assert_eq!(parse("1000", s), Some(1.0));
        assert_eq!(parse("500", s), Some(0.5));
        assert_eq!(parse("-250", s), Some(-0.25));
        assert_eq!(parse("0", s), Some(0.0));
    }

    #[test]
    fn parse_with_explicit_units() {
        let s = UnitSettings::default(); // display is mm
        // Explicit "2.5 m" should parse as 2.5 internal units even though display is mm
        assert_eq!(parse("2.5 m", s), Some(2.5));
        assert_eq!(parse("2.5m", s), Some(2.5));
        assert_eq!(parse("30 cm", s), Some(0.3));
        assert_eq!(parse("1000 mm", s), Some(1.0));
        assert_eq!(parse("1 km", s), Some(1000.0));
        assert_eq!(parse("1 in", s), Some(0.0254));
        assert_eq!(parse("1 ft", s), Some(0.3048));
        assert_eq!(parse("-10 cm", s), Some(-0.1));
        assert_eq!(parse("+100 mm", s), Some(0.1));
    }

    #[test]
    fn parse_case_insensitivity_and_whitespace() {
        let s = UnitSettings::default();
        assert_eq!(parse("  1000 MM  ", s), Some(1.0));
        assert_eq!(parse("2.5 M", s), Some(2.5));
        assert_eq!(parse("30  CM", s), Some(0.3));
        assert_eq!(parse("12 In", s), Some(12.0 * 0.0254));
        assert_eq!(parse("3 FT", s), Some(3.0 * 0.3048));
    }

    #[test]
    fn parse_scientific_notation() {
        let s = UnitSettings::default();
        assert_eq!(parse("1e3 mm", s), Some(1.0));
        assert_eq!(parse("2.5e-2 m", s), Some(0.025));
        assert_eq!(parse("1e2", s), Some(0.1)); // 100 mm = 0.1 m
    }

    #[test]
    fn parse_invalid_returns_none() {
        let s = UnitSettings::default();
        assert_eq!(parse("", s), None);
        assert_eq!(parse("   ", s), None);
        assert_eq!(parse("abc", s), None);
        assert_eq!(parse("12 px", s), None);
        assert_eq!(parse("10 m m", s), None);
        assert_eq!(parse("10 mm extra", s), None);
        assert_eq!(parse("m", s), None);
        assert_eq!(parse("mm", s), None);
        assert_eq!(parse("NaN", s), None);
        assert_eq!(parse("inf", s), None);
    }

    #[test]
    fn round_trip() {
        let s = UnitSettings::default();
        for val in [0.0, 1.0, -1.0, 0.123, -4.56, 12.345] {
            let formatted = format(val, s);
            let parsed = parse(&formatted, s).unwrap();
            assert!((parsed - val).abs() < 1e-4, "roundtrip failed for {val}: got {parsed} from '{formatted}'");
        }

        let s_in = UnitSettings { display: LengthUnit::In, unit_scale: 1.0 };
        for val in [0.0254, 0.508, -0.254] {
            let formatted = format(val, s_in);
            let parsed = parse(&formatted, s_in).unwrap();
            assert!((parsed - val).abs() < 1e-4, "roundtrip failed for {val}: got {parsed} from '{formatted}'");
        }
    }

    #[test]
    fn custom_unit_scale() {
        // unit_scale = 0.01 means 1 internal unit is 0.01 m (1 cm = 10 mm)
        let s = UnitSettings { display: LengthUnit::Mm, unit_scale: 0.01 };
        assert_eq!(format(1.0, s), "10 mm");
        assert_eq!(parse("10 mm", s), Some(1.0));
        assert_eq!(parse("10", s), Some(1.0)); // bare 10 mm = 1.0 internal
    }

    #[test]
    fn length_params_table() {
        assert!(is_length_param("box", "size"));
        assert!(is_length_param("box", "center"));
        assert!(!is_length_param("box", "divisions"));
        assert!(!is_length_param("box", "rotate"));
        assert!(is_length_param("sphere", "radius"));
        assert!(is_length_param("transform", "translate"));
        assert!(is_length_param("transform", "pivot"));
        assert!(!is_length_param("transform", "rotate"));
        assert!(!is_length_param("transform", "scale"));
        assert!(is_length_param("tube", "radius_top"));
        assert!(is_length_param("tube", "height"));
        assert!(is_length_param("polybevel", "distance"));
        assert!(is_length_param("polyextrude", "distance"));
        assert!(is_length_param("polyextrude", "inset"));
        assert!(!is_length_param("noise", "frequency"));
    }
}
