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

An app that draws its own title bar opens the window without decorations and hands moving and resizing back to the window system, so the window still snaps and crosses monitors like a native one:

```lua
local win = ui.open({ title = "Player", width = 1280, height = 800, decorations = false,
                      minWidth = 900, minHeight = 600, icon = "icon.png" })
local query = ""

ui.run(function(events, dt)
    for _, event in events do
        if event.kind == "mousedown" and event.button == "left" and event.y < 32 then
            if event.clicks == 2 then
                if win:isMaximized() then win:restore() else win:maximize() end
            else
                win:startDrag()
            end
        elseif event.kind == "textinput" then
            query = query .. event.text
        end
    end
    -- ...draw the bar, its buttons and the rest
end)
```

See `examples/window_chrome.nyk` for a complete one with edge resizing, hover cursors and an IME-aware text field.

## ui.open(options: table?): Window

Opens a window. `options` may hold:

- `title` (default `"Neyuki"`), `width` and `height` (default 800 × 600)
- `resizable` and `visible` (default `true`)
- `decorations` (default `true`): `false` removes the native title bar and border, for an app-drawn one
- `minWidth` and `minHeight`: the smallest size the user can resize to
- `icon`: a PNG path or an `Image` for the title bar and taskbar (Windows and X11; macOS and Wayland take the icon from the app bundle or desktop file)
- `transparent` (default `false`): pixels drawn with alpha show what is behind the window, where the platform supports it (Windows and macOS; X11 and Wayland present opaque)
- `shadow` (default `true`): keep the drop shadow of a window without decorations (Windows and macOS)

The canvas starts out white, or clear when `transparent`.

Without decorations, Windows 11 does not show the Snap Layouts flyout when hovering the app's own maximize button; `Win+Z` and dragging to a screen edge still snap.

## ui.run(handler: (events: {table}, dt: number) -> any, options: table?): ()

Calls `handler(events, dt)` about 60 times a second (`options.fps`) until every window is closed, where `dt` is the seconds since the previous call. Tweens from `ui.animate` are stepped just before each call. Close requests close their window unless `options.autoClose` is `false`. An error in the handler closes the windows and is raised again.

## ui.poll(timeout: number?): {table}

Pumps the event loop and returns the events since the last call. Without a timeout it returns at once; with one it waits up to that many seconds for the first event. Each event has `kind` and `window` plus:

- `close`: the close button or shortcut was used (call `window:close()` to honour it)
- `resize`: `width`, `height`
- `focus`, `blur`, `redraw`, `mouseenter`, `mouseleave`
- `keydown`, `keyup`: `key` (`"a"`, `"escape"`, `"arrowleft"`, `"f1"`), `code` (`"KeyA"`), `repeat`, `text`, `mods` (`shift`, `ctrl`, `alt`, `meta`)
- `textinput`: `text`, what was typed or committed by an input method. Use it rather than `keydown.text` for text fields: it skips shortcuts and does not repeat what an input method is composing
- `ime`: `phase` (`"start"`, `"preedit"`, `"commit"` or `"end"`), `text` (the composition so far for `preedit`, the result for `commit`) and `cursor` (bytes of `text` before the caret, or nil when hidden). Only sent after `window:setTextInput(true)`; draw `preedit` text underlined at the caret and drop it on `commit` or `end`
- `mousemove`: `x`, `y`
- `mousedown`: `button` (`"left"`, `"right"`, `"middle"`), `x`, `y`, `clicks` (1, then 2 for a double click, 3 for a triple click: presses of the same button within 500 ms and 4 pixels)
- `mouseup`: `button`, `x`, `y`. After `startDrag` or `startResize` the window system keeps the release, so it may not arrive
- `scroll`: `dx`, `dy`, `unit` (`"lines"` or `"pixels"`), `x`, `y`
- `mediakey`: `key` (`"playpause"`, `"play"`, `"pause"`, `"next"`, `"previous"`, `"stop"`), from a media key, a headset button or the OS media controls; see `ui.mediaSession`
- `mediasession`: `action` (`"seek"` with `position` in seconds or a relative `offset`, `"raise"` or `"quit"`), from the OS media controls
- `maximize`, `minimize`, `restore`: the window changed state, by the user or by a call
- `dropfile`: `path`, a file dropped on the window (one event per file)
- `dpichange`: `scale`, the window moved to a display with another scale factor

