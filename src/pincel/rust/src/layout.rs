//! Turns the element tree into draw commands.
//!
//! The box model, outermost to innermost: margin, border, padding, content.
//! `style.size` is the border box (border + padding + content). When it is 0 the size comes from the children.
//!
//! Every function takes `avail_w`, the width the element may use. Parents give their children the width left
//! in their content box, which is what text wraps to and what oversized images shrink to.
//!
//! They also take `inherited`, the text properties of the parent. An element resolves its own on top of those
//! (see [`Style::text_style`]) and passes the result to its children.

use crate::draw_cmd::{DrawCmd, Rect};
use crate::element::{Element, ElementKind, Image, Orientation};
use crate::font::{FontKey, Fonts, LineMetrics};
use crate::rich::{layout_rich, RichLayout};
use crate::style::{Color, Overflow, Position, TextStyle};
use crate::text::wrap;
use crate::text_input::InputKind;

/// What layout needs to know beyond the element it is laying out: the canvas, for elements placed against it,
/// and where their draw commands go (they must not scroll, so they are kept apart from the rest).
pub struct View<'a> {
    /// The visible canvas, from (0, 0).
    pub viewport: Rect,
    /// Draw commands for `Fixed` elements, which the canvas draws after (over) everything else.
    pub fixed: &'a mut Vec<DrawCmd>,
    /// Set when something was placed against the viewport, so the layout depends on the canvas's height too.
    pub uses_viewport: bool,
    /// Inside a `Fixed` element, where another one is just `Absolute` to it, since the whole thing is already pinned.
    in_fixed: bool,
    /// What the element being laid out has to stay inside, because it is in something that clips or scrolls.
    /// Every draw command made now is given it.
    pub clip: Option<Rect>,
}

