## Example: toolbar and context menu for RichTextEdit.
##
## This script demonstrates how to build UI chrome around the low-level
## RichTextEdit control. It is NOT part of the extension — copy and adapt
## it for your own project.
extends Control

enum CtxItem { CUT, COPY, PASTE, SELECT_ALL, UNDO, REDO }

var editor: RichTextEdit
var btn_bold: Button
var btn_italic: Button
var btn_underline: Button
var btn_strike: Button
var heading_picker: OptionButton
var btn_list_bullet: Button
var btn_list_number: Button
var btn_table_insert: Button
var btn_table_row_above: Button
var btn_table_row_below: Button
var btn_table_col_before: Button
var btn_table_col_after: Button
var btn_table_del_row: Button
var btn_table_del_col: Button
var btn_table_remove: Button
var btn_undo: Button
var btn_redo: Button
var context_menu: PopupMenu

## Prevent toolbar state updates from re-triggering button signals.
var _updating_toolbar := false


func _ready() -> void:
	editor = $VBox/HSplit/EditorBg/Editor
	btn_bold = $VBox/Toolbar/BtnBold
	btn_italic = $VBox/Toolbar/BtnItalic
	btn_underline = $VBox/Toolbar/BtnUnderline
	btn_strike = $VBox/Toolbar/BtnStrike
	heading_picker = $VBox/Toolbar/HeadingPicker
	btn_list_bullet = $VBox/Toolbar/BtnListBullet
	btn_list_number = $VBox/Toolbar/BtnListNumber
	btn_table_insert = $VBox/Toolbar/BtnTableInsert
	btn_table_row_above = $VBox/Toolbar2/BtnTableRowAbove
	btn_table_row_below = $VBox/Toolbar2/BtnTableRowBelow
	btn_table_col_before = $VBox/Toolbar2/BtnTableColBefore
	btn_table_col_after = $VBox/Toolbar2/BtnTableColAfter
	btn_table_del_row = $VBox/Toolbar2/BtnTableDelRow
	btn_table_del_col = $VBox/Toolbar2/BtnTableDelCol
	btn_table_remove = $VBox/Toolbar2/BtnTableRemove
	btn_undo = $VBox/Toolbar/BtnUndo
	btn_redo = $VBox/Toolbar/BtnRedo
	context_menu = $ContextMenu

	# Prevent toolbar buttons from stealing focus from the editor
	for btn: Button in [btn_bold, btn_italic, btn_underline, btn_strike,
			btn_list_bullet, btn_list_number, btn_table_insert,
			btn_table_row_above, btn_table_row_below,
			btn_table_col_before, btn_table_col_after,
			btn_table_del_row, btn_table_del_col, btn_table_remove,
			btn_undo, btn_redo]:
		btn.focus_mode = Control.FOCUS_NONE
	heading_picker.focus_mode = Control.FOCUS_NONE

	# Toolbar connections — use toggled for toggle buttons
	btn_bold.toggled.connect(_on_bold)
	btn_italic.toggled.connect(_on_italic)
	btn_underline.toggled.connect(_on_underline)
	btn_strike.toggled.connect(_on_strike)
	heading_picker.item_selected.connect(_on_heading_selected)
	btn_list_bullet.pressed.connect(_on_list_bullet)
	btn_list_number.pressed.connect(_on_list_number)
	btn_table_insert.pressed.connect(_on_table_insert)
	btn_table_row_above.pressed.connect(func() -> void: editor.insert_row_above())
	btn_table_row_below.pressed.connect(func() -> void: editor.insert_row_below())
	btn_table_col_before.pressed.connect(func() -> void: editor.insert_column_before())
	btn_table_col_after.pressed.connect(func() -> void: editor.insert_column_after())
	btn_table_del_row.pressed.connect(func() -> void: editor.remove_current_row())
	btn_table_del_col.pressed.connect(func() -> void: editor.remove_current_column())
	btn_table_remove.pressed.connect(func() -> void: editor.remove_current_table())
	btn_undo.pressed.connect(func() -> void: editor.undo())
	btn_redo.pressed.connect(func() -> void: editor.redo())

	# Editor signal connections for toolbar state
	editor.caret_changed.connect(_update_toolbar_state)
	editor.selection_changed.connect(_update_toolbar_state)
	editor.format_changed.connect(_update_toolbar_state)
	editor.text_changed.connect(_update_toolbar_state)
	editor.undo_redo_changed.connect(_on_undo_redo_changed)

	# Context menu
	context_menu.id_pressed.connect(_on_context_item)

	# Heading picker items
	heading_picker.add_item("Normal", 0)
	heading_picker.add_item("H1", 1)
	heading_picker.add_item("H2", 2)
	heading_picker.add_item("H3", 3)
	heading_picker.add_item("H4", 4)
	heading_picker.add_item("H5", 5)
	heading_picker.add_item("H6", 6)

	_update_toolbar_state()
	_on_undo_redo_changed(editor.can_undo(), editor.can_redo())


