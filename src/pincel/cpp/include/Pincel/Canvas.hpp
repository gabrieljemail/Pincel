#pragma once
#ifndef PINCEL_CANVAS
#define PINCEL_CANVAS

#include <cstdint>
#include <functional>
#include <optional>
#include <string>
#include <Pincel/Export.hpp>
#include <Pincel/ffi.h>
#include <Pincel/Element.hpp>
#include <Pincel/Input.hpp>
#include <Pincel/Style.hpp>
#include <Pincel/Texture.hpp>

namespace Pincel {

/// Represents a page containing elements that should be drawn to the screen.
class PINCEL_API Canvas {
public:
	Canvas();
	~Canvas();

	// The Rust canvas is owned by exactly one Canvas.
	Canvas(const Canvas&) = delete;
	Canvas& operator=(const Canvas&) = delete;

	/// The top of the element tree, a vertical Stack. Add your elements to this.
	Element Root();

	void SetBackground(Color color);

	/// The text color that elements inherit when nothing between them and the canvas sets one. Black by default.
	void SetForeground(Color color);

	/// The text shadow that text inherits when nothing between it and the canvas sets one: a copy of the text in
	/// `color`, moved by (dx, dy) pixels. By default there is none, and a color with no opacity draws nothing, so a
	/// shadow only costs anything once you turn one on. See Element::SetTextShadow().
	void SetTextShadow(Color color, int dx = 2, int dy = 2);

	/// The element with this id (see Element::SetId()), going down the tree in order. Nothing if there is none.
	std::optional<Element> Find(const std::string& id) const;

	/// The clock transitions run on, in seconds, which has to only go forward. Without it the canvas uses a clock of
	/// its own, which is right for most programs. Give it your own to run transitions on game time, so they pause
	/// when the game does.
	void SetTime(double seconds);

	/// Whether a transition was still running as of the last Draw(). A program that only draws when something
	/// changed should keep drawing while this is true.
	bool IsAnimating() const;

	/// Sets the size in pixels of the texture Draw() produces. Usually the size of your framebuffer.
	void SetSize(std::uint32_t width, std::uint32_t height);

	/// Gives the canvas this frame's input. Call it once per frame, before Draw(). Mouse positions are in
	/// canvas pixels, so if your canvas is your framebuffer they are the cursor position in framebuffer pixels.
	/// The wheel, button edges, key events and typed text only last one Draw(), so there is no need to clear them.
	void SetInput(const MouseInput& mouse, const KeyboardInput& keyboard);

	/// Whether the cursor is over something that takes the mouse (as of the last Draw()), or a press that began on
	/// something is still held. While it is, a click or drag belongs to the canvas, so don't also act on it: that is how
	/// a menu stops a click from reaching the game below it. Over empty canvas, or an element whose pointer events are
	/// None (see Element::SetPointerEvents()), it is false and the mouse is yours, so a HUD that covers the whole screen
	/// can leave the game clickable. A press that began on the canvas is still the canvas's, including its release.
	bool WantsMouse() const;

	/// What the last Draw() did with the mouse wheel.
	struct WheelUse { bool horizontal = false; bool vertical = false; };
	/// The wheel is used by a scrolling element under the cursor that could still move (or the page, if the cursor is
	/// over something), and what it didn't use is yours: a game's hotbar scrolls when the wheel is over empty canvas,
	/// or the list under it is already at its end.
	WheelUse WheelConsumed() const;

	/// Gives the canvas the host's clipboard for its inputs: `get` is asked for the text when the user pastes
	/// (Ctrl+V), and `set` is given the text when they copy or cut (Ctrl+C, Ctrl+X). Either can be empty. They are
	/// called during Draw(), on the thread that calls it, so use whatever your window library's clipboard needs
	/// there. Without this, copy and paste still work between the inputs in the canvas, through a clipboard of its own.
	void SetClipboard(std::function<std::string()> get, std::function<void(const std::string&)> set);

	/// Whether an Input has keyboard focus. While it does, the keyboard is Pincel's, so don't also use the keys
	/// for your own shortcuts (movement, hotkeys). Check it before acting on a key press.
	bool WantsKeyboard() const;

	/// Scrolling moves everything up and down. The mouse wheel does it by itself, `SetScrollStep()` pixels per
	/// notch, and it stops at the ends of the content. These are for doing it yourself, such as with a scrollbar.
	float GetScroll() const;
	/// Takes effect, clamped to the content, at the next Draw().
	void SetScroll(float y);
	void SetScrollStep(float pixels);
	/// The height of all the content as of the last Draw(), which can be more than the canvas's height.
	/// The most you can scroll is this minus the height.
	std::int32_t ContentHeight() const;

	/// Lays out the canvas, lets elements react to the input, and rasterizes it. The returned texture is owned
	/// by this Canvas and is overwritten by the next Draw(), so upload it before drawing again.
	Texture* Draw();

private:
	PincelCanvas* handle;
	Texture texture;

	// The host's clipboard functions, which the C side calls back into through `this`. A Canvas can't be copied or
	// moved, so that pointer stays good.
	std::function<std::string()> clipboardGet;
	std::function<void(const std::string&)> clipboardSet;
	std::string clipboardText; // What was last handed over, which has to stay valid until the next paste.
};

} // namespace Pincel

#endif
