# @neyuki/gpu

GPU rendering through the platform graphics API: Direct3D 12 on Windows, Metal on macOS, Vulkan (or OpenGL ES) elsewhere, behind `wgpu`.

This is a separate path from [@neyuki/ui](ui.md), not a backend for it. `ui` keeps rendering on the CPU with `tiny-skia`, with the same guarantees as before: no GPU, no driver, works on a machine with no display driver at all. What `ui` cannot do is depth testing — it has no pixel write and no raw upload path, so everything is painter's-algorithm sorting and interpenetrating geometry always sorts wrong. That capability, plus textures, is what this module exists for.

A window opened with `gpu = true` is handed to this module. It has **no CPU back buffer**: every `ui` drawing call on it, and `window:present()`, raises an error, because softbuffer and wgpu cannot both present to one window. Everything else about the window is unchanged — `ui.poll`, `ui.run`, events, resizing, the title bar and the rest of `ui` work exactly as they do for a CPU window.

The module is behind the `gpu` cargo feature, which is **off** in stock builds, as it adds a large dependency tree and several MiB to the executable. Check `gpu.isAvailable()` first: it is `false` both on a build without the feature and on a machine with no usable adapter.

wgpu's own environment variables apply — `WGPU_BACKEND=vulkan` pins a backend, `WGPU_POWER_PREF=low` prefers the integrated GPU — and `NEYUKI_GPU_SOFTWARE=1` adds the one wgpu has no variable for: use the software adapter (WARP on Windows, lavapipe elsewhere) instead of real hardware. That is how a machine with no GPU runs the same code, slowly but correctly.

> **Status: milestone 1.** A window, a device and a clear colour. Pipelines, shaders, meshes, buffers, textures, uniforms and the depth buffer are not here yet.

```lua
local gpu = require("@neyuki/gpu")
local ui = require("@neyuki/ui")

if not gpu.isAvailable() then return end

local win = ui.open({ width = 640, height = 360, gpu = true })
ui.run(function()
    gpu.clear(win, "#9ac2e8")
    return true
end)
```

## Conventions

Fixed now, before anything uses them, because scripts hard-code matrices against them. They are wgpu's own (Direct3D / Metal) conventions, so nothing is patched up on the way to the GPU:

| | |
| --- | --- |
| Clip space | `x`, `y` in `-1..1`, **y up**; `z` in **`0..1`** (0 near, 1 far) |
| Depth range | `0..1`, **not** OpenGL's `-1..1` |
| Framebuffer and texture coordinates | origin **top left**, y down, as in `@neyuki/ui` |
| World and view space | **right-handed**, y up, looking down `-z` |
| Matrices | **column-major**, column-vector convention: `clip = projection * view * model * position` |

A 4×4 matrix is 16 numbers written column after column, which is what a WGSL `mat4x4<f32>` reads with no transpose.

## gpu.isAvailable(): boolean

Whether a window can be opened with `gpu = true` here: this build has the `gpu` feature and the machine offers an adapter. Every other function raises an error when this is `false`.

## gpu.adapter(): table

The adapter in use:

- `name`, as the driver reports it
- `backend`: `"vulkan"`, `"dx12"`, `"metal"`, `"gl"`
- `type`: `"discrete"`, `"integrated"`, `"cpu"`, `"virtual"`, `"other"`
- `driver`

The adapter is chosen for the first GPU window, so this raises an error until one is open.

## gpu.clear(window, color): ()

Fills `window` with one colour and presents the frame. `window` must have been opened with `gpu = true`. `color` takes the same forms as in `@neyuki/ui`: `"#rgb"`, `"#rrggbb"`, `"#rrggbbaa"`, a `{ r, g, b, a? }` table with channels 0–255, or a packed `0xRRGGBBAA` integer.
