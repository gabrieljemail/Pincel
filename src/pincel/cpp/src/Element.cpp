#include <Pincel/Element.hpp>

namespace Pincel {

// Color has the same layout as PincelColor (checked here), so it is passed straight through.
static_assert(sizeof(Color) == sizeof(PincelColor) && alignof(Color) == alignof(PincelColor));
static PincelColor ToC(Color c) { return PincelColor{c.r, c.g, c.b, c.a}; }

static std::optional<Element> ValidOrNothing(PincelElement id) {
	if (id == 0) return std::nullopt;
	return Element(id);
}

bool Element::IsValid() const { return pincel_element_exists(id); }

Element Element::AddStack(Orientation orientation) {
	return Element(pincel_element_add_stack(id, orientation == Orientation::Vertical));
}

Element Element::AddText(const std::string& text) {
	return Element(pincel_element_add_text(id, text.c_str()));
}

Element Element::AddInput(InputKind kind) {
	return Element(pincel_element_add_input(id, static_cast<std::uint8_t>(kind)));
}

std::optional<Element> Element::AddImage(const std::uint8_t* rgba, std::uint32_t width, std::uint32_t height) {
	return ValidOrNothing(pincel_element_add_image(id, rgba, width, height));
}

std::optional<Element> Element::AddImageFile(const std::string& path) {
	return ValidOrNothing(pincel_element_add_image_file(id, path.c_str()));
}

bool Element::Remove() { return pincel_element_remove(id); }
void Element::RemoveChildren() { pincel_element_clear_children(id); }

bool Element::MoveTo(Element newParent, std::size_t index) {
	// An index past what a 32 bit one holds is past the end anyway.
	std::uint32_t clamped = index > std::numeric_limits<std::uint32_t>::max() ? std::numeric_limits<std::uint32_t>::max() : static_cast<std::uint32_t>(index);
	return pincel_element_move_to(id, newParent.id, clamped);
}

std::optional<std::size_t> Element::Index() const {
	std::int32_t index = pincel_element_index(id);
	if (index < 0) return std::nullopt;
	return static_cast<std::size_t>(index);
}

std::size_t Element::ChildCount() const { return pincel_element_child_count(id); }

std::optional<Element> Element::Child(std::size_t index) const {
	if (index > std::numeric_limits<std::uint32_t>::max()) return std::nullopt;
	return ValidOrNothing(pincel_element_child_at(id, static_cast<std::uint32_t>(index)));
}

std::optional<Element> Element::Parent() const { return ValidOrNothing(pincel_element_parent(id)); }

Element& Element::SetId(const std::string& name) {
	pincel_element_set_id(id, name.c_str());
	return *this;
}

Element& Element::SetVisible(bool visible) {
	pincel_element_set_visible(id, visible);
	return *this;
}

Element& Element::SetPosition(Position position) {
	pincel_element_set_position(id, static_cast<std::uint8_t>(position));
	return *this;
}

Element& Element::SetInsets(const Insets& insets) {
	auto side = [this](std::uint8_t which, const std::optional<int>& value) {
		pincel_element_set_inset(id, which, value.has_value(), value.value_or(0));
	};
	side(0, insets.left);
	side(1, insets.top);
	side(2, insets.right);
	side(3, insets.bottom);
	pincel_element_set_inset_center(id, insets.centerX, insets.centerY);
	return *this;
}

Element& Element::SetOverflow(Overflow overflow) {
	pincel_element_set_overflow(id, static_cast<std::uint8_t>(overflow));
	return *this;
}

Element& Element::SetStickToEnd(bool stick) {
	pincel_element_set_stick_to_end(id, stick);
	return *this;
}

ScrollState Element::GetScroll() const {
	PincelScroll s = pincel_element_get_scroll(id);
	return {s.x, s.y, s.max_x, s.max_y};
}

void Element::SetScroll(int x, int y) { pincel_element_set_scroll(id, x, y); }

Element::Rect Element::GetRect() const {
	PincelRect r = pincel_element_get_rect(id);
	return {r.x, r.y, r.w, r.h};
}

Element& Element::SetSize(std::uint16_t width, std::uint16_t height) {
	pincel_element_set_size(id, width, height);
	return *this;
}

Element& Element::SetMargin(std::uint16_t base) {
	pincel_element_set_margin(id, base);
	return *this;
}

Element& Element::SetMargin(std::uint16_t top, std::uint16_t right, std::uint16_t bottom, std::uint16_t left) {
	pincel_element_set_margin_sides(id, top, right, bottom, left);
	return *this;
}

Element& Element::SetPadding(std::uint16_t base) {
	pincel_element_set_padding(id, base);
	return *this;
}

Element& Element::SetPadding(std::uint16_t top, std::uint16_t right, std::uint16_t bottom, std::uint16_t left) {
	pincel_element_set_padding_sides(id, top, right, bottom, left);
	return *this;
}

Element& Element::SetAlign(Align x, Align y) {
	pincel_element_set_align(id, static_cast<std::uint8_t>(x), static_cast<std::uint8_t>(y));
	return *this;
}

Element& Element::SetGap(std::uint16_t gap) {
	pincel_element_set_gap(id, gap);
	return *this;
}

Element& Element::SetBorder(std::uint16_t size, Color color) {
	pincel_element_set_border(id, size, ToC(color));
	return *this;
}

Element& Element::SetBackground(Color color) {
	pincel_element_set_background(id, ToC(color));
	return *this;
}

Element& Element::SetCornerRadius(std::uint16_t radius) {
	pincel_element_set_corner_radius(id, radius);
	return *this;
}

Element& Element::SetTransition(float seconds, Easing easing) {
	pincel_element_set_transition(id, seconds, static_cast<std::uint8_t>(easing));
	return *this;
}

Element& Element::SetForeground(Color color) {
	pincel_element_set_foreground(id, ToC(color));
	return *this;
}

Element& Element::SetFontSize(std::uint16_t size) {
	pincel_element_set_font_size(id, size);
	return *this;
}

Element& Element::SetFontFamily(FontFamily family) {
	pincel_element_set_font_family(id, family.id);
	return *this;
}

Element& Element::SetFontWeight(std::uint16_t weight) {
	pincel_element_set_font_weight(id, weight);
	return *this;
}

Element& Element::SetFontStyle(FontStyle style) {
	pincel_element_set_font_style(id, static_cast<std::uint8_t>(style));
	return *this;
}

Element& Element::SetTextShadow(Color color, int dx, int dy) {
	pincel_element_set_text_shadow(id, ToC(color), dx, dy);
	return *this;
}

Element& Element::InheritTextShadow() {
	pincel_element_inherit_text_shadow(id);
	return *this;
}

bool Element::SetText(const std::string& text) { return pincel_element_set_text(id, text.c_str()); }

std::string Element::GetText() const {
	std::size_t length = pincel_element_get_text(id, nullptr, 0);
	std::string text(length, '\0');
	if (length > 0) pincel_element_get_text(id, text.data(), length + 1);
	return text;
}

Element& Element::SetPlaceholder(const std::string& text) {
	pincel_element_set_placeholder(id, text.c_str());
	return *this;
}

Element& Element::SetMaxLength(std::uint32_t chars) {
	pincel_element_set_max_length(id, chars);
	return *this;
}

bool Element::InputChanged() const { return pincel_element_input_changed(id); }
bool Element::InputSubmitted() const { return pincel_element_input_submitted(id); }
bool Element::IsFocused() const { return pincel_element_is_focused(id); }
void Element::Focus() { pincel_element_focus(id); }
void Element::Blur() { pincel_element_blur(id); }

bool Element::SetImage(const std::uint8_t* rgba, std::uint32_t width, std::uint32_t height) {
	return pincel_element_set_image(id, rgba, width, height);
}

Element& Element::SetImageFilter(ImageFilter filter) {
	pincel_element_set_image_filter(id, static_cast<std::uint8_t>(filter));
	return *this;
}

Element& Element::SetPointerEvents(PointerEvents mode) {
	pincel_element_set_pointer_events(id, static_cast<std::uint8_t>(mode));
	return *this;
}

bool Element::IsHovered() const { return pincel_element_is_hovered(id); }
bool Element::IsPressed(std::uint8_t button) const { return (pincel_element_pressed_buttons(id) & button) != 0; }
bool Element::WasClicked(std::uint8_t button) const { return (pincel_element_clicked_buttons(id) & button) != 0; }
std::uint8_t Element::PressedButtons() const { return pincel_element_pressed_buttons(id); }
std::uint8_t Element::ClickedButtons() const { return pincel_element_clicked_buttons(id); }

// ---- Rich text

// Spans as the C side wants them. The strings are only borrowed from `spans`, which outlives the call.
static std::vector<PincelTextSpan> SpansToC(const std::vector<TextSpan>& spans) {
	std::vector<PincelTextSpan> out;
	out.reserve(spans.size());
	for (const TextSpan& s : spans) {
		PincelTextSpan c{};
		c.text = s.text.c_str();
		c.has_color = s.color.has_value();
		if (s.color) c.color = ToC(*s.color);
		c.font_weight = s.weight.value_or(0);
		c.font_style = s.style ? static_cast<std::int8_t>(*s.style) : static_cast<std::int8_t>(-1);
		c.underline = s.underline;
		c.strikethrough = s.strikethrough;
		out.push_back(c);
	}
	return out;
}

Element Element::AddRichText(const std::vector<TextSpan>& spans) {
	std::vector<PincelTextSpan> c = SpansToC(spans);
	return Element(pincel_element_add_rich_text(id, c.data(), c.size()));
}

bool Element::SetSpans(const std::vector<TextSpan>& spans) {
	std::vector<PincelTextSpan> c = SpansToC(spans);
	return pincel_element_set_spans(id, c.data(), c.size());
}

// ---- Selection

void Element::Select(std::size_t start, std::size_t end) {
	constexpr std::size_t most = std::numeric_limits<std::uint32_t>::max();
	pincel_element_select(id, static_cast<std::uint32_t>(start > most ? most : start), static_cast<std::uint32_t>(end > most ? most : end));
}

void Element::SelectAll() { Select(0, std::numeric_limits<std::uint32_t>::max()); }

std::optional<std::pair<std::size_t, std::size_t>> Element::GetSelection() const {
	std::uint32_t start = 0, end = 0;
	if (!pincel_element_get_selection(id, &start, &end)) return std::nullopt;
	return std::make_pair<std::size_t, std::size_t>(start, end);
}

std::string Element::GetSelectedText() const {
	std::size_t length = pincel_element_get_selected_text(id, nullptr, 0);
	std::string text(length, '\0');
	if (length > 0) pincel_element_get_selected_text(id, text.data(), length + 1);
	return text;
}

Element& Element::SetSelectionColor(Color color) {
	pincel_element_set_selection_color(id, ToC(color));
	return *this;
}

// ---- Input highlighting and wrapping

Element& Element::SetInputHighlights(const std::vector<InputHighlight>& runs) {
	constexpr std::size_t most = std::numeric_limits<std::uint32_t>::max();
	std::vector<PincelHighlight> c;
	c.reserve(runs.size());
	for (const InputHighlight& run : runs) {
		c.push_back({ static_cast<std::uint32_t>(run.start > most ? most : run.start), static_cast<std::uint32_t>(run.end > most ? most : run.end), ToC(run.color) });
	}
	pincel_element_set_highlights(id, c.data(), c.size());
	return *this;
}

Element& Element::SetInputWrap(bool wrap, const std::string& marker) {
	pincel_element_set_wrap(id, wrap, marker.c_str());
	return *this;
}

std::optional<Element::Rect> Element::GetInputPosition(std::size_t index) const {
	constexpr std::size_t most = std::numeric_limits<std::uint32_t>::max();
	std::int32_t x = 0, y = 0, height = 0;
	if (!pincel_element_get_input_position(id, static_cast<std::uint32_t>(index > most ? most : index), &x, &y, &height)) return std::nullopt;
	return Rect{ x, y, 0, height };
}

} // namespace Pincel
