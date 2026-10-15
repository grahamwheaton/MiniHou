//! Deterministic display reduction for indexed viewport geometry.

pub const LEVELS: [(&str, &str, f32, bool); 7] = [
    ("full", "Full", 1.0, false),
    ("auto", "Auto", 0.1, true),
    ("0.5", "50%", 0.5, false),
    ("0.25", "25%", 0.25, false),
    ("0.1", "10%", 0.1, false),
    ("0.05", "5%", 0.05, false),
    ("0.01", "1%", 0.01, false),
];

/// Fraction that fits a triangle budget. An empty mesh needs no reduction.
pub fn auto_level(triangles: usize, budget: usize) -> f32 {
    if triangles == 0 { 1.0 } else { (budget as f64 / triangles as f64).min(1.0) as f32 }
}

fn reduced_groups(indices: &[u32], level: f32, width: usize) -> Vec<u32> {
    let count = indices.len() / width;
    if count == 0 || level.is_nan() || level <= 0.0 { return Vec::new(); }
    let complete = &indices[..count * width];
    if level >= 1.0 { return complete.to_vec(); }
    let keep = ((count as f64 * level as f64).floor() as usize).max(1);
    if keep >= count { return complete.to_vec(); }
    let mut out = Vec::with_capacity(keep * width);
    for i in 0..keep {
        let group = if keep == 1 { 0 } else { i * (count - 1) / (keep - 1) };
        out.extend_from_slice(&complete[group * width..(group + 1) * width]);
    }
    out
}

/// Keep complete triangles, starting with the first and spacing the rest through the last.
pub fn reduced_triangles(indices: &[u32], level: f32) -> Vec<u32> { reduced_groups(indices, level, 3) }

/// Keep complete line pairs with the same sampling rule as triangles.
pub fn reduced_edges(indices: &[u32], level: f32) -> Vec<u32> { reduced_groups(indices, level, 2) }

pub fn status_text(shown_tris: usize, total_tris: usize, _level: f32, _auto: bool) -> String {
    format!("Reduced view: {} of {} triangles", grouped(shown_tris), grouped(total_tris))
}

fn grouped(n: usize) -> String {
    let s = n.to_string();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) { out.push(','); }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn old_editor_levels_are_preserved() {
        assert_eq!(LEVELS.map(|r| r.0), ["full", "auto", "0.5", "0.25", "0.1", "0.05", "0.01"]);
        assert_eq!(LEVELS[1], ("auto", "Auto", 0.1, true));
    }

    #[test]
    fn auto_fits_budget_and_handles_empty_inputs() {
        assert_eq!(auto_level(48_000, 12_000), 0.25);
        assert_eq!(auto_level(12, 100), 1.0);
        assert_eq!(auto_level(12, 0), 0.0);
        assert_eq!(auto_level(0, 0), 1.0);
    }

    #[test]
    fn triangles_spread_without_splitting() {
        let indices: Vec<u32> = (0..30).collect();
        assert_eq!(reduced_triangles(&indices, 0.3), [0, 1, 2, 12, 13, 14, 27, 28, 29]);
        assert_eq!(reduced_triangles(&indices, 0.01), [0, 1, 2]);
        for level in [0.0, f32::NAN, -1.0] { assert!(reduced_triangles(&indices, level).is_empty()); }
        assert_eq!(reduced_triangles(&indices, 2.0), indices);
        assert_eq!(reduced_triangles(&indices, f32::INFINITY), indices);
        assert_eq!(reduced_triangles(&[0, 1, 2, 99], 1.0), [0, 1, 2]);
    }

    #[test]
    fn edges_spread_without_splitting() {
        let indices: Vec<u32> = (0..20).collect();
        assert_eq!(reduced_edges(&indices, 0.3), [0, 1, 8, 9, 18, 19]);
        for level in [0.0, f32::NAN, -1.0] { assert!(reduced_edges(&indices, level).is_empty()); }
        assert_eq!(reduced_edges(&indices, 1.5), indices);
        assert_eq!(reduced_edges(&indices, f32::INFINITY), indices);
        assert_eq!(reduced_edges(&[0, 1, 99], 1.0), [0, 1]);
    }

    #[test]
    fn status_uses_old_editor_wording() {
        assert_eq!(status_text(12_000, 48_000, 0.25, false), "Reduced view: 12,000 of 48,000 triangles");
        assert_eq!(status_text(1, 1_000_000, 0.1, true), "Reduced view: 1 of 1,000,000 triangles");
    }
}
