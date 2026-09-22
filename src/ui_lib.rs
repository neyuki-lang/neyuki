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
//! `Image` and `Font` objects and the `run` loop on top of these.
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
use std::fs::File;
use std::io::BufReader;
use std::num::NonZeroU32;
use std::rc::Rc;
use std::time::Duration;

use fontdue::layout::{CoordinateSystem, GlyphRasterConfig, Layout, LayoutSettings, TextStyle};
use num_bigint::BigInt;
use tiny_skia::{
    Color, ColorU8, FillRule, FilterQuality, Paint, Path, PathBuilder, Pixmap, PixmapPaint,
    PremultipliedColorU8, Rect, Stroke, Transform,
};
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::keyboard::{Key, ModifiersState, PhysicalKey};
use winit::platform::pump_events::EventLoopExtPumpEvents;
use winit::window::{Window, WindowId};

use crate::runtime::{Int, Value, new_table};

pub(crate) const NATIVES: &[(&str, crate::runtime::Native)] = &[
    ("__ui_available", builtin_available),
    ("__ui_open", builtin_open),
    ("__ui_close", builtin_close),
    ("__ui_set_title", builtin_set_title),
    ("__ui_set_size", builtin_set_size),
    ("__ui_size", builtin_size),
    ("__ui_poll", builtin_poll),
    ("__ui_clear", builtin_clear),
    ("__ui_fill_rect", builtin_fill_rect),
    ("__ui_stroke_rect", builtin_stroke_rect),
    ("__ui_line", builtin_line),
    ("__ui_circle", builtin_circle),
    ("__ui_polygon", builtin_polygon),
    ("__ui_text", builtin_text),
    ("__ui_measure_text", builtin_measure_text),
    ("__ui_load_image", builtin_load_image),
    ("__ui_free_image", builtin_free_image),
    ("__ui_draw_image", builtin_draw_image),
    ("__ui_load_font", builtin_load_font),
    ("__ui_free_font", builtin_free_font),
    ("__ui_present", builtin_present),
];

/// The font used when a script does not load its own: Noto Sans, under the
/// SIL Open Font License (see `lib/fonts/NotoSans-OFL.txt`).
const DEFAULT_FONT: &[u8] = include_bytes!("../lib/fonts/NotoSans-Regular.ttf");
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
    static IMAGES: RefCell<HashMap<u64, Pixmap>> = RefCell::new(HashMap::new());
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
        Some(Value::String(value)) => Ok(value.clone()),
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
    let value = args.get(index).cloned().unwrap_or(Value::Nil);
    if matches!(value, Value::Nil) {
        return Err(format!("{} must be provided", name));
    }
    let number = crate::runtime::number(value).map_err(|_| format!("{} must be a number", name))?;
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
            .cloned()
            .map(|value| {
                crate::runtime::number(value)
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
    let table = new_table(Vec::new());
    if let Value::Table(inner) = &table {
        let mut inner = inner.borrow_mut();
        for (name, value) in fields {
            inner.fields.insert(name.to_string(), value);
        }
    }
    table
}

fn integer(value: u64) -> Value {
    Value::Integer(Int::from_bigint(BigInt::from(value)))
}

fn text(value: &str) -> Value {
    Value::String(value.to_string())
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
            }
            let (width, height) = logical(win);
            kind(
                "resize",
                vec![
                    ("width", Value::Number(width)),
                    ("height", Value::Number(height)),
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
                    ("key", Value::String(name)),
                    ("code", Value::String(code)),
                    ("repeat", Value::Bool(event.repeat)),
                    ("text", typed.map(Value::String).unwrap_or(Value::Nil)),
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
                    ("x", Value::Number(position.x)),
                    ("y", Value::Number(position.y)),
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
                    ("button", Value::String(button)),
                    ("x", Value::Number(win.cursor.0)),
                    ("y", Value::Number(win.cursor.1)),
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
                    ("dx", Value::Number(dx)),
                    ("dy", Value::Number(dy)),
                    ("unit", text(unit)),
                    ("x", Value::Number(win.cursor.0)),
                    ("y", Value::Number(win.cursor.1)),
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
        );
    }
    width
}

// IMAGES -------------------------------------------------------------------

