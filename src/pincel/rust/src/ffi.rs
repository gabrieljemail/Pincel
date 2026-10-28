//! C ABI consumed by `src/pincel/cpp/include/Pincel/ffi.h`. Keep both in sync.
//!
//! Ownership: a `PincelCanvas` is created and freed by the caller, and freeing it frees every element in it.
//!
//! Elements are named by an id (a `u64`, 0 meaning "none"), not a pointer. Ids are never reused, so an id for an
//! element that has since been removed, or whose canvas was freed, just finds nothing: setters do nothing, getters
//! return a default, and functions that return an id return 0. That is what makes removing elements safe when other
//! code still holds a handle to one.
//!
//! A canvas, and the elements in it, must not be used from two threads at the same time. Taking turns is fine.

use std::cell::RefCell;
use std::ffi::{c_char, CStr, CString};
use std::sync::atomic::{AtomicBool, Ordering};

use crate::anim::Transition;
use crate::canvas::Canvas;
use crate::dirty::set;
use crate::element::{Element, ElementKind, Image, Orientation, TextSpan};
use crate::font;
use crate::input::{Key, KeyAction, KeyEvent};
use crate::registry;
use crate::style::{Align, Color, Easing, FontFamily, FontStyle, ImageFilter, Overflow, PointerEvents, Position, Spacing, TextShadow};
use crate::text_input::InputKind;
use crate::tree;

static INITIALIZED: AtomicBool = AtomicBool::new(false);

/// The version of the C interface and the C++ headers on top of it. It has to be the same as `PINCEL_ABI_VERSION` in
/// `ffi.h` (a test checks), and goes up whenever either changes in a way that code built against the old one can't use.
pub const ABI_VERSION: u32 = 2;

#[no_mangle]
pub extern "C" fn pincel_abi_version() -> u32 {
    ABI_VERSION
}

/// Like `pincel_init`, after checking that the headers the caller was built with are for this version of the library.
#[no_mangle]
pub extern "C" fn pincel_init_checked(header_version: u32) -> bool {
    if header_version != ABI_VERSION {
        set_last_error(&format!(
            "the Pincel library is interface version {ABI_VERSION}, but this program was built with headers for version {header_version}. Rebuild one to match the other."
        ));
        return false;
    }
    pincel_init();
    true
}

/// Initializes global state. Safe to call more than once.
#[no_mangle]
pub extern "C" fn pincel_init() {
    INITIALIZED.store(true, Ordering::SeqCst);
}

#[no_mangle]
pub extern "C" fn pincel_is_initialized() -> bool {
    INITIALIZED.load(Ordering::SeqCst)
}

/// Returns a static, NUL-terminated version string.
#[no_mangle]
pub extern "C" fn pincel_version() -> *const c_char {
    crate::VERSION.as_ptr().cast()
}

// ---- Handles and errors ----------------------------------------------------------------------

/// The element an id names, if it still exists.
///
/// # Safety
/// The caller must have the canvas to itself, as the module docs say.
unsafe fn element<'a>(id: u64) -> Option<&'a mut Element> {
    let uid = u32::try_from(id).ok()?;
    registry::lookup(uid).map(|ptr| &mut *ptr)
}

unsafe fn element_ptr(id: u64) -> Option<*mut Element> {
    element(id).map(|el| el as *mut Element)
}

fn id_of(el: &Element) -> u64 {
    u64::from(el.uid)
}

thread_local! {
    static LAST_ERROR: RefCell<CString> = RefCell::new(CString::default());
}

fn set_last_error(message: &str) {
    // Interior NULs can't be in a C string, so they are dropped.
    let message = CString::new(message.replace('\0', "")).unwrap_or_default();
    LAST_ERROR.with(|e| *e.borrow_mut() = message);
}

/// The message from the last call on this thread that failed, as a NUL-terminated string.
/// Valid until the next call into Pincel on this thread that fails.
#[no_mangle]
pub extern "C" fn pincel_last_error() -> *const c_char {
    LAST_ERROR.with(|e| e.borrow().as_ptr())
}

unsafe fn string(text: *const c_char) -> String {
    if text.is_null() {
        return String::new();
    }
    CStr::from_ptr(text).to_string_lossy().into_owned()
}

// ---- Canvas ----------------------------------------------------------------------------------

#[no_mangle]
pub extern "C" fn pincel_canvas_new() -> *mut Canvas {
    Box::into_raw(Box::new(Canvas::new()))
}

/// Frees the canvas and every element in it, so every id from it becomes invalid.
///
/// # Safety
/// `canvas` must come from `pincel_canvas_new` and not be used afterwards.
#[no_mangle]
pub unsafe extern "C" fn pincel_canvas_free(canvas: *mut Canvas) {
    if !canvas.is_null() {
        drop(Box::from_raw(canvas));
    }
}

/// The root element of the canvas, a vertical stack. It can't be removed.
#[no_mangle]
pub unsafe extern "C" fn pincel_canvas_root(canvas: *mut Canvas) -> u64 {
    id_of(&(*canvas).root)
}

/// The first element with this id (see `pincel_element_set_id`), going down the tree in order. 0 if there is none.
#[no_mangle]
pub unsafe extern "C" fn pincel_canvas_find(canvas: *const Canvas, id: *const c_char) -> u64 {
    (*canvas).root.find(&string(id)).map_or(0, id_of)
}

#[no_mangle]
pub unsafe extern "C" fn pincel_canvas_set_background(canvas: *mut Canvas, color: Color) {
    set(&mut (*canvas).background, color);
}

/// Sets the size in pixels of the texture `pincel_canvas_draw` produces.
#[no_mangle]
pub unsafe extern "C" fn pincel_canvas_set_size(canvas: *mut Canvas, width: u32, height: u32) {
    (*canvas).width = width;
    (*canvas).height = height;
}

/// The text color that elements inherit when nothing between them and the canvas sets one.
#[no_mangle]
pub unsafe extern "C" fn pincel_canvas_set_foreground(canvas: *mut Canvas, color: Color) {
    set(&mut (*canvas).text.foreground, color);
}

/// The text shadow that text inherits when nothing between it and the canvas sets one. A fully transparent color
/// (the default) means no shadow, and then none is drawn.
#[no_mangle]
pub unsafe extern "C" fn pincel_canvas_set_text_shadow(canvas: *mut Canvas, color: Color, offset_x: i32, offset_y: i32) {
    set(&mut (*canvas).text.text_shadow, TextShadow { color, offset_x, offset_y });
}

/// Sets the canvas's clock, in seconds, which transitions run on. It has to only go forward. Without it the canvas
/// uses a clock of its own.
#[no_mangle]
pub unsafe extern "C" fn pincel_canvas_set_time(canvas: *mut Canvas, seconds: f64) {
    (*canvas).set_time(seconds);
}

/// Whether a transition was still running as of the last draw. Keep drawing while it is.
#[no_mangle]
pub unsafe extern "C" fn pincel_canvas_is_animating(canvas: *const Canvas) -> bool {
    (*canvas).is_animating()
}

// ---- Input and scrolling ---------------------------------------------------------------------

