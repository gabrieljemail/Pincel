//! The editing behavior of an Input element: a value, a caret and a selection, and what keys, typed text and the mouse
//! do to them. This has no idea how it is drawn, so it can be tested on its own. Layout and the rasterizer draw it.
//!
//! Not supported yet: undo, and selecting by double or triple click.

use crate::clipboard::Clipboard;
use crate::input::{Key, KeyAction, Keyboard};
use crate::style::Color;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum InputKind {
    /// One line. Enter submits it.
    #[default]
    Text,
    /// Several lines, split on `\n`. Enter starts a new line.
    Multiline,
    /// One line that only takes what a decimal number is made of: digits, one `.`, and a `-` at the start.
    /// Enter submits it. Pincel doesn't parse it, so read the text and convert it.
    Number,
}

/// Where an input's text was drawn the last time it was laid out, which is what turns a mouse position into a place
/// in the text. Layout fills it in, because it is the one that knows how wide each character is.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Geometry {
    /// Where the content box is, from the top-left of the element's border box.
    pub content_dx: i32,
    pub content_dy: i32,
    pub line_height: i32,
    /// The first line that is showing, and how far the text is scrolled left.
    pub first_row: usize,
    pub scroll_x: i32,
    /// For each row (a line of the value, or a piece of one when the input wraps), how far from the left of the
    /// content box each character starts, plus the row's end: one more number than the row has characters.
    pub columns: Vec<Vec<f32>>,
    /// The character each row starts at.
    pub row_starts: Vec<usize>,
}

/// A run of an input's value that is drawn in its own color, as character positions: `start` up to `end`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Highlight {
    pub start: usize,
    pub end: usize,
    pub color: Color,
}

#[derive(Clone, Debug, PartialEq)]
pub struct InputState {
    pub kind: InputKind,
    pub value: String,
    /// Shown, dimmed, while the value is empty.
    pub placeholder: String,
    /// The most characters the value can have. `None` for no limit.
    pub max_chars: Option<usize>,
    /// Where the caret is, as a number of characters from the start (so it is between characters, and
    /// `0..=` the length of the value).
    pub caret: usize,
    /// The other end of the selection, which is the caret's end when it is a selection of nothing. The selection
    /// is everything between this and the caret.
    pub anchor: usize,

    /// The user changed the value during the last frame.
    pub changed: bool,
    /// The user pressed Enter in a single-line input during the last frame.
    pub submitted: bool,

    /// How far the text is scrolled left, and how many lines it is scrolled up, so the caret stays in view.
    /// Layout keeps these up to date, because it is the one that knows how wide the text is.
    pub scroll_x: i32,
    pub scroll_line: i32,

    /// Whether a line too long for the box carries on in a row below it, instead of scrolling sideways. The input
    /// grows taller to hold the rows. Rows after the first are drawn indented by the width of `wrap_marker`, which
    /// is shown dimmed in the indent.
    pub wrap: bool,
    pub wrap_marker: String,
    /// Runs of the value drawn in their own color. What they don't cover has the text color. A later run wins.
    pub highlights: Vec<Highlight>,

    pub geometry: Geometry,
}

/// Whether `c` is part of a word, for moving and deleting by word.
fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

impl InputState {
    pub fn new(kind: InputKind) -> InputState {
        InputState {
            kind,
            value: String::new(),
            placeholder: String::new(),
            max_chars: None,
            caret: 0,
            anchor: 0,
            changed: false,
            submitted: false,
            scroll_x: 0,
            scroll_line: 0,
            wrap: false,
            wrap_marker: String::new(),
            highlights: Vec::new(),
            geometry: Geometry::default(),
        }
    }

    /// The number of characters in the value.
    pub fn len(&self) -> usize {
        self.value.chars().count()
    }

    pub fn is_empty(&self) -> bool {
        self.value.is_empty()
    }

    /// The byte position in `value` of the character at `index` (or the end, for an index past the last one).
    fn byte_at(&self, index: usize) -> usize {
        self.value.char_indices().nth(index).map_or(self.value.len(), |(byte, _)| byte)
    }

