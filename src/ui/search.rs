//! Literal search shared by loaded grids and live SQL drafts.
use std::ops::Range;

pub fn literal_ranges(
    text: &str,
    query: &str,
    case_sensitive: bool,
    limit: usize,
) -> Vec<Range<usize>> {
    if query.is_empty() {
        return vec![];
    }
    if case_sensitive {
        return text
            .match_indices(query)
            .take(limit)
            .map(|(start, value)| start..start + value.len())
            .collect();
    }
    // Keep original UTF-8 offsets even when lowercase expands a character (İ).
    let mut folded = String::with_capacity(text.len());
    let mut boundaries = Vec::with_capacity(text.chars().count() + 1);
    for (offset, ch) in text.char_indices() {
        boundaries.push((folded.len(), offset));
        folded.extend(ch.to_lowercase());
    }
    boundaries.push((folded.len(), text.len()));
    let query: String = query.chars().flat_map(char::to_lowercase).collect();
    folded
        .match_indices(&query)
        .filter_map(|(start, value)| {
            let a = boundaries
                .binary_search_by_key(&start, |(folded, _)| *folded)
                .ok()?;
            let b = boundaries
                .binary_search_by_key(&(start + value.len()), |(folded, _)| *folded)
                .ok()?;
            Some(boundaries[a].1..boundaries[b].1)
        })
        .take(limit)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn literal_unicode_multiline_offsets() {
        assert_eq!(literal_ranges("ΟΣ", "ΟΣ", false, 10), vec![0..4]);
        let text = "é İ\nÄ.* ä.*";
        assert_eq!(literal_ranges(text, "ä.*", false, 100), vec![6..10, 11..15]);
        assert_eq!(literal_ranges(text, "ä.*", true, 100), vec![11..15]);
        assert_eq!(literal_ranges(text, "İ\nÄ", false, 100), vec![3..8]);
        assert!(literal_ranges(text, "", false, 100).is_empty());
        assert!(literal_ranges(text, "missing", false, 100).is_empty());
        assert_eq!(literal_ranges("aaaa", "aa", true, 100), vec![0..2, 2..4]);
    }
}
