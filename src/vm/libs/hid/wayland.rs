//! Keyboard and mouse input on Wayland through the virtual keyboard
//! (`zwp_virtual_keyboard_v1`) and virtual pointer (`zwlr_virtual_pointer_v1`)
//! protocols, which niri, Sway, Hyprland and other wlroots-based compositors
//! offer.
//!
//! The virtual keyboard needs an XKB keymap. Compiling one with libxkbcommon
//! would make the runtime link a C library, so the keymap text is written
//! here instead: common keys keep their evdev code from a US layout, so
//! shortcuts and games see the keys they expect, and any other character is
//! given a spare code when it is first typed.

use std::collections::HashMap;
use std::io::{Seek, Write};
use std::os::fd::AsFd;
use std::time::Instant;

use enigo::{Axis, Button, Coordinate, Direction, Key};
use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::{wl_output, wl_pointer, wl_registry, wl_seat};
use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle, WEnum, delegate_noop};
use wayland_protocols_misc::zwp_virtual_keyboard_v1::client::{
    zwp_virtual_keyboard_manager_v1::ZwpVirtualKeyboardManagerV1,
    zwp_virtual_keyboard_v1::ZwpVirtualKeyboardV1,
};
use wayland_protocols_wlr::virtual_pointer::v1::client::{
    zwlr_virtual_pointer_manager_v1::ZwlrVirtualPointerManagerV1,
    zwlr_virtual_pointer_v1::ZwlrVirtualPointerV1,
};

/// The highest evdev code an XKB keymap can name (keycode 255 minus 8).
const MAX_CODE: u32 = 247;

/// One wheel step in Wayland axis units, the value libinput reports for a
/// physical wheel click.
const SCROLL_STEP: f64 = 15.0;

/// `wl_keyboard.keymap_format.xkb_v1`.
const XKB_V1: u32 = 1;

