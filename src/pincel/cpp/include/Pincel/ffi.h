#pragma once
// C ABI exported by the Rust core (src/pincel/rust/src/ffi.rs). Keep both in sync.
//
// A canvas is created and freed by the caller, and freeing it frees every element in it.
//
// Elements are named by an id (a PincelElement, 0 meaning "none"), not a pointer. Ids are never reused, so the id of
// an element that has been removed, or whose canvas was freed, just finds nothing: setters do nothing, getters return
// a default, and functions that return an id return 0. That makes removing elements safe while other code still holds
// a handle to one.
//
// A canvas, and the elements in it, must not be used from two threads at the same time. Taking turns is fine.
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

// The version of this interface: every function and struct in this file, and the layout of the C++ classes in the other
// headers. It goes up whenever any of that changes in a way that code built against the old headers can't use. The
// library (pincel.dll) has the version it was built with, and Pincel::Init() (or pincel_init_checked) refuses to start if
// that isn't the version of the headers the program was built with, which catches a DLL that wasn't rebuilt to match.
#define PINCEL_ABI_VERSION 2u

uint32_t pincel_abi_version(void);
// Like pincel_init, but first checks `header_version` (pass PINCEL_ABI_VERSION) against the library's. Returns false,
// and sets the last error, if they differ, in which case nothing else may be called.
bool pincel_init_checked(uint32_t header_version);
void pincel_init(void);
bool pincel_is_initialized(void);
const char* pincel_version(void);
// The message from the last call on this thread that failed (returned 0 or -1 where that means failure).
const char* pincel_last_error(void);

typedef struct PincelCanvas PincelCanvas;
typedef uint64_t PincelElement;

// Straight (not premultiplied) alpha, 0.0 to 1.0.
typedef struct PincelColor {
	uint8_t r, g, b;
	float a;
} PincelColor;

// RGBA8, premultiplied alpha, tightly packed rows, row 0 at the top.
// `data` is owned by the canvas and valid until the next draw or until the canvas is freed.
// `version` goes up whenever the pixels are redrawn, so a host that keeps the last version on the GPU can skip
// uploading when it hasn't changed.
typedef struct PincelTexture {
	uint32_t width, height;
	const uint8_t* data;
	uint64_t version;
} PincelTexture;

typedef struct PincelRect {
	int32_t x, y, w, h;
} PincelRect;

// ---- Canvas ----
PincelCanvas* pincel_canvas_new(void);
void pincel_canvas_free(PincelCanvas* canvas);
// The root element, a vertical stack. It can't be removed.
PincelElement pincel_canvas_root(PincelCanvas* canvas);
// The first element with this name (see pincel_element_set_id), going down the tree in order. 0 if there is none.
PincelElement pincel_canvas_find(const PincelCanvas* canvas, const char* id);
void pincel_canvas_set_background(PincelCanvas* canvas, PincelColor color);
void pincel_canvas_set_size(PincelCanvas* canvas, uint32_t width, uint32_t height);
// The text color that elements inherit when nothing between them and the canvas sets one.
void pincel_canvas_set_foreground(PincelCanvas* canvas, PincelColor color);
// The text shadow that text inherits. A fully transparent color (the default) means no shadow, and none is drawn.
void pincel_canvas_set_text_shadow(PincelCanvas* canvas, PincelColor color, int32_t offset_x, int32_t offset_y);
// The clock transitions run on, in seconds, which only goes forward. Without it the canvas uses one of its own.
void pincel_canvas_set_time(PincelCanvas* canvas, double seconds);
// Whether a transition was still running as of the last draw. Keep drawing while it is.
bool pincel_canvas_is_animating(const PincelCanvas* canvas);
PincelTexture pincel_canvas_draw(PincelCanvas* canvas);

// ---- Input and scrolling ----
typedef struct PincelMouseInput {
	// Position in canvas pixels from the top-left. NaN when the cursor isn't over the canvas.
	float x, y;
	// Buttons that are down: 1 left, 2 right, 4 middle.
	uint8_t buttons;
	// Wheel movement since the last frame, in notches. Positive is up and right.
	float scroll_x, scroll_y;
} PincelMouseInput;

