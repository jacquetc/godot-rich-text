#![allow(dead_code)]
//! Per-widget typesetter adapter for godot-rich-text.
//!
//! Text-typeset splits its public API into a shared
//! [`TextFontService`] (fonts + glyph atlas + shaper cache) and a
//! per-widget [`DocumentFlow`] (viewport, zoom, scroll, flow layout,
//! cursor). That split is the right shape for host frameworks where
//! many widgets can share one GPU atlas — for example fern-ui, where
//! every rich-text widget in a window pulls from the same
//! `SharedTypesetter`.
//!
//! godot-rich-text is not that kind of framework. Each Godot node
//! owns its own rendering state and registers its own fonts; there
//! is no benefit to sharing an atlas across `RichTextView` /
//! `RichTextEdit` instances, and each node's lifetime is
//! independent. The correct composition here is therefore
//! "per-widget owns both halves", and this [`Typesetter`] adapter
//! is exactly that: a thin wrapper that stores a
//! `TextFontService` + `DocumentFlow` side by side and forwards the
//! pre-split method surface the godot-rich-text implementation
//! uses. Every call routes to the right half internally, so the
//! split's correctness properties still hold — this is a
//! composition adapter, not a compatibility shim.

use text_typeset::{
    AtlasSnapshot, BlockVisualInfo, CharacterGeometry, ContentWidthMode, CursorDisplay,
    DocumentFlow, FontFaceId, HitTestResult, InlineMarkup, ParagraphResult, RelayoutError,
    RenderFrame, SingleLineResult, TextFontService, TextFormat,
};

use text_typeset::layout::block::BlockLayoutParams;
use text_typeset::layout::frame::FrameLayoutParams;
use text_typeset::layout::table::TableLayoutParams;

use text_document::FlowSnapshot;

/// Per-widget typesetter bundling a [`TextFontService`] and a
/// [`DocumentFlow`]. See the [module docs](self) for why
/// godot-rich-text composes them owned-side-by-side instead of
/// sharing.
pub struct Typesetter {
    pub service: TextFontService,
    pub flow: DocumentFlow,
}

impl Typesetter {
    pub fn new() -> Self {
        Self {
            service: TextFontService::new(),
            flow: DocumentFlow::new(),
        }
    }

    // ── Font registration (service-side) ──────────────────────

    pub fn register_font(&mut self, data: &[u8]) -> FontFaceId {
        self.service.register_font(data)
    }

    pub fn register_font_as(
        &mut self,
        data: &[u8],
        family: &str,
        weight: u16,
        italic: bool,
    ) -> FontFaceId {
        self.service.register_font_as(data, family, weight, italic)
    }

    pub fn set_default_font(&mut self, face: FontFaceId, size_px: f32) {
        self.service.set_default_font(face, size_px);
    }

    pub fn set_generic_family(&mut self, generic: &str, family: &str) {
        self.service.set_generic_family(generic, family);
    }

    pub fn font_family_name(&self, face_id: FontFaceId) -> Option<String> {
        self.service.font_family_name(face_id)
    }

    pub fn set_scale_factor(&mut self, scale_factor: f32) {
        self.service.set_scale_factor(scale_factor);
    }

    pub fn scale_factor(&self) -> f32 {
        self.service.scale_factor()
    }

