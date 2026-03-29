use godot::builtin::{Color, GString, Vector2};
use godot::classes::canvas_item::TextureFilter;
use godot::classes::control::FocusMode;
use godot::classes::notify::ControlNotification;
use godot::classes::{
    Control, DisplayServer, FontFile, HScrollBar, IControl, ImageTexture, InputEvent, VScrollBar,
};
use godot::obj::{Base, Gd, WithBaseField};
use godot::prelude::*;

use text_document::{
    DocumentEvent, FlowElement, MoveMode, MoveOperation, SelectionKind, SelectionType, TextCursor,
    TextDocument,
};
use text_typeset::{CursorDisplay, HitRegion, Typesetter};

use crate::bridge::{self, ImageCache};
use crate::fonts::{self, FontIds, FontSlots};
use crate::input::{self, InputAction};

// ---------------------------------------------------------------------------
// WrapMode enum (shared with RichTextView)
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, GodotConvert, Var, Export)]
#[godot(via = i64)]
#[repr(i64)]
pub enum WrapMode {
    #[default]
    None = 0,
    Word = 1,
}

// ---------------------------------------------------------------------------
// RichTextEdit
// ---------------------------------------------------------------------------

#[derive(GodotClass)]
#[class(base = Control)]
pub struct RichTextEdit {
    base: Base<Control>,

    // --- Exported properties ---
    #[export]
    text: GString,
    #[export]
    html_text: GString,
    #[export]
    markdown_text: GString,

    #[export]
    editable: bool,

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
    selection_color: Color,
    #[export]
    caret_color: Color,
    #[export]
    text_color: Color,
    #[export]
    caret_blink: bool,
    #[export]
    caret_blink_interval: f64,

    #[export]
    scroll_active: bool,

    // --- Internal state ---
    document: Option<TextDocument>,
    typesetter: Option<Typesetter>,
    cursor: Option<TextCursor>,
    atlas_texture: Option<Gd<ImageTexture>>,
    image_cache: ImageCache,
    font_ids: FontIds,
    caret_visible: bool,
    caret_timer: f64,
    needs_redraw: bool,
    scroll_offset: f32,
    h_scroll_offset: f32,
    v_scrollbar: Option<Gd<VScrollBar>>,
    h_scrollbar: Option<Gd<HScrollBar>>,
    /// Remembered X position for vertical cursor movement (sticky column).
    preferred_x: Option<f32>,
    /// Internal rich clipboard (HTML) for preserving formatting within the app.
    rich_clipboard_html: Option<String>,
    /// Plain text corresponding to the rich clipboard, to detect external clipboard changes.
    rich_clipboard_plain: Option<String>,
    /// Click counter for triple-click detection.
    click_count: u32,
    /// Timestamp of last click (milliseconds from engine start).
    last_click_time: f64,
    /// Position of last click.
    last_click_pos: Vector2,
    /// Debounce timer for deferred work (scrollbar, signals). Reset on each edit.
    debounce_timer: f64,
    /// Scrollbar needs updating (deferred until typing pauses).
    scrollbar_dirty: bool,
    /// Content changed since last render (vs cursor-only blink).
    content_dirty: bool,
    /// Pending text_changed signal (batched).
    pending_text_changed: bool,
    /// Pending format_changed signal (batched).
    pending_format_changed: bool,
    /// Pending undo_redo_changed signal (batched).
    pending_undo_redo: Option<(bool, bool)>,
    /// Block ID from last incremental relayout (for incremental render).
    last_relayout_block_id: Option<usize>,
    /// Buffered character input, flushed as one insert at start of process().
    pending_chars: String,
    /// Ctrl+A escalation level: 0=none, 1=cell content, 2=cell, 3=table, then document.
    select_all_level: u8,
}

#[godot_api]
impl IControl for RichTextEdit {
    fn init(base: Base<Control>) -> Self {
        Self {
            base,
            text: GString::new(),
            html_text: GString::new(),
            markdown_text: GString::new(),
            editable: true,
            wrap_mode: WrapMode::None,
            default_font: None,
            bold_font: None,
            italic_font: None,
            bold_italic_font: None,
            monospace_font: None,
            default_font_size: 24,
            zoom: 1.0,
            selection_color: Color::from_rgba(0.26, 0.52, 0.96, 0.3),
            caret_color: Color::from_rgba(1.0, 1.0, 1.0, 1.0),
            text_color: Color::from_rgba(0.0, 0.0, 0.0, 1.0),
            caret_blink: true,
            caret_blink_interval: 0.65,
            scroll_active: true,
            document: None,
            typesetter: None,
            cursor: None,
            atlas_texture: None,
            image_cache: ImageCache::default(),
            font_ids: FontIds::default(),
            caret_visible: true,
            caret_timer: 0.0,
            needs_redraw: false,
            scroll_offset: 0.0,
            h_scroll_offset: 0.0,
            v_scrollbar: None,
            h_scrollbar: None,
            preferred_x: None,
            rich_clipboard_html: None,
            rich_clipboard_plain: None,
            click_count: 0,
            last_click_time: 0.0,
            last_click_pos: Vector2::ZERO,
            debounce_timer: 1.0, // start expired so initial scrollbar works
            scrollbar_dirty: false,
            content_dirty: true, // first draw must be a full render
            pending_text_changed: false,
            pending_format_changed: false,
            pending_undo_redo: None,
            last_relayout_block_id: None,
            pending_chars: String::new(),
            select_all_level: 0,
        }
    }

    fn ready(&mut self) {
        self.base_mut().set_focus_mode(FocusMode::ALL);
        self.base_mut().set_clip_contents(true);
        self.base_mut().set_texture_filter(TextureFilter::LINEAR);

        let doc = TextDocument::new();
        let mut ts = Typesetter::new();
        let cursor = doc.cursor();
        let atlas_tex = ImageTexture::new_gd();

        // Register fonts (falls back to embedded NotoSans if none set)
        let slots = FontSlots {
            default: &self.default_font,
            bold: &self.bold_font,
            italic: &self.italic_font,
            bold_italic: &self.bold_italic_font,
            monospace: &self.monospace_font,
        };
        self.font_ids = fonts::register_fonts(&mut ts, &slots, self.default_font_size as f32);

        // Set viewport size, zoom, and wrap mode
        let size = self.base().get_size();
        ts.set_viewport(size.x, size.y);
        ts.set_zoom(self.zoom);
        match self.wrap_mode {
            WrapMode::None => ts.set_content_width(f32::INFINITY),
            WrapMode::Word => ts.set_content_width_auto(),
        }

        // Set cursor/selection colors
        ts.set_selection_color([
            self.selection_color.r,
            self.selection_color.g,
            self.selection_color.b,
            self.selection_color.a,
        ]);
        ts.set_cursor_color([
            self.caret_color.r,
            self.caret_color.g,
            self.caret_color.b,
            self.caret_color.a,
        ]);
        ts.set_text_color([
            self.text_color.r,
            self.text_color.g,
            self.text_color.b,
            self.text_color.a,
        ]);

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

        // Initial layout
        let flow = doc.snapshot_flow();
        ts.layout_full(&flow);

        // Set initial cursor display
        ts.set_cursor(&CursorDisplay {
            position: cursor.position(),
            anchor: cursor.anchor(),
            visible: true,
            selected_cells: Vec::new(),
        });

        self.document = Some(doc);
        self.typesetter = Some(ts);
        self.cursor = Some(cursor);
        self.atlas_texture = Some(atlas_tex);

        // Create VScrollBar as a child node
        if self.scroll_active {
            let mut scrollbar = VScrollBar::new_alloc();
            scrollbar.set_name("_v_scrollbar");
            self.base_mut().add_child(&scrollbar);
            let callable = self.base().callable("on_v_scroll_changed");
            scrollbar.connect("value_changed", &callable);
            self.v_scrollbar = Some(scrollbar);
        }

        // Create HScrollBar (only used when wrap_mode == None)
        if self.wrap_mode == WrapMode::None {
            let mut scrollbar = HScrollBar::new_alloc();
            scrollbar.set_name("_h_scrollbar");
            self.base_mut().add_child(&scrollbar);
            let callable = self.base().callable("on_h_scroll_changed");
            scrollbar.connect("value_changed", &callable);
            self.h_scrollbar = Some(scrollbar);
        }

        self.update_scrollbar();
        self.base_mut().queue_redraw();
    }

