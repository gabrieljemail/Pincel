use crate::anim::Transition;

/// A CSS-like set of four values (top, right, bottom, left).
/// Resolution order, from least to most specific: `base`, then `x` (left and right) and `y` (top and bottom), then `sides`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Spacing {
    pub base: u16,
    pub x: Option<u16>,
    pub y: Option<u16>,
    pub sides: Option<[u16; 4]>,
}

impl Spacing {
    /// Resolves to `[top, right, bottom, left]`.
    pub fn resolve(&self) -> [u16; 4] {
        if let Some(sides) = self.sides {
            return sides;
        }
        let x = self.x.unwrap_or(self.base);
        let y = self.y.unwrap_or(self.base);
        [y, x, y, x]
    }

    /// For `size`, where x is the width and y is the height. 0 means "auto".
    pub fn width(&self) -> u16 {
        self.x.unwrap_or(self.base)
    }

    pub fn height(&self) -> u16 {
        self.y.unwrap_or(self.base)
    }
}

/// `repr(C)` because this is passed across the C ABI as-is. Alpha is straight (not premultiplied), 0.0 to 1.0.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: f32,
}

impl Color {
    pub const TRANSPARENT: Color = Color {
        r: 0,
        g: 0,
        b: 0,
        a: 0.0,
    };
    pub const BLACK: Color = Color {
        r: 0,
        g: 0,
        b: 0,
        a: 1.0,
    };
    pub const WHITE: Color = Color {
        r: 0xFF,
        g: 0xFF,
        b: 0xFF,
        a: 1.0,
    };
}

/// Which font family text is drawn in: an id for one of the built-in families ([`FontFamily::Sans`] and
/// [`FontFamily::Mono`]) or one registered with [`crate::font::register_family`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FontFamily(pub u16);

#[allow(non_upper_case_globals)]
impl FontFamily {
    /// A proportional sans-serif font (Lato).
    pub const Sans: FontFamily = FontFamily(0);
    /// A monospace font (JetBrains Mono).
    pub const Mono: FontFamily = FontFamily(1);
}

impl Default for FontFamily {
    fn default() -> FontFamily {
        FontFamily::Sans
    }
}

#[repr(u8)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum FontStyle {
    #[default]
    Normal = 0,
    Italic = 1,
}

/// How an element is placed.
#[repr(u8)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Position {
    /// In order with its siblings, in its parent's stack.
    #[default]
    Flow = 0,
    /// Taken out of the stack, so it takes no room and moves nothing, and placed by its `inset` against its
    /// parent's padding box (inside the border). It scrolls with the content, and draws over what came before it.
    Absolute = 1,
    /// Like `Absolute`, but placed against the whole canvas, and it stays put while the content scrolls.
    /// For sidebars, toolbars, overlays and anything else that should be pinned to the window.
    Fixed = 2,
}

/// Where an out-of-flow element goes in what it is placed against. All in pixels, from that box's edges.
/// - A side that is set puts the element's edge that far in from that side.
/// - Both sides of an axis set, with no size on that axis, stretches the element between them.
///   (With a size, the left or top one wins.)
/// - `center_x` and `center_y` center it on that axis, whatever the sides say.
/// - Nothing set on an axis puts it at that axis's start.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Insets {
    pub left: Option<i32>,
    pub top: Option<i32>,
    pub right: Option<i32>,
    pub bottom: Option<i32>,
    pub center_x: bool,
    pub center_y: bool,
}

/// The shape of a transition: how its speed changes from start to end. These are CSS's timing functions.
#[repr(u8)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Easing {
    Linear = 0,
    /// Quick to start, slow to settle. The usual default.
    #[default]
    Ease = 1,
    EaseIn = 2,
    EaseOut = 3,
    EaseInOut = 4,
}

/// Whether an element can be the target of the mouse: hovered, pressed and clicked, and counted as something under the
/// cursor. This is what lets a HUD that covers the whole screen leave the game below it clickable.
#[repr(u8)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum PointerEvents {
    /// The default. The element is a target wherever it is.
    #[default]
    Auto = 0,
    /// The mouse goes straight through the element, to whatever is under it. What is inside it can still be a
    /// target. It isn't hovered itself (though it counts as hovered while something inside it is).
    None = 1,
}

