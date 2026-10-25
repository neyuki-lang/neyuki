//! Native windowing primitives backing `@neyuki/ui`.
//!
//! Windows come from `winit`, frames are drawn on the CPU with `tiny-skia`
//! and `fontdue` and blitted with `softbuffer`, so no GPU, GL or C toolchain
//! is involved and one code path serves X11, Wayland, macOS and Windows.
//!
//! Like the HTTP server, the design is pull-based because natives cannot call
//! back into Neyuki functions: `__ui_open` parks a window here under an id,
//! the drawing natives paint into that window's back buffer, `__ui_present`
//! shows it and `__ui_poll` pumps the OS event loop and hands back whatever
//! happened as a list of event tables. `lib/ui.nyk` builds the `Window`,
//! `Image` and `Font` objects and the `run` loop on top of these. Image
//! decoding, resampling and compositing live in `ui/images.rs`.
//!
//! Every coordinate crossing the boundary is in logical pixels; the back
//! buffer is kept at the window's physical size and drawing is scaled by the
//! DPI factor, so scripts never see it. Colors arrive packed as `0xRRGGBBAA`.
//!
//! winit only supports the event loop on the thread that created it (and on
//! macOS only the main thread), which is where the interpreter runs; all
//! state therefore lives in thread-locals.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::io::Read;
use std::num::NonZeroU32;
use std::rc::Rc;
use std::time::Duration;

use fontdue::layout::{CoordinateSystem, GlyphRasterConfig, Layout, LayoutSettings, TextStyle};
use tiny_skia::{
    Color, FillRule, GradientStop, LineCap, LineJoin, LinearGradient, Mask, Paint, Path,
    PathBuilder, Pixmap, PixmapPaint, Point, PremultipliedColorU8, RadialGradient, Rect, Shader,
    SpreadMode, Stroke, Transform,
};
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::keyboard::{Key, ModifiersState, PhysicalKey};
use winit::platform::pump_events::EventLoopExtPumpEvents;
use winit::window::{Window, WindowId};

use crate::vm::libs::args::array as new_table;
use crate::vm::machine::VM;
use crate::vm::value::{NativeDef, StrRef, Value, VmTable};

mod effects;
mod images;

use effects::coverage_in;
use images::{Decoded, Fit, Limits};

pub(crate) const NATIVES: &[NativeDef] = &[
    NativeDef {
        name: "__ui_available",
        func: builtin_available,
    },
    NativeDef {
        name: "__ui_open",
        func: builtin_open,
    },
    NativeDef {
        name: "__ui_close",
        func: builtin_close,
    },
    NativeDef {
        name: "__ui_set_title",
        func: builtin_set_title,
    },
    NativeDef {
        name: "__ui_set_size",
        func: builtin_set_size,
    },
    NativeDef {
        name: "__ui_size",
        func: builtin_size,
    },
    NativeDef {
        name: "__ui_poll",
        func: builtin_poll,
    },
    NativeDef {
        name: "__ui_clear",
        func: builtin_clear,
    },
    NativeDef {
        name: "__ui_fill_rect",
        func: builtin_fill_rect,
    },
    NativeDef {
        name: "__ui_stroke_rect",
        func: builtin_stroke_rect,
    },
    NativeDef {
        name: "__ui_line",
        func: builtin_line,
    },
    NativeDef {
        name: "__ui_circle",
        func: builtin_circle,
    },
    NativeDef {
        name: "__ui_polygon",
        func: builtin_polygon,
    },
    NativeDef {
        name: "__ui_text",
        func: builtin_text,
    },
    NativeDef {
        name: "__ui_measure_text",
        func: builtin_measure_text,
    },
    NativeDef {
        name: "__ui_load_image",
        func: builtin_load_image,
    },
    NativeDef {
        name: "__ui_decode_image",
        func: builtin_decode_image,
    },
    NativeDef {
        name: "__ui_resize_image",
        func: builtin_resize_image,
    },
    NativeDef {
        name: "__ui_free_image",
        func: builtin_free_image,
    },
    NativeDef {
        name: "__ui_draw_image",
        func: builtin_draw_image,
    },
    NativeDef {
        name: "__ui_load_font",
        func: builtin_load_font,
    },
    NativeDef {
        name: "__ui_free_font",
        func: builtin_free_font,
    },
    NativeDef {
        name: "__ui_canvas",
        func: builtin_canvas,
    },
    NativeDef {
        name: "__ui_free_canvas",
        func: builtin_free_canvas,
    },
    NativeDef {
        name: "__ui_push_clip",
        func: builtin_push_clip,
    },
    NativeDef {
        name: "__ui_push_clip_path",
        func: builtin_push_clip_path,
    },
    NativeDef {
        name: "__ui_pop_clip",
        func: builtin_pop_clip,
    },
    NativeDef {
        name: "__ui_push_opacity",
        func: builtin_push_opacity,
    },
    NativeDef {
        name: "__ui_pop_opacity",
        func: builtin_pop_opacity,
    },
    NativeDef {
        name: "__ui_fill_path",
        func: builtin_fill_path,
    },
    NativeDef {
        name: "__ui_stroke_path",
        func: builtin_stroke_path,
    },
    NativeDef {
        name: "__ui_blur_region",
        func: builtin_blur_region,
    },
    NativeDef {
        name: "__ui_blur_image",
        func: builtin_blur_image,
    },
    NativeDef {
        name: "__ui_get_pixel",
        func: builtin_get_pixel,
    },
    NativeDef {
        name: "__ui_present",
        func: builtin_present,
    },
];

/// The font used when a script does not load its own: Noto Sans, under the
/// SIL Open Font License (see `lib/fonts/NotoSans-OFL.txt`).
const DEFAULT_FONT: &[u8] = include_bytes!("../../../lib/fonts/NotoSans-Regular.ttf");
const DEFAULT_FONT_ID: u64 = 0;

/// Glyph bitmaps are cached per font; past this many the cache starts over
/// rather than growing with every size a script ever used.
const GLYPH_CACHE_LIMIT: usize = 4096;

/// How many times `__ui_open` pumps the loop waiting for the window to be
/// created. macOS needs one pump to launch the application and another to
/// deliver `resumed`; everything else creates it on the first.
const OPEN_ATTEMPTS: usize = 8;

/// An open window: the OS handle, the surface frames are blitted to, and the
/// back buffer at physical size that the drawing natives paint into.
struct Win {
    window: Rc<Window>,
    surface: softbuffer::Surface<Rc<Window>, Rc<Window>>,
    pixmap: Pixmap,
    scale: f64,
    modifiers: ModifiersState,
    /// Last known cursor position in logical pixels, for button events,
    /// which winit reports without one.
    cursor: (f64, f64),
    /// Clips and open opacity groups of the frame being drawn.
    state: DrawState,
}

/// What clipping and group opacity keep per drawing target. Clips are a
/// stack of masks, each already intersected with the one below it, so the
/// top one is all a draw call needs.
#[derive(Default)]
struct DrawState {
    clips: Vec<Mask>,
    groups: Vec<Group>,
}

/// An open `pushOpacity`: the pixels drawn before it, which the layer now
/// being drawn into is composited onto at `popOpacity`.
struct Group {
    saved: Pixmap,
    opacity: f32,
    /// How many clips were active when it opened; it closes back to this.
    clips: usize,
}

/// An off-screen surface from `ui.canvas`, drawn into like a window.
struct Canvas {
    pixmap: Pixmap,
    scale: f64,
    state: DrawState,
}

/// What the drawing natives paint into, whether window or canvas.
struct Target<'a> {
    pixmap: &'a mut Pixmap,
    scale: f64,
    state: &'a mut DrawState,
}

struct FontEntry {
    font: fontdue::Font,
    glyphs: HashMap<GlyphRasterConfig, Rc<(fontdue::Metrics, Vec<u8>)>>,
}

/// What `__ui_open` asks the event loop to create. Windows can only be made
/// from inside a winit callback, so the request is parked here and picked up
/// by `App::resumed` / `App::about_to_wait` during the next pump.
struct OpenRequest {
    title: String,
    width: f64,
    height: f64,
    resizable: bool,
    visible: bool,
}

thread_local! {
    static EVENT_LOOP: RefCell<Option<Result<EventLoop<()>, String>>> = const { RefCell::new(None) };
    static WINDOWS: RefCell<HashMap<u64, Win>> = RefCell::new(HashMap::new());
    static WINDOW_IDS: RefCell<HashMap<WindowId, u64>> = RefCell::new(HashMap::new());
    static IMAGES: RefCell<HashMap<u64, ImageEntry>> = RefCell::new(HashMap::new());
    static CANVASES: RefCell<HashMap<u64, Canvas>> = RefCell::new(HashMap::new());
    static FONTS: RefCell<HashMap<u64, FontEntry>> = RefCell::new(HashMap::new());
    static EVENTS: RefCell<Vec<Value>> = const { RefCell::new(Vec::new()) };
    static PENDING_OPEN: RefCell<Option<OpenRequest>> = const { RefCell::new(None) };
    static OPENED: RefCell<Option<Result<(u64, f64), String>>> = const { RefCell::new(None) };
    static NEXT_ID: Cell<u64> = const { Cell::new(1) };
}

fn next_id() -> u64 {
    NEXT_ID.with(|id| {
        let value = id.get();
        id.set(value + 1);
        value
    })
}

// ARGUMENTS ----------------------------------------------------------------

fn string_arg(args: &[Value], index: usize, name: &str) -> Result<String, String> {
    match args.get(index) {
        Some(Value::String(value)) => Ok(value.to_string()),
        Some(value) => Err(format!(
            "{} must be a string, got {}",
            name,
            value.type_name()
        )),
        None => Err(format!("{} must be provided", name)),
    }
}

fn bool_arg(args: &[Value], index: usize, name: &str, default: bool) -> Result<bool, String> {
    match args.get(index) {
        Some(Value::Bool(value)) => Ok(*value),
        None | Some(Value::Nil) => Ok(default),
        Some(value) => Err(format!(
            "{} must be a boolean, got {}",
            name,
            value.type_name()
        )),
    }
}

fn float_arg(args: &[Value], index: usize, name: &str) -> Result<f64, String> {
    let value = args.get(index).unwrap_or(&Value::Nil);
    if matches!(value, Value::Nil) {
        return Err(format!("{} must be provided", name));
    }
    let number =
        crate::vm::libs::args::number(value).map_err(|_| format!("{} must be a number", name))?;
    if !number.is_finite() {
        return Err(format!("{} must be a finite number", name));
    }
    Ok(number)
}

