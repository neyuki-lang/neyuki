//! Per-user storage for desktop apps: the directories `os.dataDir`,
//! `os.configDir` and `os.cacheDir` return, and `os.protect`/`os.unprotect`,
//! which seal secrets so that only the same user on the same machine can
//! read them back.
//!
//! Directory resolution is a pure function of the platform and an
//! environment lookup, so every platform's rules are tested on any host.
//!
//! Protected blobs are base64 text whose first byte names the scheme:
//! [`SCHEME_DPAPI`] on Windows, where the bytes are a DPAPI blob, and
//! [`SCHEME_KEYCHAIN`] elsewhere, where they are `nonce || ciphertext || tag`
//! of AES-256-GCM under a random key kept in the OS keychain (Keychain on
//! macOS, the Secret Service on Linux and the BSDs).

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;

use crate::vm::libs::args::str_arg;
use crate::vm::machine::VM;
use crate::vm::value::Value;

/// The operating-system families whose directory conventions differ.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Platform {
    Windows,
    MacOs,
    /// Linux and the BSDs, following the XDG Base Directory specification.
    Xdg,
}

impl Platform {
    pub(super) const CURRENT: Platform = if cfg!(windows) {
        Platform::Windows
    } else if cfg!(target_os = "macos") {
        Platform::MacOs
    } else {
        Platform::Xdg
    };

    fn separator(self) -> char {
        match self {
            Platform::Windows => '\\',
            _ => '/',
        }
    }
}

/// What a directory is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum DirKind {
    Data,
    Config,
    Cache,
}

/// Rejects app names that would not stay a single directory below the base:
/// empty names, `.` and `..`, path separators, and characters that Windows
/// does not allow in file names, so a name that works on one platform works
/// on all of them.
pub(super) fn check_app_name(name: &str) -> Result<(), String> {
    let bad = |why: &str| Err(format!("appName {}, got '{}'", why, name.escape_debug()));
    if name.is_empty() {
        return Err("appName must not be empty".to_string());
    }
    if name == "." || name == ".." {
        return bad("must be a plain directory name");
    }
    if name.chars().any(|c| matches!(c, '/' | '\\')) {
        return bad("must not contain path separators");
    }
    if name
        .chars()
        .any(|c| c.is_control() || matches!(c, '<' | '>' | ':' | '"' | '|' | '?' | '*'))
    {
        return bad("must not contain control characters or any of < > : \" | ? *");
    }
    if name.ends_with(['.', ' ']) || name.starts_with(' ') {
        return bad("must not start with a space or end with a space or dot");
    }
    if name.len() > 255 {
        return Err("appName must be at most 255 bytes".to_string());
    }
    Ok(())
}

/// The directory of `kind` for `app` on `platform`, reading environment
/// variables through `env`. Empty variables count as unset, and XDG
/// variables holding a relative path are ignored, as the specification says.
pub(super) fn app_dir(
    platform: Platform,
    kind: DirKind,
    app: &str,
    env: &dyn Fn(&str) -> Option<String>,
) -> Result<String, String> {
    check_app_name(app)?;
    let sep = platform.separator();
    let var = |name: &str| env(name).filter(|value| !value.is_empty());
    let join = |base: &str, parts: &[&str]| {
        let mut path = base.trim_end_matches(['/', '\\']).to_string();
        for part in parts {
            path.push(sep);
            path.push_str(part);
        }
        path
    };
    let home = || var("HOME").ok_or("cannot find the home directory: HOME is not set");

    match platform {
        Platform::Windows => {
            let (name, fallback) = match kind {
                DirKind::Data | DirKind::Config => ("APPDATA", "Roaming"),
                DirKind::Cache => ("LOCALAPPDATA", "Local"),
            };
            let base = match var(name) {
                Some(base) => base,
                None => match var("USERPROFILE") {
                    Some(profile) => join(&profile, &["AppData", fallback]),
                    None => {
                        return Err(format!(
                            "cannot find the application data directory: {} is not set",
                            name
                        ));
                    }
                },
            };
            Ok(match kind {
                DirKind::Cache => join(&base, &[app, "cache"]),
                _ => join(&base, &[app]),
            })
        }
        Platform::MacOs => {
            let home = home()?;
            Ok(match kind {
                DirKind::Data | DirKind::Config => {
                    join(&home, &["Library", "Application Support", app])
                }
                DirKind::Cache => join(&home, &["Library", "Caches", app]),
            })
        }
        Platform::Xdg => {
            let (name, fallback): (&str, &[&str]) = match kind {
                DirKind::Data => ("XDG_DATA_HOME", &[".local", "share"]),
                DirKind::Config => ("XDG_CONFIG_HOME", &[".config"]),
                DirKind::Cache => ("XDG_CACHE_HOME", &[".cache"]),
            };
            let base = match var(name).filter(|base| base.starts_with('/')) {
                Some(base) => base,
                None => join(&home()?, fallback),
            };
            Ok(join(&base, &[app]))
        }
    }
}

