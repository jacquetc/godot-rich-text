use godot::builtin::Vector2;
use godot::classes::{InputEvent, InputEventKey, InputEventMouseButton, InputEventMouseMotion};
use godot::global::{Key, MouseButton};
use godot::obj::{EngineBitfield, EngineEnum, Gd};

#[derive(Debug, Clone)]
pub enum InputAction {
    None,
    // Text entry
    InsertChar(char),
    // Navigation
    MoveLeft,
    MoveRight,
    MoveUp,
    MoveDown,
    MoveWordLeft,
    MoveWordRight,
    MoveHome,
    MoveEnd,
    MoveDocStart,
    MoveDocEnd,
    PageUp,
    PageDown,
    // Selection variants
    SelectLeft,
    SelectRight,
    SelectUp,
    SelectDown,
    SelectWordLeft,
    SelectWordRight,
    SelectHome,
    SelectEnd,
    SelectDocStart,
    SelectDocEnd,
    SelectAll,
    // Editing
    Backspace,
    Delete,
    DeleteWordLeft,
    DeleteWordRight,
    Enter,
    CtrlEnter,
    Tab,
    ShiftTab,
    // Clipboard
    Cut,
    Copy,
    Paste,
    // Undo/redo
    Undo,
    Redo,
    // Mouse
    Click { position: Vector2 },
    ShiftClick { position: Vector2 },
    DragSelect { position: Vector2 },
    DoubleClick { position: Vector2 },
    // Scroll
    ScrollUp,
    ScrollDown,
    ScrollLeft,
    ScrollRight,
    // Formatting shortcuts
    ToggleBold,
    ToggleItalic,
    ToggleUnderline,
}

pub fn translate_input(event: &Gd<InputEvent>) -> InputAction {
    if let Ok(key_event) = event.clone().try_cast::<InputEventKey>()
        && (key_event.is_pressed() || key_event.is_echo())
    {
        return translate_key(&key_event);
    }

    if let Ok(mb_event) = event.clone().try_cast::<InputEventMouseButton>()
        && mb_event.is_pressed()
    {
        return translate_mouse_button(&mb_event);
    }

    if let Ok(motion_event) = event.clone().try_cast::<InputEventMouseMotion>() {
        return translate_mouse_motion(&motion_event);
    }

    InputAction::None
}

