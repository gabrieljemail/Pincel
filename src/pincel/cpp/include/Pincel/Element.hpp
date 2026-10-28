#pragma once
#ifndef PINCEL_ELEMENT
#define PINCEL_ELEMENT

#include <cstddef>
#include <cstdint>
#include <limits>
#include <optional>
#include <string>
#include <utility>
#include <vector>
#include <Pincel/Export.hpp>
#include <Pincel/ffi.h>
#include <Pincel/Input.hpp>
#include <Pincel/Style.hpp>

namespace Pincel {

/// A handle to an element in a Canvas's tree. The tree (in Rust) owns the element, so a handle is cheap to copy.
///
/// A handle is an id, not a pointer, so it is safe to keep one after the element is removed (or its Canvas is
/// destroyed): IsValid() says whether it still exists, and calls on one that doesn't do nothing, or return a default.
/// A function that makes an element returns an invalid handle if it couldn't.
///
/// TODO: let users subclass this for their own element types, as the README describes.
class PINCEL_API Element {
public:
	/// Where to put an element that goes at the end, for MoveTo().
	static constexpr std::size_t End = std::numeric_limits<std::size_t>::max();

	explicit Element(PincelElement id = 0) : id(id) {}

	/// Whether the element still exists.
	bool IsValid() const;
	PincelElement Id() const { return id; }
	bool operator==(const Element& other) const { return id == other.id; }

	// ---- Adding elements. Each returns the new child, or an invalid handle if this element doesn't exist.

	/// Adds a Stack as a child of this element and returns it.
	Element AddStack(Orientation orientation = Orientation::Vertical);

	/// Adds Text as a child and returns it. The text is UTF-8 and wraps at word boundaries to the width available.
	Element AddText(const std::string& text);

	/// Adds an Input, a field the user types into: click it to focus it, then type. It is drawn in its own box, so
	/// give it a size, background, border and padding like any element, and its text takes the inherited text style.
	/// Read what was typed with GetText(), and know when it changed with InputChanged() and InputSubmitted().
	/// While one has focus, Canvas::WantsKeyboard() is true.
	Element AddInput(InputKind kind = InputKind::Text);

	/// Adds rich text: spans that wrap together as one piece of text, each with its own color, weight, italic,
	/// underline and strikethrough. A chat line is this: a sentence in several colors that wraps as a sentence. A word
	/// split between two spans (`Hel` in red, `lo` in green) is one word, and isn't broken between the colors. It uses
	/// the inherited font family and size, and what a span leaves out comes from the inherited style too.
	Element AddRichText(const std::vector<TextSpan>& spans);

	/// Replaces the spans of a rich text element. Returns false, changing nothing, for any other kind of element. The
	/// same spans as before cost nothing, so it is fine to call every frame.
	bool SetSpans(const std::vector<TextSpan>& spans);

	/// Adds an Image as a child from `width * height * 4` bytes of RGBA8 pixels (straight alpha, tightly packed
	/// rows, row 0 at the top). The pixels are copied. Returns nothing if the size is 0; see Pincel::LastError().
	///
	/// The image is drawn at its own size, shrunk to fit the width available. SetSize() with only one dimension
	/// keeps its aspect ratio, and with both stretches it. The size is of the border box, as for any element.
	std::optional<Element> AddImage(const std::uint8_t* rgba, std::uint32_t width, std::uint32_t height);

	/// Adds an Image as a child from a PNG or JPEG file. Returns nothing if it can't be read; see Pincel::LastError().
	std::optional<Element> AddImageFile(const std::string& path);

	// ---- Editing the tree.

	/// Removes this element, and everything inside it, from its Canvas and frees them. This handle, and any other to
	/// the element or what was in it, becomes invalid. Returns false, doing nothing, if it doesn't exist or is a
	/// Canvas's root.
	bool Remove();

	/// Removes everything inside this element.
	void RemoveChildren();

	/// Moves this element, with what is inside it, to be child number `index` of `newParent` (the last, for an index
	/// past the end, such as End). Moving it within its own parent reorders it. The handle stays valid. Returns false,
	/// changing nothing, if either doesn't exist, this is a Canvas's root, or `newParent` is this element or inside it.
	bool MoveTo(Element newParent, std::size_t index = End);

	/// Which child of its parent this is, from 0. Nothing for a Canvas's root.
	std::optional<std::size_t> Index() const;
	std::size_t ChildCount() const;
	/// The child at `index`, from 0. Nothing if there isn't one.
	std::optional<Element> Child(std::size_t index) const;
	/// The element this one is inside. Nothing for a Canvas's root.
	std::optional<Element> Parent() const;