/// Resolves the directory for the current platform and environment and
/// creates it (with any missing parents) so the script can write into it
/// straight away.
fn ensure_dir(kind: DirKind, args: &[Value]) -> Result<Vec<Value>, String> {
    let app = str_arg(args, 0, "appName")?;
    let path = app_dir(Platform::CURRENT, kind, app, &|name| {
        std::env::var(name).ok()
    })?;
    std::fs::create_dir_all(&path)
        .map_err(|err| format!("cannot create directory '{}': {}", path, err))?;
    Ok(vec![Value::string(path)])
}

/// `dataDir(appName)`: where an app keeps the files it owns.
pub(super) fn os_data_dir(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    ensure_dir(DirKind::Data, args)
}

/// `configDir(appName)`: where an app keeps its settings.
pub(super) fn os_config_dir(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    ensure_dir(DirKind::Config, args)
}

/// `cacheDir(appName)`: where an app keeps files it can rebuild.
pub(super) fn os_cache_dir(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    ensure_dir(DirKind::Cache, args)
}

// ---------------------------------------------------------------------------
// Secret storage
// ---------------------------------------------------------------------------

/// A DPAPI blob sealed for the current Windows user.
const SCHEME_DPAPI: u8 = 1;
/// AES-256-GCM under the key kept in the OS keychain.
const SCHEME_KEYCHAIN: u8 = 2;

/// Where the keychain key is filed, so users can find or remove it.
#[cfg(not(windows))]
const KEYCHAIN_SERVICE: &str = "neyuki";
#[cfg(not(windows))]
const KEYCHAIN_ACCOUNT: &str = "os.protect";

/// Seals `data` with this platform's scheme and returns the tagged blob.
fn protect_bytes(data: &[u8], _account: &str) -> Result<Vec<u8>, String> {
    #[cfg(windows)]
    {
        let mut blob = vec![SCHEME_DPAPI];
        blob.extend(dpapi::protect(data)?);
        Ok(blob)
    }
    #[cfg(not(windows))]
    {
        keychain::seal(_account, data)
    }
}

/// Opens a tagged blob from [`protect_bytes`].
fn unprotect_bytes(blob: &[u8], _account: &str) -> Result<Vec<u8>, String> {
    match blob.split_first() {
        #[cfg(windows)]
        Some((&SCHEME_DPAPI, rest)) => dpapi::unprotect(rest),
        #[cfg(not(windows))]
        Some((&SCHEME_KEYCHAIN, _)) => keychain::open(_account, blob),
        #[cfg(not(windows))]
        Some((&SCHEME_DPAPI, _)) => {
            Err("blob was protected with Windows DPAPI and can only be read on Windows".to_string())
        }
        #[cfg(windows)]
        Some((&SCHEME_KEYCHAIN, _)) => Err(
            "blob was protected with a macOS or Linux keychain and can only be read there"
                .to_string(),
        ),
        _ => Err("blob is not a value returned by os.protect".to_string()),
    }
}

