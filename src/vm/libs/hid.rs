//! Native keyboard, mouse, screen and clipboard access backing `@neyuki/hid`.
//!
//! Arguments are validated before the input device is opened, so a bad key
//! name fails the same way on a machine without a display.

use std::cell::RefCell;

use arboard::Clipboard;
use enigo::{Axis, Button, Coordinate, Direction, Enigo, Key, Keyboard, Mouse, Settings};

use crate::vm::libs::args::{bool_arg, int_arg, str_arg};
use crate::vm::machine::VM;
use crate::vm::value::{NativeDef, Value};

pub(crate) const NATIVES: &[NativeDef] = &[
    NativeDef {
        name: "__hid_key",
        func: builtin_key,
    },
    NativeDef {
        name: "__hid_is_key",
        func: builtin_is_key,
    },
    NativeDef {
        name: "__hid_text",
        func: builtin_text,
    },
    NativeDef {
        name: "__hid_button",
        func: builtin_button,
    },
    NativeDef {
        name: "__hid_is_button",
        func: builtin_is_button,
    },
    NativeDef {
        name: "__hid_move",
        func: builtin_move,
    },
    NativeDef {
        name: "__hid_scroll",
        func: builtin_scroll,
    },
    NativeDef {
        name: "__hid_position",
        func: builtin_position,
    },
    NativeDef {
        name: "__hid_screen_size",
        func: builtin_screen_size,
    },
    NativeDef {
        name: "__hid_clipboard_get",
        func: builtin_clipboard_get,
    },
    NativeDef {
        name: "__hid_clipboard_set",
        func: builtin_clipboard_set,
    },
];

thread_local! {
    static ENIGO: RefCell<Option<Enigo>> = const { RefCell::new(None) };
    static CLIPBOARD: RefCell<Option<Clipboard>> = const { RefCell::new(None) };
}

/// Runs `f` with the input device, opening it on first use.
fn with_enigo<T>(f: impl FnOnce(&mut Enigo) -> Result<T, String>) -> Result<T, String> {
    ENIGO.with(|cell| {
        let mut slot = cell.borrow_mut();
        if slot.is_none() {
            let enigo = Enigo::new(&Settings::default())
                .map_err(|err| format!("cannot open the input device: {}", err))?;
            *slot = Some(enigo);
        }
        f(slot.as_mut().expect("opened above"))
    })
}

fn with_clipboard<T>(f: impl FnOnce(&mut Clipboard) -> Result<T, String>) -> Result<T, String> {
    CLIPBOARD.with(|cell| {
        let mut slot = cell.borrow_mut();
        if slot.is_none() {
            let clipboard =
                Clipboard::new().map_err(|err| format!("cannot open the clipboard: {}", err))?;
            *slot = Some(clipboard);
        }
        f(slot.as_mut().expect("opened above"))
    })
}

fn input_error(err: enigo::InputError) -> String {
    format!("input failed: {}", err)
}