/// Keys that keep their evdev code from a US layout:
/// `(code, keysym, keysym name, shifted keysym name)`.
const STATIC_KEYS: &[(u32, u32, &str, Option<&str>)] = &[
    (1, 0xff1b, "Escape", None),
    (2, 0x31, "1", Some("exclam")),
    (3, 0x32, "2", Some("at")),
    (4, 0x33, "3", Some("numbersign")),
    (5, 0x34, "4", Some("dollar")),
    (6, 0x35, "5", Some("percent")),
    (7, 0x36, "6", Some("asciicircum")),
    (8, 0x37, "7", Some("ampersand")),
    (9, 0x38, "8", Some("asterisk")),
    (10, 0x39, "9", Some("parenleft")),
    (11, 0x30, "0", Some("parenright")),
    (12, 0x2d, "minus", Some("underscore")),
    (13, 0x3d, "equal", Some("plus")),
    (14, 0xff08, "BackSpace", None),
    (15, 0xff09, "Tab", None),
    (16, 0x71, "q", Some("Q")),
    (17, 0x77, "w", Some("W")),
    (18, 0x65, "e", Some("E")),
    (19, 0x72, "r", Some("R")),
    (20, 0x74, "t", Some("T")),
    (21, 0x79, "y", Some("Y")),
    (22, 0x75, "u", Some("U")),
    (23, 0x69, "i", Some("I")),
    (24, 0x6f, "o", Some("O")),
    (25, 0x70, "p", Some("P")),
    (26, 0x5b, "bracketleft", Some("braceleft")),
    (27, 0x5d, "bracketright", Some("braceright")),
    (28, 0xff0d, "Return", None),
    (29, 0xffe3, "Control_L", None),
    (30, 0x61, "a", Some("A")),
    (31, 0x73, "s", Some("S")),
    (32, 0x64, "d", Some("D")),
    (33, 0x66, "f", Some("F")),
    (34, 0x67, "g", Some("G")),
    (35, 0x68, "h", Some("H")),
    (36, 0x6a, "j", Some("J")),
    (37, 0x6b, "k", Some("K")),
    (38, 0x6c, "l", Some("L")),
    (39, 0x3b, "semicolon", Some("colon")),
    (40, 0x27, "apostrophe", Some("quotedbl")),
    (41, 0x60, "grave", Some("asciitilde")),
    (42, 0xffe1, "Shift_L", None),
    (43, 0x5c, "backslash", Some("bar")),
    (44, 0x7a, "z", Some("Z")),
    (45, 0x78, "x", Some("X")),
    (46, 0x63, "c", Some("C")),
    (47, 0x76, "v", Some("V")),
    (48, 0x62, "b", Some("B")),
    (49, 0x6e, "n", Some("N")),
    (50, 0x6d, "m", Some("M")),
    (51, 0x2c, "comma", Some("less")),
    (52, 0x2e, "period", Some("greater")),
    (53, 0x2f, "slash", Some("question")),
    (54, 0xffe2, "Shift_R", None),
    (55, 0xffaa, "KP_Multiply", None),
    (56, 0xffe9, "Alt_L", None),
    (57, 0x20, "space", None),
    (58, 0xffe5, "Caps_Lock", None),
    (59, 0xffbe, "F1", None),
    (60, 0xffbf, "F2", None),
    (61, 0xffc0, "F3", None),
    (62, 0xffc1, "F4", None),
    (63, 0xffc2, "F5", None),
    (64, 0xffc3, "F6", None),
    (65, 0xffc4, "F7", None),
    (66, 0xffc5, "F8", None),
    (67, 0xffc6, "F9", None),
    (68, 0xffc7, "F10", None),
    (71, 0xffb7, "KP_7", None),
    (72, 0xffb8, "KP_8", None),
    (73, 0xffb9, "KP_9", None),
    (74, 0xffad, "KP_Subtract", None),
    (75, 0xffb4, "KP_4", None),
    (76, 0xffb5, "KP_5", None),
    (77, 0xffb6, "KP_6", None),
    (78, 0xffab, "KP_Add", None),
    (79, 0xffb1, "KP_1", None),
    (80, 0xffb2, "KP_2", None),
    (81, 0xffb3, "KP_3", None),
    (82, 0xffb0, "KP_0", None),
    (83, 0xffae, "KP_Decimal", None),
    (87, 0xffc8, "F11", None),
    (88, 0xffc9, "F12", None),
    (97, 0xffe4, "Control_R", None),
    (98, 0xffaf, "KP_Divide", None),
    (102, 0xff50, "Home", None),
    (103, 0xff52, "Up", None),
    (104, 0xff55, "Prior", None),
    (105, 0xff51, "Left", None),
    (106, 0xff53, "Right", None),
    (107, 0xff57, "End", None),
    (108, 0xff54, "Down", None),
    (109, 0xff56, "Next", None),
    (110, 0xff63, "Insert", None),
    (111, 0xffff, "Delete", None),
    (113, 0x1008ff12, "XF86AudioMute", None),
    (114, 0x1008ff11, "XF86AudioLowerVolume", None),
    (115, 0x1008ff13, "XF86AudioRaiseVolume", None),
    (125, 0xffeb, "Super_L", None),
    (138, 0xff6a, "Help", None),
    (163, 0x1008ff17, "XF86AudioNext", None),
    (164, 0x1008ff14, "XF86AudioPlay", None),
    (165, 0x1008ff16, "XF86AudioPrev", None),
    (183, 0xffca, "F13", None),
    (184, 0xffcb, "F14", None),
    (185, 0xffcc, "F15", None),
    (186, 0xffcd, "F16", None),
    (187, 0xffce, "F17", None),
    (188, 0xffcf, "F18", None),
    (189, 0xffd0, "F19", None),
    (190, 0xffd1, "F20", None),
];

/// Modifier keys and the XKB real modifier they set:
/// `(code, modifier name, mask)`. The masks are the modifier indices every
/// XKB keymap starts with (Shift, Lock, Control, Mod1 ... Mod5).
const MODIFIERS: &[(u32, &str, u32)] = &[
    (42, "Shift", 1 << 0),
    (54, "Shift", 1 << 0),
    (58, "Lock", 1 << 1),
    (29, "Control", 1 << 2),
    (97, "Control", 1 << 2),
    (56, "Mod1", 1 << 3),
    (125, "Mod4", 1 << 6),
];