    /// The selected range as `(start, end)` in characters, or `None` if nothing is selected.
    pub fn selection(&self) -> Option<(usize, usize)> {
        (self.anchor != self.caret).then(|| (self.anchor.min(self.caret), self.anchor.max(self.caret)))
    }

    pub fn selected_text(&self) -> String {
        self.selection().map_or_else(String::new, |(a, b)| self.value[self.byte_at(a)..self.byte_at(b)].to_string())
    }

    /// Selects from `start` to `end`, with the caret at `end`. Both are clamped to the text.
    pub fn select(&mut self, start: usize, end: usize) {
        let len = self.len();
        self.anchor = start.min(len);
        self.caret = end.min(len);
    }

    pub fn select_all(&mut self) {
        self.select(0, self.len());
    }

    /// Moves the caret, and with `extend` keeps the other end of the selection where it is, otherwise drops it.
    fn move_to(&mut self, target: usize, extend: bool) {
        self.caret = target.min(self.len());
        if !extend {
            self.anchor = self.caret;
        }
    }

    /// Whether `c` could go in at character position `at` of `value`, ignoring the length limit.
    fn accepts_in(&self, value: &str, c: char, at: usize) -> bool {
        match self.kind {
            InputKind::Text => !c.is_control(),
            InputKind::Multiline => !c.is_control() || c == '\n',
            InputKind::Number => c.is_ascii_digit() || (c == '.' && !value.contains('.')) || (c == '-' && at == 0 && !value.starts_with('-')),
        }
    }

    fn full(&self) -> bool {
        self.max_chars.is_some_and(|max| self.len() >= max)
    }

    /// Removes the characters from `start` to `end`, and puts the caret there.
    fn delete_range(&mut self, start: usize, end: usize) {
        let (a, b) = (self.byte_at(start), self.byte_at(end));
        self.value.replace_range(a..b, "");
        self.caret = start;
        self.anchor = start;
    }

    /// Removes what is selected. Returns whether anything was.
    fn delete_selection(&mut self) -> bool {
        match self.selection() {
            Some((a, b)) => {
                self.delete_range(a, b);
                true
            }
            None => false,
        }
    }

    /// Types `c` at the caret, replacing what is selected, if it is allowed there. Returns whether it went in.
    /// A character that isn't allowed leaves the selection alone.
    fn insert(&mut self, c: char) -> bool {
        // Try it on a copy with the selection gone, since that decides whether it is allowed (the selection may
        // hold the `.` or `-` that would otherwise block it, or make room under the length limit).
        let mut trial = self.clone();
        trial.delete_selection();
        if trial.full() || !trial.accepts_in(&trial.value, c, trial.caret) {
            return false;
        }
        let at = trial.byte_at(trial.caret);
        trial.value.insert(at, c);
        trial.caret += 1;
        trial.anchor = trial.caret;
        self.value = trial.value;
        self.caret = trial.caret;
        self.anchor = trial.anchor;
        true
    }

    /// Pastes text as if it were typed, except that a line break becomes a space in an input of one line, and what
    /// the input wouldn't have accepted is left out. Returns whether anything went in.
    pub fn paste(&mut self, text: &str) -> bool {
        let mut any = false;
        for c in text.chars() {
            match c {
                '\r' => {}
                '\n' if self.kind != InputKind::Multiline => any |= self.insert(' '),
                c => any |= self.insert(c),
            }
        }
        any
    }

    /// Replaces the value, as the program (not the user) would. What the input wouldn't have accepted typed
    /// is dropped, the caret goes to the end, and nothing is selected. Returns whether the value ended up different.
    pub fn set_value(&mut self, text: &str) -> bool {
        let mut next = InputState { value: String::new(), caret: 0, anchor: 0, ..self.clone() };
        for c in text.chars() {
            next.insert(c);
        }
        let changed = next.value != self.value;
        self.value = next.value;
        self.caret = self.len();
        self.anchor = self.caret;
        changed
    }

    /// The caret as `(line, column)`, both counted from 0. A single-line input is always on line 0.
    pub fn line_and_column(&self) -> (usize, usize) {
        let before = &self.value[..self.byte_at(self.caret)];
        let line = before.matches('\n').count();
        let column = before.rsplit('\n').next().map_or(0, |s| s.chars().count());
        (line, column)
    }