#[repr(C)]
pub struct PincelMouseInput {
    /// Position in canvas pixels from the top-left. NaN when the cursor isn't over the canvas.
    pub x: f32,
    pub y: f32,
    /// Buttons that are down: 1 left, 2 right, 4 middle.
    pub buttons: u8,
    /// Wheel movement since the last frame, in notches. Positive is up and right.
    pub scroll_x: f32,
    pub scroll_y: f32,
}

#[repr(C)]
pub struct PincelKeyEvent {
    /// A `PincelKey`.
    pub key: u8,
    /// 0 press, 1 repeat, 2 release.
    pub action: u8,
}

#[repr(C)]
pub struct PincelKeyboardInput {
    pub events: *const PincelKeyEvent,
    pub event_count: usize,
    /// Unicode code points typed since the last frame.
    pub text: *const u32,
    pub text_count: usize,
    /// Modifiers held: 1 shift, 2 control, 4 alt, 8 super.
    pub modifiers: u8,
}

/// Gives the canvas this frame's input. Call once per frame, before `pincel_canvas_draw`.
/// Either pointer may be null to say nothing changed for that device. The arrays are copied.
#[no_mangle]
pub unsafe extern "C" fn pincel_canvas_set_input(canvas: *mut Canvas, mouse: *const PincelMouseInput, keyboard: *const PincelKeyboardInput) {
    let input = &mut (*canvas).input;
    if let Some(m) = mouse.as_ref() {
        input.set_mouse(m.x, m.y, m.buttons, m.scroll_x, m.scroll_y);
    }
    if let Some(k) = keyboard.as_ref() {
        let events = if k.events.is_null() { &[][..] } else { std::slice::from_raw_parts(k.events, k.event_count) };
        let events = events
            .iter()
            .filter_map(|e| Some(KeyEvent { key: Key::from_u8(e.key), action: KeyAction::from_u8(e.action)? }))
            .collect();
        let codes = if k.text.is_null() { &[][..] } else { std::slice::from_raw_parts(k.text, k.text_count) };
        // Code points that aren't valid characters (surrogates, out of range) are dropped.
        let text = codes.iter().filter_map(|&c| char::from_u32(c)).collect();
        input.set_keyboard(events, text, k.modifiers);
    }
}

/// Whether an input has keyboard focus. While it does, the keyboard is Pincel's: don't also use the keys for the
/// host's own shortcuts.
#[no_mangle]
pub unsafe extern "C" fn pincel_canvas_wants_keyboard(canvas: *const Canvas) -> bool {
    (*canvas).wants_keyboard()
}

/// How far the page is scrolled up, in pixels.
#[no_mangle]
pub unsafe extern "C" fn pincel_canvas_get_scroll(canvas: *const Canvas) -> f32 {
    (*canvas).scroll_y
}

/// Scrolls the page to `y` pixels, which takes effect (and is clamped to the content) at the next draw.
#[no_mangle]
pub unsafe extern "C" fn pincel_canvas_set_scroll(canvas: *mut Canvas, y: f32) {
    (*canvas).scroll_y = y;
}

/// Pixels scrolled per notch of the mouse wheel, for the page and for scrolling elements.
#[no_mangle]
pub unsafe extern "C" fn pincel_canvas_set_scroll_step(canvas: *mut Canvas, pixels: f32) {
    (*canvas).scroll_step = pixels;
}

/// The height of all the content as of the last draw, which can be more than the canvas's height.
#[no_mangle]
pub unsafe extern "C" fn pincel_canvas_content_height(canvas: *const Canvas) -> i32 {
    (*canvas).content_height
}

/// A view of RGBA8 pixels (premultiplied alpha, tightly packed rows, row 0 at the top).
/// `data` is owned by the canvas and valid until the next draw or until the canvas is freed.
/// `version` goes up whenever the pixels are redrawn, so a host that keeps the previous version's texture on the
/// GPU can skip uploading when it hasn't changed.
#[repr(C)]
pub struct PincelTexture {
    pub width: u32,
    pub height: u32,
    pub data: *const u8,
    pub version: u64,
}

#[no_mangle]
pub unsafe extern "C" fn pincel_canvas_draw(canvas: *mut Canvas) -> PincelTexture {
    let canvas = &mut *canvas;
    let texture = canvas.draw();
    PincelTexture {
        width: texture.width,
        height: texture.height,
        data: texture.data.as_ptr(),
        version: canvas.version,
    }
}

// ---- Adding elements -------------------------------------------------------------------------

unsafe fn add(parent: u64, child: Element) -> u64 {
    element(parent).map_or(0, |parent| id_of(parent.add_child(child)))
}

/// `vertical` is true for a vertical stack. Returns the new child, or 0 if `parent` doesn't exist.
#[no_mangle]
pub unsafe extern "C" fn pincel_element_add_stack(parent: u64, vertical: bool) -> u64 {
    add(parent, Element::stack(if vertical { Orientation::Vertical } else { Orientation::Horizontal }))
}

/// Adds a Text child. `text` is a NUL-terminated UTF-8 string, which is copied.
#[no_mangle]
pub unsafe extern "C" fn pincel_element_add_text(parent: u64, text: *const c_char) -> u64 {
    add(parent, Element::text(string(text)))
}

/// Adds an Input child. `kind` is 0 for one line of text, 1 for several lines, and 2 for a number.
#[no_mangle]
pub unsafe extern "C" fn pincel_element_add_input(parent: u64, kind: u8) -> u64 {
    let kind = match kind {
        1 => InputKind::Multiline,
        2 => InputKind::Number,
        _ => InputKind::Text,
    };
    add(parent, Element::input(kind))
}

/// Adds an Image child from RGBA8 pixels (straight alpha, tightly packed rows, row 0 at the top), which are copied.
/// `rgba` must point to `width * height * 4` bytes. Returns 0, and sets the last error, if the size is 0 or too big.
#[no_mangle]
pub unsafe extern "C" fn pincel_element_add_image(parent: u64, rgba: *const u8, width: u32, height: u32) -> u64 {
    let Some(len) = (width as usize).checked_mul(height as usize).and_then(|n| n.checked_mul(4)) else {
        set_last_error("image dimensions are too large");
        return 0;
    };
    if rgba.is_null() || width == 0 || height == 0 {
        set_last_error("image needs pixels and a non-zero width and height");
        return 0;
    }
    match Image::from_rgba(width, height, std::slice::from_raw_parts(rgba, len).to_vec()) {
        Some(image) => add(parent, Element::image(image)),
        None => {
            set_last_error("image buffer doesn't match its dimensions");
            0
        }
    }
}

/// Adds an Image child decoded from a PNG or JPEG file. `path` is a NUL-terminated UTF-8 string.
/// Returns 0, and sets the last error, if the file can't be read or decoded.
#[no_mangle]
pub unsafe extern "C" fn pincel_element_add_image_file(parent: u64, path: *const c_char) -> u64 {
    match Image::from_file(&string(path)) {
        Ok(image) => add(parent, Element::image(image)),
        Err(message) => {
            set_last_error(&message);
            0
        }
    }
}

// ---- Editing the tree ------------------------------------------------------------------------

