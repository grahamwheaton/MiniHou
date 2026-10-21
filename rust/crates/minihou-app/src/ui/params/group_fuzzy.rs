//! Case-insensitive subsequence ranking for detected input group names.

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
    use super::ranked_matches;

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
}