/// The evdev code of Caps Lock, which locks its modifier instead of holding it.
const CAPS_LOCK: u32 = 58;

/// The keysym a key produces, or `None` for keys with no Linux equivalent.
fn key_keysym(key: Key) -> Option<u32> {
    Some(match key {
        Key::Unicode(c) => char_keysym(c),
        Key::Alt | Key::Option => 0xffe9,
        Key::Backspace => 0xff08,
        Key::CapsLock => 0xffe5,
        Key::Control | Key::LControl => 0xffe3,
        Key::RControl => 0xffe4,
        Key::Delete => 0xffff,
        Key::DownArrow => 0xff54,
        Key::End => 0xff57,
        Key::Return => 0xff0d,
        Key::Escape => 0xff1b,
        Key::F1 => 0xffbe,
        Key::F2 => 0xffbf,
        Key::F3 => 0xffc0,
        Key::F4 => 0xffc1,
        Key::F5 => 0xffc2,
        Key::F6 => 0xffc3,
        Key::F7 => 0xffc4,
        Key::F8 => 0xffc5,
        Key::F9 => 0xffc6,
        Key::F10 => 0xffc7,
        Key::F11 => 0xffc8,
        Key::F12 => 0xffc9,
        Key::F13 => 0xffca,
        Key::F14 => 0xffcb,
        Key::F15 => 0xffcc,
        Key::F16 => 0xffcd,
        Key::F17 => 0xffce,
        Key::F18 => 0xffcf,
        Key::F19 => 0xffd0,
        Key::F20 => 0xffd1,
        Key::Help => 0xff6a,
        Key::Home => 0xff50,
        Key::LeftArrow => 0xff51,
        Key::Meta => 0xffeb,
        Key::PageDown => 0xff56,
        Key::PageUp => 0xff55,
        Key::RightArrow => 0xff53,
        Key::Shift | Key::LShift => 0xffe1,
        Key::RShift => 0xffe2,
        Key::Space => 0x20,
        Key::Tab => 0xff09,
        Key::UpArrow => 0xff52,
        Key::Numpad0 => 0xffb0,
        Key::Numpad1 => 0xffb1,
        Key::Numpad2 => 0xffb2,
        Key::Numpad3 => 0xffb3,
        Key::Numpad4 => 0xffb4,
        Key::Numpad5 => 0xffb5,
        Key::Numpad6 => 0xffb6,
        Key::Numpad7 => 0xffb7,
        Key::Numpad8 => 0xffb8,
        Key::Numpad9 => 0xffb9,
        Key::Add => 0xffab,
        Key::Subtract => 0xffad,
        Key::Multiply => 0xffaa,
        Key::Divide => 0xffaf,
        Key::Decimal => 0xffae,
        Key::VolumeUp => 0x1008ff13,
        Key::VolumeDown => 0x1008ff11,
        Key::VolumeMute => 0x1008ff12,
        Key::MediaPlayPause => 0x1008ff14,
        Key::MediaNextTrack => 0x1008ff17,
        Key::MediaPrevTrack => 0x1008ff16,
        _ => return None,
    })
}

/// The keysym that types a character: Latin-1 characters are their own
/// keysym, everything else uses the Unicode keysym range.
fn char_keysym(c: char) -> u32 {
    match c {
        '\n' | '\r' => 0xff0d,
        '\t' => 0xff09,
        '\u{8}' => 0xff08,
        ' '..='~' | '\u{a0}'..='\u{ff}' => c as u32,
        _ => 0x0100_0000 + c as u32,
    }
}

