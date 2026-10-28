#pragma once
#ifndef PINCEL
#define PINCEL

#include <cstdint>
#include <cstdio>
#include <cstdlib>
#include <string_view>
#include "Export.hpp"
#include "Canvas.hpp"
#include "Font.hpp"

namespace Pincel {

/// The message from the last call on this thread that failed, such as Element::AddImageFile() returning nothing.
PINCEL_API std::string_view LastError();

/// Like Init(), but says whether the library matches `headerVersion` (pass PINCEL_ABI_VERSION) instead of stopping.
/// Returns false, and sets LastError(), if it doesn't, and then nothing else may be called.
PINCEL_API bool InitChecked(std::uint32_t headerVersion);

PINCEL_API bool IsInitialized();
PINCEL_API std::string_view Version();
PINCEL_API Canvas* GetCanvas();

/// Starts Pincel. Call it before anything else.
///
/// It checks that the library the program is running with (pincel.dll) is the version these headers are for, and if
/// not, prints why and stops the program: calling into a DLL that was built from different headers would corrupt
/// memory. If you rebuilt the headers' side but not the DLL (or the other way round), rebuild the other to match.
/// This is compiled into the program, not the DLL, so it sees the version of the headers the program really has.
inline void Init() {
	if (!InitChecked(PINCEL_ABI_VERSION)) {
		std::string_view why = LastError();
		std::fprintf(stderr, "Pincel: %.*s\n", static_cast<int>(why.size()), why.data());
		std::abort();
	}
}

} // namespace Pincel

#endif
