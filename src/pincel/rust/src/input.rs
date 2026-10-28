//! Input from the host application. Pincel doesn't read devices itself: each frame the host passes in what the
//! mouse and keyboard are doing, and elements that respond to input read it in [`crate::element::Element::update`].

use std::cell::Cell;

use crate::clipboard::Clipboard;
use crate::draw_cmd::Rect;

// Mouse buttons, as a bitmask.
pub const BUTTON_LEFT: u8 = 1;
pub const BUTTON_RIGHT: u8 = 2;
pub const BUTTON_MIDDLE: u8 = 4;

// Keyboard modifiers that are held, as a bitmask.
pub const MOD_SHIFT: u8 = 1;
pub const MOD_CONTROL: u8 = 2;
pub const MOD_ALT: u8 = 4;
pub const MOD_SUPER: u8 = 8;

/// A key, named for what it does rather than for any one platform's key codes.
/// The numbers are part of the C ABI (see `PincelKey` in ffi.h), so only ever append to this list.
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Key {
    Unknown = 0,
    Backspace = 1,
    Delete = 2,
    Enter = 3,
    Tab = 4,
    Escape = 5,
    Left = 6,
    Right = 7,
    Up = 8,
    Down = 9,
    Home = 10,
    End = 11,
    PageUp = 12,
    PageDown = 13,
    Space = 14,
    /// `A` is 32 and the rest follow in order up to `Z`, which is 57.
    A = 32,
    B,
    C,
    D,
    E,
    F,
    G,
    H,
    I,
    J,
    K,
    L,
    M,
    N,
    O,
    P,
    Q,
    R,
    S,
    T,
    U,
    V,
    W,
    X,
    Y,
    Z,
}

