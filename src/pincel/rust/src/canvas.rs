use std::cell::Cell;

use crate::clipboard::Clipboard;
use crate::dirty;
use crate::draw_cmd::{DrawCmd, Rect};
use crate::element::{Element, Orientation};
use crate::font::Fonts;
use crate::input::{Input, UpdateCtx, BUTTON_LEFT, BUTTON_MIDDLE, BUTTON_RIGHT};
use crate::layout::{layout, outer_size, View};
use crate::raster::Texture;
use crate::registry;
use crate::style::{Color, Overflow, PointerEvents, Position, TextStyle};

/// The mouse buttons that are tracked, in the order of `press_targets`.
const BUTTONS: [u8; 3] = [BUTTON_LEFT, BUTTON_RIGHT, BUTTON_MIDDLE];

/// How far one notch of the mouse wheel scrolls, in pixels.
const DEFAULT_SCROLL_STEP: f32 = 48.0;

/// A set of elements that should be drawn to the screen, and the texture they are drawn into.
///
/// Each frame, the host gives the canvas its input (the fields of [`Canvas::input`]) and then calls [`Canvas::draw`].
pub struct Canvas {
    pub background: Color,
    /// The text properties the root inherits, so this is where the default `foreground` comes from.
    pub text: TextStyle,
    /// The top of the element tree. Always a vertical stack.
    pub root: Box<Element>,
    pub width: u32,
    pub height: u32,

    /// How far the content is scrolled up, in pixels. Always between 0 and the content's overflow after a draw.
    pub scroll_y: f32,
    /// Pixels scrolled per notch of the mouse wheel.
    pub scroll_step: f32,
    /// The height of the whole tree as of the last draw. Anything over `height` can be scrolled to.
    pub content_height: i32,

    /// What inputs copy to and paste from. Give it the host's clipboard with [`Clipboard::set_hooks`]; without one it
    /// keeps text in the canvas, so copying and pasting between its inputs still works.
    pub clipboard: Clipboard,

    /// What the host told the canvas about this frame. The per-frame parts (wheel, button edges, key events,
    /// typed text) are cleared by `draw`, once every element has seen them.
    pub input: Input,
    /// The element each mouse button (left, right, middle) went down on, until it comes back up.
    press_targets: [Option<u32>; 3],
    /// Whether the cursor is over something that takes the mouse. See [`Canvas::wants_mouse`].
    wants_mouse: bool,
    /// Whether the last draw used the wheel's vertical and horizontal movement. See [`Canvas::wheel_consumed`].
    wheel_used: (bool, bool),
    /// The element that has keyboard focus. See [`Canvas::wants_keyboard`].
    focus: Option<u32>,

    time: Option<f64>,
    epoch: std::time::Instant,
    animating: bool,

    /// Goes up every time the texture is redrawn, so the host can tell when there are new pixels to upload.
    pub version: u64,
    /// How many times the tree has been laid out, and the texture redrawn. Frames where nothing changed add to neither.
    pub layouts: u64,
    pub paints: u64,

    // What the cached layout and texture were made from. Layout is expensive (it measures every piece of text),
    // so it is redone only when the tree changed (see `dirty`) or the width did, and moving the content for
    // scrolling is just a shift of the cached rects and commands. The texture is redrawn only when it would differ.
    laid_out_revision: Option<u64>,
    laid_out_width: u32,
    laid_out_height: u32,
    /// Whether the last layout placed something against the canvas (a `Fixed` element), so it depends on the height.
    layout_uses_viewport: bool,
    /// How far the cached rects are shifted for scrolling, in whole pixels.
    applied_scroll: i32,
    painted_size: (u32, u32),
    painted_scroll: i32,

    texture: Texture,
    fonts: Fonts,
    /// Draw commands from the last layout, where the content starts at y = 0 (before scrolling).
    cmds: Vec<DrawCmd>,
    /// Draw commands for `Fixed` elements, drawn over everything else and never scrolled.
    fixed_cmds: Vec<DrawCmd>,
}

impl Canvas {
    pub fn new() -> Canvas {
        // Boxed so the root has an address that stays put, which its children's parent links and the id registry rely on.
        let mut root = Box::new(Element::stack(Orientation::Vertical));
        // Empty canvas isn't under the cursor: only what is put on it is.
        root.style.pointer_events = PointerEvents::None;
        registry::register(&mut root);
        Canvas {
            background: Color::WHITE,
            text: TextStyle::default(),
            root,
            width: 0,
            height: 0,
            scroll_y: 0.0,
            scroll_step: DEFAULT_SCROLL_STEP,
            content_height: 0,
            clipboard: Clipboard::default(),
            input: Input::default(),
            press_targets: [None; 3],
            wants_mouse: false,
            wheel_used: (false, false),
            focus: None,
            time: None,
            epoch: std::time::Instant::now(),
            animating: false,
            version: 0,
            layouts: 0,
            paints: 0,
            laid_out_revision: None,
            laid_out_width: 0,
            laid_out_height: 0,
            layout_uses_viewport: false,
            applied_scroll: 0,
            painted_size: (0, 0),
            painted_scroll: 0,
            texture: Texture::default(),
            fonts: Fonts::new(),
            cmds: Vec::new(),
            fixed_cmds: Vec::new(),
        }
    }

    /// Whether the cursor is over something that takes the mouse (as of the last draw), or a press that began on
    /// something is still being held. While it is, a click or drag belongs to the canvas, so the host shouldn't also
    /// act on it. Over empty canvas, or an element with `pointer_events` of `None`, it is false and the host can have the
    /// mouse, which is how a HUD that covers the whole screen leaves the game clickable.
    pub fn wants_mouse(&self) -> bool {
        self.wants_mouse
    }

    /// Whether the last draw used the mouse wheel, as `(horizontal, vertical)`. It is used by a scrolling element under
    /// the cursor that could still move, or by the page when the cursor is in the canvas and the page could. A wheel the
    /// canvas didn't use (nothing could scroll, or everything is at its end, or the cursor is outside) is the host's,
    /// such as for a game's hotbar. This is separate from [`Canvas::wants_mouse`], which is about clicks.
    pub fn wheel_consumed(&self) -> (bool, bool) {
        self.wheel_used
    }

    /// Whether an element that takes keyboard input has focus. While it does, the host should treat the keyboard
    /// as Pincel's, and not also use the keys for its own shortcuts (movement keys in a game, say).
    pub fn wants_keyboard(&self) -> bool {
        self.focus.is_some()
    }

    /// Sets the canvas's clock, in seconds, which is what transitions run on. Call it every frame with a time that
    /// only goes forward, such as a game's own (so transitions pause with the game). Without it, the canvas uses a
    /// clock of its own that starts when it is created.
    pub fn set_time(&mut self, seconds: f64) {
        self.time = Some(seconds);
    }

    fn now(&self) -> f64 {
        self.time.unwrap_or_else(|| self.epoch.elapsed().as_secs_f64())
    }

    /// Whether a transition is still running as of the last draw. A host that only draws when something changed
    /// should keep drawing while this is true.
    pub fn is_animating(&self) -> bool {
        self.animating
    }

    /// The most the content can be scrolled: how far it overflows the canvas.
    pub fn max_scroll(&self) -> f32 {
        (self.content_height - self.height as i32).max(0) as f32
    }

    /// Brings the canvas up to date, and returns its texture, which is valid until the next call.
    ///
    /// Elements always get to see the input and the mouse state is always current, but the expensive parts happen
    /// only when needed: layout when the tree or the width changed, and rasterizing when the result would look
    /// different (it was laid out again, scrolled, or resized). A call where nothing changed costs almost nothing,
    /// and `version` stays the same, so the host can skip uploading.
    pub fn draw(&mut self) -> &Texture {
        // Colors first: a fade that moved a color needs the layout (which bakes colors into draw commands) redone.
        let now = self.now();
        let (colors_changed, moving) = advance_colors(&mut self.root, now);
        self.animating = moving;
        // The wheel goes to a scrolling element under the mouse first, and only what it leaves goes to the page, which
        // takes it anywhere the cursor is in the canvas (not only over an element) if it has something to scroll.
        let (elements_used_x, elements_used_y) = self.scroll_elements();
        let mut laid_out = self.layout_if_changed(colors_changed);
        let mouse = &self.input.mouse;
        let in_canvas = mouse.x >= 0.0 && mouse.x < self.width as f32 && mouse.y >= 0.0 && mouse.y < self.height as f32; // False for NaN.

        // Scrolling moves the cached layout, so it needs no new layout.
        let wheel = self.input.mouse.scroll_y;
        let scroll_before = self.scroll_y;
        if in_canvas {
            self.scroll_y = (self.scroll_y - wheel * self.scroll_step).clamp(0.0, self.max_scroll());
        }
        let page_used_y = in_canvas && wheel != 0.0 && self.scroll_y != scroll_before;
        self.wheel_used = (elements_used_x, elements_used_y || page_used_y);
        self.apply_scroll();

        self.update_elements();

        // An element that changed itself while reacting to input needs the layout redone before it is drawn.
        if self.layout_if_changed(false) {
            self.apply_scroll();
            laid_out = true;
        }

        let scroll = self.applied_scroll;
        let size = (self.width, self.height);
        if laid_out || size != self.painted_size || scroll != self.painted_scroll || self.paints == 0 {
            self.texture.reset(self.width, self.height, self.background);
            self.texture.execute(&self.cmds, -scroll, &mut self.fonts);
            self.texture.execute(&self.fixed_cmds, 0, &mut self.fonts); // Pinned to the canvas, so no scroll.
            self.painted_size = size;
            self.painted_scroll = scroll;
            self.paints += 1;
            self.version += 1;
        }
        &self.texture
    }

