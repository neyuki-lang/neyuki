//! Native web views backing `@neyuki/webview`.
//!
//! Pages are rendered by the operating system's web engine through `wry`:
//! WebView2 on Windows, WKWebView on macOS and WebKitGTK on Linux. On Windows
//! and macOS a view lives in a `winit` window created by the same event loop
//! `ui.rs` pumps (there is no second loop): `__webview_open` parks a request
//! that `ui::App` fulfils from inside the loop, exactly as `__ui_open` does.
//! WebKitGTK cannot be hosted by a `winit` window, so on Linux the view gets
//! a GTK window of its own and `__ui_poll` runs the pending GTK events after
//! it pumps `winit`.
//!
//! Like the rest of the UI, the design is pull-based: natives cannot call
//! back into Neyuki, so everything the page or the window does (messages,
//! navigation, load, title, close) is queued here and appended to the list
//! `__ui_poll` returns, as tables with `kind = "webview"`.
//!
//! The module is behind the `webview` cargo feature (on by default). Without
//! it every native except `__webview_available` fails with a clear error, so
//! a build that cannot link the Linux system libraries still runs everything
//! else. Everything the page sends is untrusted; nothing here exposes files
//! or processes to page scripts. The only file access is the optional asset
//! directory, read through a custom protocol that cannot leave it.

#[cfg(feature = "webview")]
pub(crate) use real::{create_pending, pump_toolkit, take_events, window_event};
#[cfg(feature = "webview")]
pub(crate) const NATIVES: &[crate::vm::value::NativeDef] = real::NATIVES;

#[cfg(not(feature = "webview"))]
pub(crate) use stub::{create_pending, pump_toolkit, take_events, window_event};
#[cfg(not(feature = "webview"))]
pub(crate) const NATIVES: &[crate::vm::value::NativeDef] = stub::NATIVES;

#[cfg(not(feature = "webview"))]
mod stub {
    use winit::event::WindowEvent;
    use winit::event_loop::ActiveEventLoop;
    use winit::window::WindowId;

    use crate::vm::machine::VM;
    use crate::vm::value::{NativeDef, Value};

    pub(crate) fn create_pending(_: &ActiveEventLoop) {}

    pub(crate) fn window_event(_: WindowId, _: &WindowEvent) -> bool {
        false
    }

    pub(crate) fn pump_toolkit() {}

    pub(crate) fn take_events() -> Vec<Value> {
        Vec::new()
    }

    fn available(_vm: &mut VM, _args: &[Value]) -> Result<Vec<Value>, String> {
        Ok(vec![Value::Bool(false)])
    }

    fn unsupported(_vm: &mut VM, _args: &[Value]) -> Result<Vec<Value>, String> {
        Err(
            "this build of Neyuki has no webview support (built without the `webview` feature)"
                .to_string(),
        )
    }

    pub(crate) const NATIVES: &[NativeDef] = &[
        NativeDef {
            name: "__webview_available",
            func: available,
        },
        NativeDef {
            name: "__webview_open",
            func: unsupported,
        },
        NativeDef {
            name: "__webview_close",
            func: unsupported,
        },
        NativeDef {
            name: "__webview_navigate",
            func: unsupported,
        },
        NativeDef {
            name: "__webview_load_html",
            func: unsupported,
        },
        NativeDef {
            name: "__webview_reload",
            func: unsupported,
        },
        NativeDef {
            name: "__webview_eval",
            func: unsupported,
        },
        NativeDef {
            name: "__webview_set_title",
            func: unsupported,
        },
        NativeDef {
            name: "__webview_set_size",
            func: unsupported,
        },
        NativeDef {
            name: "__webview_size",
            func: unsupported,
        },
        NativeDef {
            name: "__webview_set_visible",
            func: unsupported,
        },
        NativeDef {
            name: "__webview_clear_data",
            func: unsupported,
        },
        NativeDef {
            name: "__webview_url",
            func: unsupported,
        },
        NativeDef {
            name: "__webview_asset_base",
            func: unsupported,
        },
    ];
}

#[cfg(feature = "webview")]
mod real {
    use std::borrow::Cow;
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::path::{Path, PathBuf};

    use wry::http::Response;
    use wry::{PageLoadEvent, WebContext, WebView, WebViewBuilder};

