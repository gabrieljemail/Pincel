//! The built-in fonts, their glyph atlas, and text measurement.
//!
//! Glyphs are rasterized by fontdue the first time they are needed at a given family and size, and packed into a
//! single-channel (coverage) atlas. The rasterizer then blits from the atlas, so each glyph is only rasterized once.
//!
//! Text is positioned by [`Fonts::place`], which layout (to measure) and the rasterizer (to draw) both use,
//! so what is measured is exactly what is drawn.

use std::collections::HashMap;
use std::sync::{Arc, OnceLock, RwLock, RwLockReadGuard};

use fontdue::{Font, FontSettings};

use crate::style::{FontFamily, FontStyle};

/// Weights from this up are drawn with the bold face of a family.
const BOLD_FROM: u16 = 600;

/// A face to draw with: a family, and whether it is the bold and/or italic one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FontKey {
    pub family: FontFamily,
    pub bold: bool,
    pub italic: bool,
}

impl FontKey {
    pub fn new(family: FontFamily, weight: u16, style: FontStyle) -> FontKey {
        FontKey {
            family,
            bold: weight >= BOLD_FROM,
            italic: style == FontStyle::Italic,
        }
    }

    pub const fn regular(family: FontFamily) -> FontKey {
        FontKey { family, bold: false, italic: false }
    }
}

// ---- The library of font families ---------------------------------------------------------------------------------
//
// Families are registered once for the whole process, and every canvas can use them. That way a font loaded by one
// part of a program (or one mod) is there for the others, and ids mean the same thing everywhere. They are only ever
// added, never removed or changed, so an id stays valid.

#[derive(Clone)]
enum FaceData {
    Static(&'static [u8]),
    Owned(Arc<[u8]>),
}

impl FaceData {
    fn bytes(&self) -> &[u8] {
        match self {
            FaceData::Static(bytes) => bytes,
            FaceData::Owned(bytes) => bytes,
        }
    }
}

struct FamilyEntry {
    name: String,
    /// Regular, italic, bold, bold italic. Only the regular one has to be there.
    faces: [Option<FaceData>; 4],
    /// Another family to take glyphs from when this one doesn't have them.
    fallback: Option<FontFamily>,
}

impl FamilyEntry {
    /// Which of the faces to draw for a request: the one asked for if the family has it, else the closest it has.
    fn pick(&self, bold: bool, italic: bool) -> usize {
        let preference: &[usize] = match (bold, italic) {
            (false, false) => &[0],
            (false, true) => &[1, 0],
            (true, false) => &[2, 0],
            (true, true) => &[3, 2, 1, 0],
        };
        preference.iter().copied().find(|&i| self.faces[i].is_some()).unwrap_or(0)
    }
}

fn library() -> &'static RwLock<Vec<FamilyEntry>> {
    static LIBRARY: OnceLock<RwLock<Vec<FamilyEntry>>> = OnceLock::new();
    LIBRARY.get_or_init(|| {
        let built_in = |name: &str, faces: [&'static [u8]; 4]| FamilyEntry {
            name: name.to_string(),
            faces: faces.map(|bytes| Some(FaceData::Static(bytes))),
            fallback: None,
        };
        // The order has to match `FontFamily::Sans` and `FontFamily::Mono`.
        RwLock::new(vec![
            built_in(
                "Sans",
                [
                    include_bytes!("../assets/Lato-Regular.ttf"),
                    include_bytes!("../assets/Lato-Italic.ttf"),
                    include_bytes!("../assets/Lato-Bold.ttf"),
                    include_bytes!("../assets/Lato-BoldItalic.ttf"),
                ],
            ),
            built_in(
                "Mono",
                [
                    include_bytes!("../assets/JetBrainsMono-Regular.ttf"),
                    include_bytes!("../assets/JetBrainsMono-Italic.ttf"),
                    include_bytes!("../assets/JetBrainsMono-Bold.ttf"),
                    include_bytes!("../assets/JetBrainsMono-BoldItalic.ttf"),
                ],
            ),
        ])
    })
}

fn read_library() -> RwLockReadGuard<'static, Vec<FamilyEntry>> {
    library().read().unwrap_or_else(|e| e.into_inner())
}

