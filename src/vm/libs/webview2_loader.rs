//! The WebView2 loader for `x86_64-pc-windows-gnu` builds.
//!
//! On MSVC `webview2-com-sys` links the loader statically. On windows-gnu it
//! imports its five functions from `WebView2Loader.dll`, so the executable
//! would not start at all unless that DLL sat next to it, even for scripts
//! that never open a web view. This module defines those functions (and their
//! `__imp_` import slots) itself. The linker then resolves the crate's
//! references here and never pulls in the DLL import. Each function loads the
//! real loader on first use and forwards to it: a copy next to the executable
//! if there is one, else the one embedded below, written once to
//! `%LOCALAPPDATA%\neyuki\webview2\<hash>\`.

#![allow(non_upper_case_globals, non_snake_case, clippy::missing_safety_doc)]

use std::ffi::c_void;
use std::path::PathBuf;
use std::sync::OnceLock;

use sha2::{Digest, Sha256};
use windows_sys::Win32::Foundation::HMODULE;
use windows_sys::Win32::System::LibraryLoader::{
    GetProcAddress, LOAD_LIBRARY_SEARCH_APPLICATION_DIR, LoadLibraryExW,
};

const LOADER: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/vendor/webview2/x64/WebView2Loader.dll"
));

/// HRESULT_FROM_WIN32(ERROR_MOD_NOT_FOUND), returned when the loader cannot be loaded.
const E_MOD_NOT_FOUND: i32 = 0x8007_007Eu32 as i32;
/// HRESULT_FROM_WIN32(ERROR_PROC_NOT_FOUND).
const E_PROC_NOT_FOUND: i32 = 0x8007_007Fu32 as i32;

static MODULE: OnceLock<Result<usize, String>> = OnceLock::new();

/// Loads the WebView2 loader once and reports why it could not be loaded.
pub(crate) fn ensure() -> Result<(), String> {
    module().map(|_| ())
}

fn module() -> Result<HMODULE, String> {
    MODULE
        .get_or_init(|| load().map(|module| module as usize))
        .clone()
        .map(|module| module as HMODULE)
}

fn wide(s: &std::ffi::OsStr) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    s.encode_wide().chain(std::iter::once(0)).collect()
}

fn load() -> Result<HMODULE, String> {
    // A copy shipped next to the executable wins (only that directory is
    // searched, never the current directory).
    let name = wide(std::ffi::OsStr::new("WebView2Loader.dll"));
    let module = unsafe {
        LoadLibraryExW(
            name.as_ptr(),
            std::ptr::null_mut(),
            LOAD_LIBRARY_SEARCH_APPLICATION_DIR,
        )
    };
    if !module.is_null() {
        return Ok(module);
    }
    let path = extract()?;
    let wide_path = wide(path.as_os_str());
    let module = unsafe { LoadLibraryExW(wide_path.as_ptr(), std::ptr::null_mut(), 0) };
    if module.is_null() {
        return Err(format!(
            "cannot load the WebView2 loader from {}: {}",
            path.display(),
            std::io::Error::last_os_error()
        ));
    }
    Ok(module)
}

/// Writes the embedded loader to a per-user cache directory named after its
/// hash, so a different version never overwrites one that is in use.
fn extract() -> Result<PathBuf, String> {
    let base = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("TEMP").map(PathBuf::from))
        .ok_or_else(|| "cannot find a directory for the WebView2 loader".to_string())?;
    let hash = Sha256::digest(LOADER);
    let tag: String = hash[..8].iter().map(|byte| format!("{byte:02x}")).collect();
    let dir = base.join("neyuki").join("webview2").join(tag);
    let path = dir.join("WebView2Loader.dll");
    if std::fs::read(&path).is_ok_and(|existing| existing == LOADER) {
        return Ok(path);
    }
    std::fs::create_dir_all(&dir)
        .map_err(|err| format!("cannot create {}: {}", dir.display(), err))?;
    // Written under a unique name and renamed into place, so a second
    // process never loads a half-written file.
    let partial = dir.join(format!("WebView2Loader.{}.tmp", std::process::id()));
    std::fs::write(&partial, LOADER)
        .map_err(|err| format!("cannot write {}: {}", partial.display(), err))?;
    if let Err(err) = std::fs::rename(&partial, &path) {
        let _ = std::fs::remove_file(&partial);
        // Another process may have put it there (and may have it loaded).
        if !std::fs::read(&path).is_ok_and(|existing| existing == LOADER) {
            return Err(format!("cannot write {}: {}", path.display(), err));
        }
    }
    Ok(path)
}

/// The address of `name` (NUL-terminated) in the loader.
fn proc(name: &[u8]) -> Result<*const c_void, i32> {
    let module = module().map_err(|_| E_MOD_NOT_FOUND)?;
    match unsafe { GetProcAddress(module, name.as_ptr()) } {
        Some(f) => Ok(f as *const c_void),
        None => Err(E_PROC_NOT_FOUND),
    }
}

/// Defines `$name` and its `__imp_` slot, forwarding to the loaded DLL.
macro_rules! forward {
    ($name:ident, $imp:ident, ($($arg:ident: $ty:ty),*)) => {
        #[unsafe(no_mangle)]
        pub unsafe extern "system" fn $name($($arg: $ty),*) -> i32 {
            type F = unsafe extern "system" fn($($ty),*) -> i32;
            match proc(concat!(stringify!($name), "\0").as_bytes()) {
                Ok(f) => unsafe { std::mem::transmute::<*const c_void, F>(f)($($arg),*) },
                Err(hresult) => hresult,
            }
        }

        #[unsafe(no_mangle)]
        #[used]
        pub static $imp: unsafe extern "system" fn($($ty),*) -> i32 = $name;
    };
}

forward!(CompareBrowserVersions, __imp_CompareBrowserVersions,
    (version1: *const u16, version2: *const u16, result: *mut i32));
forward!(CreateCoreWebView2Environment, __imp_CreateCoreWebView2Environment,
    (handler: *mut c_void));
forward!(CreateCoreWebView2EnvironmentWithOptions, __imp_CreateCoreWebView2EnvironmentWithOptions,
    (browser_folder: *const u16, user_data_folder: *const u16, options: *mut c_void, handler: *mut c_void));
forward!(GetAvailableCoreWebView2BrowserVersionString, __imp_GetAvailableCoreWebView2BrowserVersionString,
    (browser_folder: *const u16, version: *mut *mut u16));
forward!(GetAvailableCoreWebView2BrowserVersionStringWithOptions,
    __imp_GetAvailableCoreWebView2BrowserVersionStringWithOptions,
    (browser_folder: *const u16, options: *mut c_void, version: *mut *mut u16));