fn optional_float_arg(
    args: &[Value],
    index: usize,
    name: &str,
    default: f64,
) -> Result<f64, String> {
    match args.get(index) {
        None | Some(Value::Nil) => Ok(default),
        _ => float_arg(args, index, name),
    }
}

fn integer_arg(args: &[Value], index: usize, name: &str) -> Result<u64, String> {
    let number = float_arg(args, index, name)?;
    if number < 0.0 || number.fract() != 0.0 {
        return Err(format!("{} must be a non-negative integer", name));
    }
    Ok(number as u64)
}

/// A color packed as `0xRRGGBBAA`, the way `lib/ui.nyk` sends it.
fn color_arg(args: &[Value], index: usize, name: &str) -> Result<(u8, u8, u8, u8), String> {
    let packed = integer_arg(args, index, name)?;
    if packed > u32::MAX as u64 {
        return Err(format!("{} is out of range", name));
    }
    Ok(unpack_color(packed as u32))
}

fn unpack_color(packed: u32) -> (u8, u8, u8, u8) {
    (
        (packed >> 24) as u8,
        (packed >> 16) as u8,
        (packed >> 8) as u8,
        packed as u8,
    )
}

/// A timeout in seconds; nil, absent or zero means "do not wait".
fn timeout_arg(args: &[Value], index: usize, name: &str) -> Result<Duration, String> {
    let seconds = optional_float_arg(args, index, name, 0.0)?;
    if seconds < 0.0 {
        return Err(format!("{} must be a non-negative number of seconds", name));
    }
    Ok(Duration::from_secs_f64(seconds))
}

/// A flat `{x1, y1, x2, y2, ...}` array of coordinates.
fn points_arg(args: &[Value], index: usize, name: &str) -> Result<Vec<(f32, f32)>, String> {
    let numbers = match args.get(index) {
        Some(Value::Table(table)) => table
            .borrow()
            .array
            .iter()
            .map(|value| {
                crate::vm::libs::args::number(value)
                    .ok()
                    .filter(|number| number.is_finite())
                    .ok_or_else(|| format!("{} must hold finite numbers", name))
            })
            .collect::<Result<Vec<f64>, String>>()?,
        Some(value) => {
            return Err(format!(
                "{} must be a table, got {}",
                name,
                value.type_name()
            ));
        }
        None => return Err(format!("{} must be provided", name)),
    };
    if numbers.len() % 2 != 0 {
        return Err(format!("{} must hold x, y pairs", name));
    }
    Ok(numbers
        .chunks(2)
        .map(|pair| (pair[0] as f32, pair[1] as f32))
        .collect())
}

fn record(fields: Vec<(&str, Value)>) -> Value {
    let mut table = VmTable::new();
    for (name, value) in fields {
        table.fields.insert(StrRef::from(name), value);
    }
    Value::Table(Rc::new(RefCell::new(table)))
}

fn integer(value: u64) -> Value {
    Value::from_u64(value)
}

fn text(value: &str) -> Value {
    Value::str(value)
}

// EVENT LOOP ---------------------------------------------------------------

/// Creates the event loop on first use and caches the outcome, so a machine
/// without a display reports the same error from every native.
fn with_event_loop<T>(body: impl FnOnce(&mut EventLoop<()>) -> T) -> Result<T, String> {
    EVENT_LOOP.with(|cell| {
        let mut slot = cell.borrow_mut();
        let entry = slot.get_or_insert_with(create_event_loop);
        match entry {
            Ok(event_loop) => Ok(body(event_loop)),
            Err(err) => Err(err.clone()),
        }
    })
}

fn create_event_loop() -> Result<EventLoop<()>, String> {
    let mut builder = EventLoop::<()>::with_user_event();
    // The interpreter runs on the main thread, but `cargo test` drives the
    // `.nyk` suite from worker threads; the platforms that allow it get told
    // that is fine. macOS does not, and panics instead of returning an error.
    #[cfg(target_os = "linux")]
    {
        winit::platform::x11::EventLoopBuilderExtX11::with_any_thread(&mut builder, true);
        winit::platform::wayland::EventLoopBuilderExtWayland::with_any_thread(&mut builder, true);
    }
    #[cfg(target_os = "windows")]
    {
        winit::platform::windows::EventLoopBuilderExtWindows::with_any_thread(&mut builder, true);
    }
    let built = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| builder.build()));
    match built {
        Ok(Ok(event_loop)) => Ok(event_loop),
        Ok(Err(err)) => Err(format!(
            "no display available: {}",
            tidy_os_error(&err.to_string())
        )),
        Err(payload) => {
            let message = payload
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| payload.downcast_ref::<&str>().map(|s| s.to_string()))
                .unwrap_or_else(|| "unknown error".to_string());
            Err(format!("cannot start the UI event loop: {}", message))
        }
    }
}

/// winit prefixes OS errors with the source location they were raised at
/// ("os error at .../mod.rs:765: ..."), which means nothing to a script.
fn tidy_os_error(message: &str) -> String {
    match message.strip_prefix("os error at ") {
        Some(rest) => match rest.split_once(": ") {
            Some((_, detail)) => detail.to_string(),
            None => rest.to_string(),
        },
        None => message.to_string(),
    }
}

/// Pumps the OS loop once, waiting up to `timeout` for the first event.
fn pump(timeout: Duration) -> Result<(), String> {
    with_event_loop(|event_loop| {
        event_loop.pump_app_events(Some(timeout), &mut App);
    })
}

struct App;

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        create_pending(event_loop);
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        create_pending(event_loop);
    }

    fn window_event(&mut self, _: &ActiveEventLoop, window_id: WindowId, event: WindowEvent) {
        let Some(id) = WINDOW_IDS.with(|ids| ids.borrow().get(&window_id).copied()) else {
            return;
        };
        WINDOWS.with(|windows| {
            let mut windows = windows.borrow_mut();
            let Some(win) = windows.get_mut(&id) else {
                return;
            };
            if let Some(event) = translate_event(id, win, event) {
                EVENTS.with(|events| events.borrow_mut().push(event));
            }
        });
    }
}

/// Fulfils a parked `__ui_open` request, if there is one.
fn create_pending(event_loop: &ActiveEventLoop) {
    let Some(request) = PENDING_OPEN.with(|pending| pending.borrow_mut().take()) else {
        return;
    };
    let result = create_window(event_loop, request);
    OPENED.with(|opened| *opened.borrow_mut() = Some(result));
}

fn create_window(event_loop: &ActiveEventLoop, request: OpenRequest) -> Result<(u64, f64), String> {
    let attributes = Window::default_attributes()
        .with_title(&request.title)
        .with_inner_size(LogicalSize::new(request.width, request.height))
        .with_resizable(request.resizable)
        .with_visible(request.visible);
    let window = Rc::new(
        event_loop
            .create_window(attributes)
            .map_err(|err| format!("cannot create window: {}", tidy_os_error(&err.to_string())))?,
    );
    let context = softbuffer::Context::new(window.clone())
        .map_err(|err| format!("cannot create window: {}", err))?;
    let surface = softbuffer::Surface::new(&context, window.clone())
        .map_err(|err| format!("cannot create window: {}", err))?;
    let scale = window.scale_factor();
    let size = window.inner_size();
    let mut pixmap = Pixmap::new(size.width.max(1), size.height.max(1))
        .ok_or_else(|| "cannot create window: back buffer too large".to_string())?;
    pixmap.fill(Color::WHITE);

    let id = next_id();
    let mut win = Win {
        window: window.clone(),
        surface,
        pixmap,
        scale,
        modifiers: ModifiersState::empty(),
        cursor: (0.0, 0.0),
        state: DrawState::default(),
    };
    // Wayland only maps a window once it has a frame, so show the blank
    // canvas straight away; a failure here just means the first real
    // `present` does it.
    let _ = blit(&mut win);
    WINDOW_IDS.with(|ids| ids.borrow_mut().insert(window.id(), id));
    WINDOWS.with(|windows| windows.borrow_mut().insert(id, win));
    Ok((id, scale))
}

/// Turns a winit event into the table `__ui_poll` returns, updating the
/// window's bookkeeping (size, DPI, modifiers, cursor) along the way.
fn translate_event(id: u64, win: &mut Win, event: WindowEvent) -> Option<Value> {
    let kind = |kind: &str, mut fields: Vec<(&str, Value)>| {
        fields.insert(0, ("kind", text(kind)));
        fields.insert(1, ("window", integer(id)));
        Some(record(fields))
    };
    let logical = |win: &Win| {
        let size = win.window.inner_size().to_logical::<f64>(win.scale);
        (size.width, size.height)
    };
    match event {
        WindowEvent::CloseRequested => kind("close", vec![]),
        WindowEvent::Resized(size) => {
            if let Some(pixmap) = Pixmap::new(size.width.max(1), size.height.max(1)) {
                let mut pixmap = pixmap;
                pixmap.fill(Color::WHITE);
                win.pixmap = pixmap;
                win.state = DrawState::default();
            }
            let (width, height) = logical(win);
            kind(
                "resize",
                vec![
                    ("width", Value::Float(width)),
                    ("height", Value::Float(height)),
                ],
            )
        }
        WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
            win.scale = scale_factor;
            None
        }
        WindowEvent::Focused(focused) => kind(if focused { "focus" } else { "blur" }, vec![]),
        WindowEvent::RedrawRequested => {
            let _ = blit(win);
            kind("redraw", vec![])
        }
        WindowEvent::ModifiersChanged(modifiers) => {
            win.modifiers = modifiers.state();
            None
        }
        WindowEvent::KeyboardInput { event, .. } => {
            let name = match &event.logical_key {
                Key::Named(named) => format!("{:?}", named).to_lowercase(),
                Key::Character(s) => s.to_lowercase(),
                Key::Unidentified(_) => "unidentified".to_string(),
                Key::Dead(_) => "dead".to_string(),
            };
            let code = match event.physical_key {
                PhysicalKey::Code(code) => format!("{:?}", code),
                PhysicalKey::Unidentified(_) => "Unidentified".to_string(),
            };
            let typed = event
                .text
                .as_ref()
                .map(|s| s.to_string())
                .filter(|s| !s.chars().all(char::is_control));
            let mods = win.modifiers;
            kind(
                match event.state {
                    ElementState::Pressed => "keydown",
                    ElementState::Released => "keyup",
                },
                vec![
                    ("key", Value::string(name)),
                    ("code", Value::string(code)),
                    ("repeat", Value::Bool(event.repeat)),
                    ("text", typed.map(Value::string).unwrap_or(Value::Nil)),
                    (
                        "mods",
                        record(vec![
                            ("shift", Value::Bool(mods.shift_key())),
                            ("ctrl", Value::Bool(mods.control_key())),
                            ("alt", Value::Bool(mods.alt_key())),
                            ("meta", Value::Bool(mods.super_key())),
                        ]),
                    ),
                ],
            )
        }
        WindowEvent::CursorMoved { position, .. } => {
            let position = position.to_logical::<f64>(win.scale);
            win.cursor = (position.x, position.y);
            kind(
                "mousemove",
                vec![
                    ("x", Value::Float(position.x)),
                    ("y", Value::Float(position.y)),
                ],
            )
        }
        WindowEvent::CursorEntered { .. } => kind("mouseenter", vec![]),
        WindowEvent::CursorLeft { .. } => kind("mouseleave", vec![]),
        WindowEvent::MouseInput { state, button, .. } => {
            let button = match button {
                MouseButton::Left => "left".to_string(),
                MouseButton::Right => "right".to_string(),
                MouseButton::Middle => "middle".to_string(),
                MouseButton::Back => "back".to_string(),
                MouseButton::Forward => "forward".to_string(),
                MouseButton::Other(n) => format!("button{}", n),
            };
            kind(
                match state {
                    ElementState::Pressed => "mousedown",
                    ElementState::Released => "mouseup",
                },
                vec![
                    ("button", Value::string(button)),
                    ("x", Value::Float(win.cursor.0)),
                    ("y", Value::Float(win.cursor.1)),
                ],
            )
        }
        WindowEvent::MouseWheel { delta, .. } => {
            let (dx, dy, unit) = match delta {
                MouseScrollDelta::LineDelta(x, y) => (x as f64, y as f64, "lines"),
                MouseScrollDelta::PixelDelta(position) => {
                    let position = position.to_logical::<f64>(win.scale);
                    (position.x, position.y, "pixels")
                }
            };
            kind(
                "scroll",
                vec![
                    ("dx", Value::Float(dx)),
                    ("dy", Value::Float(dy)),
                    ("unit", text(unit)),
                    ("x", Value::Float(win.cursor.0)),
                    ("y", Value::Float(win.cursor.1)),
                ],
            )
        }
        _ => None,
    }
}