    fn process(&mut self, delta: f64) {
        self.flush_pending_chars();

        let mut emit_document_loaded = false;
        let mut had_content_change = false;

        let widget_size = self.base().get_size();

        if let (Some(doc), Some(ts)) = (&self.document, &mut self.typesetter) {
            let events = doc.poll_events();
            let mut needs_full_layout = false;
            let mut incremental_positions: Vec<usize> = Vec::new();

            for event in &events {
                match event {
                    DocumentEvent::ContentsChanged {
                        position,
                        blocks_affected,
                        ..
                    } => {
                        if *blocks_affected <= 1 && !needs_full_layout {
                            incremental_positions.push(*position);
                        } else {
                            needs_full_layout = true;
                        }
                        self.needs_redraw = true;
                        had_content_change = true;
                        self.pending_text_changed = true;
                    }
                    DocumentEvent::FormatChanged { .. } => {
                        needs_full_layout = true;
                        self.needs_redraw = true;
                        had_content_change = true;
                        self.pending_format_changed = true;
                    }
                    DocumentEvent::DocumentReset
                    | DocumentEvent::FlowElementsInserted { .. }
                    | DocumentEvent::FlowElementsRemoved { .. }
                    | DocumentEvent::BlockCountChanged(_) => {
                        needs_full_layout = true;
                        self.needs_redraw = true;
                        had_content_change = true;
                    }
                    DocumentEvent::UndoRedoChanged {
                        can_undo, can_redo, ..
                    } => {
                        self.pending_undo_redo = Some((*can_undo, *can_redo));
                    }
                    DocumentEvent::LongOperationFinished { .. } => {
                        emit_document_loaded = true;
                    }
                    _ => {}
                }
            }

            // In word-wrap mode, pre-adjust viewport width for scrollbar
            if self.wrap_mode == WrapMode::Word && self.v_scrollbar.is_some() {
                let vsb_width = 12.0_f32;
                let zoom = ts.zoom();
                let v_visible = ts.content_height() > widget_size.y / zoom;
                let effective_width = if v_visible {
                    widget_size.x - vsb_width
                } else {
                    widget_size.x
                };
                // layout_width() returns document-space width (viewport / zoom)
                let current_width = ts.layout_width();
                let expected_width = effective_width / zoom;
                if (current_width - expected_width).abs() > 1.0 {
                    ts.set_viewport(effective_width, widget_size.y);
                    ts.set_content_width_auto();
                    needs_full_layout = true;
                }
            }

            if needs_full_layout {
                let flow = doc.snapshot_flow();
                ts.layout_full(&flow);
                self.last_relayout_block_id = None;
            } else if let Some(&last_pos) = incremental_positions.last() {
                // Multiple keystrokes in one frame all edit the same block.
                // The DB already has the final state, only relayout once.
                if let Some(snap) = doc.snapshot_block_at_position(last_pos) {
                    let block_id = snap.block_id;
                    let params = text_typeset::bridge::convert_block(&snap);
                    ts.relayout_block(&params);
                    self.last_relayout_block_id = Some(block_id);
                }
            }
        }

        // Mark content dirty for full render (vs cursor-only redraw)
        if had_content_change {
            self.content_dirty = true;
            self.scrollbar_dirty = true;
            self.debounce_timer = 0.0;
        }

        // Update cursor display after any layout changes (immediate, cheap)
        if self.needs_redraw {
            self.update_cursor_display();
            self.ensure_caret_h_visible();
        }

        // Debounced work: scrollbar + signals (fire after 150ms of no edits)
        self.debounce_timer += delta;
        if self.debounce_timer >= 0.15 {
            if self.scrollbar_dirty {
                self.scrollbar_dirty = false;
                self.update_scrollbar();
            }
            if self.pending_text_changed {
                self.pending_text_changed = false;
                self.base_mut().emit_signal("text_changed", &[]);
            }
            if self.pending_format_changed {
                self.pending_format_changed = false;
                self.base_mut().emit_signal("format_changed", &[]);
            }
            if let Some((can_undo, can_redo)) = self.pending_undo_redo.take() {
                self.base_mut().emit_signal(
                    "undo_redo_changed",
                    &[can_undo.to_variant(), can_redo.to_variant()],
                );
            }
        }

        // document_loaded is a one-shot event, emit immediately
        if emit_document_loaded {
            self.base_mut().emit_signal("document_loaded", &[]);
        }

        // Caret blink: only triggers cursor-only redraw, not full content render
        let has_focus = self.base().has_focus();
        if self.editable && self.caret_blink && has_focus {
            self.caret_timer += delta;
            if self.caret_timer >= self.caret_blink_interval {
                self.caret_timer = 0.0;
                self.caret_visible = !self.caret_visible;
                self.update_cursor_display();
                self.needs_redraw = true;
            }
        }

        if self.needs_redraw {
            self.needs_redraw = false;
            self.base_mut().queue_redraw();
        }
    }

    fn draw(&mut self) {
        let mut ts = self.typesetter.take();
        let mut atlas_tex = self.atlas_texture.take();
        let mut img_cache = std::mem::take(&mut self.image_cache);
        let doc = self.document.clone();
        let zoom = self.typesetter.as_ref().map(|ts| ts.zoom()).unwrap_or(1.0);
        let h_off = self.h_scroll_offset * zoom;
        let content_dirty = self.content_dirty;
        self.content_dirty = false;

        let incremental_block = self.last_relayout_block_id.take();

        if let (Some(ts), Some(atlas_tex)) = (&mut ts, &mut atlas_tex) {
            let frame = if content_dirty {
                if let Some(block_id) = incremental_block {
                    // Incremental render: only re-render the changed block
                    ts.render_block_only(block_id)
                } else {
                    // Full render: structural change, rebuild everything
                    ts.render()
                }
            } else {
                // Cursor-only: just update cursor/selection decorations
                ts.render_cursor_only()
            };
            if content_dirty {
                // Load any inline images not yet cached
                if let Some(doc) = &doc {
                    for img in &frame.images {
                        img_cache.get_or_load(&img.name, |name| doc.resource(name).ok().flatten());
                    }
                }
            }
            bridge::update_atlas(frame, atlas_tex);
            bridge::draw_frame(
                self.base_mut().upcast_mut::<Control>(),
                frame,
                atlas_tex,
                h_off,
                &img_cache,
            );
        }
        self.typesetter = ts;
        self.atlas_texture = atlas_tex;
        self.image_cache = img_cache;
    }