    use crate::vm::libs::ui::{
        bool_arg, float_arg, integer, integer_arg, record, string_arg, text,
    };
    use crate::vm::machine::VM;
    use crate::vm::value::{NativeDef, Value};

    pub(crate) const NATIVES: &[NativeDef] = &[
        NativeDef {
            name: "__webview_available",
            func: builtin_available,
        },
        NativeDef {
            name: "__webview_open",
            func: builtin_open,
        },
        NativeDef {
            name: "__webview_close",
            func: builtin_close,
        },
        NativeDef {
            name: "__webview_navigate",
            func: builtin_navigate,
        },
        NativeDef {
            name: "__webview_load_html",
            func: builtin_load_html,
        },
        NativeDef {
            name: "__webview_reload",
            func: builtin_reload,
        },
        NativeDef {
            name: "__webview_eval",
            func: builtin_eval,
        },
        NativeDef {
            name: "__webview_set_title",
            func: builtin_set_title,
        },
        NativeDef {
            name: "__webview_set_size",
            func: builtin_set_size,
        },
        NativeDef {
            name: "__webview_size",
            func: builtin_size,
        },
        NativeDef {
            name: "__webview_set_visible",
            func: builtin_set_visible,
        },
        NativeDef {
            name: "__webview_clear_data",
            func: builtin_clear_data,
        },
        NativeDef {
            name: "__webview_url",
            func: builtin_url,
        },
        NativeDef {
            name: "__webview_asset_base",
            func: builtin_asset_base,
        },
    ];

    /// Everything `__webview_open` was asked for, checked up front so the
    /// event loop callback that builds the view has only the OS to fail on.
    struct Spec {
        title: String,
        width: f64,
        height: f64,
        resizable: bool,
        visible: bool,
        url: Option<String>,
        html: Option<String>,
        devtools: bool,
        user_agent: Option<String>,
        init_script: Option<String>,
        autoplay: bool,
        #[cfg_attr(not(windows), allow(dead_code))]
        background_throttling: bool,
        data_dir: Option<PathBuf>,
        assets: Option<(String, PathBuf)>,
    }

    /// An open view: the web engine and the window hosting it. The view is
    /// declared first so it is dropped before its window and its context.
    struct View {
        webview: WebView,
        host: Host,
        _context: WebContext,
    }

    thread_local! {
        static VIEWS: RefCell<HashMap<u64, View>> = RefCell::new(HashMap::new());
        static EVENTS: RefCell<Vec<Value>> = const { RefCell::new(Vec::new()) };
    }

    /// Queues a `{ kind = "webview", webview = id, type = ... }` event.
    fn push(id: u64, kind: &str, mut fields: Vec<(&str, Value)>) {
        fields.insert(0, ("kind", text("webview")));
        fields.insert(1, ("webview", integer(id)));
        fields.insert(2, ("type", text(kind)));
        let event = record(fields);
        EVENTS.with(|events| events.borrow_mut().push(event));
    }

    pub(crate) fn take_events() -> Vec<Value> {
        EVENTS.with(|events| std::mem::take(&mut *events.borrow_mut()))
    }

    /// Runs `body` on view `id`. Natives cannot call back into Neyuki, and
    /// the handlers below never touch the map, so the borrow is never nested.
    fn with_view<T>(
        id: u64,
        body: impl FnOnce(&mut View) -> Result<T, String>,
    ) -> Result<T, String> {
        VIEWS.with(|views| {
            let mut views = views.borrow_mut();
            let view = views
                .get_mut(&id)
                .ok_or_else(|| "webview is closed".to_string())?;
            body(view)
        })
    }

    // OPTIONS ----------------------------------------------------------------

    fn field(options: &Value, key: &str) -> Value {
        match options {
            Value::Table(table) => table.borrow().get_str(key),
            _ => Value::Nil,
        }
    }

    fn opt_string(options: &Value, key: &str) -> Result<Option<String>, String> {
        match field(options, key) {
            Value::Nil => Ok(None),
            Value::String(value) => Ok(Some(value.to_string())),
            other => Err(format!(
                "{} must be a string, got {}",
                key,
                other.type_name()
            )),
        }
    }

    fn opt_bool(options: &Value, key: &str, default: bool) -> Result<bool, String> {
        match field(options, key) {
            Value::Nil => Ok(default),
            Value::Bool(value) => Ok(value),
            other => Err(format!(
                "{} must be a boolean, got {}",
                key,
                other.type_name()
            )),
        }
    }