impl<'a> View<'a> {
    pub fn new(viewport: Rect, fixed: &'a mut Vec<DrawCmd>) -> View<'a> {
        View { viewport, fixed, uses_viewport: false, in_fixed: false, clip: None }
    }
}

/// How wide an Input's caret is, in pixels, and the least width (in digits) an Input is without a size of its own.
const INPUT_CARET_W: i32 = 2;
const INPUT_MIN_CHARS: f32 = 12.0;

fn px(values: [u16; 4]) -> [i32; 4] {
    values.map(i32::from)
}

/// One row of an Input as it is drawn: characters `start..start + len` of the value.
struct InputRow {
    start: usize,
    len: usize,
    /// It carries on a line that was too long for one row, so it is indented.
    continued: bool,
}

/// How far the rows after the first of a wrapped line are indented: room for the wrap marker and a space after it.
fn wrap_indent(marker: &str, fonts: &mut Fonts, key: FontKey, size: u16) -> f32 {
    if marker.is_empty() {
        0.0
    } else {
        fonts.text_width(key, size, marker) + fonts.text_width(key, size, " ")
    }
}

/// Splits an Input's value into the rows it is drawn in, and gives where each character starts on its row (measured
/// from the left of the content box, so a row's indent is in it). A row is a line of the value, or with `wrap_width`
/// a piece of one that fits: it breaks after the last space that fits, or inside a word that is wider than a row.
/// `boundaries` gives where each character of a line starts, as `Fonts::boundaries` does.
fn input_rows(value: &str, wrap_width: Option<f32>, indent: f32, boundaries: &mut dyn FnMut(&str) -> Vec<f32>) -> (Vec<InputRow>, Vec<Vec<f32>>) {
    let mut rows = Vec::new();
    let mut columns = Vec::new();
    let mut line_start = 0;
    for line in value.split('\n') {
        let chars: Vec<char> = line.chars().collect();
        let n = chars.len();
        let b = boundaries(line);
        let mut start = 0;
        loop {
            let continued = start > 0;
            let lead = if continued { indent } else { 0.0 };
            let mut end = n;
            if let Some(width) = wrap_width {
                let limit = width - lead;
                if b[n] - b[start] > limit {
                    // As many characters as fit (at least one), then back to just after the last space among them.
                    end = (start + 1..=n).take_while(|&i| b[i] - b[start] <= limit).last().unwrap_or(start + 1);
                    if let Some(after_space) = (start + 1..=end).rev().find(|&k| chars[k - 1] == ' ') {
                        end = after_space;
                    }
                }
            }
            rows.push(InputRow { start: line_start + start, len: end - start, continued });
            columns.push((start..=end).map(|i| lead + b[i] - b[start]).collect());
            if end == n {
                break;
            }
            start = end;
        }
        line_start += n + 1;
    }
    (rows, columns)
}

/// The width a child of a Stack may use. In a horizontal stack, earlier siblings have already taken some.
fn child_avail(orientation: Orientation, content_avail: i32, used_along: i32) -> i32 {
    match orientation {
        Orientation::Vertical => content_avail,
        Orientation::Horizontal => (content_avail - used_along).max(0),
    }
}

/// Wraps text to the content width. Returns the lines and the line metrics they are laid out with.
fn text_lines(text: &str, style: &TextStyle, content_avail: i32, fonts: &mut Fonts) -> (Vec<String>, LineMetrics) {
    let (key, size) = (font_key(style), style.font_size);
    let metrics = fonts.line_metrics(key, size);
    let space_w = fonts.text_width(key, size, " ");
    let lines = wrap(text, content_avail as f32, |word| fonts.text_width(key, size, word), space_w);
    (lines, metrics)
}

fn font_key(style: &TextStyle) -> FontKey {
    FontKey::new(style.font_family, style.font_weight, style.font_style)
}

/// `value * num / den`, rounded, and at least 1. Used to keep an image's aspect ratio.
fn scaled(value: i32, num: u32, den: u32) -> i32 {
    ((i64::from(value) * i64::from(num) + i64::from(den) / 2) / i64::from(den)).max(1) as i32
}

/// The size an image is drawn at inside the content box.
/// `explicit_w` and `explicit_h` are the content box dimensions the style asks for, with 0 meaning unset.
/// - Both set: the image is stretched to fit.
/// - One set: the other follows from the image's aspect ratio.
/// - Neither: the image's own size, shrunk (keeping its aspect ratio) if it is wider than `content_avail`.
fn image_size(image: &Image, explicit_w: i32, explicit_h: i32, content_avail: i32) -> (i32, i32) {
    match (explicit_w > 0, explicit_h > 0) {
        (true, true) => (explicit_w, explicit_h),
        (true, false) => (explicit_w, scaled(explicit_w, image.height, image.width)),
        (false, true) => (scaled(explicit_h, image.width, image.height), explicit_h),
        (false, false) => {
            if i64::from(image.width) > i64::from(content_avail) {
                (content_avail, scaled(content_avail, image.height, image.width))
            } else {
                (image.width as i32, image.height as i32)
            }
        }
    }
}

/// How big a stack's children are all together, ignoring any size the stack has itself: the width and height of the
/// content that is there to see. Only children in the flow count, and `content_avail` is the width they may use.
fn stack_content_size(el: &Element, orientation: Orientation, content_avail: i32, text_style: &TextStyle, fonts: &mut Fonts) -> (i32, i32) {
    let gap = i32::from(el.style.gap);
    let (mut along, mut across) = (0, 0);
    for (i, child) in el.children.iter().filter(|c| c.style.visible && c.style.position == Position::Flow).enumerate() {
        if i > 0 {
            along += gap;
        }
        let (w, h) = outer_size(child, child_avail(orientation, content_avail, along), text_style, fonts);
        let (main, cross) = match orientation {
            Orientation::Vertical => (h, w),
            Orientation::Horizontal => (w, h),
        };
        along += main;
        across = across.max(cross);
    }
    match orientation {
        Orientation::Vertical => (across, along),
        Orientation::Horizontal => (along, across),
    }
}

/// Size of the element's border box, given the width it may use (excluding its margin).
fn border_box_size(el: &Element, avail_w: i32, inherited: &TextStyle, fonts: &mut Fonts) -> (i32, i32) {
    let text_style = el.text_style(inherited);
    let border = px(el.style.border_size.resolve());
    let padding = px(el.style.padding.resolve());
    let chrome_w = border[1] + border[3] + padding[1] + padding[3];
    let chrome_h = border[0] + border[2] + padding[0] + padding[2];

    let explicit_w = i32::from(el.style.size.width());
    let explicit_h = i32::from(el.style.size.height());
    let content_avail = (if explicit_w > 0 { explicit_w } else { avail_w } - chrome_w).max(0);

    let (content_w, content_h) = match &el.kind {
        // Only children in the flow take room. Placed ones are laid out separately.
        ElementKind::Stack(orientation) => stack_content_size(el, *orientation, content_avail, &text_style, fonts),
        ElementKind::Text(text) => {
            let (lines, metrics) = text_lines(text, &text_style, content_avail, fonts);
            let (key, size) = (font_key(&text_style), text_style.font_size);
            let widest = lines.iter().map(|line| fonts.text_width(key, size, line)).fold(0.0, f32::max);
            (widest.ceil() as i32, lines.len() as i32 * metrics.height)
        }
        ElementKind::Rich(spans) => {
            let rich = layout_rich(spans, &text_style, content_avail as f32, fonts);
            (rich.widest().ceil() as i32, rich.lines.len() as i32 * rich.line_height)
        }
        ElementKind::Input(state) => {
            let (key, size) = (font_key(&text_style), text_style.font_size);
            let metrics = fonts.line_metrics(key, size);
            let shown = if state.value.is_empty() { state.placeholder.as_str() } else { state.value.as_str() };
            let widest = shown.split('\n').map(|line| fonts.text_width(key, size, line)).fold(0.0, f32::max);
            // Wide enough for a dozen digits at least, so an empty input isn't a sliver.
            let min_w = (INPUT_MIN_CHARS * fonts.text_width(key, size, "0")).ceil() as i32;
            if state.wrap {
                // As wide as the text, up to the room there is, and as many rows as that takes.
                let indent = wrap_indent(&state.wrap_marker, fonts, key, size);
                let wrap_width = (content_avail - INPUT_CARET_W).max(1) as f32;
                let (rows, _) = input_rows(&state.value, Some(wrap_width), indent, &mut |line| fonts.boundaries(key, size, line));
                ((widest.ceil() as i32 + INPUT_CARET_W).min(content_avail).max(min_w), rows.len() as i32 * metrics.height)
            } else {
                let rows = if state.kind == InputKind::Multiline { state.value.split('\n').count() as i32 } else { 1 };
                ((widest.ceil() as i32 + INPUT_CARET_W).max(min_w), rows * metrics.height)
            }
        }
        ElementKind::Image(image) => {
            let explicit_cw = if explicit_w > 0 { (explicit_w - chrome_w).max(0) } else { 0 };
            let explicit_ch = if explicit_h > 0 { (explicit_h - chrome_h).max(0) } else { 0 };
            image_size(image, explicit_cw, explicit_ch, content_avail)
        }
    };

    let w = if explicit_w > 0 { explicit_w } else { content_w + chrome_w };
    let h = if explicit_h > 0 { explicit_h } else { content_h + chrome_h };
    (w, h)
}

/// Size of the element including its margin, which is the space it takes up in its parent.
pub fn outer_size(el: &Element, avail_w: i32, inherited: &TextStyle, fonts: &mut Fonts) -> (i32, i32) {
    let margin = px(el.style.margin.resolve());
    let (w, h) = border_box_size(el, (avail_w - margin[1] - margin[3]).max(0), inherited, fonts);
    (w + margin[1] + margin[3], h + margin[0] + margin[2])
}

/// Draws laid-out rich text with its top-left at `(x, y)`: each piece in its span's font and color, lined up on a
/// shared baseline, and underlines and strikethroughs as thin boxes in the span's color.
fn emit_rich(rich: &RichLayout, x: i32, y: i32, text_style: &TextStyle, clip: Option<Rect>, out: &mut Vec<DrawCmd>) {
    let size = text_style.font_size;
    let shadow = text_style.text_shadow.is_visible().then_some(text_style.text_shadow);
    // A line this thick, so decorations stay visible as the text grows.
    let thickness = (i32::from(size) / 14).max(1);

    for (row, line) in rich.lines.iter().enumerate() {
        let top = y + row as i32 * rich.line_height;
        let baseline = top + rich.ascent;
        for piece in &line.pieces {
            let style = &rich.styles[piece.span];
            let left = x + piece.x.round() as i32;
            let width = (piece.x + piece.width).round() as i32 - piece.x.round() as i32;

            if let Some(text) = &piece.text {
                out.push(DrawCmd::Text {
                    x: left,
                    // Pieces in a face with a smaller ascent are moved down to meet the line's baseline.
                    y: top + rich.ascent - style.ascent,
                    text: text.clone(),
                    font: style.font,
                    font_size: size,
                    color: style.color,
                    clip,
                    shadow,
                });
            }
            let mut decorate = |offset: i32| {
                out.push(DrawCmd::Box {
                    rect: Rect { x: left, y: baseline + offset, w: width, h: thickness },
                    background: style.color,
                    border: [0; 4],
                    border_color: Color::TRANSPARENT,
                    radius: 0,
                    clip,
                });
            };
            if style.underline {
                decorate(thickness); // Just under the baseline.
            }
            if style.strikethrough {
                decorate(-(i32::from(size) / 4)); // Through the middle of the lowercase letters.
            }
        }
    }
}

/// Lays out an element that is not in its parent's flow (`Absolute` or `Fixed`), against `container`
/// (or the canvas, for `Fixed`). See [`Insets`](crate::style::Insets) for the rules.
fn place_out_of_flow(el: &mut Element, container: Rect, inherited: &TextStyle, fonts: &mut Fonts, out: &mut Vec<DrawCmd>, view: &mut View) {
    let pinned = el.style.position == Position::Fixed && !view.in_fixed;
    let container = if pinned {
        view.uses_viewport = true;
        view.viewport
    } else {
        container
    };

    let inset = el.style.inset;
    let [margin_top, margin_right, margin_bottom, margin_left] = px(el.style.margin.resolve());
    let (left, top) = (inset.left.filter(|_| !inset.center_x), inset.top.filter(|_| !inset.center_y));
    let (right, bottom) = (inset.right.filter(|_| !inset.center_x), inset.bottom.filter(|_| !inset.center_y));

    // The room the element has, which is what text in it wraps to.
    let room_w = (container.w - left.unwrap_or(0) - right.unwrap_or(0) - margin_left - margin_right).max(0);
    let room_h = (container.h - top.unwrap_or(0) - bottom.unwrap_or(0) - margin_top - margin_bottom).max(0);
    let (mut w, mut h) = border_box_size(el, room_w, inherited, fonts);

    // Both sides of an axis with no size on it: stretch between them. The size is set for the layout below
    // and put back after, because it isn't something the user asked for.
    let saved_size = el.style.size;
    if el.style.size.width() == 0 && left.is_some() && right.is_some() {
        w = room_w;
        el.style.size.x = Some(w.min(i32::from(u16::MAX)) as u16);
    }
    if el.style.size.height() == 0 && top.is_some() && bottom.is_some() {
        h = room_h;
        el.style.size.y = Some(h.min(i32::from(u16::MAX)) as u16);
    }

    let x = if inset.center_x {
        container.x + (container.w - w - margin_left - margin_right) / 2 + margin_left
    } else if let Some(left) = left {
        container.x + left + margin_left
    } else if let Some(right) = right {
        container.x + container.w - right - margin_right - w
    } else {
        container.x + margin_left
    };
    let y = if inset.center_y {
        container.y + (container.h - h - margin_top - margin_bottom) / 2 + margin_top
    } else if let Some(top) = top {
        container.y + top + margin_top
    } else if let Some(bottom) = bottom {
        container.y + container.h - bottom - margin_bottom - h
    } else {
        container.y + margin_top
    };

    // `layout` wants the corner of the margin box, and the width that box may use.
    if pinned {
        // Fixed elements don't scroll, so their commands go in their own list (kept in order, drawn last).
        let mut commands = Vec::new();
        let (outer_clip, outer_fixed) = (view.clip.take(), view.in_fixed);
        view.in_fixed = true;
        layout(el, x - margin_left, y - margin_top, w + margin_left + margin_right, inherited, fonts, &mut commands, view);
        view.in_fixed = outer_fixed;
        view.clip = outer_clip;
        view.fixed.extend(commands);
    } else {
        layout(el, x - margin_left, y - margin_top, w + margin_left + margin_right, inherited, fonts, out, view);
    }
    el.style.size = saved_size;
}

/// Draws an Input's text, selection and caret into its content box, scrolling the text so the caret stays inside it.
/// The text is clipped to the box, since a value can be wider or taller than it. `(origin_x, origin_y)` is the
/// top-left of the element's border box, which is what the mouse is measured from when the text is clicked.
#[allow(clippy::too_many_arguments)]
fn layout_input(
    state: &mut crate::text_input::InputState,
    content: Rect,
    origin: (i32, i32),
    focused: bool,
    selection_color: Color,
    text_style: &TextStyle,
    fonts: &mut Fonts,
    out: &mut Vec<DrawCmd>,
    outer_clip: Option<Rect>,
) {
    let (key, size) = (font_key(text_style), text_style.font_size);
    let metrics = fonts.line_metrics(key, size);
    let indent = wrap_indent(&state.wrap_marker, fonts, key, size);
    let wrap_width = state.wrap.then(|| (content.w - INPUT_CARET_W).max(1) as f32);
    let chars: Vec<char> = state.value.chars().collect();

    // Where each character starts on its row. This is what turns the caret and the selection into places on the
    // screen, and (kept in `geometry`) a click into a place in the text.
    let (rows, columns) = input_rows(&state.value, wrap_width, indent, &mut |line| fonts.boundaries(key, size, line));

    // The caret is on the last row that starts at or before it, so at a break it is at the start of the next row.
    let caret_row = rows.iter().rposition(|row| row.start <= state.caret).unwrap_or(0);
    let caret_x = columns[caret_row][(state.caret - rows[caret_row].start).min(rows[caret_row].len)].round() as i32;
    let widest = if state.wrap { 0 } else { columns.iter().map(|c| c.last().copied().unwrap_or(0.0).ceil() as i32).max().unwrap_or(0) };

    let max_scroll_x = (widest + INPUT_CARET_W - content.w).max(0);
    if caret_x - state.scroll_x > content.w - INPUT_CARET_W {
        state.scroll_x = caret_x - content.w + INPUT_CARET_W;
    }
    state.scroll_x = state.scroll_x.min(caret_x).clamp(0, max_scroll_x);

    let visible_rows = (content.h / metrics.height).max(1) as usize;
    let mut first_row = state.scroll_line.max(0) as usize;
    if caret_row < first_row {
        first_row = caret_row;
    } else if caret_row >= first_row + visible_rows {
        first_row = caret_row + 1 - visible_rows;
    }
    first_row = first_row.min(rows.len().saturating_sub(visible_rows));
    state.scroll_line = first_row as i32;

    state.geometry = crate::text_input::Geometry {
        content_dx: content.x - origin.0,
        content_dy: content.y - origin.1,
        line_height: metrics.height,
        first_row,
        scroll_x: state.scroll_x,
        columns: columns.clone(),
        row_starts: rows.iter().map(|row| row.start).collect(),
    };

    let clip = Some(outer_clip.map_or(content, |c| content.intersect(c)));
    let text_cmd = |x: i32, y: i32, text: &str, color: Color| DrawCmd::Text {
        x,
        y,
        text: text.to_string(),
        font: key,
        font_size: size,
        color,
        // Cut off at the edge of the box, and at whatever the box itself is cut off by.
        clip,
        shadow: text_style.text_shadow.is_visible().then_some(text_style.text_shadow),
    };

    // The selection goes behind the text, one box per row it covers.
    if let Some((start, end)) = state.selection().filter(|_| focused) {
        for (row, r) in rows.iter().enumerate() {
            let (from, to) = (start.max(r.start), end.min(r.start + r.len));
            if row >= first_row && row < first_row + visible_rows && from < to {
                let x0 = columns[row][from - r.start].round() as i32;
                let x1 = columns[row][to - r.start].round() as i32;
                out.push(DrawCmd::Box {
                    rect: Rect { x: content.x - state.scroll_x + x0, y: content.y + (row - first_row) as i32 * metrics.height, w: x1 - x0, h: metrics.height },
                    background: selection_color,
                    border: [0; 4],
                    border_color: Color::TRANSPARENT,
                    radius: 0,
                    clip,
                });
            }
        }
    }

    let dim = Color { a: text_style.foreground.a * 0.5, ..text_style.foreground };
    if state.value.is_empty() {
        if let Some(first) = state.placeholder.split('\n').next().filter(|p| !p.is_empty()) {
            out.push(text_cmd(content.x, content.y, first, dim));
        }
    } else {
        // Each row in runs of one color, placed where the caret and selection say those characters are.
        let color_at = |i: usize| state.highlights.iter().rev().find(|h| h.start <= i && i < h.end).map_or(text_style.foreground, |h| h.color);
        for (row, r) in rows.iter().enumerate().skip(first_row).take(visible_rows) {
            let y = content.y + (row - first_row) as i32 * metrics.height;
            if r.continued && !state.wrap_marker.is_empty() {
                out.push(text_cmd(content.x - state.scroll_x, y, &state.wrap_marker, dim));
            }
            let mut at = 0;
            while at < r.len {
                let color = color_at(r.start + at);
                let mut to = at + 1;
                while to < r.len && color_at(r.start + to) == color {
                    to += 1;
                }
                let run: String = chars[r.start + at..r.start + to].iter().collect();
                if run.chars().any(|c| c != ' ') {
                    out.push(text_cmd(content.x - state.scroll_x + columns[row][at].round() as i32, y, &run, color));
                }
                at = to;
            }
        }
    }

    // The caret blinks nowhere and shows only while there is no selection to show instead.
    if focused && state.selection().is_none() {
        out.push(DrawCmd::Box {
            rect: Rect {
                x: content.x + caret_x - state.scroll_x,
                y: content.y + (caret_row - first_row) as i32 * metrics.height,
                w: INPUT_CARET_W,
                h: metrics.height,
            },
            background: text_style.foreground,
            border: [0; 4],
            border_color: Color::TRANSPARENT,
            radius: 0,
            clip: outer_clip,
        });
    }
}

/// Appends draw commands for `el` and its descendants, and records each element's `rect` as it goes.
/// `(x, y)` is the top-left of the element's margin box, and `avail_w` is the width its margin box may use.
pub fn layout(el: &mut Element, x: i32, y: i32, avail_w: i32, inherited: &TextStyle, fonts: &mut Fonts, out: &mut Vec<DrawCmd>, view: &mut View) {
    let text_style = el.text_style(inherited);
    let margin = px(el.style.margin.resolve());
    let border = px(el.style.border_size.resolve());
    let padding = px(el.style.padding.resolve());

    let (w, h) = border_box_size(el, (avail_w - margin[1] - margin[3]).max(0), inherited, fonts);
    let rect = Rect { x: x + margin[3], y: y + margin[0], w, h };
    el.rect = rect;

    // A radius can't be bigger than half the shorter side, or the corners would overlap.
    let radius = i32::from(el.style.corner_radius).min(w.min(h) / 2).max(0);

    if el.background().a > 0.0 || (border.iter().any(|&b| b > 0) && el.border_color().a > 0.0) {
        out.push(DrawCmd::Box {
            rect,
            background: el.background(),
            border,
            border_color: el.border_color(),
            radius,
            clip: view.clip,
        });
    }

    // The content box.
    let cx = rect.x + border[3] + padding[3];
    let cy = rect.y + border[0] + padding[0];
    let content_w = (w - border[1] - border[3] - padding[1] - padding[3]).max(0);
    let content_h = (h - border[0] - border[2] - padding[0] - padding[2]).max(0);

    match el.kind {
        ElementKind::Stack(orientation) => {
            let padding_box = Rect { x: rect.x + border[3], y: rect.y + border[0], w: w - border[1] - border[3], h: h - border[0] - border[2] };
            let outer_clip = view.clip;

            // A stack that clips cuts its children off at its padding box, and one that scrolls also moves them by how
            // far it is scrolled. How far it can be is how much of what is there doesn't fit.
            let (mut offset_x, mut offset_y) = (0, 0);
            if el.style.overflow != Overflow::Visible {
                let scrolls = el.style.overflow == Overflow::Scroll;
                let (total_w, total_h) = stack_content_size(el, orientation, content_w, &text_style, fonts);
                let at_end = el.scroll.y >= el.scroll.max_y;
                el.scroll.max_x = if scrolls { (total_w - content_w).max(0) } else { 0 };
                el.scroll.max_y = if scrolls { (total_h - content_h).max(0) } else { 0 };
                if el.style.stick_to_end && at_end {
                    el.scroll.y = el.scroll.max_y; // It was at the end, so it stays there as the content grows.
                }
                el.scroll.x = el.scroll.x.clamp(0, el.scroll.max_x);
                el.scroll.y = el.scroll.y.clamp(0, el.scroll.max_y);
                (offset_x, offset_y) = (el.scroll.x, el.scroll.y);
                view.clip = Some(outer_clip.map_or(padding_box, |c| padding_box.intersect(c)));
            }

            // Each child's size first, since aligning needs the total as well as each one.
            let gap = i32::from(el.style.gap);
            let (align_x, align_y) = (el.style.align_x, el.style.align_y);
            let mut sizes: Vec<(i32, i32)> = Vec::with_capacity(el.children.len());
            let mut along = 0;
            for child in el.children.iter().filter(|c| c.style.visible && c.style.position == Position::Flow) {
                if !sizes.is_empty() {
                    along += gap;
                }
                let size = outer_size(child, child_avail(orientation, content_w, along), &text_style, fonts);
                along += if orientation == Orientation::Vertical { size.1 } else { size.0 };
                sizes.push(size);
            }

            // `along` is now the length of the run, so the room left over on the main axis is what is not taken.
            let (mut x, mut y) = (cx - offset_x, cy - offset_y);
            match orientation {
                Orientation::Vertical => y += align_y.offset(content_h - along),
                Orientation::Horizontal => x += align_x.offset(content_w - along),
            }
            let mut along = 0;
            for (child, &(cw, ch)) in el.children.iter_mut().filter(|c| c.style.visible && c.style.position == Position::Flow).zip(&sizes) {
                let avail = child_avail(orientation, content_w, along);
                match orientation {
                    Orientation::Vertical => {
                        layout(child, x + align_x.offset(content_w - cw), y, avail, &text_style, fonts, out, view);
                        y += ch + gap;
                        along += ch + gap;
                    }
                    Orientation::Horizontal => {
                        layout(child, x, y + align_y.offset(content_h - ch), avail, &text_style, fonts, out, view);
                        x += cw + gap;
                        along += cw + gap;
                    }
                }
            }

            // Then the placed ones, against this element's padding box (the inside of its border).
            // They come after, so they draw over the children in the flow. They don't scroll with the content.
            for child in el.children.iter_mut().filter(|c| c.style.visible && c.style.position != Position::Flow) {
                place_out_of_flow(child, padding_box, &text_style, fonts, out, view);
            }
            view.clip = outer_clip;
        }
        ElementKind::Text(ref text) => {
            let (lines, metrics) = text_lines(text, &text_style, content_w, fonts);
            for (i, line) in lines.into_iter().enumerate() {
                if line.is_empty() {
                    continue;
                }
                out.push(DrawCmd::Text {
                    x: cx,
                    y: cy + i as i32 * metrics.height,
                    text: line,
                    font: font_key(&text_style),
                    font_size: text_style.font_size,
                    color: text_style.foreground,
                    clip: view.clip,
                    shadow: text_style.text_shadow.is_visible().then_some(text_style.text_shadow),
                });
            }
        }
        ElementKind::Rich(ref spans) => {
            let rich = layout_rich(spans, &text_style, content_w as f32, fonts);
            emit_rich(&rich, cx, cy, &text_style, view.clip, out);
        }
        ElementKind::Input(ref mut state) => {
            let content = Rect { x: cx, y: cy, w: content_w, h: content_h };
            let focused = el.interaction.focused;
            layout_input(state, content, (rect.x, rect.y), focused, el.style.selection_color, &text_style, fonts, out, view.clip);
        }
        ElementKind::Image(ref image) => {
            // The content box sits inside the border and padding, so its corners are less round than the border box's
            // by that much.
            let inset = border.iter().max().unwrap() + padding.iter().max().unwrap();
            out.push(DrawCmd::Image {
                rect: Rect { x: cx, y: cy, w: content_w, h: content_h },
                image: image.clone(),
                radius: (radius - inset).max(0).min(content_w.min(content_h) / 2),
                filter: el.style.image_filter,
                clip: view.clip,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::style::{Align, Color, FontFamily, FontStyle, WEIGHT_BOLD};

    const WIDE: i32 = 10_000;
    const SANS: FontKey = FontKey::regular(FontFamily::Sans);
    const MONO: FontKey = FontKey::regular(FontFamily::Mono);

    fn sized(w: u16, h: u16) -> Element {
        let mut el = Element::stack(Orientation::Vertical);
        el.style.size.x = Some(w);
        el.style.size.y = Some(h);
        el.style.background = Color::WHITE;
        el
    }

    fn mono_text(text: &str) -> Element {
        let mut el = Element::text(text);
        el.style.font_family = Some(FontFamily::Mono);
        el
    }

    fn image(w: u32, h: u32) -> Element {
        Element::image(Image::from_rgba(w, h, vec![255; (w * h * 4) as usize]).unwrap())
    }

    const VIEWPORT_H: i32 = 600;

    fn run(root: &mut Element, avail_w: i32, fonts: &mut Fonts) -> Vec<DrawCmd> {
        run_with_fixed(root, avail_w, fonts).0
    }

    /// Lays out with a canvas `avail_w` wide and 600 tall, and returns the scrolling commands and the fixed ones.
    fn run_with_fixed(root: &mut Element, avail_w: i32, fonts: &mut Fonts) -> (Vec<DrawCmd>, Vec<DrawCmd>) {
        let mut cmds = Vec::new();
        let mut fixed = Vec::new();
        let mut view = View::new(Rect { x: 0, y: 0, w: avail_w, h: VIEWPORT_H }, &mut fixed);
        layout(root, 0, 0, avail_w, &TextStyle::default(), fonts, &mut cmds, &mut view);
        (cmds, fixed)
    }

    fn placed(position: Position, w: u16, h: u16) -> Element {
        let mut el = sized(w, h);
        el.style.position = position;
        el
    }

    fn rects(cmds: &[DrawCmd]) -> Vec<Rect> {
        cmds.iter()
            .filter_map(|cmd| match cmd {
                DrawCmd::Box { rect, .. } => Some(*rect),
                _ => None,
            })
            .collect()
    }

    fn lines(cmds: &[DrawCmd]) -> Vec<(i32, i32, &str)> {
        cmds.iter()
            .filter_map(|cmd| match cmd {
                DrawCmd::Text { x, y, text, .. } => Some((*x, *y, text.as_str())),
                _ => None,
            })
            .collect()
    }

    fn image_rects(cmds: &[DrawCmd]) -> Vec<(Rect, i32)> {
        cmds.iter()
            .filter_map(|cmd| match cmd {
                DrawCmd::Image { rect, radius, .. } => Some((*rect, *radius)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn vertical_stack_places_children_in_order() {
        let mut fonts = Fonts::new();
        let mut root = Element::stack(Orientation::Vertical);
        root.style.padding.base = 10;
        root.add_child(sized(50, 20));
        root.add_child(sized(30, 40)).style.margin.base = 5;

        let cmds = run(&mut root, WIDE, &mut fonts);
        assert_eq!(
            rects(&cmds),
            vec![
                Rect { x: 10, y: 10, w: 50, h: 20 },
                Rect { x: 15, y: 35, w: 30, h: 40 }, // 10 + 20 + 5 margin
            ]
        );
        // Auto size: widest outer child (40) + padding, and 20 + 50 outer heights + padding.
        assert_eq!(border_box_size(&root, WIDE, &TextStyle::default(), &mut fonts), (50 + 20, 70 + 20));
    }

    #[test]
    fn gap_goes_between_children_and_not_after_the_last() {
        let mut fonts = Fonts::new();
        let mut root = Element::stack(Orientation::Vertical);
        root.style.gap = 6;
        root.add_child(sized(10, 10));
        root.add_child(sized(10, 10));
        root.add_child(sized(10, 10));
        let ys: Vec<i32> = rects(&run(&mut root, WIDE, &mut fonts)).iter().map(|r| r.y).collect();
        assert_eq!(ys, vec![0, 16, 32]);
        assert_eq!(border_box_size(&root, WIDE, &TextStyle::default(), &mut fonts), (10, 42), "two gaps, not three");
    }

    #[test]
    fn a_vertical_stack_aligns_children_across_and_the_run_along() {
        let mut fonts = Fonts::new();
        let mut root = Element::stack(Orientation::Vertical);
        root.style.size.x = Some(100);
        root.style.size.y = Some(100);
        root.style.align_x = Align::Center;
        root.style.align_y = Align::Center;
        root.style.gap = 10;
        root.add_child(sized(40, 20));
        root.add_child(sized(60, 20));

        let found = rects(&run(&mut root, WIDE, &mut fonts));
        // 20 + 10 + 20 = 50 tall in 100: starts 25 down. 40 wide in 100 starts 30 in, and 60 wide 20 in.
        assert_eq!(found[0], Rect { x: 30, y: 25, w: 40, h: 20 });
        assert_eq!(found[1], Rect { x: 20, y: 55, w: 60, h: 20 });
    }

    #[test]
    fn a_horizontal_stack_aligns_the_other_way_and_end_goes_to_the_far_side() {
        let mut fonts = Fonts::new();
        let mut root = Element::stack(Orientation::Horizontal);
        root.style.size.x = Some(100);
        root.style.size.y = Some(50);
        root.style.align_x = Align::End;
        root.style.align_y = Align::Center;
        root.add_child(sized(30, 10));
        root.add_child(sized(20, 30));

        let found = rects(&run(&mut root, WIDE, &mut fonts));
        assert_eq!(found[0], Rect { x: 50, y: 20, w: 30, h: 10 });
        assert_eq!(found[1], Rect { x: 80, y: 10, w: 20, h: 30 });
    }

    #[test]
    fn alignment_does_nothing_when_the_content_does_not_fit() {
        let mut fonts = Fonts::new();
        let mut root = Element::stack(Orientation::Vertical);
        root.style.size.x = Some(20);
        root.style.size.y = Some(20);
        root.style.align_x = Align::Center;
        root.style.align_y = Align::Center;
        root.add_child(sized(40, 50));
        let found = rects(&run(&mut root, WIDE, &mut fonts));
        assert_eq!((found[0].x, found[0].y), (0, 0));
    }

    #[test]
    fn horizontal_stack_advances_along_x() {
        let mut fonts = Fonts::new();
        let mut root = Element::stack(Orientation::Horizontal);
        root.add_child(sized(10, 10));
        root.add_child(sized(10, 10));
        assert_eq!(rects(&run(&mut root, WIDE, &mut fonts))[1].x, 10);
    }

    #[test]
    fn spacing_precedence() {
        use crate::style::Spacing;
        let s = Spacing { base: 1, x: Some(2), y: None, sides: None };
        assert_eq!(s.resolve(), [1, 2, 1, 2]);
        let s = Spacing { sides: Some([4, 3, 2, 1]), ..s };
        assert_eq!(s.resolve(), [4, 3, 2, 1]);
    }

    #[test]
    fn corner_radius_is_clamped_to_half_the_shorter_side() {
        let mut fonts = Fonts::new();
        let mut el = sized(40, 20);
        el.style.corner_radius = 500;
        match &run(&mut el, WIDE, &mut fonts)[0] {
            DrawCmd::Box { radius, .. } => assert_eq!(*radius, 10),
            other => panic!("expected a box, got {other:?}"),
        }
    }

    #[test]
    fn text_width_is_the_ceiling_of_its_drawn_width() {
        let mut fonts = Fonts::new();
        let metrics = fonts.line_metrics(SANS, 16);
        let width = fonts.text_width(SANS, 16, "Hello");
        let text = Element::text("Hello");
        assert_eq!(border_box_size(&text, WIDE, &TextStyle::default(), &mut fonts), (width.ceil() as i32, metrics.height));
    }

    #[test]
    fn proportional_text_is_narrower_for_narrow_letters() {
        let mut fonts = Fonts::new();
        let narrow = border_box_size(&Element::text("iiiiiiii"), WIDE, &TextStyle::default(), &mut fonts).0;
        let wide = border_box_size(&Element::text("MMMMMMMM"), WIDE, &TextStyle::default(), &mut fonts).0;
        assert!(narrow < wide);
    }

    #[test]
    fn text_wraps_to_the_available_width() {
        let mut fonts = Fonts::new();
        let metrics = fonts.line_metrics(MONO, 16);
        let nine_cols = (9.0 * fonts.text_width(MONO, 16, "m")).ceil() as i32;
        let mut root = Element::stack(Orientation::Vertical);
        root.add_child(mono_text("hello big world"));

        // Room for 9 columns: "hello big" fits, "world" wraps.
        let cmds = run(&mut root, nine_cols, &mut fonts);
        assert_eq!(lines(&cmds), vec![(0, 0, "hello big"), (0, metrics.height, "world")]);
        assert_eq!(border_box_size(&root, nine_cols, &TextStyle::default(), &mut fonts), (nine_cols, 2 * metrics.height));
    }

    #[test]
    fn text_wraps_inside_padding_and_a_fixed_width() {
        let mut fonts = Fonts::new();
        let six_cols = (6.0 * fonts.text_width(MONO, 16, "m")).ceil() as u16;
        let mut card = sized(six_cols + 8, 0); // 6 columns once 4px padding each side is removed.
        card.style.padding.base = 4;
        card.add_child(mono_text("one two three"));

        let cmds = run(&mut card, WIDE, &mut fonts);
        let found = lines(&cmds);
        assert_eq!(found.iter().map(|l| l.2).collect::<Vec<_>>(), ["one", "two", "three"]);
        assert_eq!(found[0].0, 4); // Starts inside the padding.
    }

    #[test]
    fn image_uses_its_own_size_by_default() {
        let mut fonts = Fonts::new();
        let cmds = run(&mut image(40, 30), WIDE, &mut fonts);
        assert_eq!(image_rects(&cmds), vec![(Rect { x: 0, y: 0, w: 40, h: 30 }, 0)]);
    }

    #[test]
    fn image_keeps_its_aspect_ratio_when_only_one_side_is_set() {
        let mut fonts = Fonts::new();
        let mut wide_only = image(40, 30);
        wide_only.style.size.x = Some(80);
        assert_eq!(border_box_size(&wide_only, WIDE, &TextStyle::default(), &mut fonts), (80, 60));

        let mut tall_only = image(40, 30);
        tall_only.style.size.y = Some(15);
        assert_eq!(border_box_size(&tall_only, WIDE, &TextStyle::default(), &mut fonts), (20, 15));
    }

    #[test]
    fn image_stretches_when_both_sides_are_set() {
        let mut fonts = Fonts::new();
        let mut el = image(40, 30);
        el.style.size = crate::style::Spacing { base: 0, x: Some(10), y: Some(100), sides: None };
        assert_eq!(border_box_size(&el, WIDE, &TextStyle::default(), &mut fonts), (10, 100));
    }

    #[test]
    fn image_shrinks_to_the_available_width() {
        let mut fonts = Fonts::new();
        let mut root = Element::stack(Orientation::Vertical);
        root.add_child(image(200, 100));
        let cmds = run(&mut root, 50, &mut fonts);
        assert_eq!(image_rects(&cmds)[0].0, Rect { x: 0, y: 0, w: 50, h: 25 });
    }

    #[test]
    fn image_is_drawn_in_the_content_box() {
        let mut fonts = Fonts::new();
        let mut el = image(20, 20);
        el.style.padding.base = 3;
        el.style.border_size.base = 2;
        el.style.corner_radius = 8;
        // Border box is 30x30, so the radius stays 8, and the content box's is 8 - (2 border + 3 padding).
        let cmds = run(&mut el, WIDE, &mut fonts);
        assert_eq!(image_rects(&cmds), vec![(Rect { x: 5, y: 5, w: 20, h: 20 }, 3)]);
    }

    /// The text commands as `(text, color, font)`.
    fn text_styles(cmds: &[DrawCmd]) -> Vec<(&str, Color, FontKey)> {
        cmds.iter()
            .filter_map(|cmd| match cmd {
                DrawCmd::Text { text, color, font, .. } => Some((text.as_str(), *color, *font)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn text_inherits_the_foreground_of_its_ancestors() {
        let mut fonts = Fonts::new();
        let red = Color { r: 255, g: 0, b: 0, a: 1.0 };
        let green = Color { r: 0, g: 255, b: 0, a: 1.0 };

        let mut root = Element::stack(Orientation::Vertical);
        root.style.foreground = Some(red);
        let card = root.add_child(Element::stack(Orientation::Vertical));
        card.add_child(Element::text("deep")); // Two levels below the one that set the color.
        let own = card.add_child(Element::text("own"));
        own.style.foreground = Some(green);
        root.add_child(Element::text("sibling"));

        let cmds = run(&mut root, WIDE, &mut fonts);
        let colors: Vec<_> = text_styles(&cmds).iter().map(|t| (t.0, t.1)).collect();
        assert_eq!(colors, vec![("deep", red), ("own", green), ("sibling", red)]);
    }

    #[test]
    fn text_uses_the_canvas_defaults_when_nothing_sets_a_property() {
        let mut fonts = Fonts::new();
        let mut root = Element::stack(Orientation::Vertical);
        root.add_child(Element::text("plain"));
        let cmds = run(&mut root, WIDE, &mut fonts);
        assert_eq!(text_styles(&cmds), vec![("plain", Color::BLACK, SANS)]);
    }

    #[test]
    fn weight_style_family_and_size_are_inherited_and_can_be_overridden() {
        let mut fonts = Fonts::new();
        let mut root = Element::stack(Orientation::Vertical);
        root.style.font_weight = Some(WEIGHT_BOLD);
        root.style.font_style = Some(FontStyle::Italic);
        root.style.font_size = Some(30);
        root.add_child(Element::text("inherits"));
        let plain = root.add_child(Element::text("overrides"));
        plain.style.font_weight = Some(400);
        plain.style.font_family = Some(FontFamily::Mono);

        let cmds = run(&mut root, WIDE, &mut fonts);
        let bold_italic = FontKey { family: FontFamily::Sans, bold: true, italic: true };
        let mono_italic = FontKey { family: FontFamily::Mono, bold: false, italic: true };
        assert_eq!(text_styles(&cmds).iter().map(|t| t.2).collect::<Vec<_>>(), vec![bold_italic, mono_italic]);
        assert!(cmds.iter().all(|c| !matches!(c, DrawCmd::Text { font_size, .. } if *font_size != 30)));
    }

    #[test]
    fn inherited_text_properties_affect_the_parents_size() {
        let mut fonts = Fonts::new();
        let measure = |fonts: &mut Fonts, size: Option<u16>, weight: Option<u16>| {
            let mut root = Element::stack(Orientation::Vertical);
            root.style.font_size = size;
            root.style.font_weight = weight;
            root.add_child(Element::text("Measure this sentence"));
            border_box_size(&root, WIDE, &TextStyle::default(), fonts)
        };
        let normal = measure(&mut fonts, None, None);
        let bigger = measure(&mut fonts, Some(32), None);
        let bolder = measure(&mut fonts, None, Some(WEIGHT_BOLD));
        assert!(bigger.0 > normal.0 && bigger.1 > normal.1, "a larger inherited size makes the stack bigger");
        assert!(bolder.0 > normal.0, "bold is wider");
    }

    #[test]
    fn layout_records_each_elements_rect() {
        let mut fonts = Fonts::new();
        let mut root = Element::stack(Orientation::Vertical);
        root.style.padding.base = 10;
        root.add_child(sized(50, 20));
        root.add_child(sized(30, 40)).style.margin.base = 5;

        run(&mut root, WIDE, &mut fonts);
        assert_eq!(root.rect, Rect { x: 0, y: 0, w: 70, h: 90 });
        assert_eq!(root.children[0].rect, Rect { x: 10, y: 10, w: 50, h: 20 });
        assert_eq!(root.children[1].rect, Rect { x: 15, y: 35, w: 30, h: 40 });
    }

    /// A parent 200x100 with a 3px border and 10px padding, at (0, 0), holding one placed child.
    fn parent_with(child: Element) -> Element {
        let mut parent = sized(200, 100);
        parent.style.border_size.base = 3;
        parent.style.padding.base = 10;
        parent.add_child(child);
        parent
    }

    #[test]
    fn placed_elements_take_no_room_and_move_nothing() {
        let mut fonts = Fonts::new();
        let mut root = Element::stack(Orientation::Vertical);
        root.add_child(sized(50, 20));
        root.add_child(placed(Position::Absolute, 300, 300));
        root.add_child(placed(Position::Fixed, 300, 300));
        root.add_child(sized(50, 30));

        run(&mut root, WIDE, &mut fonts);
        assert_eq!(root.children[3].rect.y, 20, "the next flow child follows the first, not the placed ones");
        assert_eq!(border_box_size(&root, WIDE, &TextStyle::default(), &mut fonts), (50, 50), "and the parent's size ignores them");
    }

    #[test]
    fn absolute_is_placed_from_the_parents_padding_box() {
        let mut fonts = Fonts::new();
        let mut child = placed(Position::Absolute, 20, 20);
        child.style.inset.left = Some(5);
        child.style.inset.top = Some(7);
        let mut parent = parent_with(child);
        run(&mut parent, WIDE, &mut fonts);
        // Inside the 3px border, then the inset. (The 10px padding doesn't push it: it is a different box.)
        assert_eq!(parent.children[0].rect, Rect { x: 3 + 5, y: 3 + 7, w: 20, h: 20 });
    }

    #[test]
    fn right_and_bottom_anchor_to_the_far_edges() {
        let mut fonts = Fonts::new();
        let mut child = placed(Position::Absolute, 20, 20);
        child.style.inset.right = Some(4);
        child.style.inset.bottom = Some(6);
        let mut parent = parent_with(child);
        run(&mut parent, WIDE, &mut fonts);
        // The padding box is 194x94 from (3, 3).
        assert_eq!(parent.children[0].rect, Rect { x: 3 + 194 - 4 - 20, y: 3 + 94 - 6 - 20, w: 20, h: 20 });
    }

    #[test]
    fn two_opposite_sides_stretch_an_element_with_no_size() {
        let mut fonts = Fonts::new();
        let mut child = placed(Position::Absolute, 0, 0);
        child.style.inset = crate::style::Insets { left: Some(10), right: Some(20), top: Some(1), bottom: Some(2), ..Default::default() };
        let mut parent = parent_with(child);
        run(&mut parent, WIDE, &mut fonts);
        assert_eq!(parent.children[0].rect, Rect { x: 3 + 10, y: 3 + 1, w: 194 - 10 - 20, h: 94 - 1 - 2 });
        assert_eq!(parent.children[0].style.size.x, Some(0), "the stretch put the size back to 0 (auto), as it was");
    }

    #[test]
    fn with_a_size_the_left_and_top_win_over_the_far_sides() {
        let mut fonts = Fonts::new();
        let mut child = placed(Position::Absolute, 20, 20);
        child.style.inset = crate::style::Insets { left: Some(10), right: Some(10), top: Some(10), bottom: Some(10), ..Default::default() };
        let mut parent = parent_with(child);
        run(&mut parent, WIDE, &mut fonts);
        assert_eq!(parent.children[0].rect, Rect { x: 13, y: 13, w: 20, h: 20 });
    }

    #[test]
    fn centering_works_on_each_axis_and_includes_margins() {
        let mut fonts = Fonts::new();
        let mut child = placed(Position::Absolute, 20, 40);
        child.style.inset.center_x = true;
        child.style.inset.top = Some(0);
        let mut parent = parent_with(child);
        run(&mut parent, WIDE, &mut fonts);
        assert_eq!(parent.children[0].rect.x, 3 + (194 - 20) / 2);
        assert_eq!(parent.children[0].rect.y, 3);

        let mut child = placed(Position::Absolute, 20, 40);
        child.style.inset.center_y = true;
        child.style.margin.base = 4; // The margin box (28x48) is what gets centered.
        let mut parent = parent_with(child);
        run(&mut parent, WIDE, &mut fonts);
        assert_eq!(parent.children[0].rect.y, 3 + (94 - 48) / 2 + 4);
        assert_eq!(parent.children[0].rect.x, 3 + 4, "no inset on the other axis: at the start, past its margin");
    }

    #[test]
    fn placed_elements_draw_after_the_flow_children_so_they_are_on_top() {
        let mut fonts = Fonts::new();
        let mut root = Element::stack(Orientation::Vertical);
        root.add_child(placed(Position::Absolute, 10, 10)).style.background = Color { r: 1, g: 2, b: 3, a: 1.0 };
        root.add_child(sized(30, 30)); // Later in the tree, but in the flow.
        let cmds = run(&mut root, WIDE, &mut fonts);
        let colors: Vec<u8> = cmds.iter().map(|c| match c { DrawCmd::Box { background, .. } => background.r, _ => 0 }).collect();
        assert_eq!(colors, vec![255, 1]);
    }

    #[test]
    fn a_placed_element_can_hold_content_and_wraps_text_to_its_room() {
        let mut fonts = Fonts::new();
        let mut panel = placed(Position::Absolute, 0, 0);
        panel.style.inset = crate::style::Insets { left: Some(0), top: Some(0), right: Some(0), ..Default::default() };
        panel.style.padding.base = 5;
        panel.add_child(mono_text("aaa bbb ccc ddd eee fff"));
        let mut root = sized(100, 300);
        root.add_child(panel);
        let cmds = run(&mut root, WIDE, &mut fonts);
        assert!(lines(&cmds).len() > 1, "the text wrapped to the panel's 90px of content, not to the whole canvas");
        assert_eq!(root.children[0].rect.w, 100, "stretched to the parent");
    }

    #[test]
    fn fixed_is_placed_against_the_canvas_and_kept_apart() {
        let mut fonts = Fonts::new();
        let mut bar = placed(Position::Fixed, 40, 0);
        bar.style.inset = crate::style::Insets { right: Some(0), top: Some(0), bottom: Some(0), ..Default::default() };
        let mut parent = sized(10, 10); // Small and nowhere near the right edge, to show it isn't used.
        parent.add_child(bar);
        let (cmds, fixed) = run_with_fixed(&mut parent, 500, &mut fonts);

        assert_eq!(parent.children[0].rect, Rect { x: 460, y: 0, w: 40, h: VIEWPORT_H });
        assert_eq!(rects(&cmds).len(), 1, "only the parent is in the scrolling commands");
        assert_eq!(rects(&fixed), vec![Rect { x: 460, y: 0, w: 40, h: VIEWPORT_H }]);
    }

    #[test]
    fn fixed_inside_fixed_is_placed_against_the_outer_one() {
        let mut fonts = Fonts::new();
        let mut inner = placed(Position::Fixed, 10, 10);
        inner.style.inset.left = Some(5);
        inner.style.inset.top = Some(5);
        let mut outer = placed(Position::Fixed, 100, 100);
        outer.style.inset.left = Some(200);
        outer.style.inset.top = Some(50);
        outer.add_child(inner);
        let mut root = Element::stack(Orientation::Vertical);
        root.add_child(outer);
        let (_, fixed) = run_with_fixed(&mut root, 500, &mut fonts);

        assert_eq!(root.children[0].children[0].rect, Rect { x: 205, y: 55, w: 10, h: 10 });
        assert_eq!(rects(&fixed).len(), 2);
        assert_eq!(rects(&fixed)[0].w, 100, "the outer's background comes first, so the inner is over it");
    }

    fn span(text: &str, color: Option<Color>) -> crate::element::TextSpan {
        crate::element::TextSpan { text: text.to_string(), foreground: color, ..Default::default() }
    }

    fn mono_rich(spans: Vec<crate::element::TextSpan>) -> Element {
        let mut el = Element::rich(spans);
        el.style.font_family = Some(FontFamily::Mono);
        el
    }

    #[test]
    fn rich_text_draws_each_span_in_its_own_color() {
        let mut fonts = Fonts::new();
        let red = Color { r: 255, g: 0, b: 0, a: 1.0 };
        let mut el = mono_rich(vec![span("red ", Some(red)), span("plain", None)]);
        el.style.foreground = Some(Color::WHITE);
        let cmds = run(&mut el, WIDE, &mut fonts);
        let drawn: Vec<(&str, Color)> = cmds.iter().filter_map(|c| match c { DrawCmd::Text { text, color, .. } => Some((text.as_str(), *color)), _ => None }).collect();
        assert_eq!(drawn, vec![("red", red), ("plain", Color::WHITE)], "spaces draw nothing, the unset span inherits");
    }

    #[test]
    fn rich_text_wraps_as_one_sentence_and_takes_the_height_of_its_lines() {
        let mut fonts = Fonts::new();
        let metrics = fonts.line_metrics(MONO, 16);
        let nine = (9.0 * fonts.text_width(MONO, 16, "m")).ceil() as i32;
        let mut root = Element::stack(Orientation::Vertical);
        root.add_child(mono_rich(vec![span("hello ", None), span("big world", Some(Color::WHITE))]));
        let cmds = run(&mut root, nine, &mut fonts);
        let rows: Vec<(i32, String)> = cmds.iter().filter_map(|c| match c { DrawCmd::Text { y, text, .. } => Some((*y, text.clone())), _ => None }).collect();
        assert_eq!(rows, vec![(0, "hello".to_string()), (0, "big".to_string()), (metrics.height, "world".to_string())]);
        assert_eq!(border_box_size(&root, nine, &TextStyle::default(), &mut fonts).1, 2 * metrics.height);
    }

    #[test]
    fn underline_and_strikethrough_are_boxes_in_the_spans_color_that_run_across_spaces() {
        let mut fonts = Fonts::new();
        let blue = Color { r: 0, g: 0, b: 255, a: 1.0 };
        let underlined = crate::element::TextSpan { underline: true, foreground: Some(blue), ..span("two words", None) };
        let struck = crate::element::TextSpan { strikethrough: true, ..span(" gone", None) };
        let mut el = mono_rich(vec![underlined, struck]);
        let cmds = run(&mut el, WIDE, &mut fonts);
        let boxes: Vec<(Rect, Color)> = cmds.iter().filter_map(|c| match c { DrawCmd::Box { rect, background, .. } => Some((*rect, *background)).filter(|(r, _)| r.h < 4), _ => None }).collect();

        let underlines: Vec<_> = boxes.iter().filter(|(_, c)| *c == blue).collect();
        assert_eq!(underlines.len(), 3, "\"two\", the space, and \"words\"");
        let widths: i32 = underlines.iter().map(|(r, _)| r.w).sum();
        let text_width = fonts.text_width(MONO, 16, "two words").round() as i32;
        assert!((widths - text_width).abs() <= 2, "together they cover the text, with no gap at the space");
        assert_eq!(underlines[0].0.y, underlines[2].0.y, "on one line");

        let metrics = fonts.line_metrics(MONO, 16);
        let struck_through: Vec<_> = boxes.iter().filter(|(_, c)| *c != blue).collect();
        assert!(!struck_through.is_empty());
        assert!(struck_through.iter().all(|(r, _)| r.y < metrics.ascent && r.y > 0), "through the text, above the baseline");
    }

    #[test]
    fn rich_text_inside_a_scroller_is_clipped_and_gets_the_inherited_shadow() {
        let mut fonts = Fonts::new();
        let shadow = crate::style::TextShadow { color: Color { r: 0, g: 0, b: 0, a: 0.6 }, offset_x: 2, offset_y: 2 };
        let mut list = sized(100, 20);
        list.style.overflow = Overflow::Hidden;
        list.style.text_shadow = Some(shadow);
        list.add_child(mono_rich(vec![span("one ", None), span("two three four five", None)]));
        let cmds = run(&mut list, WIDE, &mut fonts);
        assert!(cmds.iter().filter(|c| matches!(c, DrawCmd::Text { .. })).all(|c| matches!(c, DrawCmd::Text { clip: Some(_), shadow: Some(s), .. } if *s == shadow)));
    }

    #[test]
    fn rich_text_values_can_be_read_and_replaced() {
        let mut el = Element::rich(vec![span("Hello, ", None), span("world", None)]);
        assert_eq!(el.text_value().as_deref(), Some("Hello, world"), "the spans put together");
        assert!(el.set_text("plain now"));
        assert_eq!(el.text_value().as_deref(), Some("plain now"));
        assert!(el.set_spans(vec![span("a", None)]));
        assert!(!Element::text("t").set_spans(vec![]), "only a rich element has spans");
    }

    #[test]
    fn setting_the_same_spans_again_costs_nothing() {
        let before = crate::dirty::revision();
        let mut el = Element::rich(vec![span("a", None)]);
        el.set_spans(vec![span("a", None)]);
        assert_eq!(crate::dirty::revision(), before, "no change, no redraw");
        el.set_spans(vec![span("b", None)]);
        assert_eq!(crate::dirty::revision(), before + 1);
    }

    fn shadows(cmds: &[DrawCmd]) -> Vec<Option<crate::style::TextShadow>> {
        cmds.iter().filter_map(|c| match c { DrawCmd::Text { shadow, .. } => Some(*shadow), _ => None }).collect()
    }

    #[test]
    fn text_has_no_shadow_by_default() {
        let mut fonts = Fonts::new();
        let mut root = Element::stack(Orientation::Vertical);
        root.add_child(Element::text("plain"));
        assert_eq!(shadows(&run(&mut root, WIDE, &mut fonts)), vec![None]);
    }

    #[test]
    fn a_shadow_is_inherited_like_the_text_color() {
        let mut fonts = Fonts::new();
        let shadow = crate::style::TextShadow { color: Color { r: 0, g: 0, b: 0, a: 0.5 }, offset_x: 2, offset_y: 3 };
        let mut root = Element::stack(Orientation::Vertical);
        root.style.text_shadow = Some(shadow);
        root.add_child(Element::text("one"));
        let inner = root.add_child(Element::stack(Orientation::Vertical));
        inner.add_child(Element::text("deeper"));
        let own = inner.add_child(Element::text("its own"));
        own.style.text_shadow = Some(crate::style::TextShadow::NONE); // Turned off just here.
        assert_eq!(shadows(&run(&mut root, WIDE, &mut fonts)), vec![Some(shadow), Some(shadow), None]);
    }

    #[test]
    fn a_fully_transparent_shadow_is_no_shadow() {
        let mut fonts = Fonts::new();
        let mut root = Element::stack(Orientation::Vertical);
        root.style.text_shadow = Some(crate::style::TextShadow { color: Color { r: 9, g: 9, b: 9, a: 0.0 }, offset_x: 2, offset_y: 2 });
        root.add_child(Element::text("x"));
        assert_eq!(shadows(&run(&mut root, WIDE, &mut fonts)), vec![None], "nothing for the rasterizer to check or draw");
    }

    #[test]
    fn invalid_image_buffers_are_rejected() {
        assert!(Image::from_rgba(2, 2, vec![0; 15]).is_none());
        assert!(Image::from_rgba(0, 2, vec![]).is_none());
        assert!(Image::from_rgba(2, 2, vec![0; 16]).is_some());
    }
}

#[cfg(test)]
mod input_row_tests {
    use super::input_rows;

    /// Every character 10 pixels wide, so rows can be worked out by hand.
    fn fixed(line: &str) -> Vec<f32> {
        (0..=line.chars().count()).map(|i| i as f32 * 10.0).collect()
    }

    fn rows_of(value: &str, width: Option<f32>, indent: f32) -> Vec<(usize, usize, bool)> {
        input_rows(value, width, indent, &mut fixed).0.iter().map(|r| (r.start, r.len, r.continued)).collect()
    }

    #[test]
    fn without_wrapping_every_line_is_one_row() {
        assert_eq!(rows_of("ab\n\ncde", None, 0.0), vec![(0, 2, false), (3, 0, false), (4, 3, false)]);
        assert_eq!(rows_of("", Some(50.0), 0.0), vec![(0, 0, false)], "an empty value is still one row");
    }

    #[test]
    fn a_long_line_breaks_after_the_last_space_that_fits() {
        // Ten characters fit a row; later rows lose two to the indent.
        let rows = rows_of("hello world foo", Some(100.0), 20.0);
        assert_eq!(rows, vec![(0, 6, false), (6, 6, true), (12, 3, true)]);
    }

    #[test]
    fn a_word_wider_than_a_row_breaks_inside_it() {
        assert_eq!(rows_of("abcdefghijklmnop", Some(100.0), 0.0), vec![(0, 10, false), (10, 6, true)]);
    }

    #[test]
    fn rows_start_where_their_line_does_and_columns_include_the_indent() {
        let (rows, columns) = input_rows("ab\nhello world", Some(70.0), 20.0, &mut fixed);
        assert_eq!(rows.iter().map(|r| r.start).collect::<Vec<_>>(), vec![0, 3, 9]);
        assert_eq!(columns[0], vec![0.0, 10.0, 20.0]);
        assert_eq!(columns[2][0], 20.0, "a continued row starts at the indent");
        assert_eq!(columns[2].len(), rows[2].len + 1);
    }

    #[test]
    fn a_row_always_takes_at_least_one_character() {
        assert_eq!(rows_of("abc", Some(5.0), 0.0), vec![(0, 1, false), (1, 1, true), (2, 1, true)]);
    }
}