    fn gui_input(&mut self, event: Gd<InputEvent>) {
        if !self.editable {
            // Even in non-editable mode, handle link/image clicks
            self.handle_readonly_input(&event);
            return;
        }

        let action = input::translate_input(&event);

        // Clear sticky X for any non-vertical action (but not for None/scroll)
        if !matches!(
            action,
            InputAction::None
                | InputAction::MoveUp
                | InputAction::MoveDown
                | InputAction::SelectUp
                | InputAction::SelectDown
                | InputAction::PageUp
                | InputAction::PageDown
                | InputAction::ScrollUp
                | InputAction::ScrollDown
                | InputAction::ScrollLeft
                | InputAction::ScrollRight
        ) {
            self.preferred_x = None;
        }

        // Reset Ctrl+A escalation on any non-SelectAll action
        if !matches!(action, InputAction::SelectAll) {
            self.select_all_level = 0;
        }

        // Flush any buffered characters before processing a non-char action
        self.flush_pending_chars();

        match action {
            InputAction::None => return,
            InputAction::InsertChar(ch) => {
                self.pending_chars.push(ch);
                self.caret_visible = true;
                self.caret_timer = 0.0;
                self.base_mut().accept_event();
                return;
            }
            InputAction::Enter => {
                if let Some(cursor) = &self.cursor {
                    if let Some(cell_ref) = cursor.current_table_cell() {
                        self.navigate_table_cell_down(&cell_ref);
                    } else {
                        let _ = cursor.insert_block();
                    }
                }
            }
            InputAction::CtrlEnter => {
                if let Some(cursor) = &self.cursor {
                    let _ = cursor.insert_block();
                }
            }
            InputAction::Backspace => {
                if let Some(cursor) = &self.cursor {
                    if cursor.at_block_start() && self.is_cursor_in_list() {
                        // Backspace at start of list item:
                        // - If indented: decrease indent (Qt: QTextBlockFormat::setIndent)
                        // - If at indent 0: remove from list (Qt: QTextList::remove(QTextBlock))
                        if let Ok(fmt) = cursor.block_format() {
                            let level = fmt.indent.unwrap_or(0);
                            if level > 0 {
                                let new_fmt = text_document::BlockFormat {
                                    indent: Some(level - 1),
                                    ..Default::default()
                                };
                                let _ = cursor.set_block_format(&new_fmt);
                            } else {
                                let _ = cursor.remove_current_block_from_list();
                            }
                        }
                    } else {
                        let _ = cursor.delete_previous_char();
                    }
                }
            }
            InputAction::Delete => {
                if let Some(cursor) = &self.cursor {
                    let _ = cursor.delete_char();
                }
            }
            InputAction::DeleteWordLeft => {
                if let Some(cursor) = &self.cursor {
                    cursor.move_position(MoveOperation::WordLeft, MoveMode::KeepAnchor, 1);
                    let _ = cursor.remove_selected_text();
                }
            }
            InputAction::DeleteWordRight => {
                if let Some(cursor) = &self.cursor {
                    cursor.move_position(MoveOperation::WordRight, MoveMode::KeepAnchor, 1);
                    let _ = cursor.remove_selected_text();
                }
            }
            InputAction::Tab => {
                if let Some(cursor) = &self.cursor {
                    if let Some(cell_ref) = cursor.current_table_cell() {
                        // Tab in table: move to next cell
                        self.navigate_table_cell(&cell_ref, 1);
                    } else if cursor.at_block_start() && self.is_cursor_in_list() {
                        if let Ok(fmt) = cursor.block_format() {
                            let level = fmt.indent.unwrap_or(0);
                            let new_fmt = text_document::BlockFormat {
                                indent: Some(level + 1),
                                ..Default::default()
                            };
                            let _ = cursor.set_block_format(&new_fmt);
                        }
                    } else {
                        let _ = cursor.insert_text("\t");
                    }
                }
            }
            InputAction::ShiftTab => {
                if let Some(cursor) = &self.cursor {
                    if let Some(cell_ref) = cursor.current_table_cell() {
                        // Shift+Tab in table: move to previous cell
                        self.navigate_table_cell(&cell_ref, -1);
                    } else if self.is_cursor_in_list()
                        && let Ok(fmt) = cursor.block_format()
                    {
                        let level = fmt.indent.unwrap_or(0);
                        if level > 0 {
                            let new_fmt = text_document::BlockFormat {
                                indent: Some(level - 1),
                                ..Default::default()
                            };
                            let _ = cursor.set_block_format(&new_fmt);
                        }
                    }
                }
            }

            // Navigation
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
            InputAction::PageUp => self.move_cursor_page(-1),
            InputAction::PageDown => self.move_cursor_page(1),

            // Selection (with cell selection support for tables)
            InputAction::SelectLeft => {
                if !self.try_extend_cell_selection(-1, 0) {
                    self.move_cursor(MoveOperation::Left, MoveMode::KeepAnchor);
                }
            }
            InputAction::SelectRight => {
                if !self.try_extend_cell_selection(1, 0) {
                    self.move_cursor(MoveOperation::Right, MoveMode::KeepAnchor);
                }
            }
            InputAction::SelectUp => {
                if !self.try_extend_cell_selection(0, -1) {
                    self.move_cursor_vertical(-1, MoveMode::KeepAnchor);
                }
            }
            InputAction::SelectDown => {
                if !self.try_extend_cell_selection(0, 1) {
                    self.move_cursor_vertical(1, MoveMode::KeepAnchor);
                }
            }
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
            InputAction::SelectDocEnd => self.move_cursor(MoveOperation::End, MoveMode::KeepAnchor),
            InputAction::SelectAll => {
                if let Some(cursor) = &self.cursor {
                    let level = self.select_all_level + 1;
                    let cell_ref = cursor.current_table_cell();

                    if let Some(cell_ref) = cell_ref {
                        match level {
                            1 => cursor.select(SelectionType::BlockUnderCursor),
                            2 => cursor.select_table_cell(
                                cell_ref.table.id(),
                                cell_ref.row,
                                cell_ref.column,
                            ),
                            3 => {
                                let rows = cell_ref.table.rows();
                                let cols = cell_ref.table.columns();
                                cursor.select_cell_range(
                                    cell_ref.table.id(),
                                    0,
                                    0,
                                    if rows > 0 { rows - 1 } else { 0 },
                                    if cols > 0 { cols - 1 } else { 0 },
                                );
                            }
                            _ => {
                                cursor.select(SelectionType::Document);
                            }
                        }
                        self.select_all_level = if level >= 4 { 0 } else { level };
                    } else {
                        cursor.select(SelectionType::Document);
                        self.select_all_level = 0;
                    }

                    self.update_cursor_display();
                    self.base_mut().emit_signal("selection_changed", &[]);
                }
            }

            // Clipboard
            InputAction::Copy => self.clipboard_copy(),
            InputAction::Cut => {
                self.clipboard_copy();
                if let Some(cursor) = &self.cursor {
                    let _ = cursor.remove_selected_text();
                }
            }
            InputAction::Paste => self.clipboard_paste(),

            // Undo/redo
            InputAction::Undo => {
                if let Some(doc) = &self.document {
                    let _ = doc.undo();
                }
            }
            InputAction::Redo => {
                if let Some(doc) = &self.document {
                    let _ = doc.redo();
                }
            }

            // Formatting
            InputAction::ToggleBold => self.toggle_bold(),
            InputAction::ToggleItalic => self.toggle_italic(),
            InputAction::ToggleUnderline => self.toggle_underline(),

            // Mouse: click counting for single/double/triple click
            InputAction::Click { position } => {
                let now = godot::classes::Time::singleton().get_ticks_msec() as f64 / 1000.0;
                let dist = (position - self.last_click_pos).length();
                if now - self.last_click_time < 0.4 && dist < 5.0 {
                    self.click_count += 1;
                } else {
                    self.click_count = 1;
                }
                self.last_click_time = now;
                self.last_click_pos = position;

                match self.click_count {
                    1 => self.handle_click(position, false),
                    2 => {
                        self.handle_click(position, false);
                        if let Some(cursor) = &self.cursor {
                            cursor.select(SelectionType::WordUnderCursor);
                            self.update_cursor_display();
                        }
                        self.base_mut().emit_signal("selection_changed", &[]);
                    }
                    _ => {
                        // Triple (or more) click: select paragraph
                        self.handle_click(position, false);
                        if let Some(cursor) = &self.cursor {
                            cursor.select(SelectionType::BlockUnderCursor);
                            self.update_cursor_display();
                        }
                        self.click_count = 3;
                        self.base_mut().emit_signal("selection_changed", &[]);
                    }
                }
                self.base_mut().emit_signal("caret_changed", &[]);
            }
            InputAction::DoubleClick { position } => {
                // Godot fires DoubleClick; fold into our click counter
                self.click_count = 2;
                self.last_click_time =
                    godot::classes::Time::singleton().get_ticks_msec() as f64 / 1000.0;
                self.last_click_pos = position;
                self.handle_click(position, false);
                if let Some(cursor) = &self.cursor {
                    cursor.select(SelectionType::WordUnderCursor);
                    self.update_cursor_display();
                }
                self.base_mut().emit_signal("caret_changed", &[]);
                self.base_mut().emit_signal("selection_changed", &[]);
            }
            InputAction::ShiftClick { position } => {
                self.handle_click(position, true);
                self.base_mut().emit_signal("caret_changed", &[]);
                self.base_mut().emit_signal("selection_changed", &[]);
            }
            InputAction::DragSelect { position } => {
                self.handle_drag_select(position);
                self.base_mut().accept_event();
                return;
            }

            // Scroll: handle and return early (don't snap viewport back to caret)
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
            InputAction::ScrollLeft => {
                self.scroll_h_by(-40.0);
                self.base_mut().accept_event();
                return;
            }
            InputAction::ScrollRight => {
                self.scroll_h_by(40.0);
                self.base_mut().accept_event();
                return;
            }
        }

        // Ensure caret is visible after editing/navigation actions
        if let Some(ts) = &mut self.typesetter
            && let Some(new_offset) = ts.ensure_caret_visible()
        {
            self.scroll_offset = new_offset;
        }
        self.ensure_caret_h_visible();
        self.update_scrollbar();

        // Reset caret blink on input
        self.caret_visible = true;
        self.caret_timer = 0.0;

        self.base_mut().accept_event();
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
            self.content_dirty = true;
            self.base_mut().queue_redraw();
        } else if what == ControlNotification::FOCUS_ENTER {
            self.caret_visible = true;
            self.caret_timer = 0.0;
            self.update_cursor_display();
            self.update_ime_position();
            DisplayServer::singleton().window_set_ime_active(true);
            self.base_mut().queue_redraw();
        } else if what == ControlNotification::FOCUS_EXIT {
            DisplayServer::singleton().window_set_ime_active(false);
            self.update_cursor_display();
            self.base_mut().queue_redraw();
        }
    }
}