fn decode_png(path: &str) -> Result<Pixmap, String> {
    let file = File::open(path).map_err(|err| format!("cannot open {}: {}", path, err))?;
    let mut decoder = png::Decoder::new(BufReader::new(file));
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder
        .read_info()
        .map_err(|err| format!("cannot decode {}: {}", path, err))?;
    let size = reader
        .output_buffer_size()
        .ok_or_else(|| format!("cannot decode {}: image too large", path))?;
    let mut buffer = vec![0; size];
    let info = reader
        .next_frame(&mut buffer)
        .map_err(|err| format!("cannot decode {}: {}", path, err))?;
    let bytes = &buffer[..info.buffer_size()];
    let rgba: Vec<u8> = match info.color_type {
        png::ColorType::Rgba => bytes.to_vec(),
        png::ColorType::Rgb => bytes
            .chunks(3)
            .flat_map(|p| [p[0], p[1], p[2], 255])
            .collect(),
        png::ColorType::Grayscale => bytes.iter().flat_map(|&g| [g, g, g, 255]).collect(),
        png::ColorType::GrayscaleAlpha => bytes
            .chunks(2)
            .flat_map(|p| [p[0], p[0], p[0], p[1]])
            .collect(),
        png::ColorType::Indexed => {
            return Err(format!("cannot decode {}: unexpected indexed output", path));
        }
    };
    rgba_to_pixmap(&rgba, info.width, info.height)
        .ok_or_else(|| format!("cannot decode {}: image too large", path))
}

fn rgba_to_pixmap(rgba: &[u8], width: u32, height: u32) -> Option<Pixmap> {
    let mut pixmap = Pixmap::new(width, height)?;
    for (pixel, source) in pixmap.pixels_mut().iter_mut().zip(rgba.chunks(4)) {
        *pixel = ColorU8::from_rgba(source[0], source[1], source[2], source[3]).premultiply();
    }
    Some(pixmap)
}

// NATIVES ------------------------------------------------------------------

/// `__ui_available()` reports whether a display can be reached.
fn builtin_available(_args: Vec<Value>) -> Result<Vec<Value>, String> {
    Ok(vec![Value::Bool(with_event_loop(|_| ()).is_ok())])
}

/// `__ui_open(title, width, height, resizable, visible)` creates a window and
/// returns its id and DPI scale factor.
fn builtin_open(args: Vec<Value>) -> Result<Vec<Value>, String> {
    let request = OpenRequest {
        title: string_arg(&args, 0, "title")?,
        width: float_arg(&args, 1, "width")?,
        height: float_arg(&args, 2, "height")?,
        resizable: bool_arg(&args, 3, "resizable", true)?,
        visible: bool_arg(&args, 4, "visible", true)?,
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
            return Ok(vec![integer(id), Value::Number(scale)]);
        }
    }
    PENDING_OPEN.with(|pending| *pending.borrow_mut() = None);
    Err("cannot create window: the event loop did not respond".to_string())
}

