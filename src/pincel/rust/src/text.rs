/// Greedy word wrap into lines no wider than `max_w`.
///
/// `width` measures a string without spaces, and `space_w` is the width of one space. Both use the same unit as `max_w`.
///
/// - `\n` always starts a new line.
/// - Words are separated by spaces, which are dropped at a wrap point but kept inside a line.
/// - A word wider than a whole line is split across lines, with at least one character on each.
pub fn wrap(text: &str, max_w: f32, mut width: impl FnMut(&str) -> f32, space_w: f32) -> Vec<String> {
    const EPSILON: f32 = 1e-3; // Widths are sums of floats, so "exactly fits" needs a little slack.
    let mut lines = Vec::new();

    for paragraph in text.split('\n') {
        let mut line = String::new();
        let mut line_w = 0.0;

        for word in paragraph.split(' ') {
            let word: String = word.chars().filter(|c| !c.is_control()).collect();
            let word_w = width(&word);

            // Where the word fits on the current line, put it there with a space before it (unless the line is empty).
            let needed = if line.is_empty() { word_w } else { line_w + space_w + word_w };
            if needed <= max_w + EPSILON {
                if !line.is_empty() {
                    line.push(' ');
                }
                line.push_str(&word);
                line_w = needed;
                continue;
            }

            // Otherwise wrap: finish the line, and hard-split the word if it can't fit even on its own line.
            if !line.is_empty() {
                lines.push(std::mem::take(&mut line));
            }
            let mut rest = word;
            while width(&rest) > max_w + EPSILON && rest.chars().count() > 1 {
                let split = longest_prefix(&rest, max_w + EPSILON, &mut width);
                lines.push(rest[..split].to_string());
                rest = rest[split..].to_string();
            }
            line_w = width(&rest);
            line = rest;
        }
        lines.push(line);
    }

    // A space left at the end of a line (where a wrap fell, or at the end of the text) has nothing after it to
    // separate, so it isn't part of the line, or its width.
    for line in &mut lines {
        line.truncate(line.trim_end_matches(' ').len());
    }
    lines
}

/// The byte length of the longest prefix that is no wider than `max_w`, which is at least one character.
fn longest_prefix(word: &str, max_w: f32, width: &mut impl FnMut(&str) -> f32) -> usize {
    let mut best = word.chars().next().map_or(0, char::len_utf8);
    for (i, c) in word.char_indices() {
        let end = i + c.len_utf8();
        if end > best && width(&word[..end]) <= max_w {
            best = end;
        } else if end > best {
            break;
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::wrap;

    /// Every character is one unit wide, so `cols` is the line length in characters.
    fn w(text: &str, cols: usize) -> Vec<String> {
        wrap(text, cols as f32, |word| word.chars().count() as f32, 1.0)
    }

    #[test]
    fn fits_on_one_line() {
        assert_eq!(w("hello world", 11), ["hello world"]);
    }

    #[test]
    fn wraps_at_word_boundaries() {
        assert_eq!(w("hello big world", 9), ["hello big", "world"]);
        assert_eq!(w("hello big world", 5), ["hello", "big", "world"]);
    }

    #[test]
    fn splits_words_longer_than_a_line() {
        assert_eq!(w("abcdefgh", 3), ["abc", "def", "gh"]);
        assert_eq!(w("ab abcdefg", 4), ["ab", "abcd", "efg"]);
    }

    #[test]
    fn trailing_spaces_are_not_part_of_a_line() {
        assert_eq!(w("ab   cd", 4), ["ab", "cd"], "the spaces where it wrapped");
        assert_eq!(w("ab ", 10), ["ab"], "and at the end");
        assert_eq!(w("a  b", 10), ["a  b"], "while spaces inside a line stay");
        assert_eq!(w("a \nb", 10), ["a", "b"], "and before a line break");
    }

    #[test]
    fn newlines_are_hard_breaks() {
        assert_eq!(w("a\n\nb", 10), ["a", "", "b"]);
    }

    #[test]
    fn zero_width_still_makes_progress() {
        assert_eq!(w("ab", 0), ["a", "b"]);
    }

    #[test]
    fn proportional_widths_pack_more_narrow_letters() {
        // 'i' is 1 wide and 'M' is 3 wide.
        let width = |word: &str| word.chars().map(|c| if c == 'i' { 1.0 } else { 3.0 }).sum();
        assert_eq!(wrap("iiii iiii MM", 9.0, width, 1.0), ["iiii iiii", "MM"]);
        assert_eq!(wrap("MM MM", 9.0, width, 1.0), ["MM", "MM"]);
    }

    #[test]
    fn multibyte_characters_split_on_boundaries() {
        assert_eq!(w("ééé", 2), ["éé", "é"]);
    }
}