// ---------------------------------------------------------------------------
// Signals and GDScript-callable methods
// ---------------------------------------------------------------------------

#[godot_api]
impl RichTextEdit {
    // --- Signals ---
    #[signal]
    fn text_changed();
    #[signal]
    fn format_changed();
    #[signal]
    fn caret_changed();
    #[signal]
    fn selection_changed();
    #[signal]
    fn link_clicked(url: GString);
    #[signal]
    fn image_clicked(name: GString);
    #[signal]
    fn undo_redo_changed(can_undo: bool, can_redo: bool);
    #[signal]
    fn document_loaded();

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

    // --- Cursor / selection methods ---

    #[func]
    fn select_all(&mut self) {
        if let Some(cursor) = &self.cursor {
            cursor.select(SelectionType::Document);
            self.update_cursor_display();
        }
    }

    #[func]
    fn select_word(&mut self) {
        if let Some(cursor) = &self.cursor {
            cursor.select(SelectionType::WordUnderCursor);
            self.update_cursor_display();
        }
    }

    #[func]
    fn select_line(&mut self) {
        if let Some(cursor) = &self.cursor {
            cursor.select(SelectionType::LineUnderCursor);
            self.update_cursor_display();
        }
    }

    #[func]
    fn deselect(&mut self) {
        if let Some(cursor) = &self.cursor {
            cursor.clear_selection();
            self.update_cursor_display();
        }
    }

    #[func]
    fn get_selected_text(&self) -> GString {
        self.cursor
            .as_ref()
            .and_then(|c| c.selected_text().ok())
            .map(|s| GString::from(s.as_str()))
            .unwrap_or_default()
    }

    #[func]
    fn get_caret_position(&self) -> i32 {
        self.cursor
            .as_ref()
            .map(|c| c.position() as i32)
            .unwrap_or(0)
    }

    #[func]
    fn set_caret_position(&mut self, pos: i32) {
        if let Some(cursor) = &self.cursor {
            cursor.set_position(pos as usize, MoveMode::MoveAnchor);
            self.update_cursor_display();
        }
    }

    // --- Editing methods ---

    #[func]
    fn insert_text(&mut self, text: GString) {
        if let Some(cursor) = &self.cursor {
            let _ = cursor.insert_text(&text.to_string());
        }
    }

    #[func]
    fn insert_html(&mut self, html: GString) {
        if let Some(cursor) = &self.cursor {
            let _ = cursor.insert_html(&html.to_string());
        }
    }

    #[func]
    fn insert_image(&mut self, name: GString, width: i32, height: i32) {
        if let Some(cursor) = &self.cursor {
            let _ = cursor.insert_image(&name.to_string(), width as u32, height as u32);
        }
    }

    #[func]
    fn delete_selection(&mut self) {
        if let Some(cursor) = &self.cursor {
            let _ = cursor.remove_selected_text();
        }
    }

    /// Copy selection to the system clipboard, preserving rich HTML internally.
    #[func]
    fn copy_rich(&mut self) {
        self.clipboard_copy();
    }

    /// Paste from the clipboard, restoring rich HTML if the content matches
    /// what was previously copied with [method copy_rich].
    #[func]
    fn paste_rich(&mut self) {
        self.clipboard_paste();
    }

    /// Cut selection to the system clipboard, preserving rich HTML internally.
    #[func]
    fn cut_rich(&mut self) {
        self.clipboard_copy();
        if let Some(cursor) = &self.cursor {
            let _ = cursor.remove_selected_text();
        }
    }

    // --- Formatting methods ---

    #[func]
    fn set_bold(&mut self, enabled: bool) {
        if let Some(cursor) = &self.cursor {
            let fmt = text_document::TextFormat {
                font_bold: Some(enabled),
                ..Default::default()
            };
            let _ = cursor.merge_char_format(&fmt);
        }
    }

    #[func]
    fn set_italic(&mut self, enabled: bool) {
        if let Some(cursor) = &self.cursor {
            let fmt = text_document::TextFormat {
                font_italic: Some(enabled),
                ..Default::default()
            };
            let _ = cursor.merge_char_format(&fmt);
        }
    }

