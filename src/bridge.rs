use std::collections::HashMap;

use godot::builtin::{Color, PackedByteArray, Rect2, Vector2};
use godot::classes::image::Format;
use godot::classes::{Control, Image, ImageTexture};
use godot::obj::{Gd, NewGd};
use text_typeset::{DecorationKind, RenderFrame};

/// Cache for inline image textures, keyed by resource name.
#[derive(Default)]
pub struct ImageCache {
    textures: HashMap<String, Gd<ImageTexture>>,
}

impl ImageCache {
    /// Get or create a texture for an image resource.
    /// `data_fn` is called to fetch the raw image bytes if not cached.
    pub fn get_or_load(
        &mut self,
        name: &str,
        data_fn: impl FnOnce(&str) -> Option<Vec<u8>>,
    ) -> Option<&Gd<ImageTexture>> {
        if !self.textures.contains_key(name) {
            let bytes = data_fn(name)?;
            let image = load_image_from_bytes(&bytes)?;
            let tex = ImageTexture::create_from_image(&image)?;
            self.textures.insert(name.to_string(), tex);
        }
        self.textures.get(name)
    }

    pub fn clear(&mut self) {
        self.textures.clear();
    }
}

/// Try to load a Godot Image from raw bytes (PNG, JPG, WebP, etc.)
fn load_image_from_bytes(bytes: &[u8]) -> Option<Gd<Image>> {
    let mut packed = PackedByteArray::new();
    packed.resize(bytes.len());
    packed.as_mut_slice().copy_from_slice(bytes);

    // Try common formats
    let mut image = Image::new_gd();
    if image.load_png_from_buffer(&packed) == godot::global::Error::OK {
        return Some(image);
    }
    let mut image = Image::new_gd();
    if image.load_jpg_from_buffer(&packed) == godot::global::Error::OK {
        return Some(image);
    }
    let mut image = Image::new_gd();
    if image.load_webp_from_buffer(&packed) == godot::global::Error::OK {
        return Some(image);
    }
    None
}

fn to_color(c: [f32; 4]) -> Color {
    Color::from_rgba(c[0], c[1], c[2], c[3])
}

fn to_rect2(r: [f32; 4]) -> Rect2 {
    Rect2::new(Vector2::new(r[0], r[1]), Vector2::new(r[2], r[3]))
}

fn to_rect2_h(r: [f32; 4], h_offset: f32) -> Rect2 {
    Rect2::new(
        Vector2::new(r[0] - h_offset, r[1]),
        Vector2::new(r[2], r[3]),
    )
}

/// Upload atlas pixel data to a Godot ImageTexture if the atlas has changed.
pub fn update_atlas(frame: &RenderFrame, atlas_tex: &mut Gd<ImageTexture>) {
    if !frame.atlas_dirty || frame.atlas_width == 0 || frame.atlas_height == 0 {
        return;
    }

    let mut packed = PackedByteArray::new();
    packed.resize(frame.atlas_pixels.len());
    {
        let slice = packed.as_mut_slice();
        slice.copy_from_slice(&frame.atlas_pixels);
    }

    if let Some(image) = Image::create_from_data(
        frame.atlas_width as i32,
        frame.atlas_height as i32,
        false,
        Format::RGBA8,
        &packed,
    ) {
        atlas_tex.set_image(&image);
    }
}

/// Draw a RenderFrame onto a Control's canvas.
///
/// Draw order: background decorations -> glyphs -> foreground decorations.
/// This ensures selections render behind text, while cursor/underlines render on top.
pub fn draw_frame(
    control: &mut Control,
    frame: &RenderFrame,
    atlas_tex: &Gd<ImageTexture>,
    h_offset: f32,
    image_cache: &ImageCache,
) {
    // Pass 1: Background decorations (behind text)
    for deco in &frame.decorations {
        match deco.kind {
            DecorationKind::Selection
            | DecorationKind::CellSelection
            | DecorationKind::Background
            | DecorationKind::BlockBackground
            | DecorationKind::TableCellBackground
            | DecorationKind::TableBorder => {
                let rect = to_rect2_h(deco.rect, h_offset);
                let color = to_color(deco.color);
                if matches!(deco.kind, DecorationKind::TableBorder) {
                    control.draw_rect_ex(rect, color).filled(false).done();
                } else {
                    control.draw_rect(rect, color);
                }
            }
            _ => {}
        }
    }

    // Pass 2: Glyph quads from the atlas
    for glyph in &frame.glyphs {
        let screen_rect = to_rect2_h(glyph.screen, h_offset);
        let atlas_rect = to_rect2(glyph.atlas); // atlas rect is NOT offset
        let color = to_color(glyph.color);
        control
            .draw_texture_rect_region_ex(atlas_tex, screen_rect, atlas_rect)
            .modulate(color)
            .done();
    }

    // Pass 3: Inline images
    for img in &frame.images {
        if let Some(tex) = image_cache.textures.get(&img.name) {
            let rect = to_rect2_h(img.screen, h_offset);
            control.draw_texture_rect(tex, rect, false);
        }
    }

    // Pass 4: Foreground decorations (on top of text)
    for deco in &frame.decorations {
        let rect = to_rect2_h(deco.rect, h_offset);
        let color = to_color(deco.color);
        match deco.kind {
            DecorationKind::Cursor => {
                control.draw_rect(rect, color);
            }
            DecorationKind::Underline | DecorationKind::Overline => {
                let y = if matches!(deco.kind, DecorationKind::Overline) {
                    rect.position.y
                } else {
                    rect.position.y + rect.size.y
                };
                control.draw_line(
                    Vector2::new(rect.position.x, y),
                    Vector2::new(rect.position.x + rect.size.x, y),
                    color,
                );
            }
            DecorationKind::Strikeout => {
                let y = rect.position.y + rect.size.y / 2.0;
                control.draw_line(
                    Vector2::new(rect.position.x, y),
                    Vector2::new(rect.position.x + rect.size.x, y),
                    color,
                );
            }
            _ => {} // Background decorations already drawn in pass 1
        }
    }
}