/// Removes the element, and everything inside it, from its canvas and frees them. Their ids become invalid.
/// Returns false, doing nothing, if it doesn't exist or is a canvas's root.
#[no_mangle]
pub unsafe extern "C" fn pincel_element_remove(el: u64) -> bool {
    element_ptr(el).is_some_and(|ptr| tree::remove(ptr))
}

/// Removes everything inside the element, freeing it.
#[no_mangle]
pub unsafe extern "C" fn pincel_element_clear_children(el: u64) {
    if let Some(el) = element(el) {
        el.clear_children();
    }
}

/// Moves the element, with what is inside it, to be child number `index` of `new_parent` (the last, for an index
/// past the end). The same parent just reorders it. Its id stays the same. Returns false, changing nothing, if either
/// doesn't exist, it is a canvas's root, or `new_parent` is the element or is inside it.
#[no_mangle]
pub unsafe extern "C" fn pincel_element_move_to(el: u64, new_parent: u64, index: u32) -> bool {
    match (element_ptr(el), element_ptr(new_parent)) {
        (Some(el), Some(parent)) => tree::move_to(el, parent, index as usize),
        _ => false,
    }
}

/// Which child of its parent the element is, from 0. -1 if it doesn't exist or is a canvas's root.
#[no_mangle]
pub unsafe extern "C" fn pincel_element_index(el: u64) -> i32 {
    element_ptr(el).and_then(|ptr| tree::index_of(ptr)).map_or(-1, |i| i as i32)
}

#[no_mangle]
pub unsafe extern "C" fn pincel_element_child_count(el: u64) -> u32 {
    element(el).map_or(0, |el| el.children.len() as u32)
}

/// Child number `index`, from 0. 0 if there is none.
#[no_mangle]
pub unsafe extern "C" fn pincel_element_child_at(el: u64, index: u32) -> u64 {
    element(el).and_then(|el| el.children.get(index as usize)).map_or(0, |child| id_of(child))
}

/// The element this one is inside. 0 for a canvas's root or an element that doesn't exist.
#[no_mangle]
pub unsafe extern "C" fn pincel_element_parent(el: u64) -> u64 {
    element(el).filter(|el| !el.parent.is_null()).map_or(0, |el| id_of(&*el.parent))
}

/// Gives the element a name, for `pincel_canvas_find`. Names don't have to be unique.
#[no_mangle]
pub unsafe extern "C" fn pincel_element_set_id(el: u64, name: *const c_char) {
    if let Some(el) = element(el) {
        el.id = string(name);
    }
}

/// Whether the id names an element that exists.
#[no_mangle]
pub unsafe extern "C" fn pincel_element_exists(el: u64) -> bool {
    element(el).is_some()
}

/// Hides or shows the element. A hidden element is as if it weren't in the tree (no room, no drawing, no input),
/// but it keeps its place and state.
#[no_mangle]
pub unsafe extern "C" fn pincel_element_set_visible(el: u64, visible: bool) {
    if let Some(el) = element(el) {
        set(&mut el.style.visible, visible);
    }
}

// ---- Box model and colors --------------------------------------------------------------------

/// The border-box size in pixels. 0 for a dimension means "auto".
#[no_mangle]
pub unsafe extern "C" fn pincel_element_set_size(el: u64, width: u16, height: u16) {
    if let Some(el) = element(el) {
        set(&mut el.style.size.x, Some(width));
        set(&mut el.style.size.y, Some(height));
    }
}

/// The same margin on every side. Replaces any set by `pincel_element_set_margin_sides`.
#[no_mangle]
pub unsafe extern "C" fn pincel_element_set_margin(el: u64, base: u16) {
    if let Some(el) = element(el) {
        set(&mut el.style.margin, Spacing { base, ..Spacing::default() });
    }
}

/// A margin for each side, in CSS's order.
#[no_mangle]
pub unsafe extern "C" fn pincel_element_set_margin_sides(el: u64, top: u16, right: u16, bottom: u16, left: u16) {
    if let Some(el) = element(el) {
        set(&mut el.style.margin, Spacing { sides: Some([top, right, bottom, left]), ..Spacing::default() });
    }
}

/// The same padding on every side. Replaces any set by `pincel_element_set_padding_sides`.
#[no_mangle]
pub unsafe extern "C" fn pincel_element_set_padding(el: u64, base: u16) {
    if let Some(el) = element(el) {
        set(&mut el.style.padding, Spacing { base, ..Spacing::default() });
    }
}

/// A padding for each side, in CSS's order.
#[no_mangle]
pub unsafe extern "C" fn pincel_element_set_padding_sides(el: u64, top: u16, right: u16, bottom: u16, left: u16) {
    if let Some(el) = element(el) {
        set(&mut el.style.padding, Spacing { sides: Some([top, right, bottom, left]), ..Spacing::default() });
    }
}

/// For a stack: where its children go in the room left over, on x then on y (0 start, 1 center, 2 end).
#[no_mangle]
pub unsafe extern "C" fn pincel_element_set_align(el: u64, x: u8, y: u8) {
    if let Some(el) = element(el) {
        let align = |value: u8| match value {
            1 => Align::Center,
            2 => Align::End,
            _ => Align::Start,
        };
        set(&mut el.style.align_x, align(x));
        set(&mut el.style.align_y, align(y));
    }
}

/// For a stack: the space in pixels between its children.
#[no_mangle]
pub unsafe extern "C" fn pincel_element_set_gap(el: u64, gap: u16) {
    if let Some(el) = element(el) {
        set(&mut el.style.gap, gap);
    }
}

#[no_mangle]
pub unsafe extern "C" fn pincel_element_set_border(el: u64, size: u16, color: Color) {
    if let Some(el) = element(el) {
        set(&mut el.style.border_size.base, size);
        set(&mut el.style.border_color, color);
    }
}

#[no_mangle]
pub unsafe extern "C" fn pincel_element_set_background(el: u64, color: Color) {
    if let Some(el) = element(el) {
        set(&mut el.style.background, color);
    }
}

#[no_mangle]
pub unsafe extern "C" fn pincel_element_set_corner_radius(el: u64, radius: u16) {
    if let Some(el) = element(el) {
        set(&mut el.style.corner_radius, radius);
    }
}

/// How the colors (background, border color, and the text color when set) change when they are set to something
/// else: a fade of `seconds`, or at once for 0. `easing` is 0 linear, 1 ease, 2 ease-in, 3 ease-out, 4 ease-in-out.
#[no_mangle]
pub unsafe extern "C" fn pincel_element_set_transition(el: u64, seconds: f32, easing: u8) {
    if let Some(el) = element(el) {
        let easing = match easing {
            0 => Easing::Linear,
            2 => Easing::EaseIn,
            3 => Easing::EaseOut,
            4 => Easing::EaseInOut,
            _ => Easing::Ease,
        };
        set(&mut el.style.transition, Transition { duration: seconds.max(0.0), easing });
    }
}

// ---- Text ------------------------------------------------------------------------------------