// WINDOWS ------------------------------------------------------------------

/// Runs `body` on window `id`. Natives cannot call back into Neyuki, so
/// nothing can reach for the map while the window is borrowed.
fn with_window<T>(id: u64, body: impl FnOnce(&mut Win) -> Result<T, String>) -> Result<T, String> {
    WINDOWS.with(|windows| {
        let mut windows = windows.borrow_mut();
        let win = windows
            .get_mut(&id)
            .ok_or_else(|| "window is closed".to_string())?;
        body(win)
    })
}

/// Runs `body` on the window or canvas `id`, whichever it names.
fn with_target<T>(
    id: u64,
    body: impl FnOnce(&mut Target) -> Result<T, String>,
) -> Result<T, String> {
    if WINDOWS.with(|windows| windows.borrow().contains_key(&id)) {
        return with_window(id, |win| {
            body(&mut Target {
                pixmap: &mut win.pixmap,
                scale: win.scale,
                state: &mut win.state,
            })
        });
    }
    CANVASES.with(|canvases| {
        let mut canvases = canvases.borrow_mut();
        let canvas = canvases
            .get_mut(&id)
            .ok_or_else(|| "window is closed".to_string())?;
        body(&mut Target {
            pixmap: &mut canvas.pixmap,
            scale: canvas.scale,
            state: &mut canvas.state,
        })
    })
}

/// Copies the back buffer to the screen. softbuffer wants `0RGB` words;
/// the pixmap is premultiplied, which over an opaque window is exactly the
/// composited color.
fn blit(win: &mut Win) -> Result<(), String> {
    let (width, height) = (win.pixmap.width(), win.pixmap.height());
    let (Some(w), Some(h)) = (NonZeroU32::new(width), NonZeroU32::new(height)) else {
        return Ok(());
    };
    win.surface
        .resize(w, h)
        .map_err(|err| format!("cannot present window: {}", err))?;
    let mut buffer = win
        .surface
        .buffer_mut()
        .map_err(|err| format!("cannot present window: {}", err))?;
    pack_pixels(win.pixmap.pixels(), &mut buffer);
    buffer
        .present()
        .map_err(|err| format!("cannot present window: {}", err))
}

fn pack_pixels(pixels: &[PremultipliedColorU8], out: &mut [u32]) {
    for (pixel, word) in pixels.iter().zip(out.iter_mut()) {
        *word = (pixel.red() as u32) << 16 | (pixel.green() as u32) << 8 | pixel.blue() as u32;
    }
}

fn paint(color: (u8, u8, u8, u8)) -> Paint<'static> {
    let mut paint = Paint::default();
    paint.set_color_rgba8(color.0, color.1, color.2, color.3);
    paint.anti_alias = true;
    paint
}

fn stroke(width: f64) -> Stroke {
    Stroke {
        width: width as f32,
        ..Stroke::default()
    }
}

/// A stroke with the given line cap (`"butt"`, `"round"` or `"square"`) and
/// join (`"miter"`, `"round"` or `"bevel"`).
fn styled_stroke(width: f64, cap: &str, join: &str) -> Result<Stroke, String> {
    let line_cap = match cap {
        "butt" => LineCap::Butt,
        "round" => LineCap::Round,
        "square" => LineCap::Square,
        _ => return Err("cap must be \"butt\", \"round\" or \"square\"".to_string()),
    };
    let line_join = match join {
        "miter" => LineJoin::Miter,
        "round" => LineJoin::Round,
        "bevel" => LineJoin::Bevel,
        _ => return Err("join must be \"miter\", \"round\" or \"bevel\"".to_string()),
    };
    Ok(Stroke {
        width: width as f32,
        line_cap,
        line_join,
        ..Stroke::default()
    })
}

/// An optional table of finite numbers, such as a gradient or shadow spec.
fn numbers_arg(args: &[Value], index: usize, name: &str) -> Result<Option<Vec<f64>>, String> {
    match args.get(index) {
        None | Some(Value::Nil) => Ok(None),
        Some(Value::Table(table)) => table
            .borrow()
            .array
            .iter()
            .map(|value| {
                crate::vm::libs::args::number(value)
                    .ok()
                    .filter(|number| number.is_finite())
                    .ok_or_else(|| format!("{} must hold finite numbers", name))
            })
            .collect::<Result<Vec<f64>, String>>()
            .map(Some),
        Some(value) => Err(format!(
            "{} must be a table, got {}",
            name,
            value.type_name()
        )),
    }
}

/// A path from the flat command list `lib/ui.nyk` builds: `0 x y` moves,
/// `1 x y` draws a line, `2 cx cy x y` a quadratic curve, `3 c1x c1y c2x
/// c2y x y` a cubic curve and `4` closes the sub-path.
fn path_from_commands(commands: &[f64]) -> Result<Path, String> {
    let mut pb = PathBuilder::new();
    let mut i = 0;
    while i < commands.len() {
        let count = match commands[i] as i32 {
            0 | 1 => 2,
            2 => 4,
            3 => 6,
            4 => 0,
            _ => return Err("path holds an unknown command".to_string()),
        };
        let Some(a) = commands.get(i + 1..i + 1 + count) else {
            return Err("path ends in the middle of a command".to_string());
        };
        let f = |k: usize| a[k] as f32;
        match commands[i] as i32 {
            0 => pb.move_to(f(0), f(1)),
            1 => pb.line_to(f(0), f(1)),
            2 => pb.quad_to(f(0), f(1), f(2), f(3)),
            3 => pb.cubic_to(f(0), f(1), f(2), f(3), f(4), f(5)),
            _ => pb.close(),
        }
        i += 1 + count;
    }
    pb.finish().ok_or_else(|| "path is empty".to_string())
}

fn path_arg(args: &[Value], index: usize) -> Result<Path, String> {
    let commands = numbers_arg(args, index, "path")?.ok_or("path must be provided")?;
    path_from_commands(&commands)
}

/// The shader of a gradient spec: `kind` (0 linear, 1 radial), four numbers
/// (`x1, y1, x2, y2` of a linear gradient; `cx, cy, radius` and a spare of a
/// radial one), then `offset, color` pairs. Coordinates are logical.
fn gradient_shader(spec: &[f64]) -> Result<Shader<'static>, String> {
    if spec.len() < 9 || (spec.len() - 5) % 2 != 0 {
        return Err("gradient needs at least two stops".to_string());
    }
    let stops = spec[5..]
        .chunks(2)
        .map(|pair| {
            let (r, g, b, a) = unpack_color(pair[1] as u32);
            GradientStop::new(pair[0] as f32, Color::from_rgba8(r, g, b, a))
        })
        .collect();
    let point = |x: f64, y: f64| Point::from_xy(x as f32, y as f32);
    let shader = if spec[0] == 0.0 {
        LinearGradient::new(
            point(spec[1], spec[2]),
            point(spec[3], spec[4]),
            stops,
            SpreadMode::Pad,
            Transform::identity(),
        )
    } else {
        let center = point(spec[1], spec[2]);
        RadialGradient::new(
            center,
            center,
            spec[3] as f32,
            stops,
            SpreadMode::Pad,
            Transform::identity(),
        )
    };
    shader.ok_or_else(|| "gradient is degenerate (zero length or radius)".to_string())
}

/// Fills `path` (logical coordinates) with `color` or the `gradient`, after
/// painting the `shadow` it casts.
fn fill_shape(
    target: &mut Target,
    path: &Path,
    color: (u8, u8, u8, u8),
    gradient: Option<&[f64]>,
    shadow: Option<&[f64]>,
) -> Result<(), String> {
    if let Some(shadow) = shadow {
        draw_shadow(target, path, shadow)?;
    }
    let mut paint = paint(color);
    if let Some(gradient) = gradient {
        paint.shader = gradient_shader(gradient)?;
    }
    let scale = target.scale as f32;
    target.pixmap.fill_path(
        path,
        &paint,
        FillRule::Winding,
        Transform::from_scale(scale, scale),
        target.state.clips.last(),
    );
    Ok(())
}

