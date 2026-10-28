use std::borrow::Cow;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use crate::anim::ColorAnim;
use crate::dirty::invalidate;
use crate::draw_cmd::Rect;
use crate::input::{Interaction, UpdateCtx, BUTTON_LEFT};
use crate::registry;
use crate::style::{Color, FontStyle, Style, TextStyle};
use crate::text_input::{InputKind, InputState};

static NEXT_UID: AtomicU32 = AtomicU32::new(1);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Orientation {
    Vertical,
    Horizontal,
}

/// RGBA8 pixels with straight (not premultiplied) alpha, tightly packed, row 0 at the top.
#[derive(Debug, PartialEq)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
}

impl Image {
    /// Returns `None` if `data` isn't exactly `width * height * 4` bytes or the image is empty.
    pub fn from_rgba(width: u32, height: u32, data: Vec<u8>) -> Option<Image> {
        let expected = (width as usize).checked_mul(height as usize)?.checked_mul(4)?;
        (width > 0 && height > 0 && data.len() == expected).then_some(Image { width, height, data })
    }

    /// Decodes a PNG or JPEG file.
    pub fn from_file(path: &str) -> Result<Image, String> {
        let decoded = image::open(path).map_err(|e| format!("{path}: {e}"))?.into_rgba8();
        let (width, height) = decoded.dimensions();
        Image::from_rgba(width, height, decoded.into_raw()).ok_or_else(|| format!("{path}: image is empty"))
    }
}

/// A run of text inside rich text. What a span doesn't set, it takes from the element's text style.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TextSpan {
    pub text: String,
    pub foreground: Option<Color>,
    pub font_weight: Option<u16>,
    pub font_style: Option<FontStyle>,
    /// A line under the text (and the spaces between underlined text), in the span's color.
    pub underline: bool,
    /// A line through the middle of the text, in the span's color.
    pub strikethrough: bool,
}

/// What an element is. Built-in elements from the README (Text, Image, Button, ...) become variants here.
#[derive(Clone, Debug, PartialEq)]
pub enum ElementKind {
    /// A container that places its children one after another.
    Stack(Orientation),
    /// Text drawn with the inherited text style (`foreground`, `font_size`, ...), wrapped to the space available.
    Text(String),
    /// Pixels drawn into the content box. Shared with draw commands, so a frame never copies them.
    Image(Arc<Image>),
    /// Text in runs that look different (a color, bold, italic, underline), wrapped together as one piece of text,
    /// such as a chat line. It uses the inherited font family and size, and each span can change the rest.
    Rich(Vec<TextSpan>),
    /// A field the user types into. It is drawn in the element's own box (so give it a background, border and
    /// padding like any other), with the value in the inherited text style, and a caret while it has focus.
    Input(InputState),
}

impl ElementKind {
    /// Whether clicking this kind of element gives it keyboard focus.
    pub fn takes_focus(&self) -> bool {
        matches!(self, ElementKind::Input(_))
    }
}

/// How far a scrolling element is scrolled, and how far it can be.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ScrollState {
    /// How far the content is moved left and up, in pixels.
    pub x: i32,
    pub y: i32,
    /// The most it can be scrolled on each axis, as of the last layout: how much of the content doesn't fit.
    pub max_x: i32,
    pub max_y: i32,
}

/// The colors an element is showing right now, which differ from its style's while a transition is under way.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShownColors {
    pub background: Color,
    pub border_color: Color,
    /// `None` while the element doesn't set a text color of its own, and takes its parent's.
    pub foreground: Option<Color>,
}

#[derive(Default)]
pub(crate) struct ColorAnims {
    background: ColorAnim,
    border_color: ColorAnim,
    foreground: ColorAnim,
}

/// A node in the element tree.
pub struct Element {
    pub id: String,
    pub classes: String,
    pub style: Style,
    pub kind: ElementKind,
    /// Each child is boxed so its address never changes when siblings are added.
    /// The C++ side holds raw pointers to elements, and a `Vec<Element>` would move them on growth.
    pub children: Vec<Box<Element>>,