/// Text properties are inherited: text uses the closest value set on it or an element it is inside.
#[no_mangle]
pub unsafe extern "C" fn pincel_element_set_foreground(el: u64, color: Color) {
    if let Some(el) = element(el) {
        set(&mut el.style.foreground, Some(color));
    }
}

/// Font size in pixels.
#[no_mangle]
pub unsafe extern "C" fn pincel_element_set_font_size(el: u64, size: u16) {
    if let Some(el) = element(el) {
        set(&mut el.style.font_size, Some(size));
    }
}

/// A font family id: 0 and 1 are the built-in sans-serif and monospace, and `pincel_font_register` gives more.
#[no_mangle]
pub unsafe extern "C" fn pincel_element_set_font_family(el: u64, family: u16) {
    if let Some(el) = element(el) {
        set(&mut el.style.font_family, Some(FontFamily(family)));
    }
}

/// 100 to 900. 600 and up is drawn bold, and lower is regular.
#[no_mangle]
pub unsafe extern "C" fn pincel_element_set_font_weight(el: u64, weight: u16) {
    if let Some(el) = element(el) {
        set(&mut el.style.font_weight, Some(weight));
    }
}

/// 0 is normal and 1 is italic.
#[no_mangle]
pub unsafe extern "C" fn pincel_element_set_font_style(el: u64, style: u8) {
    if let Some(el) = element(el) {
        set(&mut el.style.font_style, Some(if style == FontStyle::Italic as u8 { FontStyle::Italic } else { FontStyle::Normal }));
    }
}

/// A shadow behind text: a copy in `color`, `offset_x` and `offset_y` pixels away. A fully transparent `color` means
/// no shadow, including for text inside this element that would otherwise get one from further up.
#[no_mangle]
pub unsafe extern "C" fn pincel_element_set_text_shadow(el: u64, color: Color, offset_x: i32, offset_y: i32) {
    if let Some(el) = element(el) {
        set(&mut el.style.text_shadow, Some(TextShadow { color, offset_x, offset_y }));
    }
}

/// Goes back to taking the text shadow from the element's parent, which is the default.
#[no_mangle]
pub unsafe extern "C" fn pincel_element_inherit_text_shadow(el: u64) {
    if let Some(el) = element(el) {
        set(&mut el.style.text_shadow, None);
    }
}

/// Replaces the text of a Text element, or the value of an Input. `text` is NUL-terminated UTF-8 and is copied.
/// Returns false, changing nothing, if the element isn't one of those.
#[no_mangle]
pub unsafe extern "C" fn pincel_element_set_text(el: u64, text: *const c_char) -> bool {
    element(el).is_some_and(|el| el.set_text(string(text)))
}

/// Copies `text` into `buffer` (at most `capacity` bytes, including a terminating NUL, which is always written when
/// `capacity` is more than 0), and returns how many bytes the whole text is, not counting the NUL.
unsafe fn copy_out(text: &str, buffer: *mut c_char, capacity: usize) -> usize {
    if !buffer.is_null() && capacity > 0 {
        // Cut at a character boundary, so the copy is still valid UTF-8.
        let mut n = text.len().min(capacity - 1);
        while !text.is_char_boundary(n) {
            n -= 1;
        }
        std::ptr::copy_nonoverlapping(text.as_ptr(), buffer.cast::<u8>(), n);
        *buffer.add(n) = 0;
    }
    text.len()
}

/// Copies the text of a Text element, the value of an Input, or the spans of a rich text element put together into
/// `buffer` (at most `capacity` bytes, including a terminating NUL, which is always written when `capacity` is more
/// than 0), and returns how many bytes the whole text is, not counting the NUL. So a result of `capacity` or more means
/// it was cut short. `buffer` may be NULL to just ask the length. Returns 0 for an element with no text.
#[no_mangle]
pub unsafe extern "C" fn pincel_element_get_text(el: u64, buffer: *mut c_char, capacity: usize) -> usize {
    let Some(el) = element(el) else { return 0 };
    let Some(text) = el.text_value() else { return 0 };
    copy_out(&text, buffer, capacity)
}

// ---- Fonts -----------------------------------------------------------------------------------

/// Font files in memory, for `pincel_font_register`. Only the regular face is needed: leave the others NULL, and a style
/// the family has no face for is drawn with the closest one it has.
#[repr(C)]
pub struct PincelFontFaces {
    pub regular: *const u8,
    pub regular_len: usize,
    pub italic: *const u8,
    pub italic_len: usize,
    pub bold: *const u8,
    pub bold_len: usize,
    pub bold_italic: *const u8,
    pub bold_italic_len: usize,
}

fn family_result(result: Result<FontFamily, String>) -> i32 {
    match result {
        Ok(family) => i32::from(family.0),
        Err(message) => {
            set_last_error(&message);
            -1
        }
    }
}

/// Adds a font family and returns its id, for `pincel_element_set_font_family`. The font data is copied. The name must
/// be new (ignoring case). Returns -1, and sets the last error, if a file isn't a usable font or the name is taken.
/// The built-in families are named "Sans" (0) and "Mono" (1).
#[no_mangle]
pub unsafe extern "C" fn pincel_font_register(name: *const c_char, faces: *const PincelFontFaces) -> i32 {
    let Some(faces) = faces.as_ref() else {
        set_last_error("no font faces given");
        return -1;
    };
    let face = |data: *const u8, len: usize| (!data.is_null()).then(|| std::slice::from_raw_parts(data, len).to_vec());
    family_result(font::register_family(
        &string(name),
        [
            face(faces.regular, faces.regular_len),
            face(faces.italic, faces.italic_len),
            face(faces.bold, faces.bold_len),
            face(faces.bold_italic, faces.bold_italic_len),
        ],
    ))
}

/// Like `pincel_font_register`, reading the faces from files (NUL-terminated UTF-8 paths, NULL for a face the family
/// doesn't have). TrueType and OpenType files work.
#[no_mangle]
pub unsafe extern "C" fn pincel_font_register_files(
    name: *const c_char,
    regular: *const c_char,
    italic: *const c_char,
    bold: *const c_char,
    bold_italic: *const c_char,
) -> i32 {
    let paths = [regular, italic, bold, bold_italic].map(|p| (!p.is_null()).then(|| string(p)));
    family_result(font::register_family_files(&string(name), paths.each_ref().map(|p| p.as_deref())))
}

/// The id of the family with this name (ignoring case), or -1.
#[no_mangle]
pub unsafe extern "C" fn pincel_font_find(name: *const c_char) -> i32 {
    font::find_family(&string(name)).map_or(-1, |family| i32::from(family.0))
}

/// Says to draw characters `family` has no glyph for from `fallback` instead (symbols and emoji a pixel font lacks,
/// say). Pass -1 for `fallback` to remove it. Returns false if either family doesn't exist.
#[no_mangle]
pub extern "C" fn pincel_font_set_fallback(family: u16, fallback: i32) -> bool {
    let fallback = if fallback < 0 {
        None
    } else {
        match u16::try_from(fallback) {
            Ok(id) => Some(FontFamily(id)),
            Err(_) => return false,
        }
    };
    font::set_fallback(FontFamily(family), fallback)
}

// ---- Images ----------------------------------------------------------------------------------

