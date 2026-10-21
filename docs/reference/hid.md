# @neyuki/hid

Keyboard and mouse automation, screen size and the clipboard. Input goes to whichever window has focus, exactly as if a person typed or clicked. It works on Windows, macOS, X11 and Wayland compositors that support the virtual keyboard and pointer protocols (wlroots-based ones such as Sway and Hyprland, and niri); on other Wayland compositors, such as GNOME, it falls back to XWayland and only reaches XWayland windows. macOS asks for accessibility permission the first time.

Keys are single characters (`"a"`, `"7"`, `"/"`) or case-insensitive names: `enter`, `tab`, `space`, `backspace`, `delete`, `escape`, `up`, `down`, `left`, `right`, `home`, `end`, `pageup`, `pagedown`, `shift`, `ctrl`, `alt`, `meta` (also `win`, `cmd`, `super`), `capslock`, `f1` to `f20`, `numpad0` to `numpad9`, `volumeup`, `volumedown`, `mute`, `playpause`, `nexttrack` and `prevtrack`. Mouse buttons are `"left"` (the default), `"right"`, `"middle"`, `"back"` and `"forward"`. Coordinates are pixels from the top-left corner of the main display.

```lua
local hid = require("@neyuki/hid")

local w, h = hid.screen.size()
hid.mouse.glide(w / 2, h / 2)
hid.mouse.click()
hid.keyboard.write("Hello from Neyuki")
hid.keyboard.combo("ctrl+a")
hid.keyboard.combo("ctrl+c")
os.sleep(0.1)
print(hid.clipboard.get())
```

## hid.keyboard.press(key: string): ()

Holds a key down until `release`.

## hid.keyboard.release(key: string): ()

Lets go of a key.

## hid.keyboard.tap(key: string, times: number?): ()

Presses and releases a key `times` times (default once).

## hid.keyboard.write(text: string): ()

Types text, including characters the keyboard layout has no key for.

## hid.keyboard.combo(...: string): ()

Presses the keys in order and releases them in reverse, for shortcuts: `combo("ctrl", "shift", "s")` or `combo("ctrl+shift+s")`.

## hid.keyboard.hold(keys: string, action: () -> any): any

Runs `action` while `keys` (`"shift"` or `"ctrl+alt"`) are held, releases them even if it fails, and returns what it returned.

```lua
hid.keyboard.hold("shift", function()
    hid.keyboard.tap("right", 5) -- select five characters
end)
```

## hid.keyboard.isKey(name: string): boolean

Whether `name` is a key this library understands.

## hid.mouse.move(x: number, y: number): ()

Moves the pointer to a screen position.

## hid.mouse.moveBy(dx: number, dy: number): ()

Moves the pointer by an offset from where it is.

## hid.mouse.glide(x: number, y: number, seconds: number?): ()

Moves the pointer to a position in a straight line over `seconds` (default 0.25), for programs that ignore instant jumps.

## hid.mouse.position(): (number, number)

The pointer position. Wayland does not let programs read it, so there it is where the last `move`, `moveBy` or `glide` put the pointer (it does not see a person moving the mouse), and an error until the pointer has been moved to a position. Until then `glide` and `drag` jump instead of gliding.

## hid.mouse.click(button: string?, count: number?): ()

Clicks a button `count` times (default once; 2 is a double click).

## hid.mouse.press(button: string?): ()

Holds a mouse button down.

## hid.mouse.release(button: string?): ()

Lets go of a mouse button.

## hid.mouse.drag(x: number, y: number, button: string?, seconds: number?): ()

Presses a button where the pointer is, glides to `(x, y)` and releases it.

## hid.mouse.scroll(amount: number, horizontal: boolean?): ()

Scrolls by wheel steps: positive scrolls down (or right with `horizontal`), negative up (or left).

## hid.screen.size(): (number, number)

The width and height of the main display in pixels.

## hid.clipboard.get(): string?

The text on the clipboard, or `nil` when it holds something else or nothing.

## hid.clipboard.set(text: string): ()

Puts text on the clipboard.