	/// Names the element so Canvas::Find() can get it. Names don't have to be unique.
	Element& SetId(const std::string& name);

	/// A hidden element is as if it weren't in the tree: it takes no room, draws nothing, and gets no input. It keeps
	/// its place and its state, so showing it again restores it.
	Element& SetVisible(bool visible);

	// ---- Placement.

	/// Sets how the element is placed. Elements are in their parent's flow by default. An Absolute or Fixed one
	/// is taken out of it and placed by SetInsets, so it takes no room and moves nothing. A sidebar is a Fixed
	/// element with its top, right and bottom insets set to 0, and a width.
	Element& SetPosition(Position position);
	Element& SetInsets(const Insets& insets);

	/// What happens to children that don't fit in this element. Scroll cuts them off and lets the mouse wheel scroll
	/// them into view (the deepest scrolling element under the cursor gets it, and hands it on at the end of its
	/// scroll). A scrolling element needs a size on the axis it scrolls on, or it just grows to hold its children.
	/// Absolute children aren't moved by the scrolling, but are cut off by the box.
	Element& SetOverflow(Overflow overflow);

	/// For a scrolling element: when the content grows while it is scrolled to the end, it stays at the end, so it
	/// follows new content. For a chat log or a console.
	Element& SetStickToEnd(bool stick);

	/// How far a scrolling element is scrolled, and how far it can be, as of the last Canvas::Draw().
	ScrollState GetScroll() const;
	/// Scrolls to this many pixels. More than the content allows is fine, and goes to the end.
	void SetScroll(int x, int y);

	/// The element's border box as of the last Canvas::Draw(), in canvas pixels with scrolling applied.
	struct Rect { int x, y, w, h; };
	Rect GetRect() const;

	// ---- Style. Setters return this element so calls can be chained.

	/// The size is that of the border box in pixels (so it includes padding and border). 0 means auto.
	Element& SetSize(std::uint16_t width, std::uint16_t height);
	/// The same on every side, or a side each in CSS's order. The last one set is the one that counts.
	Element& SetMargin(std::uint16_t base);
	Element& SetMargin(std::uint16_t top, std::uint16_t right, std::uint16_t bottom, std::uint16_t left);
	Element& SetPadding(std::uint16_t base);
	Element& SetPadding(std::uint16_t top, std::uint16_t right, std::uint16_t bottom, std::uint16_t left);
	/// For a stack: where its children go in the room it has left over, on x and then y. A vertical stack lines each
	/// child up on x and moves the whole run on y, and a horizontal one the other way round. Without a size of its own
	/// there is no room left over on the main axis, so give it one to center what is in it.
	Element& SetAlign(Align x, Align y);
	/// For a stack: the space in pixels between its children.
	Element& SetGap(std::uint16_t gap);
	Element& SetBorder(std::uint16_t size, Color color);
	Element& SetBackground(Color color);
	/// Rounds the corners of the border box, and of an image's content box inside it. Clamped to half the shorter side.
	Element& SetCornerRadius(std::uint16_t radius);

	/// Makes the color properties (background, border color, and the text color if it is set) fade over `seconds`
	/// when they are set to something else, instead of jumping. Changing one again halfway starts from the color that
	/// is showing, so it never jumps. 0 turns it off. While a fade is running, Canvas::IsAnimating() is true.
	Element& SetTransition(float seconds, Easing easing = Easing::Ease);

	/// Text properties are inherited: Text draws with the closest value set on itself or an element it is inside, and
	/// the canvas supplies the rest (see Canvas::SetForeground). Setting one on a container styles all the text in it.
	/// The color of text.
	Element& SetForeground(Color color);
	/// Text size in pixels (default 16).
	Element& SetFontSize(std::uint16_t size);
	Element& SetFontFamily(FontFamily family);
	/// 100 to 900, see FontWeight. Use FontWeight::Bold for bold.
	Element& SetFontWeight(std::uint16_t weight);
	Element& SetFontStyle(FontStyle style);

	/// A shadow behind text: a copy of it in `color`, moved by (dx, dy) pixels. This is the hard-edged kind, which
	/// is cheap, and a color with no opacity (the default) draws no shadow at all. It is inherited like the other
	/// text properties, and a fully transparent color here turns it off for the text inside.
	Element& SetTextShadow(Color color, int dx = 2, int dy = 2);
	/// Goes back to taking the shadow from the parent, which is the default.
	Element& InheritTextShadow();

	/// Replaces the text of a Text element, or the value of an Input (which drops what it wouldn't have accepted typed,
	/// and puts the caret at the end). Returns false, changing nothing, for any other kind of element.
	/// Setting what is already there costs nothing, so it is fine to do every frame.
	bool SetText(const std::string& text);

