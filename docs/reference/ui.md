# @neyuki/ui

Windows, 2D drawing, input events and animation on Windows, macOS, X11 and Wayland from one code path. Frames are drawn on the CPU (no GPU or GL needed). The script owns the loop: `ui.poll` returns what happened, drawing calls paint a back buffer and `window:present()` shows it; `ui.run` wraps that loop.

Coordinates are logical pixels (HiDPI scaling is handled underneath). Colors are `"#rgb"`, `"#rrggbb"` or `"#rrggbbaa"` strings, `{ r, g, b, a? }` tables with channels from 0 to 255, or packed `0xRRGGBBAA` integers.

```lua
local ui = require("@neyuki/ui")
local win = ui.open({ title = "Hello", width = 480, height = 320 })
local ball = { x = 60, y = 160 }
ui.animate(ball, { x = 420 }, 1.5, { easing = "inOutSine", ["repeat"] = true, yoyo = true })

ui.run(function(events, dt)
    for _, event in events do
        if event.kind == "keydown" and event.key == "escape" then win:close() end
    end
    win:clear("#202030")
    win:circle(ball.x, ball.y, 24, "#4488ff")
    win:text(20, 20, "Press Escape to quit", { size = 18, color = "#ffffff" })
    win:present()
end)
```

## ui.open(options: table?): Window

Opens a window. `options` may hold `title` (default `"Neyuki"`), `width` and `height` (default 800 × 600), `resizable` (default `true`) and `visible` (default `true`). The canvas starts out white.

## ui.run(handler: (events: {table}, dt: number) -> any, options: table?): ()

Calls `handler(events, dt)` about 60 times a second (`options.fps`) until every window is closed, where `dt` is the seconds since the previous call. Tweens from `ui.animate` are stepped just before each call. Close requests close their window unless `options.autoClose` is `false`. An error in the handler closes the windows and is raised again.

## ui.poll(timeout: number?): {table}

Pumps the event loop and returns the events since the last call. Without a timeout it returns at once; with one it waits up to that many seconds for the first event. Each event has `kind` and `window` plus:

- `close`: the close button or shortcut was used (call `window:close()` to honour it)
- `resize`: `width`, `height`
- `focus`, `blur`, `redraw`, `mouseenter`, `mouseleave`
- `keydown`, `keyup`: `key` (`"a"`, `"escape"`, `"arrowleft"`, `"f1"`), `code` (`"KeyA"`), `repeat`, `text`, `mods` (`shift`, `ctrl`, `alt`, `meta`)
- `mousemove`: `x`, `y`
- `mousedown`, `mouseup`: `button` (`"left"`, `"right"`, `"middle"`), `x`, `y`
- `scroll`: `dx`, `dy`, `unit` (`"lines"` or `"pixels"`), `x`, `y`

## ui.isAvailable(): boolean

Whether a display can be reached. On a headless machine the other window functions raise an error.

## ui.color(color: any): number

Packs any color form into a `0xRRGGBBAA` integer, to avoid re-parsing a color every frame.

## ui.loadFont(path: string): Font

Loads a TrueType or OpenType font for `text` and `measureText`. Without one, text uses the bundled Noto Sans. `font:close()` frees it.

## ui.loadImage(path: string, options: table?): Image

Decodes a PNG, JPEG, WebP, GIF or BMP file into an `Image`. The format is detected from the file's first bytes, not its name. Animated GIF, WebP and PNG files decode every frame.

An `Image` has `width`, `height`, `frames` (1 for stills), `loopCount` (0 plays forever) and `path`. `options.maxPixels` (default 64 million) and `options.maxBytes` (memory for all decoded frames, default 512 MiB) reject oversized images with an error. `image:close()` frees it.

## ui.decodeImage(data: buffer | string, options: table?): Image

Like `loadImage`, from the bytes of an image held in memory, such as an HTTP response body.

## Image:frameDelay(i: number): number

The seconds frame `i` (from 1) stays on screen. Delays of 10 ms or less are read as 100 ms, as browsers do. Stills report 0.

## Image:resized(w: number, h: number): Image

A new image with every frame resampled to `w` x `h` pixels, for shrinking a large animation once instead of scaling it on every draw.

## ui.animation(image: Image, options: table?): ImageAnimation

Plays an animated image from elapsed time. `anim:update(dt)` advances `anim.frame` by the image's frame delays and returns it; `anim:reset()` starts over. `options.speed` scales time (default 1) and `options.loop = true` repeats forever whatever the image's `loopCount`. When the loops are used up, `anim.finished` becomes true on the last frame. `anim.playing` can be set to `false` to pause.