/// The name a keymap uses for a keysym. Keysyms outside the static keys
/// only come from characters, which have a `U<hex>` name.
fn keysym_name(keysym: u32) -> String {
    if let Some((_, _, name, _)) = STATIC_KEYS.iter().find(|key| key.1 == keysym) {
        return (*name).to_string();
    }
    let codepoint = keysym.checked_sub(0x0100_0000).unwrap_or(keysym);
    format!("U{:04X}", codepoint)
}

/// Which evdev code produces which keysym, handing out spare codes to
/// characters the static keys don't cover.
struct Keymap {
    codes: HashMap<u32, u32>,
    /// Codes handed out on demand, with their keysym, oldest first.
    dynamic: Vec<(u32, u32)>,
    /// Unused codes, the next one to hand out last.
    spare: Vec<u32>,
    changed: bool,
}

impl Keymap {
    fn new() -> Self {
        let codes = STATIC_KEYS
            .iter()
            .map(|&(code, keysym, _, _)| (keysym, code))
            .collect();
        let is_static = |code: &u32| STATIC_KEYS.iter().any(|key| key.0 == *code);
        // Codes past the static keys go first, so characters land on codes
        // no keyboard sends before they borrow the gaps in between.
        let high = STATIC_KEYS.iter().map(|key| key.0).max().unwrap_or(0) + 1;
        let mut spare: Vec<u32> = (high..=MAX_CODE)
            .chain((1..high).filter(|code| !is_static(code)))
            .collect();
        spare.reverse();
        Keymap {
            codes,
            dynamic: Vec::new(),
            spare,
            changed: true,
        }
    }

    /// The code for a keysym, giving it a spare one if it has none.
    fn assign(&mut self, keysym: u32) -> Option<u32> {
        if let Some(&code) = self.codes.get(&keysym) {
            return Some(code);
        }
        let code = self.spare.pop()?;
        self.codes.insert(keysym, code);
        self.dynamic.push((code, keysym));
        self.changed = true;
        Some(code)
    }

    /// Takes back every handed-out code that is not in `held`.
    fn recycle(&mut self, held: &[u32]) {
        let before = self.dynamic.len();
        let codes = &mut self.codes;
        let spare = &mut self.spare;
        self.dynamic.retain(|&(code, keysym)| {
            if held.contains(&code) {
                return true;
            }
            codes.remove(&keysym);
            spare.push(code);
            false
        });
        if self.dynamic.len() != before {
            self.changed = true;
        }
    }

    /// The keymap in XKB text format. Types and compatibility come from the
    /// standard "complete" set every XKB installation ships.
    fn text(&self) -> String {
        let mut keys: Vec<(u32, String, Option<&str>)> = STATIC_KEYS
            .iter()
            .map(|&(code, _, name, shifted)| (code, name.to_string(), shifted))
            .collect();
        keys.extend(
            self.dynamic
                .iter()
                .map(|&(code, keysym)| (code, keysym_name(keysym), None)),
        );
        keys.sort_by_key(|key| key.0);

        let mut text = String::from("xkb_keymap {\nxkb_keycodes \"neyuki\" {\n");
        text.push_str("minimum = 8;\nmaximum = 255;\n");
        for (code, _, _) in &keys {
            text.push_str(&format!("<K{}> = {};\n", code, code + 8));
        }
        text.push_str("};\nxkb_types \"neyuki\" { include \"complete\" };\n");
        text.push_str("xkb_compatibility \"neyuki\" { include \"complete\" };\n");
        text.push_str("xkb_symbols \"neyuki\" {\n");
        for (code, name, shifted) in &keys {
            match shifted {
                Some(shifted) if name.len() == 1 && name.as_bytes()[0].is_ascii_lowercase() => text
                    .push_str(&format!(
                        "key <K{}> {{ type = \"ALPHABETIC\", [ {}, {} ] }};\n",
                        code, name, shifted
                    )),
                Some(shifted) => text.push_str(&format!(
                    "key <K{}> {{ type = \"TWO_LEVEL\", [ {}, {} ] }};\n",
                    code, name, shifted
                )),
                None => text.push_str(&format!("key <K{}> {{ [ {} ] }};\n", code, name)),
            }
        }
        for &(code, modifier, _) in MODIFIERS {
            text.push_str(&format!("modifier_map {} {{ <K{}> }};\n", modifier, code));
        }
        text.push_str("};\n};\n");
        text
    }
}

