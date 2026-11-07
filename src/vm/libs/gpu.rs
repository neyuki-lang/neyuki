//! The GPU surface backing `@neyuki/gpu`.
//!
//! `@neyuki/ui` renders on the CPU with `tiny-skia` and that does not change.
//! This module is a second, separate way to fill a window: a `wgpu` surface
//! on a window opened with `gpu = true`, which `@neyuki/ui` then leaves
//! alone. The two cannot share a window — softbuffer and wgpu would both
//! present to it — so a GPU window has no back buffer and the `ui` drawing
//! calls refuse to touch it.
//!
//! The adapter, device and queue are shared by every GPU window, as surfaces
//! from one instance must be. They live in thread locals, like the windows
//! in `ui.rs`, because the whole UI runs on the event loop thread.
//!
//! The module is behind the `gpu` cargo feature, which is **off** by default
//! (unlike `webview`). Without it `gpu.isAvailable()` returns `false` and
//! every other native fails with a clear error, so a default build links no
//! graphics driver loader at all.
//!
//! wgpu's own environment variables work (`WGPU_BACKEND`, `WGPU_POWER_PREF`,
//! the validation flags), and `NEYUKI_GPU_SOFTWARE=1` adds the one it has
//! none for: take the software adapter (WARP on Windows, lavapipe
//! elsewhere), which is how a machine with no GPU runs the same checks.
//!
//! ## Conventions
//!
//! Fixed now, because scripts will hard-code matrices against them. These
//! are wgpu's own (Direct3D / Metal) conventions, so nothing has to be
//! patched up on the way to the GPU:
//!
//! - **Clip space** (what a vertex shader returns): `x` and `y` in `-1..1`
//!   with **y up**, `z` in **`0..1`** — 0 at the near plane, 1 at the far
//!   plane. Not OpenGL's `-1..1` depth range.
//! - **Framebuffer and texture coordinates**: origin at the **top left**,
//!   y down, so `(0, 0)` is the top-left pixel as in `@neyuki/ui`.
//! - **World and view space**: **right-handed**, y up, looking down `-z`.
//!   The projection matrix does the handedness flip into clip space.
//! - **Matrices**: **column-major**, column-vector convention
//!   (`clip = projection * view * model * position`). A `mat4x4<f32>`
//!   uniform is 16 floats, column after column, which is what WGSL reads
//!   without a transpose.

#[cfg(feature = "gpu")]
pub(crate) use real::{Surface, create_surface, resize};
#[cfg(feature = "gpu")]
pub(crate) const NATIVES: &[crate::vm::value::NativeDef] = real::NATIVES;

#[cfg(not(feature = "gpu"))]
pub(crate) use stub::{Surface, create_surface, resize};
#[cfg(not(feature = "gpu"))]
pub(crate) const NATIVES: &[crate::vm::value::NativeDef] = stub::NATIVES;

/// The error `ui.open` reports for `gpu = true` on a build without the
/// feature, and every native but `__gpu_available` reports always.
#[cfg(not(feature = "gpu"))]
const UNSUPPORTED: &str =
    "this build of Neyuki has no GPU support (built without the `gpu` feature)";

#[cfg(not(feature = "gpu"))]
mod stub {
    use std::sync::Arc;

    use winit::window::Window;

    use crate::vm::machine::VM;
    use crate::vm::value::{NativeDef, Value};

    /// No surface can exist, so this type has no values: the
    /// `Option<Surface>` in `ui::Win` is always `None` and takes no space.
    pub(crate) enum Surface {}

    pub(crate) fn create_surface(_: &Arc<Window>) -> Result<Surface, String> {
        Err(super::UNSUPPORTED.to_string())
    }

    pub(crate) fn resize(surface: &mut Surface, _: u32, _: u32) {
        match *surface {}
    }

    fn available(_vm: &mut VM, _args: &[Value]) -> Result<Vec<Value>, String> {
        Ok(vec![Value::Bool(false)])
    }

    fn unsupported(_vm: &mut VM, _args: &[Value]) -> Result<Vec<Value>, String> {
        Err(super::UNSUPPORTED.to_string())
    }

    pub(crate) const NATIVES: &[NativeDef] = &[
        NativeDef {
            name: "__gpu_available",
            func: available,
        },
        NativeDef {
            name: "__gpu_adapter",
            func: unsupported,
        },
        NativeDef {
            name: "__gpu_clear",
            func: unsupported,
        },
    ];
}