    fn opt_number(options: &Value, key: &str, default: f64) -> Result<f64, String> {
        match field(options, key) {
            Value::Nil => Ok(default),
            other => {
                let number = crate::vm::libs::args::number(&other)
                    .map_err(|_| format!("{} must be a number", key))?;
                if number.is_finite() {
                    Ok(number)
                } else {
                    Err(format!("{} must be a finite number", key))
                }
            }
        }
    }

    fn parse_spec(options: &Value) -> Result<Spec, String> {
        let assets = match field(options, "assets") {
            Value::Nil => None,
            table @ Value::Table(_) => {
                let dir = opt_string(&table, "dir")?.ok_or("assets.dir must be provided")?;
                let scheme = opt_string(&table, "scheme")?.unwrap_or_else(|| "app".to_string());
                if scheme.is_empty()
                    || !scheme.starts_with(|c: char| c.is_ascii_lowercase())
                    || !scheme
                        .chars()
                        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
                {
                    return Err(
                        "assets.scheme must be lowercase letters, digits and dashes".to_string()
                    );
                }
                Some((scheme, PathBuf::from(dir)))
            }
            other => {
                return Err(format!("assets must be a table, got {}", other.type_name()));
            }
        };
        let spec = Spec {
            title: opt_string(options, "title")?.unwrap_or_else(|| "Neyuki".to_string()),
            width: opt_number(options, "width", 800.0)?,
            height: opt_number(options, "height", 600.0)?,
            resizable: opt_bool(options, "resizable", true)?,
            visible: opt_bool(options, "visible", true)?,
            url: opt_string(options, "url")?,
            html: opt_string(options, "html")?,
            devtools: opt_bool(options, "devtools", false)?,
            user_agent: opt_string(options, "userAgent")?,
            init_script: opt_string(options, "initScript")?,
            autoplay: opt_bool(options, "autoplay", false)?,
            background_throttling: opt_bool(options, "backgroundThrottling", true)?,
            data_dir: opt_string(options, "dataDir")?.map(PathBuf::from),
            assets,
        };
        if spec.width <= 0.0 || spec.height <= 0.0 {
            return Err("width and height must be positive".to_string());
        }
        Ok(spec)
    }

    // ASSETS -----------------------------------------------------------------

