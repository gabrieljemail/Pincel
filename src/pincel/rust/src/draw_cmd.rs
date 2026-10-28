use std::sync::Arc;

use crate::element::Image;
use crate::font::FontKey;
use crate::style::{Color, ImageFilter, TextShadow};

/// A rectangle in pixels. The origin is the top-left of the texture.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Rect {
    /// The overlap of two rectangles, which has no area (but is a valid rect) if they don't overlap.
    pub fn intersect(self, other: Rect) -> Rect {
        let x0 = self.x.max(other.x);
        let y0 = self.y.max(other.y);
        let x1 = (self.x + self.w).min(other.x + other.w);
        let y1 = (self.y + self.h).min(other.y + other.h);
        Rect { x: x0, y: y0, w: (x1 - x0).max(0), h: (y1 - y0).max(0) }
    }

    /// Whether the point is inside, with the left and top edges included and the right and bottom excluded.
    /// NaN is never inside.
    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x as f32 && x < (self.x + self.w) as f32 && y >= self.y as f32 && y < (self.y + self.h) as f32
    }
}

/// A rasterizer drawing command. Layout produces these, and the rasterizer consumes them in order.
#[derive(Clone, Debug, PartialEq)]
pub enum DrawCmd {
    /// A filled rectangle with an optional border drawn inside it.
    Box {
        rect: Rect,
        background: Color,
        /// `[top, right, bottom, left]` in pixels.
        border: [i32; 4],
        border_color: Color,
        /// Corner radius in pixels, already clamped to fit the rect.
        radius: i32,
        /// If set, nothing is drawn outside this rect, such as the box of a scrolling container the element is in.
        clip: Option<Rect>,
    },
    /// One line of text. `(x, y)` is the top-left of the line's box, whose height is the font's line height.
    Text {
        x: i32,
        y: i32,
        text: String,
        font: FontKey,
        font_size: u16,
        color: Color,
        /// If set, nothing is drawn outside this rect. Used by text that can overflow its box, and by anything inside a
        /// container that clips or scrolls.
        clip: Option<Rect>,
        /// `None` when there is no shadow, so there is nothing to check or draw.
        shadow: Option<TextShadow>,
    },
    /// An image stretched to fill `rect`, with its corners rounded by `radius`.
    Image {
        rect: Rect,
        image: Arc<Image>,
        radius: i32,
        filter: ImageFilter,
        clip: Option<Rect>,
    },
    // TODO: Shader.
    // Oh, and if you didn't already know, Shader just passes on an empty layout-calculated Image and relies on the application using Pincel to execute the shader.
    // When the application is done with the shader part, it just passes back the texture buffer the shader rendered into to be composited into the final texture.
    // The point of having people use Pincel to wrap this is layout calculation and integration. It would be hard to render a shader overlaying a black box texture.
}
