//! Case-insensitive subsequence ranking for detected input group names.
use eframe::egui;
use serde_json::{json, Value};

/// Searchable menu shared by input group and attribute fields.
pub fn dropdown(ui: &mut egui::Ui, value: &mut Value, label: &str, allow_pattern: bool, names: impl FnOnce() -> Vec<String>) {
    ui.menu_button("▼", |ui| {
        ui.set_min_width(180.);
        let search_id = ui.id().with("detected_name_search");
        let mut search = ui.ctx().data(|d| d.get_temp::<String>(search_id)).unwrap_or_default();
        let search_response = ui.add(egui::TextEdit::singleline(&mut search).hint_text(format!("Search {label}")));
        ui.ctx().data_mut(|d| d.insert_temp(search_id, search.clone()));
        let names = names();
        let wildcard = allow_pattern && has_wildcard(&search);
        if wildcard && (ui.button(format!("Use pattern: {search}")).clicked()
            || search_response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter))) {
            *value = json!(search);
            ui.close();
        }
        if names.is_empty() { ui.label(format!("No {label} on input")); }
        else {
            let matches = matching_names(names.iter().map(String::as_str), &search, wildcard);
            if matches.is_empty() { ui.label(format!("No matching {label}")); }
            egui::ScrollArea::vertical().max_height(240.).show(ui, |ui| {
                for name in matches {
                    if ui.button(name).clicked() { *value = json!(name); ui.close(); }
                }
            });
        }
    }).response.on_hover_text(format!("{label} detected on the input"));
}

fn has_wildcard(query: &str) -> bool { query.contains(['*', '?', '[']) }

fn matching_names<'a>(names: impl IntoIterator<Item = &'a str>, query: &str, wildcard: bool) -> Vec<&'a str> {
    if wildcard {
        let mut matches: Vec<_> = names.into_iter().filter(|name| minihou_core::group_pattern::matches(query, name)).collect();
        matches.sort_unstable();
        matches
    } else { ranked_matches(names, query) }
}

/// Return matching names with the closest, earliest matches first.
pub fn ranked_matches<'a>(names: impl IntoIterator<Item = &'a str>, query: &str) -> Vec<&'a str> {
    let needle: Vec<char> = query.chars().flat_map(char::to_lowercase).collect();
    let mut matches: Vec<_> = names.into_iter().filter_map(|name| {
        let haystack: Vec<char> = name.chars().flat_map(char::to_lowercase).collect();
        let score = if needle.is_empty() {
            (0, 0, 0)
        } else {
            haystack.iter().enumerate().filter(|(_, c)| **c == needle[0]).filter_map(|(start, _)| {
                let mut next = start + 1;
                for letter in needle.iter().skip(1) {
                    let offset = haystack[next..].iter().position(|c| c == letter)?;
                    next += offset + 1;
                }
                Some((next - start - needle.len(), start, haystack.len()))
            }).min()?
        };
        Some((score, name))
    }).collect();
    matches.sort_by(|(a_score, a_name), (b_score, b_name)|
        a_score.cmp(b_score).then_with(|| a_name.to_lowercase().cmp(&b_name.to_lowercase())));
    matches.into_iter().map(|(_, name)| name).collect()
}

#[cfg(test)]
mod tests {
    use super::{matching_names, ranked_matches};

    #[test]
    fn subsequence_is_case_insensitive() {
        assert_eq!(ranked_matches(["FrontGroup", "Back", "f_r_o_n_t"], "FRG"), ["FrontGroup"]);
    }

    #[test]
    fn compact_and_early_matches_rank_first() {
        assert_eq!(ranked_matches(["xxab", "a___b", "ab", "abc"], "ab"),
            ["ab", "abc", "xxab", "a___b"]);
    }

    #[test]
    fn prefers_later_compact_subsequence_over_early_scattered_one() {
        assert_eq!(ranked_matches(["a---b---cabc", "axbyc"], "abc"), ["a---b---cabc", "axbyc"]);
    }

    #[test]
    fn empty_query_keeps_names_in_alphabetical_order() {
        assert_eq!(ranked_matches(["zeta", "Alpha", "beta"], ""), ["Alpha", "beta", "zeta"]);
    }

    #[test]
    fn wildcard_search_uses_group_pattern_semantics() {
        assert_eq!(matching_names(["front_1", "front_2", "back_1"], "front_*", true), ["front_1", "front_2"]);
        assert_eq!(matching_names(["front_1", "front_2", "front_3"], "front_[12]", true), ["front_1", "front_2"]);
    }
}
