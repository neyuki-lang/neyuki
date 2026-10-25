# @neyuki/webview

Web pages inside a window, rendered by the operating system's web engine: WebView2 on Windows, WKWebView on macOS and WebKitGTK on Linux. Pages can come from a URL, an HTML string or a directory of app files, and the page and the script talk through messages.

A view lives in the same event loop as [@neyuki/ui](ui.md): `ui.run` and `ui.poll` pump it, keep running while a view is open and deliver its events, which are tables with `kind = "webview"`:

- `message`: `data`, the string the page passed to `window.neyuki.postMessage`
- `navigate`: `url`, the page is about to go there
- `load`: `url`, the page finished loading
- `title`: `title`
- `close`: the window's close button was used (`ui.run` honours it unless `autoClose` is false)

Every event also carries `webview`, the view it came from. Everything a page sends is untrusted input: `data` is a string to parse defensively. Pages cannot reach files or processes; the only file access is the optional asset directory, served read-only.

The module is behind the `webview` cargo feature, on by default. On Linux it needs `libwebkit2gtk-4.1` and GTK 3 to build and run; build with `--no-default-features` to leave it out, in which case `isAvailable` returns `false` and `open` raises an error. On Windows the WebView2 Evergreen Runtime is required (preinstalled on Windows 11).

```lua
local webview = require("@neyuki/webview")
local ui = require("@neyuki/ui")

local view = webview.open({
    title = "Hello",
    html = "<button onclick=\"window.neyuki.postMessage('hi')\">Say hi</button>",
})
ui.run(function(events)
    for _, event in events do
        if event.kind == "webview" and event.type == "message" then
            print("page said", event.data)
            view:send({ reply = true })
        end
    end
end)
```

## webview.isAvailable(): boolean

Whether a view can be created here: a display and the system web engine.

## webview.open(options: table?): WebView

Opens a window showing a page. `options` may hold:

- `title`, `width`, `height`, `resizable`, `visible`: as for `ui.open`
- `url` or `html`: the first page
- `devtools`: allow the inspector (default `false`)
- `userAgent`: replace the user agent string
- `initScript`: JavaScript that runs before the page's own scripts on every document
- `autoplay`: media may play without a user gesture (default `false`)
- `backgroundThrottling`: set to `false` to keep timers and audio running while hidden (Windows)
- `dataDir`: a directory for a persistent profile, so cookies and storage survive restarts
- `assets`: `{ dir, scheme? }` serves the files under `dir` (cannot escape it) at `webview.assetUrl(scheme, path)`; `scheme` defaults to `"app"`

Pages get `window.neyuki.postMessage(string)` to send messages to the script.

## webview.assetUrl(scheme: string?, path: string?): string

The address of `path` in an asset directory served under `scheme` (default `"app"`). It is `http://app.localhost/...` on Windows and `app://localhost/...` elsewhere, so build it with this function.

## view:navigate(url: string): ()

Loads `url`.

## view:loadHtml(html: string): ()

Replaces the page with `html`.

## view:reload(): ()

Reloads the page.

## view:eval(js: string): ()

Runs JavaScript in the page without waiting. A result comes back by having the script call `window.neyuki.postMessage`.

## view:send(value: any): ()

Sends `value` to the page: a `neyuki` event is fired on `window`, and `event.detail` holds the value as JSON text.

## view:url(): string

The address the page is at.

## view:setTitle(title: string): ()

Sets the window title.

## view:size(): (number, number)

The logical width and height of the window.

## view:setSize(width: number, height: number): ()

Resizes the window.

## view:show(): ()

Shows the window.

## view:hide(): ()

Hides the window. The page keeps running.

## view:clearData(): ()

Forgets cookies, storage and caches (signs the user out).

## view:assetUrl(path: string?): string

`webview.assetUrl` for the scheme this view was opened with.

## view:isOpen(): boolean

Whether the view has not been closed.

## view:close(): ()

Closes the window and the view. Closing twice is fine.