/// Replaces the pixels of an Image element, such as with a new frame. `rgba` is `width * height * 4` bytes, as for
/// `pincel_element_add_image`, and is copied. Returns false, changing nothing, if the element isn't an Image or the
/// size is 0.
#[no_mangle]
pub unsafe extern "C" fn pincel_element_set_image(el: u64, rgba: *const u8, width: u32, height: u32) -> bool {
    let Some(len) = (width as usize).checked_mul(height as usize).and_then(|n| n.checked_mul(4)) else {
        return false;
    };
    if rgba.is_null() {
        return false;
    }
    let Some(image) = Image::from_rgba(width, height, std::slice::from_raw_parts(rgba, len).to_vec()) else {
        return false;
    };
    element(el).is_some_and(|el| el.set_image(image))
}

/// 0 is smooth (bilinear, averaging when shrunk), and 1 is nearest (crisp pixels, for pixel art and plain copies).
#[no_mangle]
pub unsafe extern "C" fn pincel_element_set_image_filter(el: u64, filter: u8) {
    if let Some(el) = element(el) {
        set(&mut el.style.image_filter, if filter == 1 { ImageFilter::Nearest } else { ImageFilter::Smooth });
    }
}

// ---- Positioning, overflow and scrolling -----------------------------------------------------

/// 0 is in the parent's flow, 1 is absolute (against the parent's padding box, scrolls), and 2 is fixed
/// (against the whole canvas, doesn't scroll). Anything else is in the flow.
#[no_mangle]
pub unsafe extern "C" fn pincel_element_set_position(el: u64, position: u8) {
    if let Some(el) = element(el) {
        let position = match position {
            1 => Position::Absolute,
            2 => Position::Fixed,
            _ => Position::Flow,
        };
        set(&mut el.style.position, position);
    }
}

/// Sets, or with `has_value` false clears, how far in from one side an out-of-flow element is placed.
/// `side` is 0 left, 1 top, 2 right, 3 bottom.
#[no_mangle]
pub unsafe extern "C" fn pincel_element_set_inset(el: u64, side: u8, has_value: bool, value: i32) {
    if let Some(el) = element(el) {
        let inset = &mut el.style.inset;
        let slot = match side {
            0 => &mut inset.left,
            1 => &mut inset.top,
            2 => &mut inset.right,
            _ => &mut inset.bottom,
        };
        set(slot, has_value.then_some(value));
    }
}

/// Centers an out-of-flow element on an axis, whatever its insets say.
#[no_mangle]
pub unsafe extern "C" fn pincel_element_set_inset_center(el: u64, center_x: bool, center_y: bool) {
    if let Some(el) = element(el) {
        set(&mut el.style.inset.center_x, center_x);
        set(&mut el.style.inset.center_y, center_y);
    }
}

/// What happens to children that don't fit: 0 they are drawn outside the box, 1 they are cut off at it, and 2 they are
/// cut off and the user can scroll them into view with the mouse wheel. A scrolling element needs a size on the axis
/// it scrolls on.
#[no_mangle]
pub unsafe extern "C" fn pincel_element_set_overflow(el: u64, overflow: u8) {
    if let Some(el) = element(el) {
        let overflow = match overflow {
            1 => Overflow::Hidden,
            2 => Overflow::Scroll,
            _ => Overflow::Visible,
        };
        set(&mut el.style.overflow, overflow);
    }
}

/// For a scrolling element: when the content grows while it is at the end, it follows. For a chat log.
#[no_mangle]
pub unsafe extern "C" fn pincel_element_set_stick_to_end(el: u64, stick: bool) {
    if let Some(el) = element(el) {
        set(&mut el.style.stick_to_end, stick);
    }
}

#[repr(C)]
pub struct PincelScroll {
    /// How far the content is scrolled left and up, in pixels.
    pub x: i32,
    pub y: i32,
    /// The most it can be scrolled, as of the last draw.
    pub max_x: i32,
    pub max_y: i32,
}

#[no_mangle]
pub unsafe extern "C" fn pincel_element_get_scroll(el: u64) -> PincelScroll {
    let scroll = element(el).map(|el| el.scroll).unwrap_or_default();
    PincelScroll { x: scroll.x, y: scroll.y, max_x: scroll.max_x, max_y: scroll.max_y }
}

/// Scrolls a scrolling element to `(x, y)` pixels. Asking for more than the content allows is fine, and goes to the end.
#[no_mangle]
pub unsafe extern "C" fn pincel_element_set_scroll(el: u64, x: i32, y: i32) {
    if let Some(el) = element(el) {
        el.set_scroll(x, y);
    }
}

#[repr(C)]
pub struct PincelRect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

/// The element's border box as of the last draw, in canvas pixels with scrolling applied (so it matches the mouse).
#[no_mangle]
pub unsafe extern "C" fn pincel_element_get_rect(el: u64) -> PincelRect {
    let rect = element(el).map(|el| el.rect).unwrap_or(crate::draw_cmd::Rect { x: 0, y: 0, w: 0, h: 0 });
    PincelRect { x: rect.x, y: rect.y, w: rect.w, h: rect.h }
}

// ---- Interaction, input fields and focus -----------------------------------------------------

// What the mouse did to an element as of the last `pincel_canvas_draw`. Hover and press bubble up, so a
// container is hovered whenever something in it is.

#[no_mangle]
pub unsafe extern "C" fn pincel_element_is_hovered(el: u64) -> bool {
    element(el).is_some_and(|el| el.interaction.hovered)
}

/// The left button is down, and went down on this element or inside it.
#[no_mangle]
pub unsafe extern "C" fn pincel_element_is_pressed(el: u64) -> bool {
    element(el).is_some_and(|el| el.interaction.pressed)
}

/// The left button came up this frame over the element it went down on. True for one draw.
#[no_mangle]
pub unsafe extern "C" fn pincel_element_was_clicked(el: u64) -> bool {
    element(el).is_some_and(|el| el.interaction.clicked)
}

#[no_mangle]
pub unsafe extern "C" fn pincel_element_is_focused(el: u64) -> bool {
    element(el).is_some_and(|el| el.interaction.focused)
}

/// The grey text shown while an Input is empty. `text` is NUL-terminated UTF-8 and is copied.
/// Does nothing if the element isn't an Input.
#[no_mangle]
pub unsafe extern "C" fn pincel_element_set_placeholder(el: u64, text: *const c_char) {
    if let Some(Element { kind: ElementKind::Input(state), .. }) = element(el) {
        set(&mut state.placeholder, string(text));
    }
}

/// The most characters an Input takes, or 0 for no limit. Does nothing if the element isn't an Input.
#[no_mangle]
pub unsafe extern "C" fn pincel_element_set_max_length(el: u64, max_chars: u32) {
    if let Some(Element { kind: ElementKind::Input(state), .. }) = element(el) {
        set(&mut state.max_chars, (max_chars > 0).then_some(max_chars as usize));
    }
}

/// The user changed the Input's value during the last draw.
#[no_mangle]
pub unsafe extern "C" fn pincel_element_input_changed(el: u64) -> bool {
    matches!(element(el), Some(Element { kind: ElementKind::Input(state), .. }) if state.changed)
}

