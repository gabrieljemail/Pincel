//! Text made of runs that each look a little different (a color, bold, italic, underline), wrapped together as one
//! piece of text. This is what a chat line is: a sentence in several colors that has to wrap as a sentence, not as
//! separate boxes.
//!
//! The wrapping is the same as for plain text: lines break at spaces, `\n` always breaks, and a word wider than a
//! whole line is split. The difference is that a word can be made of pieces from different spans (`Hel` in red and
//! `lo` in green is one word, which won't be broken between the colors), and that every piece remembers its span.

use crate::element::TextSpan;
use crate::font::{FontKey, Fonts};
use crate::style::{Color, TextStyle};

/// Widths are sums of floats, so "exactly fits" needs a little slack.
const EPSILON: f32 = 1e-3;

/// What a span looks like once the text style it inherits is filled in.
#[derive(Clone, Copy, Debug)]
pub struct SpanStyle {
    pub font: FontKey,
    pub color: Color,
    pub ascent: i32,
    pub underline: bool,
    pub strikethrough: bool,
}

/// A stretch of one line from one span: some text, or a space (which is drawn as nothing, but can be underlined).
#[derive(Clone, Debug, PartialEq)]
pub struct Piece {
    /// Distance from the start of the line, in pixels.
    pub x: f32,
    pub width: f32,
    /// Which span it is from.
    pub span: usize,
    /// The text, or `None` for a space.
    pub text: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct RichLine {
    pub pieces: Vec<Piece>,
    pub width: f32,
}

impl RichLine {
    /// The text of the line, with spaces as spaces.
    pub fn plain(&self) -> String {
        self.pieces.iter().map(|p| p.text.as_deref().unwrap_or(" ")).collect()
    }

