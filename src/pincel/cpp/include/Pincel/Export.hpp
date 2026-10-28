#pragma once
#ifndef PINCEL_EXPORT
#define PINCEL_EXPORT

// PINCEL_API marks what pincel.dll exports to the programs that use it. It only does anything when Pincel is built as a
// shared library (PINCEL_SHARED, which CMake defines for you): the library itself defines PINCEL_BUILDING and exports,
// and everything that includes the headers imports. When Pincel is linked statically it is nothing.
#if defined(_WIN32) && defined(PINCEL_SHARED)
	#ifdef PINCEL_BUILDING
		#define PINCEL_API __declspec(dllexport)
	#else
		#define PINCEL_API __declspec(dllimport)
	#endif
#elif defined(PINCEL_SHARED)
	#define PINCEL_API __attribute__((visibility("default")))
#else
	#define PINCEL_API
#endif

#endif