/// Paints the blurred shadow of `path`. `spec` is `blur, offsetX, offsetY,
/// spread, color`: `blur` is the CSS-style blur radius (the gaussian's
/// standard deviation is half of it), `spread` grows the shape first.
fn draw_shadow(target: &mut Target, path: &Path, spec: &[f64]) -> Result<(), String> {
    if spec.len() < 5 {
        return Err("shadow needs blur, offsetX, offsetY, spread and color".to_string());
    }
    let blur = spec[0].max(0.0) as f32;
    let (dx, dy) = (spec[1] as f32, spec[2] as f32);
    let spread = spec[3].max(0.0) as f32;
    let color = unpack_color(spec[4] as u32);
    let scale = target.scale as f32;
    let sigma = blur * scale / 2.0;
    let pad = (sigma * 3.0).ceil() + 1.0 + spread * scale;
    let (pw, ph) = (target.pixmap.width() as i32, target.pixmap.height() as i32);
    let bounds = path.bounds();
    let left = (((bounds.left() + dx) * scale - pad).floor() as i32).max(0);
    let top = (((bounds.top() + dy) * scale - pad).floor() as i32).max(0);
    let right = (((bounds.right() + dx) * scale + pad).ceil() as i32).min(pw);
    let bottom = (((bounds.bottom() + dy) * scale + pad).ceil() as i32).min(ph);
    if right <= left || bottom <= top {
        return Ok(());
    }
    let mut layer = Pixmap::new((right - left) as u32, (bottom - top) as u32)
        .ok_or_else(|| "cannot allocate shadow".to_string())?;
    let transform = Transform::from_row(
        scale,
        0.0,
        0.0,
        scale,
        dx * scale - left as f32,
        dy * scale - top as f32,
    );
    let shadow_paint = paint(color);
    layer.fill_path(path, &shadow_paint, FillRule::Winding, transform, None);
    if spread > 0.0 {
        layer.stroke_path(
            path,
            &shadow_paint,
            &stroke(spread as f64 * 2.0),
            transform,
            None,
        );
    }
    effects::blur(&mut layer, sigma);
    target.pixmap.draw_pixmap(
        left,
        top,
        layer.as_ref(),
        &PixmapPaint::default(),
        Transform::identity(),
        target.state.clips.last(),
    );
    Ok(())
}

/// The deepest clip and opacity nesting a frame may use.
const MAX_CLIPS: usize = 64;
const MAX_GROUPS: usize = 16;

/// Pushes a clip: the area of `path` (logical coordinates; none clips
/// everything away) intersected with the clip below it.
fn push_clip(target: &mut Target, path: Option<&Path>) -> Result<(), String> {
    if target.state.clips.len() >= MAX_CLIPS {
        return Err(format!("clips nest at most {} deep", MAX_CLIPS));
    }
    let scale = target.scale as f32;
    let transform = Transform::from_scale(scale, scale);
    let mut mask = match target.state.clips.last() {
        Some(top) => top.clone(),
        None => {
            let (w, h) = (target.pixmap.width(), target.pixmap.height());
            let mut mask = Mask::new(w, h).ok_or_else(|| "cannot allocate clip".to_string())?;
            mask.data_mut().fill(255);
            mask
        }
    };
    match path {
        Some(path) => mask.intersect_path(path, FillRule::Winding, true, transform),
        None => mask.data_mut().fill(0),
    }
    target.state.clips.push(mask);
    Ok(())
}

fn pop_clip(target: &mut Target) -> Result<(), String> {
    let floor = target.state.groups.last().map_or(0, |group| group.clips);
    if target.state.clips.len() <= floor {
        return Err("popClip without a matching pushClip".to_string());
    }
    target.state.clips.pop();
    Ok(())
}

/// Starts a group: what is drawn next goes to a transparent layer that
/// `pop_opacity` composites over the pixels drawn so far.
fn push_opacity(target: &mut Target, opacity: f32) -> Result<(), String> {
    if target.state.groups.len() >= MAX_GROUPS {
        return Err(format!("opacity groups nest at most {} deep", MAX_GROUPS));
    }
    let (w, h) = (target.pixmap.width(), target.pixmap.height());
    let layer = Pixmap::new(w, h).ok_or_else(|| "cannot allocate layer".to_string())?;
    let saved = std::mem::replace(target.pixmap, layer);
    let clips = target.state.clips.len();
    target.state.groups.push(Group {
        saved,
        opacity,
        clips,
    });
    Ok(())
}

fn pop_opacity(target: &mut Target) -> Result<(), String> {
    match target.state.groups.last() {
        None => return Err("popOpacity without a matching pushOpacity".to_string()),
        Some(group) if group.clips != target.state.clips.len() => {
            return Err("popClip is missing before popOpacity".to_string());
        }
        Some(_) => {}
    }
    let Some(group) = target.state.groups.pop() else {
        return Ok(());
    };
    let layer = std::mem::replace(target.pixmap, group.saved);
    let paint = PixmapPaint {
        opacity: group.opacity,
        ..PixmapPaint::default()
    };
    target
        .pixmap
        .draw_pixmap(0, 0, layer.as_ref(), &paint, Transform::identity(), None);
    Ok(())
}

/// A rectangle with rounded corners as a path; a zero radius is a plain
/// rectangle, and larger radii are clamped to what the sides allow.
fn rounded_rect(x: f32, y: f32, w: f32, h: f32, radius: f32) -> Option<Path> {
    let r = radius.min(w / 2.0).min(h / 2.0).max(0.0);
    if r == 0.0 {
        return PathBuilder::from_rect(Rect::from_xywh(x, y, w, h)?).into();
    }
    // Circular arcs as cubic Béziers, the usual 0.5523 control distance.
    let k = r * 0.552_284_8;
    let (right, bottom) = (x + w, y + h);
    let mut pb = PathBuilder::new();
    pb.move_to(x + r, y);
    pb.line_to(right - r, y);
    pb.cubic_to(right - r + k, y, right, y + r - k, right, y + r);
    pb.line_to(right, bottom - r);
    pb.cubic_to(
        right,
        bottom - r + k,
        right - r + k,
        bottom,
        right - r,
        bottom,
    );
    pb.line_to(x + r, bottom);
    pb.cubic_to(x + r - k, bottom, x, bottom - r + k, x, bottom - r);
    pb.line_to(x, y + r);
    pb.cubic_to(x, y + r - k, x + r - k, y, x + r, y);
    pb.close();
    pb.finish()
}

fn polygon_path(points: &[(f32, f32)]) -> Option<Path> {
    let mut pb = PathBuilder::new();
    let (first, rest) = points.split_first()?;
    pb.move_to(first.0, first.1);
    for (x, y) in rest {
        pb.line_to(*x, *y);
    }
    pb.close();
    pb.finish()
}

/// Reads the window id plus the `x, y, w, h` that every rectangle native
/// starts with.
fn rect_args(args: &[Value]) -> Result<(u64, f32, f32, f32, f32), String> {
    let id = integer_arg(args, 0, "window")?;
    let x = float_arg(args, 1, "x")? as f32;
    let y = float_arg(args, 2, "y")? as f32;
    let w = float_arg(args, 3, "width")? as f32;
    let h = float_arg(args, 4, "height")? as f32;
    if w < 0.0 || h < 0.0 {
        return Err("width and height must be non-negative".to_string());
    }
    Ok((id, x, y, w, h))
}

// FONTS --------------------------------------------------------------------

fn load_font(bytes: Vec<u8>) -> Result<FontEntry, String> {
    let font = fontdue::Font::from_bytes(bytes, fontdue::FontSettings::default())
        .map_err(|err| format!("cannot load font: {}", err))?;
    Ok(FontEntry {
        font,
        glyphs: HashMap::new(),
    })
}

/// Runs `body` on font `id`, loading the bundled font on first use.
fn with_font<T>(
    id: u64,
    body: impl FnOnce(&mut FontEntry) -> Result<T, String>,
) -> Result<T, String> {
    FONTS.with(|fonts| {
        let mut fonts = fonts.borrow_mut();
        if id == DEFAULT_FONT_ID && !fonts.contains_key(&id) {
            fonts.insert(id, load_font(DEFAULT_FONT.to_vec())?);
        }
        let entry = fonts
            .get_mut(&id)
            .ok_or_else(|| "font was freed".to_string())?;
        body(entry)
    })
}

/// Lays `text` out at `px` pixels per em, top-left at the origin, honouring
/// newlines. Returns the layout and its width and height.
fn layout_text(font: &fontdue::Font, text: &str, px: f32) -> (Layout, f32, f32) {
    let mut layout = Layout::new(CoordinateSystem::PositiveYDown);
    layout.reset(&LayoutSettings::default());
    layout.append(&[font], &TextStyle::new(text, px, 0));
    let width = layout
        .glyphs()
        .iter()
        .map(|glyph| glyph.x + glyph.width as f32)
        .fold(0.0_f32, f32::max);
    let height = layout.height();
    (layout, width, height)
}

/// Composites an 8-bit coverage mask at `(x0, y0)` in `color` over the
/// pixmap, clipping to its bounds.
fn blend_mask(
    pixmap: &mut Pixmap,
    x0: i32,
    y0: i32,
    width: usize,
    height: usize,
    mask: &[u8],
    color: (u8, u8, u8, u8),
    clip: Option<&Mask>,
) {
    let (pw, ph) = (pixmap.width() as i32, pixmap.height() as i32);
    let pixels = pixmap.pixels_mut();
    let (r, g, b, a) = (
        color.0 as u32,
        color.1 as u32,
        color.2 as u32,
        color.3 as u32,
    );
    for row in 0..height {
        let y = y0 + row as i32;
        if y < 0 || y >= ph {
            continue;
        }
        for column in 0..width {
            let x = x0 + column as i32;
            if x < 0 || x >= pw {
                continue;
            }
            let coverage = mask[row * width + column] as u32;
            let coverage = match clip {
                Some(clip) => coverage_in(clip, x, y, coverage),
                None => coverage,
            };
            if coverage == 0 {
                continue;
            }
            let alpha = coverage * a / 255;
            if alpha == 0 {
                continue;
            }
            let index = (y * pw + x) as usize;
            let dst = pixels[index];
            let inverse = 255 - alpha;
            let na = alpha + dst.alpha() as u32 * inverse / 255;
            let nr = (r * alpha / 255 + dst.red() as u32 * inverse / 255).min(na);
            let ng = (g * alpha / 255 + dst.green() as u32 * inverse / 255).min(na);
            let nb = (b * alpha / 255 + dst.blue() as u32 * inverse / 255).min(na);
            if let Some(blended) =
                PremultipliedColorU8::from_rgba(nr as u8, ng as u8, nb as u8, na as u8)
            {
                pixels[index] = blended;
            }
        }
    }
}