    #[func]
    fn set_underline(&mut self, enabled: bool) {
        if let Some(cursor) = &self.cursor {
            let fmt = text_document::TextFormat {
                font_underline: Some(enabled),
                ..Default::default()
            };
            let _ = cursor.merge_char_format(&fmt);
        }
    }

    #[func]
    fn set_strikethrough(&mut self, enabled: bool) {
        if let Some(cursor) = &self.cursor {
            let fmt = text_document::TextFormat {
                font_strikeout: Some(enabled),
                ..Default::default()
            };
            let _ = cursor.merge_char_format(&fmt);
        }
    }

    #[func]
    fn set_font_size(&mut self, size: i32) {
        if let Some(cursor) = &self.cursor {
            let fmt = text_document::TextFormat {
                font_point_size: Some(size as u32),
                ..Default::default()
            };
            let _ = cursor.merge_char_format(&fmt);
        }
    }

    #[func]
    fn set_font_family(&mut self, family: GString) {
        if let Some(cursor) = &self.cursor {
            let fmt = text_document::TextFormat {
                font_family: Some(family.to_string()),
                ..Default::default()
            };
            let _ = cursor.merge_char_format(&fmt);
        }
    }

    #[func]
    fn set_alignment(&mut self, align: i32) {
        if let Some(cursor) = &self.cursor {
            let alignment = match align {
                0 => text_document::Alignment::Left,
                1 => text_document::Alignment::Center,
                2 => text_document::Alignment::Right,
                3 => text_document::Alignment::Justify,
                _ => text_document::Alignment::Left,
            };
            let fmt = text_document::BlockFormat {
                alignment: Some(alignment),
                ..Default::default()
            };
            let _ = cursor.set_block_format(&fmt);
        }
    }

    #[func]
    fn set_heading_level(&mut self, level: i32) {
        if let Some(cursor) = &self.cursor {
            let fmt = text_document::BlockFormat {
                heading_level: Some(level as u8),
                ..Default::default()
            };
            let _ = cursor.set_block_format(&fmt);
        }
    }

    // --- Lists ---

    #[func]
    fn insert_list(&mut self, ordered: bool) {
        if let Some(cursor) = &self.cursor {
            let style = if ordered {
                text_document::ListStyle::Decimal
            } else {
                text_document::ListStyle::Disc
            };
            let _ = cursor.create_list(style);
        }
    }

    // --- Tables ---

    #[func]
    fn insert_table(&mut self, rows: i32, cols: i32) {
        if let Some(cursor) = &self.cursor {
            let _ = cursor.insert_table(rows as usize, cols as usize);
        }
    }

    #[func]
    fn remove_current_table(&mut self) {
        if let Some(cursor) = &self.cursor {
            let _ = cursor.remove_current_table();
        }
    }

    #[func]
    fn insert_row_above(&mut self) {
        if let Some(cursor) = &self.cursor {
            let _ = cursor.insert_row_above();
        }
    }

    #[func]
    fn insert_row_below(&mut self) {
        if let Some(cursor) = &self.cursor {
            let _ = cursor.insert_row_below();
        }
    }

    #[func]
    fn insert_column_before(&mut self) {
        if let Some(cursor) = &self.cursor {
            let _ = cursor.insert_column_before();
        }
    }

    #[func]
    fn insert_column_after(&mut self) {
        if let Some(cursor) = &self.cursor {
            let _ = cursor.insert_column_after();
        }
    }

    #[func]
    fn remove_current_row(&mut self) {
        if let Some(cursor) = &self.cursor {
            let _ = cursor.remove_current_row();
        }
    }

    #[func]
    fn remove_current_column(&mut self) {
        if let Some(cursor) = &self.cursor {
            let _ = cursor.remove_current_column();
        }
    }

    #[func]
    fn is_in_table(&self) -> bool {
        self.cursor
            .as_ref()
            .and_then(|c| c.current_table())
            .is_some()
    }

    // --- Undo / Redo ---

    #[func]
    fn undo(&mut self) {
        if let Some(doc) = &self.document {
            let _ = doc.undo();
        }
    }

    #[func]
    fn redo(&mut self) {
        if let Some(doc) = &self.document {
            let _ = doc.redo();
        }
    }

    #[func]
    fn can_undo(&self) -> bool {
        self.document
            .as_ref()
            .map(|d| d.can_undo())
            .unwrap_or(false)
    }

