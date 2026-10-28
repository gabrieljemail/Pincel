#include <Pincel/Font.hpp>
#include <Pincel/ffi.h>

namespace Pincel {

static std::optional<FontFamily> ToFamily(std::int32_t id) {
	if (id < 0) return std::nullopt;
	return FontFamily{static_cast<std::uint16_t>(id)};
}

std::optional<FontFamily> LoadFontFamily(const std::string& name, const FontFaces& faces) {
	// An empty span means no face, which is NULL on the C side.
	auto data = [](std::span<const std::uint8_t> face) { return face.empty() ? nullptr : face.data(); };
	PincelFontFaces c{
		data(faces.regular), faces.regular.size(),
		data(faces.italic), faces.italic.size(),
		data(faces.bold), faces.bold.size(),
		data(faces.boldItalic), faces.boldItalic.size(),
	};
	return ToFamily(pincel_font_register(name.c_str(), &c));
}

std::optional<FontFamily> LoadFontFamilyFiles(const std::string& name, const std::string& regularPath,
	const std::string& italicPath, const std::string& boldPath, const std::string& boldItalicPath) {
	auto path = [](const std::string& p) { return p.empty() ? nullptr : p.c_str(); };
	return ToFamily(pincel_font_register_files(name.c_str(), path(regularPath), path(italicPath), path(boldPath), path(boldItalicPath)));
}

std::optional<FontFamily> FindFontFamily(const std::string& name) {
	return ToFamily(pincel_font_find(name.c_str()));
}

bool SetFontFallback(FontFamily family, std::optional<FontFamily> fallback) {
	return pincel_font_set_fallback(family.id, fallback ? fallback->id : -1);
}

} // namespace Pincel