fn translate_key(event: &Gd<InputEventKey>) -> InputAction {
    let keycode = event.get_keycode();
    let ctrl = event.is_ctrl_pressed();
    let shift = event.is_shift_pressed();

    // Ctrl shortcuts
    if ctrl && !shift {
        if keycode == Key::A {
            return InputAction::SelectAll;
        }
        if keycode == Key::C {
            return InputAction::Copy;
        }
        if keycode == Key::X {
            return InputAction::Cut;
        }
        if keycode == Key::V {
            return InputAction::Paste;
        }
        if keycode == Key::Z {
            return InputAction::Undo;
        }
        if keycode == Key::Y {
            return InputAction::Redo;
        }
        if keycode == Key::B {
            return InputAction::ToggleBold;
        }
        if keycode == Key::I {
            return InputAction::ToggleItalic;
        }
        if keycode == Key::U {
            return InputAction::ToggleUnderline;
        }
        if keycode == Key::BACKSPACE {
            return InputAction::DeleteWordLeft;
        }
        if keycode == Key::DELETE {
            return InputAction::DeleteWordRight;
        }
    }

    // Ctrl+Shift shortcuts
    if ctrl && shift {
        if keycode == Key::Z {
            return InputAction::Redo;
        }
        if keycode == Key::HOME {
            return InputAction::SelectDocStart;
        }
        if keycode == Key::END {
            return InputAction::SelectDocEnd;
        }
    }

    // Navigation with Ctrl (word/doc movement)
    if ctrl && !shift {
        if keycode == Key::LEFT {
            return InputAction::MoveWordLeft;
        }
        if keycode == Key::RIGHT {
            return InputAction::MoveWordRight;
        }
        if keycode == Key::HOME {
            return InputAction::MoveDocStart;
        }
        if keycode == Key::END {
            return InputAction::MoveDocEnd;
        }
    }

    // Navigation with Ctrl+Shift (word selection)
    if ctrl && shift {
        if keycode == Key::LEFT {
            return InputAction::SelectWordLeft;
        }
        if keycode == Key::RIGHT {
            return InputAction::SelectWordRight;
        }
    }

    // Navigation with Shift (selection)
    if shift && !ctrl {
        if keycode == Key::LEFT {
            return InputAction::SelectLeft;
        }
        if keycode == Key::RIGHT {
            return InputAction::SelectRight;
        }
        if keycode == Key::UP {
            return InputAction::SelectUp;
        }
        if keycode == Key::DOWN {
            return InputAction::SelectDown;
        }
        if keycode == Key::HOME {
            return InputAction::SelectHome;
        }
        if keycode == Key::END {
            return InputAction::SelectEnd;
        }
    }

    // Enter: Ctrl+Enter is a separate action
    if keycode == Key::ENTER || keycode == Key::KP_ENTER {
        if ctrl {
            return InputAction::CtrlEnter;
        }
        return InputAction::Enter;
    }

    // Keys that work regardless of Shift state
    if !ctrl {
        if keycode == Key::BACKSPACE {
            return InputAction::Backspace;
        }
        if keycode == Key::DELETE {
            return InputAction::Delete;
        }
        if keycode == Key::TAB {
            if shift {
                return InputAction::ShiftTab;
            }
            return InputAction::Tab;
        }
    }

    // Plain navigation (no modifiers)
    if !ctrl && !shift {
        if keycode == Key::LEFT {
            return InputAction::MoveLeft;
        }
        if keycode == Key::RIGHT {
            return InputAction::MoveRight;
        }
        if keycode == Key::UP {
            return InputAction::MoveUp;
        }
        if keycode == Key::DOWN {
            return InputAction::MoveDown;
        }
        if keycode == Key::HOME {
            return InputAction::MoveHome;
        }
        if keycode == Key::END {
            return InputAction::MoveEnd;
        }
        if keycode == Key::PAGEUP {
            return InputAction::PageUp;
        }
        if keycode == Key::PAGEDOWN {
            return InputAction::PageDown;
        }
    }

    // Printable character input
    let unicode = event.get_unicode();
    if unicode > 0
        && let Some(ch) = char::from_u32(unicode)
        && !ch.is_control()
    {
        return InputAction::InsertChar(ch);
    }

    InputAction::None
}

fn translate_mouse_button(event: &Gd<InputEventMouseButton>) -> InputAction {
    let button = event.get_button_index();
    let position = event.get_position();

    if button == MouseButton::LEFT {
        if event.is_double_click() {
            return InputAction::DoubleClick { position };
        }
        if event.is_shift_pressed() {
            return InputAction::ShiftClick { position };
        }
        return InputAction::Click { position };
    }

    if button == MouseButton::WHEEL_UP {
        if event.is_shift_pressed() {
            return InputAction::ScrollLeft;
        }
        return InputAction::ScrollUp;
    }
    if button == MouseButton::WHEEL_DOWN {
        if event.is_shift_pressed() {
            return InputAction::ScrollRight;
        }
        return InputAction::ScrollDown;
    }
    if button == MouseButton::WHEEL_LEFT {
        return InputAction::ScrollLeft;
    }
    if button == MouseButton::WHEEL_RIGHT {
        return InputAction::ScrollRight;
    }

    InputAction::None
}

fn translate_mouse_motion(event: &Gd<InputEventMouseMotion>) -> InputAction {
    let mask = event.get_button_mask();
    // Check if left button is held during motion (drag-select)
    if (mask.ord() as i32) & MouseButton::LEFT.ord() != 0 {
        return InputAction::DragSelect {
            position: event.get_position(),
        };
    }
    InputAction::None
}