impl Key {
    /// Anything that isn't a known key is `Unknown`.
    pub fn from_u8(value: u8) -> Key {
        const LETTERS: [Key; 26] = [
            Key::A, Key::B, Key::C, Key::D, Key::E, Key::F, Key::G, Key::H, Key::I, Key::J, Key::K, Key::L, Key::M,
            Key::N, Key::O, Key::P, Key::Q, Key::R, Key::S, Key::T, Key::U, Key::V, Key::W, Key::X, Key::Y, Key::Z,
        ];
        match value {
            1 => Key::Backspace,
            2 => Key::Delete,
            3 => Key::Enter,
            4 => Key::Tab,
            5 => Key::Escape,
            6 => Key::Left,
            7 => Key::Right,
            8 => Key::Up,
            9 => Key::Down,
            10 => Key::Home,
            11 => Key::End,
            12 => Key::PageUp,
            13 => Key::PageDown,
            14 => Key::Space,
            32..=57 => LETTERS[usize::from(value - 32)],
            _ => Key::Unknown,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyAction {
    Press,
    /// The key is held and the operating system is repeating it.
    Repeat,
    Release,
}

impl KeyAction {
    pub fn from_u8(value: u8) -> Option<KeyAction> {
        match value {
            0 => Some(KeyAction::Press),
            1 => Some(KeyAction::Repeat),
            2 => Some(KeyAction::Release),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeyEvent {
    pub key: Key,
    pub action: KeyAction,
}

#[derive(Clone, Debug)]
pub struct Mouse {
    /// Position in canvas pixels, from the top-left. NaN when the cursor isn't over the canvas.
    pub x: f32,
    pub y: f32,
    /// Buttons that are down now, as a mask of `BUTTON_*`.
    pub buttons: u8,
    /// Buttons that went down since the previous frame.
    pub pressed: u8,
    /// Buttons that came up since the previous frame.
    pub released: u8,
    /// Wheel movement this frame, positive for up and right, in notches.
    pub scroll_x: f32,
    pub scroll_y: f32,
}

impl Default for Mouse {
    fn default() -> Mouse {
        Mouse { x: f32::NAN, y: f32::NAN, buttons: 0, pressed: 0, released: 0, scroll_x: 0.0, scroll_y: 0.0 }
    }
}

impl Mouse {
    /// Whether the cursor is over `rect`.
    pub fn over(&self, rect: Rect) -> bool {
        rect.contains(self.x, self.y)
    }

    pub fn down(&self, button: u8) -> bool {
        self.buttons & button != 0
    }
}

#[derive(Clone, Debug, Default)]
pub struct Keyboard {
    /// Key presses, repeats and releases this frame, in order.
    pub events: Vec<KeyEvent>,
    /// Text typed this frame. This is characters, after the keyboard layout and modifiers, unlike `events`.
    pub text: String,
    /// Modifiers that are held now, as a mask of `MOD_*`.
    pub modifiers: u8,
}

impl Keyboard {
    /// Whether `key` went down (or repeated) this frame.
    pub fn pressed(&self, key: Key) -> bool {
        self.events.iter().any(|e| e.key == key && e.action != KeyAction::Release)
    }

    pub fn shift(&self) -> bool {
        self.modifiers & MOD_SHIFT != 0
    }

    pub fn control(&self) -> bool {
        self.modifiers & MOD_CONTROL != 0
    }
}

/// Everything the host told Pincel about this frame.
#[derive(Clone, Debug, Default)]
pub struct Input {
    pub mouse: Mouse,
    pub keyboard: Keyboard,
}

impl Input {
    /// Replaces the mouse state. `pressed` and `released` are worked out from what was down last time,
    /// so this is meant to be called once per frame.
    pub fn set_mouse(&mut self, x: f32, y: f32, buttons: u8, scroll_x: f32, scroll_y: f32) {
        let previous = self.mouse.buttons;
        self.mouse = Mouse {
            x,
            y,
            buttons,
            pressed: buttons & !previous,
            released: previous & !buttons,
            scroll_x,
            scroll_y,
        };
    }

    pub fn set_keyboard(&mut self, events: Vec<KeyEvent>, text: String, modifiers: u8) {
        self.keyboard = Keyboard { events, text, modifiers };
    }
}

/// What the mouse is doing to one element. Hover and press bubble up like in CSS, so a container is
/// hovered, pressed and clicked whenever something inside it is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Interaction {
    /// The cursor is over the element.
    pub hovered: bool,
    /// The left button is down and the press began on this element (or inside it).
    pub pressed: bool,
    /// The left button was released this frame, over the element where the press began.
    pub clicked: bool,
    /// The same for every button: a mask of `BUTTON_*`. `pressed` is the left bit of the first, and `clicked` of the second.
    pub pressed_buttons: u8,
    pub clicked_buttons: u8,
    /// The element has keyboard focus. Only one element in a canvas has it at a time, and only elements that take
    /// keyboard input (like an Input) can. Clicking one focuses it, and clicking anywhere else removes the focus.
    /// Unlike the others, this doesn't bubble.
    pub focused: bool,
}

/// What an element can read when it is updated.
pub struct UpdateCtx<'a> {
    pub input: &'a Input,
    /// The element's border box, in the same coordinates as the mouse.
    pub rect: Rect,
    pub interaction: Interaction,
    /// The element was given focus by `Element::request_focus` during this very frame, rather than by a click.
    /// The key press that made the host ask (a "T" to open a chat box, say) is still in this frame's input, so an
    /// element that was just focused this way ignores the frame's typing instead of putting that key in its text.
    pub just_focused: bool,
    /// An element sets this to give up the focus it has, such as an Input on Escape.
    pub release_focus: Cell<bool>,
    /// What copy and paste use.
    pub clipboard: &'a Clipboard,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mouse_edges_come_from_the_previous_frame() {
        let mut input = Input::default();
        input.set_mouse(1.0, 1.0, BUTTON_LEFT, 0.0, 0.0);
        assert_eq!((input.mouse.pressed, input.mouse.released), (BUTTON_LEFT, 0));
        input.set_mouse(1.0, 1.0, BUTTON_LEFT, 0.0, 0.0);
        assert_eq!((input.mouse.pressed, input.mouse.released), (0, 0), "held, not pressed again");
        input.set_mouse(1.0, 1.0, 0, 0.0, 0.0);
        assert_eq!((input.mouse.pressed, input.mouse.released), (0, BUTTON_LEFT));
    }

    #[test]
    fn mouse_is_nowhere_until_told_otherwise() {
        let input = Input::default();
        assert!(!input.mouse.over(Rect { x: -1000, y: -1000, w: 5000, h: 5000 }));
    }

    #[test]
    fn keys_round_trip_through_their_numbers() {
        for key in [Key::Backspace, Key::Enter, Key::PageDown, Key::Space, Key::A, Key::M, Key::Z] {
            assert_eq!(Key::from_u8(key as u8), key);
        }
        assert_eq!(Key::A as u8, 32);
        assert_eq!(Key::Z as u8, 57);
        assert_eq!(Key::from_u8(0), Key::Unknown);
        assert_eq!(Key::from_u8(200), Key::Unknown);
    }

    #[test]
    fn key_press_helper_ignores_releases() {
        let mut keyboard = Keyboard::default();
        keyboard.events = vec![
            KeyEvent { key: Key::Enter, action: KeyAction::Release },
            KeyEvent { key: Key::Tab, action: KeyAction::Repeat },
        ];
        assert!(!keyboard.pressed(Key::Enter));
        assert!(keyboard.pressed(Key::Tab));
    }
}
