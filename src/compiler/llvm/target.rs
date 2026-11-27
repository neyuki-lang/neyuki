//! The `target triple` and `target datalayout` of an emitted module.
//!
//! The backend writes textual IR without linking LLVM, so it asks the
//! installed clang (`NEYUKI_CLANG`, or `clang` on `PATH`) what it would use:
//! that is the host's LLVM default, or the one for an explicit `--target`.
//! A module with the same triple as clang's also compiles without clang's
//! "overriding the module target triple" warning. Without clang the module
//! gets the triple alone (the host's, as Rust names it), and LLVM fills in
//! the data layout for that triple itself.

use std::path::PathBuf;
use std::process::{Command, Stdio};

/// What an emitted module is compiled for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TargetSpec {
    pub triple: String,
    pub datalayout: Option<String>,
}

impl TargetSpec {
    /// The module header lines for this target.
    pub fn header(&self) -> String {
        let mut out = String::new();
        if let Some(layout) = &self.datalayout {
            out.push_str(&format!("target datalayout = \"{}\"\n", layout));
        }
        out.push_str(&format!("target triple = \"{}\"\n", self.triple));
        out
    }

    /// `triple` (or the host when `None`), as the installed clang describes
    /// it, falling back to the triple alone without clang.
    pub fn resolve(triple: Option<&str>) -> TargetSpec {
        query_clang(triple).unwrap_or_else(|| TargetSpec {
            triple: triple.map_or_else(host_triple, str::to_string),
            datalayout: None,
        })
    }
}

/// The triple of the host `neyuki` was built for, in Rust's spelling (which
/// LLVM accepts).
fn host_triple() -> String {
    let arch = std::env::consts::ARCH;
    let rest = if cfg!(target_os = "macos") {
        "apple-darwin"
    } else if cfg!(all(target_os = "windows", target_env = "gnu")) {
        "pc-windows-gnu"
    } else if cfg!(target_os = "windows") {
        "pc-windows-msvc"
    } else if cfg!(all(target_os = "linux", target_env = "musl")) {
        "unknown-linux-musl"
    } else if cfg!(target_os = "linux") {
        "unknown-linux-gnu"
    } else {
        return format!("{}-unknown-{}", arch, std::env::consts::OS);
    };
    format!("{}-{}", arch, rest)
}

/// The clang the LLVM backend uses: `NEYUKI_CLANG`, else `clang`.
pub fn clang_path() -> PathBuf {
    std::env::var_os("NEYUKI_CLANG")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("clang"))
}

/// Compiles an empty C file to IR with clang and reads back its target
/// lines.
fn query_clang(triple: Option<&str>) -> Option<TargetSpec> {
    let mut cmd = Command::new(clang_path());
    if let Some(triple) = triple {
        cmd.arg("-target").arg(triple);
    }
    let out = cmd
        .args(["-S", "-emit-llvm", "-x", "c", "-", "-o", "-"])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    parse_target_lines(&String::from_utf8_lossy(&out.stdout))
}

/// The `target triple` and `target datalayout` of a textual module.
fn parse_target_lines(ir: &str) -> Option<TargetSpec> {
    let quoted = |prefix: &str| {
        ir.lines().find_map(|line| {
            let rest = line.strip_prefix(prefix)?.trim();
            Some(rest.strip_prefix('"')?.strip_suffix('"')?.to_string())
        })
    };
    Some(TargetSpec {
        triple: quoted("target triple =")?,
        datalayout: quoted("target datalayout ="),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_target_lines_reads_clang_output() {
        let ir = "; ModuleID = '-'\nsource_filename = \"-\"\n\
                  target datalayout = \"e-m:e-i64:64-n8:16:32:64-S128\"\n\
                  target triple = \"x86_64-pc-linux-gnu\"\n";
        let spec = parse_target_lines(ir).expect("target lines");
        assert_eq!(spec.triple, "x86_64-pc-linux-gnu");
        assert_eq!(
            spec.datalayout.as_deref(),
            Some("e-m:e-i64:64-n8:16:32:64-S128")
        );
        assert_eq!(
            spec.header(),
            "target datalayout = \"e-m:e-i64:64-n8:16:32:64-S128\"\n\
             target triple = \"x86_64-pc-linux-gnu\"\n"
        );
        assert_eq!(parse_target_lines("no target here"), None);
    }
}
