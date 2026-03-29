use godot::classes::FontFile;
use godot::obj::Gd;
use godot::prelude::*;
use text_typeset::{FontFaceId, Typesetter};

// Embedded default fonts (NotoSans, SIL Open Font License)
static EMBEDDED_REGULAR: &[u8] = include_bytes!("NotoSans-Regular.ttf");
static EMBEDDED_BOLD: &[u8] = include_bytes!("NotoSans-Bold.ttf");
static EMBEDDED_ITALIC: &[u8] = include_bytes!("NotoSans-Italic.ttf");
static EMBEDDED_BOLD_ITALIC: &[u8] = include_bytes!("NotoSans-BoldItalic.ttf");

pub struct FontSlots<'a> {
    pub default: &'a Option<Gd<FontFile>>,
    pub bold: &'a Option<Gd<FontFile>>,
    pub italic: &'a Option<Gd<FontFile>>,
    pub bold_italic: &'a Option<Gd<FontFile>>,
    pub monospace: &'a Option<Gd<FontFile>>,
}

#[derive(Default)]
pub struct FontIds {
    pub default: Option<FontFaceId>,
    pub bold: Option<FontFaceId>,
    pub italic: Option<FontFaceId>,
    pub bold_italic: Option<FontFaceId>,
    pub monospace: Option<FontFaceId>,
}

/// Extract raw font bytes from a FontFile. Returns None if data is empty.
fn extract_font_bytes(font: &Gd<FontFile>) -> Option<Vec<u8>> {
    let packed = font.get_data();
    let bytes = packed.as_slice().to_vec();
    if bytes.is_empty() { None } else { Some(bytes) }
}

/// Try to register a font from a Godot FontFile, catching panics from invalid font data.
fn try_register(ts: &mut Typesetter, font: &Gd<FontFile>) -> Option<FontFaceId> {
    let bytes = extract_font_bytes(font)?;
    try_register_bytes(ts, &bytes)
}

/// Try to register a font from raw bytes, catching panics from invalid font data.
fn try_register_bytes(ts: &mut Typesetter, bytes: &[u8]) -> Option<FontFaceId> {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| ts.register_font(bytes))) {
        Ok(id) => Some(id),
        Err(_) => {
            godot_warn!("RichTextEdit: Failed to register font (invalid font data).");
            None
        }
    }
}

/// Register all provided font slots with the typesetter.
/// Falls back to embedded NotoSans fonts for any unset slots.
pub fn register_fonts(ts: &mut Typesetter, slots: &FontSlots, default_size: f32) -> FontIds {
    let mut ids = FontIds::default();

    // Default font: user-provided or embedded
    if let Some(font) = slots.default
        && let Some(id) = try_register(ts, font)
    {
        ts.set_default_font(id, default_size);
        ids.default = Some(id);
    }
    if ids.default.is_none()
        && let Some(id) = try_register_bytes(ts, EMBEDDED_REGULAR)
    {
        ts.set_default_font(id, default_size);
        ids.default = Some(id);
    }

    // Bold: user-provided or embedded
    ids.bold = slots
        .bold
        .as_ref()
        .and_then(|f| try_register(ts, f))
        .or_else(|| try_register_bytes(ts, EMBEDDED_BOLD));

    // Italic: user-provided or embedded
    ids.italic = slots
        .italic
        .as_ref()
        .and_then(|f| try_register(ts, f))
        .or_else(|| try_register_bytes(ts, EMBEDDED_ITALIC));

    // Bold italic: user-provided or embedded
    ids.bold_italic = slots
        .bold_italic
        .as_ref()
        .and_then(|f| try_register(ts, f))
        .or_else(|| try_register_bytes(ts, EMBEDDED_BOLD_ITALIC));

    // Monospace: user-provided only (no embedded monospace)
    if let Some(font) = slots.monospace {
        ids.monospace = try_register(ts, font);
        if let Some(id) = ids.monospace
            && let Some(name) = ts.font_family_name(id)
        {
            ts.set_generic_family("monospace", &name);
        }
    }

    ids
}