/// Draws `text` with its line box's top-left corner at physical `(x, y)`.
fn draw_text(
    pixmap: &mut Pixmap,
    entry: &mut FontEntry,
    text: &str,
    px: f32,
    x: f32,
    y: f32,
    color: (u8, u8, u8, u8),
    clip: Option<&Mask>,
) -> f32 {
    let (layout, width, _) = layout_text(&entry.font, text, px);
    if entry.glyphs.len() > GLYPH_CACHE_LIMIT {
        entry.glyphs.clear();
    }
    for glyph in layout.glyphs() {
        if !glyph.char_data.rasterize() || glyph.width == 0 {
            continue;
        }
        let font = &entry.font;
        let rasterized = entry
            .glyphs
            .entry(glyph.key)
            .or_insert_with(|| Rc::new(font.rasterize_config(glyph.key)))
            .clone();
        let (metrics, bitmap) = &*rasterized;
        blend_mask(
            pixmap,
            (x + glyph.x).round() as i32,
            (y + glyph.y).round() as i32,
            metrics.width,
            metrics.height,
            bitmap,
            color,
            clip,
        );
    }
    width
}

// IMAGES -------------------------------------------------------------------

/// Scaled copies an image may keep for redrawing at the same size. A full
/// HD animation scaled to a window keeps one copy per frame until this is
/// reached, then starts over, so it rescales each frame at most once per
/// loop instead of on every draw.
const SCALED_CACHE_BYTES: usize = 128 * 1024 * 1024;

/// A decoded image parked under an id. Drawing only reads `frames`, so an
/// off-screen canvas can later be stored and drawn the same way.
struct ImageEntry {
    frames: Vec<ImageFrame>,
    delays: Vec<f64>,
    loop_count: u32,
    /// The limits it was decoded under, which `resized` copies keep.
    limits: Limits,
    /// The last prepared (scaled or cropped) copy of each frame.
    scaled: HashMap<usize, Scaled>,
    scaled_bytes: usize,
}

struct ImageFrame {
    pixmap: Pixmap,
    opaque: bool,
}

/// What a prepared copy was made from: the source rectangle and the size
/// in physical pixels.
#[derive(Clone, Copy, PartialEq)]
struct ScaleKey {
    src: images::RectF,
    width: u32,
    height: u32,
}

struct Scaled {
    key: ScaleKey,
    frame: ImageFrame,
}

impl ImageEntry {
    fn new(decoded: Decoded, limits: Limits) -> ImageEntry {
        let frames = decoded
            .frames
            .into_iter()
            .map(|pixmap| ImageFrame {
                opaque: images::is_opaque(&pixmap),
                pixmap,
            })
            .collect();
        ImageEntry {
            frames,
            delays: decoded.delays,
            loop_count: decoded.loop_count,
            limits,
            scaled: HashMap::new(),
            scaled_bytes: 0,
        }
    }

    fn width(&self) -> u32 {
        self.frames[0].pixmap.width()
    }

    fn height(&self) -> u32 {
        self.frames[0].pixmap.height()
    }

    /// Frame `index` prepared at `key`'s crop and size, from the cache when
    /// the last draw of this frame used the same ones.
    fn prepared(&mut self, index: usize, key: ScaleKey) -> Option<&ImageFrame> {
        let hit = self.scaled.get(&index).is_some_and(|s| s.key == key);
        if !hit {
            let pixmap =
                images::resample(&self.frames[index].pixmap, key.src, key.width, key.height)?;
            let bytes = pixmap.data().len();
            if let Some(old) = self.scaled.remove(&index) {
                self.scaled_bytes -= old.frame.pixmap.data().len();
            }
            if self.scaled_bytes + bytes > SCALED_CACHE_BYTES {
                self.scaled.clear();
                self.scaled_bytes = 0;
            }
            self.scaled_bytes += bytes;
            let frame = ImageFrame {
                opaque: images::is_opaque(&pixmap),
                pixmap,
            };
            self.scaled.insert(index, Scaled { key, frame });
        }
        self.scaled.get(&index).map(|s| &s.frame)
    }
}

/// The optional `maxPixels` and `maxBytes` limits at `index` and after.
fn limits_arg(args: &[Value], index: usize) -> Result<Limits, String> {
    let defaults = Limits::default();
    let limit = |offset: usize, name: &str, default: u64| match args.get(index + offset) {
        None | Some(Value::Nil) => Ok(default),
        _ => match integer_arg(args, index + offset, name)? {
            0 => Err(format!("{} must be positive", name)),
            value => Ok(value),
        },
    };
    Ok(Limits {
        max_pixels: limit(0, "maxPixels", defaults.max_pixels)?,
        max_bytes: limit(1, "maxBytes", defaults.max_bytes)?,
    })
}

/// Parks a decoded image and returns what `lib/ui.nyk` builds the `Image`
/// from: id, width, height, frame count, loop count and frame delays.
fn register_image(entry: ImageEntry) -> Vec<Value> {
    let (width, height) = (entry.width(), entry.height());
    let frames = entry.frames.len();
    let loop_count = entry.loop_count;
    let delays = entry.delays.iter().map(|d| Value::Float(*d)).collect();
    let id = next_id();
    IMAGES.with(|images| images.borrow_mut().insert(id, entry));
    vec![
        integer(id),
        integer(width as u64),
        integer(height as u64),
        integer(frames as u64),
        integer(loop_count as u64),
        new_table(delays),
    ]
}

// NATIVES ------------------------------------------------------------------

/// `__ui_available()` reports whether a display can be reached.
fn builtin_available(_vm: &mut VM, _args: &[Value]) -> Result<Vec<Value>, String> {
    Ok(vec![Value::Bool(with_event_loop(|_| ()).is_ok())])
}

/// `__ui_open(title, width, height, resizable, visible)` creates a window and
/// returns its id and DPI scale factor.
fn builtin_open(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let request = OpenRequest {
        title: string_arg(args, 0, "title")?,
        width: float_arg(args, 1, "width")?,
        height: float_arg(args, 2, "height")?,
        resizable: bool_arg(args, 3, "resizable", true)?,
        visible: bool_arg(args, 4, "visible", true)?,
    };
    if request.width <= 0.0 || request.height <= 0.0 {
        return Err("width and height must be positive".to_string());
    }
    PENDING_OPEN.with(|pending| *pending.borrow_mut() = Some(request));
    OPENED.with(|opened| *opened.borrow_mut() = None);
    for _ in 0..OPEN_ATTEMPTS {
        if let Err(err) = pump(Duration::ZERO) {
            PENDING_OPEN.with(|pending| *pending.borrow_mut() = None);
            return Err(err);
        }
        if let Some(result) = OPENED.with(|opened| opened.borrow_mut().take()) {
            let (id, scale) = result?;
            return Ok(vec![integer(id), Value::Float(scale)]);
        }
    }
    PENDING_OPEN.with(|pending| *pending.borrow_mut() = None);
    Err("cannot create window: the event loop did not respond".to_string())
}

/// `__ui_close(window)` destroys the window; later calls on the id fail.
fn builtin_close(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let id = integer_arg(args, 0, "window")?;
    let removed = WINDOWS.with(|windows| windows.borrow_mut().remove(&id));
    if let Some(win) = removed {
        WINDOW_IDS.with(|ids| ids.borrow_mut().remove(&win.window.id()));
        drop(win);
        // Let the window system process the teardown so the window really
        // disappears even if the script never polls again.
        let _ = pump(Duration::ZERO);
    }
    Ok(vec![])
}

/// `__ui_set_title(window, title)`.
fn builtin_set_title(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let id = integer_arg(args, 0, "window")?;
    let title = string_arg(args, 1, "title")?;
    with_window(id, |win| {
        win.window.set_title(&title);
        Ok(())
    })?;
    Ok(vec![])
}

/// `__ui_set_size(window, width, height)` asks for a new logical size; the
/// `resize` event confirms what the window system granted.
fn builtin_set_size(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let id = integer_arg(args, 0, "window")?;
    let width = float_arg(args, 1, "width")?;
    let height = float_arg(args, 2, "height")?;
    if width <= 0.0 || height <= 0.0 {
        return Err("width and height must be positive".to_string());
    }
    with_window(id, |win| {
        let _ = win
            .window
            .request_inner_size(LogicalSize::new(width, height));
        Ok(())
    })?;
    Ok(vec![])
}

/// `__ui_size(window)` returns the logical width, height and scale factor.
fn builtin_size(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let id = integer_arg(args, 0, "window")?;
    with_window(id, |win| {
        let size = win.window.inner_size().to_logical::<f64>(win.scale);
        Ok(vec![
            Value::Float(size.width),
            Value::Float(size.height),
            Value::Float(win.scale),
        ])
    })
}

/// `__ui_poll(timeout?)` pumps the event loop, waiting up to `timeout`
/// seconds for something to happen, and returns the events as an array.
fn builtin_poll(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let timeout = timeout_arg(args, 0, "timeout")?;
    pump(timeout)?;
    let events = EVENTS.with(|events| std::mem::take(&mut *events.borrow_mut()));
    Ok(vec![new_table(events)])
}

/// `__ui_clear(window, color)` fills the whole back buffer.
fn builtin_clear(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let id = integer_arg(args, 0, "window")?;
    let (r, g, b, a) = color_arg(args, 1, "color")?;
    with_target(id, |win| {
        win.pixmap.fill(Color::from_rgba8(r, g, b, a));
        Ok(())
    })?;
    Ok(vec![])
}

/// `__ui_fill_rect(window, x, y, width, height, color, radius?, gradient?,
/// shadow?)`.
fn builtin_fill_rect(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let (id, x, y, w, h) = rect_args(args)?;
    let color = color_arg(args, 5, "color")?;
    let radius = optional_float_arg(args, 6, "radius", 0.0)? as f32;
    let gradient = numbers_arg(args, 7, "gradient")?;
    let shadow = numbers_arg(args, 8, "shadow")?;
    with_target(id, |target| {
        if let Some(path) = rounded_rect(x, y, w, h, radius) {
            fill_shape(target, &path, color, gradient.as_deref(), shadow.as_deref())?;
        }
        Ok(())
    })?;
    Ok(vec![])
}

/// `__ui_stroke_rect(window, x, y, width, height, color, radius?, lineWidth?)`.
fn builtin_stroke_rect(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let (id, x, y, w, h) = rect_args(args)?;
    let color = color_arg(args, 5, "color")?;
    let radius = optional_float_arg(args, 6, "radius", 0.0)? as f32;
    let width = optional_float_arg(args, 7, "lineWidth", 1.0)?;
    with_target(id, |win| {
        if let Some(path) = rounded_rect(x, y, w, h, radius) {
            let transform = Transform::from_scale(win.scale as f32, win.scale as f32);
            win.pixmap.stroke_path(
                &path,
                &paint(color),
                &stroke(width),
                transform,
                win.state.clips.last(),
            );
        }
        Ok(())
    })?;
    Ok(vec![])
}