/// The user pressed Enter in a single-line Input during the last draw.
#[no_mangle]
pub unsafe extern "C" fn pincel_element_input_submitted(el: u64) -> bool {
    matches!(element(el), Some(Element { kind: ElementKind::Input(state), .. }) if state.submitted)
}

/// Gives the element keyboard focus at the next draw, as if it were clicked. Only inputs can have focus. The key
/// press that made you ask is ignored by it, if it is still in that frame's input.
#[no_mangle]
pub unsafe extern "C" fn pincel_element_focus(el: u64) {
    if let Some(el) = element(el) {
        el.request_focus();
    }
}

/// Takes the element's keyboard focus away at the next draw, if it has it.
#[no_mangle]
pub unsafe extern "C" fn pincel_element_blur(el: u64) {
    if let Some(el) = element(el) {
        el.request_blur();
    }
}

// ---- Pointer events and the mouse buttons ----------------------------------------------------

/// 0 the element is a target of the mouse (the default), 1 the mouse goes through it to whatever is under it, though
/// what is inside it can still be a target. Use 1 on containers that cover areas the host should still get clicks in.
#[no_mangle]
pub unsafe extern "C" fn pincel_element_set_pointer_events(el: u64, mode: u8) {
    if let Some(el) = element(el) {
        set(&mut el.style.pointer_events, if mode == 1 { PointerEvents::None } else { PointerEvents::Auto });
    }
}

/// The buttons that are down and went down on this element or inside it, as a mask: 1 left, 2 right, 4 middle.
#[no_mangle]
pub unsafe extern "C" fn pincel_element_pressed_buttons(el: u64) -> u8 {
    element(el).map_or(0, |el| el.interaction.pressed_buttons)
}

/// The buttons that came up this frame over the element they went down on, as a mask: 1 left, 2 right, 4 middle.
/// A button is in it for one draw.
#[no_mangle]
pub unsafe extern "C" fn pincel_element_clicked_buttons(el: u64) -> u8 {
    element(el).map_or(0, |el| el.interaction.clicked_buttons)
}

/// Whether the cursor is over something that takes the mouse, or a press that began on something is still held.
/// While it is, the mouse belongs to the canvas and the host shouldn't also act on it.
#[no_mangle]
pub unsafe extern "C" fn pincel_canvas_wants_mouse(canvas: *const Canvas) -> bool {
    (*canvas).wants_mouse()
}

/// Whether the last draw used the mouse wheel, as a mask: 1 horizontal, 2 vertical. A wheel the canvas didn't use is
/// the host's.
#[no_mangle]
pub unsafe extern "C" fn pincel_canvas_wheel_consumed(canvas: *const Canvas) -> u8 {
    let (horizontal, vertical) = (*canvas).wheel_consumed();
    u8::from(horizontal) | (u8::from(vertical) << 1)
}

// ---- Rich text -------------------------------------------------------------------------------

/// One run of rich text. What isn't set is taken from the element's text style.
#[repr(C)]
pub struct PincelTextSpan {
    /// NUL-terminated UTF-8.
    pub text: *const c_char,
    pub has_color: bool,
    pub color: Color,
    /// 100 to 900, or 0 to take the element's.
    pub font_weight: u16,
    /// -1 takes the element's, 0 is normal, 1 is italic.
    pub font_style: i8,
    pub underline: bool,
    pub strikethrough: bool,
}

unsafe fn spans(spans: *const PincelTextSpan, count: usize) -> Vec<TextSpan> {
    if spans.is_null() {
        return Vec::new();
    }
    std::slice::from_raw_parts(spans, count)
        .iter()
        .map(|s| TextSpan {
            text: string(s.text),
            foreground: s.has_color.then_some(s.color),
            font_weight: (s.font_weight != 0).then_some(s.font_weight),
            font_style: match s.font_style {
                0 => Some(FontStyle::Normal),
                1 => Some(FontStyle::Italic),
                _ => None,
            },
            underline: s.underline,
            strikethrough: s.strikethrough,
        })
        .collect()
}

/// Adds rich text: spans that wrap together as one piece of text, each with its own color, weight, italic, underline and
/// strikethrough. The text is copied.
#[no_mangle]
pub unsafe extern "C" fn pincel_element_add_rich_text(parent: u64, text_spans: *const PincelTextSpan, count: usize) -> u64 {
    add(parent, Element::rich(spans(text_spans, count)))
}

/// Replaces the spans of a rich text element. Returns false, changing nothing, if the element isn't one. The same
/// spans as before cost nothing.
#[no_mangle]
pub unsafe extern "C" fn pincel_element_set_spans(el: u64, text_spans: *const PincelTextSpan, count: usize) -> bool {
    let new_spans = spans(text_spans, count);
    element(el).is_some_and(|el| el.set_spans(new_spans))
}

// ---- Selection and the clipboard -------------------------------------------------------------

/// Selects from character `start` to `end` in an Input (counting characters, not bytes), with the caret at `end`.
/// They are clamped to the text. Does nothing for other elements.
#[no_mangle]
pub unsafe extern "C" fn pincel_element_select(el: u64, start: u32, end: u32) {
    if let Some(Element { kind: ElementKind::Input(state), .. }) = element(el) {
        let before = (state.anchor, state.caret);
        state.select(start as usize, end as usize);
        if (state.anchor, state.caret) != before {
            crate::dirty::invalidate();
        }
    }
}

/// The selection in an Input as character positions. Returns false, leaving the outputs alone, if nothing is selected.
#[no_mangle]
pub unsafe extern "C" fn pincel_element_get_selection(el: u64, start: *mut u32, end: *mut u32) -> bool {
    let Some(Element { kind: ElementKind::Input(state), .. }) = element(el) else { return false };
    let Some((a, b)) = state.selection() else { return false };
    if !start.is_null() {
        *start = a as u32;
    }
    if !end.is_null() {
        *end = b as u32;
    }
    true
}

/// Copies the selected text of an Input into `buffer`, the same way `pincel_element_get_text` does.
#[no_mangle]
pub unsafe extern "C" fn pincel_element_get_selected_text(el: u64, buffer: *mut c_char, capacity: usize) -> usize {
    let Some(Element { kind: ElementKind::Input(state), .. }) = element(el) else { return 0 };
    copy_out(&state.selected_text(), buffer, capacity)
}

/// The color behind selected text in an Input.
#[no_mangle]
pub unsafe extern "C" fn pincel_element_set_selection_color(el: u64, color: Color) {
    if let Some(el) = element(el) {
        set(&mut el.style.selection_color, color);
    }
}

/// One run of an Input's value drawn in its own color, as character positions (not bytes): `start` up to `end`.
#[repr(C)]
pub struct PincelHighlight {
    pub start: u32,
    pub end: u32,
    pub color: Color,
}