/// Adds a font family and returns its id, to use as a `font_family`. `faces` are the font files' contents (TrueType
/// or OpenType) for regular, italic, bold and bold italic. Only regular is needed: a style the family has no face for is
/// drawn with the closest one it does have (so bold with no bold face is regular), rather than made up.
///
/// The name is how `find_family` looks it up, ignoring case, so it has to be new. The names `Sans` and `Mono` are the
/// built-in families. The files are checked, and the family isn't added if any can't be read. The check only looks at
/// the file's structure, so it is quick; the glyph data is read the first time the face is used. (A file that passes
/// the check but is broken further in is drawn with the built-in sans-serif face instead.)
pub fn register_family(name: &str, faces: [Option<Vec<u8>>; 4]) -> Result<FontFamily, String> {
    if name.trim().is_empty() {
        return Err("a font family needs a name".to_string());
    }
    if faces[0].is_none() {
        return Err(format!("{name}: a font family needs at least a regular face"));
    }
    let faces = faces.map(|face| face.map(|bytes| FaceData::Owned(bytes.into())));
    for (i, face) in faces.iter().enumerate() {
        if let Some(face) = face {
            ttf_parser::Face::parse(face.bytes(), 0).map_err(|e| format!("{name}: face {i} isn't a usable font: {e}"))?;
        }
    }

    let mut library = library().write().unwrap_or_else(|e| e.into_inner());
    if library.iter().any(|entry| entry.name.eq_ignore_ascii_case(name)) {
        return Err(format!("a font family named {name} already exists"));
    }
    let id = u16::try_from(library.len()).map_err(|_| "too many font families".to_string())?;
    library.push(FamilyEntry { name: name.to_string(), faces, fallback: None });
    Ok(FontFamily(id))
}

/// Like [`register_family`], reading the faces from files.
pub fn register_family_files(name: &str, paths: [Option<&str>; 4]) -> Result<FontFamily, String> {
    let mut faces: [Option<Vec<u8>>; 4] = Default::default();
    for (face, path) in faces.iter_mut().zip(paths) {
        if let Some(path) = path {
            *face = Some(std::fs::read(path).map_err(|e| format!("{path}: {e}"))?);
        }
    }
    register_family(name, faces)
}

/// The family with this name, ignoring case. `Sans` and `Mono` are the built-in ones.
pub fn find_family(name: &str) -> Option<FontFamily> {
    read_library().iter().position(|entry| entry.name.eq_ignore_ascii_case(name)).map(|i| FontFamily(i as u16))
}

/// Says to draw a character from `fallback` when `family` has no glyph for it, such as symbols and emoji that a
/// pixel font lacks. `None` removes it. Fallbacks can chain, a few deep. Returns false if either family is unknown.
pub fn set_fallback(family: FontFamily, fallback: Option<FontFamily>) -> bool {
    let mut library = library().write().unwrap_or_else(|e| e.into_inner());
    let count = library.len();
    if usize::from(family.0) >= count || fallback.is_some_and(|f| usize::from(f.0) >= count) {
        return false;
    }
    library[usize::from(family.0)].fallback = fallback;
    true
}

/// The most families a character is looked up through before giving up, which also stops a loop of fallbacks.
const FALLBACK_DEPTH: usize = 4;

/// Vertical metrics for a font at a size, rounded to whole pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LineMetrics {
    /// Distance between baselines, which is the height of a line of text.
    pub height: i32,
    /// Distance from the top of the line to the baseline.
    pub ascent: i32,
}

/// Where a glyph is in the atlas, and how to place it relative to the pen.
#[derive(Clone, Copy, Debug)]
pub struct Glyph {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    /// Offset of the bitmap's left edge from the pen.
    pub left: i32,
    /// Offset of the bitmap's bottom edge above the baseline.
    pub bottom: i32,
    /// How far the pen moves after this glyph, in fractional pixels.
    pub advance: f32,
    /// The face the glyph came from, which isn't the one asked for if that family borrows it from a fallback.
    pub face: FontKey,
}

/// A single-channel image that glyphs are packed into, row by row ("shelf" packing).
/// The width is fixed and the height grows, so glyphs already in the atlas never move.
pub struct Atlas {
    pub width: u32,
    pub height: u32,
    /// One coverage byte (0 to 255) per pixel, rows tightly packed.
    pub data: Vec<u8>,
    cursor_x: u32,
    cursor_y: u32,
    row_height: u32,
}