/// What happens to children that don't fit in an element's box.
#[repr(u8)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Overflow {
    /// They are drawn outside it, over whatever is there.
    #[default]
    Visible = 0,
    /// They are cut off at the edge of the box (inside its border).
    Hidden = 1,
    /// They are cut off, and the user can scroll them into view with the mouse wheel, or the program can with
    /// `set_scroll`. The element needs a size on the axis it scrolls on, or it just grows to hold them.
    Scroll = 2,
}

/// Where a stack puts its children in the room it has left over.
#[repr(u8)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Align {
    /// At the start (left, or top). The default.
    #[default]
    Start = 0,
    Center = 1,
    /// At the end (right, or bottom).
    End = 2,
}

impl Align {
    /// How far in to put something that leaves `free` pixels spare. Nothing when it doesn't fit.
    pub fn offset(self, free: i32) -> i32 {
        match self {
            Align::Start => 0,
            Align::Center => free.max(0) / 2,
            Align::End => free.max(0),
        }
    }
}

/// How an image is sampled when it is drawn at a size other than its own.
#[repr(u8)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ImageFilter {
    /// Bilinear when enlarged, averaged when shrunk. Right for photos and anything smooth.
    #[default]
    Smooth = 0,
    /// The nearest source pixel, so enlarged pixel art stays crisp.
    Nearest = 1,
}

/// The weight of regular text, and of the lightest text that is drawn bold. Weights are on the CSS scale, 100 to 900.
pub const WEIGHT_NORMAL: u16 = 400;
pub const WEIGHT_BOLD: u16 = 700;

/// A shadow behind text: a copy of it in `color`, moved by `(offset_x, offset_y)` pixels and drawn first. This is the
/// hard-edged kind (no blur), which is cheap. With a fully transparent color (the default) there is no shadow, and
/// nothing is spent on one. The shadow's opacity is multiplied by the text's, so fading text fades its shadow too.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextShadow {
    pub color: Color,
    pub offset_x: i32,
    pub offset_y: i32,
}

impl TextShadow {
    pub const NONE: TextShadow = TextShadow { color: Color::TRANSPARENT, offset_x: 0, offset_y: 0 };

    /// Whether there is a shadow to draw.
    pub fn is_visible(&self) -> bool {
        self.color.a > 0.0
    }
}

impl Default for TextShadow {
    fn default() -> TextShadow {
        TextShadow::NONE
    }
}

/// The text properties that elements inherit from their parent: a Text element draws with whatever it
/// (or the closest ancestor that sets them) provides. The canvas supplies the values at the top of the tree.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextStyle {
    pub foreground: Color,
    pub font_family: FontFamily,
    /// Font size in pixels.
    pub font_size: u16,
    /// 100 to 900. Only regular and bold faces are built in, so this is bold from 600 up and regular below.
    pub font_weight: u16,
    pub font_style: FontStyle,
    pub text_shadow: TextShadow,
}

impl Default for TextStyle {
    fn default() -> TextStyle {
        TextStyle {
            foreground: Color::BLACK,
            font_family: FontFamily::Sans,
            font_size: 16,
            font_weight: WEIGHT_NORMAL,
            font_style: FontStyle::Normal,
            text_shadow: TextShadow::NONE,
        }
    }
}

