//! Pincel core. The C++ API in `src/pincel/cpp` wraps the C ABI exposed by [`ffi`].
//!
//! Each frame: the element tree is turned into draw commands by [`layout`], and [`raster`] draws them into a texture.

pub mod anim;
pub mod canvas;
pub mod clipboard;
pub mod dirty;
pub mod draw_cmd;
pub mod element;
pub mod ffi;
pub mod font;
pub mod input;
pub mod layout;
pub mod raster;
pub mod rich;
pub(crate) mod registry;
pub mod tree;
pub mod style;
pub mod text_input;
pub mod text;

/// Version of the Pincel core, taken from Cargo.toml.
pub const VERSION: &str = concat!(env!("CARGO_PKG_VERSION"), "\0");