/// `protect(data)`: seals a string so only the current user on this machine
/// can read it back with `unprotect`. Returns base64 text.
pub(super) fn os_protect(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let data = str_arg(args, 0, "data")?;
    #[cfg(windows)]
    let account = "";
    #[cfg(not(windows))]
    let account = KEYCHAIN_ACCOUNT;
    let blob = protect_bytes(data.as_bytes(), account)?;
    Ok(vec![Value::string(BASE64.encode(blob))])
}

/// `unprotect(blob)`: the string `protect` sealed into `blob`.
pub(super) fn os_unprotect(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let blob = str_arg(args, 0, "blob")?;
    let blob = BASE64
        .decode(blob.trim())
        .map_err(|_| "blob is not a value returned by os.protect".to_string())?;
    #[cfg(windows)]
    let account = "";
    #[cfg(not(windows))]
    let account = KEYCHAIN_ACCOUNT;
    let data = unprotect_bytes(&blob, account)?;
    let text = String::from_utf8(data)
        .map_err(|_| "blob is not a value returned by os.protect".to_string())?;
    Ok(vec![Value::string(text)])
}

#[cfg(windows)]
mod dpapi {
    //! The Windows Data Protection API, scoped to the current user.

    use windows_sys::Win32::Foundation::{BOOL, LocalFree};
    use windows_sys::Win32::Security::Cryptography::{
        CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptProtectData, CryptUnprotectData,
    };

    /// Runs one DPAPI call over `input` and copies out the blob it allocates.
    fn call(
        input: &[u8],
        what: &str,
        f: impl FnOnce(*const CRYPT_INTEGER_BLOB, *mut CRYPT_INTEGER_BLOB) -> BOOL,
    ) -> Result<Vec<u8>, String> {
        let len = u32::try_from(input.len()).map_err(|_| "data is too large".to_string())?;
        let input = CRYPT_INTEGER_BLOB {
            cbData: len,
            pbData: input.as_ptr() as *mut u8,
        };
        let mut output = CRYPT_INTEGER_BLOB {
            cbData: 0,
            pbData: std::ptr::null_mut(),
        };
        if f(&input, &mut output) == 0 || output.pbData.is_null() {
            return Err(format!("{}: {}", what, std::io::Error::last_os_error()));
        }
        // SAFETY: on success DPAPI sets `output` to a LocalAlloc'd buffer of
        // `cbData` bytes, which is copied and then freed exactly once.
        let bytes = unsafe {
            let bytes = std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec();
            LocalFree(output.pbData as _);
            bytes
        };
        Ok(bytes)
    }

    pub(super) fn protect(data: &[u8]) -> Result<Vec<u8>, String> {
        call(data, "cannot protect data", |input, output| {
            // SAFETY: `input` points at a blob whose bytes outlive the call
            // (DPAPI only reads them), `output` is a valid out-pointer, and
            // the optional arguments are null, which DPAPI accepts.
            unsafe {
                CryptProtectData(
                    input,
                    std::ptr::null(),
                    std::ptr::null(),
                    std::ptr::null(),
                    std::ptr::null(),
                    CRYPTPROTECT_UI_FORBIDDEN,
                    output,
                )
            }
        })
    }

    pub(super) fn unprotect(blob: &[u8]) -> Result<Vec<u8>, String> {
        call(
            blob,
            "cannot unprotect blob (it was protected by another user or machine, or is damaged)",
            |input, output| {
                // SAFETY: as in `protect`; the description out-pointer is
                // null, so DPAPI allocates nothing besides `output`.
                unsafe {
                    CryptUnprotectData(
                        input,
                        std::ptr::null_mut(),
                        std::ptr::null(),
                        std::ptr::null(),
                        std::ptr::null(),
                        CRYPTPROTECT_UI_FORBIDDEN,
                        output,
                    )
                }
            },
        )
    }
}