/// Every key name `@neyuki/hid` accepts besides single characters.
const KEY_NAMES: &[(&str, Key)] = &[
    ("alt", Key::Alt),
    ("backspace", Key::Backspace),
    ("capslock", Key::CapsLock),
    ("ctrl", Key::Control),
    ("control", Key::Control),
    ("lctrl", Key::LControl),
    ("rctrl", Key::RControl),
    ("delete", Key::Delete),
    ("down", Key::DownArrow),
    ("end", Key::End),
    ("enter", Key::Return),
    ("return", Key::Return),
    ("escape", Key::Escape),
    ("esc", Key::Escape),
    ("f1", Key::F1),
    ("f2", Key::F2),
    ("f3", Key::F3),
    ("f4", Key::F4),
    ("f5", Key::F5),
    ("f6", Key::F6),
    ("f7", Key::F7),
    ("f8", Key::F8),
    ("f9", Key::F9),
    ("f10", Key::F10),
    ("f11", Key::F11),
    ("f12", Key::F12),
    ("f13", Key::F13),
    ("f14", Key::F14),
    ("f15", Key::F15),
    ("f16", Key::F16),
    ("f17", Key::F17),
    ("f18", Key::F18),
    ("f19", Key::F19),
    ("f20", Key::F20),
    ("help", Key::Help),
    ("home", Key::Home),
    ("left", Key::LeftArrow),
    ("meta", Key::Meta),
    ("super", Key::Meta),
    ("win", Key::Meta),
    ("cmd", Key::Meta),
    ("command", Key::Meta),
    ("option", Key::Option),
    ("pagedown", Key::PageDown),
    ("pageup", Key::PageUp),
    ("right", Key::RightArrow),
    ("shift", Key::Shift),
    ("lshift", Key::LShift),
    ("rshift", Key::RShift),
    ("space", Key::Space),
    ("tab", Key::Tab),
    ("up", Key::UpArrow),
    ("numpad0", Key::Numpad0),
    ("numpad1", Key::Numpad1),
    ("numpad2", Key::Numpad2),
    ("numpad3", Key::Numpad3),
    ("numpad4", Key::Numpad4),
    ("numpad5", Key::Numpad5),
    ("numpad6", Key::Numpad6),
    ("numpad7", Key::Numpad7),
    ("numpad8", Key::Numpad8),
    ("numpad9", Key::Numpad9),
    ("add", Key::Add),
    ("subtract", Key::Subtract),
    ("multiply", Key::Multiply),
    ("divide", Key::Divide),
    ("decimal", Key::Decimal),
    ("volumeup", Key::VolumeUp),
    ("volumedown", Key::VolumeDown),
    ("mute", Key::VolumeMute),
    ("playpause", Key::MediaPlayPause),
    ("nexttrack", Key::MediaNextTrack),
    ("prevtrack", Key::MediaPrevTrack),
];

/// A key from its name (case-insensitive) or a single character.
fn parse_key(name: &str) -> Result<Key, String> {
    let mut chars = name.chars();
    if let (Some(c), None) = (chars.next(), chars.next()) {
        // Letters go through as lowercase so `"A"` means the A key, not a
        // shifted one; shift is the caller's to hold.
        return Ok(Key::Unicode(c.to_ascii_lowercase()));
    }
    let lower = name.to_ascii_lowercase();
    KEY_NAMES
        .iter()
        .find(|(known, _)| *known == lower)
        .map(|(_, key)| *key)
        .ok_or_else(|| format!("unknown key `{}`", name))
}

fn parse_button(name: &str) -> Result<Button, String> {
    match name.to_ascii_lowercase().as_str() {
        "left" => Ok(Button::Left),
        "right" => Ok(Button::Right),
        "middle" => Ok(Button::Middle),
        "back" => Ok(Button::Back),
        "forward" => Ok(Button::Forward),
        _ => Err(format!(
            "unknown mouse button `{}` (expected left, right, middle, back or forward)",
            name
        )),
    }
}

fn parse_direction(name: &str) -> Result<Direction, String> {
    match name {
        "press" => Ok(Direction::Press),
        "release" => Ok(Direction::Release),
        "click" => Ok(Direction::Click),
        _ => Err(format!(
            "invalid direction `{}` (expected press, release or click)",
            name
        )),
    }
}

fn coordinate(args: &[Value], index: usize, name: &str) -> Result<i32, String> {
    let value = int_arg(args, index, name)?;
    i32::try_from(value).map_err(|_| format!("{} is out of range", name))
}

/// `(key, direction)`: presses, releases or clicks one key.
fn builtin_key(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let key = parse_key(str_arg(args, 0, "key")?)?;
    let direction = parse_direction(str_arg(args, 1, "direction")?)?;
    with_enigo(|enigo| enigo.key(key, direction).map_err(input_error))?;
    Ok(vec![])
}

/// `(key)`: whether a key name is known, without touching the device.
fn builtin_is_key(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let name = str_arg(args, 0, "key")?;
    Ok(vec![Value::Bool(parse_key(name).is_ok())])
}