/// `__ui_line(window, x1, y1, x2, y2, color, lineWidth?)`.
fn builtin_line(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let id = integer_arg(args, 0, "window")?;
    let x1 = float_arg(args, 1, "x1")? as f32;
    let y1 = float_arg(args, 2, "y1")? as f32;
    let x2 = float_arg(args, 3, "x2")? as f32;
    let y2 = float_arg(args, 4, "y2")? as f32;
    let color = color_arg(args, 5, "color")?;
    let width = optional_float_arg(args, 6, "lineWidth", 1.0)?;
    with_target(id, |win| {
        let mut pb = PathBuilder::new();
        pb.move_to(x1, y1);
        pb.line_to(x2, y2);
        if let Some(path) = pb.finish() {
            let transform = Transform::from_scale(win.scale as f32, win.scale as f32);
            win.pixmap.stroke_path(
                &path,
                &paint(color),
                &stroke(width),
                transform,
                win.state.clips.last(),
            );
        }
        Ok(())
    })?;
    Ok(vec![])
}

/// `__ui_circle(window, cx, cy, radius, color, fill, lineWidth?, gradient?,
/// shadow?)`.
fn builtin_circle(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let id = integer_arg(args, 0, "window")?;
    let cx = float_arg(args, 1, "x")? as f32;
    let cy = float_arg(args, 2, "y")? as f32;
    let radius = float_arg(args, 3, "radius")? as f32;
    let color = color_arg(args, 4, "color")?;
    let fill = bool_arg(args, 5, "fill", true)?;
    let width = optional_float_arg(args, 6, "lineWidth", 1.0)?;
    let gradient = numbers_arg(args, 7, "gradient")?;
    let shadow = numbers_arg(args, 8, "shadow")?;
    if radius < 0.0 {
        return Err("radius must be non-negative".to_string());
    }
    with_target(id, |win| {
        if let Some(path) = PathBuilder::from_circle(cx, cy, radius) {
            let transform = Transform::from_scale(win.scale as f32, win.scale as f32);
            if fill {
                fill_shape(win, &path, color, gradient.as_deref(), shadow.as_deref())?;
            } else {
                win.pixmap.stroke_path(
                    &path,
                    &paint(color),
                    &stroke(width),
                    transform,
                    win.state.clips.last(),
                );
            }
        }
        Ok(())
    })?;
    Ok(vec![])
}

/// `__ui_polygon(window, points, color, fill, lineWidth?, gradient?, shadow?)`
/// where `points`
/// is a flat `{x1, y1, x2, y2, ...}` array.
fn builtin_polygon(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let id = integer_arg(args, 0, "window")?;
    let points = points_arg(args, 1, "points")?;
    let color = color_arg(args, 2, "color")?;
    let fill = bool_arg(args, 3, "fill", true)?;
    let width = optional_float_arg(args, 4, "lineWidth", 1.0)?;
    let gradient = numbers_arg(args, 5, "gradient")?;
    let shadow = numbers_arg(args, 6, "shadow")?;
    if points.len() < 2 {
        return Err("points must hold at least two points".to_string());
    }
    with_target(id, |win| {
        if let Some(path) = polygon_path(&points) {
            let transform = Transform::from_scale(win.scale as f32, win.scale as f32);
            if fill {
                fill_shape(win, &path, color, gradient.as_deref(), shadow.as_deref())?;
            } else {
                win.pixmap.stroke_path(
                    &path,
                    &paint(color),
                    &stroke(width),
                    transform,
                    win.state.clips.last(),
                );
            }
        }
        Ok(())
    })?;
    Ok(vec![])
}

/// `__ui_text(window, x, y, text, size, color, font?)` draws `text` with
/// the top-left of its line box at `(x, y)` and returns the width drawn.
fn builtin_text(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let id = integer_arg(args, 0, "window")?;
    let x = float_arg(args, 1, "x")?;
    let y = float_arg(args, 2, "y")?;
    let text = string_arg(args, 3, "text")?;
    let size = float_arg(args, 4, "size")?;
    let color = color_arg(args, 5, "color")?;
    let font = match args.get(6) {
        None | Some(Value::Nil) => DEFAULT_FONT_ID,
        _ => integer_arg(args, 6, "font")?,
    };
    if size <= 0.0 {
        return Err("size must be positive".to_string());
    }
    let width = with_target(id, |win| {
        let scale = win.scale;
        with_font(font, |entry| {
            Ok(draw_text(
                &mut win.pixmap,
                entry,
                &text,
                (size * scale) as f32,
                (x * scale) as f32,
                (y * scale) as f32,
                color,
                win.state.clips.last(),
            ) as f64
                / scale)
        })
    })?;
    Ok(vec![Value::Float(width)])
}

/// `__ui_measure_text(text, size, font?)` returns the logical width and
/// height `__ui_text` would cover.
fn builtin_measure_text(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let text = string_arg(args, 0, "text")?;
    let size = float_arg(args, 1, "size")?;
    let font = match args.get(2) {
        None | Some(Value::Nil) => DEFAULT_FONT_ID,
        _ => integer_arg(args, 2, "font")?,
    };
    if size <= 0.0 {
        return Err("size must be positive".to_string());
    }
    let (width, height) = with_font(font, |entry| {
        let (_, width, height) = layout_text(&entry.font, &text, size as f32);
        Ok((width, height))
    })?;
    Ok(vec![
        Value::Float(width as f64),
        Value::Float(height as f64),
    ])
}

/// `__ui_load_image(path, maxPixels?, maxBytes?)` decodes a PNG, JPEG,
/// WebP, GIF or BMP file and returns what `register_image` describes.
fn builtin_load_image(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let path = string_arg(args, 0, "path")?;
    let limits = limits_arg(args, 1)?;
    let file =
        std::fs::File::open(&path).map_err(|err| format!("cannot open {}: {}", path, err))?;
    // No image format compresses worse than raw pixels by much, so a file
    // over the memory limit could not decode within it anyway.
    let mut bytes = Vec::new();
    file.take(limits.max_bytes + 1)
        .read_to_end(&mut bytes)
        .map_err(|err| format!("cannot read {}: {}", path, err))?;
    if bytes.len() as u64 > limits.max_bytes {
        return Err(format!(
            "cannot decode {}: file is over {} bytes (maxBytes)",
            path, limits.max_bytes
        ));
    }
    let decoded =
        images::decode(&bytes, limits).map_err(|err| format!("cannot decode {}: {}", path, err))?;
    Ok(register_image(ImageEntry::new(decoded, limits)))
}

/// `__ui_decode_image(data, maxPixels?, maxBytes?)` decodes an image held
/// in a buffer or string, like `__ui_load_image`.
fn builtin_decode_image(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let limits = limits_arg(args, 1)?;
    let decoded = match args.first() {
        Some(Value::Buffer(buffer)) => images::decode(&buffer.borrow().data, limits),
        Some(Value::String(s)) => images::decode(s.0.str.as_bytes(), limits),
        Some(value) => {
            return Err(format!(
                "data must be a buffer or string, got {}",
                value.type_name()
            ));
        }
        None => return Err("data must be provided".to_string()),
    }
    .map_err(|err| format!("cannot decode image: {}", err))?;
    Ok(register_image(ImageEntry::new(decoded, limits)))
}

/// `__ui_resize_image(image, width, height)` makes a new image with every
/// frame resampled to `width` × `height` pixels.
fn builtin_resize_image(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let id = integer_arg(args, 0, "image")?;
    let width = integer_arg(args, 1, "width")?;
    let height = integer_arg(args, 2, "height")?;
    if width == 0 || height == 0 || width > u32::MAX as u64 || height > u32::MAX as u64 {
        return Err("width and height must be positive whole numbers".to_string());
    }
    let (width, height) = (width as u32, height as u32);
    let entry = IMAGES.with(|images| {
        let images = images.borrow();
        let source = images
            .get(&id)
            .ok_or_else(|| "image was freed".to_string())?;
        let frame_bytes = images::check_size(width, height, source.limits)?;
        if frame_bytes * source.frames.len() as u64 > source.limits.max_bytes {
            return Err(format!(
                "resized image needs more than {} bytes (maxBytes)",
                source.limits.max_bytes
            ));
        }
        let full = (0.0, 0.0, source.width() as f32, source.height() as f32);
        let frames = source
            .frames
            .iter()
            .map(|frame| images::resample(&frame.pixmap, full, width, height))
            .collect::<Option<Vec<Pixmap>>>()
            .ok_or_else(|| "cannot resize image".to_string())?;
        let decoded = Decoded {
            frames,
            delays: source.delays.clone(),
            loop_count: source.loop_count,
        };
        Ok(ImageEntry::new(decoded, source.limits))
    })?;
    Ok(register_image(entry))
}

/// `__ui_free_image(image)`.
fn builtin_free_image(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let id = integer_arg(args, 0, "image")?;
    IMAGES.with(|images| images.borrow_mut().remove(&id));
    Ok(vec![])
}

