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

## ui.loadFont(source: string | buffer): Font

Loads a TrueType or OpenType font for `text` and `measureText`, from a file path or from a buffer of its bytes (an app can embed its own CJK font this way). Without one, text uses the bundled Noto Sans. The `Font` has `name` and, when loaded from a path, `path`. `font:close()` frees it and drops it from the fallback list.

## ui.systemFont(family: string, options: table?): Font?

Loads the installed font whose family or PostScript name is `family` (ignoring case), or returns `nil` when there is none. `options.weight` (100 to 900, default 400) and `options.italic` pick the closest face. The installed fonts are scanned on the first call.

## ui.setFallbackFonts(fonts: {Font}?): ()

Sets the fonts every text searches, in order, for characters its own font lacks (kana, kanji, Hangul, emoji). The bundled font is always tried last. An empty list or `nil` clears them. A `fallback` option on a single call replaces this list for that call.

## ui.loadImage(path: string): Image

Decodes a PNG file into an `Image` with `width` and `height`. `image:close()` frees it.

## ui.measureText(s: string, options: table?): (number, number, number)

The width, height and number of lines `window:text` would cover for `s` with the same options (`size`, `font`, `fallback`, `maxWidth`, `overflow`, `lineHeight`, `letterSpacing`), truncation and wrapping included.

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

## Window:text(x: number, y: number, s: string, options: table?): (number, number)

Draws text with the top-left corner of its first line at `(x, y)`; `"
"` starts a new line. Returns the width and height drawn. `options` may hold:

- `size` (default 16), `color` (default black), `opacity` (0 to 1) and `font`.
- `fallback`: fonts to try for characters `font` lacks, instead of the ones from `ui.setFallbackFonts`.
- `maxWidth` with `overflow`: `"wrap"` (default) breaks lines, `"ellipsis"` shortens a line and ends it with `...`, `"clip"` cuts it at the width.
- `align`: `"left"` (default), `"center"` or `"right"`. Within `maxWidth` the text aligns inside it; without it, centered text is centered on `x` and right-aligned text ends at `x`.
- `lineHeight` (a multiple of the size) and `letterSpacing` (pixels).

## Window:drawImage(image: Image, x: number, y: number, options: table?): ()

Draws an image at its own size, or scaled to `options.width` and/or `options.height`.

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
