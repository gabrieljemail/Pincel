#pragma once
#ifndef PINCEL_TEXTURE
#define PINCEL_TEXTURE

#include <cstddef>
#include <cstdint>

namespace Pincel {

/// A view of the pixels Pincel rasterized. Upload `data` to the GPU however your graphics backend likes.
/// The pixels are RGBA8 with premultiplied alpha, tightly packed, and row 0 is the top of the image.
/// `data` is owned by the Canvas and is only valid until the next Canvas::Draw() or until the Canvas is destroyed.
class Texture {
public:
	std::uint32_t width = 0;
	std::uint32_t height = 0;
	const std::uint8_t* data = nullptr;
	/// Goes up each time Pincel redraws the pixels, and stays the same on frames where nothing changed. Compare it
	/// with the version you last uploaded to skip uploading (and everything after it) when there is nothing new.
	/// Starts at 1 for the first drawn frame, so 0 can mean "nothing uploaded yet".
	std::uint64_t version = 0;

	/// Size of `data` in bytes.
	std::size_t Size() const { return static_cast<std::size_t>(width) * height * 4; }
};

} // namespace Pincel

#endif