    /// Identifies this element for the life of the program, so the canvas can remember which one a press began on.
    pub uid: u32,
    /// The border box as of the last layout, in canvas pixels with scrolling applied (so it matches the mouse).
    pub rect: Rect,
    /// Mouse state for this element as of the last frame. See [`UpdateCtx`].
    pub interaction: Interaction,
    /// Set by [`Element::request_focus`] and [`Element::request_blur`], and acted on by the next draw.
    pub focus_requested: bool,
    pub blur_requested: bool,

    /// The element this one is a child of, or null for a canvas's root and for an element that isn't in a tree yet.
    /// It stays valid because children are boxed, as long as an element isn't moved after it gets children, without
    /// then being added to a parent (which fixes them up). A canvas's root is boxed, so it is fine.
    pub(crate) parent: *mut Element,

    /// What the element's colors are showing as of the last [`Element::advance`], or `None` before the first. Layout
    /// uses these (through [`Element::background`] and friends) rather than the style, so transitions can be drawn.
    pub shown: Option<ShownColors>,
    pub(crate) anims: ColorAnims,

    /// For an element whose `overflow` is `Scroll`. Layout keeps it within what the content allows.
    pub scroll: ScrollState,
}

impl Element {
    /// The background that is showing: the style's, except while it is fading to a new one.
    pub fn background(&self) -> Color {
        self.shown.map_or(self.style.background, |s| s.background)
    }

    pub fn border_color(&self) -> Color {
        self.shown.map_or(self.style.border_color, |s| s.border_color)
    }

    /// The text properties this element passes down: its own where set, and `inherited` for the rest.
    pub fn text_style(&self, inherited: &TextStyle) -> TextStyle {
        let mut text = self.style.text_style(inherited);
        if let (Some(_), Some(shown)) = (self.style.foreground, self.shown.and_then(|s| s.foreground)) {
            text.foreground = shown;
        }
        text
    }

    /// Brings the colors that are showing up to date for the time `now` (seconds on the canvas's clock), starting a
    /// fade if the style's target changed. Returns whether anything showing changed, and whether a color is still
    /// on its way somewhere.
    pub fn advance(&mut self, now: f64) -> (bool, bool) {
        let transition = self.style.transition;
        let background = self.anims.background.update(self.style.background, now, &transition);
        let border_color = self.anims.border_color.update(self.style.border_color, now, &transition);
        let foreground = match self.style.foreground {
            Some(target) => Some(self.anims.foreground.update(target, now, &transition)),
            None => {
                self.anims.foreground.reset();
                None
            }
        };

        let shown = ShownColors { background, border_color, foreground };
        let changed = self.shown != Some(shown);
        self.shown = Some(shown);
        let moving = self.anims.background.is_moving(now, &transition)
            || self.anims.border_color.is_moving(now, &transition)
            || self.anims.foreground.is_moving(now, &transition);
        (changed, moving)
    }

    pub fn new(kind: ElementKind) -> Element {
        Element {
            id: String::new(),
            classes: String::new(),
            style: Style::default(),
            kind,
            children: Vec::new(),
            uid: NEXT_UID.fetch_add(1, Ordering::Relaxed),
            rect: Rect { x: 0, y: 0, w: 0, h: 0 },
            interaction: Interaction::default(),
            focus_requested: false,
            blur_requested: false,
            parent: std::ptr::null_mut(),
            shown: None,
            anims: ColorAnims::default(),
            scroll: ScrollState::default(),
        }
    }

