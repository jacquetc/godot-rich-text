use godot::builtin::{Color, GString, Vector2};
use godot::classes::canvas_item::TextureFilter;
use godot::classes::control::FocusMode;
use godot::classes::notify::ControlNotification;
use godot::classes::{
    Control, DisplayServer, FontFile, IControl, ImageTexture, InputEvent, VScrollBar,
};
use godot::obj::{Base, Gd, WithBaseField};
use godot::prelude::*;

use text_document::{
    DocumentEvent, MoveMode, MoveOperation, SelectionType, TextCursor, TextDocument,
};
use text_typeset::{CursorDisplay, HitRegion, Typesetter};

use crate::bridge::{self, ImageCache};
use crate::fonts::{self, FontSlots};
use crate::input::{self, InputAction};
use crate::rich_text_edit::WrapMode;

// ---------------------------------------------------------------------------
// RichTextView - read-only rich text display
// ---------------------------------------------------------------------------

#[derive(GodotClass)]
#[class(base = Control)]
pub struct RichTextView {
    base: Base<Control>,

    // --- Exported properties ---
    #[export]
    text: GString,
    #[export]
    html_text: GString,
    #[export]
    markdown_text: GString,

    #[export]
    wrap_mode: WrapMode,

    #[export]
    default_font: Option<Gd<FontFile>>,
    #[export]
    bold_font: Option<Gd<FontFile>>,
    #[export]
    italic_font: Option<Gd<FontFile>>,
    #[export]
    bold_italic_font: Option<Gd<FontFile>>,
    #[export]
    monospace_font: Option<Gd<FontFile>>,

    #[export]
    default_font_size: i32,

    #[export(range = (0.1, 10.0, 0.05))]
    zoom: f32,

    #[export]
    scroll_active: bool,

    #[export]
    selectable: bool,
    #[export]
    selection_color: Color,
    #[export]
    text_color: Color,

    // --- Internal state ---
    document: Option<TextDocument>,
    typesetter: Option<Typesetter>,
    cursor: Option<TextCursor>,
    atlas_texture: Option<Gd<ImageTexture>>,
    image_cache: ImageCache,
    needs_redraw: bool,
    scroll_offset: f32,
    v_scrollbar: Option<Gd<VScrollBar>>,
    preferred_x: Option<f32>,
}

#[godot_api]
impl IControl for RichTextView {
    fn init(base: Base<Control>) -> Self {
        Self {
            base,
            text: GString::new(),
            html_text: GString::new(),
            markdown_text: GString::new(),
            wrap_mode: WrapMode::None,
            default_font: None,
            bold_font: None,
            italic_font: None,
            bold_italic_font: None,
            monospace_font: None,
            default_font_size: 24,
            zoom: 1.0,
            scroll_active: true,
            selectable: false,
            selection_color: Color::from_rgba(0.26, 0.52, 0.96, 0.3),
            text_color: Color::from_rgba(0.0, 0.0, 0.0, 1.0),
            document: None,
            typesetter: None,
            cursor: None,
            atlas_texture: None,
            image_cache: ImageCache::default(),
            needs_redraw: false,
            scroll_offset: 0.0,
            v_scrollbar: None,
            preferred_x: None,
        }
    }