/// `__ui_draw_image(window, image, x, y, width?, height?, frame?, opacity?,
/// radius?, fit?)` draws frame `frame` (from 1) of an image into the box at
/// `(x, y)`, which is the image's own size unless `width` and/or `height`
/// say otherwise, fitted by `fit` ("fill", "contain" or "cover").
///
/// The frame is resampled once to the physical size it covers and that copy
/// is cached, so redrawing it at the same size is a plain blit.
fn builtin_draw_image(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let id = integer_arg(args, 0, "window")?;
    let image = integer_arg(args, 1, "image")?;
    let x = float_arg(args, 2, "x")? as f32;
    let y = float_arg(args, 3, "y")? as f32;
    let width = optional_float_arg(args, 4, "width", 0.0)? as f32;
    let height = optional_float_arg(args, 5, "height", 0.0)? as f32;
    let frame = match args.get(6) {
        None | Some(Value::Nil) => 1,
        _ => integer_arg(args, 6, "frame")?,
    };
    let opacity = optional_float_arg(args, 7, "opacity", 1.0)? as f32;
    let radius = optional_float_arg(args, 8, "radius", 0.0)? as f32;
    let fit = match args.get(9) {
        None | Some(Value::Nil) => Fit::Fill,
        _ => Fit::parse(&string_arg(args, 9, "fit")?)?,
    };
    if width < 0.0 || height < 0.0 {
        return Err("width and height must be non-negative".to_string());
    }
    if !(0.0..=1.0).contains(&opacity) {
        return Err("opacity must be from 0 to 1".to_string());
    }
    if radius < 0.0 {
        return Err("radius must be non-negative".to_string());
    }
    if image == id {
        return Err("a canvas cannot be drawn onto itself".to_string());
    }
    // A canvas is lifted out while it is drawn, so drawing it onto another
    // canvas does not borrow the table twice.
    if let Some(canvas) = CANVASES.with(|canvases| canvases.borrow_mut().remove(&image)) {
        let result = draw_canvas(id, &canvas, x, y, width, height, opacity, radius, fit);
        CANVASES.with(|canvases| canvases.borrow_mut().insert(image, canvas));
        result?;
        return Ok(vec![]);
    }
    IMAGES.with(|images| {
        let mut images = images.borrow_mut();
        let entry = images
            .get_mut(&image)
            .ok_or_else(|| "image was freed".to_string())?;
        let count = entry.frames.len() as u64;
        if frame == 0 || frame > count {
            return Err(format!("frame must be from 1 to {}", count));
        }
        let index = (frame - 1) as usize;
        let (image_w, image_h) = (entry.width() as f32, entry.height() as f32);
        let target = (
            x,
            y,
            if width > 0.0 { width } else { image_w },
            if height > 0.0 { height } else { image_h },
        );
        let (src, dest) = images::place(image_w, image_h, target, fit);
        with_target(id, |win| {
            let scale = win.scale as f32;
            let (left, top, w, h) = images::snap((
                dest.0 * scale,
                dest.1 * scale,
                dest.2 * scale,
                dest.3 * scale,
            ));
            if w == 0 || h == 0 || opacity == 0.0 {
                return Ok(());
            }
            let unscaled =
                src == (0.0, 0.0, image_w, image_h) && (w, h) == (entry.width(), entry.height());
            let prepared = if unscaled {
                &entry.frames[index]
            } else {
                let key = ScaleKey {
                    src,
                    width: w,
                    height: h,
                };
                match entry.prepared(index, key) {
                    Some(frame) => frame,
                    None => return Ok(()),
                }
            };
            images::draw(
                &mut win.pixmap,
                prepared.pixmap.as_ref(),
                prepared.opaque,
                left,
                top,
                opacity,
                radius * scale,
                win.state.clips.last(),
            );
            Ok(())
        })
    })?;
    Ok(vec![])
}

/// `__ui_canvas(width, height, scale?)` makes an off-screen surface of
/// `width` × `height` logical pixels at `scale` physical pixels per logical
/// one (default 1), transparent at first, and returns its id. Drawing
/// natives take the id like a window's, and `__ui_draw_image` draws it.
fn builtin_canvas(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let width = float_arg(args, 0, "width")?;
    let height = float_arg(args, 1, "height")?;
    let scale = optional_float_arg(args, 2, "scale", 1.0)?;
    if width <= 0.0 || height <= 0.0 || scale <= 0.0 {
        return Err("width, height and scale must be positive".to_string());
    }
    let (pw, ph) = ((width * scale).ceil(), (height * scale).ceil());
    if pw > u32::MAX as f64 || ph > u32::MAX as f64 {
        return Err("canvas is too large".to_string());
    }
    let (pw, ph) = (pw as u32, ph as u32);
    images::check_size(pw, ph, Limits::default())?;
    let pixmap = Pixmap::new(pw, ph).ok_or_else(|| "canvas is too large".to_string())?;
    let id = next_id();
    CANVASES.with(|canvases| {
        canvases.borrow_mut().insert(
            id,
            Canvas {
                pixmap,
                scale,
                state: DrawState::default(),
            },
        )
    });
    Ok(vec![integer(id)])
}

/// `__ui_free_canvas(canvas)`.
fn builtin_free_canvas(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let id = integer_arg(args, 0, "canvas")?;
    CANVASES.with(|canvases| canvases.borrow_mut().remove(&id));
    Ok(vec![])
}

/// `__ui_push_clip(target, x, y, width, height, radius?)` limits drawing to
/// a (rounded) rectangle inside the current clip.
fn builtin_push_clip(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let (id, x, y, w, h) = rect_args(args)?;
    let radius = optional_float_arg(args, 5, "radius", 0.0)? as f32;
    if radius < 0.0 {
        return Err("radius must be non-negative".to_string());
    }
    with_target(id, |target| {
        let path = rounded_rect(x, y, w, h, radius);
        push_clip(target, path.as_ref())
    })?;
    Ok(vec![])
}

/// `__ui_push_clip_path(target, path)` limits drawing to the inside of a
/// path.
fn builtin_push_clip_path(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let id = integer_arg(args, 0, "target")?;
    let path = path_arg(args, 1)?;
    with_target(id, |target| push_clip(target, Some(&path)))?;
    Ok(vec![])
}

/// `__ui_pop_clip(target)`.
fn builtin_pop_clip(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let id = integer_arg(args, 0, "target")?;
    with_target(id, pop_clip)?;
    Ok(vec![])
}

/// `__ui_push_opacity(target, opacity)`.
fn builtin_push_opacity(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let id = integer_arg(args, 0, "target")?;
    let opacity = float_arg(args, 1, "opacity")? as f32;
    if !(0.0..=1.0).contains(&opacity) {
        return Err("opacity must be from 0 to 1".to_string());
    }
    with_target(id, |target| push_opacity(target, opacity))?;
    Ok(vec![])
}

/// `__ui_pop_opacity(target)`.
fn builtin_pop_opacity(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let id = integer_arg(args, 0, "target")?;
    with_target(id, pop_opacity)?;
    Ok(vec![])
}

/// `__ui_fill_path(target, path, color, gradient?, shadow?, evenOdd?)`.
fn builtin_fill_path(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let id = integer_arg(args, 0, "target")?;
    let path = path_arg(args, 1)?;
    let color = color_arg(args, 2, "color")?;
    let gradient = numbers_arg(args, 3, "gradient")?;
    let shadow = numbers_arg(args, 4, "shadow")?;
    let even_odd = bool_arg(args, 5, "evenOdd", false)?;
    with_target(id, |target| {
        if even_odd {
            // Shadows and gradients go through `fill_shape`, which fills
            // with the winding rule; even-odd is the plain-color case.
            if gradient.is_some() || shadow.is_some() {
                return Err("evenOdd cannot be combined with a gradient or shadow".to_string());
            }
            let scale = target.scale as f32;
            target.pixmap.fill_path(
                &path,
                &paint(color),
                FillRule::EvenOdd,
                Transform::from_scale(scale, scale),
                target.state.clips.last(),
            );
            return Ok(());
        }
        fill_shape(target, &path, color, gradient.as_deref(), shadow.as_deref())
    })?;
    Ok(vec![])
}

/// `__ui_stroke_path(target, path, color, lineWidth?, cap?, join?)`.
fn builtin_stroke_path(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let id = integer_arg(args, 0, "target")?;
    let path = path_arg(args, 1)?;
    let color = color_arg(args, 2, "color")?;
    let width = optional_float_arg(args, 3, "lineWidth", 1.0)?;
    let cap = match args.get(4) {
        None | Some(Value::Nil) => "butt".to_string(),
        _ => string_arg(args, 4, "cap")?,
    };
    let join = match args.get(5) {
        None | Some(Value::Nil) => "miter".to_string(),
        _ => string_arg(args, 5, "join")?,
    };
    let stroke = styled_stroke(width, &cap, &join)?;
    with_target(id, |target| {
        let scale = target.scale as f32;
        target.pixmap.stroke_path(
            &path,
            &paint(color),
            &stroke,
            Transform::from_scale(scale, scale),
            target.state.clips.last(),
        );
        Ok(())
    })?;
    Ok(vec![])
}

/// The largest blur radius, in logical pixels, a script may ask for.
const MAX_BLUR: f64 = 200.0;

/// `__ui_blur_region(target, x, y, width, height, radius, cornerRadius?)`
/// blurs what is already drawn under a rectangle, the way a CSS
/// `backdrop-filter` does. `radius` is the gaussian's standard deviation.
fn builtin_blur_region(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let (id, x, y, w, h) = rect_args(args)?;
    let sigma = float_arg(args, 5, "radius")?;
    let corner = optional_float_arg(args, 6, "cornerRadius", 0.0)? as f32;
    if !(0.0..=MAX_BLUR).contains(&sigma) {
        return Err(format!("radius must be from 0 to {}", MAX_BLUR));
    }
    if corner < 0.0 {
        return Err("cornerRadius must be non-negative".to_string());
    }
    if sigma == 0.0 {
        return Ok(vec![]);
    }
    with_target(id, |target| {
        let scale = target.scale as f32;
        let (left, top, width, height) = images::snap((x * scale, y * scale, w * scale, h * scale));
        let (pw, ph) = (target.pixmap.width() as i64, target.pixmap.height() as i64);
        // Blur a margin around the rectangle too, so its edges see their
        // surroundings instead of repeating their own border.
        let pad = (sigma as f32 * scale * 3.0).ceil() as i64;
        let x0 = (left as i64 - pad).max(0);
        let y0 = (top as i64 - pad).max(0);
        let x1 = (left as i64 + width as i64 + pad).min(pw);
        let y1 = (top as i64 + height as i64 + pad).min(ph);
        if x1 <= x0 || y1 <= y0 || width == 0 || height == 0 {
            return Ok(());
        }
        let (rw, rh) = ((x1 - x0) as usize, (y1 - y0) as usize);
        let mut region =
            Pixmap::new(rw as u32, rh as u32).ok_or_else(|| "cannot allocate blur".to_string())?;
        let stride = pw as usize * 4;
        let source = target.pixmap.data();
        for row in 0..rh {
            let from = (y0 as usize + row) * stride + x0 as usize * 4;
            region.data_mut()[row * rw * 4..(row + 1) * rw * 4]
                .copy_from_slice(&source[from..from + rw * 4]);
        }
        effects::blur(&mut region, sigma as f32 * scale);
        let mut cover =
            Mask::new(rw as u32, rh as u32).ok_or_else(|| "cannot allocate blur".to_string())?;
        if let Some(path) = rounded_rect(
            left as f32,
            top as f32,
            width as f32,
            height as f32,
            corner * scale,
        ) {
            cover.fill_path(
                &path,
                FillRule::Winding,
                true,
                Transform::from_translate(-x0 as f32, -y0 as f32),
            );
        }
        effects::blend_region(
            target.pixmap,
            &region,
            x0 as i32,
            y0 as i32,
            Some(&cover),
            target.state.clips.last(),
        );
        Ok(())
    })?;
    Ok(vec![])
}