    /// The character index of the start of `line`, and how many characters it has.
    pub fn line_bounds(&self, line: usize) -> (usize, usize) {
        let mut start = 0;
        for (i, text) in self.value.split('\n').enumerate() {
            let len = text.chars().count();
            if i == line {
                return (start, len);
            }
            start += len + 1;
        }
        (self.len(), 0)
    }

    /// Where the start of the word the caret is in (or the one before it) is: skip what isn't a word going left,
    /// then the word.
    fn word_start(&self, from: usize) -> usize {
        let chars: Vec<char> = self.value.chars().collect();
        let mut i = from.min(chars.len());
        while i > 0 && !is_word(chars[i - 1]) {
            i -= 1;
        }
        while i > 0 && is_word(chars[i - 1]) {
            i -= 1;
        }
        i
    }

    /// Where the end of the next word is: skip what isn't a word going right, then the word.
    fn word_end(&self, from: usize) -> usize {
        let chars: Vec<char> = self.value.chars().collect();
        let mut i = from.min(chars.len());
        while i < chars.len() && !is_word(chars[i]) {
            i += 1;
        }
        while i < chars.len() && is_word(chars[i]) {
            i += 1;
        }
        i
    }

    /// The place in the text that is nearest to a point, given as how far it is from the top-left of the content box.
    /// This uses where the text was drawn the last time (see [`Geometry`]).
    pub fn index_at(&self, x: f32, y: f32) -> usize {
        let g = &self.geometry;
        if g.columns.is_empty() || g.line_height <= 0 {
            return self.caret;
        }
        let row = g.first_row as i32 + (y / g.line_height as f32).floor().max(0.0) as i32;
        let row = (row.max(0) as usize).min(g.columns.len() - 1);
        let x = x + g.scroll_x as f32;
        let column = g.columns[row]
            .iter()
            .enumerate()
            .min_by(|a, b| (a.1 - x).abs().total_cmp(&(b.1 - x).abs()))
            .map_or(0, |(i, _)| i);
        g.row_starts.get(row).copied().unwrap_or_else(|| self.line_bounds(row).0) + column
    }

    /// Puts the caret at a point (as for `index_at`) because the mouse went down there. With `extend` (shift held),
    /// the selection grows to it instead of starting there.
    pub fn click_at(&mut self, x: f32, y: f32, extend: bool) {
        let target = self.index_at(x, y);
        self.move_to(target, extend);
    }

    /// Extends the selection to a point, because the mouse is being dragged there.
    pub fn drag_to(&mut self, x: f32, y: f32) {
        self.caret = self.index_at(x, y);
    }