    fn ready(&mut self) {
        self.base_mut().set_clip_contents(true);
        self.base_mut().set_texture_filter(TextureFilter::LINEAR);

        let doc = TextDocument::new();
        let mut ts = Typesetter::new();
        let atlas_tex = ImageTexture::new_gd();

        // Register fonts (falls back to embedded NotoSans if none set)
        let slots = FontSlots {
            default: &self.default_font,
            bold: &self.bold_font,
            italic: &self.italic_font,
            bold_italic: &self.bold_italic_font,
            monospace: &self.monospace_font,
        };
        let _ = fonts::register_fonts(&mut ts, &slots, self.default_font_size as f32);

        // Set viewport size, zoom, and wrap mode
        let size = self.base().get_size();
        ts.set_viewport(size.x, size.y);
        ts.set_zoom(self.zoom);
        match self.wrap_mode {
            WrapMode::None => ts.set_content_width(f32::INFINITY),
            WrapMode::Word => ts.set_content_width_auto(),
        }

        // Load initial content from exported properties.
        // set_markdown and set_html return Operations that must complete before layout.
        if !self.markdown_text.is_empty() {
            if let Ok(op) = doc.set_markdown(&self.markdown_text.to_string()) {
                let _ = op.wait();
            }
            let _ = doc.poll_events();
        } else if !self.html_text.is_empty() {
            if let Ok(op) = doc.set_html(&self.html_text.to_string()) {
                let _ = op.wait();
            }
            let _ = doc.poll_events();
        } else if !self.text.is_empty() {
            let _ = doc.set_plain_text(&self.text.to_string());
            let _ = doc.poll_events();
        }

        ts.set_text_color([
            self.text_color.r,
            self.text_color.g,
            self.text_color.b,
            self.text_color.a,
        ]);

        // Selection support
        let cursor = if self.selectable {
            self.base_mut().set_focus_mode(FocusMode::CLICK);
            ts.set_selection_color([
                self.selection_color.r,
                self.selection_color.g,
                self.selection_color.b,
                self.selection_color.a,
            ]);
            Some(doc.cursor())
        } else {
            None
        };

        // Initial layout
        let flow = doc.snapshot_flow();
        ts.layout_full(&flow);

        self.document = Some(doc);
        self.typesetter = Some(ts);
        self.cursor = cursor;
        self.atlas_texture = Some(atlas_tex);

        // Create VScrollBar
        if self.scroll_active {
            let mut scrollbar = VScrollBar::new_alloc();
            scrollbar.set_name("_v_scrollbar");
            self.base_mut().add_child(&scrollbar);
            let callable = self.base().callable("on_v_scroll_changed");
            scrollbar.connect("value_changed", &callable);
            self.v_scrollbar = Some(scrollbar);
        }

        self.update_scrollbar();
        self.base_mut().queue_redraw();
    }

    fn process(&mut self, _delta: f64) {
        let mut emit_document_loaded = false;

        if let (Some(doc), Some(ts)) = (&self.document, &mut self.typesetter) {
            let events = doc.poll_events();
            let mut needs_full_layout = false;

            for event in &events {
                match event {
                    DocumentEvent::ContentsChanged { .. }
                    | DocumentEvent::FormatChanged { .. }
                    | DocumentEvent::DocumentReset
                    | DocumentEvent::FlowElementsInserted { .. }
                    | DocumentEvent::FlowElementsRemoved { .. }
                    | DocumentEvent::BlockCountChanged(_) => {
                        needs_full_layout = true;
                        self.needs_redraw = true;
                    }
                    DocumentEvent::LongOperationFinished { .. } => {
                        emit_document_loaded = true;
                    }
                    _ => {}
                }
            }

            if needs_full_layout {
                let flow = doc.snapshot_flow();
                ts.layout_full(&flow);
                self.needs_redraw = true;
            }
        }

        if self.needs_redraw {
            self.update_scrollbar();
        }

        if emit_document_loaded {
            self.base_mut().emit_signal("document_loaded", &[]);
        }

        if self.needs_redraw {
            self.needs_redraw = false;
            self.base_mut().queue_redraw();
        }
    }

    fn draw(&mut self) {
        let _size = self.base().get_size();

        let mut ts = self.typesetter.take();
        let mut atlas_tex = self.atlas_texture.take();
        let mut img_cache = std::mem::take(&mut self.image_cache);
        let doc = self.document.clone();
        if let (Some(ts), Some(atlas_tex)) = (&mut ts, &mut atlas_tex) {
            let frame = ts.render();
            if let Some(doc) = &doc {
                for img in &frame.images {
                    img_cache.get_or_load(&img.name, |name| doc.resource(name).ok().flatten());
                }
            }
            bridge::update_atlas(frame, atlas_tex);
            bridge::draw_frame(
                self.base_mut().upcast_mut::<Control>(),
                frame,
                atlas_tex,
                0.0,
                &img_cache,
            );
        }
        self.typesetter = ts;
        self.atlas_texture = atlas_tex;
        self.image_cache = img_cache;
    }