impl Atlas {
    const WIDTH: u32 = 1024;
    const PADDING: u32 = 1; // Keeps neighbors from bleeding into each other if the atlas is ever sampled with filtering.

    fn new() -> Atlas {
        Atlas {
            width: Self::WIDTH,
            height: 256,
            data: vec![0; (Self::WIDTH * 256) as usize],
            cursor_x: 0,
            cursor_y: 0,
            row_height: 0,
        }
    }

    /// Copies a `width` x `height` bitmap in and returns its top-left corner.
    fn insert(&mut self, width: u32, height: u32, bitmap: &[u8]) -> (u32, u32) {
        assert!(width + Self::PADDING <= self.width, "glyph wider than the atlas");

        if self.cursor_x + width + Self::PADDING > self.width {
            // Start a new row.
            self.cursor_y += self.row_height + Self::PADDING;
            self.cursor_x = 0;
            self.row_height = 0;
        }
        while self.cursor_y + height + Self::PADDING > self.height {
            self.height *= 2;
            self.data.resize((self.width * self.height) as usize, 0);
        }

        let (x, y) = (self.cursor_x, self.cursor_y);
        for row in 0..height {
            let src = (row * width) as usize;
            let dst = ((y + row) * self.width + x) as usize;
            self.data[dst..dst + width as usize].copy_from_slice(&bitmap[src..src + width as usize]);
        }
        self.cursor_x += width + Self::PADDING;
        self.row_height = self.row_height.max(height);
        (x, y)
    }
}

pub struct Fonts {
    /// Parsed on first use, since most programs only ever draw a few faces. Keyed by family and which of its four
    /// faces, after the choice of face has been made, so two requests that end up at the same file share one.
    faces: HashMap<(u16, usize), Font>,
    pub atlas: Atlas,
    glyphs: HashMap<(FontKey, char, u16), Glyph>,
    lines: HashMap<(FontKey, u16), LineMetrics>,
}

impl Fonts {
    pub fn new() -> Fonts {
        Fonts {
            faces: HashMap::new(),
            atlas: Atlas::new(),
            glyphs: HashMap::new(),
            lines: HashMap::new(),
        }
    }

    /// Makes sure the face for `key` is parsed, and says where it is. A family id nothing was registered for is the
    /// built-in sans family.
    fn load(&mut self, key: FontKey) -> (u16, usize) {
        let library = read_library();
        let id = if usize::from(key.family.0) < library.len() { key.family.0 } else { 0 };
        let entry = &library[usize::from(id)];
        let face = entry.pick(key.bold, key.italic);
        let slot = (id, face);
        if !self.faces.contains_key(&slot) {
            let bytes = entry.faces[face].as_ref().expect("the pick is a face that exists").bytes();
            let font = match Font::from_bytes(bytes, FontSettings::default()) {
                Ok(font) => font,
                Err(_) => {
                    // Passed the quick check at registration but can't be read, so draw with the built-in face
                    // rather than fail. (Built-in fonts are known good.)
                    let built_in = read_library()[0].faces[0].as_ref().expect("the built-in family has a regular face").bytes().to_vec();
                    Font::from_bytes(built_in, FontSettings::default()).expect("the built-in font is valid")
                }
            };
            self.faces.insert(slot, font);
        }
        slot
    }

    fn font(&self, slot: (u16, usize)) -> &Font {
        &self.faces[&slot]
    }

    /// Vertical metrics for a font size in pixels. They come from the family itself, whatever its glyphs are
    /// borrowed from, so a line is the same height whichever characters are on it.
    pub fn line_metrics(&mut self, key: FontKey, size: u16) -> LineMetrics {
        if let Some(&metrics) = self.lines.get(&(key, size)) {
            return metrics;
        }
        let slot = self.load(key);
        let line = self.font(slot).horizontal_line_metrics(f32::from(size.max(1))).expect("font has horizontal metrics");
        let metrics = LineMetrics {
            height: line.new_line_size.round().max(1.0) as i32,
            ascent: line.ascent.round() as i32,
        };
        self.lines.insert((key, size), metrics);
        metrics
    }