    #[func]
    fn can_redo(&self) -> bool {
        self.document
            .as_ref()
            .map(|d| d.can_redo())
            .unwrap_or(false)
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

    /// Get the character format at the effective query position.
    /// When there's a selection, queries at the start of the selection
    /// (not cursor position, which is at the end and may be outside the selection).
    fn query_char_format(&self) -> Option<text_document::TextFormat> {
        let cursor = self.cursor.as_ref()?;
        let doc = self.document.as_ref()?;
        let pos = if cursor.has_selection() {
            cursor.selection_start()
        } else {
            cursor.position()
        };
        let block = doc.block_at_position(pos)?;
        let offset = pos.saturating_sub(block.position());
        block.char_format_at(offset)
    }

    /// Query current character format at cursor/selection (for toolbar state).
    #[func]
    fn is_bold(&self) -> bool {
        self.query_char_format()
            .and_then(|f| f.font_bold)
            .unwrap_or(false)
    }

    #[func]
    fn is_italic(&self) -> bool {
        self.query_char_format()
            .and_then(|f| f.font_italic)
            .unwrap_or(false)
    }

    #[func]
    fn is_underline(&self) -> bool {
        self.query_char_format()
            .and_then(|f| f.font_underline)
            .unwrap_or(false)
    }

    #[func]
    fn is_strikethrough(&self) -> bool {
        self.query_char_format()
            .and_then(|f| f.font_strikeout)
            .unwrap_or(false)
    }

    #[func]
    fn get_heading_level(&self) -> i32 {
        self.cursor
            .as_ref()
            .and_then(|c| c.block_format().ok())
            .and_then(|f| f.heading_level)
            .unwrap_or(0) as i32
    }

    #[func]
    fn get_alignment(&self) -> i32 {
        self.cursor
            .as_ref()
            .and_then(|c| c.block_format().ok())
            .and_then(|f| f.alignment)
            .map(|a| match a {
                text_document::Alignment::Left => 0,
                text_document::Alignment::Center => 1,
                text_document::Alignment::Right => 2,
                text_document::Alignment::Justify => 3,
            })
            .unwrap_or(0)
    }

    #[func]
    fn has_selection(&self) -> bool {
        self.cursor
            .as_ref()
            .map(|c| c.has_selection())
            .unwrap_or(false)
    }

    /// Called by the VScrollBar's value_changed signal.
    #[func]
    fn on_v_scroll_changed(&mut self, value: f64) {
        self.scroll_offset = value as f32;
        if let Some(ts) = &mut self.typesetter {
            ts.set_scroll_offset(self.scroll_offset);
        }
        self.content_dirty = true;
        self.needs_redraw = true;
        self.base_mut().queue_redraw();
    }

    /// Called by the HScrollBar's value_changed signal.
    #[func]
    fn on_h_scroll_changed(&mut self, value: f64) {
        self.h_scroll_offset = value as f32;
        self.content_dirty = true;
        self.needs_redraw = true;
        self.base_mut().queue_redraw();
    }

    // --- Zoom methods ---

    #[func]
    fn set_zoom_level(&mut self, zoom: f32) {
        self.zoom = zoom.clamp(0.1, 10.0);
        if let (Some(ts), Some(doc)) = (&mut self.typesetter, &self.document) {
            ts.set_zoom(self.zoom);
            // Relayout needed: layout_width changes with zoom in Auto mode
            let flow = doc.snapshot_flow();
            ts.layout_full(&flow);
            self.content_dirty = true;
            self.needs_redraw = true;
        }
        self.scrollbar_dirty = true;
    }

    #[func]
    fn get_zoom_level(&self) -> f32 {
        self.zoom
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

impl RichTextEdit {
    /// Flush buffered character input as a single insert_text call.
    fn flush_pending_chars(&mut self) {
        if self.pending_chars.is_empty() {
            return;
        }
        if let Some(cursor) = &self.cursor {
            let _ = cursor.insert_text(&self.pending_chars);
        }
        self.pending_chars.clear();
    }

    /// Check if the cursor is currently inside a list item.
    /// Navigate to the next (direction=1) or previous (direction=-1) table cell.
    /// At the last cell with direction=1, inserts a new row.
    fn navigate_table_cell(&mut self, cell_ref: &text_document::TableCellRef, direction: i32) {
        let table = &cell_ref.table;
        let rows = table.rows();
        let cols = table.columns();
        let row = cell_ref.row;
        let col = cell_ref.column;

        let (target_row, target_col) = if direction > 0 {
            // Next cell
            if col + 1 < cols {
                (row, col + 1)
            } else if row + 1 < rows {
                (row + 1, 0)
            } else {
                // Last cell: insert a new row
                if let Some(cursor) = &self.cursor {
                    let _ = cursor.insert_row_below();
                }
                (row + 1, 0)
            }
        } else {
            // Previous cell
            if col > 0 {
                (row, col - 1)
            } else if row > 0 {
                (row - 1, cols.saturating_sub(1))
            } else {
                return; // Already at first cell
            }
        };

        // Move cursor to the target cell's first block
        if let Some(cell) = table.cell(target_row, target_col) {
            let blocks = cell.blocks();
            if let Some(block) = blocks.first()
                && let Some(cursor) = &self.cursor
            {
                cursor.set_position(block.position(), MoveMode::MoveAnchor);
                self.update_cursor_display();
            }
        }
    }

    /// Navigate to the cell below (same column) or the block after the table.
    fn navigate_table_cell_down(&mut self, cell_ref: &text_document::TableCellRef) {
        let table = &cell_ref.table;
        let row = cell_ref.row;
        let col = cell_ref.column;

        if row + 1 < table.rows() {
            // Move to same column in the next row
            if let Some(target_cell) = table.cell(row + 1, col) {
                let blocks = target_cell.blocks();
                if let Some(block) = blocks.first()
                    && let Some(cursor) = &self.cursor
                {
                    cursor.set_position(block.position(), MoveMode::MoveAnchor);
                    self.update_cursor_display();
                }
            }
        } else {
            // Last row: move cursor to the block after the table
            self.move_cursor_after_table(table);
        }
    }

    /// Move the cursor to the first flow element after the given table.
    fn move_cursor_after_table(&mut self, table: &text_document::TextTable) {
        if let (Some(doc), Some(cursor)) = (&self.document, &self.cursor) {
            let flow = doc.flow();
            let table_id = table.id();
            let mut found = false;
            for element in &flow {
                if found {
                    match element {
                        FlowElement::Block(block) => {
                            cursor.set_position(block.position(), MoveMode::MoveAnchor);
                            self.update_cursor_display();
                            return;
                        }
                        FlowElement::Table(t) => {
                            if let Some(cell) = t.cell(0, 0)
                                && let Some(block) = cell.blocks().first()
                            {
                                cursor.set_position(block.position(), MoveMode::MoveAnchor);
                                self.update_cursor_display();
                                return;
                            }
                        }
                        _ => {}
                    }
                }
                if let FlowElement::Table(t) = element
                    && t.id() == table_id
                {
                    found = true;
                }
            }
        }
    }

    /// Update IME composition window position to match the caret.
    fn update_ime_position(&self) {
        if let (Some(cursor), Some(ts)) = (&self.cursor, &self.typesetter) {
            let caret = ts.caret_rect(cursor.position());
            // caret is [x, y, w, h] in screen-space (scroll-adjusted)
            // Convert to global screen position for IME
            let local_pos = Vector2::new(
                caret[0] - self.h_scroll_offset * ts.zoom(),
                caret[1] + caret[3],
            );
            let global_pos = self.base().get_global_transform() * local_pos;
            DisplayServer::singleton().window_set_ime_position(godot::builtin::Vector2i::new(
                global_pos.x as i32,
                global_pos.y as i32,
            ));
        }
    }

    fn is_cursor_in_list(&self) -> bool {
        if let (Some(cursor), Some(doc)) = (&self.cursor, &self.document) {
            let pos = cursor.position();
            if let Some(block) = doc.block_at_position(pos) {
                return block.list().is_some();
            }
        }
        false
    }

    fn move_cursor(&mut self, op: MoveOperation, mode: MoveMode) {
        if let Some(cursor) = &self.cursor {
            cursor.move_position(op, mode, 1);
            self.update_cursor_display();
            self.base_mut().emit_signal("caret_changed", &[]);
            if mode == MoveMode::KeepAnchor {
                self.base_mut().emit_signal("selection_changed", &[]);
            }
        }
    }

    /// Move the cursor up or down by one visual line using the typesetter's layout.
    /// Uses a "sticky X" so the cursor remembers its column across short/empty lines.
    fn move_cursor_vertical(&mut self, direction: i32, mode: MoveMode) {
        let Some(cursor) = &self.cursor else { return };
        let Some(ts) = &mut self.typesetter else {
            return;
        };

        let pos = cursor.position();
        let caret = ts.caret_rect(pos);
        let line_height = caret[3].max(16.0);
        let center_y = caret[1] + caret[3] / 2.0;

        // Use sticky X if available, otherwise remember current X
        let x = self.preferred_x.unwrap_or(caret[0]);
        if self.preferred_x.is_none() {
            self.preferred_x = Some(caret[0]);
        }

        // Probe one line height above/below the caret center
        let mut target_y = center_y + (direction as f32) * line_height;

        // If target is above viewport, scroll up first to reveal content
        if target_y < 0.0 {
            if self.scroll_offset <= 0.0 {
                return; // Already at top of document
            }
            // scroll_delta is in screen pixels; convert to document space
            let scroll_delta = (-target_y + line_height) / ts.zoom();
            self.scroll_offset = (self.scroll_offset - scroll_delta).max(0.0);
            ts.set_scroll_offset(self.scroll_offset);
            // Re-probe after scroll adjustment
            let new_caret = ts.caret_rect(pos);
            let new_center_y = new_caret[1] + new_caret[3] / 2.0;
            target_y = new_center_y + (direction as f32) * line_height;
        }

        // If target is below content, don't move
        // target_y is in screen space; content_height is document space
        if target_y > ts.content_height() * ts.zoom() {
            return;
        }

        if let Some(hit) = ts.hit_test(x, target_y)
            && hit.position != pos
        {
            cursor.set_position(hit.position, mode);
            self.update_cursor_display();
            self.base_mut().emit_signal("caret_changed", &[]);
            if mode == MoveMode::KeepAnchor {
                self.base_mut().emit_signal("selection_changed", &[]);
            }
        }
    }

    /// Move the cursor up or down by roughly one viewport page.
    fn move_cursor_page(&mut self, direction: i32) {
        let view_height = self.base().get_size().y;
        let Some(cursor) = &self.cursor else { return };
        let Some(ts) = &mut self.typesetter else {
            return;
        };

        let pos = cursor.position();
        let caret = ts.caret_rect(pos);
        let line_height = caret[3].max(16.0);
        let center_y = caret[1] + caret[3] / 2.0;

        let x = self.preferred_x.unwrap_or(caret[0]);
        if self.preferred_x.is_none() {
            self.preferred_x = Some(caret[0]);
        }

        // Move by one viewport minus one line (so context is preserved)
        let page_step = (view_height - line_height).max(line_height);

        // Scroll the viewport (inline to avoid borrow conflict with ts)
        // page_step is in screen pixels; scroll_offset is in document space
        let zoom = ts.zoom();
        let old_scroll = self.scroll_offset;
        let delta = (direction as f32) * page_step / zoom;
        self.scroll_offset = (self.scroll_offset + delta).max(0.0);
        let max_scroll = (ts.content_height() - view_height / zoom).max(0.0);
        self.scroll_offset = self.scroll_offset.min(max_scroll);
        ts.set_scroll_offset(self.scroll_offset);
        self.content_dirty = true;
        self.scrollbar_dirty = true;

        // hit_test converts screen to doc coords internally.
        // Compensate for the scroll already applied so the cursor moves by exactly
        // page_step in screen space (handles clamping at top/bottom correctly).
        let actual_scroll_delta = (self.scroll_offset - old_scroll) * zoom;
        let target_y = center_y + (direction as f32) * page_step - actual_scroll_delta;
        if let Some(hit) = ts.hit_test(x, target_y)
            && hit.position != pos
        {
            cursor.set_position(hit.position, MoveMode::MoveAnchor);
            self.update_cursor_display();
            self.base_mut().emit_signal("caret_changed", &[]);
        }
    }

    fn update_cursor_display(&mut self) {
        let has_focus = self.base().has_focus();
        if let (Some(cursor), Some(ts)) = (&self.cursor, &mut self.typesetter) {
            let visible = self.caret_visible && has_focus;
            let selected_cells = Self::build_selected_cells(cursor);
            ts.set_cursor(&CursorDisplay {
                position: cursor.position(),
                anchor: cursor.anchor(),
                visible,
                selected_cells,
            });
            self.needs_redraw = true;
        }
        if has_focus {
            self.update_ime_position();
        }
    }

    /// Build the `(table_id, row, col)` vec for the typesetter from cursor state.
    fn build_selected_cells(cursor: &TextCursor) -> Vec<(usize, usize, usize)> {
        match cursor.selection_kind() {
            SelectionKind::Cells(range)
            | SelectionKind::Mixed {
                cell_range: range, ..
            } => {
                let mut cells = Vec::new();
                for row in range.start_row..=range.end_row {
                    for col in range.start_col..=range.end_col {
                        cells.push((range.table_id, row, col));
                    }
                }
                cells
            }
            _ => Vec::new(),
        }
    }

    /// Try to start or extend a rectangular cell selection.
    ///
    /// `dcol` and `drow` are -1/0/+1 indicating the direction.
    /// Returns `true` if the event was consumed (cell selection handled),
    /// `false` if the caller should fall back to normal selection.
    fn try_extend_cell_selection(&mut self, dcol: i32, drow: i32) -> bool {
        let cursor = match &self.cursor {
            Some(c) => c,
            None => return false,
        };

        // If already in cell selection mode, extend the range
        if let Some(range) = cursor.selected_cell_range() {
            // Get table dimensions from the first selected cell
            let cells = cursor.selected_cells();
            let (rows, cols) = cells
                .first()
                .map(|c| (c.table.rows(), c.table.columns()))
                .unwrap_or((0, 0));
            if rows == 0 || cols == 0 {
                return false;
            }

            let new_end_row = (range.end_row as i32 + drow).clamp(0, rows as i32 - 1) as usize;
            let new_end_col = (range.end_col as i32 + dcol).clamp(0, cols as i32 - 1) as usize;

            cursor.select_cell_range(
                range.table_id,
                range.start_row,
                range.start_col,
                new_end_row,
                new_end_col,
            );
            self.update_cursor_display();
            self.base_mut().emit_signal("selection_changed", &[]);
            return true;
        }

        // Not yet in cell selection: check if cursor is at cell boundary
        let cell_ref = match cursor.current_table_cell() {
            Some(c) => c,
            None => return false,
        };

        let at_start = cursor.at_block_start();
        let at_end = cursor.at_block_end();

        // Only activate cell selection when moving out of the cell boundary
        let should_activate = match (dcol, drow) {
            (-1, 0) => at_start && cell_ref.column > 0, // left at start of cell
            (1, 0) => at_end && cell_ref.column + 1 < cell_ref.table.columns(), // right at end
            (0, -1) => at_start && cell_ref.row > 0,    // up at start of cell
            (0, 1) => at_end && cell_ref.row + 1 < cell_ref.table.rows(), // down at end
            _ => false,
        };

        if !should_activate {
            return false;
        }

        let table_id = cell_ref.table.id();
        let target_row = (cell_ref.row as i32 + drow).max(0) as usize;
        let target_col = (cell_ref.column as i32 + dcol).max(0) as usize;

        cursor.select_cell_range(
            table_id,
            cell_ref.row.min(target_row),
            cell_ref.column.min(target_col),
            cell_ref.row.max(target_row),
            cell_ref.column.max(target_col),
        );
        self.update_cursor_display();
        self.base_mut().emit_signal("selection_changed", &[]);
        true
    }

    /// Handle drag-select with auto-scrolling when mouse is above/below viewport.
    fn handle_drag_select(&mut self, mouse_pos: Vector2) {
        let view_height = self.base().get_size().y;
        let auto_scroll_margin = 20.0;
        let auto_scroll_speed = 60.0;

        // Auto-scroll if mouse is near/past viewport edges
        if mouse_pos.y < auto_scroll_margin {
            // Mouse above viewport, scroll up
            let intensity = (auto_scroll_margin - mouse_pos.y) / auto_scroll_margin;
            self.scroll_by(-auto_scroll_speed * intensity);
        } else if mouse_pos.y > view_height - auto_scroll_margin {
            // Mouse below viewport, scroll down
            let intensity = (mouse_pos.y - (view_height - auto_scroll_margin)) / auto_scroll_margin;
            self.scroll_by(auto_scroll_speed * intensity);
        }

        // Clamp the hit-test Y to the visible content area
        let clamped_y = mouse_pos.y.clamp(2.0, view_height - 2.0);

        let zoom = self.typesetter.as_ref().map(|ts| ts.zoom()).unwrap_or(1.0);
        let hx = mouse_pos.x + self.h_scroll_offset * zoom;
        let hit = self
            .typesetter
            .as_ref()
            .and_then(|ts| ts.hit_test(hx, clamped_y));

        if let Some(hit) = hit
            && let Some(cursor) = &self.cursor
        {
            cursor.set_position(hit.position, MoveMode::KeepAnchor);
            self.update_cursor_display();
        }
    }

    fn handle_click(&mut self, position: Vector2, extend_selection: bool) {
        // Adjust for horizontal scroll offset (h_scroll_offset is in document space)
        let zoom = self.typesetter.as_ref().map(|ts| ts.zoom()).unwrap_or(1.0);
        let hx = position.x + self.h_scroll_offset * zoom;
        let hit = self
            .typesetter
            .as_ref()
            .and_then(|ts| ts.hit_test(hx, position.y));

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

        // Position cursor
        if let Some(cursor) = &self.cursor {
            let mode = if extend_selection {
                MoveMode::KeepAnchor
            } else {
                MoveMode::MoveAnchor
            };
            cursor.set_position(hit.position, mode);
            self.update_cursor_display();
        }
    }

    fn handle_readonly_input(&mut self, event: &Gd<InputEvent>) {
        let action = input::translate_input(event);
        match action {
            InputAction::Click { position } => {
                let zoom = self.typesetter.as_ref().map(|ts| ts.zoom()).unwrap_or(1.0);
                let hx = position.x + self.h_scroll_offset * zoom;
                let hit_region = self
                    .typesetter
                    .as_ref()
                    .and_then(|ts| ts.hit_test(hx, position.y))
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
            InputAction::ScrollLeft => self.scroll_h_by(-40.0),
            InputAction::ScrollRight => self.scroll_h_by(40.0),
            _ => {}
        }
    }

    /// Adjust horizontal scroll to keep the caret visible.
    fn ensure_caret_h_visible(&mut self) {
        if self.h_scrollbar.is_none() {
            return; // No horizontal scrolling in word-wrap mode
        }
        let Some(cursor) = &self.cursor else { return };
        let Some(ts) = &self.typesetter else { return };
        let zoom = ts.zoom();
        let caret = ts.caret_rect(cursor.position());
        let caret_x = caret[0]; // zoomed screen-space X
        let view_width = self.base().get_size().x;
        let margin = 20.0;

        // caret_x is in zoomed screen space. h_scroll_offset is in document space.
        // The displayed X = caret_x - h_scroll_offset * zoom.
        let screen_x = caret_x - self.h_scroll_offset * zoom;

        if screen_x < margin {
            // Convert back to document space for h_scroll_offset
            self.h_scroll_offset = ((caret_x - margin) / zoom).max(0.0);
            self.needs_redraw = true;
        } else if screen_x > view_width - margin {
            self.h_scroll_offset = (caret_x - view_width + margin) / zoom;
            self.needs_redraw = true;
        }
    }

    fn scroll_h_by(&mut self, delta: f32) {
        if self.h_scrollbar.is_none() {
            return; // No horizontal scrolling in word-wrap mode
        }
        let view_width = self.base().get_size().x;
        let zoom = self.typesetter.as_ref().map(|ts| ts.zoom()).unwrap_or(1.0);
        let max_width = self
            .typesetter
            .as_ref()
            .map(|ts| ts.max_content_width())
            .unwrap_or(0.0);
        // delta is in screen pixels; h_scroll_offset is in document space
        self.h_scroll_offset = (self.h_scroll_offset + delta / zoom).max(0.0);
        let max_scroll = (max_width - view_width / zoom).max(0.0);
        self.h_scroll_offset = self.h_scroll_offset.min(max_scroll);
        self.update_scrollbar();
        self.content_dirty = true;
        self.needs_redraw = true;
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
            self.content_dirty = true;
            self.needs_redraw = true;
        }
        self.update_scrollbar();
    }

    fn update_scrollbar(&mut self) {
        let size = self.base().get_size();
        let zoom = self.typesetter.as_ref().map(|ts| ts.zoom()).unwrap_or(1.0) as f64;
        let content_height = self
            .typesetter
            .as_ref()
            .map(|ts| ts.content_height() as f64)
            .unwrap_or(0.0);
        let max_content_width = self
            .typesetter
            .as_ref()
            .map(|ts| ts.max_content_width() as f64)
            .unwrap_or(0.0);

        let has_hbar = self.h_scrollbar.is_some();
        let has_vbar = self.v_scrollbar.is_some();
        let hsb_height = if has_hbar { 12.0_f32 } else { 0.0 };
        let vsb_width = if has_vbar { 12.0_f32 } else { 0.0 };

        let v_page = size.y as f64 / zoom;
        let h_page = size.x as f64 / zoom;
        let v_visible = content_height > v_page;
        let h_visible = max_content_width > h_page;

        // Word-wrap viewport adjustment is now handled in process()
        // before layout, so no re-layout is needed here.

        // VScrollBar: right edge, full height minus HScrollBar if visible
        if let Some(scrollbar) = &mut self.v_scrollbar {
            let sb_width = scrollbar.get_size().x.max(vsb_width);
            let h = if h_visible {
                size.y - hsb_height
            } else {
                size.y
            };
            scrollbar.set_position(Vector2::new(size.x - sb_width, 0.0));
            scrollbar.set_size(Vector2::new(sb_width, h));
            scrollbar.set_max(content_height);
            scrollbar.set_page(h as f64 / zoom);
            scrollbar.set_value_no_signal(self.scroll_offset as f64);
            scrollbar.set_visible(v_visible);
        }

        // HScrollBar: bottom edge, full width minus VScrollBar if visible
        if let Some(scrollbar) = &mut self.h_scrollbar {
            let sb_height = scrollbar.get_size().y.max(hsb_height);
            let w = if v_visible {
                size.x - vsb_width
            } else {
                size.x
            };
            scrollbar.set_position(Vector2::new(0.0, size.y - sb_height));
            scrollbar.set_size(Vector2::new(w, sb_height));
            scrollbar.set_max(max_content_width);
            scrollbar.set_page(w as f64 / zoom);
            scrollbar.set_value_no_signal(self.h_scroll_offset as f64);
            scrollbar.set_visible(h_visible);
        }
    }

    fn clipboard_copy(&mut self) {
        let Some(cursor) = &self.cursor else { return };
        if !cursor.has_selection() {
            return;
        }

        // Store rich HTML for internal paste
        let fragment = cursor.selection();
        let plain = fragment.to_plain_text().to_string();
        if plain.is_empty() {
            return;
        }
        let html = fragment.to_html();

        // Set system clipboard with plain text; keep selection
        DisplayServer::singleton().clipboard_set(&GString::from(plain.as_str()));
        self.rich_clipboard_html = Some(html);
        self.rich_clipboard_plain = Some(plain);
    }

    fn clipboard_paste(&mut self) {
        let Some(cursor) = &self.cursor else { return };
        let system_text = DisplayServer::singleton().clipboard_get().to_string();

        // If system clipboard matches what we copied, paste rich HTML
        if let (Some(html), Some(our_plain)) =
            (&self.rich_clipboard_html, &self.rich_clipboard_plain)
            && system_text == *our_plain
        {
            let _ = cursor.insert_html(html);
            cursor.clear_selection();
            self.update_cursor_display();
            return;
        }

        // Otherwise paste plain text from system clipboard
        if !system_text.is_empty() {
            let _ = cursor.insert_text(&system_text);
            cursor.clear_selection();
            self.update_cursor_display();
        }
    }

    fn toggle_bold(&mut self) {
        let current = self
            .cursor
            .as_ref()
            .and_then(|c| c.char_format().ok())
            .and_then(|f| f.font_bold)
            .unwrap_or(false);
        self.set_bold(!current);
    }

    fn toggle_italic(&mut self) {
        let current = self
            .cursor
            .as_ref()
            .and_then(|c| c.char_format().ok())
            .and_then(|f| f.font_italic)
            .unwrap_or(false);
        self.set_italic(!current);
    }

    fn toggle_underline(&mut self) {
        let current = self
            .cursor
            .as_ref()
            .and_then(|c| c.char_format().ok())
            .and_then(|f| f.font_underline)
            .unwrap_or(false);
        self.set_underline(!current);
    }
}