/// `__ui_close(window)` destroys the window; later calls on the id fail.
fn builtin_close(args: Vec<Value>) -> Result<Vec<Value>, String> {
    let id = integer_arg(&args, 0, "window")?;
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
fn builtin_set_title(args: Vec<Value>) -> Result<Vec<Value>, String> {
    let id = integer_arg(&args, 0, "window")?;
    let title = string_arg(&args, 1, "title")?;
    with_window(id, |win| {
        win.window.set_title(&title);
        Ok(())
    })?;
    Ok(vec![])
}

/// `__ui_set_size(window, width, height)` asks for a new logical size; the
/// `resize` event confirms what the window system granted.
fn builtin_set_size(args: Vec<Value>) -> Result<Vec<Value>, String> {
    let id = integer_arg(&args, 0, "window")?;
    let width = float_arg(&args, 1, "width")?;
    let height = float_arg(&args, 2, "height")?;
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
fn builtin_size(args: Vec<Value>) -> Result<Vec<Value>, String> {
    let id = integer_arg(&args, 0, "window")?;
    with_window(id, |win| {
        let size = win.window.inner_size().to_logical::<f64>(win.scale);
        Ok(vec![
            Value::Number(size.width),
            Value::Number(size.height),
            Value::Number(win.scale),
        ])
    })
}

/// `__ui_poll(timeout?)` pumps the event loop, waiting up to `timeout`
/// seconds for something to happen, and returns the events as an array.
fn builtin_poll(args: Vec<Value>) -> Result<Vec<Value>, String> {
    let timeout = timeout_arg(&args, 0, "timeout")?;
    pump(timeout)?;
    let events = EVENTS.with(|events| std::mem::take(&mut *events.borrow_mut()));
    Ok(vec![new_table(events)])
}

/// `__ui_clear(window, color)` fills the whole back buffer.
fn builtin_clear(args: Vec<Value>) -> Result<Vec<Value>, String> {
    let id = integer_arg(&args, 0, "window")?;
    let (r, g, b, a) = color_arg(&args, 1, "color")?;
    with_window(id, |win| {
        win.pixmap.fill(Color::from_rgba8(r, g, b, a));
        Ok(())
    })?;
    Ok(vec![])
}

/// `__ui_fill_rect(window, x, y, width, height, color, radius?)`.
fn builtin_fill_rect(args: Vec<Value>) -> Result<Vec<Value>, String> {
    let (id, x, y, w, h) = rect_args(&args)?;
    let color = color_arg(&args, 5, "color")?;
    let radius = optional_float_arg(&args, 6, "radius", 0.0)? as f32;
    with_window(id, |win| {
        if let Some(path) = rounded_rect(x, y, w, h, radius) {
            let transform = Transform::from_scale(win.scale as f32, win.scale as f32);
            win.pixmap
                .fill_path(&path, &paint(color), FillRule::Winding, transform, None);
        }
        Ok(())
    })?;
    Ok(vec![])
}

/// `__ui_stroke_rect(window, x, y, width, height, color, radius?, lineWidth?)`.
fn builtin_stroke_rect(args: Vec<Value>) -> Result<Vec<Value>, String> {
    let (id, x, y, w, h) = rect_args(&args)?;
    let color = color_arg(&args, 5, "color")?;
    let radius = optional_float_arg(&args, 6, "radius", 0.0)? as f32;
    let width = optional_float_arg(&args, 7, "lineWidth", 1.0)?;
    with_window(id, |win| {
        if let Some(path) = rounded_rect(x, y, w, h, radius) {
            let transform = Transform::from_scale(win.scale as f32, win.scale as f32);
            win.pixmap
                .stroke_path(&path, &paint(color), &stroke(width), transform, None);
        }
        Ok(())
    })?;
    Ok(vec![])
}

/// `__ui_line(window, x1, y1, x2, y2, color, lineWidth?)`.
fn builtin_line(args: Vec<Value>) -> Result<Vec<Value>, String> {
    let id = integer_arg(&args, 0, "window")?;
    let x1 = float_arg(&args, 1, "x1")? as f32;
    let y1 = float_arg(&args, 2, "y1")? as f32;
    let x2 = float_arg(&args, 3, "x2")? as f32;
    let y2 = float_arg(&args, 4, "y2")? as f32;
    let color = color_arg(&args, 5, "color")?;
    let width = optional_float_arg(&args, 6, "lineWidth", 1.0)?;
    with_window(id, |win| {
        let mut pb = PathBuilder::new();
        pb.move_to(x1, y1);
        pb.line_to(x2, y2);
        if let Some(path) = pb.finish() {
            let transform = Transform::from_scale(win.scale as f32, win.scale as f32);
            win.pixmap
                .stroke_path(&path, &paint(color), &stroke(width), transform, None);
        }
        Ok(())
    })?;
    Ok(vec![])
}

/// `__ui_circle(window, cx, cy, radius, color, fill, lineWidth?)`.
fn builtin_circle(args: Vec<Value>) -> Result<Vec<Value>, String> {
    let id = integer_arg(&args, 0, "window")?;
    let cx = float_arg(&args, 1, "x")? as f32;
    let cy = float_arg(&args, 2, "y")? as f32;
    let radius = float_arg(&args, 3, "radius")? as f32;
    let color = color_arg(&args, 4, "color")?;
    let fill = bool_arg(&args, 5, "fill", true)?;
    let width = optional_float_arg(&args, 6, "lineWidth", 1.0)?;
    if radius < 0.0 {
        return Err("radius must be non-negative".to_string());
    }
    with_window(id, |win| {
        if let Some(path) = PathBuilder::from_circle(cx, cy, radius) {
            let transform = Transform::from_scale(win.scale as f32, win.scale as f32);
            if fill {
                win.pixmap
                    .fill_path(&path, &paint(color), FillRule::Winding, transform, None);
            } else {
                win.pixmap
                    .stroke_path(&path, &paint(color), &stroke(width), transform, None);
            }
        }
        Ok(())
    })?;
    Ok(vec![])
}

/// `__ui_polygon(window, points, color, fill, lineWidth?)` where `points`
/// is a flat `{x1, y1, x2, y2, ...}` array.
fn builtin_polygon(args: Vec<Value>) -> Result<Vec<Value>, String> {
    let id = integer_arg(&args, 0, "window")?;
    let points = points_arg(&args, 1, "points")?;
    let color = color_arg(&args, 2, "color")?;
    let fill = bool_arg(&args, 3, "fill", true)?;
    let width = optional_float_arg(&args, 4, "lineWidth", 1.0)?;
    if points.len() < 2 {
        return Err("points must hold at least two points".to_string());
    }
    with_window(id, |win| {
        if let Some(path) = polygon_path(&points) {
            let transform = Transform::from_scale(win.scale as f32, win.scale as f32);
            if fill {
                win.pixmap
                    .fill_path(&path, &paint(color), FillRule::Winding, transform, None);
            } else {
                win.pixmap
                    .stroke_path(&path, &paint(color), &stroke(width), transform, None);
            }
        }
        Ok(())
    })?;
    Ok(vec![])
}

/// `__ui_text(window, x, y, text, size, color, font?)` draws `text` with
/// the top-left of its line box at `(x, y)` and returns the width drawn.
fn builtin_text(args: Vec<Value>) -> Result<Vec<Value>, String> {
    let id = integer_arg(&args, 0, "window")?;
    let x = float_arg(&args, 1, "x")?;
    let y = float_arg(&args, 2, "y")?;
    let text = string_arg(&args, 3, "text")?;
    let size = float_arg(&args, 4, "size")?;
    let color = color_arg(&args, 5, "color")?;
    let font = match args.get(6) {
        None | Some(Value::Nil) => DEFAULT_FONT_ID,
        _ => integer_arg(&args, 6, "font")?,
    };
    if size <= 0.0 {
        return Err("size must be positive".to_string());
    }
    let width = with_window(id, |win| {
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
            ) as f64
                / scale)
        })
    })?;
    Ok(vec![Value::Number(width)])
}

