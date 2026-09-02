//! Terminal text measurement.
//!
//! Width in a terminal is not `str::len()` and not `chars().count()`: it
//! is the sum of [`unicode_width`] scores over **grapheme clusters**.
//! Splitting on `char` instead would score a combining mark or a ZWJ
//! emoji sequence as several cells, and the resulting bug presents as a
//! layout bug rather than a text bug — which is why this is its own
//! module with its own tests.

use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

/// The number of cells `text` occupies on one line.
///
/// A cluster with no width of its own (a combining mark, a variation
/// selector) contributes nothing; a wide cluster (CJK, most emoji)
/// contributes two.
pub fn text_width(text: &str) -> u16 {
    text.graphemes(true)
        .map(|cluster| cluster.width().max(cluster_floor(cluster)) as u16)
        .sum()
}

/// A cluster whose scalars are all zero-width still occupies no cells,
/// but a cluster that *starts* with a printable scalar always occupies
/// at least one — `unicode-width` scores some emoji-with-modifier
/// sequences as 0 when summed naively.
fn cluster_floor(cluster: &str) -> usize {
    match cluster.chars().next() {
        Some(first) if first.is_control() => 0,
        Some(first) if unicode_width::UnicodeWidthChar::width(first).unwrap_or(0) > 0 => 1,
        _ => 0,
    }
}

/// Break `text` into lines no wider than `width` cells.
///
/// Breaks at ASCII whitespace where it can and mid-cluster-run where it
/// cannot (a single word longer than the line). Returns at least one
/// line, so an empty string still occupies one row — matching what a
/// terminal actually shows.
pub fn wrap(text: &str, width: u16) -> Vec<String> {
    if width == 0 {
        return vec![String::new()];
    }
    let mut lines = Vec::new();
    for hard_line in text.split('\n') {
        let mut current = String::new();
        let mut current_width = 0u16;
        for word in hard_line.split_whitespace() {
            let word_width = text_width(word);
            let space = u16::from(!current.is_empty());
            if current_width + space + word_width <= width {
                if space == 1 {
                    current.push(' ');
                }
                current.push_str(word);
                current_width += space + word_width;
                continue;
            }
            if !current.is_empty() {
                lines.push(std::mem::take(&mut current));
                current_width = 0;
            }
            if word_width <= width {
                current.push_str(word);
                current_width = word_width;
            } else {
                // A single word wider than the line: split it by cluster.
                for cluster in word.graphemes(true) {
                    let cluster_width = text_width(cluster);
                    if current_width + cluster_width > width {
                        lines.push(std::mem::take(&mut current));
                        current_width = 0;
                    }
                    current.push_str(cluster);
                    current_width += cluster_width;
                }
            }
        }
        lines.push(current);
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ascii_is_one_cell_per_character() {
        assert_eq!(text_width("hello"), 5);
    }

    #[test]
    fn east_asian_wide_characters_are_two_cells() {
        assert_eq!(text_width("日本語"), 6);
        assert_eq!(text_width("a日b"), 4);
    }

    #[test]
    fn combining_marks_do_not_add_width() {
        // "e" + U+0301 COMBINING ACUTE ACCENT is one cluster, one cell.
        assert_eq!(text_width("e\u{0301}"), 1);
        assert_eq!(text_width("e\u{0301}e\u{0301}"), 2);
    }

    #[test]
    fn a_zwj_emoji_sequence_is_one_cluster() {
        // Family emoji: four scalars joined by ZWJ. Counting `char`s
        // would score this far too wide.
        let family = "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}";
        assert_eq!(family.chars().count(), 5);
        assert_eq!(text_width(family), 2);
    }

    #[test]
    fn wrapping_breaks_on_whitespace() {
        assert_eq!(wrap("one two three", 7), vec!["one two", "three"]);
    }

    #[test]
    fn wrapping_splits_a_word_longer_than_the_line() {
        assert_eq!(wrap("abcdefgh", 3), vec!["abc", "def", "gh"]);
    }

    #[test]
    fn wrapping_respects_wide_clusters() {
        // Three double-width characters do not fit in five cells.
        assert_eq!(wrap("日本語", 5), vec!["日本", "語"]);
    }

    #[test]
    fn an_empty_string_still_occupies_one_row() {
        assert_eq!(wrap("", 10), vec![String::new()]);
    }
}