    /// The face to take the glyph for `c` from: `key`'s own, or the first of its family's fallbacks that has one.
    /// If none does, `key` itself, which then draws its "missing glyph" shape.
    fn face_for(&mut self, key: FontKey, c: char) -> FontKey {
        let mut current = key;
        for _ in 0..FALLBACK_DEPTH {
            let slot = self.load(current);
            if self.font(slot).lookup_glyph_index(c) != 0 {
                return current;
            }
            let fallback = read_library().get(usize::from(current.family.0)).and_then(|entry| entry.fallback);
            match fallback {
                Some(family) => current = FontKey { family, ..key },
                None => break,
            }
        }
        key
    }

    /// The atlas entry for a character, rasterizing it first if needed.
    pub fn glyph(&mut self, key: FontKey, c: char, size: u16) -> Glyph {
        if let Some(&glyph) = self.glyphs.get(&(key, c, size)) {
            return glyph;
        }

        let face = self.face_for(key, c);
        let slot = self.load(face);
        let (metrics, bitmap) = self.font(slot).rasterize(c, f32::from(size.max(1)));
        let (x, y) = self.atlas.insert(metrics.width as u32, metrics.height as u32, &bitmap);
        let glyph = Glyph {
            x,
            y,
            width: metrics.width as u32,
            height: metrics.height as u32,
            left: metrics.xmin,
            bottom: metrics.ymin,
            advance: metrics.advance_width,
            face,
        };
        self.glyphs.insert((key, c, size), glyph);
        glyph
    }

    /// Positions each drawable character of a line, as the distance of its pen from the start of the line,
    /// and returns that with the line's total width. Spaces advance the pen but aren't returned.
    /// Control characters are skipped.
    ///
    /// Kerning is applied between adjacent non-space characters when the font has a kern table.
    /// It is never applied across a space, so the width of `a b` is exactly width(`a`) + width(` `) + width(`b`),
    /// which is what lets word wrapping add word widths up.
    pub fn place(&mut self, key: FontKey, size: u16, text: &str) -> (Vec<(char, f32)>, f32) {
        let px = f32::from(size.max(1));
        let mut placed = Vec::with_capacity(text.len());
        let mut pen = 0.0;
        let mut prev: Option<(char, FontKey)> = None;

        for c in text.chars().filter(|c| !c.is_control()) {
            let glyph = self.glyph(key, c, size);
            if c == ' ' {
                pen += glyph.advance;
                prev = None;
                continue;
            }
            // Kerning is a property of one font's glyph pairs, so it only applies between two from the same face.
            if let Some((p, face)) = prev {
                if face == glyph.face {
                    let slot = self.load(face);
                    pen += self.font(slot).horizontal_kern(p, c, px).unwrap_or(0.0);
                }
            }
            placed.push((c, pen));
            pen += glyph.advance;
            prev = Some((c, glyph.face));
        }
        (placed, pen)
    }

    /// Where each character of a line starts, as the distance from the start of the line, followed by where the line
    /// ends: one more number than the line has characters. These are the same positions `place` draws the characters at
    /// (a space starts where the pen is when it is reached), which is what turns a caret position into a place on the
    /// line and back. Control characters don't take any room, so they start where the next character does.
    pub fn boundaries(&mut self, key: FontKey, size: u16, text: &str) -> Vec<f32> {
        let px = f32::from(size.max(1));
        let mut starts = Vec::with_capacity(text.len() + 1);
        let mut pen = 0.0;
        let mut prev: Option<(char, FontKey)> = None;

        for c in text.chars() {
            if c.is_control() {
                starts.push(pen);
                continue;
            }
            let glyph = self.glyph(key, c, size);
            if c == ' ' {
                starts.push(pen);
                pen += glyph.advance;
                prev = None;
                continue;
            }
            if let Some((p, face)) = prev {
                if face == glyph.face {
                    let slot = self.load(face);
                    pen += self.font(slot).horizontal_kern(p, c, px).unwrap_or(0.0);
                }
            }
            starts.push(pen);
            pen += glyph.advance;
            prev = Some((c, glyph.face));
        }
        starts.push(pen);
        starts
    }

    /// The width in pixels of a line of text, as drawn.
    pub fn text_width(&mut self, key: FontKey, size: u16, text: &str) -> f32 {
        self.place(key, size, text).1
    }
}

