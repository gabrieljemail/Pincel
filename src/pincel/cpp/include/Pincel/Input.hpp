#pragma once
#ifndef PINCEL_INPUT
#define PINCEL_INPUT

#include <cstdint>
#include <limits>
#include <string>
#include <vector>

namespace Pincel {

/// Mouse buttons, as a bitmask for MouseInput::buttons.
namespace MouseButton {
	constexpr std::uint8_t Left = 1;
	constexpr std::uint8_t Right = 2;
	constexpr std::uint8_t Middle = 4;
}

/// Keyboard modifiers, as a bitmask for KeyboardInput::modifiers.
namespace Modifier {
	constexpr std::uint8_t Shift = 1;
	constexpr std::uint8_t Control = 2;
	constexpr std::uint8_t Alt = 4;
	constexpr std::uint8_t Super = 8;
}

/// A key, named for what it does rather than for any one library's key codes, so you translate yours to these.
/// The values are part of the ABI and match PincelKey. The letters are consecutive, A through Z.
enum class Key : std::uint8_t {
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
	A = 32, B, C, D, E, F, G, H, I, J, K, L, M, N, O, P, Q, R, S, T, U, V, W, X, Y, Z,
};

enum class KeyAction : std::uint8_t {
	Press = 0,
	Repeat = 1, ///< The key is held and the operating system is repeating it.
	Release = 2,
};

struct KeyEvent {
	Key key = Key::Unknown;
	KeyAction action = KeyAction::Press;
};

/// What the mouse is doing this frame.
struct MouseInput {
	/// Position in canvas pixels from the top-left. NaN (the default) means the cursor isn't over the canvas.
	float x = std::numeric_limits<float>::quiet_NaN();
	float y = std::numeric_limits<float>::quiet_NaN();
	/// The buttons that are down right now, as a mask of MouseButton values.
	std::uint8_t buttons = 0;
	/// Wheel movement since the last frame, in notches. Positive is up and right.
	float scrollX = 0.0f;
	float scrollY = 0.0f;
};

/// What the keyboard is doing this frame.
struct KeyboardInput {
	/// Key presses, repeats and releases since the last frame, in order.
	std::vector<KeyEvent> events;
	/// Characters typed since the last frame, as Unicode code points. This is what the keyboard layout and
	/// modifiers produce, so use this for text and `events` for keys like Backspace and the arrows.
	std::u32string text;
	/// The modifiers held right now, as a mask of Modifier values.
	std::uint8_t modifiers = 0;
};

} // namespace Pincel

#endif