/// Replaces the colored runs of an Input's value. What they don't cover has the text color, and a later run wins where
/// they overlap. The same runs as before cost nothing. Does nothing if the element isn't an Input.
#[no_mangle]
pub unsafe extern "C" fn pincel_element_set_highlights(el: u64, items: *const PincelHighlight, count: usize) {
    let runs: Vec<crate::text_input::Highlight> = if items.is_null() {
        Vec::new()
    } else {
        std::slice::from_raw_parts(items, count)
            .iter()
            .map(|h| crate::text_input::Highlight { start: h.start as usize, end: h.end as usize, color: h.color })
            .collect()
    };
    if let Some(Element { kind: ElementKind::Input(state), .. }) = element(el) {
        set(&mut state.highlights, runs);
    }
}

/// Where character `index` of an Input is on the canvas as of the last draw: the top-left of the place the caret would be
/// there, in canvas pixels, and the height of a row. Returns false, leaving the outputs alone, if the element isn't an
/// Input or hasn't been drawn. An index past the end is the end, and one that is scrolled out of view is where it would be.
#[no_mangle]
pub unsafe extern "C" fn pincel_element_get_input_position(el: u64, index: u32, x: *mut i32, y: *mut i32, height: *mut i32) -> bool {
    let Some(Element { kind: ElementKind::Input(state), rect, .. }) = element(el) else { return false };
    let g = &state.geometry;
    if g.columns.is_empty() || g.line_height <= 0 {
        return false;
    }
    let index = (index as usize).min(state.len());
    let row = g.row_starts.iter().rposition(|&start| start <= index).unwrap_or(0).min(g.columns.len() - 1);
    let column = (index - g.row_starts[row]).min(g.columns[row].len() - 1);
    if !x.is_null() {
        *x = rect.x + g.content_dx + g.columns[row][column].round() as i32 - g.scroll_x;
    }
    if !y.is_null() {
        *y = rect.y + g.content_dy + (row as i32 - g.first_row as i32) * g.line_height;
    }
    if !height.is_null() {
        *height = g.line_height;
    }
    true
}

/// Makes an Input carry a line that is too long for it on in a row below, instead of scrolling sideways. It grows taller
/// to hold the rows. Those rows are indented by the width of `marker` (NUL-terminated UTF-8, or NULL for none), which is
/// drawn dimmed in the indent. Does nothing if the element isn't an Input.
#[no_mangle]
pub unsafe extern "C" fn pincel_element_set_wrap(el: u64, wrap: bool, marker: *const c_char) {
    let marker = if marker.is_null() { String::new() } else { string(marker) };
    if let Some(Element { kind: ElementKind::Input(state), .. }) = element(el) {
        set(&mut state.wrap, wrap);
        set(&mut state.wrap_marker, marker);
    }
}