#[cfg(not(windows))]
mod keychain {
    //! AES-256-GCM under a random key that lives in the OS keychain. The key
    //! is created on first use and cached for the rest of the process.

    use std::collections::HashMap;
    use std::sync::Mutex;

    use aes_gcm::Aes256Gcm;
    use aes_gcm::aead::{Aead, KeyInit, Nonce, Payload};
    use rand::RngCore;
    use rand::rngs::OsRng;

    use super::{KEYCHAIN_SERVICE, SCHEME_KEYCHAIN};

    const NONCE_LEN: usize = 12;

    static KEYS: Mutex<Option<HashMap<String, [u8; 32]>>> = Mutex::new(None);

    /// Explains a keychain failure, pointing headless Linux users at the
    /// missing piece.
    fn describe(err: keyring::Error) -> String {
        match err {
            keyring::Error::NoStorageAccess(_) | keyring::Error::PlatformFailure(_) => format!(
                "no OS keychain is available: {}{}",
                err,
                if cfg!(target_os = "macos") {
                    ""
                } else {
                    " (os.protect needs a Secret Service provider such as GNOME Keyring or \
                     KWallet running in the desktop session)"
                }
            ),
            other => format!("keychain error: {}", other),
        }
    }

    /// The key filed under `account`, created and stored when `create` is
    /// set and there is none yet.
    pub(super) fn key(account: &str, create: bool) -> Result<[u8; 32], String> {
        let mut cache = KEYS.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(key) = cache.as_ref().and_then(|keys| keys.get(account)) {
            return Ok(*key);
        }
        let entry = keyring::Entry::new(KEYCHAIN_SERVICE, account).map_err(describe)?;
        let stored = match entry.get_secret() {
            Ok(bytes) => bytes,
            Err(keyring::Error::NoEntry) if create => {
                let mut fresh = [0u8; 32];
                OsRng.fill_bytes(&mut fresh);
                entry.set_secret(&fresh).map_err(describe)?;
                // Read it back: if another process stored a key at the same
                // moment, whichever write won is the key everyone uses.
                entry.get_secret().map_err(describe)?
            }
            Err(keyring::Error::NoEntry) => {
                return Err(format!(
                    "cannot unprotect blob: the keychain has no '{}' key for '{}' (it was \
                     protected by another user or machine, or the key was deleted)",
                    account, KEYCHAIN_SERVICE
                ));
            }
            Err(err) => return Err(describe(err)),
        };
        let key: [u8; 32] = stored.try_into().map_err(|_| {
            format!(
                "the keychain's '{}' key for '{}' is damaged",
                account, KEYCHAIN_SERVICE
            )
        })?;
        cache
            .get_or_insert_with(HashMap::new)
            .insert(account.to_string(), key);
        Ok(key)
    }

    pub(super) fn seal(account: &str, data: &[u8]) -> Result<Vec<u8>, String> {
        let key = key(account, true)?;
        let cipher = Aes256Gcm::new_from_slice(&key).map_err(|_| "cannot protect data")?;
        let mut nonce = Nonce::<Aes256Gcm>::default();
        OsRng.fill_bytes(nonce.as_mut_slice());
        let header = [SCHEME_KEYCHAIN];
        let sealed = cipher
            .encrypt(
                &nonce,
                Payload {
                    msg: data,
                    aad: &header,
                },
            )
            .map_err(|_| "cannot protect data".to_string())?;
        let mut blob = Vec::with_capacity(1 + NONCE_LEN + sealed.len());
        blob.extend_from_slice(&header);
        blob.extend_from_slice(&nonce);
        blob.extend(sealed);
        Ok(blob)
    }