    /// Gives this frame's mouse wheel to the deepest scrolling element under the cursor that can still move that way.
    /// An element at the end of its scroll hands the wheel on to the one it is inside, and then the page, as a web page
    /// does. Whatever gets used is taken out of the input, so the page doesn't scroll as well.
    fn scroll_elements(&mut self) -> (bool, bool) {
        let (x, y) = (self.input.mouse.x, self.input.mouse.y);
        let (wheel_x, wheel_y) = (self.input.mouse.scroll_x, self.input.mouse.scroll_y);
        let step = self.scroll_step;
        let mut used = (false, false);
        if wheel_y != 0.0 && scroll_under(&mut self.root, x, y, 0, (-wheel_y * step) as i32) {
            self.input.mouse.scroll_y = 0.0;
            used.1 = true;
        }
        if wheel_x != 0.0 && scroll_under(&mut self.root, x, y, (wheel_x * step) as i32, 0) {
            self.input.mouse.scroll_x = 0.0;
            used.0 = true;
        }
        used
    }

    /// Lays the tree out again if it changed since the last time. Returns whether it did.
    fn layout_if_changed(&mut self, force: bool) -> bool {
        let revision = dirty::revision();
        let size_matches = self.laid_out_width == self.width && (!self.layout_uses_viewport || self.laid_out_height == self.height);
        if !force && self.laid_out_revision == Some(revision) && size_matches {
            return false;
        }

        let avail_w = self.width as i32;
        self.content_height = outer_size(&self.root, avail_w, &self.text, &mut self.fonts).1;
        self.cmds.clear();
        self.fixed_cmds.clear();
        let viewport = Rect { x: 0, y: 0, w: avail_w, h: self.height as i32 };
        let mut view = View::new(viewport, &mut self.fixed_cmds);
        layout(&mut self.root, 0, 0, avail_w, &self.text, &mut self.fonts, &mut self.cmds, &mut view);
        self.layout_uses_viewport = view.uses_viewport;

        // `layout` puts every rect at scroll 0, so the shift starts over.
        self.applied_scroll = 0;
        self.scroll_y = self.scroll_y.clamp(0.0, self.max_scroll());
        self.laid_out_revision = Some(revision);
        self.laid_out_width = self.width;
        self.laid_out_height = self.height;
        self.layouts += 1;
        true
    }

    /// Makes the elements' rects match `scroll_y`, which they do by moving the ones from the last layout.
    fn apply_scroll(&mut self) {
        let target = self.scroll_y.round() as i32;
        if target != self.applied_scroll {
            shift_rects(&mut self.root, self.applied_scroll - target);
            self.applied_scroll = target;
        }
    }

    /// Works out what the mouse is doing to each element, lets each one react to the input, then clears
    /// what only lasts a frame.
    fn update_elements(&mut self) {
        // An element that was removed or hidden can't keep the focus or the press that began on it.
        if self.focus.is_some_and(|uid| !is_shown(&self.root, uid)) {
            self.focus = None;
        }
        for target in &mut self.press_targets {
            if target.is_some_and(|uid| !is_shown(&self.root, uid)) {
                *target = None;
            }
        }

        let focus_before = self.focus;
        let mouse = &self.input.mouse;
        for (target, button) in self.press_targets.iter_mut().zip(BUTTONS) {
            if mouse.pressed & button != 0 {
                *target = hit_test(&self.root, mouse.x, mouse.y);
            }
        }
        if mouse.pressed & BUTTON_LEFT != 0 {
            // Clicking something that takes keyboard input focuses it, and clicking anything else drops the focus.
            self.focus = hit_focusable(&self.root, mouse.x, mouse.y);
        }
        let released = mouse.released;
        self.wants_mouse = hit_test(&self.root, mouse.x, mouse.y).is_some() || self.press_targets.iter().any(Option::is_some);

        let just_focused = apply_focus_requests(&mut self.root, &mut self.focus);
        interact(&mut self.root, &self.input, self.press_targets, released, self.focus, None);
        run_updates(&mut self.root, &self.input, &self.clipboard, &mut self.focus, just_focused);

        // The caret shows only while focused, so a change of focus has to be drawn.
        if self.focus != focus_before {
            dirty::invalidate();
        }

        for (target, button) in self.press_targets.iter_mut().zip(BUTTONS) {
            if !self.input.mouse.down(button) {
                *target = None;
            }
        }
        let mouse = &mut self.input.mouse;
        (mouse.pressed, mouse.released, mouse.scroll_x, mouse.scroll_y) = (0, 0, 0.0, 0.0);
        self.input.keyboard.events.clear();
        self.input.keyboard.text.clear();
    }
}

impl Default for Canvas {
    fn default() -> Canvas {
        Canvas::new()
    }
}

/// Updates the colors every element is showing. Returns whether any changed, and whether any is still fading.
fn advance_colors(el: &mut Element, now: f64) -> (bool, bool) {
    let (mut changed, mut moving) = el.advance(now);
    for child in &mut el.children {
        let (child_changed, child_moving) = advance_colors(child, now);
        changed |= child_changed;
        moving |= child_moving;
    }
    (changed, moving)
}

/// Moves `el` and everything below it down by `dy`.
fn shift_rects(el: &mut Element, dy: i32) {
    if el.style.position == Position::Fixed {
        return; // Pinned to the canvas, so scrolling doesn't move it or anything inside it.
    }
    el.rect.y += dy;
    for child in &mut el.children {
        shift_rects(child, dy);
    }
}

/// Scrolls the deepest scrolling element under the point that can move by `(dx, dy)` pixels. Returns whether one did.
/// An element that clips only has its children looked at if the point is inside it.
fn scroll_under(el: &mut Element, x: f32, y: f32, dx: i32, dy: i32) -> bool {
    if !el.style.visible {
        return false;
    }
    let clips = el.style.overflow != Overflow::Visible;
    if clips && !el.rect.contains(x, y) {
        return false;
    }
    if el.children.iter_mut().rev().any(|child| scroll_under(child, x, y, dx, dy)) {
        return true;
    }
    el.style.overflow == Overflow::Scroll && el.scroll_by(dx, dy)
}

/// The deepest element under the point, or `None`. Later siblings are drawn on top, so they are checked first.
fn hit_test(el: &Element, x: f32, y: f32) -> Option<u32> {
    if !el.style.visible {
        return None;
    }
    // What is cut off by a clipping element can't be reached through it.
    if el.style.overflow != Overflow::Visible && !el.rect.contains(x, y) {
        return None;
    }
    // Children are looked at even when the point isn't in this element: one that is placed can be outside it.
    el.children
        .iter()
        .rev()
        .find_map(|child| hit_test(child, x, y))
        .or_else(|| (el.style.pointer_events == PointerEvents::Auto && el.rect.contains(x, y)).then_some(el.uid))
}

/// The deepest element under the point that takes keyboard input, or `None`.
fn hit_focusable(el: &Element, x: f32, y: f32) -> Option<u32> {
    if !el.style.visible || (el.style.overflow != Overflow::Visible && !el.rect.contains(x, y)) {
        return None;
    }
    el.children
        .iter()
        .rev()
        .find_map(|child| hit_focusable(child, x, y))
        .or_else(|| (el.kind.takes_focus() && el.style.pointer_events == PointerEvents::Auto && el.rect.contains(x, y)).then_some(el.uid))
}

/// Carries out `request_focus` and `request_blur` calls made since the last draw. Returns the element that was
/// just given focus this way, if any.
fn apply_focus_requests(el: &mut Element, focus: &mut Option<u32>) -> Option<u32> {
    let mut just_focused = None;
    if std::mem::take(&mut el.blur_requested) && *focus == Some(el.uid) {
        *focus = None;
    }
    if std::mem::take(&mut el.focus_requested) && el.kind.takes_focus() {
        *focus = Some(el.uid);
        just_focused = Some(el.uid);
    }
    for child in &mut el.children {
        just_focused = apply_focus_requests(child, focus).or(just_focused);
    }
    just_focused
}

/// Sets `interaction` on `el` and everything below it. Returns which mouse buttons were pressed somewhere in this
/// subtree (as a mask), and whether the cursor is over any of it. An element counts as hovered when the cursor is over
/// it or over anything inside it, which matters for a placed child that sits outside its parent's box.
/// `released` is the mask of buttons that came up this frame.
fn interact(el: &mut Element, input: &Input, press_targets: [Option<u32>; 3], released: u8, focus: Option<u32>, clip: Option<Rect>) -> (u8, bool) {
    if !el.style.visible {
        clear_interaction(el);
        return (0, false);
    }
    // The part of the canvas this element can be seen in: not what a clipping ancestor cuts off, except that a
    // fixed element is pinned to the canvas and so isn't cut off by what it happens to be inside.
    let clip = if el.style.position == Position::Fixed { None } else { clip };
    let visible_here = clip.map_or(true, |c| c.contains(input.mouse.x, input.mouse.y));
    let child_clip = if el.style.overflow != Overflow::Visible { Some(clip.map_or(el.rect, |c| c.intersect(el.rect))) } else { clip };

    // Which buttons went down on this element itself.
    let mut holds_press = BUTTONS.iter().zip(press_targets).filter(|(_, target)| *target == Some(el.uid)).fold(0, |mask, (button, _)| mask | button);
    // An element the mouse passes through isn't a target itself, but what is inside it still can be.
    let mut hovered = visible_here && el.style.pointer_events == PointerEvents::Auto && input.mouse.over(el.rect);
    for child in &mut el.children {
        let (child_press, child_hover) = interact(child, input, press_targets, released, focus, child_clip);
        holds_press |= child_press;
        hovered |= child_hover;
    }

    el.interaction.hovered = hovered;
    el.interaction.pressed_buttons = holds_press & input.mouse.buttons;
    el.interaction.clicked_buttons = if hovered { holds_press & released } else { 0 };
    el.interaction.pressed = el.interaction.pressed_buttons & BUTTON_LEFT != 0;
    el.interaction.clicked = el.interaction.clicked_buttons & BUTTON_LEFT != 0;
    el.interaction.focused = focus == Some(el.uid);
    (holds_press, hovered)
}