/// `__ui_measure_text(text, size, font?)` returns the logical width and
/// height `__ui_text` would cover.
fn builtin_measure_text(args: Vec<Value>) -> Result<Vec<Value>, String> {
    let text = string_arg(&args, 0, "text")?;
    let size = float_arg(&args, 1, "size")?;
    let font = match args.get(2) {
        None | Some(Value::Nil) => DEFAULT_FONT_ID,
        _ => integer_arg(&args, 2, "font")?,
    };
    if size <= 0.0 {
        return Err("size must be positive".to_string());
    }
    let (width, height) = with_font(font, |entry| {
        let (_, width, height) = layout_text(&entry.font, &text, size as f32);
        Ok((width, height))
    })?;
    Ok(vec![
        Value::Number(width as f64),
        Value::Number(height as f64),
    ])
}

/// `__ui_load_image(path)` decodes a PNG and returns its id, width and
/// height.
fn builtin_load_image(args: Vec<Value>) -> Result<Vec<Value>, String> {
    let path = string_arg(&args, 0, "path")?;
    let pixmap = decode_png(&path)?;
    let (width, height) = (pixmap.width(), pixmap.height());
    let id = next_id();
    IMAGES.with(|images| images.borrow_mut().insert(id, pixmap));
    Ok(vec![
        integer(id),
        integer(width as u64),
        integer(height as u64),
    ])
}