#[cfg(feature = "gpu")]
mod real {
    use std::cell::RefCell;
    use std::sync::Arc;

    use winit::window::Window;

    use crate::vm::libs::ui;
    use crate::vm::machine::VM;
    use crate::vm::value::{NativeDef, Value};

    /// The adapter, device and queue every GPU window shares. Created from
    /// the first `gpu = true` window's surface, so the adapter chosen can
    /// actually present to a window, and then kept for the process.
    struct Context {
        adapter: wgpu::Adapter,
        device: wgpu::Device,
        queue: wgpu::Queue,
    }

    thread_local! {
        /// Surfaces are only valid on the instance that created them, so
        /// there is exactly one, made on first use.
        static INSTANCE: RefCell<Option<wgpu::Instance>> = const { RefCell::new(None) };
        /// `Ok` once a device was created, `Err` once that failed. The error
        /// is cached so a script that retries every frame does not probe the
        /// drivers again each time.
        static CONTEXT: RefCell<Option<Result<Context, String>>> = const { RefCell::new(None) };
    }

    /// A window's swap chain: the surface, and the size it is configured
    /// for, so `resize` can skip reconfiguring when nothing moved.
    pub(crate) struct Surface {
        surface: wgpu::Surface<'static>,
        config: wgpu::SurfaceConfiguration,
    }

    /// `display: None` leaves the display handle to `create_surface`, which
    /// takes it from the window. That is the only arrangement that fits:
    /// `@neyuki/ui` owns the event loop and hands out windows, and a GPU
    /// window may be opened long after the loop started.
    ///
    /// Reading the environment keeps wgpu's own variables working, so
    /// `WGPU_BACKEND=vulkan` (and the debug and validation flags) can pin a
    /// backend without a rebuild. That is how a headless or CI machine
    /// chooses a software driver, and how a bug report can be narrowed to
    /// one backend.
    fn instance_descriptor() -> wgpu::InstanceDescriptor {
        wgpu::InstanceDescriptor::new_without_display_handle_from_env()
    }

    /// The adapter to prefer, overridable with `WGPU_POWER_PREF`. A 3D
    /// workload is what this module is for, so the default is the fastest
    /// adapter rather than the one that saves battery.
    fn power_preference() -> wgpu::PowerPreference {
        wgpu::PowerPreference::from_env().unwrap_or(wgpu::PowerPreference::HighPerformance)
    }

    /// `NEYUKI_GPU_SOFTWARE=1` asks for the software adapter instead of real
    /// hardware: WARP ("Microsoft Basic Render Driver") on Windows, lavapipe
    /// where it is installed. wgpu has no environment variable of its own
    /// for this, and it is what makes a continuous-integration machine with
    /// no GPU able to run the same checks. Anything but `0` turns it on, to
    /// match how wgpu reads its own variables.
    fn force_software_adapter() -> bool {
        std::env::var("NEYUKI_GPU_SOFTWARE").is_ok_and(|value| value != "0")
    }

    /// Runs `body` on the one instance, creating it on first use.
    fn with_instance<T>(body: impl FnOnce(&wgpu::Instance) -> T) -> T {
        INSTANCE.with(|cell| {
            let mut cell = cell.borrow_mut();
            let instance = cell.get_or_insert_with(|| wgpu::Instance::new(instance_descriptor()));
            body(instance)
        })
    }

