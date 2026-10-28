#include <Pincel/Pincel.hpp>
#include "Window.hpp"
#include <algorithm>
#include <chrono>
#include <cmath>
#include <filesystem>
#include <string>
#include <vector>

using namespace Pincel;

/// A made-up RGBA8 image: a diagonal gradient, with a soft transparent hole to show alpha blending.
static std::vector<std::uint8_t> MakeTestImage(std::uint32_t size) {
	std::vector<std::uint8_t> pixels(static_cast<std::size_t>(size) * size * 4);
	for (std::uint32_t y = 0; y < size; y++) {
		for (std::uint32_t x = 0; x < size; x++) {
			float fx = static_cast<float>(x) / size, fy = static_cast<float>(y) / size;
			float dx = fx - 0.5f, dy = fy - 0.5f;
			float hole = std::sqrt(dx * dx + dy * dy) / 0.2f; // 0 in the middle, 1 at the edge of the hole.
			std::uint8_t* p = &pixels[(static_cast<std::size_t>(y) * size + x) * 4];
			p[0] = static_cast<std::uint8_t>(255 * fx);
			p[1] = static_cast<std::uint8_t>(255 * fy);
			p[2] = static_cast<std::uint8_t>(255 * (1.0f - fx));
			p[3] = static_cast<std::uint8_t>(255 * std::fmin(hole, 1.0f));
		}
	}
	return pixels;
}

/// An element whose background follows the mouse. Pincel tells us what the mouse did each frame, and we restyle.
struct Hoverable {
	Element element;
	Color normal;
	Color hover;
};