    /// Applies one frame of keyboard input: the keys first, then what was typed. Returns whether Escape was pressed,
    /// which the caller decides what to do with (an input gives up focus). `changed` and `submitted` say what happened.
    ///
    /// - Shift with a movement key extends the selection, and Ctrl moves by word (or to the start or end of the text).
    /// - Ctrl+A selects everything, Ctrl+C and Ctrl+X copy the selection, and Ctrl+V pastes.
    /// - Typing, Backspace, Delete, Enter and pasting replace the selection.
    pub fn edit(&mut self, keyboard: &Keyboard, clipboard: &Clipboard) -> bool {
        self.changed = false;
        self.submitted = false;
        let mut escape = false;
        let (shift, ctrl) = (keyboard.shift(), keyboard.control());

        for event in keyboard.events.iter().filter(|e| e.action != KeyAction::Release) {
            let (line, column) = self.line_and_column();
            let (line_start, line_len) = self.line_bounds(line);
            match event.key {
                Key::Backspace => {
                    if self.delete_selection() {
                        self.changed = true;
                    } else if self.caret > 0 {
                        let from = if ctrl { self.word_start(self.caret) } else { self.caret - 1 };
                        self.delete_range(from, self.caret);
                        self.changed = true;
                    }
                }
                Key::Delete => {
                    if self.delete_selection() {
                        self.changed = true;
                    } else if self.caret < self.len() {
                        let to = if ctrl { self.word_end(self.caret) } else { self.caret + 1 };
                        self.delete_range(self.caret, to);
                        self.changed = true;
                    }
                }
                Key::Left => {
                    let target = match self.selection() {
                        _ if ctrl => self.word_start(self.caret),
                        Some((start, _)) if !shift => start, // Collapses to where the selection began.
                        _ => self.caret.saturating_sub(1),
                    };
                    self.move_to(target, shift);
                }
                Key::Right => {
                    let target = match self.selection() {
                        _ if ctrl => self.word_end(self.caret),
                        Some((_, end)) if !shift => end,
                        _ => self.caret + 1,
                    };
                    self.move_to(target, shift);
                }
                Key::Home => self.move_to(if ctrl { 0 } else { line_start }, shift),
                Key::End => self.move_to(if ctrl { self.len() } else { line_start + line_len }, shift),
                Key::Up if self.kind == InputKind::Multiline && line > 0 => {
                    let (start, len) = self.line_bounds(line - 1);
                    self.move_to(start + column.min(len), shift);
                }
                Key::Down if self.kind == InputKind::Multiline && line + 1 < self.value.split('\n').count() => {
                    let (start, len) = self.line_bounds(line + 1);
                    self.move_to(start + column.min(len), shift);
                }
                Key::Enter if self.kind == InputKind::Multiline => self.changed |= self.insert('\n'),
                Key::Enter => self.submitted = true,
                Key::Escape => escape = true,
                Key::A if ctrl => self.select_all(),
                Key::C if ctrl => {
                    if self.selection().is_some() {
                        clipboard.set(&self.selected_text());
                    }
                }
                Key::X if ctrl => {
                    if self.selection().is_some() {
                        clipboard.set(&self.selected_text());
                        self.changed |= self.delete_selection();
                    }
                }
                Key::V if ctrl => self.changed |= self.paste(&clipboard.get()),
                _ => {}
            }
        }

        for c in keyboard.text.chars() {
            self.changed |= self.insert(c);
        }
        escape
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::{KeyEvent, MOD_CONTROL, MOD_SHIFT};

    fn keyboard(keys: &[Key], modifiers: u8) -> Keyboard {
        Keyboard {
            events: keys.iter().map(|&key| KeyEvent { key, action: KeyAction::Press }).collect(),
            modifiers,
            ..Keyboard::default()
        }
    }

    fn keys(keys: &[Key]) -> Keyboard {
        keyboard(keys, 0)
    }

    fn shift(keys: &[Key]) -> Keyboard {
        keyboard(keys, MOD_SHIFT)
    }

    fn ctrl(keys: &[Key]) -> Keyboard {
        keyboard(keys, MOD_CONTROL)
    }

    fn typed(text: &str) -> Keyboard {
        Keyboard { text: text.to_string(), ..Keyboard::default() }
    }

    fn with(kind: InputKind, value: &str) -> InputState {
        let mut state = InputState::new(kind);
        state.set_value(value);
        state
    }

    /// Applies a frame of input with a fresh clipboard.
    fn press(input: &mut InputState, kb: &Keyboard) -> bool {
        input.edit(kb, &Clipboard::default())
    }

    #[test]
    fn typing_inserts_at_the_caret() {
        let mut input = with(InputKind::Text, "ac");
        input.caret = 1;
        input.anchor = 1;
        press(&mut input, &typed("b"));
        assert_eq!((input.value.as_str(), input.caret, input.changed), ("abc", 2, true));
    }

    #[test]
    fn backspace_and_delete_remove_around_the_caret() {
        let mut input = with(InputKind::Text, "abcd");
        input.select(2, 2);
        press(&mut input, &keys(&[Key::Backspace]));
        assert_eq!((input.value.as_str(), input.caret), ("acd", 1));
        press(&mut input, &keys(&[Key::Delete]));
        assert_eq!((input.value.as_str(), input.caret), ("ad", 1));

        let mut empty = InputState::new(InputKind::Text);
        press(&mut empty, &keys(&[Key::Backspace, Key::Delete]));
        assert!(!empty.changed, "nothing to remove at either end");
    }

    #[test]
    fn the_caret_moves_and_stops_at_the_ends() {
        let mut input = with(InputKind::Text, "abc");
        press(&mut input, &keys(&[Key::Right, Key::Right]));
        assert_eq!(input.caret, 3, "already at the end");
        press(&mut input, &keys(&[Key::Left, Key::Left, Key::Left, Key::Left]));
        assert_eq!(input.caret, 0);
        press(&mut input, &keys(&[Key::End]));
        assert_eq!(input.caret, 3);
        press(&mut input, &keys(&[Key::Home]));
        assert_eq!(input.caret, 0);
    }

    #[test]
    fn multibyte_characters_count_as_one() {
        let mut input = with(InputKind::Text, "héé");
        assert_eq!((input.len(), input.caret), (3, 3));
        press(&mut input, &keys(&[Key::Backspace]));
        assert_eq!(input.value, "hé");
        press(&mut input, &keys(&[Key::Left]));
        press(&mut input, &typed("ü"));
        assert_eq!(input.value, "hüé");
    }

    #[test]
    fn enter_submits_a_single_line_and_breaks_a_multiline() {
        let mut single = with(InputKind::Text, "hi");
        press(&mut single, &keys(&[Key::Enter]));
        assert!(single.submitted && !single.changed);
        press(&mut single, &typed("x"));
        assert!(!single.submitted, "it only lasts the frame it happened in");

        let mut multi = with(InputKind::Multiline, "ab");
        multi.select(1, 1);
        press(&mut multi, &keys(&[Key::Enter]));
        assert_eq!((multi.value.as_str(), multi.caret, multi.submitted), ("a\nb", 2, false));
    }

    #[test]
    fn up_and_down_keep_the_column_in_a_multiline() {
        let mut input = with(InputKind::Multiline, "abcd\nxy\nmnop");
        input.select(3, 3);
        press(&mut input, &keys(&[Key::Down]));
        assert_eq!(input.line_and_column(), (1, 2), "line 1 is shorter, so the caret goes to its end");
        press(&mut input, &keys(&[Key::Down]));
        assert_eq!(input.line_and_column(), (2, 2));
        press(&mut input, &keys(&[Key::Down]));
        assert_eq!(input.line_and_column(), (2, 2), "no line below");
        press(&mut input, &keys(&[Key::Up, Key::Up, Key::Up]));
        assert_eq!(input.line_and_column(), (0, 2));
    }

    #[test]
    fn home_and_end_go_to_the_ends_of_the_current_line() {
        let mut input = with(InputKind::Multiline, "abc\ndefg");
        input.select(5, 5);
        press(&mut input, &keys(&[Key::Home]));
        assert_eq!(input.caret, 4);
        press(&mut input, &keys(&[Key::End]));
        assert_eq!(input.caret, 8);
        press(&mut input, &keys(&[Key::Up, Key::End]));
        assert_eq!(input.caret, 3);
    }

    #[test]
    fn the_length_limit_stops_typing() {
        let mut input = InputState::new(InputKind::Text);
        input.max_chars = Some(3);
        press(&mut input, &typed("abcdef"));
        assert_eq!(input.value, "abc");
        assert!(!input.set_value("abcdef") || input.value == "abc", "setting it is limited the same way");
        assert_eq!(input.value, "abc");
    }

    #[test]
    fn control_characters_never_go_in() {
        let mut input = InputState::new(InputKind::Text);
        press(&mut input, &typed("a\tb\u{7}c\nd"));
        assert_eq!(input.value, "abcd");
    }

    #[test]
    fn a_number_input_only_takes_what_makes_a_number() {
        let mut input = InputState::new(InputKind::Number);
        press(&mut input, &typed("-12a.5.6-7"));
        assert_eq!(input.value, "-12.567", "one minus at the start, one point, only digits otherwise");

        let mut other = InputState::new(InputKind::Number);
        press(&mut other, &typed("5-"));
        assert_eq!(other.value, "5", "a minus anywhere but the start is refused");
        assert!(!other.set_value("12abc34") || other.value == "1234");
    }

    #[test]
    fn setting_the_value_reports_whether_it_changed() {
        let mut input = InputState::new(InputKind::Text);
        assert!(input.set_value("hello"));
        assert!(!input.set_value("hello"), "same text");
        assert_eq!((input.caret, input.anchor), (5, 5));
        assert!(input.set_value(""));
        assert_eq!(input.caret, 0);
    }

    #[test]
    fn escape_is_reported_and_releases_are_ignored() {
        let mut input = with(InputKind::Text, "ab");
        assert!(press(&mut input, &keys(&[Key::Escape])));
        let release = Keyboard { events: vec![KeyEvent { key: Key::Backspace, action: KeyAction::Release }], ..Keyboard::default() };
        press(&mut input, &release);
        assert_eq!(input.value, "ab");
    }

    #[test]
    fn keys_apply_before_typed_text() {
        let mut input = with(InputKind::Text, "ab");
        let both = Keyboard { events: vec![KeyEvent { key: Key::Backspace, action: KeyAction::Press }], text: "X".into(), modifiers: 0 };
        press(&mut input, &both);
        assert_eq!(input.value, "aX");
    }

    // ---- Selection

    #[test]
    fn shift_with_a_movement_key_grows_the_selection_from_where_it_started() {
        let mut input = with(InputKind::Text, "hello world");
        input.select(2, 2);
        press(&mut input, &shift(&[Key::Right, Key::Right, Key::Right]));
        assert_eq!((input.selection(), input.selected_text().as_str()), (Some((2, 5)), "llo"));
        press(&mut input, &shift(&[Key::Left]));
        assert_eq!(input.selection(), Some((2, 4)), "shrinks back");
        press(&mut input, &shift(&[Key::Left, Key::Left, Key::Left, Key::Left]));
        assert_eq!(input.selection(), Some((0, 2)), "and past where it started, to the other side");
        press(&mut input, &shift(&[Key::End]));
        assert_eq!(input.selection(), Some((2, 11)));
        press(&mut input, &shift(&[Key::Home]));
        assert_eq!(input.selection(), Some((0, 2)));
    }

    #[test]
    fn moving_without_shift_drops_the_selection_collapsing_to_the_side_you_moved_toward() {
        let mut input = with(InputKind::Text, "hello world");
        input.select(2, 7);
        press(&mut input, &keys(&[Key::Left]));
        assert_eq!((input.selection(), input.caret), (None, 2), "left goes to where the selection began");
        input.select(2, 7);
        press(&mut input, &keys(&[Key::Right]));
        assert_eq!((input.selection(), input.caret), (None, 7), "right to where it ended");
        input.select(7, 2); // Made right to left.
        press(&mut input, &keys(&[Key::Right]));
        assert_eq!(input.caret, 7, "whichever way it was made");
    }

    #[test]
    fn typing_replaces_the_selection() {
        let mut input = with(InputKind::Text, "hello world");
        input.select(0, 5);
        press(&mut input, &typed("J"));
        assert_eq!((input.value.as_str(), input.caret, input.selection()), ("J world", 1, None));
    }

    #[test]
    fn backspace_and_delete_remove_the_selection_not_a_character_beside_it() {
        for key in [Key::Backspace, Key::Delete] {
            let mut input = with(InputKind::Text, "hello world");
            input.select(5, 11);
            press(&mut input, &keys(&[key]));
            assert_eq!((input.value.as_str(), input.caret, input.changed), ("hello", 5, true));
        }
    }

    #[test]
    fn a_character_that_is_refused_leaves_the_selection_alone() {
        let mut input = with(InputKind::Number, "123");
        input.select(0, 3);
        press(&mut input, &typed("x"));
        assert_eq!((input.value.as_str(), input.selection()), ("123", Some((0, 3))));
        press(&mut input, &typed("9"));
        assert_eq!(input.value, "9", "but one that is accepted replaces it");
    }

    #[test]
    fn a_selection_can_make_room_under_the_length_limit_and_for_a_blocked_point() {
        let mut limited = with(InputKind::Text, "abc");
        limited.max_chars = Some(3);
        limited.select(0, 3);
        press(&mut limited, &typed("xyz"));
        assert_eq!(limited.value, "xyz", "the three selected make room for three new ones");

        let mut number = with(InputKind::Number, "1.5");
        number.select(0, 3);
        press(&mut number, &typed(".2"));
        assert_eq!(number.value, ".2", "the point in the selection no longer blocks a new one");
    }

    #[test]
    fn select_all_with_ctrl_a() {
        let mut input = with(InputKind::Text, "hello");
        input.select(2, 2);
        press(&mut input, &ctrl(&[Key::A]));
        assert_eq!(input.selection(), Some((0, 5)));
        assert!(!input.changed);
    }

    #[test]
    fn selecting_by_program_clamps_to_the_text() {
        let mut input = with(InputKind::Text, "abc");
        input.select(1, 99);
        assert_eq!(input.selection(), Some((1, 3)));
        input.select(50, 60);
        assert_eq!(input.selection(), None, "both ends at the end: nothing selected");
    }

    // ---- Words

    #[test]
    fn ctrl_moves_by_word() {
        let mut input = with(InputKind::Text, "hello, big   world");
        input.select(0, 0);
        press(&mut input, &ctrl(&[Key::Right]));
        assert_eq!(input.caret, 5, "to the end of \"hello\"");
        press(&mut input, &ctrl(&[Key::Right]));
        assert_eq!(input.caret, 10, "past the comma and space, to the end of \"big\"");
        press(&mut input, &ctrl(&[Key::Right, Key::Right]));
        assert_eq!(input.caret, 18, "and then the end of the text");
        press(&mut input, &ctrl(&[Key::Left]));
        assert_eq!(input.caret, 13, "back to the start of \"world\"");
        press(&mut input, &ctrl(&[Key::Left]));
        assert_eq!(input.caret, 7, "of \"big\"");
    }

    #[test]
    fn ctrl_with_shift_selects_by_word_and_ctrl_home_and_end_go_to_the_ends_of_the_text() {
        let mut input = with(InputKind::Multiline, "one two\nthree");
        input.select(0, 0);
        press(&mut input, &keyboard(&[Key::Right], MOD_CONTROL | MOD_SHIFT));
        assert_eq!(input.selected_text(), "one");
        press(&mut input, &ctrl(&[Key::End]));
        assert_eq!(input.caret, 13);
        press(&mut input, &ctrl(&[Key::Home]));
        assert_eq!(input.caret, 0);
    }

    #[test]
    fn ctrl_backspace_and_delete_remove_a_word() {
        let mut input = with(InputKind::Text, "one two three");
        input.select(7, 7);
        press(&mut input, &ctrl(&[Key::Backspace]));
        assert_eq!((input.value.as_str(), input.caret), ("one  three", 4), "back to the start of \"two\"");
        press(&mut input, &ctrl(&[Key::Delete]));
        assert_eq!(input.value, "one ", "forward to the end of \"three\"");
    }

    // ---- Clipboard

    #[test]
    fn copy_cut_and_paste_go_through_the_clipboard() {
        let clipboard = Clipboard::default();
        let mut input = with(InputKind::Text, "hello world");
        input.select(0, 5);

        input.edit(&ctrl(&[Key::C]), &clipboard);
        assert_eq!((clipboard.get().as_str(), input.value.as_str(), input.changed), ("hello", "hello world", false), "copy leaves the text alone");

        input.select(6, 11);
        input.edit(&ctrl(&[Key::X]), &clipboard);
        assert_eq!((clipboard.get().as_str(), input.value.as_str(), input.changed), ("world", "hello ", true));

        input.edit(&ctrl(&[Key::V]), &clipboard);
        assert_eq!((input.value.as_str(), input.caret, input.changed), ("hello world", 11, true));

        input.select(0, 5);
        input.edit(&ctrl(&[Key::V]), &clipboard);
        assert_eq!(input.value, "world world", "paste replaces the selection");
    }

    #[test]
    fn copying_nothing_keeps_what_the_clipboard_had() {
        let clipboard = Clipboard::default();
        clipboard.set("kept");
        let mut input = with(InputKind::Text, "abc");
        input.select(1, 1);
        input.edit(&ctrl(&[Key::C]), &clipboard);
        input.edit(&ctrl(&[Key::X]), &clipboard);
        assert_eq!((clipboard.get().as_str(), input.value.as_str()), ("kept", "abc"));
    }

    #[test]
    fn pasting_is_filtered_like_typing() {
        let clipboard = Clipboard::default();

        clipboard.set("line one\r\nline two");
        let mut single = InputState::new(InputKind::Text);
        single.edit(&ctrl(&[Key::V]), &clipboard);
        assert_eq!(single.value, "line one line two", "a line break becomes a space in one line");

        let mut multi = InputState::new(InputKind::Multiline);
        multi.edit(&ctrl(&[Key::V]), &clipboard);
        assert_eq!(multi.value, "line one\nline two", "and stays one in several (the \\r is dropped)");

        clipboard.set("Ph: 12-34.5.6");
        let mut number = InputState::new(InputKind::Number);
        number.edit(&ctrl(&[Key::V]), &clipboard);
        assert_eq!(number.value, "1234.56");

        let mut limited = InputState::new(InputKind::Text);
        limited.max_chars = Some(4);
        limited.edit(&ctrl(&[Key::V]), &clipboard);
        assert_eq!(limited.value, "Ph: ");
    }

    #[test]
    fn pasting_nothing_changes_nothing() {
        let mut input = with(InputKind::Text, "abc");
        input.select(0, 3);
        input.edit(&ctrl(&[Key::V]), &Clipboard::default()); // An empty clipboard.
        assert_eq!((input.value.as_str(), input.changed, input.selection()), ("abc", false, Some((0, 3))));
    }

    // ---- The mouse

    /// An input whose one line "abcd" has characters 10px wide, drawn with its left edge at 0.
    fn laid_out() -> InputState {
        let mut input = with(InputKind::Text, "abcd");
        input.geometry = Geometry { line_height: 20, columns: vec![vec![0.0, 10.0, 20.0, 30.0, 40.0]], ..Geometry::default() };
        input
    }

    #[test]
    fn a_click_puts_the_caret_at_the_nearest_gap() {
        let mut input = laid_out();
        for (x, expected) in [(-5.0, 0), (4.0, 0), (6.0, 1), (24.0, 2), (26.0, 3), (39.0, 4), (500.0, 4)] {
            input.click_at(x, 5.0, false);
            assert_eq!((input.caret, input.anchor), (expected, expected), "at x = {x}");
        }
    }

    #[test]
    fn dragging_selects_from_where_the_mouse_went_down() {
        let mut input = laid_out();
        input.click_at(14.0, 5.0, false); // Between b and c... nearest gap 10 (1).
        assert_eq!(input.caret, 1);
        input.drag_to(33.0, 5.0);
        assert_eq!((input.selection(), input.selected_text().as_str()), (Some((1, 3)), "bc"));
        input.drag_to(2.0, 5.0);
        assert_eq!(input.selection(), Some((0, 1)), "dragging back past the start swaps sides");
    }

    #[test]
    fn shift_click_extends_the_selection() {
        let mut input = laid_out();
        input.click_at(6.0, 5.0, false);
        input.click_at(36.0, 5.0, true);
        assert_eq!(input.selection(), Some((1, 4)));
    }

    #[test]
    fn a_scrolled_input_maps_the_mouse_through_its_scroll() {
        let mut input = laid_out();
        input.geometry.scroll_x = 20; // The first two characters are scrolled out of view.
        input.click_at(2.0, 5.0, false);
        assert_eq!(input.caret, 2, "x = 2 on screen is x = 22 in the text");
    }

    #[test]
    fn a_click_picks_the_line_and_column_in_a_multiline() {
        let mut input = with(InputKind::Multiline, "ab\ncdef\ng");
        input.geometry = Geometry {
            line_height: 20,
            columns: vec![vec![0.0, 10.0, 20.0], vec![0.0, 10.0, 20.0, 30.0, 40.0], vec![0.0, 10.0]],
            ..Geometry::default()
        };
        input.click_at(25.0, 25.0, false); // Second row, between d and e... the gap at 20 is 2 of "cdef".
        assert_eq!(input.caret, 3 + 2);
        input.click_at(5.0, 45.0, false);
        assert_eq!(input.caret, 8 + 0, "the third row, its start");
        input.click_at(99.0, 500.0, false);
        assert_eq!(input.caret, 9, "past the last row is the last row's end");
        input.click_at(99.0, -50.0, false);
        assert_eq!(input.caret, 2, "above the first row is the first row's end");
    }

    #[test]
    fn with_no_layout_yet_a_click_changes_nothing() {
        let mut input = with(InputKind::Text, "abc");
        input.click_at(5.0, 5.0, false);
        assert_eq!(input.caret, 3);
    }
}