    /// Called once per frame, after layout, for every element in the tree (parents first).
    /// This is where an element that responds to input reads it: `ctx` has the mouse and keyboard, and
    /// whether this element is hovered, pressed, clicked or focused. Elements that aren't interactive have nothing
    /// to do; a new interactive kind adds its behavior to the match.
    ///
    /// Anything that changes how the element looks has to call [`crate::dirty::invalidate`] (or change it with
    /// [`crate::dirty::set`]), or the canvas will keep showing what it last drew.
    pub fn update(&mut self, ctx: &UpdateCtx) {
        match &mut self.kind {
            ElementKind::Stack(_) | ElementKind::Text(_) | ElementKind::Rich(_) | ElementKind::Image(_) => {}
            ElementKind::Input(state) => {
                let keyboard = &ctx.input.keyboard;
                let mouse = &ctx.input.mouse;
                let typing = ctx.interaction.focused && !ctx.just_focused && (!keyboard.events.is_empty() || !keyboard.text.is_empty());
                // A press on the input puts the caret where it landed (with shift held, it extends the selection
                // instead), and holding the button and moving selects.
                let press_here = mouse.pressed & BUTTON_LEFT != 0 && ctx.interaction.hovered;
                let dragging = !press_here && ctx.interaction.pressed;
                if typing || press_here || dragging {
                    let before = state.clone();
                    state.changed = false;
                    state.submitted = false;
                    if press_here || dragging {
                        // The mouse, as a distance from the top-left of the text's box.
                        let x = mouse.x - (ctx.rect.x + state.geometry.content_dx) as f32;
                        let y = mouse.y - (ctx.rect.y + state.geometry.content_dy) as f32;
                        if press_here {
                            state.click_at(x, y, keyboard.shift());
                        } else {
                            state.drag_to(x, y);
                        }
                    }
                    if typing && state.edit(keyboard, ctx.clipboard) {
                        ctx.release_focus.set(true);
                    }
                    if *state != before {
                        invalidate();
                    }
                } else if state.changed || state.submitted {
                    // What happened last frame is over. (The flags don't change how it looks, so no invalidate.)
                    state.changed = false;
                    state.submitted = false;
                }
            }
        }
    }

    /// Scrolls to `(x, y)` pixels. Layout keeps it within what the content allows, so asking for too much (a
    /// huge `y` to go to the end) is fine. Setting where it already is changes nothing.
    pub fn set_scroll(&mut self, x: i32, y: i32) {
        let target = ScrollState { x: x.max(0), y: y.max(0), ..self.scroll };
        if target != self.scroll {
            self.scroll = target;
            invalidate();
        }
    }

    /// Moves the scroll by `(dx, dy)` pixels, within what the content allows. Returns whether it moved.
    pub fn scroll_by(&mut self, dx: i32, dy: i32) -> bool {
        let x = (self.scroll.x + dx).clamp(0, self.scroll.max_x);
        let y = (self.scroll.y + dy).clamp(0, self.scroll.max_y);
        let moved = (x, y) != (self.scroll.x, self.scroll.y);
        if moved {
            self.scroll.x = x;
            self.scroll.y = y;
            invalidate();
        }
        moved
    }

    /// Asks for this element to get keyboard focus at the next draw, as if it had been clicked.
    /// Only elements that take keyboard input can have it.
    pub fn request_focus(&mut self) {
        self.focus_requested = true;
    }

    /// Asks for this element to lose keyboard focus at the next draw, if it has it.
    pub fn request_blur(&mut self) {
        self.blur_requested = true;
    }