/// `__ui_blur_image(image, radius)` makes a new image with every frame
/// blurred by a gaussian of standard deviation `radius` pixels.
fn builtin_blur_image(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let id = integer_arg(args, 0, "image")?;
    let sigma = float_arg(args, 1, "radius")?;
    if !(0.0..=MAX_BLUR).contains(&sigma) {
        return Err(format!("radius must be from 0 to {}", MAX_BLUR));
    }
    let entry = IMAGES.with(|images| {
        let images = images.borrow();
        let source = images
            .get(&id)
            .ok_or_else(|| "image was freed".to_string())?;
        let frames = source
            .frames
            .iter()
            .map(|frame| {
                let mut pixmap = frame.pixmap.clone();
                effects::blur(&mut pixmap, sigma as f32);
                pixmap
            })
            .collect();
        let decoded = Decoded {
            frames,
            delays: source.delays.clone(),
            loop_count: source.loop_count,
        };
        Ok::<ImageEntry, String>(ImageEntry::new(decoded, source.limits))
    })?;
    Ok(register_image(entry))
}

/// `__ui_get_pixel(target, x, y)` returns the red, green, blue and alpha
/// (0 to 255, not premultiplied) of the pixel under the logical point.
fn builtin_get_pixel(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let id = integer_arg(args, 0, "target")?;
    let x = float_arg(args, 1, "x")?;
    let y = float_arg(args, 2, "y")?;
    let pixel = with_target(id, |target| {
        let (px, py) = ((x * target.scale).floor(), (y * target.scale).floor());
        let (w, h) = (target.pixmap.width() as f64, target.pixmap.height() as f64);
        if px < 0.0 || py < 0.0 || px >= w || py >= h {
            return Err("pixel is outside the surface".to_string());
        }
        let pixel = target.pixmap.pixels()[(py as usize) * (w as usize) + px as usize];
        Ok(pixel.demultiply())
    })?;
    Ok(vec![
        integer(pixel.red() as u64),
        integer(pixel.green() as u64),
        integer(pixel.blue() as u64),
        integer(pixel.alpha() as u64),
    ])
}

/// Draws a canvas like an image: `__ui_draw_image` with a canvas id lands
/// here. The canvas is resampled to the size it covers unless it is drawn
/// one to one.
#[allow(clippy::too_many_arguments)]
fn draw_canvas(
    id: u64,
    canvas: &Canvas,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    opacity: f32,
    radius: f32,
    fit: Fit,
) -> Result<(), String> {
    let (pixel_w, pixel_h) = (canvas.pixmap.width(), canvas.pixmap.height());
    let scale = canvas.scale as f32;
    let target = (
        x,
        y,
        if width > 0.0 {
            width
        } else {
            pixel_w as f32 / scale
        },
        if height > 0.0 {
            height
        } else {
            pixel_h as f32 / scale
        },
    );
    let (src, dest) = images::place(pixel_w as f32, pixel_h as f32, target, fit);
    with_target(id, |dst| {
        let scale = dst.scale as f32;
        let (left, top, w, h) = images::snap((
            dest.0 * scale,
            dest.1 * scale,
            dest.2 * scale,
            dest.3 * scale,
        ));
        if w == 0 || h == 0 || opacity == 0.0 {
            return Ok(());
        }
        let resampled;
        let source =
            if src == (0.0, 0.0, pixel_w as f32, pixel_h as f32) && (w, h) == (pixel_w, pixel_h) {
                &canvas.pixmap
            } else {
                resampled = images::resample(&canvas.pixmap, src, w, h)
                    .ok_or_else(|| "cannot scale canvas".to_string())?;
                &resampled
            };
        images::draw(
            dst.pixmap,
            source.as_ref(),
            false,
            left,
            top,
            opacity,
            radius * scale,
            dst.state.clips.last(),
        );
        Ok(())
    })
}

/// `__ui_load_font(path)` reads a TrueType or OpenType font and returns its
/// id.
fn builtin_load_font(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let path = string_arg(args, 0, "path")?;
    let bytes = std::fs::read(&path).map_err(|err| format!("cannot open {}: {}", path, err))?;
    let entry = load_font(bytes)?;
    let id = next_id();
    FONTS.with(|fonts| fonts.borrow_mut().insert(id, entry));
    Ok(vec![integer(id)])
}

/// `__ui_free_font(font)`.
fn builtin_free_font(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let id = integer_arg(args, 0, "font")?;
    if id == DEFAULT_FONT_ID {
        return Err("the default font cannot be freed".to_string());
    }
    FONTS.with(|fonts| fonts.borrow_mut().remove(&id));
    Ok(vec![])
}

/// `__ui_present(window)` shows what has been drawn since the last call.
fn builtin_present(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let id = integer_arg(args, 0, "window")?;
    with_window(id, |win| {
        if !win.state.groups.is_empty() || !win.state.clips.is_empty() {
            // Drop the unfinished layers so the next frame starts clean.
            let state = std::mem::take(&mut win.state);
            if let Some(first) = state.groups.into_iter().next() {
                win.pixmap = first.saved;
            }
            return Err("popClip or popOpacity is missing before present".to_string());
        }
        blit(win)
    })?;
    Ok(vec![])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colors_unpack_as_rgba() {
        assert_eq!(unpack_color(0x11223344), (0x11, 0x22, 0x33, 0x44));
        assert_eq!(unpack_color(0xff0000ff), (255, 0, 0, 255));
    }

    #[test]
    fn color_arg_rejects_out_of_range_values() {
        let err = color_arg(&[Value::Float(1e12)], 0, "color").unwrap_err();
        assert!(err.contains("out of range"), "{}", err);
        let err = color_arg(&[Value::Float(-1.0)], 0, "color").unwrap_err();
        assert!(err.contains("non-negative"), "{}", err);
    }

    #[test]
    fn packing_drops_alpha_and_orders_channels() {
        let pixels = [PremultipliedColorU8::from_rgba(10, 20, 30, 255).unwrap()];
        let mut out = [0u32; 1];
        pack_pixels(&pixels, &mut out);
        assert_eq!(out[0], 0x000a141e);
    }

    #[test]
    fn mask_blending_covers_and_clips() {
        let mut pixmap = Pixmap::new(4, 4).unwrap();
        pixmap.fill(Color::WHITE);
        // A 2×2 fully covered mask drawn half outside the pixmap.
        blend_mask(&mut pixmap, 3, 3, 2, 2, &[255; 4], (255, 0, 0, 255), None);
        let pixels = pixmap.pixels();
        assert_eq!(
            (pixels[15].red(), pixels[15].green(), pixels[15].blue()),
            (255, 0, 0)
        );
        assert_eq!(
            (pixels[0].red(), pixels[0].green(), pixels[0].blue()),
            (255, 255, 255)
        );
        // Half coverage of an opaque color leaves a blend.
        blend_mask(&mut pixmap, 0, 0, 1, 1, &[128], (0, 0, 255, 255), None);
        let blended = pixmap.pixels()[0];
        assert!(blended.red() < 255 && blended.red() > 0);
        assert_eq!(blended.alpha(), 255);
    }

    #[test]
    fn default_font_renders_glyphs() {
        let mut entry = load_font(DEFAULT_FONT.to_vec()).unwrap();
        let mut pixmap = Pixmap::new(64, 32).unwrap();
        pixmap.fill(Color::WHITE);
        let width = draw_text(
            &mut pixmap,
            &mut entry,
            "Ab",
            20.0,
            2.0,
            2.0,
            (0, 0, 0, 255),
            None,
        );
        assert!(width > 10.0 && width < 40.0, "unexpected width {}", width);
        assert!(pixmap.pixels().iter().any(|p| p.red() < 128));
        assert!(!entry.glyphs.is_empty());

        let (_, w, h) = layout_text(&entry.font, "line\nline", 16.0);
        assert!(h > 24.0, "two lines should be taller than one: {}", h);
        assert!(w > 0.0);
    }

    #[test]
    fn os_errors_lose_their_source_location() {
        assert_eq!(
            tidy_os_error("os error at /x/mod.rs:765: DISPLAY is not set."),
            "DISPLAY is not set."
        );
        assert_eq!(tidy_os_error("plain message"), "plain message");
    }

    #[test]
    fn rounded_rect_paths_are_built() {
        assert!(rounded_rect(0.0, 0.0, 10.0, 10.0, 0.0).is_some());
        assert!(rounded_rect(0.0, 0.0, 10.0, 10.0, 50.0).is_some());
        assert!(rounded_rect(0.0, 0.0, -1.0, 10.0, 0.0).is_none());
    }

    #[test]
    fn image_limits_are_read_and_checked() {
        let limits = limits_arg(&[Value::Nil], 1).unwrap();
        assert_eq!(limits.max_pixels, images::DEFAULT_MAX_PIXELS);
        assert_eq!(limits.max_bytes, images::DEFAULT_MAX_BYTES);
        let limits = limits_arg(&[Value::Nil, Value::Int(10), Value::Int(20)], 1).unwrap();
        assert_eq!((limits.max_pixels, limits.max_bytes), (10, 20));
        let err = limits_arg(&[Value::Nil, Value::Int(0)], 1).unwrap_err();
        assert!(err.contains("maxPixels must be positive"), "{}", err);
    }

    #[test]
    fn scaled_frames_are_cached_per_size() {
        let mut pixmap = Pixmap::new(8, 8).unwrap();
        pixmap.fill(Color::BLACK);
        let decoded = Decoded {
            frames: vec![pixmap.clone(), pixmap],
            delays: vec![0.1, 0.1],
            loop_count: 0,
        };
        let mut entry = ImageEntry::new(decoded, Limits::default());
        assert!(entry.frames[0].opaque);
        let key = ScaleKey {
            src: (0.0, 0.0, 8.0, 8.0),
            width: 4,
            height: 4,
        };
        let first = entry.prepared(0, key).unwrap().pixmap.data().as_ptr();
        let again = entry.prepared(0, key).unwrap().pixmap.data().as_ptr();
        assert_eq!(first, again, "the same size should reuse the cached copy");
        entry.prepared(1, key).unwrap();
        assert_eq!(entry.scaled.len(), 2, "each frame keeps its own copy");
        assert_eq!(entry.scaled_bytes, 2 * 4 * 4 * 4);
        let bigger = ScaleKey { width: 6, ..key };
        assert_eq!(entry.prepared(0, bigger).unwrap().pixmap.width(), 6);
        assert_eq!(entry.scaled_bytes, 4 * 4 * 4 + 6 * 4 * 4);
    }

    #[test]
    fn points_need_pairs() {
        let table = new_table(vec![
            Value::Float(1.0),
            Value::Float(2.0),
            Value::Float(3.0),
        ]);
        let err = points_arg(&[table], 0, "points").unwrap_err();
        assert!(err.contains("pairs"), "{}", err);
    }
}