// The idea behind the Style struct is to allow applying a bunch of CSS-like properties to elements and have
// Pincel automatically apply usable ones appropriately. For example, the Text renderer reads Style::foreground.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Style {
    // The box model, outermost to innermost.
    pub margin: Spacing,
    pub padding: Spacing,
    pub border_size: Spacing,
    /// Width (x) and height (y) of the border box, so it includes padding and border. 0 is auto-calculated from the content.
    pub size: Spacing,

    // Colors.
    pub background: Color,
    pub border_color: Color,
    pub shadow_color: Color,

    // Text properties. `None` means "inherit from the parent", which is how they start.
    pub foreground: Option<Color>,
    pub font_family: Option<FontFamily>,
    pub font_size: Option<u16>,
    pub font_weight: Option<u16>,
    pub font_style: Option<FontStyle>,
    pub text_shadow: Option<TextShadow>,

    /// Radius in pixels of the border box's corners. Clamped to half the box's shorter side.
    pub corner_radius: u16,

    /// How an Image element is sampled when it isn't drawn at its own size. Not inherited.
    pub image_filter: ImageFilter,

    /// How the color properties (`background`, `border_color`, and `foreground` when set) change when they are set to a
    /// new value: a fade of `transition.duration` seconds, or at once with the default of 0. Not inherited.
    pub transition: Transition,

    /// What happens to children that don't fit. See [`Overflow`]. Placed (absolute) children don't scroll with the
    /// content, but are clipped by it.
    pub overflow: Overflow,
    /// For a scrolling element: when the content grows while it is scrolled all the way to the end (or hasn't been
    /// scrolled yet), it stays at the end and so follows the new content. For a chat log or a console.
    pub stick_to_end: bool,

    /// The color behind selected text in an Input. Not inherited.
    pub selection_color: Color,

    /// Whether the mouse can target this element. See [`PointerEvents`]. A canvas's root starts out as `None`, so empty
    /// canvas isn't something under the cursor.
    pub pointer_events: PointerEvents,

    /// Whether the element is shown at all. A hidden element is as if it weren't in the tree: it takes no room,
    /// draws nothing, gets no input, and neither do the elements inside it. It keeps its place and its state.
    pub visible: bool,

    /// Whether the element is in its parent's stack or placed on its own, and where. See [`Position`] and [`Insets`].
    pub position: Position,
    pub inset: Insets,

    /// For a stack: where its children go in the room left over on each axis, and the space between them.
    pub align_x: Align,
    pub align_y: Align,
    pub gap: u16,

    // Effects.
    pub shadow_size: Spacing,
    pub shadow_spread: Spacing,
}

impl Style {
    /// The text properties this element passes down: its own where set, and `inherited` for the rest.
    pub fn text_style(&self, inherited: &TextStyle) -> TextStyle {
        TextStyle {
            foreground: self.foreground.unwrap_or(inherited.foreground),
            font_family: self.font_family.unwrap_or(inherited.font_family),
            font_size: self.font_size.unwrap_or(inherited.font_size),
            font_weight: self.font_weight.unwrap_or(inherited.font_weight),
            font_style: self.font_style.unwrap_or(inherited.font_style),
            text_shadow: self.text_shadow.unwrap_or(inherited.text_shadow),
        }
    }
}

impl Default for Style {
    fn default() -> Style {
        Style {
            margin: Spacing::default(),
            padding: Spacing::default(),
            border_size: Spacing::default(),
            size: Spacing::default(),

            // Containers are see-through by default, so the canvas background shows.
            background: Color::TRANSPARENT,
            border_color: Color::BLACK,
            shadow_color: Color::BLACK,

            foreground: None,
            font_family: None,
            font_size: None,
            font_weight: None,
            font_style: None,
            text_shadow: None,

            corner_radius: 0,

            image_filter: ImageFilter::Smooth,

            transition: Transition::default(),

            overflow: Overflow::Visible,
            stick_to_end: false,

            selection_color: Color { r: 51, g: 144, b: 255, a: 0.45 },
            pointer_events: PointerEvents::Auto,

            visible: true,

            position: Position::Flow,
            inset: Insets::default(),

            align_x: Align::Start,
            align_y: Align::Start,
            gap: 0,

            shadow_size: Spacing::default(),
            shadow_spread: Spacing::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unset_text_properties_inherit_and_set_ones_override() {
        let parent = TextStyle {
            foreground: Color::WHITE,
            font_size: 20,
            font_weight: WEIGHT_BOLD,
            ..TextStyle::default()
        };
        let child = Style {
            font_size: Some(12),
            font_style: Some(FontStyle::Italic),
            ..Style::default()
        };
        let resolved = child.text_style(&parent);
        assert_eq!(resolved.foreground, Color::WHITE, "inherited");
        assert_eq!(resolved.font_weight, WEIGHT_BOLD, "inherited");
        assert_eq!(resolved.font_size, 12, "overridden");
        assert_eq!(resolved.font_style, FontStyle::Italic, "overridden");
        assert_eq!(resolved.font_family, FontFamily::Sans, "default from the parent");
    }
}
