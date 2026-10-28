#include <Pincel/Pincel.hpp>
#include <Pincel/ffi.h>

namespace Pincel {

bool InitChecked(std::uint32_t headerVersion) { return pincel_init_checked(headerVersion); }
bool IsInitialized() { return pincel_is_initialized(); }
std::string_view Version() { return pincel_version(); }

std::string_view LastError() { return pincel_last_error(); }

Canvas* GetCanvas() {
	static Canvas canvas;
	return &canvas;
}

} // namespace Pincel