    fn gui_input(&mut self, event: Gd<InputEvent>) {
        if self.cursor.is_some() {
            self.handle_selectable_input(&event);
        } else {
            self.handle_readonly_input(&event);
        }
    }

    fn on_notification(&mut self, what: ControlNotification) {
        if what == ControlNotification::RESIZED {
            let size = self.base().get_size();
            if let Some(ts) = &mut self.typesetter {
                ts.set_viewport(size.x, size.y);
                match self.wrap_mode {
                    WrapMode::None => ts.set_content_width(f32::INFINITY),
                    WrapMode::Word => ts.set_content_width_auto(),
                }
                if let Some(doc) = &self.document {
                    let flow = doc.snapshot_flow();
                    ts.layout_full(&flow);
                }
            }
            self.update_scrollbar();
            self.base_mut().queue_redraw();
        }
    }
}

// ---------------------------------------------------------------------------
// Signals and GDScript-callable methods
// ---------------------------------------------------------------------------

#[godot_api]
impl RichTextView {
    // --- Signals ---
    #[signal]
    fn link_clicked(url: GString);
    #[signal]
    fn image_clicked(name: GString);
    #[signal]
    fn document_loaded();
    #[signal]
    fn selection_changed();

    // --- Content methods ---

    #[func]
    fn set_plain_text(&mut self, value: GString) {
        if let Some(doc) = &self.document {
            let _ = doc.set_plain_text(&value.to_string());
            self.image_cache.clear();
        }
    }

    #[func]
    fn get_plain_text(&self) -> GString {
        self.document
            .as_ref()
            .and_then(|doc| doc.to_plain_text().ok())
            .map(|s| GString::from(s.as_str()))
            .unwrap_or_default()
    }

    #[func]
    fn set_html(&mut self, value: GString) {
        if let Some(doc) = &self.document {
            let _ = doc.set_html(&value.to_string());
            self.image_cache.clear();
        }
    }

    #[func]
    fn get_html(&self) -> GString {
        self.document
            .as_ref()
            .and_then(|doc| doc.to_html().ok())
            .map(|s| GString::from(s.as_str()))
            .unwrap_or_default()
    }

    #[func]
    fn set_markdown(&mut self, value: GString) {
        if let Some(doc) = &self.document {
            let _ = doc.set_markdown(&value.to_string());
            self.image_cache.clear();
        }
    }

    #[func]
    fn get_markdown(&self) -> GString {
        self.document
            .as_ref()
            .and_then(|doc| doc.to_markdown().ok())
            .map(|s| GString::from(s.as_str()))
            .unwrap_or_default()
    }

    #[func]
    fn clear(&mut self) {
        if let Some(doc) = &self.document {
            let _ = doc.clear();
            self.image_cache.clear();
        }
    }

    // --- Query methods ---

    #[func]
    fn get_character_count(&self) -> i32 {
        self.document
            .as_ref()
            .map(|d| d.character_count() as i32)
            .unwrap_or(0)
    }

    #[func]
    fn get_word_count(&self) -> i32 {
        self.document
            .as_ref()
            .map(|d| d.stats().word_count as i32)
            .unwrap_or(0)
    }

    #[func]
    fn get_block_count(&self) -> i32 {
        self.document
            .as_ref()
            .map(|d| d.block_count() as i32)
            .unwrap_or(0)
    }

    /// Called by the VScrollBar's value_changed signal.
    #[func]
    fn on_v_scroll_changed(&mut self, value: f64) {
        self.scroll_offset = value as f32;
        if let Some(ts) = &mut self.typesetter {
            ts.set_scroll_offset(self.scroll_offset);
        }
        self.needs_redraw = true;
        self.base_mut().queue_redraw();
    }

    // --- Selection methods ---

    #[func]
    fn get_selected_text(&self) -> GString {
        self.cursor
            .as_ref()
            .and_then(|c| c.selected_text().ok())
            .map(|s| GString::from(s.as_str()))
            .unwrap_or_default()
    }