/// The modifier mask a held key sets.
fn modifier_mask(code: u32) -> u32 {
    MODIFIERS
        .iter()
        .find(|modifier| modifier.0 == code)
        .map_or(0, |modifier| modifier.2)
}

fn button_code(button: Button) -> Result<u32, String> {
    // From linux/input-event-codes.h; back and forward are the side buttons
    // mice report, which is what applications listen for.
    match button {
        Button::Left => Ok(0x110),
        Button::Right => Ok(0x111),
        Button::Middle => Ok(0x112),
        Button::Back => Ok(0x113),
        Button::Forward => Ok(0x114),
        _ => Err("this mouse button is not available on Wayland".to_string()),
    }
}

struct Output {
    output: wl_output::WlOutput,
    width: i32,
    height: i32,
    /// Whether the output is turned a quarter, so its mode is sideways.
    rotated: bool,
}

#[derive(Default)]
struct State {
    outputs: Vec<Output>,
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for State {
    fn event(
        _: &mut Self,
        _: &wl_registry::WlRegistry,
        _: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<wl_output::WlOutput, ()> for State {
    fn event(
        state: &mut Self,
        proxy: &wl_output::WlOutput,
        event: wl_output::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let Some(output) = state.outputs.iter_mut().find(|o| &o.output == proxy) else {
            return;
        };
        match event {
            wl_output::Event::Geometry { transform, .. } => {
                use wl_output::Transform;
                output.rotated = matches!(
                    transform,
                    WEnum::Value(
                        Transform::_90
                            | Transform::_270
                            | Transform::Flipped90
                            | Transform::Flipped270
                    )
                );
            }
            wl_output::Event::Mode {
                flags: WEnum::Value(flags),
                width,
                height,
                ..
            } if flags.contains(wl_output::Mode::Current) => {
                output.width = width;
                output.height = height;
            }
            _ => {}
        }
    }
}

delegate_noop!(State: ignore wl_seat::WlSeat);
delegate_noop!(State: ignore ZwpVirtualKeyboardManagerV1);
delegate_noop!(State: ignore ZwpVirtualKeyboardV1);
delegate_noop!(State: ignore ZwlrVirtualPointerManagerV1);
delegate_noop!(State: ignore ZwlrVirtualPointerV1);

/// A virtual keyboard and pointer on the compositor's seat.
pub(super) struct Input {
    connection: Connection,
    queue: EventQueue<State>,
    state: State,
    keyboard: ZwpVirtualKeyboardV1,
    pointer: ZwlrVirtualPointerV1,
    keymap: Keymap,
    held_keys: Vec<u32>,
    held_buttons: Vec<u32>,
    modifiers: (u32, u32),
    start: Instant,
}

fn lost(_: impl std::fmt::Display) -> String {
    "lost the connection to the Wayland compositor".to_string()
}

impl Input {
    /// Connects to `$WAYLAND_DISPLAY` and creates the virtual devices, or
    /// says why the compositor can't take input this way.
    pub(super) fn connect() -> Result<Input, String> {
        let connection = Connection::connect_to_env()
            .map_err(|err| format!("cannot connect to the Wayland compositor: {}", err))?;
        let (globals, queue) = registry_queue_init::<State>(&connection)
            .map_err(|err| format!("cannot connect to the Wayland compositor: {}", err))?;
        let qh = queue.handle();
        let seat: wl_seat::WlSeat = globals
            .bind(&qh, 1..=1, ())
            .map_err(|_| "the Wayland compositor has no seat".to_string())?;
        let keyboards: ZwpVirtualKeyboardManagerV1 = globals
            .bind(&qh, 1..=1, ())
            .map_err(|_| "the Wayland compositor does not support virtual keyboards".to_string())?;
        let pointers: ZwlrVirtualPointerManagerV1 = globals
            .bind(&qh, 1..=2, ())
            .map_err(|_| "the Wayland compositor does not support virtual pointers".to_string())?;

        let mut state = State::default();
        for global in globals.contents().clone_list() {
            if global.interface == wl_output::WlOutput::interface().name {
                let output = globals.registry().bind(global.name, 1, &qh, ());
                state.outputs.push(Output {
                    output,
                    width: 0,
                    height: 0,
                    rotated: false,
                });
            }
        }
        // Tie the pointer to the main display so absolute moves and
        // `main_display` agree on what the coordinates mean.
        let pointer =
            match state.outputs.first() {
                Some(main) if pointers.version() >= 2 => pointers
                    .create_virtual_pointer_with_output(Some(&seat), Some(&main.output), &qh, ()),
                _ => pointers.create_virtual_pointer(Some(&seat), &qh, ()),
            };
        let keyboard = keyboards.create_virtual_keyboard(&seat, &qh, ());

        let mut input = Input {
            connection,
            queue,
            state,
            keyboard,
            pointer,
            keymap: Keymap::new(),
            held_keys: Vec::new(),
            held_buttons: Vec::new(),
            modifiers: (0, 0),
            start: Instant::now(),
        };
        input.upload_keymap()?;
        // A compositor that refuses virtual devices says so with a protocol
        // error, which arrives on this roundtrip along with the output sizes.
        input
            .queue
            .roundtrip(&mut input.state)
            .map_err(|err| format!("the Wayland compositor refused virtual input: {}", err))?;
        Ok(input)
    }

    /// Milliseconds since the devices were created; wraps like evdev time.
    fn time(&self) -> u32 {
        self.start.elapsed().as_millis() as u32
    }

    fn flush(&mut self) -> Result<(), String> {
        self.queue.dispatch_pending(&mut self.state).map_err(lost)?;
        self.connection.flush().map_err(lost)
    }

    /// Sends the keymap to the compositor if a code was handed out or taken
    /// back since it was last sent.
    fn upload_keymap(&mut self) -> Result<(), String> {
        if !self.keymap.changed {
            return Ok(());
        }
        let mut text = self.keymap.text().into_bytes();
        text.push(0);
        let size = u32::try_from(text.len()).map_err(|_| "the keymap is too large".to_string())?;
        let mut file =
            tempfile::tempfile().map_err(|err| format!("cannot store the keymap: {}", err))?;
        file.write_all(&text)
            .and_then(|_| file.rewind())
            .map_err(|err| format!("cannot store the keymap: {}", err))?;
        self.keyboard.keymap(XKB_V1, file.as_fd(), size);
        self.keymap.changed = false;
        Ok(())
    }

    /// The code for a keysym, taking back codes from typed characters when
    /// every spare one is in use.
    fn code_for(&mut self, keysym: u32) -> Result<u32, String> {
        let code = match self.keymap.assign(keysym) {
            Some(code) => code,
            None => {
                self.keymap.recycle(&self.held_keys);
                self.keymap
                    .assign(keysym)
                    .ok_or_else(|| "too many keys are held down".to_string())?
            }
        };
        self.upload_keymap()?;
        Ok(code)
    }

    fn send_key(&mut self, code: u32, pressed: bool) {
        let time = self.time();
        self.keyboard.key(time, code, u32::from(pressed));
        if pressed {
            self.held_keys.push(code);
            if code == CAPS_LOCK {
                self.modifiers.1 ^= modifier_mask(CAPS_LOCK);
            }
        } else {
            self.held_keys.retain(|&held| held != code);
        }
        let depressed = self
            .held_keys
            .iter()
            .filter(|&&held| held != CAPS_LOCK)
            .fold(0, |mask, &held| mask | modifier_mask(held));
        if depressed != self.modifiers.0 || modifier_mask(code) != 0 {
            self.modifiers.0 = depressed;
            self.keyboard.modifiers(depressed, 0, self.modifiers.1, 0);
        }
    }

    pub(super) fn key(&mut self, key: Key, direction: Direction) -> Result<(), String> {
        let keysym =
            key_keysym(key).ok_or_else(|| "this key is not available on Wayland".to_string())?;
        let code = self.code_for(keysym)?;
        if matches!(direction, Direction::Press | Direction::Click) {
            self.send_key(code, true);
        }
        if matches!(direction, Direction::Release | Direction::Click) {
            self.send_key(code, false);
        }
        self.flush()
    }

    pub(super) fn text(&mut self, text: &str) -> Result<(), String> {
        let keysyms: Vec<u32> = text.chars().map(char_keysym).collect();
        let mut start = 0;
        while start < keysyms.len() {
            // Give codes to as many characters as fit, send the keymap once,
            // then type them; a longer run of new characters takes back the
            // codes and goes again.
            let mut end = start;
            while end < keysyms.len() && self.keymap.assign(keysyms[end]).is_some() {
                end += 1;
            }
            if end == start {
                self.keymap.recycle(&self.held_keys);
                if self.keymap.assign(keysyms[start]).is_none() {
                    return Err("too many keys are held down".to_string());
                }
                continue;
            }
            self.upload_keymap()?;
            for &keysym in &keysyms[start..end] {
                let code = self.keymap.codes[&keysym];
                self.send_key(code, true);
                self.send_key(code, false);
            }
            self.flush()?;
            start = end;
        }
        Ok(())
    }

    pub(super) fn button(&mut self, button: Button, direction: Direction) -> Result<(), String> {
        let code = button_code(button)?;
        if matches!(direction, Direction::Press | Direction::Click) {
            self.send_button(code, true);
        }
        if matches!(direction, Direction::Release | Direction::Click) {
            self.send_button(code, false);
        }
        self.flush()
    }

    fn send_button(&mut self, code: u32, pressed: bool) {
        let state = if pressed {
            self.held_buttons.push(code);
            wl_pointer::ButtonState::Pressed
        } else {
            self.held_buttons.retain(|&held| held != code);
            wl_pointer::ButtonState::Released
        };
        self.pointer.button(self.time(), code, state);
        self.pointer.frame();
    }

    pub(super) fn move_mouse(&mut self, x: i32, y: i32, mode: Coordinate) -> Result<(), String> {
        let time = self.time();
        match mode {
            Coordinate::Rel => self.pointer.motion(time, f64::from(x), f64::from(y)),
            Coordinate::Abs => {
                let (width, height) = self.cached_display()?;
                let x = x.clamp(0, width.saturating_sub(1)) as u32;
                let y = y.clamp(0, height.saturating_sub(1)) as u32;
                self.pointer
                    .motion_absolute(time, x, y, width as u32, height as u32);
            }
        }
        self.pointer.frame();
        self.flush()
    }

    pub(super) fn scroll(&mut self, amount: i32, axis: Axis) -> Result<(), String> {
        let axis = match axis {
            Axis::Horizontal => wl_pointer::Axis::HorizontalScroll,
            Axis::Vertical => wl_pointer::Axis::VerticalScroll,
        };
        let time = self.time();
        self.pointer.axis_source(wl_pointer::AxisSource::Wheel);
        self.pointer
            .axis_discrete(time, axis, f64::from(amount) * SCROLL_STEP, amount);
        self.pointer.frame();
        self.flush()
    }

    /// The main display's size as last reported, sideways when it is turned.
    fn cached_display(&self) -> Result<(i32, i32), String> {
        match self.state.outputs.first() {
            Some(main) if main.width > 0 && main.height > 0 => Ok(if main.rotated {
                (main.height, main.width)
            } else {
                (main.width, main.height)
            }),
            _ => Err("no display is connected".to_string()),
        }
    }

    pub(super) fn main_display(&mut self) -> Result<(i32, i32), String> {
        self.queue.roundtrip(&mut self.state).map_err(lost)?;
        self.cached_display()
    }
}

impl Drop for Input {
    /// Lets go of everything still held, so a script that ends mid-drag or
    /// mid-shortcut doesn't leave the seat stuck.
    fn drop(&mut self) {
        for code in std::mem::take(&mut self.held_keys) {
            let time = self.time();
            self.keyboard.key(time, code, 0);
        }
        self.keyboard.modifiers(0, 0, self.modifiers.1, 0);
        for code in std::mem::take(&mut self.held_buttons) {
            self.pointer
                .button(self.time(), code, wl_pointer::ButtonState::Released);
            self.pointer.frame();
        }
        self.keyboard.destroy();
        self.pointer.destroy();
        let _ = self.connection.flush();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn static_keys_are_unique_and_in_range() {
        for (index, key) in STATIC_KEYS.iter().enumerate() {
            assert!(
                (1..=MAX_CODE).contains(&key.0),
                "code {} out of range",
                key.0
            );
            for other in &STATIC_KEYS[index + 1..] {
                assert_ne!(key.0, other.0, "code {} used twice", key.0);
                assert_ne!(key.1, other.1, "keysym {:#x} used twice", key.1);
            }
        }
        for &(code, _, _) in MODIFIERS {
            assert!(STATIC_KEYS.iter().any(|key| key.0 == code));
        }
    }

    #[test]
    fn keys_map_to_keysyms() {
        assert_eq!(key_keysym(Key::Unicode('a')), Some(0x61));
        assert_eq!(key_keysym(Key::Return), Some(0xff0d));
        assert_eq!(key_keysym(Key::Meta), Some(0xffeb));
        assert_eq!(char_keysym('\n'), 0xff0d);
        assert_eq!(char_keysym('é'), 0xe9);
        assert_eq!(char_keysym('☺'), 0x0100_263a);
        assert_eq!(keysym_name(0x0100_263a), "U263A");
        assert_eq!(keysym_name(0x41), "U0041");
        assert_eq!(keysym_name(0x61), "a");
    }

    #[test]
    fn common_keys_keep_their_codes() {
        let mut keymap = Keymap::new();
        assert_eq!(keymap.assign(0x61), Some(30));
        assert_eq!(keymap.assign(0xff0d), Some(28));
        assert!(keymap.dynamic.is_empty());
    }

    #[test]
    fn characters_get_spare_codes_and_give_them_back() {
        let mut keymap = Keymap::new();
        keymap.changed = false;
        let smile = keymap.assign(0x0100_263a).unwrap();
        assert_eq!(smile, 191);
        assert!(keymap.changed);
        assert_eq!(keymap.assign(0x0100_263a), Some(smile));

        let mut given = vec![smile];
        while let Some(code) = keymap.assign(0x0100_4e00 + given.len() as u32) {
            assert!(!STATIC_KEYS.iter().any(|key| key.0 == code));
            given.push(code);
        }
        assert_eq!(given.len(), MAX_CODE as usize - STATIC_KEYS.len());

        keymap.recycle(&[smile]);
        assert_eq!(keymap.dynamic, vec![(smile, 0x0100_263a)]);
        assert!(keymap.assign(0x0100_263b).is_some());
    }

    #[test]
    fn keymap_text_names_every_key() {
        let mut keymap = Keymap::new();
        keymap.assign(0x0100_263a);
        let text = keymap.text();
        assert!(text.contains("<K30> = 38;"));
        assert!(text.contains("key <K30> { type = \"ALPHABETIC\", [ a, A ] };"));
        assert!(text.contains("key <K2> { type = \"TWO_LEVEL\", [ 1, exclam ] };"));
        assert!(text.contains("key <K191> { [ U263A ] };"));
        assert!(text.contains("modifier_map Mod4 { <K125> };"));
        assert_eq!(text.matches('{').count(), text.matches('}').count());
    }

    #[test]
    fn modifier_keys_have_masks() {
        assert_eq!(modifier_mask(42), 1);
        assert_eq!(modifier_mask(29), 4);
        assert_eq!(modifier_mask(30), 0);
    }
}