    pub fn atlas_snapshot(&mut self, advance_generation: bool) -> AtlasSnapshot<'_> {
        self.service.atlas_snapshot(advance_generation)
    }

    // ── Viewport / scroll / zoom (flow-side) ──────────────────

    pub fn set_viewport(&mut self, width: f32, height: f32) {
        self.flow.set_viewport(width, height);
    }

    pub fn set_content_width(&mut self, width: f32) {
        self.flow.set_content_width(width);
    }

    pub fn set_content_width_auto(&mut self) {
        self.flow.set_content_width_auto();
    }

    pub fn layout_width(&self) -> f32 {
        self.flow.layout_width()
    }

    pub fn set_scroll_offset(&mut self, offset: f32) {
        self.flow.set_scroll_offset(offset);
    }

    pub fn content_height(&self) -> f32 {
        self.flow.content_height()
    }

    pub fn max_content_width(&self) -> f32 {
        self.flow.max_content_width()
    }

    pub fn set_zoom(&mut self, zoom: f32) {
        self.flow.set_zoom(zoom);
    }

    pub fn zoom(&self) -> f32 {
        self.flow.zoom()
    }

    pub fn content_width_mode(&self) -> ContentWidthMode {
        self.flow.content_width_mode()
    }

    // ── Layout ────────────────────────────────────────────────

    pub fn layout_full(&mut self, flow: &FlowSnapshot) {
        self.flow.layout_full(&self.service, flow);
    }

    pub fn layout_blocks(&mut self, block_params: Vec<BlockLayoutParams>) {
        self.flow.layout_blocks(&self.service, block_params);
    }

    pub fn add_frame(&mut self, params: &FrameLayoutParams) {
        self.flow.add_frame(&self.service, params);
    }

    pub fn add_table(&mut self, params: &TableLayoutParams) {
        self.flow.add_table(&self.service, params);
    }

    pub fn relayout_block(&mut self, params: &BlockLayoutParams) -> Result<(), RelayoutError> {
        self.flow.relayout_block(&self.service, params)
    }

    // ── Rendering ─────────────────────────────────────────────

    pub fn render(&mut self) -> &RenderFrame {
        self.flow.render(&mut self.service)
    }

    pub fn render_block_only(&mut self, block_id: usize) -> &RenderFrame {
        self.flow.render_block_only(&mut self.service, block_id)
    }

    pub fn render_cursor_only(&mut self) -> &RenderFrame {
        self.flow.render_cursor_only(&mut self.service)
    }

    // ── Single-line & paragraph layout ────────────────────────

    pub fn layout_single_line(
        &mut self,
        text: &str,
        format: &TextFormat,
        max_width: Option<f32>,
    ) -> SingleLineResult {
        self.flow
            .layout_single_line(&mut self.service, text, format, max_width)
    }

    pub fn layout_paragraph(
        &mut self,
        text: &str,
        format: &TextFormat,
        max_width: f32,
        max_lines: Option<usize>,
    ) -> ParagraphResult {
        self.flow
            .layout_paragraph(&mut self.service, text, format, max_width, max_lines)
    }

    pub fn layout_single_line_markup(
        &mut self,
        markup: &InlineMarkup,
        format: &TextFormat,
        max_width: Option<f32>,
    ) -> SingleLineResult {
        self.flow
            .layout_single_line_markup(&mut self.service, markup, format, max_width)
    }

    pub fn layout_paragraph_markup(
        &mut self,
        markup: &InlineMarkup,
        format: &TextFormat,
        max_width: f32,
        max_lines: Option<usize>,
    ) -> ParagraphResult {
        self.flow
            .layout_paragraph_markup(&mut self.service, markup, format, max_width, max_lines)
    }

    // ── Hit testing & geometry ────────────────────────────────

    pub fn hit_test(&self, x: f32, y: f32) -> Option<HitTestResult> {
        self.flow.hit_test(x, y)
    }

    pub fn character_geometry(
        &self,
        block_id: usize,
        char_start: usize,
        char_end: usize,
    ) -> Vec<CharacterGeometry> {
        self.flow.character_geometry(block_id, char_start, char_end)
    }

    pub fn caret_rect(&self, position: usize) -> [f32; 4] {
        self.flow.caret_rect(position)
    }

    // ── Cursor & colors ───────────────────────────────────────

    pub fn set_cursor(&mut self, cursor: &CursorDisplay) {
        self.flow.set_cursor(cursor);
    }

    pub fn set_cursors(&mut self, cursors: &[CursorDisplay]) {
        self.flow.set_cursors(cursors);
    }

    pub fn set_selection_color(&mut self, color: [f32; 4]) {
        self.flow.set_selection_color(color);
    }

    pub fn set_cursor_color(&mut self, color: [f32; 4]) {
        self.flow.set_cursor_color(color);
    }

    pub fn set_text_color(&mut self, color: [f32; 4]) {
        self.flow.set_text_color(color);
    }

    // ── Scrolling helpers ─────────────────────────────────────

    pub fn block_visual_info(&self, block_id: usize) -> Option<BlockVisualInfo> {
        self.flow.block_visual_info(block_id)
    }

    pub fn is_block_in_table(&self, block_id: usize) -> bool {
        self.flow.is_block_in_table(block_id)
    }

    pub fn scroll_to_position(&mut self, position: usize) -> f32 {
        self.flow.scroll_to_position(position)
    }

    pub fn ensure_caret_visible(&mut self) -> Option<f32> {
        self.flow.ensure_caret_visible()
    }
}

impl Default for Typesetter {
    fn default() -> Self {
        Self::new()
    }
}
