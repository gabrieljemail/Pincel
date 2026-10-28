#pragma once
#ifndef PINCEL_STYLE
#define PINCEL_STYLE

#include <cstdint>
#include <optional>
#include <string>

namespace Pincel {

/// Straight (not premultiplied) alpha, 0.0 to 1.0. Layout matches the Rust side, so it crosses the FFI as-is.
struct Color {
	std::uint8_t r = 0, g = 0, b = 0;
	float a = 1.0f;

	/// Parses "#rrggbb" at compile time. Anything else gives opaque black.
	static constexpr Color Hex(const char* hex, float alpha = 1.0f) {
		auto digit = [](char c) -> int {
			return c >= '0' && c <= '9' ? c - '0' : c >= 'a' && c <= 'f' ? c - 'a' + 10 : c >= 'A' && c <= 'F' ? c - 'A' + 10 : -1;
		};
		if (hex[0] != '#') return {0, 0, 0, alpha};
		int v[6];
		for (int i = 0; i < 6; i++) {
			v[i] = digit(hex[i + 1]);
			if (v[i] < 0) return {0, 0, 0, alpha};
		}
		return {
			static_cast<std::uint8_t>(v[0] * 16 + v[1]),
			static_cast<std::uint8_t>(v[2] * 16 + v[3]),
			static_cast<std::uint8_t>(v[4] * 16 + v[5]),
			alpha
		};
	}
};

enum class Orientation : std::uint8_t { Vertical, Horizontal };

/// A font family text can be drawn in: one of the built-in ones, FontFamily::Sans and FontFamily::Mono, or one you
/// load with LoadFontFamily() (see Font.hpp).
struct FontFamily {
	std::uint16_t id = 0;

	constexpr bool operator==(const FontFamily&) const = default;

	static const FontFamily Sans; ///< A proportional sans-serif font (Lato).
	static const FontFamily Mono; ///< A monospace font (JetBrains Mono).
};
inline constexpr FontFamily FontFamily::Sans{0};
inline constexpr FontFamily FontFamily::Mono{1};

enum class FontStyle : std::uint8_t { Normal = 0, Italic = 1 };

/// How the speed of a transition changes from start to end. These are CSS's timing functions.
enum class Easing : std::uint8_t {
	Linear = 0,
	Ease = 1, ///< Quick to start, slow to settle. The usual default.
	EaseIn = 2,
	EaseOut = 3,
	EaseInOut = 4,
};

/// What happens to children that don't fit in an element's box.
enum class Overflow : std::uint8_t {
	Visible = 0, ///< They are drawn outside it.
	Hidden = 1,  ///< They are cut off at the edge of the box (inside its border).
	Scroll = 2,  ///< They are cut off, and can be scrolled into view with the mouse wheel. Needs a size on that axis.
};

/// Where a stack puts its children in the room it has left over. See Element::SetAlign().
enum class Align : std::uint8_t {
	Start = 0, ///< Left, or top. The default.
	Center = 1,
	End = 2,   ///< Right, or bottom.
};

/// Whether an element can be the target of the mouse. See Element::SetPointerEvents().
enum class PointerEvents : std::uint8_t {
	Auto = 0, ///< The element is a target wherever it is. The default.
	None = 1, ///< The mouse goes through it to whatever is under it. What is inside it can still be a target.
};

/// One run of rich text: some text, and the ways it differs from the element it is in. Anything left out is taken from
/// the element's text style. See Element::AddRichText().
struct TextSpan {
	std::string text;
	std::optional<Color> color;
	std::optional<std::uint16_t> weight; ///< 100 to 900, see FontWeight.
	std::optional<FontStyle> style;
	bool underline = false;
	bool strikethrough = false;
};

/// A run of an Input's value drawn in its own color, as character positions (not bytes): `start` up to `end`.
/// See Element::SetInputHighlights().
struct InputHighlight {
	std::size_t start = 0, end = 0;
	Color color;
};

/// How far a scrolling element is scrolled, and how far it can be, in pixels.
struct ScrollState {
	int x = 0, y = 0;
	int maxX = 0, maxY = 0;
};

/// How an element is placed.
enum class Position : std::uint8_t {
	Flow = 0,     ///< In order with its siblings, in its parent's stack.
	Absolute = 1, ///< Out of the stack (takes no room), placed against its parent's padding box. Scrolls with the content.
	Fixed = 2,    ///< Like Absolute, but placed against the whole canvas and it stays put while the content scrolls.
};

/// How far in from each edge an out-of-flow (Absolute or Fixed) element is placed. A side that is left out isn't
/// used. Both sides of an axis with no size on it stretch the element between them, and with a size, left and top win.
struct Insets {
	std::optional<int> left, top, right, bottom;
	bool centerX = false; ///< Center on the x axis, whatever left and right say.
	bool centerY = false;
};

/// How an Image is sampled when it isn't drawn at its own size.
enum class ImageFilter : std::uint8_t {
	Smooth = 0,  ///< Bilinear when enlarged, averaged when shrunk. Right for photos.
	Nearest = 1, ///< The nearest source pixel: crisp enlarged pixel art, and a plain copy of the pixels at 1:1.
};

/// What an Input takes.
enum class InputKind : std::uint8_t {
	Text = 0,      ///< One line. Enter submits it.
	Multiline = 1, ///< Several lines. Enter starts a new one.
	Number = 2,    ///< One line of digits, with one decimal point and a minus at the start. Read it and convert it.
};

/// Font weights on the CSS scale, 100 to 900. A family has up to a regular and a bold face, so anything from 600 up
/// is drawn bold and anything lower is regular.
namespace FontWeight {
	constexpr std::uint16_t Normal = 400;
	constexpr std::uint16_t Bold = 700;
}

} // namespace Pincel

#endif