// The numbers are part of the ABI. A is 32 and the letters follow in order up to Z (57).
typedef enum PincelKey {
	PINCEL_KEY_UNKNOWN = 0,
	PINCEL_KEY_BACKSPACE = 1,
	PINCEL_KEY_DELETE = 2,
	PINCEL_KEY_ENTER = 3,
	PINCEL_KEY_TAB = 4,
	PINCEL_KEY_ESCAPE = 5,
	PINCEL_KEY_LEFT = 6,
	PINCEL_KEY_RIGHT = 7,
	PINCEL_KEY_UP = 8,
	PINCEL_KEY_DOWN = 9,
	PINCEL_KEY_HOME = 10,
	PINCEL_KEY_END = 11,
	PINCEL_KEY_PAGE_UP = 12,
	PINCEL_KEY_PAGE_DOWN = 13,
	PINCEL_KEY_SPACE = 14,
	PINCEL_KEY_A = 32,
	PINCEL_KEY_Z = 57,
} PincelKey;

typedef struct PincelKeyEvent {
	uint8_t key;    // A PincelKey.
	uint8_t action; // 0 press, 1 repeat, 2 release.
} PincelKeyEvent;

typedef struct PincelKeyboardInput {
	const PincelKeyEvent* events;
	size_t event_count;
	// Unicode code points typed since the last frame.
	const uint32_t* text;
	size_t text_count;
	// Modifiers held: 1 shift, 2 control, 4 alt, 8 super.
	uint8_t modifiers;
} PincelKeyboardInput;

// Call once per frame, before pincel_canvas_draw. Either pointer may be NULL to say nothing changed for that
// device. The arrays are copied. The per-frame parts are cleared by the draw.
void pincel_canvas_set_input(PincelCanvas* canvas, const PincelMouseInput* mouse, const PincelKeyboardInput* keyboard);
// Whether an input has keyboard focus. While it does, the keyboard is Pincel's: don't also use the keys for your own
// shortcuts.
bool pincel_canvas_wants_keyboard(const PincelCanvas* canvas);
// How far the page is scrolled up, in pixels.
float pincel_canvas_get_scroll(const PincelCanvas* canvas);
// Takes effect, clamped to the content, at the next draw.
void pincel_canvas_set_scroll(PincelCanvas* canvas, float y);
// Pixels scrolled per notch of the mouse wheel, for the page and for scrolling elements.
void pincel_canvas_set_scroll_step(PincelCanvas* canvas, float pixels);
// The height of all the content as of the last draw, which can be more than the canvas's height.
int32_t pincel_canvas_content_height(const PincelCanvas* canvas);

// ---- Adding elements (each returns the new element, or 0 if `parent` doesn't exist or it couldn't be made) ----
PincelElement pincel_element_add_stack(PincelElement parent, bool vertical);
// `text` is NUL-terminated UTF-8 and is copied.
PincelElement pincel_element_add_text(PincelElement parent, const char* text);
// kind: 0 one line of text, 1 several lines, 2 a number.
PincelElement pincel_element_add_input(PincelElement parent, uint8_t kind);
// `rgba` is width * height * 4 bytes of straight-alpha RGBA8 and is copied.
PincelElement pincel_element_add_image(PincelElement parent, const uint8_t* rgba, uint32_t width, uint32_t height);
// `path` is NUL-terminated UTF-8 to a PNG or JPEG.
PincelElement pincel_element_add_image_file(PincelElement parent, const char* path);

// ---- Editing the tree ----
// Removes the element, and everything inside it, from its canvas and frees them. Their ids become invalid.
// Returns false, doing nothing, if it doesn't exist or is a canvas's root.
bool pincel_element_remove(PincelElement el);
void pincel_element_clear_children(PincelElement el);
// Moves the element, with what is inside it, to be child number `index` of `new_parent` (the last, for an index past
// the end). The same parent just reorders it. Its id stays the same. Returns false, changing nothing, if either
// doesn't exist, it is a canvas's root, or `new_parent` is the element or is inside it.
bool pincel_element_move_to(PincelElement el, PincelElement new_parent, uint32_t index);
// Which child of its parent it is, from 0. -1 if it doesn't exist or is a canvas's root.
int32_t pincel_element_index(PincelElement el);
uint32_t pincel_element_child_count(PincelElement el);
PincelElement pincel_element_child_at(PincelElement el, uint32_t index); // 0 if there is none.
PincelElement pincel_element_parent(PincelElement el);                   // 0 for a canvas's root.
// Names the element, for pincel_canvas_find. Names don't have to be unique.
void pincel_element_set_id(PincelElement el, const char* name);
bool pincel_element_exists(PincelElement el);
// A hidden element is as if it weren't in the tree (no room, no drawing, no input), but keeps its place and state.
void pincel_element_set_visible(PincelElement el, bool visible);

