#pragma once
#ifndef PINCEL_FONT
#define PINCEL_FONT

#include <cstdint>
#include <optional>
#include <span>
#include <string>
#include <Pincel/Export.hpp>
#include <Pincel/Style.hpp>

namespace Pincel {

/// Font files in memory (TrueType or OpenType): regular, italic, bold and bold italic. Only regular is needed.
/// A style the family has no face for is drawn with the closest one it does have, so a family with no bold face
/// draws bold text in regular, rather than a made-up bold.
struct FontFaces {
	std::span<const std::uint8_t> regular;
	std::span<const std::uint8_t> italic;
	std::span<const std::uint8_t> bold;
	std::span<const std::uint8_t> boldItalic;
};

/// Adds a font family and returns it, to give to Element::SetFontFamily(). The data is copied. Families are shared by
/// every canvas, and are never removed. The name has to be new (ignoring case): the built-in families are "Sans" and
/// "Mono". Returns nothing if a face isn't a usable font or the name is taken; see Pincel::LastError().
PINCEL_API std::optional<FontFamily> LoadFontFamily(const std::string& name, const FontFaces& faces);

/// The same, reading the faces from files. An empty path means the family has no such face.
PINCEL_API std::optional<FontFamily> LoadFontFamilyFiles(const std::string& name, const std::string& regularPath,
	const std::string& italicPath = "", const std::string& boldPath = "", const std::string& boldItalicPath = "");

/// The family with this name, ignoring case.
PINCEL_API std::optional<FontFamily> FindFontFamily(const std::string& name);

/// Says to draw characters `family` has no glyph for from `fallback` instead, such as symbols and emoji that a pixel
/// font lacks. Pass nothing to remove it. Fallbacks can chain a few deep. Returns false if either family is unknown.
PINCEL_API bool SetFontFallback(FontFamily family, std::optional<FontFamily> fallback);

} // namespace Pincel

#endif