# --- Toolbar handlers ---

func _on_bold(pressed: bool) -> void:
	if _updating_toolbar:
		return
	editor.set_bold(pressed)


func _on_italic(pressed: bool) -> void:
	if _updating_toolbar:
		return
	editor.set_italic(pressed)


func _on_underline(pressed: bool) -> void:
	if _updating_toolbar:
		return
	editor.set_underline(pressed)


func _on_strike(pressed: bool) -> void:
	if _updating_toolbar:
		return
	editor.set_strikethrough(pressed)


func _on_heading_selected(index: int) -> void:
	if _updating_toolbar:
		return
	editor.set_heading_level(heading_picker.get_item_id(index))


func _on_list_bullet() -> void:
	editor.insert_list(false)


func _on_list_number() -> void:
	editor.insert_list(true)


func _on_table_insert() -> void:
	editor.insert_table(3, 3)


# --- Toolbar state ---

func _update_toolbar_state() -> void:
	_updating_toolbar = true
	btn_bold.button_pressed = editor.is_bold()
	btn_italic.button_pressed = editor.is_italic()
	btn_underline.button_pressed = editor.is_underline()
	btn_strike.button_pressed = editor.is_strikethrough()

	var level := editor.get_heading_level()
	for i in heading_picker.item_count:
		if heading_picker.get_item_id(i) == level:
			heading_picker.select(i)
			break

	# Enable/disable table management buttons based on cursor context
	var in_table := editor.is_in_table()
	btn_table_row_above.disabled = !in_table
	btn_table_row_below.disabled = !in_table
	btn_table_col_before.disabled = !in_table
	btn_table_col_after.disabled = !in_table
	btn_table_del_row.disabled = !in_table
	btn_table_del_col.disabled = !in_table
	btn_table_remove.disabled = !in_table
	_updating_toolbar = false


func _on_undo_redo_changed(can_undo: bool, can_redo: bool) -> void:
	btn_undo.disabled = !can_undo
	btn_redo.disabled = !can_redo


# --- Context menu ---

func _input(event: InputEvent) -> void:
	if event is InputEventMouseButton and event.pressed:
		if event.button_index == MOUSE_BUTTON_RIGHT:
			var local: Vector2 = editor.get_global_transform().affine_inverse() * event.position
			if editor.get_rect().has_point(local + editor.position):
				_show_context_menu(event.global_position)
				get_viewport().set_input_as_handled()


func _show_context_menu(pos: Vector2) -> void:
	context_menu.clear()
	context_menu.add_item("Cut", CtxItem.CUT)
	context_menu.add_item("Copy", CtxItem.COPY)
	context_menu.add_item("Paste", CtxItem.PASTE)
	context_menu.add_separator()
	context_menu.add_item("Select All", CtxItem.SELECT_ALL)
	context_menu.add_separator()
	context_menu.add_item("Undo", CtxItem.UNDO)
	context_menu.add_item("Redo", CtxItem.REDO)

	# Disable items based on state
	var has_sel := editor.has_selection()
	context_menu.set_item_disabled(context_menu.get_item_index(CtxItem.CUT), !has_sel)
	context_menu.set_item_disabled(context_menu.get_item_index(CtxItem.COPY), !has_sel)
	context_menu.set_item_disabled(context_menu.get_item_index(CtxItem.UNDO), !editor.can_undo())
	context_menu.set_item_disabled(context_menu.get_item_index(CtxItem.REDO), !editor.can_redo())

	context_menu.position = Vector2i(pos)
	context_menu.popup()


func _on_context_item(id: int) -> void:
	match id:
		CtxItem.CUT:
			editor.grab_focus()
			editor.cut_rich()
		CtxItem.COPY:
			editor.copy_rich()
		CtxItem.PASTE:
			editor.grab_focus()
			editor.paste_rich()
		CtxItem.SELECT_ALL:
			editor.grab_focus()
			editor.select_all()
		CtxItem.UNDO:
			editor.grab_focus()
			editor.undo()
		CtxItem.REDO:
			editor.grab_focus()
			editor.redo()