// ---- Box model and colors ----
// The border-box size in pixels. 0 for a dimension means "auto".
void pincel_element_set_size(PincelElement el, uint16_t width, uint16_t height);
void pincel_element_set_margin(PincelElement el, uint16_t base);
void pincel_element_set_margin_sides(PincelElement el, uint16_t top, uint16_t right, uint16_t bottom, uint16_t left);
void pincel_element_set_padding(PincelElement el, uint16_t base);
void pincel_element_set_padding_sides(PincelElement el, uint16_t top, uint16_t right, uint16_t bottom, uint16_t left);
// For a stack: where its children go in the room left over, on x then on y (0 start, 1 center, 2 end).
void pincel_element_set_align(PincelElement el, uint8_t x, uint8_t y);
// For a stack: the space in pixels between its children.
void pincel_element_set_gap(PincelElement el, uint16_t gap);
void pincel_element_set_border(PincelElement el, uint16_t size, PincelColor color);
void pincel_element_set_background(PincelElement el, PincelColor color);
void pincel_element_set_corner_radius(PincelElement el, uint16_t radius);
// How the colors (background, border color, and the text color when set) change when set to something else: a fade of
// `seconds`, or at once for 0. easing: 0 linear, 1 ease, 2 ease-in, 3 ease-out, 4 ease-in-out.
void pincel_element_set_transition(PincelElement el, float seconds, uint8_t easing);

// ---- Text (these properties are inherited: text uses the closest value set on it or an element it is inside) ----
void pincel_element_set_foreground(PincelElement el, PincelColor color);
void pincel_element_set_font_size(PincelElement el, uint16_t size);
// A font family id: 0 and 1 are the built-in sans-serif and monospace, and pincel_font_register gives more.
void pincel_element_set_font_family(PincelElement el, uint16_t family);
// 100 to 900. 600 and up is drawn bold, and lower is regular.
void pincel_element_set_font_weight(PincelElement el, uint16_t weight);
// 0 is normal, 1 is italic.
void pincel_element_set_font_style(PincelElement el, uint8_t style);
// A copy of the text in `color`, offset by (offset_x, offset_y) pixels, drawn behind it. A fully transparent color
// means no shadow, including for text inside that would otherwise get one from further up.
void pincel_element_set_text_shadow(PincelElement el, PincelColor color, int32_t offset_x, int32_t offset_y);
// Goes back to taking the shadow from the parent, which is the default.
void pincel_element_inherit_text_shadow(PincelElement el);
// `text` is NUL-terminated UTF-8 and is copied. Returns false, changing nothing, if the element isn't Text or an Input.
bool pincel_element_set_text(PincelElement el, const char* text);
// Copies the text of a Text element or the value of an Input into `buffer` (at most `capacity` bytes, with a NUL), and
// returns how many bytes the whole text is. `buffer` may be NULL to just ask. Returns 0 for no text.
size_t pincel_element_get_text(PincelElement el, char* buffer, size_t capacity);