impl Default for Fonts {
    fn default() -> Fonts {
        Fonts::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SANS: FontKey = FontKey::regular(FontFamily::Sans);
    const MONO: FontKey = FontKey::regular(FontFamily::Mono);

    #[test]
    fn weight_and_style_pick_the_face() {
        let key = |weight, style| FontKey::new(FontFamily::Sans, weight, style);
        assert_eq!(key(400, FontStyle::Normal), SANS);
        assert!(!key(500, FontStyle::Normal).bold);
        assert!(key(600, FontStyle::Normal).bold);
        assert!(key(900, FontStyle::Normal).bold);
        assert!(key(400, FontStyle::Italic).italic);
        // All eight faces map to distinct slots.
        let mut seen = std::collections::HashSet::new();
        for family in [FontFamily::Sans, FontFamily::Mono] {
            for (weight, style) in [(400, FontStyle::Normal), (400, FontStyle::Italic), (700, FontStyle::Normal), (700, FontStyle::Italic)] {
                assert!(seen.insert(FontKey::new(family, weight, style)));
            }
        }
        assert_eq!(seen.len(), 8);
    }

    #[test]
    fn bold_text_is_wider_and_italic_glyphs_differ() {
        let mut fonts = Fonts::new();
        let bold = FontKey::new(FontFamily::Sans, 700, FontStyle::Normal);
        let italic = FontKey::new(FontFamily::Sans, 400, FontStyle::Italic);
        assert!(fonts.text_width(bold, 20, "Bold text here") > fonts.text_width(SANS, 20, "Bold text here"));

        let (a, b) = (fonts.glyph(SANS, 'f', 40), fonts.glyph(italic, 'f', 40));
        let ink = |fonts: &Fonts, g: Glyph| -> Vec<u8> {
            (0..g.height).flat_map(|row| (0..g.width).map(move |col| (col, row))).map(|(c, r)| fonts.atlas.data[((g.y + r) * fonts.atlas.width + g.x + c) as usize]).collect()
        };
        assert_ne!(ink(&fonts, a), ink(&fonts, b), "the italic face draws different shapes");
    }

    const JETBRAINS: &[u8] = include_bytes!("../assets/JetBrainsMono-Regular.ttf");
    const JETBRAINS_BOLD: &[u8] = include_bytes!("../assets/JetBrainsMono-Bold.ttf");
    const LATO: &[u8] = include_bytes!("../assets/Lato-Regular.ttf");

    /// Registers a family under a name no other test uses, since families are shared by the whole process.
    fn family(name: &str, faces: [Option<&[u8]>; 4]) -> FontFamily {
        register_family(name, faces.map(|f| f.map(<[u8]>::to_vec))).expect("a valid family")
    }

    /// Not a test: prints how long registering a real system font takes.
    /// Run with `cargo test bench_register -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn bench_register_a_system_font() {
        let paths = ["C:/Windows/Fonts/segoeui.ttf", "C:/Windows/Fonts/segoeuii.ttf", "C:/Windows/Fonts/segoeuib.ttf", "C:/Windows/Fonts/segoeuiz.ttf"];
        let start = std::time::Instant::now();
        let id = register_family_files("Bench Segoe", paths.map(Some)).expect("Segoe UI is installed");
        println!("register (4 faces): {:?}", start.elapsed());
        let start = std::time::Instant::now();
        let mut fonts = Fonts::new();
        fonts.text_width(FontKey::regular(id), 16, "Hello");
        println!("first use (parse 1 face): {:?}", start.elapsed());
        let start = std::time::Instant::now();
        let mut fonts = Fonts::new();
        fonts.text_width(SANS, 16, "Hello");
        println!("first use of built-in Lato: {:?}", start.elapsed());
    }

    #[test]
    fn built_in_families_are_found_by_name() {
        assert_eq!(find_family("Sans"), Some(FontFamily::Sans));
        assert_eq!(find_family("mono"), Some(FontFamily::Mono), "ignoring case");
        assert_eq!(find_family("no such family"), None);
    }

    #[test]
    fn a_registered_family_has_its_own_id_and_its_own_look() {
        let custom = family("Test Custom Look", [Some(JETBRAINS), None, None, None]);
        assert!(custom.0 > FontFamily::Mono.0, "after the built-in ones");
        assert_eq!(find_family("TEST CUSTOM LOOK"), Some(custom));

        let mut fonts = Fonts::new();
        let mine = FontKey::regular(custom);
        // It is JetBrains Mono underneath, so it matches the built-in mono family and not the sans one.
        assert_eq!(fonts.text_width(mine, 16, "iiMM"), fonts.text_width(MONO, 16, "iiMM"));
        assert_ne!(fonts.text_width(mine, 16, "iiMM"), fonts.text_width(SANS, 16, "iiMM"));
    }