    pub(super) fn open(account: &str, blob: &[u8]) -> Result<Vec<u8>, String> {
        if blob.len() < 1 + NONCE_LEN + 16 {
            return Err("blob is not a value returned by os.protect".to_string());
        }
        let key = key(account, false)?;
        let cipher = Aes256Gcm::new_from_slice(&key).map_err(|_| "cannot unprotect blob")?;
        let (header, rest) = blob.split_at(1);
        let (nonce, sealed) = rest.split_at(NONCE_LEN);
        cipher
            .decrypt(
                Nonce::<Aes256Gcm>::from_slice(nonce),
                Payload {
                    msg: sealed,
                    aad: header,
                },
            )
            .map_err(|_| {
                "cannot unprotect blob (it was protected by another user or machine, or is \
                 damaged)"
                    .to_string()
            })
    }

    /// Removes the key filed under `account` (for tests).
    #[cfg(test)]
    pub(super) fn forget(account: &str) {
        if let Ok(entry) = keyring::Entry::new(KEYCHAIN_SERVICE, account) {
            let _ = entry.delete_credential();
        }
        let mut cache = KEYS.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(keys) = cache.as_mut() {
            keys.remove(account);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env<'a>(vars: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        move |name| {
            vars.iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| value.to_string())
        }
    }

    #[test]
    fn windows_dirs() {
        let vars = env(&[
            ("APPDATA", r"C:\Users\yuki\AppData\Roaming"),
            ("LOCALAPPDATA", r"C:\Users\yuki\AppData\Local\"),
        ]);
        let dir = |kind| app_dir(Platform::Windows, kind, "Notes", &vars).unwrap();
        assert_eq!(dir(DirKind::Data), r"C:\Users\yuki\AppData\Roaming\Notes");
        assert_eq!(dir(DirKind::Config), r"C:\Users\yuki\AppData\Roaming\Notes");
        assert_eq!(
            dir(DirKind::Cache),
            r"C:\Users\yuki\AppData\Local\Notes\cache"
        );

        let profile = env(&[("USERPROFILE", r"C:\Users\yuki"), ("APPDATA", "")]);
        assert_eq!(
            app_dir(Platform::Windows, DirKind::Cache, "Notes", &profile).unwrap(),
            r"C:\Users\yuki\AppData\Local\Notes\cache"
        );
        assert_eq!(
            app_dir(Platform::Windows, DirKind::Data, "Notes", &profile).unwrap(),
            r"C:\Users\yuki\AppData\Roaming\Notes"
        );
        assert!(app_dir(Platform::Windows, DirKind::Data, "Notes", &env(&[])).is_err());
    }

    #[test]
    fn macos_dirs() {
        let vars = env(&[("HOME", "/Users/yuki"), ("XDG_CONFIG_HOME", "/elsewhere")]);
        let dir = |kind| app_dir(Platform::MacOs, kind, "Notes", &vars).unwrap();
        assert_eq!(
            dir(DirKind::Data),
            "/Users/yuki/Library/Application Support/Notes"
        );
        assert_eq!(
            dir(DirKind::Config),
            "/Users/yuki/Library/Application Support/Notes"
        );
        assert_eq!(dir(DirKind::Cache), "/Users/yuki/Library/Caches/Notes");
        assert!(app_dir(Platform::MacOs, DirKind::Data, "Notes", &env(&[])).is_err());
    }

    #[test]
    fn xdg_dirs() {
        let home = env(&[("HOME", "/home/yuki/")]);
        let dir = |kind| app_dir(Platform::Xdg, kind, "notes", &home).unwrap();
        assert_eq!(dir(DirKind::Data), "/home/yuki/.local/share/notes");
        assert_eq!(dir(DirKind::Config), "/home/yuki/.config/notes");
        assert_eq!(dir(DirKind::Cache), "/home/yuki/.cache/notes");

        let vars = env(&[
            ("HOME", "/home/yuki"),
            ("XDG_DATA_HOME", "/data"),
            ("XDG_CONFIG_HOME", "/cfg/"),
            // Relative paths are ignored, as the specification says.
            ("XDG_CACHE_HOME", "relative/cache"),
        ]);
        let dir = |kind| app_dir(Platform::Xdg, kind, "notes", &vars).unwrap();
        assert_eq!(dir(DirKind::Data), "/data/notes");
        assert_eq!(dir(DirKind::Config), "/cfg/notes");
        assert_eq!(dir(DirKind::Cache), "/home/yuki/.cache/notes");

        // An XDG variable is enough without HOME; the fallback needs it.
        let only_xdg = env(&[("XDG_CONFIG_HOME", "/cfg")]);
        assert_eq!(
            app_dir(Platform::Xdg, DirKind::Config, "notes", &only_xdg).unwrap(),
            "/cfg/notes"
        );
        assert!(app_dir(Platform::Xdg, DirKind::Cache, "notes", &only_xdg).is_err());
    }

    #[test]
    fn app_names() {
        for good in ["notes", "My App", "com.example.notes", "app-2", "ノート"] {
            assert!(check_app_name(good).is_ok(), "{good}");
        }
        for bad in [
            "",
            ".",
            "..",
            "a/b",
            r"a\b",
            "../x",
            "C:",
            "a*",
            "nul\0",
            "tab\there",
            "dot.",
            "space ",
            " lead",
        ] {
            assert!(check_app_name(bad).is_err(), "{bad:?}");
        }
        assert!(check_app_name(&"a".repeat(256)).is_err());
        let vars = env(&[("HOME", "/home/yuki")]);
        assert!(app_dir(Platform::Xdg, DirKind::Data, "..", &vars).is_err());
    }

    #[test]
    fn creates_the_directory() {
        let base = std::env::temp_dir().join(format!("neyuki-os-dirs-{}", std::process::id()));
        let base_str = base.to_string_lossy().into_owned();
        let vars = [
            ("APPDATA", base_str.as_str()),
            ("HOME", base_str.as_str()),
            ("XDG_DATA_HOME", base_str.as_str()),
        ];
        let path = app_dir(Platform::CURRENT, DirKind::Data, "created", &env(&vars)).unwrap();
        std::fs::create_dir_all(&path).unwrap();
        assert!(std::path::Path::new(&path).is_dir());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn rejects_foreign_blobs() {
        assert!(unprotect_bytes(&[], "unused").is_err());
        assert!(unprotect_bytes(&[9, 1, 2, 3], "unused").is_err());
        #[cfg(windows)]
        assert!(unprotect_bytes(&[SCHEME_KEYCHAIN, 0, 0], "unused").is_err());
        #[cfg(not(windows))]
        assert!(unprotect_bytes(&[SCHEME_DPAPI, 0, 0], "unused").is_err());
    }

    /// Round-trips through the platform's secret store. Where there is none
    /// (Linux without a Secret Service, as on most CI machines) the test
    /// reports that and passes.
    #[test]
    fn protect_round_trip() {
        let account = "os.protect test";
        let secret = "session=abc123; héllo";
        let blob = match protect_bytes(secret.as_bytes(), account) {
            Ok(blob) => blob,
            Err(err) if err.starts_with("no OS keychain") || err.starts_with("keychain error") => {
                eprintln!("skipping protect_round_trip: {err}");
                return;
            }
            Err(err) => panic!("protect failed: {err}"),
        };
        assert!(!blob.windows(secret.len()).any(|w| w == secret.as_bytes()));
        assert_eq!(unprotect_bytes(&blob, account).unwrap(), secret.as_bytes());
        let mut damaged = blob.clone();
        let last = damaged.len() - 1;
        damaged[last] ^= 1;
        assert!(unprotect_bytes(&damaged, account).is_err());
        assert_eq!(
            unprotect_bytes(&protect_bytes(b"", account).unwrap(), account).unwrap(),
            b""
        );
        #[cfg(not(windows))]
        keychain::forget(account);
    }
}