/// A hidden element, and everything in it, isn't being hovered, pressed, clicked or focused.
fn clear_interaction(el: &mut Element) {
    el.interaction = Default::default();
    for child in &mut el.children {
        clear_interaction(child);
    }
}

/// Whether the element with this id is in the tree and shown, which means it can have focus and be pressed.
fn is_shown(el: &Element, uid: u32) -> bool {
    el.style.visible && (el.uid == uid || el.children.iter().any(|child| is_shown(child, uid)))
}

/// Calls `Element::update` on each element, parents before children.
fn run_updates(el: &mut Element, input: &Input, clipboard: &Clipboard, focus: &mut Option<u32>, just_focused: Option<u32>) {
    if !el.style.visible {
        return;
    }
    let ctx = UpdateCtx {
        input,
        rect: el.rect,
        interaction: el.interaction,
        just_focused: just_focused == Some(el.uid),
        release_focus: Cell::new(false),
        clipboard,
    };
    el.update(&ctx);
    if ctx.release_focus.get() && *focus == Some(el.uid) {
        *focus = None;
    }
    for child in &mut el.children {
        run_updates(child, input, clipboard, focus, just_focused);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::{Key, KeyAction, KeyEvent, BUTTON_MIDDLE, BUTTON_RIGHT};

    /// A canvas 100 wide and `height` tall with three 40px-tall boxes stacked in the root, so 120px of content.
    fn canvas(height: u32) -> Canvas {
        let mut canvas = Canvas::new();
        canvas.width = 100;
        canvas.height = height;
        for _ in 0..3 {
            let el = canvas.root.add_child(Element::stack(Orientation::Vertical));
            el.style.size.x = Some(100);
            el.style.size.y = Some(40);
            el.style.background = Color::BLACK;
        }
        canvas
    }

    fn mouse(canvas: &mut Canvas, x: f32, y: f32, buttons: u8, wheel: f32) {
        canvas.input.set_mouse(x, y, buttons, 0.0, wheel);
    }

    fn box_y(canvas: &Canvas, index: usize) -> i32 {
        canvas.root.children[index].rect.y
    }

    #[test]
    fn nothing_scrolls_when_the_content_fits() {
        let mut canvas = canvas(200);
        mouse(&mut canvas, 5.0, 5.0, 0, -3.0);
        canvas.draw();
        assert_eq!((canvas.scroll_y, canvas.content_height, canvas.max_scroll()), (0.0, 120, 0.0));
        assert_eq!(box_y(&canvas, 0), 0);
    }

    #[test]
    fn the_wheel_moves_everything_up_and_down() {
        let mut canvas = canvas(60); // 120 tall content in a 60 tall canvas: 60px of scroll.
        canvas.scroll_step = 10.0;

        mouse(&mut canvas, 5.0, 5.0, 0, -2.0); // Two notches toward the user: content moves up.
        canvas.draw();
        assert_eq!(canvas.scroll_y, 20.0);
        assert_eq!((box_y(&canvas, 0), box_y(&canvas, 1)), (-20, 20));

        mouse(&mut canvas, 5.0, 5.0, 0, 1.0); // One notch back: content moves down.
        canvas.draw();
        assert_eq!(canvas.scroll_y, 10.0);
        assert_eq!(box_y(&canvas, 0), -10);
    }

    #[test]
    fn scrolling_stops_at_both_ends() {
        let mut canvas = canvas(60);
        canvas.scroll_step = 100.0;
        mouse(&mut canvas, 5.0, 5.0, 0, -5.0);
        canvas.draw();
        assert_eq!(canvas.scroll_y, 60.0, "can't go past the end");

        mouse(&mut canvas, 5.0, 5.0, 0, 5.0);
        canvas.draw();
        assert_eq!(canvas.scroll_y, 0.0, "can't go above the start");
    }

    #[test]
    fn the_wheel_only_counts_once() {
        let mut canvas = canvas(60);
        canvas.scroll_step = 10.0;
        mouse(&mut canvas, 5.0, 5.0, 0, -1.0);
        canvas.draw();
        canvas.draw(); // Drawn again without new input.
        assert_eq!(canvas.scroll_y, 10.0);
    }

    #[test]
    fn resizing_larger_pulls_the_scroll_back_in_range() {
        let mut canvas = canvas(60);
        canvas.scroll_y = 60.0;
        canvas.height = 100;
        canvas.draw();
        assert_eq!(canvas.scroll_y, 20.0);
    }

    /// Not a test: prints where a frame's time goes for a page like the example's.
    /// Run with `cargo test bench_frame -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn bench_frame_phases() {
        for (width, height) in [(854, 480), (1920, 1080), (2560, 1440), (3840, 2160)] {
            println!("--- {width} x {height}");
            bench_frame_at(width, height);
        }
    }

    fn bench_frame_at(width: u32, height: u32) {
        use std::time::Instant;

        let mut canvas = Canvas::new();
        canvas.width = width;
        canvas.height = height;
        canvas.root.style.padding.base = 24;
        let card = canvas.root.add_child(Element::stack(Orientation::Vertical));
        card.style.size.x = Some(380);
        card.style.padding.base = 14;
        card.style.background = Color::BLACK;
        card.style.corner_radius = 16;
        for line in ["Hello, World!", "This card sets no color on its text. It inherits the canvas's, and a container can override it.", "Italic, for emphasis."] {
            card.add_child(Element::text(line));
        }
        for i in 0..24 {
            let item = canvas.root.add_child(Element::stack(Orientation::Vertical));
            item.style.size.x = Some(380);
            item.style.padding.base = 10;
            item.style.background = Color::BLACK;
            item.style.corner_radius = 8;
            item.add_child(Element::text(format!("Row {i}: hover me")));
        }
        canvas.draw(); // Warm up: glyphs get rasterized into the atlas the first time.

        const N: u32 = 100;
        let time = |label: &str, mut f: Box<dyn FnMut() + '_>| {
            let start = Instant::now();
            for _ in 0..N {
                f();
            }
            println!("{label:>22}: {:>8.3} ms", start.elapsed().as_secs_f64() * 1000.0 / f64::from(N));
        };

        let mut cmds = Vec::new();
        let (w, h) = (canvas.width, canvas.height);
        time("full draw (invalidated)", Box::new(|| { crate::dirty::invalidate(); canvas.draw(); }));
        time("idle draw (nothing changed)", Box::new(|| { canvas.draw(); }));
        time("measure only", Box::new(|| { outer_size(&canvas.root, w as i32, &canvas.text, &mut canvas.fonts); }));
        time("layout (measure + emit)", Box::new(|| {
            cmds.clear();
            let mut fixed = Vec::new();
            let mut view = View::new(Rect { x: 0, y: 0, w: w as i32, h: h as i32 }, &mut fixed);
            layout(&mut canvas.root, 0, 0, w as i32, &canvas.text, &mut canvas.fonts, &mut cmds, &mut view);
        }));
        let mut texture = Texture::default();
        time("texture reset", Box::new(|| texture.reset(w, h, Color::WHITE)));
        time("raster cmds", Box::new(|| texture.execute(&cmds, 0, &mut canvas.fonts)));
    }

    #[test]
    fn scrolled_elements_are_hit_where_they_are_drawn() {
        let mut canvas = canvas(60);
        canvas.scroll_y = 40.0; // The second box is now at the top.
        mouse(&mut canvas, 5.0, 5.0, 0, 0.0);
        canvas.draw();
        let hovered: Vec<bool> = canvas.root.children.iter().map(|c| c.interaction.hovered).collect();
        assert_eq!(hovered, vec![false, true, false]);
    }

    #[test]
    fn hover_bubbles_up_and_leaves_with_the_cursor() {
        let mut canvas = canvas(200);
        mouse(&mut canvas, 5.0, 45.0, 0, 0.0);
        canvas.draw();
        assert!(canvas.root.interaction.hovered && canvas.root.children[1].interaction.hovered);
        assert!(!canvas.root.children[0].interaction.hovered);

        mouse(&mut canvas, f32::NAN, f32::NAN, 0, 0.0); // The cursor left the window.
        canvas.draw();
        assert!(!canvas.root.interaction.hovered && !canvas.root.children[1].interaction.hovered);
    }

    #[test]
    fn a_click_is_a_press_and_release_on_the_same_element() {
        let mut canvas = canvas(200);
        let states = |canvas: &Canvas| canvas.root.children.iter().map(|c| (c.interaction.pressed, c.interaction.clicked)).collect::<Vec<_>>();

        mouse(&mut canvas, 5.0, 45.0, BUTTON_LEFT, 0.0);
        canvas.draw();
        assert_eq!(states(&canvas), vec![(false, false), (true, false), (false, false)], "pressed while held");

        mouse(&mut canvas, 5.0, 45.0, BUTTON_LEFT, 0.0);
        canvas.draw();
        assert_eq!(states(&canvas)[1], (true, false), "still held, not clicked yet");

        mouse(&mut canvas, 5.0, 45.0, 0, 0.0);
        canvas.draw();
        assert_eq!(states(&canvas), vec![(false, false), (false, true), (false, false)], "clicked on release");
        assert!(canvas.root.interaction.clicked, "and the click bubbles to the container");

        mouse(&mut canvas, 5.0, 45.0, 0, 0.0);
        canvas.draw();
        assert!(!canvas.root.children[1].interaction.clicked, "a click lasts one frame");
    }

    /// A 200x100 canvas the way a game HUD is: a full-screen container that the mouse passes through, holding a
    /// 40x20 "hotbar" at the top left that is a real target.
    fn hud() -> Canvas {
        let mut canvas = Canvas::new();
        canvas.width = 200;
        canvas.height = 100;
        let screen = canvas.root.add_child(Element::stack(Orientation::Vertical));
        screen.style.size.x = Some(200);
        screen.style.size.y = Some(100);
        screen.style.pointer_events = PointerEvents::None;
        let hotbar = screen.add_child(Element::stack(Orientation::Vertical));
        hotbar.style.size.x = Some(40);
        hotbar.style.size.y = Some(20);
        hotbar.style.background = Color::BLACK;
        canvas
    }

    #[test]
    fn a_hud_container_the_mouse_passes_through_leaves_the_game_clickable() {
        let mut canvas = hud();
        mouse(&mut canvas, 150.0, 70.0, 0, 0.0); // Over the HUD's container, but not its hotbar.
        canvas.draw();
        assert!(!canvas.wants_mouse(), "empty HUD space is the game's");
        assert!(!canvas.root.children[0].interaction.hovered, "and the container itself isn't hovered");

        mouse(&mut canvas, 10.0, 10.0, 0, 0.0); // Over the hotbar.
        canvas.draw();
        assert!(canvas.wants_mouse(), "the hotbar is Pincel's");
        assert!(canvas.root.children[0].children[0].interaction.hovered);
        assert!(canvas.root.children[0].interaction.hovered, "and the container counts as hovered while something in it is");
    }

    #[test]
    fn a_press_that_began_on_something_keeps_the_mouse_until_it_is_released() {
        let mut canvas = hud();
        mouse(&mut canvas, 10.0, 10.0, BUTTON_LEFT, 0.0);
        canvas.draw();
        mouse(&mut canvas, 150.0, 70.0, BUTTON_LEFT, 0.0); // Dragged off onto empty HUD.
        canvas.draw();
        assert!(canvas.wants_mouse(), "a drag that began on the hotbar isn't the game's");
        mouse(&mut canvas, 150.0, 70.0, 0, 0.0);
        canvas.draw();
        assert!(canvas.wants_mouse(), "the frame it is released is still the canvas's: the release ends its press");
        canvas.draw();
        assert!(!canvas.wants_mouse(), "and after that, over nothing, the mouse is the host's again");
    }

    #[test]
    fn the_mouse_goes_through_to_what_is_underneath() {
        let mut canvas = Canvas::new();
        canvas.width = 100;
        canvas.height = 100;
        let under = canvas.root.add_child(Element::stack(Orientation::Vertical));
        under.style.size.x = Some(50);
        under.style.size.y = Some(50);
        under.style.background = Color::BLACK;
        // A see-through overlay on top of it, which comes later in the tree.
        let over = canvas.root.add_child(Element::stack(Orientation::Vertical));
        over.style.position = Position::Absolute;
        over.style.size.x = Some(100);
        over.style.size.y = Some(100);
        over.style.pointer_events = PointerEvents::None;

        click(&mut canvas, 10.0, 10.0);
        assert!(canvas.root.children[0].interaction.clicked, "the element below got the click, not the overlay");
        assert!(!canvas.root.children[1].interaction.clicked && !canvas.root.children[1].interaction.hovered);

        // The same overlay, but taking the mouse, does take the click.
        canvas.root.children[1].style.pointer_events = PointerEvents::Auto;
        click(&mut canvas, 10.0, 10.0);
        assert!(canvas.root.children[1].interaction.clicked && !canvas.root.children[0].interaction.clicked);
    }

    #[test]
    fn each_mouse_button_clicks_on_its_own() {
        let mut canvas = canvas(200);
        let (first, second) = (5.0, 45.0); // Over the first and second of the three boxes.
        let state = |canvas: &Canvas, i: usize| (canvas.root.children[i].interaction.pressed_buttons, canvas.root.children[i].interaction.clicked_buttons);

        // A right press and release on the first box.
        mouse(&mut canvas, 5.0, first, BUTTON_RIGHT, 0.0);
        canvas.draw();
        assert_eq!(state(&canvas, 0), (BUTTON_RIGHT, 0));
        assert!(!canvas.root.children[0].interaction.pressed, "the left-button flag is for the left button only");
        mouse(&mut canvas, 5.0, first, 0, 0.0);
        canvas.draw();
        assert_eq!(state(&canvas, 0), (0, BUTTON_RIGHT));
        assert!(!canvas.root.children[0].interaction.clicked);

        // Left on one box and right on another at the same time, each its own click.
        mouse(&mut canvas, 5.0, first, BUTTON_LEFT, 0.0);
        canvas.draw();
        mouse(&mut canvas, 5.0, second, BUTTON_LEFT | BUTTON_RIGHT, 0.0);
        canvas.draw();
        assert_eq!(state(&canvas, 0).0, BUTTON_LEFT, "the left press began on the first box");
        assert_eq!(state(&canvas, 1).0, BUTTON_RIGHT, "the right press began on the second");
        mouse(&mut canvas, 5.0, second, 0, 0.0); // Both let go over the second box.
        canvas.draw();
        assert_eq!(state(&canvas, 1).1, BUTTON_RIGHT, "right clicks the box it began on, which is under the cursor");
        assert_eq!(state(&canvas, 0).1, 0, "left began on the first, but let go over the second: no click");
    }

    #[test]
    fn middle_clicks_work_too_and_do_not_focus_inputs() {
        let mut canvas = canvas_with_input();
        mouse(&mut canvas, 10.0, 10.0, BUTTON_MIDDLE, 0.0);
        canvas.draw();
        mouse(&mut canvas, 10.0, 10.0, 0, 0.0);
        canvas.draw();
        assert_eq!(canvas.root.children[0].interaction.clicked_buttons, BUTTON_MIDDLE);
        assert!(!canvas.wants_keyboard(), "only the left button focuses");
    }

    #[test]
    fn dragging_off_the_element_before_releasing_is_not_a_click() {
        let mut canvas = canvas(200);
        mouse(&mut canvas, 5.0, 45.0, BUTTON_LEFT, 0.0);
        canvas.draw();
        mouse(&mut canvas, 5.0, 5.0, BUTTON_LEFT, 0.0); // Dragged up onto the first box.
        canvas.draw();
        assert!(canvas.root.children[1].interaction.pressed, "the press stays with where it began");
        mouse(&mut canvas, 5.0, 5.0, 0, 0.0);
        canvas.draw();
        assert!(canvas.root.children.iter().all(|c| !c.interaction.clicked));
    }

    #[test]
    fn pressing_on_empty_space_then_releasing_over_an_element_is_not_a_click() {
        let mut canvas = canvas(200);
        mouse(&mut canvas, 5.0, 150.0, BUTTON_LEFT, 0.0); // Below all three boxes.
        canvas.draw();
        mouse(&mut canvas, 5.0, 5.0, 0, 0.0);
        canvas.draw();
        assert!(canvas.root.children.iter().all(|c| !c.interaction.clicked));
    }

    fn counts(canvas: &Canvas) -> (u64, u64, u64) {
        (canvas.layouts, canvas.paints, canvas.version)
    }

    #[test]
    fn a_frame_where_nothing_changed_does_no_work() {
        let mut canvas = canvas(200);
        canvas.draw();
        let after_first = counts(&canvas);
        assert_eq!((after_first.0, after_first.1), (1, 1));
        for _ in 0..5 {
            canvas.draw();
        }
        assert_eq!(counts(&canvas), after_first, "no new layout, no new pixels, same version");
    }

    #[test]
    fn hover_still_tracks_the_mouse_when_nothing_needs_drawing() {
        let mut canvas = canvas(200);
        canvas.draw();
        let before = counts(&canvas);

        mouse(&mut canvas, 5.0, 45.0, 0, 0.0);
        canvas.draw();
        assert!(canvas.root.children[1].interaction.hovered);
        mouse(&mut canvas, 5.0, 5.0, 0, 0.0);
        canvas.draw();
        assert!(canvas.root.children[0].interaction.hovered && !canvas.root.children[1].interaction.hovered);
        assert_eq!(counts(&canvas), before, "the mouse moving is not a reason to redraw by itself");
    }

    #[test]
    fn setting_what_is_already_set_costs_nothing_but_a_real_change_lays_out_again() {
        use crate::dirty::set;
        let mut canvas = canvas(200);
        let text = canvas.root.add_child(Element::text("hello"));
        let text_ptr: *mut Element = text;
        canvas.draw();
        let before = counts(&canvas);

        // The tree is only touched from this test's thread and the pointer stays valid, since children are boxed.
        let text = unsafe { &mut *text_ptr };
        set(&mut canvas.root.children[0].style.size.y, Some(40));
        assert!(text.set_text("hello"));
        canvas.draw();
        assert_eq!(counts(&canvas), before, "same values");

        set(&mut canvas.root.children[0].style.size.y, Some(50));
        canvas.draw();
        assert_eq!((canvas.layouts, canvas.paints), (before.0 + 1, before.1 + 1));
        assert_eq!(canvas.root.children[1].rect.y, 50, "and the new size took effect");

        assert!(unsafe { &mut *text_ptr }.set_text("different"));
        canvas.draw();
        assert_eq!(canvas.layouts, before.0 + 2);
    }

    #[test]
    fn scrolling_redraws_but_does_not_lay_out_again() {
        let mut canvas = canvas(60);
        canvas.scroll_step = 10.0;
        canvas.draw();
        let before = counts(&canvas);

        mouse(&mut canvas, 5.0, 5.0, 0, -1.0);
        canvas.draw();
        assert_eq!((canvas.layouts, canvas.paints), (before.0, before.1 + 1));
        assert!(canvas.version > before.2);
        assert_eq!(box_y(&canvas, 0), -10, "the cached rects moved with it");
    }

    #[test]
    fn resizing_lays_out_again_only_when_the_width_changed() {
        let mut canvas = canvas(200);
        canvas.draw();
        let before = counts(&canvas);

        canvas.height = 150;
        canvas.draw();
        assert_eq!((canvas.layouts, canvas.paints), (before.0, before.1 + 1), "a new height needs new pixels, not new layout");
        assert_eq!(canvas.texture.height, 150);

        canvas.width = 80;
        canvas.draw();
        assert_eq!(canvas.layouts, before.0 + 1, "text could wrap differently at a new width");
    }

    use crate::element::ElementKind;
    use crate::text_input::{InputKind, InputState};

    /// A 200x100 canvas whose only child is a 120x24 single-line input at the top left.
    fn canvas_with_input() -> Canvas {
        let mut canvas = Canvas::new();
        canvas.width = 200;
        canvas.height = 100;
        let input = canvas.root.add_child(Element::input(InputKind::Text));
        input.style.size.x = Some(120);
        input.style.size.y = Some(24);
        input.style.background = Color::WHITE;
        canvas
    }

    fn input_state(canvas: &Canvas) -> &InputState {
        match &canvas.root.children[0].kind {
            ElementKind::Input(state) => state,
            other => panic!("expected an input, got {other:?}"),
        }
    }

    fn type_text(canvas: &mut Canvas, text: &str) {
        canvas.input.set_keyboard(Vec::new(), text.to_string(), 0);
        canvas.draw();
    }

    fn press_key(canvas: &mut Canvas, key: Key) {
        canvas.input.set_keyboard(vec![KeyEvent { key, action: KeyAction::Press }], String::new(), 0);
        canvas.draw();
    }

    fn click(canvas: &mut Canvas, x: f32, y: f32) {
        mouse(canvas, x, y, BUTTON_LEFT, 0.0);
        canvas.draw();
        mouse(canvas, x, y, 0, 0.0);
        canvas.draw();
    }

    #[test]
    fn clicking_an_input_focuses_it_and_typing_edits_it() {
        let mut canvas = canvas_with_input();
        canvas.draw();
        assert!(!canvas.wants_keyboard());
        type_text(&mut canvas, "ignored"); // Not focused, so the keyboard isn't for it.
        assert_eq!(input_state(&canvas).value, "");

        click(&mut canvas, 10.0, 10.0);
        assert!(canvas.wants_keyboard() && canvas.root.children[0].interaction.focused);

        type_text(&mut canvas, "hi");
        let state = input_state(&canvas);
        assert_eq!((state.value.as_str(), state.caret, state.changed), ("hi", 2, true));
        canvas.draw();
        assert!(!input_state(&canvas).changed, "changed lasts one frame");

        press_key(&mut canvas, Key::Backspace);
        assert_eq!(input_state(&canvas).value, "h");
    }

    fn press_with(canvas: &mut Canvas, keys: &[Key], modifiers: u8) {
        let events = keys.iter().map(|&key| KeyEvent { key, action: KeyAction::Press }).collect();
        canvas.input.set_keyboard(events, String::new(), modifiers);
        canvas.draw();
    }

    /// Where character boundary `i` of the input's first line is, on the canvas (the input is at the left edge).
    fn boundary(canvas: &Canvas, i: usize) -> f32 {
        input_state(canvas).geometry.columns[0][i]
    }

    /// Clicks (press and release) `offset` pixels right of boundary `i`, at the middle of the input's height.
    fn click_at(canvas: &mut Canvas, i: usize, offset: f32) {
        let x = boundary(canvas, i) + offset;
        click(canvas, x, 10.0);
    }

    /// Moves the mouse to `offset` pixels right of boundary `i` with these buttons held, and draws.
    fn mouse_at(canvas: &mut Canvas, i: usize, offset: f32, buttons: u8) {
        let x = boundary(canvas, i) + offset;
        mouse(canvas, x, 10.0, buttons, 0.0);
        canvas.draw();
    }

    fn selection_boxes(canvas: &Canvas) -> usize {
        let color = crate::style::Style::default().selection_color;
        canvas.cmds.iter().filter(|c| matches!(c, DrawCmd::Box { background, rect, .. } if *background == color && rect.w > 0)).count()
    }

    #[test]
    fn a_click_in_the_text_puts_the_caret_between_the_characters_it_landed_nearest() {
        let mut canvas = canvas_with_input();
        canvas.root.children[0].set_text("hello world");
        canvas.draw();
        click_at(&mut canvas, 5, 1.0);
        assert_eq!((input_state(&canvas).caret, input_state(&canvas).anchor), (5, 5));

        click_at(&mut canvas, 2, -1.0);
        assert_eq!(input_state(&canvas).caret, 2);
        click(&mut canvas, 115.0, 10.0); // Past the end of the text, still inside the box.
        assert_eq!(input_state(&canvas).caret, 11);
    }

    #[test]
    fn dragging_across_the_text_selects_it_and_it_is_drawn() {
        let mut canvas = canvas_with_input();
        canvas.root.children[0].set_text("hello world");
        canvas.draw();
        mouse_at(&mut canvas, 2, 1.0, BUTTON_LEFT);
        assert_eq!(selection_boxes(&canvas), 0, "just a caret so far");

        mouse_at(&mut canvas, 7, 1.0, BUTTON_LEFT); // Held, and moved.
        assert_eq!((input_state(&canvas).selection(), input_state(&canvas).selected_text().as_str()), (Some((2, 7)), "llo w"));
        assert_eq!(selection_boxes(&canvas), 1, "a highlight behind the selected characters");
        let carets = canvas.cmds.iter().filter(|c| matches!(c, DrawCmd::Box { rect, .. } if rect.w == 2 && rect.h > 4)).count();
        assert_eq!(carets, 0, "and no caret while something is selected");

        mouse_at(&mut canvas, 7, 1.0, 0);
        assert_eq!(input_state(&canvas).selection(), Some((2, 7)), "it stays after the mouse is released");
        type_text(&mut canvas, "X");
        assert_eq!(input_state(&canvas).value, "heXorld", "and typing replaces it");
    }

    #[test]
    fn shift_click_extends_a_selection() {
        let mut canvas = canvas_with_input();
        canvas.root.children[0].set_text("hello world");
        canvas.draw();
        click_at(&mut canvas, 2, 1.0);
        canvas.input.set_keyboard(Vec::new(), String::new(), crate::input::MOD_SHIFT);
        click_at(&mut canvas, 8, 1.0);
        assert_eq!(input_state(&canvas).selection(), Some((2, 8)));
    }

    #[test]
    fn the_selection_is_only_shown_while_the_input_has_focus() {
        let mut canvas = canvas_with_input();
        canvas.root.children[0].set_text("hello");
        canvas.draw();
        click(&mut canvas, 10.0, 10.0);
        press_with(&mut canvas, &[Key::A], crate::input::MOD_CONTROL);
        assert_eq!(selection_boxes(&canvas), 1);
        click(&mut canvas, 150.0, 80.0); // Click away.
        assert_eq!(selection_boxes(&canvas), 0, "hidden when unfocused");
        assert_eq!(input_state(&canvas).selection(), Some((0, 5)), "but remembered");
    }

    #[test]
    fn copy_and_paste_work_through_the_canvas_clipboard() {
        let mut canvas = canvas_with_input();
        click(&mut canvas, 10.0, 10.0);
        type_text(&mut canvas, "abc");
        press_with(&mut canvas, &[Key::A], crate::input::MOD_CONTROL);
        press_with(&mut canvas, &[Key::C], crate::input::MOD_CONTROL);
        assert_eq!(canvas.clipboard.get(), "abc");
        press_with(&mut canvas, &[Key::Right], 0); // Collapses to the end.
        press_with(&mut canvas, &[Key::V], crate::input::MOD_CONTROL);
        assert_eq!(input_state(&canvas).value, "abcabc");

        press_with(&mut canvas, &[Key::A], crate::input::MOD_CONTROL);
        press_with(&mut canvas, &[Key::X], crate::input::MOD_CONTROL);
        assert_eq!((input_state(&canvas).value.as_str(), canvas.clipboard.get().as_str()), ("", "abcabc"));
    }

    #[test]
    fn a_second_input_pastes_what_the_first_copied() {
        let mut canvas = canvas_with_input();
        let second = canvas.root.add_child(Element::input(InputKind::Text));
        second.style.size.x = Some(120);
        second.style.size.y = Some(24);
        canvas.draw();

        click(&mut canvas, 10.0, 10.0);
        type_text(&mut canvas, "shared");
        press_with(&mut canvas, &[Key::A, Key::C], crate::input::MOD_CONTROL);
        click(&mut canvas, 10.0, 35.0); // The second input is under the first.
        press_with(&mut canvas, &[Key::V], crate::input::MOD_CONTROL);
        match &canvas.root.children[1].kind {
            ElementKind::Input(state) => assert_eq!(state.value, "shared"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn clicking_does_not_move_the_caret_of_an_input_that_was_not_clicked() {
        let mut canvas = canvas_with_input();
        canvas.root.children[0].set_text("hello");
        canvas.draw();
        let before = input_state(&canvas).caret;
        click(&mut canvas, 150.0, 80.0); // Beside it.
        assert_eq!(input_state(&canvas).caret, before);
    }

    #[test]
    fn clicking_elsewhere_or_pressing_escape_drops_the_focus() {
        let mut canvas = canvas_with_input();
        click(&mut canvas, 10.0, 10.0);
        assert!(canvas.wants_keyboard());
        click(&mut canvas, 150.0, 80.0); // Empty canvas.
        assert!(!canvas.wants_keyboard());
        type_text(&mut canvas, "x");
        assert_eq!(input_state(&canvas).value, "");

        click(&mut canvas, 10.0, 10.0);
        press_key(&mut canvas, Key::Escape);
        assert!(!canvas.wants_keyboard());
        assert!(!canvas.root.children[0].interaction.focused || canvas.focus.is_none());
    }

    #[test]
    fn enter_reports_a_submit_for_one_frame() {
        let mut canvas = canvas_with_input();
        click(&mut canvas, 10.0, 10.0);
        type_text(&mut canvas, "go");
        press_key(&mut canvas, Key::Enter);
        assert!(input_state(&canvas).submitted);
        assert!(canvas.wants_keyboard(), "submitting doesn't drop the focus");
        canvas.draw();
        assert!(!input_state(&canvas).submitted);
    }

    /// A 200x40 canvas with a 100x60 list at the top left, holding five 100x30 items (150px of content, so 90 of scroll).
    fn canvas_with_list(overflow: Overflow) -> Canvas {
        let mut canvas = Canvas::new();
        canvas.width = 200;
        canvas.height = 40;
        canvas.scroll_step = 10.0;
        let list = canvas.root.add_child(Element::stack(Orientation::Vertical));
        list.style.size.x = Some(100);
        list.style.size.y = Some(60);
        list.style.overflow = overflow;
        list.style.background = Color::WHITE;
        for _ in 0..5 {
            let item = list.add_child(Element::stack(Orientation::Vertical));
            item.style.size.x = Some(100);
            item.style.size.y = Some(30);
            item.style.background = Color::BLACK;
        }
        canvas
    }

    fn list(canvas: &Canvas) -> &Element {
        &canvas.root.children[0]
    }

    #[test]
    fn a_scrolling_element_knows_how_much_of_its_content_is_out_of_view() {
        let mut canvas = canvas_with_list(Overflow::Scroll);
        canvas.draw();
        assert_eq!((list(&canvas).scroll.max_x, list(&canvas).scroll.max_y), (0, 90));
        assert_eq!(list(&canvas).rect.h, 60, "it keeps its own size");
        assert_eq!(canvas.content_height, 60, "and the page's content is the list, not what is inside it");
    }

    #[test]
    fn its_children_are_clipped_to_its_box_but_it_is_not() {
        let mut canvas = canvas_with_list(Overflow::Scroll);
        canvas.draw();
        let clips: Vec<Option<Rect>> = canvas.cmds.iter().map(|c| match c { DrawCmd::Box { clip, .. } => *clip, _ => panic!() }).collect();
        assert_eq!(clips[0], None, "the list's own background isn't cut by itself");
        assert!(clips[1..].iter().all(|c| *c == Some(Rect { x: 0, y: 0, w: 100, h: 60 })), "its items are: {clips:?}");
    }

    #[test]
    fn the_wheel_over_a_scrolling_element_scrolls_it_and_not_the_page() {
        let mut canvas = canvas_with_list(Overflow::Scroll);
        canvas.draw();
        mouse(&mut canvas, 10.0, 10.0, 0, -2.0);
        canvas.draw();
        assert_eq!(list(&canvas).scroll.y, 20);
        assert_eq!(canvas.scroll_y, 0.0, "the page stayed where it was, though it could have scrolled");
        assert_eq!(list(&canvas).children[0].rect.y, -20, "and the items moved");
        assert_eq!(list(&canvas).rect.y, 0, "while the list didn't");

        mouse(&mut canvas, 10.0, 10.0, 0, 1.0);
        canvas.draw();
        assert_eq!(list(&canvas).scroll.y, 10, "and back");
    }

    #[test]
    fn the_wheel_over_empty_canvas_still_scrolls_the_page() {
        let mut canvas = canvas_with_list(Overflow::Scroll);
        canvas.draw();
        mouse(&mut canvas, 150.0, 10.0, 0, -2.0); // Beside the list, over no element at all.
        canvas.draw();
        assert!(!canvas.wants_mouse(), "the mouse is free for clicks there");
        assert_eq!((list(&canvas).scroll.y, canvas.scroll_y), (0, 20.0), "but the page scrolls, since it can");
        assert_eq!(canvas.wheel_consumed(), (false, true));
    }

    #[test]
    fn a_page_with_nothing_to_scroll_leaves_the_wheel_to_the_host() {
        let mut canvas = canvas_with_list(Overflow::Scroll);
        canvas.height = 100; // Everything fits.
        canvas.draw();
        mouse(&mut canvas, 150.0, 10.0, 0, -2.0);
        canvas.draw();
        assert_eq!((canvas.scroll_y, canvas.wheel_consumed()), (0.0, (false, false)), "nothing moved, so the host's");
    }

    #[test]
    fn a_wheel_with_the_cursor_outside_the_canvas_is_not_used() {
        let mut canvas = canvas_with_list(Overflow::Scroll);
        canvas.draw();
        mouse(&mut canvas, f32::NAN, f32::NAN, 0, -2.0); // The cursor isn't over the canvas at all.
        canvas.draw();
        assert_eq!((canvas.scroll_y, canvas.wheel_consumed()), (0.0, (false, false)));
        mouse(&mut canvas, 500.0, 10.0, 0, -2.0); // Off the right edge of a 200 wide canvas.
        canvas.draw();
        assert_eq!(canvas.scroll_y, 0.0);
    }

    #[test]
    fn the_wheel_over_the_page_content_scrolls_the_page_and_says_so() {
        let mut canvas = canvas_with_list(Overflow::Visible); // Not scrolling itself.
        canvas.draw();
        mouse(&mut canvas, 10.0, 10.0, 0, -2.0);
        canvas.draw();
        assert_eq!(canvas.scroll_y, 20.0);
        assert_eq!(canvas.wheel_consumed(), (false, true));
    }

    #[test]
    fn a_scrolling_element_reports_the_wheel_as_used_and_one_at_its_end_does_not() {
        let mut canvas = canvas_with_list(Overflow::Scroll);
        canvas.height = 60; // The page has nothing to scroll either.
        canvas.draw();
        mouse(&mut canvas, 10.0, 10.0, 0, -1.0);
        canvas.draw();
        assert_eq!(canvas.wheel_consumed(), (false, true), "the list took it");

        canvas.root.children[0].set_scroll(0, 90);
        canvas.draw();
        mouse(&mut canvas, 10.0, 10.0, 0, -1.0);
        canvas.draw();
        assert_eq!(canvas.wheel_consumed(), (false, false), "the list is at its end and the page can't move: the host's again");
    }

    #[test]
    fn an_element_at_the_end_of_its_scroll_hands_the_wheel_to_the_page() {
        let mut canvas = canvas_with_list(Overflow::Scroll);
        canvas.draw();
        canvas.root.children[0].set_scroll(0, 90);
        canvas.draw();
        assert_eq!(list(&canvas).scroll.y, 90);

        mouse(&mut canvas, 10.0, 10.0, 0, -1.0); // Further down than it goes.
        canvas.draw();
        assert_eq!((list(&canvas).scroll.y, canvas.scroll_y), (90, 10.0), "so the page takes it");
    }

    #[test]
    fn the_innermost_scroller_gets_the_wheel_and_passes_it_out_when_it_is_done() {
        let mut canvas = canvas_with_list(Overflow::Scroll);
        canvas.height = 200;
        // Put a smaller scroller inside the first item: 50 tall with 80 of content.
        let inner = canvas.root.children[0].children[0].add_child(Element::stack(Orientation::Vertical));
        inner.style.size.x = Some(50);
        inner.style.size.y = Some(20);
        inner.style.overflow = Overflow::Scroll;
        for _ in 0..2 {
            let line = inner.add_child(Element::stack(Orientation::Vertical));
            line.style.size.x = Some(50);
            line.style.size.y = Some(15);
        }
        canvas.draw();
        assert_eq!(canvas.root.children[0].children[0].children[0].scroll.max_y, 10);

        mouse(&mut canvas, 5.0, 5.0, 0, -1.0);
        canvas.draw();
        assert_eq!((canvas.root.children[0].children[0].children[0].scroll.y, list(&canvas).scroll.y), (10, 0), "inner first");
        mouse(&mut canvas, 5.0, 5.0, 0, -1.0);
        canvas.draw();
        assert_eq!(list(&canvas).scroll.y, 10, "then, with the inner one at its end, the outer");
    }

    #[test]
    fn what_is_scrolled_out_of_view_cannot_be_hovered_or_clicked() {
        let mut canvas = canvas_with_list(Overflow::Scroll);
        canvas.height = 200;
        canvas.draw();
        // The fourth item is at y = 90..120 in content, so it is well below the 60px list, but over empty canvas.
        let item = |canvas: &Canvas, i: usize| canvas.root.children[0].children[i].interaction;
        mouse(&mut canvas, 10.0, 100.0, BUTTON_LEFT, 0.0);
        canvas.draw();
        mouse(&mut canvas, 10.0, 100.0, 0, 0.0);
        canvas.draw();
        assert!(!item(&canvas, 3).hovered && !item(&canvas, 3).clicked, "it is there on paper, but cut off");
        assert!(!list(&canvas).interaction.hovered);

        mouse(&mut canvas, 10.0, 40.0, 0, 0.0); // Inside the list, over the second item.
        canvas.draw();
        assert!(item(&canvas, 1).hovered && !item(&canvas, 0).hovered);
    }

    #[test]
    fn hidden_overflow_clips_but_does_not_scroll() {
        let mut canvas = canvas_with_list(Overflow::Hidden);
        canvas.draw();
        assert_eq!(list(&canvas).scroll.max_y, 0);
        assert!(canvas.cmds.iter().skip(1).all(|c| matches!(c, DrawCmd::Box { clip: Some(_), .. })), "still clipped");
        mouse(&mut canvas, 10.0, 10.0, 0, -2.0);
        canvas.draw();
        assert_eq!((list(&canvas).scroll.y, canvas.scroll_y), (0, 20.0), "so the wheel goes to the page");
    }

    #[test]
    fn a_scroller_can_be_set_to_follow_its_content() {
        let mut canvas = canvas_with_list(Overflow::Scroll);
        canvas.root.children[0].style.stick_to_end = true;
        canvas.draw();
        assert_eq!(list(&canvas).scroll.y, 90, "starts at the end");

        let item = canvas.root.children[0].add_child(Element::stack(Orientation::Vertical));
        item.style.size.x = Some(100);
        item.style.size.y = Some(30);
        canvas.draw();
        assert_eq!((list(&canvas).scroll.y, list(&canvas).scroll.max_y), (120, 120), "it followed the new content");

        // Once the user has scrolled up, it leaves them where they are.
        canvas.root.children[0].set_scroll(0, 50);
        canvas.draw();
        let item = canvas.root.children[0].add_child(Element::stack(Orientation::Vertical));
        item.style.size.x = Some(100);
        item.style.size.y = Some(30);
        canvas.draw();
        assert_eq!((list(&canvas).scroll.y, list(&canvas).scroll.max_y), (50, 150));
    }

    #[test]
    fn asking_to_scroll_too_far_stops_at_the_end() {
        let mut canvas = canvas_with_list(Overflow::Scroll);
        canvas.draw();
        canvas.root.children[0].set_scroll(0, 10_000);
        canvas.draw();
        assert_eq!(list(&canvas).scroll.y, 90);
        canvas.root.children[0].set_scroll(0, 0);
        canvas.draw();
        assert_eq!(list(&canvas).scroll.y, 0);
    }

    #[test]
    fn placed_children_stay_put_in_a_scroller_and_a_fixed_one_is_not_clipped_by_it() {
        let mut canvas = canvas_with_list(Overflow::Scroll);
        let badge = canvas.root.children[0].add_child(Element::stack(Orientation::Vertical));
        badge.style.position = Position::Absolute;
        badge.style.size.x = Some(10);
        badge.style.size.y = Some(10);
        badge.style.inset.left = Some(5);
        badge.style.inset.top = Some(5);
        badge.style.background = Color::WHITE;
        let bar = canvas.root.children[0].add_child(Element::stack(Orientation::Vertical));
        bar.style.position = Position::Fixed;
        bar.style.size.x = Some(10);
        bar.style.size.y = Some(10);
        bar.style.background = Color::WHITE;
        canvas.draw();
        canvas.root.children[0].set_scroll(0, 40);
        canvas.draw();

        let badge = &canvas.root.children[0].children[5];
        assert_eq!(badge.rect.y, 5, "it didn't scroll with the content");
        assert!(canvas.cmds.iter().any(|c| matches!(c, DrawCmd::Box { rect, clip: Some(_), .. } if rect.w == 10)), "but it is clipped by the box");
        assert!(canvas.fixed_cmds.iter().all(|c| matches!(c, DrawCmd::Box { clip: None, .. })), "the pinned one is not");
    }

    /// A canvas with one 20x20 box (red, with a one second linear transition) and a text line inside it.
    fn canvas_with_fading_box() -> Canvas {
        use crate::anim::Transition;
        use crate::style::Easing;
        let mut canvas = Canvas::new();
        canvas.width = 100;
        canvas.height = 100;
        canvas.set_time(0.0);
        let el = canvas.root.add_child(Element::stack(Orientation::Vertical));
        el.style.size.x = Some(20);
        el.style.size.y = Some(20);
        el.style.background = Color { r: 255, g: 0, b: 0, a: 1.0 };
        el.style.foreground = Some(Color { r: 255, g: 0, b: 0, a: 1.0 });
        el.style.transition = Transition { duration: 1.0, easing: Easing::Linear };
        el.add_child(Element::text("hi"));
        canvas
    }

    const BLUE: Color = Color { r: 0, g: 0, b: 255, a: 1.0 };

    fn box_background(canvas: &Canvas) -> Color {
        canvas.cmds.iter().find_map(|c| match c { DrawCmd::Box { background, .. } => Some(*background), _ => None }).unwrap()
    }

    fn text_color(canvas: &Canvas) -> Color {
        canvas.cmds.iter().find_map(|c| match c { DrawCmd::Text { color, .. } => Some(*color), _ => None }).unwrap()
    }

    #[test]
    fn a_color_change_fades_over_the_transitions_duration() {
        let mut canvas = canvas_with_fading_box();
        canvas.draw();
        assert!(!canvas.is_animating());

        crate::dirty::set(&mut canvas.root.children[0].style.background, BLUE);
        canvas.draw(); // At t = 0, the moment it was changed: still red.
        assert_eq!(box_background(&canvas).r, 255);
        assert!(canvas.is_animating());

        canvas.set_time(0.5);
        canvas.draw();
        let half = box_background(&canvas);
        assert_eq!((half.r, half.b), (128, 128), "halfway");
        assert!(canvas.is_animating());

        canvas.set_time(1.0);
        canvas.draw();
        assert_eq!(box_background(&canvas), BLUE, "arrived, and drawn that way");
        assert!(!canvas.is_animating());
    }

    #[test]
    fn a_finished_transition_costs_nothing_again() {
        let mut canvas = canvas_with_fading_box();
        canvas.draw();
        crate::dirty::set(&mut canvas.root.children[0].style.background, BLUE);
        for t in [0.0, 0.5, 1.0] {
            canvas.set_time(t);
            canvas.draw();
        }
        let done = counts(&canvas);
        for t in [1.5, 2.0, 3.0] {
            canvas.set_time(t);
            canvas.draw();
        }
        assert_eq!(counts(&canvas), done, "no layout, no pixels, same version");
    }

    #[test]
    fn setting_the_same_target_again_does_not_restart_the_fade() {
        // Restyling every frame to the same value, as the example's hover does, must not keep resetting it.
        let mut canvas = canvas_with_fading_box();
        canvas.draw();
        for t in [0.0, 0.25, 0.5, 0.75] {
            canvas.set_time(t);
            crate::dirty::set(&mut canvas.root.children[0].style.background, BLUE);
            canvas.draw();
        }
        let near_end = box_background(&canvas);
        assert_eq!((near_end.r, near_end.b), (64, 191), "three quarters of the way, not starting over");
    }

    #[test]
    fn changing_direction_mid_fade_does_not_jump() {
        let mut canvas = canvas_with_fading_box();
        canvas.draw();
        crate::dirty::set(&mut canvas.root.children[0].style.background, BLUE);
        canvas.draw();
        canvas.set_time(0.5);
        canvas.draw();
        let showing = box_background(&canvas);

        crate::dirty::set(&mut canvas.root.children[0].style.background, Color { r: 255, g: 0, b: 0, a: 1.0 });
        canvas.draw(); // Same instant, new direction.
        assert_eq!(box_background(&canvas), showing);
        canvas.set_time(1.0);
        canvas.draw();
        let back = box_background(&canvas);
        assert!(back.r > showing.r && back.r < 255, "on its way back to red");
    }

    #[test]
    fn without_a_transition_a_change_is_immediate() {
        let mut canvas = canvas_with_fading_box();
        canvas.root.children[0].style.transition = crate::anim::Transition::default();
        canvas.draw();
        crate::dirty::set(&mut canvas.root.children[0].style.background, BLUE);
        canvas.draw();
        assert_eq!(box_background(&canvas), BLUE);
        assert!(!canvas.is_animating());
    }

    #[test]
    fn inherited_text_color_fades_with_the_element_that_sets_it() {
        let mut canvas = canvas_with_fading_box();
        canvas.draw();
        crate::dirty::set(&mut canvas.root.children[0].style.foreground, Some(BLUE));
        canvas.draw();
        canvas.set_time(0.5);
        canvas.draw();
        let color = text_color(&canvas);
        assert_eq!((color.r, color.b), (128, 128), "the text child, which sets no color, follows its parent's fade");
    }

    #[test]
    fn a_hidden_element_takes_no_room_draws_nothing_and_gets_no_input() {
        let mut canvas = canvas(200); // Three 40px boxes.
        canvas.draw();
        assert_eq!((canvas.content_height, canvas.cmds.len()), (120, 3));

        crate::dirty::set(&mut canvas.root.children[1].style.visible, false);
        mouse(&mut canvas, 5.0, 45.0, 0, 0.0); // Where the second box was.
        canvas.draw();
        assert_eq!((canvas.content_height, canvas.cmds.len()), (80, 2), "two boxes left");
        assert_eq!(box_y(&canvas, 2), 40, "the third moved up into its place");
        assert!(!canvas.root.children[1].interaction.hovered);
        assert!(canvas.root.children[2].interaction.hovered, "and it is what is under the cursor now");

        crate::dirty::set(&mut canvas.root.children[1].style.visible, true);
        canvas.draw();
        assert_eq!(canvas.content_height, 120, "showing it again restores it");
    }

    #[test]
    fn removing_or_hiding_the_focused_input_drops_the_focus() {
        let mut canvas = canvas_with_input();
        click(&mut canvas, 10.0, 10.0);
        assert!(canvas.wants_keyboard());
        crate::dirty::set(&mut canvas.root.children[0].style.visible, false);
        canvas.draw();
        assert!(!canvas.wants_keyboard(), "hidden");

        crate::dirty::set(&mut canvas.root.children[0].style.visible, true);
        click(&mut canvas, 10.0, 10.0);
        assert!(canvas.wants_keyboard());
        let removed = canvas.root.remove_child(0);
        assert!(removed.is_some());
        canvas.draw();
        assert!(!canvas.wants_keyboard(), "removed");
    }

    #[test]
    fn asking_for_focus_ignores_the_keypress_that_asked() {
        let mut canvas = canvas_with_input();
        canvas.draw();
        canvas.root.children[0].request_focus();
        // The same frame still has the "t" that made the host open the input, as typed text.
        type_text(&mut canvas, "t");
        assert!(canvas.wants_keyboard());
        assert_eq!(input_state(&canvas).value, "", "it didn't land in the input");
        type_text(&mut canvas, "x");
        assert_eq!(input_state(&canvas).value, "x", "but the next frame's typing does");

        canvas.root.children[0].request_blur();
        canvas.draw();
        assert!(!canvas.wants_keyboard());
    }

    #[test]
    fn the_caret_is_drawn_only_while_focused() {
        let caret_boxes = |canvas: &Canvas| canvas.cmds.iter().filter(|c| matches!(c, DrawCmd::Box { rect, .. } if rect.w == 2 && rect.h > 2)).count();
        let mut canvas = canvas_with_input();
        canvas.draw();
        assert_eq!(caret_boxes(&canvas), 0);
        click(&mut canvas, 10.0, 10.0);
        assert_eq!(caret_boxes(&canvas), 1);
        click(&mut canvas, 150.0, 80.0);
        assert_eq!(caret_boxes(&canvas), 0);
    }

    #[test]
    fn a_long_value_scrolls_to_keep_the_caret_in_view() {
        let mut canvas = canvas_with_input();
        click(&mut canvas, 10.0, 10.0);
        type_text(&mut canvas, &"w".repeat(60)); // Far wider than 120px.
        let state = input_state(&canvas);
        assert!(state.scroll_x > 0, "scrolled right to follow the caret");
        // Everything the input draws is clipped to its box.
        assert!(canvas.cmds.iter().all(|c| match c { DrawCmd::Text { clip, .. } => clip.is_some(), _ => true }));

        press_key(&mut canvas, Key::Home);
        assert_eq!(input_state(&canvas).scroll_x, 0, "and back when the caret goes home");
    }

    #[test]
    fn a_placeholder_shows_while_empty_and_goes_when_typed_over() {
        let texts = |canvas: &Canvas| canvas.cmds.iter().filter(|c| matches!(c, DrawCmd::Text { .. })).count();
        let mut canvas = canvas_with_input();
        if let ElementKind::Input(state) = &mut canvas.root.children[0].kind {
            state.placeholder = "Type here".into();
        }
        canvas.draw();
        assert_eq!(texts(&canvas), 1, "the placeholder");
        click(&mut canvas, 10.0, 10.0);
        type_text(&mut canvas, "a");
        assert_eq!(texts(&canvas), 1, "the value now");
        assert!(matches!(canvas.cmds.iter().find(|c| matches!(c, DrawCmd::Text { .. })), Some(DrawCmd::Text { text, .. }) if text == "a"));
    }

    #[test]
    fn set_text_changes_an_inputs_value_and_set_image_changes_an_image() {
        let mut canvas = canvas_with_input();
        canvas.draw();
        assert!(canvas.root.children[0].set_text("preset"));
        canvas.draw();
        assert_eq!((input_state(&canvas).value.as_str(), input_state(&canvas).caret), ("preset", 6));
        assert_eq!(canvas.root.children[0].text_value().as_deref(), Some("preset"));

        let pixel = |rgb: [u8; 3]| crate::element::Image::from_rgba(1, 1, vec![rgb[0], rgb[1], rgb[2], 255]).unwrap();
        canvas.root.add_child(Element::image(pixel([9, 9, 9])));
        canvas.draw();
        let before = canvas.layouts;
        assert!(canvas.root.children[1].set_image(pixel([9, 9, 9])), "the same pixels");
        canvas.draw();
        assert_eq!(canvas.layouts, before, "nothing changed, so nothing to redo");
        assert!(canvas.root.children[1].set_image(pixel([1, 2, 3])));
        canvas.draw();
        assert_eq!(canvas.layouts, before + 1);
        assert!(!canvas.root.children[0].set_image(pixel([0, 0, 0])), "an input isn't an image");
    }

    /// A canvas 100x60 with 120px of content, plus a fixed sidebar 20 wide down the right edge.
    fn canvas_with_sidebar() -> Canvas {
        let mut canvas = canvas(60);
        let bar = canvas.root.add_child(Element::stack(Orientation::Vertical));
        bar.style.position = Position::Fixed;
        bar.style.size.x = Some(20);
        bar.style.inset.right = Some(0);
        bar.style.inset.top = Some(0);
        bar.style.inset.bottom = Some(0);
        bar.style.background = Color { r: 200, g: 0, b: 0, a: 1.0 };
        canvas
    }

    fn pixel(canvas: &Canvas, x: u32, y: u32) -> [u8; 4] {
        let i = ((y * canvas.texture.width + x) * 4) as usize;
        canvas.texture.data[i..i + 4].try_into().unwrap()
    }

    #[test]
    fn a_fixed_element_stays_put_while_the_content_scrolls() {
        let mut canvas = canvas_with_sidebar();
        canvas.scroll_step = 10.0;
        canvas.draw();
        let bar_rect = canvas.root.children[3].rect;
        assert_eq!(bar_rect, Rect { x: 80, y: 0, w: 20, h: 60 }, "against the canvas, not the 120px of content");
        assert_eq!(canvas.content_height, 120, "and it adds nothing to the content");
        assert_eq!(pixel(&canvas, 90, 30), [200, 0, 0, 255]);

        mouse(&mut canvas, 5.0, 5.0, 0, -3.0);
        canvas.draw();
        assert_eq!(canvas.scroll_y, 30.0);
        assert_eq!(canvas.root.children[3].rect, bar_rect, "it didn't scroll");
        assert_eq!(box_y(&canvas, 0), -30, "while the rest did");
        assert_eq!(pixel(&canvas, 90, 30), [200, 0, 0, 255], "and it is still drawn there");
    }

    #[test]
    fn a_fixed_element_is_drawn_over_the_content() {
        let mut canvas = canvas_with_sidebar(); // The three black boxes are 100 wide, so they run under the bar.
        canvas.draw();
        assert_eq!(pixel(&canvas, 50, 30), [0, 0, 0, 255], "content");
        assert_eq!(pixel(&canvas, 90, 30), [200, 0, 0, 255], "the bar, over the content, though it is last in the tree");
    }

    #[test]
    fn only_a_layout_with_a_fixed_element_depends_on_the_height() {
        let mut plain = canvas(60);
        plain.draw();
        plain.height = 80;
        plain.draw();
        assert_eq!(plain.layouts, 1, "nothing is placed against the canvas, so a new height needs no new layout");

        let mut with_bar = canvas_with_sidebar();
        with_bar.draw();
        with_bar.height = 80;
        with_bar.draw();
        assert_eq!(with_bar.layouts, 2);
        assert_eq!(with_bar.root.children[3].rect.h, 80, "the bar follows the canvas");
    }

    #[test]
    fn a_placed_element_can_be_clicked_even_outside_its_parents_box() {
        let mut canvas = Canvas::new();
        canvas.width = 200;
        canvas.height = 200;
        let parent = canvas.root.add_child(Element::stack(Orientation::Vertical));
        parent.style.size.x = Some(50);
        parent.style.size.y = Some(50);
        let child = parent.add_child(Element::stack(Orientation::Vertical));
        child.style.position = Position::Absolute;
        child.style.size.x = Some(40);
        child.style.size.y = Some(40);
        child.style.inset.left = Some(100); // Well to the right of the 50px parent.
        child.style.inset.top = Some(0);

        mouse(&mut canvas, 120.0, 20.0, BUTTON_LEFT, 0.0);
        canvas.draw();
        mouse(&mut canvas, 120.0, 20.0, 0, 0.0);
        canvas.draw();
        assert!(canvas.root.children[0].children[0].interaction.clicked);
        assert!(canvas.root.children[0].interaction.clicked, "and it bubbles to the parent, which is what holds it");
    }

    #[test]
    fn a_scrolled_cache_draws_exactly_what_a_fresh_layout_would() {
        let build = || {
            let mut canvas = Canvas::new();
            canvas.width = 120;
            canvas.height = 70;
            canvas.background = Color::WHITE;
            for i in 0..8 {
                let row = canvas.root.add_child(Element::stack(Orientation::Vertical));
                row.style.padding.base = 4;
                row.style.margin.base = 2;
                row.style.background = Color { r: 40, g: 90, b: 200, a: 0.8 };
                row.style.corner_radius = 6;
                row.add_child(Element::text(format!("Row number {i}")));
            }
            canvas.scroll_step = 17.0;
            canvas
        };

        let mut scrolled = build();
        scrolled.draw();
        for _ in 0..3 {
            mouse(&mut scrolled, 5.0, 5.0, 0, -1.0);
            scrolled.draw();
        }

        let mut fresh = build();
        fresh.scroll_y = scrolled.scroll_y;
        fresh.draw();

        assert_eq!(scrolled.scroll_y, 51.0);
        assert!(scrolled.texture.data == fresh.texture.data, "pixels differ between cached scrolling and a fresh layout");
        assert_eq!(scrolled.layouts, 1, "the scrolled one never laid out again");
    }

    #[test]
    fn keyboard_events_and_text_last_one_frame() {
        let mut canvas = canvas(200);
        canvas.input.set_keyboard(vec![KeyEvent { key: Key::Enter, action: KeyAction::Press }], "x".into(), 0);
        assert!(canvas.input.keyboard.pressed(Key::Enter));
        canvas.draw();
        assert!(canvas.input.keyboard.events.is_empty() && canvas.input.keyboard.text.is_empty());
    }

    #[test]
    fn text_can_be_replaced_and_other_kinds_refuse() {
        let mut canvas = canvas(200);
        let text = canvas.root.add_child(Element::text("before"));
        assert!(text.set_text("after"));
        assert!(!canvas.root.children[0].set_text("nope"), "a stack has no text");
    }

    #[test]
    fn the_canvas_foreground_is_what_text_inherits() {
        let mut canvas = canvas(200);
        canvas.text.foreground = Color { r: 9, g: 8, b: 7, a: 1.0 };
        canvas.root.add_child(Element::text("hi"));
        canvas.draw();
        assert!(canvas.cmds.iter().any(|c| matches!(c, DrawCmd::Text { color, .. } if color.r == 9)));
    }
}
