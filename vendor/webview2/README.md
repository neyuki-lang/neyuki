# WebView2Loader.dll

`x64/WebView2Loader.dll` is the WebView2 loader from the Microsoft Edge WebView2 SDK, copied unchanged from
the `webview2-com-sys` 0.38.2 crate (which redistributes it). Microsoft allows apps to redistribute it with the
WebView2 SDK.

Only `x86_64-pc-windows-gnu` builds use it. On `windows-gnu`, `webview2-com-sys` imports the loader
from this DLL instead of linking it statically as it does on MSVC. That made `neyuki.exe` fail to start
whenever the DLL was not next to it, even for scripts that never open a web view.
`src/vm/libs/webview2_loader.rs` defines the loader functions itself and loads this embedded copy on first use.

When `wry`/`webview2-com-sys` is upgraded, copy the DLL from the new crate version (`x64/WebView2Loader.dll`).