    /// The text of a Text element, the value of an Input, or the text of all of a Rich element's spans put together.
    /// `None` for anything else.
    pub fn text_value(&self) -> Option<Cow<'_, str>> {
        match &self.kind {
            ElementKind::Text(text) => Some(Cow::Borrowed(text)),
            ElementKind::Input(state) => Some(Cow::Borrowed(&state.value)),
            ElementKind::Rich(spans) => Some(Cow::Owned(spans.iter().map(|s| s.text.as_str()).collect())),
            _ => None,
        }
    }

    /// Replaces the text of a Text element, or the value of an Input (which moves its caret to the end and drops
    /// what it wouldn't have accepted typed). Returns false, changing nothing, for any other kind.
    /// Setting the text it already has changes nothing, so it costs nothing to do every frame.
    pub fn set_text(&mut self, text: impl Into<String>) -> bool {
        match &mut self.kind {
            ElementKind::Text(current) => {
                let text = text.into();
                if *current != text {
                    *current = text;
                    invalidate();
                }
                true
            }
            ElementKind::Input(state) => {
                if state.set_value(&text.into()) {
                    invalidate();
                }
                true
            }
            // Plain text in place of the spans, so a rich element can be set like any other.
            ElementKind::Rich(spans) => {
                let text = text.into();
                if spans.len() != 1 || spans[0] != (TextSpan { text: text.clone(), ..TextSpan::default() }) {
                    *spans = vec![TextSpan { text, ..TextSpan::default() }];
                    invalidate();
                }
                true
            }
            _ => false,
        }
    }

    /// Replaces the spans of a Rich element. Returns false, changing nothing, for any other kind. The same spans as
    /// before change nothing, so this is fine to call every frame.
    pub fn set_spans(&mut self, new_spans: Vec<TextSpan>) -> bool {
        match &mut self.kind {
            ElementKind::Rich(spans) => {
                if *spans != new_spans {
                    *spans = new_spans;
                    invalidate();
                }
                true
            }
            _ => false,
        }
    }

    pub fn rich(spans: Vec<TextSpan>) -> Element {
        Element::new(ElementKind::Rich(spans))
    }

    /// Replaces the pixels of an Image element, such as to show a new frame. Returns false, changing nothing, for
    /// any other kind. The same pixels as before change nothing. A different size lays out again like any other change.
    pub fn set_image(&mut self, image: Image) -> bool {
        match &mut self.kind {
            ElementKind::Image(current) => {
                if **current != image {
                    *current = Arc::new(image);
                    invalidate();
                }
                true
            }
            _ => false,
        }
    }

    pub fn input(kind: InputKind) -> Element {
        Element::new(ElementKind::Input(InputState::new(kind)))
    }

    pub fn stack(orientation: Orientation) -> Element {
        Element::new(ElementKind::Stack(orientation))
    }

    pub fn text(text: impl Into<String>) -> Element {
        Element::new(ElementKind::Text(text.into()))
    }

    pub fn image(image: Image) -> Element {
        Element::new(ElementKind::Image(Arc::new(image)))
    }

    /// Adds a child and returns a reference to it inside the tree.
    pub fn add_child(&mut self, child: Element) -> &mut Element {
        let end = self.children.len();
        self.insert_child(end, child)
    }

    /// Adds a child at `index` (or at the end, for an index past it), and returns a reference to it.
    pub fn insert_child(&mut self, index: usize, child: Element) -> &mut Element {
        invalidate();
        let mut child = Box::new(child);
        child.parent = self as *mut Element;
        // The child was built before it had its final address, so its own children point at where it used to be.
        let child_address: *mut Element = &mut *child;
        for grandchild in &mut child.children {
            grandchild.parent = child_address;
        }
        registry::register(&mut child);

        let index = index.min(self.children.len());
        self.children.insert(index, child);
        &mut self.children[index]
    }

    /// Takes the child at `index` out of the tree and returns it. Dropping it frees it and everything inside it.
    pub fn remove_child(&mut self, index: usize) -> Option<Box<Element>> {
        if index >= self.children.len() {
            return None;
        }
        invalidate();
        let mut child = self.children.remove(index);
        child.parent = std::ptr::null_mut();
        Some(child)
    }

    /// Removes every child, freeing them and everything inside them.
    pub fn clear_children(&mut self) {
        if !self.children.is_empty() {
            invalidate();
            self.children.clear();
        }
    }

    /// The first element at or below this one with the given `id`, going down in order.
    pub fn find(&self, id: &str) -> Option<&Element> {
        if self.id == id {
            return Some(self);
        }
        self.children.iter().find_map(|child| child.find(id))
    }
}

impl Drop for Element {
    fn drop(&mut self) {
        registry::unregister(self.uid);
    }
}

// An element's `parent` is a raw pointer only so the tree can be edited through an id (see `tree`). It is only ever
// followed by code that has the whole tree to itself, so an element can still go to another thread with its canvas.
unsafe impl Send for Element {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn image_loads_from_a_png_file() {
        let path = std::env::temp_dir().join(format!("pincel_test_{}.png", std::process::id()));
        let mut source = image::RgbaImage::new(3, 2);
        source.put_pixel(0, 0, image::Rgba([255, 0, 0, 255]));
        source.put_pixel(2, 1, image::Rgba([0, 0, 255, 128]));
        source.save(&path).unwrap();

        let loaded = Image::from_file(path.to_str().unwrap()).unwrap();
        std::fs::remove_file(&path).ok();

        assert_eq!((loaded.width, loaded.height), (3, 2));
        assert_eq!(&loaded.data[0..4], [255, 0, 0, 255]);
        assert_eq!(&loaded.data[(5 * 4)..(5 * 4 + 4)], [0, 0, 255, 128]);
    }

    #[test]
    fn loading_a_missing_file_reports_the_path() {
        let error = Image::from_file("definitely/not/here.png").unwrap_err();
        assert!(error.contains("definitely/not/here.png"), "{error}");
    }
}