## ui.isAvailable(): boolean

Whether a display can be reached. On a headless machine the other window functions raise an error.

## ui.mediaSession(info: table?): (boolean, string?)

Shows what is playing in the OS media controls (the volume flyout and lock screen on Windows, MPRIS applets on Linux, Control Center on macOS) and starts receiving their commands as `mediakey` and `mediasession` events. Once a session exists, the global media keys (keyboards, headsets) reach the app even while it is in the background; without one, they arrive only while a window has focus.

`info` may hold `title`, `artist`, `album`, `artwork` (an `Image`, a file path or an `http(s)` URL), `duration` and `position` in seconds, `playing` (default `false`) and the `window` to attach to (default: the first open one). Each call replaces what is shown; fields left out are cleared. `ui.mediaSession(nil)` ends the session, and closing its window does too. Returns `true`, or `false` and the reason when the OS has no media controls to offer (no D-Bus session bus, say).

```lua
ui.mediaSession({ title = "Second Wind", artist = "The Placeholders", duration = 212, position = 0, playing = true })
for _, event in ui.poll(0.1) do
    if event.kind == "mediakey" and event.key == "playpause" then togglePlayback() end
end
```
## ui.clipboard.get(): string?

The clipboard's text, or nil when it holds something else or nothing. The same clipboard as `@neyuki/hid`.

## ui.clipboard.set(text: string): ()

Replaces the clipboard contents with `text`.

## ui.openUrl(url: string): ()

Opens a link in the default browser or mail client and returns without waiting. Only `http://`, `https://` and `mailto:` links without spaces are accepted; anything else (a file path, `file:`, a custom protocol) raises an error, because the system would happily run a program for it.

## ui.color(color: any): number

Packs any color form into a `0xRRGGBBAA` integer, to avoid re-parsing a color every frame.

## ui.loadFont(path: string): Font

Loads a TrueType or OpenType font for `text` and `measureText`. Without one, text uses the bundled Noto Sans. `font:close()` frees it.

## ui.loadImage(path: string): Image

Decodes a PNG file into an `Image` with `width` and `height`. `image:close()` frees it.

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

## Window:startDrag(): boolean

Moves the window with the mouse as if its title bar were being dragged. Call it on `mousedown` in a custom title bar, while the button is still held; returns whether the window system took over.

## Window:startResize(direction: string): boolean

Resizes the window with the mouse from an edge or corner: `"n"`, `"s"`, `"e"`, `"w"`, `"ne"`, `"nw"`, `"se"` or `"sw"`. Call it on `mousedown` near the edge, like `startDrag`. A window without decorations has no resize border of its own, so this is how one gets resized.

## Window:minimize(): ()

Minimizes the window to the taskbar or dock.

## Window:maximize(): ()

Maximizes the window.

## Window:restore(): ()

Brings a minimized window back, or else returns a maximized one to its normal size.

## Window:isMaximized(): boolean

Whether the window is maximized.

## Window:setFullscreen(fullscreen: boolean): ()

Switches to borderless fullscreen on the window's current monitor, or back.

## Window:setCursor(cursor: string): ()

The pointer shown over the window: `"default"`, `"pointer"` (the hand for links and buttons), `"text"`, `"grab"`, `"grabbing"`, `"crosshair"`, `"move"`, `"notAllowed"`, `"wait"`, `"progress"`, `"help"`, `"copy"`, `"contextMenu"`, `"zoomIn"`, `"zoomOut"`, `"ewResize"`, `"nsResize"`, `"neswResize"`, `"nwseResize"`, `"colResize"`, `"rowResize"`, or `"hidden"`.

## Window:setTextInput(enabled: boolean, area: table?): ()

Turns text input through the input method (for Chinese, Japanese, Korean and other composed text) on or off; `ime` events are only sent while it is on. `area` is the `{ x, y, w, h }` of the text being edited, so the candidate window opens next to it; call it again as the caret moves. Turn it off when the text field loses focus.

## Window:focus(): ()

Brings the window to the front and gives it the keyboard, as far as the window system allows.

## Window:requestAttention(): ()

Flashes the taskbar entry or bounces the dock icon until the window is focused.

## Window:setIcon(icon: any): ()

Sets the title bar and taskbar icon from an `Image` or a PNG path, or clears it with nil. Like the `icon` option, it has no effect on macOS and Wayland.

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