// ---- Fonts ----
// Font files in memory. Only the regular face is needed: leave the others NULL, and a style the family has no face for
// is drawn with the closest one it has.
typedef struct PincelFontFaces {
	const uint8_t* regular;
	size_t regular_len;
	const uint8_t* italic;
	size_t italic_len;
	const uint8_t* bold;
	size_t bold_len;
	const uint8_t* bold_italic;
	size_t bold_italic_len;
} PincelFontFaces;
// Adds a font family (TrueType or OpenType) and returns its id, for pincel_element_set_font_family. The data is
// copied. The name must be new, ignoring case. Returns -1, and sets the last error, if a face isn't a usable font or
// the name is taken. The built-in families are named "Sans" (0) and "Mono" (1).
int32_t pincel_font_register(const char* name, const PincelFontFaces* faces);
// The same, reading the faces from files (NUL-terminated UTF-8 paths, NULL for a face the family doesn't have).
int32_t pincel_font_register_files(const char* name, const char* regular, const char* italic, const char* bold, const char* bold_italic);
// The id of the family with this name, ignoring case, or -1.
int32_t pincel_font_find(const char* name);
// Draw characters `family` has no glyph for from `fallback` instead. -1 for `fallback` removes it. Returns false if
// either family doesn't exist.
bool pincel_font_set_fallback(uint16_t family, int32_t fallback);

// ---- Images ----
// Replaces the pixels of an Image element, such as with a new frame. Same buffer as pincel_element_add_image, copied.
// Returns false, changing nothing, if the element isn't an Image or the size is 0.
bool pincel_element_set_image(PincelElement el, const uint8_t* rgba, uint32_t width, uint32_t height);
// 0 smooth (bilinear, averaging when shrunk), 1 nearest (crisp pixels, and a plain copy at 1:1).
void pincel_element_set_image_filter(PincelElement el, uint8_t filter);

// ---- Positioning, overflow and scrolling ----
// 0 in the parent's flow, 1 absolute (against the parent's padding box, scrolls), 2 fixed (against the whole canvas,
// doesn't scroll). Anything else is in the flow.
void pincel_element_set_position(PincelElement el, uint8_t position);
// Sets, or with has_value false clears, how far in from one side an out-of-flow element is placed.
// side: 0 left, 1 top, 2 right, 3 bottom. Both sides of an axis with no size on it stretch the element.
void pincel_element_set_inset(PincelElement el, uint8_t side, bool has_value, int32_t value);
// Centers an out-of-flow element on an axis, whatever its insets say.
void pincel_element_set_inset_center(PincelElement el, bool center_x, bool center_y);
// What happens to children that don't fit: 0 drawn outside the box, 1 cut off at it, 2 cut off and scrollable with the
// mouse wheel (or pincel_element_set_scroll). A scrolling element needs a size on the axis it scrolls on.
void pincel_element_set_overflow(PincelElement el, uint8_t overflow);
// For a scrolling element: when the content grows while it is at the end, it follows. For a chat log.
void pincel_element_set_stick_to_end(PincelElement el, bool stick);

typedef struct PincelScroll {
	int32_t x, y;         // How far the content is scrolled left and up, in pixels.
	int32_t max_x, max_y; // The most it can be scrolled, as of the last draw.
} PincelScroll;
PincelScroll pincel_element_get_scroll(PincelElement el);
// Asking for more than the content allows is fine, and goes to the end.
void pincel_element_set_scroll(PincelElement el, int32_t x, int32_t y);
// The element's border box as of the last draw, in canvas pixels with scrolling applied (so it matches the mouse).
PincelRect pincel_element_get_rect(PincelElement el);

// ---- Interaction, input fields and focus ----
// What the mouse did to an element as of the last pincel_canvas_draw. Hover and press bubble up, so a container is
// hovered whenever something in it is.
bool pincel_element_is_hovered(PincelElement el);
bool pincel_element_is_pressed(PincelElement el); // The left button is down, and went down on or inside it.
bool pincel_element_was_clicked(PincelElement el); // The left button came up over the element it went down on. One draw.
bool pincel_element_is_focused(PincelElement el);
// For an Input. These do nothing for other elements.
void pincel_element_set_placeholder(PincelElement el, const char* text);
void pincel_element_set_max_length(PincelElement el, uint32_t max_chars); // 0 for no limit.
bool pincel_element_input_changed(PincelElement el);   // The user changed the value during the last draw.
bool pincel_element_input_submitted(PincelElement el); // The user pressed Enter in a single-line Input.
// Takes effect at the next draw. The key press that made you ask is ignored by the input, if it is in that frame.
void pincel_element_focus(PincelElement el);
void pincel_element_blur(PincelElement el);