	/// The text of a Text element or the value of an Input, as UTF-8. Empty for anything else.
	std::string GetText() const;

	// ---- Inputs.

	/// Shown, dimmed, while the Input is empty. These do nothing for other elements.
	Element& SetPlaceholder(const std::string& text);
	/// The most characters it takes. 0 for no limit.
	Element& SetMaxLength(std::uint32_t chars);
	/// The user changed the value during the last Draw(). True for one draw.
	bool InputChanged() const;
	/// The user pressed Enter in a single-line Input during the last Draw(). True for one draw.
	bool InputSubmitted() const;

	/// Selects characters `start` up to `end` in an Input (counting characters, not bytes), with the caret at `end`.
	/// Clamped to the text. Users select with the mouse (drag, or shift+click), shift with the movement keys, and
	/// Ctrl+A; ctrl moves by word; Ctrl+C, Ctrl+X and Ctrl+V copy, cut and paste (see Canvas::SetClipboard()); and what
	/// is selected is replaced by typing and pasting.
	void Select(std::size_t start, std::size_t end);
	void SelectAll();
	/// The selected range, or nothing if nothing is selected.
	std::optional<std::pair<std::size_t, std::size_t>> GetSelection() const;
	std::string GetSelectedText() const;
	/// The color behind selected text. A translucent blue by default.
	Element& SetSelectionColor(Color color);

	/// For an Input: draws runs of its value in their own colors, for syntax highlighting. What the runs don't cover has
	/// the text color, and a later run wins where they overlap. The same runs as before cost nothing, so it is fine to
	/// call every frame. These do nothing for other elements.
	Element& SetInputHighlights(const std::vector<InputHighlight>& runs);
	/// For an Input: a line too long for it carries on in a row below instead of scrolling sideways, and the input grows
	/// taller to hold the rows. Those rows are indented by the width of `marker`, which is drawn dimmed in the indent.
	Element& SetInputWrap(bool wrap, const std::string& marker = "");
	/// For an Input: where character `index` was drawn as of the last Draw(), as the top-left of the caret's place there
	/// in canvas pixels (`w` is 0) and the height of a row. Nothing for other elements or before the first Draw().
	std::optional<Rect> GetInputPosition(std::size_t index) const;

	/// Whether the element has keyboard focus. Clicking an Input gives it, and clicking elsewhere takes it away.
	bool IsFocused() const;
	/// Focuses it at the next Draw(), as if it were clicked. If the key that made you do this (a "T" that opens a
	/// chat box) is in that frame's KeyboardInput, the Input ignores it instead of typing it.
	void Focus();
	void Blur();

	// ---- Images.

	/// Replaces the pixels of an Image element, such as with a new frame of something you render. Same format as
	/// AddImage(), and copied. Returns false, changing nothing, if this isn't an Image or the size is 0.
	/// The same pixels as before cost nothing.
	bool SetImage(const std::uint8_t* rgba, std::uint32_t width, std::uint32_t height);
	/// Nearest is crisp pixel art, and a plain copy of the pixels at 1:1. Smooth (the default) is for photos.
	Element& SetImageFilter(ImageFilter filter);

	/// Whether the mouse can target this element. The default, Auto, makes it a target wherever it is. None makes the
	/// mouse go straight through it to whatever is under it (though what is inside it can still be a target), and it
	/// isn't hovered itself. Put None on containers that cover areas the host should still get clicks in, like a HUD
	/// that spans the screen: Canvas::WantsMouse() is then only true over its real widgets. (A Canvas's root is None.)
	Element& SetPointerEvents(PointerEvents mode);

	// ---- What the mouse did to this element as of the last Canvas::Draw(), which uses the input you gave it with
	// Canvas::SetInput(). Hover and press bubble up, so a container is hovered whenever something inside it is.
	//
	// Pincel only reports this: what a click does is up to you. A click is a press and a release on the same element,
	// per button, so a left and a right click each tell you which element they were on (an inventory slot, say).

	bool IsHovered() const;
	/// The button (MouseButton::Left, Right or Middle) is down, and went down on this element or inside it.
	bool IsPressed(std::uint8_t button = MouseButton::Left) const;
	/// The button came up this frame over the element it went down on. True for one draw.
	bool WasClicked(std::uint8_t button = MouseButton::Left) const;
	/// All the buttons that are pressed, or that were clicked, as a mask of MouseButton values.
	std::uint8_t PressedButtons() const;
	std::uint8_t ClickedButtons() const;

private:
	PincelElement id;
};

} // namespace Pincel

#endif