    #[test]
    fn a_style_a_family_has_no_face_for_uses_the_closest_it_has() {
        let regular_only = family("Test Regular Only", [Some(JETBRAINS), None, None, None]);
        let with_bold = family("Test With Bold", [Some(JETBRAINS), None, Some(JETBRAINS_BOLD), None]);
        let mut fonts = Fonts::new();

        let bold = |family| FontKey { family, bold: true, italic: false };
        let regular = |family| FontKey::regular(family);
        assert_eq!(fonts.glyph(bold(regular_only), 'M', 16).face, bold(regular_only), "asked for, so named, but");
        let width = |fonts: &mut Fonts, key| fonts.glyph(key, 'M', 24).height * 1000 + fonts.glyph(key, 'M', 24).width;
        assert_eq!(width(&mut fonts, bold(regular_only)), width(&mut fonts, regular(regular_only)), "drawn with the regular face");
        assert_ne!(width(&mut fonts, bold(with_bold)), width(&mut fonts, regular(with_bold)), "while this one has a real bold");
        let italic_bold = FontKey { family: with_bold, bold: true, italic: true };
        assert_eq!(width(&mut fonts, italic_bold), width(&mut fonts, bold(with_bold)), "bold italic falls back to bold");
    }

    #[test]
    fn registering_bad_families_is_refused_and_leaves_the_library_alone() {
        assert!(register_family("Test Empty", [None, None, None, None]).is_err(), "no regular face");
        assert!(register_family("Test Garbage", [Some(b"not a font".to_vec()), None, None, None]).is_err());
        assert!(register_family("  ", [Some(JETBRAINS.to_vec()), None, None, None]).is_err(), "no name");
        assert_eq!(find_family("Test Garbage"), None);

        family("Test Duplicate", [Some(JETBRAINS), None, None, None]);
        assert!(register_family("test duplicate", [Some(JETBRAINS.to_vec()), None, None, None]).is_err(), "names are unique, ignoring case");
        assert!(register_family("Sans", [Some(JETBRAINS.to_vec()), None, None, None]).is_err(), "and the built-in names are taken");
    }

    #[test]
    fn an_unknown_family_id_draws_as_sans() {
        let mut fonts = Fonts::new();
        let nowhere = FontKey::regular(FontFamily(60_000));
        assert_eq!(fonts.text_width(nowhere, 16, "Hello"), fonts.text_width(SANS, 16, "Hello"));
    }

    #[test]
    fn files_can_be_registered_by_path() {
        let path = format!("{}/assets/JetBrainsMono-Regular.ttf", env!("CARGO_MANIFEST_DIR"));
        let from_file = register_family_files("Test From File", [Some(&path), None, None, None]).unwrap();
        let mut fonts = Fonts::new();
        assert_eq!(fonts.text_width(FontKey::regular(from_file), 16, "abc"), fonts.text_width(MONO, 16, "abc"));
        assert!(register_family_files("Test Missing File", [Some("definitely/not/here.ttf"), None, None, None]).is_err());
    }

    #[test]
    fn a_missing_glyph_comes_from_the_fallback_family() {
        // A character the primary face lacks and the fallback has, found rather than assumed.
        let lato = Font::from_bytes(LATO, FontSettings::default()).unwrap();
        let jetbrains = Font::from_bytes(JETBRAINS, FontSettings::default()).unwrap();
        let symbol = ['█', '▀', '─', '│', '→', '✓', '≈', '∑']
            .into_iter()
            .find(|&c| lato.lookup_glyph_index(c) == 0 && jetbrains.lookup_glyph_index(c) != 0)
            .expect("some symbol that JetBrains Mono has and Lato doesn't");

        let primary = family("Test Fallback Primary", [Some(LATO), None, None, None]);
        let mut fonts = Fonts::new();
        let without = fonts.glyph(FontKey::regular(primary), symbol, 20);
        assert_eq!(without.face, FontKey::regular(primary), "no fallback yet: its own missing-glyph shape");

        let borrowing = family("Test Fallback Borrowing", [Some(LATO), None, None, None]);
        assert!(set_fallback(borrowing, Some(FontFamily::Mono)));
        let with = fonts.glyph(FontKey::regular(borrowing), symbol, 20);
        assert_eq!(with.face, FontKey::regular(FontFamily::Mono), "taken from the mono family");
        assert_eq!(with.advance, fonts.glyph(MONO, symbol, 20).advance, "and as wide as it is there");

        // Characters the primary has are still its own, and fallbacks only matter for missing ones.
        assert_eq!(fonts.glyph(FontKey::regular(borrowing), 'A', 20).face, FontKey::regular(borrowing));
        // A line mixing both still measures and places every character.
        let text = format!("A{symbol}B");
        assert_eq!(fonts.place(FontKey::regular(borrowing), 20, &text).0.len(), 3);
    }

