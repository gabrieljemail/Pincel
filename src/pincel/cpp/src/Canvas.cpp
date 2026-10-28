#include <Pincel/Canvas.hpp>

namespace Pincel {

Canvas::Canvas() : handle(pincel_canvas_new()) {}
Canvas::~Canvas() { pincel_canvas_free(handle); }

Element Canvas::Root() { return Element(pincel_canvas_root(handle)); }

void Canvas::SetBackground(Color color) {
	pincel_canvas_set_background(handle, PincelColor{color.r, color.g, color.b, color.a});
}

void Canvas::SetForeground(Color color) {
	pincel_canvas_set_foreground(handle, PincelColor{color.r, color.g, color.b, color.a});
}

void Canvas::SetTextShadow(Color color, int dx, int dy) {
	pincel_canvas_set_text_shadow(handle, PincelColor{color.r, color.g, color.b, color.a}, dx, dy);
}

void Canvas::SetTime(double seconds) { pincel_canvas_set_time(handle, seconds); }
bool Canvas::IsAnimating() const { return pincel_canvas_is_animating(handle); }

std::optional<Element> Canvas::Find(const std::string& id) const {
	PincelElement found = pincel_canvas_find(handle, id.c_str());
	if (found == 0) return std::nullopt;
	return Element(found);
}

void Canvas::SetSize(std::uint32_t width, std::uint32_t height) {
	pincel_canvas_set_size(handle, width, height);
}

// These match the C structs field for field, so the arrays can be handed over without copying.
static_assert(sizeof(KeyEvent) == sizeof(PincelKeyEvent));
static_assert(sizeof(char32_t) == sizeof(std::uint32_t));

void Canvas::SetInput(const MouseInput& mouse, const KeyboardInput& keyboard) {
	PincelMouseInput m{mouse.x, mouse.y, mouse.buttons, mouse.scrollX, mouse.scrollY};
	PincelKeyboardInput k{
		reinterpret_cast<const PincelKeyEvent*>(keyboard.events.data()), keyboard.events.size(),
		reinterpret_cast<const std::uint32_t*>(keyboard.text.data()), keyboard.text.size(),
		keyboard.modifiers
	};
	pincel_canvas_set_input(handle, &m, &k);
}

bool Canvas::WantsKeyboard() const { return pincel_canvas_wants_keyboard(handle); }
bool Canvas::WantsMouse() const { return pincel_canvas_wants_mouse(handle); }

Canvas::WheelUse Canvas::WheelConsumed() const {
	std::uint8_t used = pincel_canvas_wheel_consumed(handle);
	return {(used & 1) != 0, (used & 2) != 0};
}

void Canvas::SetClipboard(std::function<std::string()> get, std::function<void(const std::string&)> set) {
	clipboardGet = std::move(get);
	clipboardSet = std::move(set);
	if (!clipboardGet && !clipboardSet) {
		pincel_canvas_set_clipboard(handle, nullptr, nullptr, nullptr);
		return;
	}
	pincel_canvas_set_clipboard(handle,
		clipboardGet ? +[](void* user) -> const char* {
			Canvas* canvas = static_cast<Canvas*>(user);
			canvas->clipboardText = canvas->clipboardGet();
			return canvas->clipboardText.c_str();
		} : nullptr,
		clipboardSet ? +[](void* user, const char* text) {
			static_cast<Canvas*>(user)->clipboardSet(text);
		} : nullptr,
		this);
}

float Canvas::GetScroll() const { return pincel_canvas_get_scroll(handle); }
void Canvas::SetScroll(float y) { pincel_canvas_set_scroll(handle, y); }
void Canvas::SetScrollStep(float pixels) { pincel_canvas_set_scroll_step(handle, pixels); }
std::int32_t Canvas::ContentHeight() const { return pincel_canvas_content_height(handle); }

Texture* Canvas::Draw() {
	PincelTexture t = pincel_canvas_draw(handle);
	texture.width = t.width;
	texture.height = t.height;
	texture.data = t.data;
	texture.version = t.version;
	return &texture;
}

} // namespace Pincel