    fn push(&mut self, span: usize, width: f32, text: Option<String>) {
        self.pieces.push(Piece { x: self.width, width, span, text });
        self.width += width;
    }
}

pub struct RichLayout {
    pub lines: Vec<RichLine>,
    /// The height of every line, and how far down the baseline is in one. They are the same for every line, so
    /// a line of bold text and a line of regular text sit on an even grid, and are the largest any span needs.
    pub line_height: i32,
    pub ascent: i32,
    /// One per span, in the same order.
    pub styles: Vec<SpanStyle>,
}

impl RichLayout {
    pub fn widest(&self) -> f32 {
        self.lines.iter().map(|l| l.width).fold(0.0, f32::max)
    }
}

enum Token {
    /// Pieces of one word: the characters up to the next space, each with the span it came from.
    Word(Vec<(usize, String)>),
    Space(usize),
    Break,
}

fn tokenize(spans: &[TextSpan]) -> Vec<Token> {
    let mut tokens = Vec::new();
    let mut word: Vec<(usize, String)> = Vec::new();
    let flush = |word: &mut Vec<(usize, String)>, tokens: &mut Vec<Token>| {
        if !word.is_empty() {
            tokens.push(Token::Word(std::mem::take(word)));
        }
    };

    for (i, span) in spans.iter().enumerate() {
        for c in span.text.chars() {
            match c {
                ' ' => {
                    flush(&mut word, &mut tokens);
                    tokens.push(Token::Space(i));
                }
                '\n' => {
                    flush(&mut word, &mut tokens);
                    tokens.push(Token::Break);
                }
                c if c.is_control() => {}
                c => match word.last_mut() {
                    Some((span, text)) if *span == i => text.push(c),
                    _ => word.push((i, c.to_string())),
                },
            }
        }
    }
    flush(&mut word, &mut tokens);
    tokens
}

/// Lays `spans` out in lines no wider than `max_w`, in the font family and size of `base`.
pub fn layout_rich(spans: &[TextSpan], base: &TextStyle, max_w: f32, fonts: &mut Fonts) -> RichLayout {
    let size = base.font_size;
    let base_metrics = fonts.line_metrics(FontKey::new(base.font_family, base.font_weight, base.font_style), size);

    let styles: Vec<SpanStyle> = spans
        .iter()
        .map(|span| {
            let font = FontKey::new(base.font_family, span.font_weight.unwrap_or(base.font_weight), span.font_style.unwrap_or(base.font_style));
            SpanStyle {
                font,
                color: span.foreground.unwrap_or(base.foreground),
                ascent: fonts.line_metrics(font, size).ascent,
                underline: span.underline,
                strikethrough: span.strikethrough,
            }
        })
        .collect();
    let line_height = styles.iter().map(|s| fonts.line_metrics(s.font, size).height).fold(base_metrics.height, i32::max);
    let ascent = styles.iter().map(|s| s.ascent).fold(base_metrics.ascent, i32::max);

    let mut lines: Vec<RichLine> = Vec::new();
    let mut line = RichLine::default();
    let mut spaces: Vec<(usize, f32)> = Vec::new(); // Spaces since the last word, which only count if the next word fits.

    for token in tokenize(spans) {
        match token {
            Token::Break => {
                lines.push(std::mem::take(&mut line));
                spaces.clear();
            }
            Token::Space(span) => {
                // A space at the start of a line isn't kept, as in plain text.
                if !line.pieces.is_empty() {
                    spaces.push((span, fonts.text_width(styles[span].font, size, " ")));
                }
            }
            Token::Word(frags) => {
                let widths: Vec<f32> = frags.iter().map(|(span, text)| fonts.text_width(styles[*span].font, size, text)).collect();
                let word_w: f32 = widths.iter().sum();
                let spaces_w: f32 = spaces.iter().map(|s| s.1).sum();
                let needed = if line.pieces.is_empty() { word_w } else { line.width + spaces_w + word_w };

                if needed <= max_w + EPSILON {
                    for (span, width) in spaces.drain(..) {
                        line.push(span, width, None);
                    }
                    for ((span, text), width) in frags.into_iter().zip(widths) {
                        line.push(span, width, Some(text));
                    }
                } else {
                    spaces.clear();
                    if !line.pieces.is_empty() {
                        lines.push(std::mem::take(&mut line));
                    }
                    if word_w <= max_w + EPSILON {
                        for ((span, text), width) in frags.into_iter().zip(widths) {
                            line.push(span, width, Some(text));
                        }
                    } else {
                        split_long_word(frags, &styles, size, max_w, fonts, &mut lines, &mut line);
                    }
                }
                spaces.clear();
            }
        }
    }
    lines.push(line);

    RichLayout { lines, line_height, ascent, styles }
}

/// A word wider than a whole line, which goes across lines a character at a time (at least one on each).
fn split_long_word(frags: Vec<(usize, String)>, styles: &[SpanStyle], size: u16, max_w: f32, fonts: &mut Fonts, lines: &mut Vec<RichLine>, line: &mut RichLine) {
    for (span, text) in frags {
        let key = styles[span].font;
        let mut piece = String::new();
        for c in text.chars() {
            let mut candidate = piece.clone();
            candidate.push(c);
            let fits = line.width + fonts.text_width(key, size, &candidate) <= max_w + EPSILON;
            if !fits && (!piece.is_empty() || !line.pieces.is_empty()) {
                // Out of room: close what is on the line, and start the next with this character.
                if !piece.is_empty() {
                    let width = fonts.text_width(key, size, &piece);
                    line.push(span, width, Some(std::mem::take(&mut piece)));
                }
                lines.push(std::mem::take(line));
            }
            piece.push(c);
        }
        if !piece.is_empty() {
            let width = fonts.text_width(key, size, &piece);
            line.push(span, width, Some(piece));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::style::FontFamily;
    use crate::text::wrap;

    const RED: Color = Color { r: 255, g: 0, b: 0, a: 1.0 };
    const GREEN: Color = Color { r: 0, g: 255, b: 0, a: 1.0 };

    fn mono() -> TextStyle {
        TextStyle { font_family: FontFamily::Mono, font_size: 16, ..TextStyle::default() }
    }

    fn span(text: &str) -> TextSpan {
        TextSpan { text: text.to_string(), ..TextSpan::default() }
    }

    fn colored(text: &str, color: Color) -> TextSpan {
        TextSpan { foreground: Some(color), ..span(text) }
    }

    /// The width of `n` mono characters.
    fn cols(fonts: &mut Fonts, n: usize) -> f32 {
        n as f32 * fonts.text_width(FontKey::regular(FontFamily::Mono), 16, "m")
    }

    fn texts(layout: &RichLayout) -> Vec<String> {
        layout.lines.iter().map(RichLine::plain).collect()
    }

    #[test]
    fn spans_wrap_together_as_one_sentence() {
        let mut fonts = Fonts::new();
        let spans = [colored("hello ", RED), colored("big ", GREEN), span("world")];
        let max = cols(&mut fonts, 9);
        let layout = layout_rich(&spans, &mono(), max, &mut fonts);
        assert_eq!(texts(&layout), ["hello big", "world"], "the same lines as if it were one string");
    }

    #[test]
    fn it_wraps_exactly_like_plain_text_for_one_span() {
        let mut fonts = Fonts::new();
        let sentence = "the quick brown fox jumps over the lazy dog  and  again";
        let key = FontKey::regular(FontFamily::Mono);
        let space = fonts.text_width(key, 16, " ");
        for width in [3usize, 7, 11, 19, 40] {
            let max = cols(&mut fonts, width);
            let plain = wrap(sentence, max, |w| fonts.text_width(key, 16, w), space);
            let rich = layout_rich(&[span(sentence)], &mono(), max, &mut fonts);
            assert_eq!(texts(&rich), plain, "at {width} columns");
        }
    }

    #[test]
    fn a_word_split_between_spans_is_not_broken_between_them() {
        let mut fonts = Fonts::new();
        // "Hel" and "lo" are one word, in two colors. With room for 8 columns, "big Hello" (9) wraps before the word.
        let spans = [span("big "), colored("Hel", RED), colored("lo", GREEN)];
        let max = cols(&mut fonts, 8);
        let layout = layout_rich(&spans, &mono(), max, &mut fonts);
        assert_eq!(texts(&layout), ["big", "Hello"]);
        let second = &layout.lines[1].pieces;
        assert_eq!((second[0].span, second[1].span), (1, 2), "and it keeps its two colors");
        assert!((second[1].x - second[0].width).abs() < 1e-3, "the second piece follows the first directly");
    }

    #[test]
    fn newlines_break_and_an_empty_line_is_kept() {
        let mut fonts = Fonts::new();
        let layout = layout_rich(&[span("a\n\nb")], &mono(), 1000.0, &mut fonts);
        assert_eq!(texts(&layout), ["a", "", "b"]);
        let layout = layout_rich(&[span("x"), span("\ny")], &mono(), 1000.0, &mut fonts);
        assert_eq!(texts(&layout), ["x", "y"], "a newline at the start of a span breaks too");
    }

    #[test]
    fn no_text_is_one_empty_line() {
        let mut fonts = Fonts::new();
        assert_eq!(texts(&layout_rich(&[], &mono(), 100.0, &mut fonts)), [""]);
        assert_eq!(texts(&layout_rich(&[span("")], &mono(), 100.0, &mut fonts)), [""]);
    }

    #[test]
    fn a_word_wider_than_a_line_is_split_across_its_spans() {
        let mut fonts = Fonts::new();
        let spans = [colored("abcd", RED), colored("efgh", GREEN)];
        let max = cols(&mut fonts, 3);
        let layout = layout_rich(&spans, &mono(), max, &mut fonts);
        assert_eq!(texts(&layout), ["abc", "def", "gh"]);
        assert_eq!(layout.lines[1].pieces.iter().map(|p| p.span).collect::<Vec<_>>(), [0, 1], "\"def\" is the end of red, then green");
    }

    #[test]
    fn a_line_always_makes_progress_however_narrow() {
        let mut fonts = Fonts::new();
        let layout = layout_rich(&[span("ab")], &mono(), 0.0, &mut fonts);
        assert_eq!(texts(&layout), ["a", "b"]);
    }

    #[test]
    fn spaces_are_dropped_at_a_wrap_and_at_the_start_but_kept_inside_a_line() {
        let mut fonts = Fonts::new();
        let layout = layout_rich(&[span("  a  b")], &mono(), 1000.0, &mut fonts);
        assert_eq!(texts(&layout), ["a  b"], "no leading spaces, but the two between kept");
        let max = cols(&mut fonts, 4);
        let layout = layout_rich(&[span("abc   def")], &mono(), max, &mut fonts);
        assert_eq!(texts(&layout), ["abc", "def"], "the spaces at the wrap are gone");
    }

    #[test]
    fn pieces_are_placed_end_to_end_and_the_line_width_is_their_sum() {
        let mut fonts = Fonts::new();
        let layout = layout_rich(&[colored("ab ", RED), span("cd")], &mono(), 1000.0, &mut fonts);
        let line = &layout.lines[0];
        let mut x = 0.0;
        for piece in &line.pieces {
            assert!((piece.x - x).abs() < 1e-3);
            x += piece.width;
        }
        assert!((line.width - x).abs() < 1e-3);
        assert!(line.pieces.iter().any(|p| p.text.is_none()), "the space is a piece of its own, so it can be underlined");
    }

    #[test]
    fn bold_and_italic_spans_change_the_font_and_the_width() {
        let mut fonts = Fonts::new();
        let base = TextStyle { font_family: FontFamily::Sans, font_size: 20, ..TextStyle::default() };
        let bold = TextSpan { font_weight: Some(700), ..span("Measure this") };
        let plain = layout_rich(&[span("Measure this")], &base, 10_000.0, &mut fonts);
        let heavy = layout_rich(&[bold], &base, 10_000.0, &mut fonts);
        assert!(heavy.widest() > plain.widest(), "bold is wider");
        assert!(heavy.styles[0].font.bold && !plain.styles[0].font.bold);

        let italic = TextSpan { font_style: Some(crate::style::FontStyle::Italic), ..span("x") };
        assert!(layout_rich(&[italic], &base, 100.0, &mut fonts).styles[0].font.italic);
    }

    #[test]
    fn the_line_height_is_the_tallest_any_span_needs_and_the_same_for_every_line() {
        let mut fonts = Fonts::new();
        let base = TextStyle { font_family: FontFamily::Sans, font_size: 20, ..TextStyle::default() };
        let spans = [span("regular"), TextSpan { font_weight: Some(700), ..span("bold") }];
        let layout = layout_rich(&spans, &base, 10_000.0, &mut fonts);
        let regular = fonts.line_metrics(FontKey::regular(FontFamily::Sans), 20);
        let bold = fonts.line_metrics(FontKey::new(FontFamily::Sans, 700, crate::style::FontStyle::Normal), 20);
        assert_eq!(layout.line_height, regular.height.max(bold.height));
        assert_eq!(layout.ascent, regular.ascent.max(bold.ascent));
    }

    #[test]
    fn a_span_without_a_color_uses_the_inherited_one() {
        let mut fonts = Fonts::new();
        let base = TextStyle { foreground: GREEN, ..mono() };
        let layout = layout_rich(&[span("a"), colored("b", RED)], &base, 100.0, &mut fonts);
        assert_eq!((layout.styles[0].color, layout.styles[1].color), (GREEN, RED));
    }

    #[test]
    fn control_characters_are_dropped() {
        let mut fonts = Fonts::new();
        assert_eq!(texts(&layout_rich(&[span("a\tb\u{7}c")], &mono(), 1000.0, &mut fonts)), ["abc"]);
    }
}