/// `__ui_free_image(image)`.
fn builtin_free_image(args: Vec<Value>) -> Result<Vec<Value>, String> {
    let id = integer_arg(&args, 0, "image")?;
    IMAGES.with(|images| images.borrow_mut().remove(&id));
    Ok(vec![])
}

/// `__ui_draw_image(window, image, x, y, width?, height?)` draws an image
/// at its own size or scaled to `width` × `height`.
fn builtin_draw_image(args: Vec<Value>) -> Result<Vec<Value>, String> {
    let id = integer_arg(&args, 0, "window")?;
    let image = integer_arg(&args, 1, "image")?;
    let x = float_arg(&args, 2, "x")? as f32;
    let y = float_arg(&args, 3, "y")? as f32;
    let width = optional_float_arg(&args, 4, "width", 0.0)? as f32;
    let height = optional_float_arg(&args, 5, "height", 0.0)? as f32;
    if width < 0.0 || height < 0.0 {
        return Err("width and height must be non-negative".to_string());
    }
    IMAGES.with(|images| {
        let images = images.borrow();
        let source = images
            .get(&image)
            .ok_or_else(|| "image was freed".to_string())?;
        with_window(id, |win| {
            let sx = if width > 0.0 {
                width / source.width() as f32
            } else {
                1.0
            };
            let sy = if height > 0.0 {
                height / source.height() as f32
            } else {
                1.0
            };
            let transform = Transform::from_scale(sx, sy)
                .post_translate(x, y)
                .post_scale(win.scale as f32, win.scale as f32);
            let paint = PixmapPaint {
                quality: FilterQuality::Bilinear,
                ..PixmapPaint::default()
            };
            win.pixmap
                .draw_pixmap(0, 0, source.as_ref(), &paint, transform, None);
            Ok(())
        })
    })?;
    Ok(vec![])
}

/// `__ui_load_font(path)` reads a TrueType or OpenType font and returns its
/// id.
fn builtin_load_font(args: Vec<Value>) -> Result<Vec<Value>, String> {
    let path = string_arg(&args, 0, "path")?;
    let bytes = std::fs::read(&path).map_err(|err| format!("cannot open {}: {}", path, err))?;
    let entry = load_font(bytes)?;
    let id = next_id();
    FONTS.with(|fonts| fonts.borrow_mut().insert(id, entry));
    Ok(vec![integer(id)])
}

/// `__ui_free_font(font)`.
fn builtin_free_font(args: Vec<Value>) -> Result<Vec<Value>, String> {
    let id = integer_arg(&args, 0, "font")?;
    if id == DEFAULT_FONT_ID {
        return Err("the default font cannot be freed".to_string());
    }
    FONTS.with(|fonts| fonts.borrow_mut().remove(&id));
    Ok(vec![])
}

/// `__ui_present(window)` shows what has been drawn since the last call.
fn builtin_present(args: Vec<Value>) -> Result<Vec<Value>, String> {
    let id = integer_arg(&args, 0, "window")?;
    with_window(id, blit)?;
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
        let err = color_arg(&[Value::Number(1e12)], 0, "color").unwrap_err();
        assert!(err.contains("out of range"), "{}", err);
        let err = color_arg(&[Value::Number(-1.0)], 0, "color").unwrap_err();
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
        blend_mask(&mut pixmap, 3, 3, 2, 2, &[255; 4], (255, 0, 0, 255));
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
        blend_mask(&mut pixmap, 0, 0, 1, 1, &[128], (0, 0, 255, 255));
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
    fn rgba_images_are_premultiplied() {
        let pixmap = rgba_to_pixmap(&[255, 255, 255, 128, 0, 0, 0, 0], 2, 1).unwrap();
        let pixels = pixmap.pixels();
        assert_eq!(pixels[0].alpha(), 128);
        assert!(pixels[0].red() <= 128);
        assert_eq!(pixels[1].alpha(), 0);
    }

    #[test]
    fn points_need_pairs() {
        let table = new_table(vec![
            Value::Number(1.0),
            Value::Number(2.0),
            Value::Number(3.0),
        ]);
        let err = points_arg(&[table], 0, "points").unwrap_err();
        assert!(err.contains("pairs"), "{}", err);
    }
}