// ---- Pointer events and the mouse buttons ----
// 0 the element is a target of the mouse (the default), 1 the mouse goes through it to whatever is under it, though
// what is inside it can still be a target. Use 1 on containers that cover areas the host should still get clicks in.
void pincel_element_set_pointer_events(PincelElement el, uint8_t mode);
// The buttons down that went down on the element or inside it, and the buttons that came up this frame over the element
// they went down on, as masks: 1 left, 2 right, 4 middle. A button is in the second for one draw.
uint8_t pincel_element_pressed_buttons(PincelElement el);
uint8_t pincel_element_clicked_buttons(PincelElement el);
// Whether the cursor is over something that takes the mouse, or a press that began on something is still held. While it
// is, the mouse belongs to the canvas and the host shouldn't also act on it.
bool pincel_canvas_wants_mouse(const PincelCanvas* canvas);
// Whether the last draw used the mouse wheel, as a mask: 1 horizontal, 2 vertical. A wheel it didn't use is the host's.
uint8_t pincel_canvas_wheel_consumed(const PincelCanvas* canvas);

// ---- Rich text ----
// One run of rich text. What isn't set is taken from the element's text style.
typedef struct PincelTextSpan {
	const char* text;   // NUL-terminated UTF-8.
	bool has_color;
	PincelColor color;
	uint16_t font_weight; // 100 to 900, or 0 to take the element's.
	int8_t font_style;    // -1 takes the element's, 0 is normal, 1 is italic.
	bool underline;
	bool strikethrough;
} PincelTextSpan;
// Spans wrap together as one piece of text, each with its own look. The text is copied.
PincelElement pincel_element_add_rich_text(PincelElement parent, const PincelTextSpan* spans, size_t count);
// Returns false, changing nothing, if the element isn't rich text. The same spans as before cost nothing.
bool pincel_element_set_spans(PincelElement el, const PincelTextSpan* spans, size_t count);

// ---- Selection and the clipboard ----
// Character positions (not bytes) in an Input, with the caret at `end`. Clamped to the text.
void pincel_element_select(PincelElement el, uint32_t start, uint32_t end);
// Returns false, leaving the outputs alone, if nothing is selected.
bool pincel_element_get_selection(PincelElement el, uint32_t* start, uint32_t* end);
size_t pincel_element_get_selected_text(PincelElement el, char* buffer, size_t capacity); // As pincel_element_get_text.
void pincel_element_set_selection_color(PincelElement el, PincelColor color);

// ---- Input highlighting and wrapping ----
// One run of an Input's value in its own color, as character positions (not bytes): start up to end.
typedef struct PincelHighlight {
	uint32_t start, end;
	PincelColor color;
} PincelHighlight;
// What the runs don't cover has the text color, and a later run wins where they overlap. The same runs as before cost
// nothing. Does nothing for other elements.
void pincel_element_set_highlights(PincelElement el, const PincelHighlight* items, size_t count);
// Where character `index` of an Input was drawn, as of the last draw: the top-left of the caret's place there in canvas
// pixels, and a row's height. Returns false, leaving the outputs alone, for other elements or before the first draw.
bool pincel_element_get_input_position(PincelElement el, uint32_t index, int32_t* x, int32_t* y, int32_t* height);
// Makes an Input carry a long line on in a row below instead of scrolling sideways, growing taller to hold the rows.
// Those rows are indented by the width of `marker` (NUL-terminated UTF-8, or NULL for none), drawn dimmed in the indent.
void pincel_element_set_wrap(PincelElement el, bool wrap, const char* marker);

// The host's clipboard, which inputs copy to (Ctrl+C, Ctrl+X) and paste from (Ctrl+V). `get` returns the text as
// NUL-terminated UTF-8 that stays valid until the next call (or NULL for none), and `set` puts text on it. Either may
// be NULL. They are called during pincel_canvas_draw, on the calling thread. Without callbacks, copy and paste still
// work between inputs in the canvas.
typedef const char* (*PincelClipboardGet)(void* user);
typedef void (*PincelClipboardSet)(void* user, const char* text);
void pincel_canvas_set_clipboard(PincelCanvas* canvas, PincelClipboardGet get, PincelClipboardSet set, void* user);

#ifdef __cplusplus
}
#endif