    /// Picks the adapter and creates the device, blocking until the driver
    /// answers. `compatible` is the surface it has to be able to present to.
    ///
    /// Blocking is deliberate. Both requests are futures, but on every
    /// native backend they resolve on the calling thread without waiting on
    /// I/O, and this runs once, from `ui.open`, before any frame. Making it
    /// task-aware would mean handing the work to a `background` worker,
    /// while the device, queue and surface all belong to the event loop
    /// thread (Metal requires it), so the task would hand the result back
    /// and wait anyway. Nothing called per frame blocks, so `ui.run` pacing
    /// is untouched.
    fn create_context(compatible: &wgpu::Surface<'static>) -> Result<Context, String> {
        let adapter = with_instance(|instance| {
            pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: power_preference(),
                force_fallback_adapter: force_software_adapter(),
                compatible_surface: Some(compatible),
                ..Default::default()
            }))
        })
        .map_err(|err| format!("no graphics adapter is available: {err}"))?;
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("neyuki"),
            ..Default::default()
        }))
        .map_err(|err| format!("cannot create a graphics device: {err}"))?;
        Ok(Context {
            adapter,
            device,
            queue,
        })
    }

    /// Runs `body` on the shared context. `probe` is a surface the adapter
    /// must be able to present to, needed only the first time.
    fn with_context<T>(
        probe: Option<&wgpu::Surface<'static>>,
        body: impl FnOnce(&Context) -> Result<T, String>,
    ) -> Result<T, String> {
        CONTEXT.with(|cell| {
            let mut cell = cell.borrow_mut();
            if cell.is_none() {
                let Some(probe) = probe else {
                    return Err("no GPU window is open".to_string());
                };
                *cell = Some(create_context(probe));
            }
            match cell.as_ref().expect("the context was just created") {
                Ok(context) => body(context),
                Err(err) => Err(err.clone()),
            }
        })
    }

    /// Creates the surface for a window opened with `gpu = true`.
    ///
    /// The window arrives as `Arc<Window>`, not by reference: wgpu's safe
    /// `create_surface` wants a `Send + Sync` handle it can own, and owning
    /// a clone of the `Arc` is what makes the surface sound with no unsafe
    /// lifetime argument. `ui::Win` keeps the same `Arc` and drops the
    /// surface before the window.
    pub(crate) fn create_surface(window: &Arc<Window>) -> Result<Surface, String> {
        let surface = with_instance(|instance| instance.create_surface(Arc::clone(window)))
            .map_err(|err| format!("cannot create a GPU surface for this window: {err}"))?;
        let size = window.inner_size();
        let (width, height) = (size.width.max(1), size.height.max(1));
        let config = with_context(Some(&surface), |context| {
            surface
                .get_default_config(&context.adapter, width, height)
                .ok_or_else(|| "the graphics adapter cannot present to this window".to_string())
        })?;
        let mut surface = Surface { surface, config };
        configure(&mut surface)?;
        Ok(surface)
    }

    fn configure(target: &mut Surface) -> Result<(), String> {
        let Surface { surface, config } = target;
        with_context(None, |context| {
            surface.configure(&context.device, config);
            Ok(())
        })
    }

    /// Reconfigures the swap chain after the window changed size. A failure
    /// is dropped: a resize is not a script call, and the next `clear`
    /// reports whatever is still wrong.
    pub(crate) fn resize(surface: &mut Surface, width: u32, height: u32) {
        let (width, height) = (width.max(1), height.max(1));
        if surface.config.width == width && surface.config.height == height {
            return;
        }
        surface.config.width = width;
        surface.config.height = height;
        let _ = configure(surface);
    }

    /// Why a frame could not be acquired, for the states that are real
    /// failures rather than "try again next frame".
    fn acquire_error(acquired: &wgpu::CurrentSurfaceTexture) -> &'static str {
        use wgpu::CurrentSurfaceTexture as Acquired;
        match acquired {
            Acquired::Validation => "the graphics driver rejected the frame",
            Acquired::Outdated => "the surface is outdated",
            Acquired::Lost => "the surface was lost",
            Acquired::Timeout => "the graphics driver timed out",
            Acquired::Occluded => "the window is not visible",
            Acquired::Success(_) | Acquired::Suboptimal(_) => "no error",
        }
    }

    /// Fills the window with `color` (components in `0..1`) and presents it.
    ///
    /// This is the whole of the spike's drawing: one render pass whose only
    /// operation is the clear. Pipelines, meshes and depth come later.
    fn present(target: &mut Surface, color: [f64; 4]) -> Result<(), String> {
        use wgpu::CurrentSurfaceTexture as Acquired;

        let frame = match target.surface.get_current_texture() {
            Acquired::Success(frame) | Acquired::Suboptimal(frame) => frame,
            // The surface went stale (resized, the display changed, the
            // compositor took it away). Reconfiguring and asking once more
            // is the documented recovery; a second failure is reported.
            Acquired::Outdated | Acquired::Lost => {
                configure(target)?;
                match target.surface.get_current_texture() {
                    Acquired::Success(frame) | Acquired::Suboptimal(frame) => frame,
                    other => {
                        return Err(format!("cannot present window: {}", acquire_error(&other)));
                    }
                }
            }
            // Nothing is wrong and nothing can be drawn: the window is
            // minimized or hidden, or the compositor was busy. Skipping the
            // frame is what wgpu asks for, and a script that clears every
            // frame will simply try again.
            Acquired::Occluded | Acquired::Timeout => return Ok(()),
            other => return Err(format!("cannot present window: {}", acquire_error(&other))),
        };
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        with_context(None, |context| {
            let mut encoder =
                context
                    .device
                    .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                        label: Some("clear"),
                    });
            encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("clear"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: color[0],
                            g: color[1],
                            b: color[2],
                            a: color[3],
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            context.queue.submit([encoder.finish()]);
            // In wgpu 30 the queue presents the frame, not the frame itself.
            // A `SurfaceTexture` that is dropped unpresented discards itself,
            // so this has to happen while the queue is still in hand.
            context.queue.present(frame);
            Ok(())
        })
    }

    /// `__gpu_available()` — whether a window can be opened with
    /// `gpu = true` here: this build has the feature, and the system offers
    /// an adapter. Answered without a window, so before any GPU window
    /// exists it asks for an adapter with no compatible surface.
    fn available(_vm: &mut VM, _args: &[Value]) -> Result<Vec<Value>, String> {
        if CONTEXT.with(|cell| matches!(*cell.borrow(), Some(Ok(_)))) {
            return Ok(vec![Value::Bool(true)]);
        }
        let found = with_instance(|instance| {
            pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: power_preference(),
                force_fallback_adapter: force_software_adapter(),
                ..Default::default()
            }))
        })
        .is_ok();
        Ok(vec![Value::Bool(found)])
    }

    /// `__gpu_adapter()` describes the adapter in use: `name`, `backend`
    /// (`"vulkan"`, `"dx12"`, `"metal"`, `"gl"`), `type` (`"discrete"`,
    /// `"integrated"`, `"cpu"`, ...) and `driver`. Only once a GPU window
    /// exists, because the adapter is chosen for a window.
    fn adapter(_vm: &mut VM, _args: &[Value]) -> Result<Vec<Value>, String> {
        with_context(None, |context| {
            let info = context.adapter.get_info();
            Ok(vec![ui::record(vec![
                ("name", ui::text(&info.name)),
                (
                    "backend",
                    ui::text(match info.backend {
                        wgpu::Backend::Vulkan => "vulkan",
                        wgpu::Backend::Dx12 => "dx12",
                        wgpu::Backend::Metal => "metal",
                        wgpu::Backend::Gl => "gl",
                        wgpu::Backend::Noop => "none",
                        _ => "other",
                    }),
                ),
                (
                    "type",
                    ui::text(match info.device_type {
                        wgpu::DeviceType::DiscreteGpu => "discrete",
                        wgpu::DeviceType::IntegratedGpu => "integrated",
                        wgpu::DeviceType::VirtualGpu => "virtual",
                        wgpu::DeviceType::Cpu => "cpu",
                        wgpu::DeviceType::Other => "other",
                    }),
                ),
                ("driver", ui::text(&info.driver)),
            ])])
        })
    }

    /// `__gpu_clear(window, color)` fills the window with one colour and
    /// presents it. `color` takes the same forms as in `@neyuki/ui`, parsed
    /// by the same code.
    fn clear(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
        let id = ui::integer_arg(args, 0, "window")?;
        let (r, g, b, a) = ui::color_arg(args, 1, "color")?;
        // The surface uses the adapter's preferred format, which is a plain
        // (non-sRGB) 8-bit format on every backend tested, so the channels
        // reach the screen as the script wrote them.
        let color = [
            r as f64 / 255.0,
            g as f64 / 255.0,
            b as f64 / 255.0,
            a as f64 / 255.0,
        ];
        ui::with_window_gpu(id, |surface| present(surface, color))?;
        Ok(vec![])
    }

    pub(crate) const NATIVES: &[NativeDef] = &[
        NativeDef {
            name: "__gpu_available",
            func: available,
        },
        NativeDef {
            name: "__gpu_adapter",
            func: adapter,
        },
        NativeDef {
            name: "__gpu_clear",
            func: clear,
        },
    ];
}