    fn percent_decode(path: &str) -> String {
        let bytes = path.as_bytes();
        let mut out = Vec::with_capacity(bytes.len());
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] == b'%'
                && i + 2 < bytes.len()
                && let Ok(hex) = std::str::from_utf8(&bytes[i + 1..i + 3])
                && let Ok(byte) = u8::from_str_radix(hex, 16)
            {
                out.push(byte);
                i += 3;
                continue;
            }
            out.push(bytes[i]);
            i += 1;
        }
        String::from_utf8_lossy(&out).into_owned()
    }

    fn mime_type(path: &Path) -> &'static str {
        let extension = path
            .extension()
            .and_then(|ext| ext.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        match extension.as_str() {
            "html" | "htm" => "text/html; charset=utf-8",
            "js" | "mjs" => "text/javascript; charset=utf-8",
            "css" => "text/css; charset=utf-8",
            "json" | "map" => "application/json",
            "txt" => "text/plain; charset=utf-8",
            "png" => "image/png",
            "jpg" | "jpeg" => "image/jpeg",
            "gif" => "image/gif",
            "webp" => "image/webp",
            "svg" => "image/svg+xml",
            "ico" => "image/x-icon",
            "wasm" => "application/wasm",
            "woff" => "font/woff",
            "woff2" => "font/woff2",
            "ttf" => "font/ttf",
            "otf" => "font/otf",
            "mp3" => "audio/mpeg",
            "ogg" => "audio/ogg",
            "wav" => "audio/wav",
            "mp4" => "video/mp4",
            "webm" => "video/webm",
            _ => "application/octet-stream",
        }
    }

    fn status_response(status: u16) -> Response<Cow<'static, [u8]>> {
        Response::builder()
            .status(status)
            .body(Cow::Borrowed(&b""[..]))
            .unwrap_or_else(|_| Response::new(Cow::Borrowed(&b""[..])))
    }

    /// Answers a request for `uri_path` from the files under `root`. Only
    /// plain path segments are followed, so a page cannot name a file outside
    /// the directory the script chose to publish.
    fn serve_asset(root: &Path, uri_path: &str) -> Response<Cow<'static, [u8]>> {
        let decoded = percent_decode(uri_path);
        let mut path = root.to_path_buf();
        for part in decoded.split('/') {
            if part.is_empty() || part == "." {
                continue;
            }
            if part == ".." || part.contains(['\\', ':', '\0']) {
                return status_response(403);
            }
            path.push(part);
        }
        if path.is_dir() {
            path.push("index.html");
        }
        match std::fs::read(&path) {
            Ok(bytes) => Response::builder()
                .status(200)
                .header("Content-Type", mime_type(&path))
                .body(Cow::Owned(bytes))
                .unwrap_or_else(|_| status_response(500)),
            Err(_) => status_response(404),
        }
    }

    /// Defines `window.neyuki.postMessage` on top of wry's IPC channel.
    const BRIDGE_SCRIPT: &str = "window.neyuki = Object.freeze({ postMessage: function (message) { window.ipc.postMessage(String(message)); } });";

    // BUILDING ---------------------------------------------------------------

    /// Applies `spec` and the event handlers for view `id` to a builder.
    fn configure<'a>(mut builder: WebViewBuilder<'a>, spec: &Spec, id: u64) -> WebViewBuilder<'a> {
        builder = builder
            .with_devtools(spec.devtools)
            .with_autoplay(spec.autoplay)
            .with_ipc_handler(move |request| {
                push(id, "message", vec![("data", text(request.body()))]);
            })
            .with_navigation_handler(move |url| {
                push(id, "navigate", vec![("url", text(&url))]);
                true
            })
            .with_on_page_load_handler(move |event, url| {
                if matches!(event, PageLoadEvent::Finished) {
                    push(id, "load", vec![("url", text(&url))]);
                }
            })
            .with_document_title_changed_handler(move |title| {
                push(id, "title", vec![("title", text(&title))]);
            });
        if let Some(agent) = &spec.user_agent {
            builder = builder.with_user_agent(agent.as_str());
        }
        // `window.neyuki.postMessage(string)` is the page's only way out; it
        // runs before the page and the app's own init script.
        let mut script = String::from(BRIDGE_SCRIPT);
        if let Some(extra) = &spec.init_script {
            script.push('
');
            script.push_str(extra);
        }
        builder = builder.with_initialization_script(script.as_str());
        if let Some((scheme, dir)) = &spec.assets {
            let dir = dir.clone();
            builder = builder.with_custom_protocol(scheme.clone(), move |_, request| {
                serve_asset(&dir, request.uri().path())
            });
        }
        builder = match (&spec.html, &spec.url) {
            (Some(html), _) => builder.with_html(html.as_str()),
            (None, Some(url)) => builder.with_url(url.as_str()),
            (None, None) => builder,
        };
        #[cfg(windows)]
        {
            use wry::WebViewBuilderExtWindows;
            let mut args = vec!["--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection"];
            if spec.autoplay {
                args.push("--autoplay-policy=no-user-gesture-required");
            }
            if !spec.background_throttling {
                args.push("--disable-background-timer-throttling");
                args.push("--disable-renderer-backgrounding");
                args.push("--disable-backgrounding-occluded-windows");
            }
            builder = builder.with_additional_browser_args(args.join(" "));
        }
        builder
    }

    fn build_error(err: impl std::fmt::Display) -> String {
        let hint = if cfg!(windows) {
            " (is the Microsoft Edge WebView2 Runtime installed?)"
        } else if cfg!(target_os = "macos") {
            ""
        } else {
            " (is WebKitGTK installed?)"
        };
        format!("cannot create webview: {}{}", err, hint)
    }

    // WINDOWS AND MACOS: a winit window ----------------------------------------

    #[cfg(any(windows, target_os = "macos"))]
    pub(crate) use host::*;
    #[cfg(any(windows, target_os = "macos"))]
    mod host {
        use std::cell::RefCell;
        use std::collections::HashMap;
        use std::rc::Rc;
        use std::time::Duration;

        use winit::dpi::LogicalSize;
        use winit::event::WindowEvent;
        use winit::event_loop::ActiveEventLoop;
        use winit::window::{Window, WindowId};

        use super::{Spec, VIEWS, View, build_error, configure, push};
        use crate::vm::libs::ui::{OPEN_ATTEMPTS, next_id, pump, tidy_os_error, with_event_loop};

        pub(super) type Host = Rc<Window>;

        thread_local! {
            static WINDOW_IDS: RefCell<HashMap<WindowId, u64>> = RefCell::new(HashMap::new());
            static PENDING: RefCell<Option<Spec>> = const { RefCell::new(None) };
            static OPENED: RefCell<Option<Result<u64, String>>> = const { RefCell::new(None) };
        }

        pub(super) fn toolkit_available() -> bool {
            with_event_loop(|_| ()).is_ok() && wry::webview_version().is_ok()
        }

        /// Asks the event loop for a window and waits for it to be made.
        pub(super) fn open(spec: Spec) -> Result<u64, String> {
            PENDING.with(|pending| *pending.borrow_mut() = Some(spec));
            OPENED.with(|opened| *opened.borrow_mut() = None);
            for _ in 0..OPEN_ATTEMPTS {
                if let Err(err) = pump(Duration::ZERO) {
                    PENDING.with(|pending| *pending.borrow_mut() = None);
                    return Err(err);
                }
                if let Some(result) = OPENED.with(|opened| opened.borrow_mut().take()) {
                    return result;
                }
            }
            PENDING.with(|pending| *pending.borrow_mut() = None);
            Err("cannot create webview: the event loop did not respond".to_string())
        }

        /// Fulfils a parked `__webview_open`, from inside the event loop.
        pub(crate) fn create_pending(event_loop: &ActiveEventLoop) {
            let Some(spec) = PENDING.with(|pending| pending.borrow_mut().take()) else {
                return;
            };
            let result = build(event_loop, spec);
            OPENED.with(|opened| *opened.borrow_mut() = Some(result));
        }

        fn build(event_loop: &ActiveEventLoop, spec: Spec) -> Result<u64, String> {
            let attributes = Window::default_attributes()
                .with_title(&spec.title)
                .with_inner_size(LogicalSize::new(spec.width, spec.height))
                .with_resizable(spec.resizable)
                .with_visible(spec.visible);
            let window = Rc::new(event_loop.create_window(attributes).map_err(|err| {
                format!("cannot create window: {}", tidy_os_error(&err.to_string()))
            })?);
            let id = next_id();
            let mut context = wry::WebContext::new(spec.data_dir.clone());
            let builder = configure(
                wry::WebViewBuilder::new_with_web_context(&mut context),
                &spec,
                id,
            );
            let webview = builder.build(&*window).map_err(build_error)?;
            WINDOW_IDS.with(|ids| ids.borrow_mut().insert(window.id(), id));
            VIEWS.with(|views| {
                views.borrow_mut().insert(
                    id,
                    View {
                        webview,
                        host: window,
                        _context: context,
                    },
                );
            });
            Ok(id)
        }

        /// Reports a close request; the script decides whether to honour it.
        pub(crate) fn window_event(window_id: WindowId, event: &WindowEvent) -> bool {
            let Some(id) = WINDOW_IDS.with(|ids| ids.borrow().get(&window_id).copied()) else {
                return false;
            };
            if matches!(event, WindowEvent::CloseRequested) {
                push(id, "close", vec![]);
            }
            true
        }

        pub(crate) fn pump_toolkit() {}

        pub(super) fn set_title(host: &Host, title: &str) {
            host.set_title(title);
        }

        pub(super) fn set_size(host: &Host, width: f64, height: f64) {
            let _ = host.request_inner_size(LogicalSize::new(width, height));
        }

        pub(super) fn size(host: &Host) -> (f64, f64) {
            let size = host.inner_size().to_logical::<f64>(host.scale_factor());
            (size.width, size.height)
        }

        pub(super) fn set_visible(host: &Host, visible: bool) {
            host.set_visible(visible);
        }

        pub(super) fn destroy(view: View) {
            WINDOW_IDS.with(|ids| ids.borrow_mut().remove(&view.host.id()));
            drop(view);
            // Let the window system process the teardown so the window
            // really disappears even if the script never polls again.
            let _ = pump(Duration::ZERO);
        }
    }

    // LINUX: a GTK window ------------------------------------------------------

    #[cfg(all(unix, not(target_os = "macos")))]
    pub(crate) use host::*;
    #[cfg(all(unix, not(target_os = "macos")))]
    mod host {
        use gtk::prelude::*;
        use winit::event::WindowEvent;
        use winit::event_loop::ActiveEventLoop;
        use winit::window::WindowId;

        use super::{Spec, VIEWS, View, build_error, configure, push};
        use crate::vm::libs::ui::next_id;

        pub(super) type Host = gtk::Window;

        /// GTK may only be initialised on one thread. A second thread (the
        /// test runner uses several) panics inside gtk-rs, which counts as
        /// "not available" there.
        pub(super) fn toolkit_available() -> bool {
            std::panic::catch_unwind(|| gtk::init().is_ok()).unwrap_or(false)
                && wry::webview_version().is_ok()
        }

        pub(super) fn open(spec: Spec) -> Result<u64, String> {
            if !toolkit_available() {
                return Err("no display available: cannot initialise GTK".to_string());
            }
            let window = gtk::Window::new(gtk::WindowType::Toplevel);
            window.set_title(&spec.title);
            window.set_default_size(spec.width as i32, spec.height as i32);
            window.set_resizable(spec.resizable);
            let id = next_id();
            window.connect_delete_event(move |_, _| {
                push(id, "close", vec![]);
                gtk::glib::Propagation::Stop
            });
            let vbox = gtk::Box::new(gtk::Orientation::Vertical, 0);
            window.add(&vbox);
            let mut context = wry::WebContext::new(spec.data_dir.clone());
            let builder = configure(
                wry::WebViewBuilder::new_with_web_context(&mut context),
                &spec,
                id,
            );
            let webview = {
                use wry::WebViewBuilderExtUnix;
                builder.build_gtk(&vbox).map_err(build_error)?
            };
            if spec.visible {
                window.show_all();
            }
            VIEWS.with(|views| {
                views.borrow_mut().insert(
                    id,
                    View {
                        webview,
                        host: window,
                        _context: context,
                    },
                );
            });
            Ok(id)
        }

        pub(crate) fn create_pending(_: &ActiveEventLoop) {}

        pub(crate) fn window_event(_: WindowId, _: &WindowEvent) -> bool {
            false
        }

        /// Runs the GTK events that arrived while `winit` was waiting.
        pub(crate) fn pump_toolkit() {
            if VIEWS.with(|views| views.borrow().is_empty()) {
                return;
            }
            while gtk::events_pending() {
                gtk::main_iteration_do(false);
            }
        }

        pub(super) fn set_title(host: &Host, title: &str) {
            host.set_title(title);
        }

        pub(super) fn set_size(host: &Host, width: f64, height: f64) {
            host.resize(width as i32, height as i32);
        }

        pub(super) fn size(host: &Host) -> (f64, f64) {
            let (width, height) = host.size();
            (width as f64, height as f64)
        }

        pub(super) fn set_visible(host: &Host, visible: bool) {
            if visible {
                host.show_all();
            } else {
                host.hide();
            }
        }

        pub(super) fn destroy(view: View) {
            // SAFETY: the window is owned by this view and nothing else
            // keeps a reference to the destroyed widget.
            unsafe { view.host.destroy() };
            drop(view);
            pump_toolkit();
        }
    }

    // NATIVES ----------------------------------------------------------------

    /// `__webview_available()` reports whether a view can be created here:
    /// a display, and the web engine (WebView2 runtime, WebKitGTK).
    fn builtin_available(_vm: &mut VM, _args: &[Value]) -> Result<Vec<Value>, String> {
        Ok(vec![Value::Bool(toolkit_available())])
    }

    /// `__webview_open(options)` creates a window with a view in it and
    /// returns the id.
    fn builtin_open(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
        let options = args.first().cloned().unwrap_or(Value::Nil);
        if !matches!(options, Value::Table(_)) {
            return Err("options must be a table".to_string());
        }
        let spec = parse_spec(&options)?;
        Ok(vec![integer(open(spec)?)])
    }

    /// `__webview_close(webview)` destroys the view and its window.
    fn builtin_close(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
        let id = integer_arg(args, 0, "webview")?;
        let removed = VIEWS.with(|views| views.borrow_mut().remove(&id));
        if let Some(view) = removed {
            destroy(view);
        }
        Ok(vec![])
    }

    /// `__webview_navigate(webview, url)`.
    fn builtin_navigate(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
        let id = integer_arg(args, 0, "webview")?;
        let url = string_arg(args, 1, "url")?;
        with_view(id, |view| {
            view.webview
                .load_url(&url)
                .map_err(|err| format!("cannot navigate: {}", err))
        })?;
        Ok(vec![])
    }

    /// `__webview_load_html(webview, html)`.
    fn builtin_load_html(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
        let id = integer_arg(args, 0, "webview")?;
        let html = string_arg(args, 1, "html")?;
        with_view(id, |view| {
            view.webview
                .load_html(&html)
                .map_err(|err| format!("cannot load html: {}", err))
        })?;
        Ok(vec![])
    }

    /// `__webview_reload(webview)`.
    fn builtin_reload(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
        let id = integer_arg(args, 0, "webview")?;
        with_view(id, |view| {
            view.webview
                .evaluate_script("location.reload()")
                .map_err(|err| format!("cannot reload: {}", err))
        })?;
        Ok(vec![])
    }

    /// `__webview_eval(webview, js)` runs a script in the page; its result is
    /// not returned (scripts answer through `window.neyuki.postMessage`).
    fn builtin_eval(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
        let id = integer_arg(args, 0, "webview")?;
        let script = string_arg(args, 1, "script")?;
        with_view(id, |view| {
            view.webview
                .evaluate_script(&script)
                .map_err(|err| format!("cannot run script: {}", err))
        })?;
        Ok(vec![])
    }

    /// `__webview_set_title(webview, title)`.
    fn builtin_set_title(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
        let id = integer_arg(args, 0, "webview")?;
        let title = string_arg(args, 1, "title")?;
        with_view(id, |view| {
            set_title(&view.host, &title);
            Ok(())
        })?;
        Ok(vec![])
    }

    /// `__webview_set_size(webview, width, height)` in logical pixels.
    fn builtin_set_size(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
        let id = integer_arg(args, 0, "webview")?;
        let width = float_arg(args, 1, "width")?;
        let height = float_arg(args, 2, "height")?;
        if width <= 0.0 || height <= 0.0 {
            return Err("width and height must be positive".to_string());
        }
        with_view(id, |view| {
            set_size(&view.host, width, height);
            Ok(())
        })?;
        Ok(vec![])
    }

    /// `__webview_size(webview)` returns the logical width and height.
    fn builtin_size(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
        let id = integer_arg(args, 0, "webview")?;
        with_view(id, |view| {
            let (width, height) = size(&view.host);
            Ok(vec![Value::Float(width), Value::Float(height)])
        })
    }

    /// `__webview_set_visible(webview, visible)` shows or hides the window.
    fn builtin_set_visible(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
        let id = integer_arg(args, 0, "webview")?;
        let visible = bool_arg(args, 1, "visible", true)?;
        with_view(id, |view| {
            set_visible(&view.host, visible);
            Ok(())
        })?;
        Ok(vec![])
    }

    /// `__webview_clear_data(webview)` forgets cookies, storage and caches.
    fn builtin_clear_data(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
        let id = integer_arg(args, 0, "webview")?;
        with_view(id, |view| {
            view.webview
                .clear_all_browsing_data()
                .map_err(|err| format!("cannot clear data: {}", err))
        })?;
        Ok(vec![])
    }

    /// `__webview_url(webview)` returns the address the page is at.
    fn builtin_url(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
        let id = integer_arg(args, 0, "webview")?;
        with_view(id, |view| {
            let url = view
                .webview
                .url()
                .map_err(|err| format!("cannot read the address: {}", err))?;
            Ok(vec![text(&url)])
        })
    }

    /// `__webview_asset_base(scheme)` returns the origin pages reach files
    /// from the asset directory at. WebView2 cannot register a real scheme,
    /// so Windows (and Android) serve custom protocols from `http://scheme.localhost`.
    fn builtin_asset_base(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
        let scheme = string_arg(args, 0, "scheme")?;
        let base = if cfg!(windows) {
            format!("http://{}.localhost", scheme)
        } else {
            format!("{}://localhost", scheme)
        };
        Ok(vec![text(&base)])
    }
}