    #[func]
    fn select_all(&mut self) {
        if let Some(cursor) = &self.cursor {
            cursor.select(SelectionType::Document);
            self.update_cursor_display();
            self.base_mut().emit_signal("selection_changed", &[]);
        }
    }

    #[func]
    fn deselect(&mut self) {
        if let Some(cursor) = &self.cursor {
            cursor.clear_selection();
            self.update_cursor_display();
            self.base_mut().emit_signal("selection_changed", &[]);
        }
    }

    // --- Zoom methods ---

    #[func]
    fn set_zoom_level(&mut self, zoom: f32) {
        self.zoom = zoom.clamp(0.1, 10.0);
        if let Some(ts) = &mut self.typesetter {
            ts.set_zoom(self.zoom);
            // Relayout needed: layout_width changes with zoom in Auto mode
            if let Some(doc) = &self.document {
                let flow = doc.snapshot_flow();
                ts.layout_full(&flow);
            }
            self.needs_redraw = true;
        }
        self.update_scrollbar();
    }

    #[func]
    fn get_zoom_level(&self) -> f32 {
        self.zoom
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

impl RichTextView {
    fn handle_readonly_input(&mut self, event: &Gd<InputEvent>) {
        let action = input::translate_input(event);
        match action {
            InputAction::Click { position } => {
                let hit_region = self
                    .typesetter
                    .as_ref()
                    .and_then(|ts| ts.hit_test(position.x, position.y))
                    .map(|hit| hit.region);

                match hit_region {
                    Some(HitRegion::Link { href }) => {
                        let url = GString::from(href.as_str());
                        self.base_mut()
                            .emit_signal("link_clicked", &[url.to_variant()]);
                    }
                    Some(HitRegion::Image { name }) => {
                        let name = GString::from(name.as_str());
                        self.base_mut()
                            .emit_signal("image_clicked", &[name.to_variant()]);
                    }
                    _ => {}
                }
            }
            InputAction::ScrollUp => self.scroll_by(-40.0),
            InputAction::ScrollDown => self.scroll_by(40.0),
            _ => {}
        }
    }

    fn handle_selectable_input(&mut self, event: &Gd<InputEvent>) {
        let action = input::translate_input(event);

        // Clear sticky X for any non-vertical action
        if !matches!(
            action,
            InputAction::None
                | InputAction::MoveUp
                | InputAction::MoveDown
                | InputAction::SelectUp
                | InputAction::SelectDown
                | InputAction::ScrollUp
                | InputAction::ScrollDown
        ) {
            self.preferred_x = None;
        }

        match action {
            InputAction::None => return,

            // Mouse
            InputAction::Click { position } => self.handle_click(position),
            InputAction::ShiftClick { position } => {
                // Extend selection to clicked position
                let hit = self
                    .typesetter
                    .as_ref()
                    .and_then(|ts| ts.hit_test(position.x, position.y));
                if let Some(hit) = hit
                    && let Some(cursor) = &self.cursor
                {
                    cursor.set_position(hit.position, MoveMode::KeepAnchor);
                    self.update_cursor_display();
                    self.base_mut().emit_signal("selection_changed", &[]);
                }
            }
            InputAction::DoubleClick { position } => {
                self.handle_click(position);
                if let Some(cursor) = &self.cursor {
                    cursor.select(SelectionType::WordUnderCursor);
                    self.update_cursor_display();
                    self.base_mut().emit_signal("selection_changed", &[]);
                }
            }
            InputAction::DragSelect { position } => {
                self.handle_drag_select(position);
                self.base_mut().accept_event();
                return;
            }

            // Navigation (moves cursor, clears selection)
            InputAction::MoveLeft => self.move_cursor(MoveOperation::Left, MoveMode::MoveAnchor),
            InputAction::MoveRight => self.move_cursor(MoveOperation::Right, MoveMode::MoveAnchor),
            InputAction::MoveUp => self.move_cursor_vertical(-1, MoveMode::MoveAnchor),
            InputAction::MoveDown => self.move_cursor_vertical(1, MoveMode::MoveAnchor),
            InputAction::MoveWordLeft => {
                self.move_cursor(MoveOperation::WordLeft, MoveMode::MoveAnchor)
            }
            InputAction::MoveWordRight => {
                self.move_cursor(MoveOperation::WordRight, MoveMode::MoveAnchor)
            }
            InputAction::MoveHome => {
                self.move_cursor(MoveOperation::StartOfBlock, MoveMode::MoveAnchor)
            }
            InputAction::MoveEnd => {
                self.move_cursor(MoveOperation::EndOfBlock, MoveMode::MoveAnchor)
            }
            InputAction::MoveDocStart => {
                self.move_cursor(MoveOperation::Start, MoveMode::MoveAnchor)
            }
            InputAction::MoveDocEnd => self.move_cursor(MoveOperation::End, MoveMode::MoveAnchor),

            // Selection
            InputAction::SelectLeft => self.move_cursor(MoveOperation::Left, MoveMode::KeepAnchor),
            InputAction::SelectRight => {
                self.move_cursor(MoveOperation::Right, MoveMode::KeepAnchor)
            }
            InputAction::SelectUp => self.move_cursor_vertical(-1, MoveMode::KeepAnchor),
            InputAction::SelectDown => self.move_cursor_vertical(1, MoveMode::KeepAnchor),
            InputAction::SelectWordLeft => {
                self.move_cursor(MoveOperation::WordLeft, MoveMode::KeepAnchor)
            }
            InputAction::SelectWordRight => {
                self.move_cursor(MoveOperation::WordRight, MoveMode::KeepAnchor)
            }
            InputAction::SelectHome => {
                self.move_cursor(MoveOperation::StartOfBlock, MoveMode::KeepAnchor)
            }
            InputAction::SelectEnd => {
                self.move_cursor(MoveOperation::EndOfBlock, MoveMode::KeepAnchor)
            }
            InputAction::SelectDocStart => {
                self.move_cursor(MoveOperation::Start, MoveMode::KeepAnchor)
            }
            InputAction::SelectDocEnd => {
                self.move_cursor(MoveOperation::End, MoveMode::KeepAnchor)
            }
            InputAction::SelectAll => {
                if let Some(cursor) = &self.cursor {
                    cursor.select(SelectionType::Document);
                    self.update_cursor_display();
                    self.base_mut().emit_signal("selection_changed", &[]);
                }
            }

            // Clipboard
            InputAction::Copy => self.clipboard_copy(),

            // Scroll
            InputAction::ScrollUp => {
                self.scroll_by(-40.0);
                self.base_mut().accept_event();
                return;
            }
            InputAction::ScrollDown => {
                self.scroll_by(40.0);
                self.base_mut().accept_event();
                return;
            }

            // Ignore all editing actions
            _ => return,
        }

        self.base_mut().accept_event();
    }

    fn handle_click(&mut self, position: Vector2) {
        let hit = self
            .typesetter
            .as_ref()
            .and_then(|ts| ts.hit_test(position.x, position.y));

        let Some(hit) = hit else { return };

        match &hit.region {
            HitRegion::Link { href } => {
                let url = GString::from(href.as_str());
                self.base_mut()
                    .emit_signal("link_clicked", &[url.to_variant()]);
                return;
            }
            HitRegion::Image { name } => {
                let name = GString::from(name.as_str());
                self.base_mut()
                    .emit_signal("image_clicked", &[name.to_variant()]);
                return;
            }
            _ => {}
        }

        if let Some(cursor) = &self.cursor {
            cursor.set_position(hit.position, MoveMode::MoveAnchor);
            self.update_cursor_display();
        }
    }

    fn handle_drag_select(&mut self, mouse_pos: Vector2) {
        let view_height = self.base().get_size().y;
        let auto_scroll_margin = 20.0;
        let auto_scroll_speed = 60.0;

        if mouse_pos.y < auto_scroll_margin {
            let intensity = (auto_scroll_margin - mouse_pos.y) / auto_scroll_margin;
            self.scroll_by(-auto_scroll_speed * intensity);
        } else if mouse_pos.y > view_height - auto_scroll_margin {
            let intensity = (mouse_pos.y - (view_height - auto_scroll_margin)) / auto_scroll_margin;
            self.scroll_by(auto_scroll_speed * intensity);
        }

        let clamped_y = mouse_pos.y.clamp(2.0, view_height - 2.0);

        let hit = self
            .typesetter
            .as_ref()
            .and_then(|ts| ts.hit_test(mouse_pos.x, clamped_y));

        if let Some(hit) = hit
            && let Some(cursor) = &self.cursor
        {
            cursor.set_position(hit.position, MoveMode::KeepAnchor);
            self.update_cursor_display();
        }
    }

    fn move_cursor(&mut self, op: MoveOperation, mode: MoveMode) {
        if let Some(cursor) = &self.cursor {
            cursor.move_position(op, mode, 1);
            self.update_cursor_display();
            if mode == MoveMode::KeepAnchor {
                self.base_mut().emit_signal("selection_changed", &[]);
            }
        }
    }

    fn move_cursor_vertical(&mut self, direction: i32, mode: MoveMode) {
        let Some(cursor) = &self.cursor else { return };
        let Some(ts) = &self.typesetter else { return };

        let pos = cursor.position();
        let caret = ts.caret_rect(pos);
        let line_height = caret[3].max(16.0);
        let center_y = caret[1] + caret[3] / 2.0;

        let x = self.preferred_x.unwrap_or(caret[0]);
        if self.preferred_x.is_none() {
            self.preferred_x = Some(caret[0]);
        }

        let target_y = center_y + (direction as f32) * line_height;

        // target_y is in screen space; content_height is document space
        if target_y < 0.0 || target_y > ts.content_height() * ts.zoom() {
            return;
        }

        if let Some(hit) = ts.hit_test(x, target_y)
            && hit.position != pos
        {
            cursor.set_position(hit.position, mode);
            self.update_cursor_display();
            if mode == MoveMode::KeepAnchor {
                self.base_mut().emit_signal("selection_changed", &[]);
            }
        }
    }

    fn update_cursor_display(&mut self) {
        if let (Some(cursor), Some(ts)) = (&self.cursor, &mut self.typesetter) {
            ts.set_cursor(&CursorDisplay {
                position: cursor.position(),
                anchor: cursor.anchor(),
                visible: false, // Never show the caret in view mode
                selected_cells: Vec::new(),
            });
            self.needs_redraw = true;
        }
    }

    fn clipboard_copy(&mut self) {
        let Some(cursor) = &self.cursor else { return };
        if !cursor.has_selection() {
            return;
        }
        let Ok(plain) = cursor.selected_text() else {
            return;
        };
        if plain.is_empty() {
            return;
        }
        DisplayServer::singleton().clipboard_set(&GString::from(plain.as_str()));
    }

    fn scroll_by(&mut self, delta: f32) {
        let view_height = self.base().get_size().y;
        if let Some(ts) = &mut self.typesetter {
            let zoom = ts.zoom();
            // delta is in screen pixels; scroll_offset is in document space
            self.scroll_offset = (self.scroll_offset + delta / zoom).max(0.0);
            let max_scroll = (ts.content_height() - view_height / zoom).max(0.0);
            self.scroll_offset = self.scroll_offset.min(max_scroll);
            ts.set_scroll_offset(self.scroll_offset);
            self.needs_redraw = true;
        }
        self.update_scrollbar();
    }

    fn update_scrollbar(&mut self) {
        let size = self.base().get_size();
        let zoom = self
            .typesetter
            .as_ref()
            .map(|ts| ts.zoom())
            .unwrap_or(1.0) as f64;
        let content_height = self
            .typesetter
            .as_ref()
            .map(|ts| ts.content_height() as f64)
            .unwrap_or(0.0);
        let page = size.y as f64 / zoom;
        if let Some(scrollbar) = &mut self.v_scrollbar {
            let sb_width = scrollbar.get_size().x.max(12.0);
            scrollbar.set_position(Vector2::new(size.x - sb_width, 0.0));
            scrollbar.set_size(Vector2::new(sb_width, size.y));
            scrollbar.set_max(content_height);
            scrollbar.set_page(page);
            scrollbar.set_value_no_signal(self.scroll_offset as f64);
            scrollbar.set_visible(content_height > page);
        }
    }
}