/// `(text)`: types text as if entered on the keyboard.
fn builtin_text(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let text = str_arg(args, 0, "text")?;
    if text.is_empty() {
        return Ok(vec![]);
    }
    with_enigo(|enigo| enigo.text(text).map_err(input_error))?;
    Ok(vec![])
}

/// `(button, direction)`: presses, releases or clicks a mouse button.
fn builtin_button(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let button = parse_button(str_arg(args, 0, "button")?)?;
    let direction = parse_direction(str_arg(args, 1, "direction")?)?;
    with_enigo(|enigo| enigo.button(button, direction).map_err(input_error))?;
    Ok(vec![])
}

/// `(button)`: whether a mouse button name is known.
fn builtin_is_button(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let name = str_arg(args, 0, "button")?;
    Ok(vec![Value::Bool(parse_button(name).is_ok())])
}

/// `(x, y, relative)`: moves the pointer to a screen position, or by an
/// offset when `relative` is true.
fn builtin_move(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let x = coordinate(args, 0, "x")?;
    let y = coordinate(args, 1, "y")?;
    let mode = if bool_arg(args, 2, "relative")? {
        Coordinate::Rel
    } else {
        Coordinate::Abs
    };
    with_enigo(|enigo| enigo.move_mouse(x, y, mode).map_err(input_error))?;
    Ok(vec![])
}

/// `(amount, horizontal)`: scrolls by whole wheel steps; positive amounts
/// scroll down (or right).
fn builtin_scroll(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let amount = coordinate(args, 0, "amount")?;
    let axis = if bool_arg(args, 1, "horizontal")? {
        Axis::Horizontal
    } else {
        Axis::Vertical
    };
    if amount != 0 {
        with_enigo(|enigo| enigo.scroll(amount, axis).map_err(input_error))?;
    }
    Ok(vec![])
}

/// `()`: the pointer position in screen pixels.
fn builtin_position(_vm: &mut VM, _args: &[Value]) -> Result<Vec<Value>, String> {
    let (x, y) = with_enigo(|enigo| enigo.location().map_err(input_error))?;
    Ok(vec![Value::Int(x as i64), Value::Int(y as i64)])
}

/// `()`: the main display size in pixels.
fn builtin_screen_size(_vm: &mut VM, _args: &[Value]) -> Result<Vec<Value>, String> {
    let (width, height) = with_enigo(|enigo| enigo.main_display().map_err(input_error))?;
    Ok(vec![Value::Int(width as i64), Value::Int(height as i64)])
}

/// `()`: the clipboard text, or nil when it holds none.
fn builtin_clipboard_get(_vm: &mut VM, _args: &[Value]) -> Result<Vec<Value>, String> {
    with_clipboard(|clipboard| match clipboard.get_text() {
        Ok(text) => Ok(vec![Value::string(text)]),
        Err(arboard::Error::ContentNotAvailable) => Ok(vec![Value::Nil]),
        Err(err) => Err(format!("cannot read the clipboard: {}", err)),
    })
}

/// `(text)`: replaces the clipboard contents with text.
fn builtin_clipboard_set(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let text = str_arg(args, 0, "text")?;
    with_clipboard(|clipboard| {
        clipboard
            .set_text(text)
            .map_err(|err| format!("cannot write the clipboard: {}", err))
    })?;
    Ok(vec![])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_names() {
        assert_eq!(parse_key("a").unwrap(), Key::Unicode('a'));
        assert_eq!(parse_key("A").unwrap(), Key::Unicode('a'));
        assert_eq!(parse_key("Enter").unwrap(), Key::Return);
        assert_eq!(parse_key("ctrl").unwrap(), parse_key("control").unwrap());
        assert!(parse_key("").is_err());
        assert!(parse_key("hyper").is_err());
    }

    #[test]
    fn buttons_and_directions() {
        assert_eq!(parse_button("Right").unwrap(), Button::Right);
        assert!(parse_button("fourth").is_err());
        assert!(parse_direction("tap").is_err());
    }
}