/// Gives the canvas the host's clipboard, which inputs copy to (Ctrl+C and Ctrl+X) and paste from (Ctrl+V).
/// `get` returns the clipboard's text as NUL-terminated UTF-8 that stays valid until the next call (or NULL for none),
/// and `set` puts text on it. Either may be NULL. They are called during `pincel_canvas_draw`, on the calling thread.
/// Without callbacks, copy and paste still work between inputs in the canvas.
#[no_mangle]
pub unsafe extern "C" fn pincel_canvas_set_clipboard(
    canvas: *mut Canvas,
    get: Option<crate::clipboard::GetFn>,
    set: Option<crate::clipboard::SetFn>,
    user: *mut std::ffi::c_void,
) {
    let hooks = (get.is_some() || set.is_some()).then_some(crate::clipboard::Hooks { get, set, user });
    (*canvas).clipboard.set_hooks(hooks);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CString;

    fn c(text: &str) -> CString {
        CString::new(text).unwrap()
    }

    unsafe fn text(parent: u64, content: &str) -> u64 {
        pincel_element_add_text(parent, c(content).as_ptr())
    }

    const HEADER: &str = include_str!("../../cpp/include/Pincel/ffi.h");
    const SOURCE: &str = include_str!("ffi.rs");

    /// The names of the functions the header declares: every `pincel_*` that is followed by an opening parenthesis in a
    /// line of code (comments are skipped).
    fn declared_in_header() -> std::collections::BTreeSet<String> {
        let mut names = std::collections::BTreeSet::new();
        for line in HEADER.lines().map(str::trim).filter(|l| !l.starts_with("//") && !l.starts_with('#')) {
            let mut rest = line;
            while let Some(at) = rest.find("pincel_") {
                let tail = &rest[at..];
                let end = tail.find(|c: char| !(c.is_ascii_alphanumeric() || c == '_')).unwrap_or(tail.len());
                if tail[end..].trim_start().starts_with('(') {
                    names.insert(tail[..end].to_string());
                }
                rest = &tail[end..];
            }
        }
        names
    }

    /// The functions this file exports to C.
    fn exported_from_rust() -> std::collections::BTreeSet<String> {
        let mut names = std::collections::BTreeSet::new();
        for line in SOURCE.lines().map(str::trim) {
            for prefix in ["pub unsafe extern \"C\" fn ", "pub extern \"C\" fn "] {
                if let Some(rest) = line.strip_prefix(prefix) {
                    names.insert(rest[..rest.find('(').unwrap()].to_string());
                }
            }
        }
        names
    }

    #[test]
    fn the_header_and_the_rust_exports_declare_the_same_functions() {
        let (header, rust) = (declared_in_header(), exported_from_rust());
        assert!(header.len() > 60, "the parser found the declarations: {}", header.len());
        let only_header: Vec<_> = header.difference(&rust).collect();
        let only_rust: Vec<_> = rust.difference(&header).collect();
        assert!(only_header.is_empty(), "declared in ffi.h but not exported by Rust (the DLL would fail to link): {only_header:?}");
        assert!(only_rust.is_empty(), "exported by Rust but not declared in ffi.h (mods can't see them): {only_rust:?}");
    }

    #[test]
    fn the_headers_abi_version_is_the_librarys() {
        let defined = HEADER
            .lines()
            .find_map(|l| l.trim().strip_prefix("#define PINCEL_ABI_VERSION"))
            .expect("ffi.h defines PINCEL_ABI_VERSION");
        let version: u32 = defined.trim().trim_end_matches('u').parse().unwrap();
        assert_eq!(version, ABI_VERSION, "bump both together");
        assert_eq!(pincel_abi_version(), ABI_VERSION);
    }

    #[test]
    fn init_refuses_headers_from_a_different_version() {
        assert!(pincel_init_checked(ABI_VERSION));
        assert!(pincel_is_initialized());
        assert!(!pincel_init_checked(ABI_VERSION + 1));
        let why = unsafe { CStr::from_ptr(pincel_last_error()) }.to_str().unwrap().to_string();
        assert!(why.contains("Rebuild"), "{why}");
    }

    #[test]
    fn an_id_for_a_removed_element_is_ignored_not_followed() {
        unsafe {
            let canvas = pincel_canvas_new();
            let root = pincel_canvas_root(canvas);
            let group = pincel_element_add_stack(root, true);
            let inner = text(group, "hi");
            assert!(pincel_element_exists(group) && pincel_element_exists(inner));

            assert!(pincel_element_remove(group));
            assert!(!pincel_element_exists(group) && !pincel_element_exists(inner), "the contents went with it");

            // Everything that takes a stale id does nothing, or says there is nothing there.
            pincel_element_set_margin(group, 5);
            assert!(!pincel_element_set_text(inner, c("x").as_ptr()));
            assert_eq!(pincel_element_add_stack(group, true), 0);
            assert_eq!(pincel_element_child_count(group), 0);
            assert_eq!(pincel_element_parent(inner), 0);
            assert_eq!(pincel_element_index(group), -1);
            assert!(!pincel_element_remove(group), "removing it twice");
            assert!(!pincel_element_is_hovered(group));
            assert_eq!(pincel_element_get_text(inner, std::ptr::null_mut(), 0), 0);
            assert_eq!(pincel_element_get_rect(group).w, 0);

            pincel_canvas_free(canvas);
        }
    }

    #[test]
    fn freeing_a_canvas_invalidates_all_its_ids_and_ids_are_not_reused() {
        unsafe {
            let first = pincel_canvas_new();
            let root = pincel_canvas_root(first);
            let child = pincel_element_add_stack(root, true);
            pincel_canvas_free(first);
            assert!(!pincel_element_exists(root) && !pincel_element_exists(child));

            let second = pincel_canvas_new();
            let new_root = pincel_canvas_root(second);
            let new_child = pincel_element_add_stack(new_root, true);
            assert!(new_root != root && new_child != child, "even though the memory may well be the same");
            assert!(!pincel_element_exists(child), "so the old id still isn't some other element");
            pincel_canvas_free(second);
        }
    }

    #[test]
    fn the_root_cannot_be_removed_and_zero_is_never_an_element() {
        unsafe {
            let canvas = pincel_canvas_new();
            let root = pincel_canvas_root(canvas);
            assert!(root != 0);
            assert!(!pincel_element_remove(root));
            assert!(!pincel_element_move_to(root, root, 0));
            assert!(pincel_element_exists(root));
            assert!(!pincel_element_exists(0));
            pincel_canvas_free(canvas);
        }
    }

    #[test]
    fn elements_can_be_listed_found_and_moved_by_id() {
        unsafe {
            let canvas = pincel_canvas_new();
            let root = pincel_canvas_root(canvas);
            let a = text(root, "a");
            let b = text(root, "b");
            let group = pincel_element_add_stack(root, true);
            pincel_element_set_id(group, c("the group").as_ptr());

            assert_eq!(pincel_element_child_count(root), 3);
            assert_eq!((pincel_element_child_at(root, 0), pincel_element_child_at(root, 2)), (a, group));
            assert_eq!(pincel_element_child_at(root, 3), 0);
            assert_eq!(pincel_canvas_find(canvas, c("the group").as_ptr()), group);
            assert_eq!(pincel_canvas_find(canvas, c("nobody").as_ptr()), 0);

            assert!(pincel_element_move_to(b, group, 0), "into the group");
            assert_eq!((pincel_element_parent(b), pincel_element_index(b)), (group, 0));
            assert_eq!(pincel_element_child_count(root), 2);
            assert!(!pincel_element_move_to(group, b, 0), "a group can't go inside what is inside it");
            assert!(pincel_element_move_to(a, root, 99), "to the end of where it already is");
            assert_eq!(pincel_element_index(a), 1);

            pincel_element_clear_children(group);
            assert!(!pincel_element_exists(b), "cleared");
            pincel_canvas_free(canvas);
        }
    }

    #[test]
    fn text_is_copied_out_safely_and_never_cut_mid_character() {
        unsafe {
            let canvas = pincel_canvas_new();
            let root = pincel_canvas_root(canvas);
            let el = text(root, "héllo");
            assert_eq!(pincel_element_get_text(el, std::ptr::null_mut(), 0), 6, "bytes, with é being two");

            let mut buffer = [0x7f as c_char; 8];
            assert_eq!(pincel_element_get_text(el, buffer.as_mut_ptr(), buffer.len()), 6);
            assert_eq!(CStr::from_ptr(buffer.as_ptr()).to_str().unwrap(), "héllo");

            // Room for 2 bytes and a NUL: "h" fits, and the half of é that would follow it must not be copied.
            let mut small = [0x7f as c_char; 3];
            assert_eq!(pincel_element_get_text(el, small.as_mut_ptr(), small.len()), 6, "still says how long it really is");
            assert_eq!(CStr::from_ptr(small.as_ptr()).to_str().unwrap(), "h");
            pincel_canvas_free(canvas);
        }
    }

    #[test]
    fn fonts_register_by_name_and_report_why_not() {
        unsafe {
            assert_eq!(pincel_font_find(c("sans").as_ptr()), 0);
            assert_eq!(pincel_font_find(c("Mono").as_ptr()), 1);
            assert_eq!(pincel_font_find(c("Ffi Test Nothing").as_ptr()), -1);

            let jetbrains = include_bytes!("../assets/JetBrainsMono-Regular.ttf");
            let faces = PincelFontFaces {
                regular: jetbrains.as_ptr(),
                regular_len: jetbrains.len(),
                italic: std::ptr::null(),
                italic_len: 0,
                bold: std::ptr::null(),
                bold_len: 0,
                bold_italic: std::ptr::null(),
                bold_italic_len: 0,
            };
            let id = pincel_font_register(c("Ffi Test Family").as_ptr(), &faces);
            assert!(id >= 2);
            assert_eq!(pincel_font_find(c("ffi test family").as_ptr()), id);

            assert_eq!(pincel_font_register(c("Ffi Test Family").as_ptr(), &faces), -1, "the name is taken");
            assert!(CStr::from_ptr(pincel_last_error()).to_str().unwrap().contains("already exists"));
            assert_eq!(pincel_font_register(c("Ffi Test Nothing").as_ptr(), std::ptr::null()), -1);

            assert!(pincel_font_set_fallback(id as u16, 0));
            assert!(pincel_font_set_fallback(id as u16, -1), "removing it");
            assert!(!pincel_font_set_fallback(id as u16, 60_000), "no such family");
        }
    }

    #[test]
    fn style_setters_through_ids_take_effect_in_a_draw() {
        unsafe {
            let canvas = pincel_canvas_new();
            pincel_canvas_set_size(canvas, 100, 60);
            let root = pincel_canvas_root(canvas);
            let list = pincel_element_add_stack(root, true);
            pincel_element_set_size(list, 100, 40);
            pincel_element_set_overflow(list, 2);
            for _ in 0..4 {
                let item = pincel_element_add_stack(list, true);
                pincel_element_set_size(item, 100, 30);
            }
            pincel_canvas_draw(canvas);
            let scroll = pincel_element_get_scroll(list);
            assert_eq!((scroll.max_y, pincel_element_get_rect(list).h), (80, 40), "120 of content in 40");

            pincel_element_set_scroll(list, 0, 1_000);
            pincel_canvas_draw(canvas);
            assert_eq!(pincel_element_get_scroll(list).y, 80, "clamped to the end");

            pincel_element_set_visible(list, false);
            pincel_canvas_draw(canvas);
            assert_eq!(pincel_canvas_content_height(canvas), 0, "hidden, so nothing in the page");
            pincel_canvas_free(canvas);
        }
    }
}