int main() {
	auto started = std::chrono::steady_clock::now();
	Pincel::Init();
	std::cout << "Pincel " << Pincel::Version() << std::endl;

	Window window;
	window.Init();

	Canvas* canvas = Pincel::GetCanvas();
	window.ConnectClipboard(canvas); // Ctrl+C, Ctrl+X and Ctrl+V in the inputs use the real clipboard.
	canvas->SetBackground(Color::Hex("#262626"));
	canvas->SetForeground(Color::Hex("#f7f7f7")); // Every Text below inherits this, so none of them needs its own color.

	Element root = canvas->Root();
	root.SetPadding(24);

	// A font family loaded from files (on Windows, Segoe UI), with the monospace family as its fallback for any
	// character it lacks. Without the files, text just uses the built-in sans-serif.
	std::optional<FontFamily> segoe;
	if (std::filesystem::exists("C:/Windows/Fonts/segoeui.ttf")) {
		segoe = LoadFontFamilyFiles("Segoe UI", "C:/Windows/Fonts/segoeui.ttf", "C:/Windows/Fonts/segoeuii.ttf",
			"C:/Windows/Fonts/segoeuib.ttf", "C:/Windows/Fonts/segoeuiz.ttf");
		if (segoe) SetFontFallback(*segoe, FontFamily::Mono);
	}

	std::vector<Hoverable> hoverables;
	auto hoverable = [&](Element element, Color normal, Color hover) {
		element.SetBackground(normal);
		element.SetTransition(0.18f, Easing::EaseOut); // Colors fade between normal and hover instead of jumping.
		hoverables.push_back({element, normal, hover});
	};

	Element status = root.AddText("").SetFontSize(13).SetMargin(8).SetForeground(Color::Hex("#a9a9a9"));

	// A sidebar pinned to the right edge of the window. It is Fixed, so it takes no room in the page and stays put
	// while the page scrolls underneath it.
	Element sidebar = root.AddStack();
	sidebar.SetPosition(Position::Fixed).SetInsets({.top = 0, .right = 0, .bottom = 0})
		.SetSize(240, 0).SetPadding(14).SetBackground(Color::Hex("#1b1b1b")).SetBorder(2, Color::Hex("#3a3a3a"))
		.SetOverflow(Overflow::Scroll); // Taller than the window, so it scrolls by itself, separately from the page.
	sidebar.AddText("Sidebar").SetFontSize(20).SetFontWeight(FontWeight::Bold).SetMargin(4);

	std::vector<Element> inputs;
	auto field = [&](const char* label, InputKind kind, const char* placeholder, std::uint16_t height) {
		sidebar.AddText(label).SetFontSize(13).SetForeground(Color::Hex("#a9a9a9")).SetMargin(4);
		Element input = sidebar.AddInput(kind);
		input.SetSize(210, height).SetMargin(4).SetPadding(6).SetBorder(2, Color::Hex("#555555"))
			.SetBackground(Color::Hex("#2b2b2b")).SetCornerRadius(6).SetPlaceholder(placeholder);
		inputs.push_back(input);
		return input;
	};
	Element nameInput = field("Name", InputKind::Text, "Type your name", 40);
	Element ageInput = field("Number", InputKind::Number, "Digits only", 40);
	Element notesInput = field("Notes (Enter makes a new line)", InputKind::Multiline, "Several lines...", 78);
	nameInput.SetMaxLength(24);
	Element greeting = sidebar.AddText("Hello!").SetMargin(6).SetFontWeight(FontWeight::Bold);

	// A buffer of pixels from "somewhere else", such as a game's render, drawn as a plain copy. SetImage() swaps in
	// each new frame.
	sidebar.AddText("Live buffer (nearest)").SetFontSize(13).SetForeground(Color::Hex("#a9a9a9")).SetMargin(4);
	std::vector<std::uint8_t> liveFrame(32 * 32 * 4, 255);
	Element liveImage = *sidebar.AddImage(liveFrame.data(), 32, 32);
	liveImage.SetImageFilter(ImageFilter::Nearest).SetSize(128, 0).SetMargin(4).SetCornerRadius(6);

	// A log that scrolls by itself and follows new lines, until you scroll up, like a chat window.
	sidebar.AddText("Log (follows new lines)").SetFontSize(13).SetForeground(Color::Hex("#a9a9a9")).SetMargin(4);
	Element log = sidebar.AddStack();
	log.SetSize(210, 100).SetMargin(4).SetPadding(6).SetBackground(Color::Hex("#222222")).SetCornerRadius(6)
		.SetOverflow(Overflow::Scroll).SetStickToEnd(true);
	int logLines = 0;
	auto lastLine = std::chrono::steady_clock::now();

	// A card. Its foreground, size and weight are set once and inherited by the text inside.
	Element card = root.AddStack();
	card.SetSize(380, 0).SetPadding(14).SetMargin(8).SetBorder(3, Color::Hex("#f7f7f7")).SetBackground(Color::Hex("#0060cb")).SetCornerRadius(16);
	// Absolute: placed against the card's padding box, hanging over its top right corner. It takes no room in the card.
	Element badge = card.AddStack();
	badge.SetPosition(Position::Absolute).SetInsets({.top = -14, .right = 16}).SetPadding(5).SetCornerRadius(8)
		.SetBackground(Color::Hex("#ffb510"));
	badge.AddText("NEW").SetFontSize(12).SetFontWeight(FontWeight::Bold).SetForeground(Color::Hex("#262626"));

	card.AddText("Hello, World!").SetFontSize(26).SetFontWeight(FontWeight::Bold)
		.SetTextShadow(Color::Hex("#000000", 0.6f), 2, 2); // A hard shadow, only drawn because this has one.
	if (segoe) {
		card.AddText("Segoe UI, loaded from a file, with a fallback: █ → ✓").SetFontFamily(*segoe);
	}
	card.AddText("This card sets no color on its text. It inherits the canvas's, and a container can override it for everything inside.");
	card.AddText("Italic, for emphasis.").SetFontStyle(FontStyle::Italic);
	card.AddText("Bold and italic together.").SetFontWeight(FontWeight::Bold).SetFontStyle(FontStyle::Italic);

	Element accent = card.AddStack(); // Everything in here is yellow and monospace, from properties set on this one element.
	accent.SetMargin(4).SetForeground(Color::Hex("#ffd866")).SetFontFamily(FontFamily::Mono).SetFontSize(14);
	accent.AddText("inherited: yellow, monospace");
	accent.AddText("and this is bold").SetFontWeight(FontWeight::Bold);

	// A button: hover and click come from Pincel, and we decide what they mean.
	Element button = root.AddStack();
	button.SetPadding(12).SetMargin(8).SetCornerRadius(10);
	hoverable(button, Color::Hex("#2f7d32"), Color::Hex("#43a047"));
	Element buttonLabel = button.AddText("Click me").SetFontWeight(FontWeight::Bold);
	int clicks = 0;

	// Rich text: spans with their own looks that wrap together as one line, the way a chat message does.
	root.AddText("Rich text wraps across its colors").SetMargin(8).SetFontWeight(FontWeight::Bold);
	Element chat = root.AddStack();
	chat.SetSize(380, 0).SetMargin(8).SetPadding(8).SetCornerRadius(8).SetBackground(Color::Hex("#000000", 0.35f));
	chat.AddRichText({
		TextSpan{"<Steve> "},
		TextSpan{.text = "This message is in a few colors, ", .color = Color::Hex("#ffffff")},
		TextSpan{.text = "bold red", .color = Color::Hex("#ff5555"), .weight = FontWeight::Bold},
		TextSpan{" then plain, "},
		TextSpan{.text = "italic green", .color = Color::Hex("#55ff55"), .style = FontStyle::Italic},
		TextSpan{", then "},
		TextSpan{.text = "underlined across the wrap", .color = Color::Hex("#55aaff"), .underline = true},
		TextSpan{", and "},
		TextSpan{.text = "struck out", .color = Color::Hex("#aaaaaa"), .strikethrough = true},
		TextSpan{". A word split between two Hel"},
		TextSpan{.text = "lo", .color = Color::Hex("#ffaa00")},
		TextSpan{" spans never breaks between colors."},
	}).SetFontSize(15);

	// Boxes with different radii.
	Element row = root.AddStack(Orientation::Horizontal);
	hoverable(row.AddStack().SetSize(80, 80).SetMargin(8).SetCornerRadius(8), Color::Hex("#2282ed"), Color::Hex("#6fb0ff"));
	hoverable(row.AddStack().SetSize(80, 80).SetMargin(8).SetCornerRadius(24), Color::Hex("#8e8e8e"), Color::Hex("#c4c4c4"));
	row.AddStack().SetSize(80, 80).SetMargin(8).SetBorder(3, Color::Hex("#a9a9a9")).SetCornerRadius(40);

	std::vector<std::uint8_t> pixels = MakeTestImage(64);
	if (auto image = root.AddImage(pixels.data(), 64, 64)) {
		image->SetMargin(8).SetCornerRadius(16);
	}

	// A list that scrolls by itself, separately from the page: the wheel over it scrolls the list, and the page gets
	// it when the list reaches its end. Rows can be added, and clicking one removes it.
	root.AddText("A list with its own scrolling. Left-click removes a row, right-click marks it.").SetMargin(8).SetFontWeight(FontWeight::Bold);
	Element controls = root.AddStack(Orientation::Horizontal);
	Element addButton = controls.AddStack();
	addButton.SetPadding(10).SetMargin(4).SetCornerRadius(8);
	hoverable(addButton, Color::Hex("#2f7d32"), Color::Hex("#43a047"));
	addButton.AddText("Add a row").SetFontWeight(FontWeight::Bold);
	Element clearButton = controls.AddStack();
	clearButton.SetPadding(10).SetMargin(4).SetCornerRadius(8);
	hoverable(clearButton, Color::Hex("#a33a3a"), Color::Hex("#c75050"));
	clearButton.AddText("Remove all").SetFontWeight(FontWeight::Bold);

	Element list = root.AddStack();
	list.SetSize(400, 190).SetMargin(8).SetPadding(6).SetBorder(2, Color::Hex("#3a3a3a")).SetCornerRadius(10)
		.SetBackground(Color::Hex("#1f1f1f")).SetOverflow(Overflow::Scroll);
	std::vector<Element> rows;
	int rowNumber = 0;
	auto addRow = [&] {
		Element item = list.AddStack();
		item.SetSize(370, 0).SetPadding(8).SetMargin(2).SetCornerRadius(6);
		hoverable(item, Color::Hex("#333333"), Color::Hex("#4a4a4a"));
		item.AddText("Row " + std::to_string(++rowNumber));
		rows.push_back(item);
	};
	for (int i = 0; i < 20; i++) addRow();

	root.AddText("The page itself scrolls too, when the wheel isn't over a list.").SetMargin(8).SetFontSize(13).SetForeground(Color::Hex("#a9a9a9"));
	for (int i = 1; i <= 12; i++) {
		root.AddStack().SetSize(400, 44).SetMargin(3).SetCornerRadius(8).SetBackground(Color::Hex("#2b2b2b"));
	}

	bool firstFrame = true;
	while (!window.ShouldClose()) {
		window.Render(canvas);
		if (firstFrame) {
			firstFrame = false;
			std::cout << "First frame drawn " << std::chrono::duration_cast<std::chrono::milliseconds>(std::chrono::steady_clock::now() - started).count()
				<< " ms after main() started" << std::endl;
		}

		// What Pincel found out during that Draw().
		for (Hoverable& h : hoverables) {
			h.element.SetBackground(h.element.IsHovered() ? h.hover : h.normal);
		}
		// Rows come and go, and handles to removed ones are safely invalid, so there is nothing to track by hand.
		if (addButton.WasClicked()) addRow();
		if (clearButton.WasClicked()) {
			list.RemoveChildren();
			rows.clear();
		}
		for (Element& rowElement : rows) {
			// Pincel says which element each button was clicked on. What it means is up to the program.
			if (rowElement.WasClicked(MouseButton::Left)) rowElement.Remove();
			else if (rowElement.WasClicked(MouseButton::Right)) rowElement.SetBorder(2, Color::Hex("#fbb510"));
		}
		std::erase_if(rows, [](const Element& e) { return !e.IsValid(); });
		std::erase_if(hoverables, [](const Hoverable& h) { return !h.element.IsValid(); });

		// A line in the log every second and a half.
		if (std::chrono::steady_clock::now() - lastLine > std::chrono::milliseconds(1500)) {
			lastLine = std::chrono::steady_clock::now();
			log.AddText("Line " + std::to_string(++logLines)).SetFontSize(13);
		}

		if (button.WasClicked()) {
			clicks++;
			buttonLabel.SetText("Clicked " + std::to_string(clicks) + (clicks == 1 ? " time" : " times"));
		}
		// Inputs: highlight the focused one, and react to what was typed.
		for (Element& input : inputs) {
			input.SetBorder(2, input.IsFocused() ? Color::Hex("#fbb510") : Color::Hex("#555555"));
		}
		if (nameInput.InputChanged() || nameInput.InputSubmitted()) {
			std::string name = nameInput.GetText();
			greeting.SetText(name.empty() ? "Hello!" : "Hello, " + name + "!");
		}

		// A new frame of the live buffer ten times a second: a moving diagonal pattern.
		static auto lastTick = std::chrono::steady_clock::now();
		static int tick = 0;
		if (std::chrono::steady_clock::now() - lastTick > std::chrono::milliseconds(100)) {
			lastTick = std::chrono::steady_clock::now();
			tick++;
			for (int y = 0; y < 32; y++) {
				for (int x = 0; x < 32; x++) {
					std::uint8_t* p = &liveFrame[(static_cast<std::size_t>(y) * 32 + x) * 4];
					bool on = ((x + y + tick) / 4) % 2 == 0;
					p[0] = on ? 80 : 20; p[1] = on ? 200 : 60; p[2] = on ? 120 : 40; p[3] = 255;
				}
			}
			liveImage.SetImage(liveFrame.data(), 32, 32);
		}

		status.SetText("Keyboard: " + std::string(canvas->WantsKeyboard() ? "Pincel's" : "free") + "   Mouse: " + std::string(canvas->WantsMouse() ? "Pincel's" : "free") + "   Scrolled " + std::to_string(static_cast<int>(canvas->GetScroll())) + " of " +
			std::to_string(std::max(0, canvas->ContentHeight() - static_cast<int>(window.FramebufferHeight()))) + " px");
	}
	return 0;
}