```lua
local anim = ui.animation(img)
anim:update(dt)
win:drawImage(img, 0, 0, { width = w, height = h, fit = "cover", frame = anim.frame })
```

## ui.measureText(s: string, options: table?): (number, number)

The width and height `window:text` would cover for `s`, with the same `size` and `font` options.

## Window:clear(color: any?): ()

Fills the whole canvas (default white).

## Window:fillRect(x: number, y: number, w: number, h: number, color: any, options: table?): ()

A filled rectangle; `options.radius` rounds the corners.

## Window:strokeRect(x: number, y: number, w: number, h: number, color: any, options: table?): ()

A rectangle outline; `options.width` is the line width and `options.radius` rounds the corners.

## Window:line(x1: number, y1: number, x2: number, y2: number, color: any, options: table?): ()

A line; `options.width` is its width.

## Window:circle(cx: number, cy: number, radius: number, color: any, options: table?): ()

A filled circle, or an outline of `options.width` when `options.fill` is `false`.

## Window:polygon(points: {number}, color: any, options: table?): ()

A closed shape through the points of a flat `{ x1, y1, x2, y2, ... }` list, filled unless `options.fill` is `false`.

## Window:text(x: number, y: number, s: string, options: table?): number

Draws text with the top-left corner of its first line at `(x, y)`; `"\n"` starts a new line. `options` may hold `size` (default 16), `color` (default black) and `font`. Returns the width drawn.

## Window:drawImage(image: Image, x: number, y: number, options: table?): ()

Draws an image at its own size, or into the box given by `options.width` and/or `options.height`. More options: `frame` (which frame of an animation, from 1), `opacity` (0 to 1), `radius` (rounds the corners) and `fit`: `"fill"` (default) stretches the image to the box, `"contain"` shows all of it centered, `"cover"` fills the box and crops the overflow around the center. Scaling is smooth, and the scaled copy of a frame is cached, so redrawing it at the same size is cheap.

## Window:present(): ()

Shows everything drawn since the last call.

## Window:size(): (number, number)

The current logical width and height.

## Window:setSize(w: number, h: number): ()

Resizes the window.

## Window:scale(): number

Physical pixels per logical pixel (2 on a typical HiDPI display).

## Window:title(): string

The window title.

## Window:setTitle(title: string): ()

Changes the window title.

## Window:isOpen(): boolean

Whether the window has not been closed.

## Window:close(): ()

Closes the window. Calling it again does nothing.

## ui.tween(target: table, goal: table, duration: number, options: table?): Tween

Animates the fields of `target` toward the values in `goal` over `duration` seconds. Number fields move numerically; any other goal is a color, and the field ends up a packed color. Starting values are read when the tween starts, so a tween chained from `onComplete` continues where the previous one ended.

`options` may hold `easing` (a function or the name of a `ui.easing` curve, default `"outQuad"`), `delay` (seconds), `repeat` (extra runs, or `true` for forever), `yoyo` (alternate runs play backwards), `onUpdate(target)` and `onComplete(target)`.

A `Tween` does nothing until `tween:update(dt)` is called, which returns whether it has ended. It also has `pause()`, `resume()`, `cancel()` (stop in place), `finish()` (jump to the goal and call `onComplete`), `isDone()`, `isPaused()` and `progress()` (0 to 1).

## ui.animate(target: table, goal: table, duration: number, options: table?): Tween

Like `tween`, but registered so `ui.run` (or `ui.update`) steps it until it ends.

```lua
local panel = { y = -200, fill = "#334455" }
ui.animate(panel, { y = 0 }, 0.4, { easing = "outBack" })
ui.animate(panel, { fill = "#5577aa" }, 0.8, { delay = 0.4 })
```

## ui.update(dt: number): number

Steps every tween started with `animate` by `dt` seconds and returns how many are still running. Only needed in a loop written with `poll` instead of `run`.

## ui.easing: table

Easing curves that map progress from 0 to 1: `linear`, `inQuad`, `outQuad`, `inOutQuad`, `inCubic`, `outCubic`, `inOutCubic`, `inSine`, `outSine`, `inOutSine`, `inExpo`, `outExpo`, `inBack`, `outBack`, `outElastic` and `outBounce`.

## ui.lerp(a: number, b: number, t: number): number

The value `t` of the way from `a` to `b`.

## ui.mixColor(from: any, to: any, t: number): number

Mixes two colors channel by channel, alpha included, and returns the packed result.