    #[test]
    fn a_loop_of_fallbacks_ends() {
        let a = family("Test Loop A", [Some(LATO), None, None, None]);
        let b = family("Test Loop B", [Some(LATO), None, None, None]);
        assert!(set_fallback(a, Some(b)) && set_fallback(b, Some(a)));
        let mut fonts = Fonts::new();
        // Neither has it, and it must give up instead of going round forever.
        let glyph = fonts.glyph(FontKey::regular(a), '\u{10FFFD}', 16);
        assert_eq!(glyph.face, FontKey::regular(a));
        assert!(!set_fallback(a, Some(FontFamily(60_000))), "an unknown family can't be a fallback");
    }

    #[test]
    fn metrics_scale_with_size() {
        let mut fonts = Fonts::new();
        let small = fonts.line_metrics(SANS, 12);
        let big = fonts.line_metrics(SANS, 24);
        assert!(small.height > 0 && small.ascent > 0);
        assert!(big.height > small.height && big.ascent > small.ascent);
    }

    #[test]
    fn sans_is_proportional_and_mono_is_not() {
        let mut fonts = Fonts::new();
        assert!(fonts.text_width(SANS, 16, "iiii") < fonts.text_width(SANS, 16, "MMMM"));
        assert_eq!(fonts.text_width(MONO, 16, "iiii"), fonts.text_width(MONO, 16, "MMMM"));
    }

    #[test]
    fn width_is_additive_across_spaces() {
        let mut fonts = Fonts::new();
        let whole = fonts.text_width(SANS, 18, "AV To");
        let parts = fonts.text_width(SANS, 18, "AV") + fonts.text_width(SANS, 18, " ") + fonts.text_width(SANS, 18, "To");
        assert!((whole - parts).abs() < 1e-3, "{whole} vs {parts}");
    }

    #[test]
    fn place_skips_spaces_and_controls_but_keeps_their_width() {
        let mut fonts = Fonts::new();
        let (placed, width) = fonts.place(MONO, 16, "a\tb c");
        assert_eq!(placed.iter().map(|p| p.0).collect::<String>(), "abc");
        let advance = fonts.glyph(MONO, 'a', 16).advance;
        assert!((width - 4.0 * advance).abs() < 1e-3, "a, b, space, c");
        assert!((placed[2].1 - 3.0 * advance).abs() < 1e-3);
    }

    #[test]
    fn glyph_is_rasterized_once_and_has_coverage() {
        let mut fonts = Fonts::new();
        let a = fonts.glyph(SANS, 'M', 24);
        let b = fonts.glyph(SANS, 'M', 24);
        assert_eq!((a.x, a.y), (b.x, b.y));

        let covered = (0..a.height)
            .flat_map(|row| (0..a.width).map(move |col| (col, row)))
            .any(|(col, row)| fonts.atlas.data[((a.y + row) * fonts.atlas.width + a.x + col) as usize] > 200);
        assert!(covered, "an 'M' should have fully covered pixels");
    }

    #[test]
    fn atlas_grows_without_moving_glyphs() {
        let mut fonts = Fonts::new();
        let first = fonts.glyph(SANS, 'A', 32);
        let start_height = fonts.atlas.height;
        for c in '!'..='~' {
            for size in [20, 28, 36, 48, 64] {
                fonts.glyph(SANS, c, size);
            }
        }
        assert!(fonts.atlas.height > start_height, "test should have forced growth");
        let again = fonts.glyph(SANS, 'A', 32);
        assert_eq!((first.x, first.y), (again.x, again.y));
    }
}
