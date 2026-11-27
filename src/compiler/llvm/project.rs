//! Finds every module a project `require`s, so that the whole project
//! compiles into one LLVM module (see `emit_llvm_modules`).
//!
//! The VM resolves `require` while the program runs; a native executable
//! has no compiler and no source files, so the modules are found at build
//! time instead. A call `require("name")` whose argument is a constant
//! string names a module the compiler can find, compile and include. The
//! resolution follows the VM's (`builtin_require`): the same order, the same
//! base directory (the entry script's), the same files.
//!
//! A module that does not exist is a build error. What a native build cannot
//! include yet (`@neyuki/...` standard library modules, native packages with
//! a `[native]` section, `.nykb` bytecode) is a warning, and the `require`
//! fails when the program runs.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use crate::compiler::mir::{
    LocalId, MirBody, MirProgram, MirSpreadSource, Operand, Place, Rvalue, StatementKind,
    TerminatorKind,
};
use crate::compiler::{self, OptLevel};
use crate::module_loader::{Located, ModuleLoader, packages};

/// One module of a project, compiled.
pub struct ProjectModule {
    pub path: PathBuf,
    pub program: MirProgram,
    /// The strings `require` names it by, each as a constant argument
    /// somewhere in the project.
    pub names: Vec<String>,
}

/// The entry script and every module it requires, directly or not. The
/// entry is first.
pub struct Project {
    pub modules: Vec<ProjectModule>,
    /// Things the build could not check, such as a `require` with an
    /// argument that is not a constant string.
    pub warnings: Vec<String>,
}

/// The entry script of what a command line names: the file itself, or for
/// a directory the `entry` of its `neyuki.toml` (`main.nyk` without one),
/// as `neyuki build` finds it.
pub fn entry_of(input: &Path) -> Result<PathBuf, String> {
    if input.is_file() {
        return Ok(input.to_path_buf());
    }
    if !input.is_dir() {
        return Err(format!("{} does not exist", input.display()));
    }
    let manifest_path = input.join("neyuki.toml");
    let entry = if manifest_path.is_file() {
        let text = std::fs::read_to_string(&manifest_path)
            .map_err(|err| format!("cannot read {}: {}", manifest_path.display(), err))?;
        crate::pkg::PackageManifest::parse(&text)
            .map_err(|err| format!("{}: {}", manifest_path.display(), err))?
            .entry
    } else {
        "main.nyk".to_string()
    };
    let relative = crate::app::bundle::normalize(&entry).ok_or_else(|| {
        format!(
            "the entry script '{}' must be a path inside the project",
            entry
        )
    })?;
    let path = input.join(relative);
    if !path.is_file() {
        return Err(format!("entry script {} not found", path.display()));
    }
    Ok(path)
}

/// How a `require` call names its module.
#[derive(Debug, PartialEq, Eq)]
enum Required {
    Constant(String),
    /// Anything but a constant string.
    Dynamic,
}

/// The locals of `body` that hold the global `require` and the constant
/// strings that are assigned once, to read the arguments of its calls.
struct Locals {
    require: HashSet<LocalId>,
    strings: HashMap<LocalId, String>,
}

impl Locals {
    fn of(body: &MirBody) -> Self {
        let mut assigned: HashMap<LocalId, usize> = HashMap::new();
        let mut require = HashSet::new();
        let mut strings = HashMap::new();
        let mut note = |local: LocalId| *assigned.entry(local).or_insert(0) += 1;
        for bb in &body.blocks {
            for stmt in &bb.statements {
                match &stmt.kind {
                    StatementKind::Assign(place, rvalue) if place.projections.is_empty() => {
                        note(place.local);
                        match rvalue {
                            Rvalue::Global(name) if name == "require" => {
                                require.insert(place.local);
                            }
                            Rvalue::Use(Operand::Constant(
                                crate::compiler::mir::MirConst::String(s),
                            )) => {
                                strings.insert(place.local, s.clone());
                            }
                            _ => {}
                        }
                    }
                    StatementKind::VarargMany(locals) => locals.iter().for_each(|l| note(*l)),
                    _ => {}
                }
            }
            match &bb.terminator {
                TerminatorKind::Call { dsts, .. } | TerminatorKind::SpreadCall { dsts, .. } => {
                    dsts.iter().for_each(|l| note(*l));
                }
                TerminatorKind::ForPrep {
                    base,
                    limit,
                    step,
                    loop_var,
                    ..
                } => [base, limit, step, loop_var].iter().for_each(|l| note(**l)),
                TerminatorKind::ForLoop { base, .. } => note(*base),
                TerminatorKind::TForStep { vars, .. } => vars.iter().for_each(|l| note(*l)),
                _ => {}
            }
        }
        // A local assigned more than once may hold anything at the call.
        require.retain(|l| assigned.get(l) == Some(&1));
        strings.retain(|l, _| assigned.get(l) == Some(&1));
        Locals { require, strings }
    }

    fn local(op: &Operand) -> Option<LocalId> {
        match op {
            Operand::Copy(Place { local, projections })
            | Operand::Move(Place { local, projections })
                if projections.is_empty() =>
            {
                Some(*local)
            }
            _ => None,
        }
    }

    /// The module `callee(args...)` requires, when `callee` is `require`.
    fn required(&self, callee: &Operand, args: &[Operand]) -> Option<Required> {
        if !Self::local(callee).is_some_and(|l| self.require.contains(&l)) {
            return None;
        }
        Some(match args.first() {
            Some(Operand::Constant(crate::compiler::mir::MirConst::String(s))) => {
                Required::Constant(s.clone())
            }
            Some(arg) => match Self::local(arg).and_then(|l| self.strings.get(&l)) {
                Some(s) => Required::Constant(s.clone()),
                None => Required::Dynamic,
            },
            // `require()` fails when it runs, whatever it names.
            None => return None,
        })
    }

    fn spread(&self, source: &MirSpreadSource, out: &mut Vec<Required>) {
        if let MirSpreadSource::Call {
            callee,
            args,
            trailing,
        } = source
        {
            out.extend(self.required(callee, args));
            if let Some(trailing) = trailing {
                self.spread(trailing, out);
            }
        }
    }
}

/// Every `require` call in `body` and the functions inside it.
fn scan_requires(body: &MirBody, out: &mut Vec<Required>) {
    let locals = Locals::of(body);
    for bb in &body.blocks {
        match &bb.terminator {
            TerminatorKind::Call { callee, args, .. } => {
                out.extend(locals.required(callee, args));
            }
            TerminatorKind::SpreadCall {
                callee,
                fixed_args,
                trailing,
                ..
            } => {
                out.extend(locals.required(callee, fixed_args));
                locals.spread(trailing, out);
            }
            TerminatorKind::SpreadList { source, .. }
            | TerminatorKind::SpreadReturn { source, .. } => locals.spread(source, out),
            _ => {}
        }
    }
    for child in &body.functions {
        scan_requires(child, out);
    }
}

/// The names of the standard library modules (`json`, `@neyuki/json`, ...):
/// the libraries every VM starts with, and the bundled `lib/*.nyk` ones.
fn stdlib_names() -> HashSet<String> {
    let mut names: HashSet<String> = crate::vm::machine::VM::new()
        .modules
        .keys()
        .cloned()
        .collect();
    names.extend(
        crate::vm::libs::BUNDLED_LIBRARIES
            .iter()
            .filter_map(|(name, _)| name.strip_prefix("@neyuki/"))
            .map(str::to_string),
    );
    names
}

/// What `require(spec)` finds.
enum Resolved {
    File(PathBuf),
    /// Something that exists, which a native build cannot include (yet),
    /// and why.
    Unsupported(String),
}

/// Finds the file `require(spec)` loads, the way the VM does.
fn resolve(spec: &str, base_dir: &Path, stdlib: &HashSet<String>) -> Result<Resolved, String> {
    let bare = spec.strip_prefix("@neyuki/").unwrap_or(spec);
    if stdlib.contains(bare) {
        return Ok(Resolved::Unsupported(format!(
            "require(\"{}\"): standard library modules are not available in native builds yet",
            spec
        )));
    }
    if let Some(moved) = crate::vm::libs::moved_library_error(spec) {
        return Err(moved);
    }
    let path = Path::new(spec);
    if spec.contains('\0')
        || spec.contains("..")
        || spec.starts_with(['/', '\\'])
        || path.is_absolute()
    {
        return Err(format!(
            "security error: path traversal forbidden in require: '{}'",
            spec
        ));
    }
    let located = match packages::parse(spec) {
        Some(request) => packages::locate(&request?, Some(base_dir))?,
        None => ModuleLoader::new()
            .with_base_dir(base_dir)
            .locate_module(spec)
            .map_err(|_| format!("cannot find module '{}'", spec))?,
    };
    match located {
        Located::Disk(file) if file.extension().is_some_and(|ext| ext == "nykb") => {
            Ok(Resolved::Unsupported(format!(
                "require(\"{}\"): precompiled bytecode (.nykb) is not supported in native builds",
                spec
            )))
        }
        Located::Disk(file) => Ok(Resolved::File(file)),
        Located::Native { package, .. } => Ok(Resolved::Unsupported(format!(
            "require(\"{}\"): {} is a native package, which native builds cannot load yet",
            spec, package
        ))),
        Located::Bundled(_) => Err(format!("cannot find module '{}'", spec)),
    }
}

fn compile_file(path: &Path, opt_level: OptLevel) -> Result<MirProgram, String> {
    let source = std::fs::read_to_string(path)
        .map_err(|err| format!("failed to read {}: {}", path.display(), err))?;
    compiler::compile_source_to_mir(
        &source,
        &path.to_string_lossy(),
        compiler::Options::RUN.with_opt_level(opt_level),
    )
}

/// Compiles the entry script and every module reachable from it through
/// `require` with a constant string.
///
/// Like the VM, every module is found from the entry script's directory,
/// whichever file requires it, and one file is one module however it is
/// spelled (`util/x`, `./util/x.nyk`, `util.x`).
pub fn load(entry: &Path, opt_level: OptLevel) -> Result<Project, String> {
    let base_dir = match entry.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent.to_path_buf(),
        _ => PathBuf::from("."),
    };
    let mut modules = vec![ProjectModule {
        path: entry.to_path_buf(),
        program: compile_file(entry, opt_level)?,
        names: Vec::new(),
    }];
    // The entry is not a module `require` can name: requiring its file loads
    // a second copy, as it does in the VM.
    let mut ids: HashMap<PathBuf, usize> = HashMap::new();
    let stdlib = stdlib_names();
    let mut warnings = Vec::new();
    let mut errors = Vec::new();
    let mut next = 0;
    while next < modules.len() {
        let mut found = Vec::new();
        scan_requires(&modules[next].program.main, &mut found);
        let from = modules[next].path.display().to_string();
        let mut dynamic = false;
        for required in found {
            let spec = match required {
                Required::Constant(spec) => spec,
                Required::Dynamic => {
                    dynamic = true;
                    continue;
                }
            };
            let file = match resolve(&spec, &base_dir, &stdlib) {
                Ok(Resolved::File(file)) => file,
                Ok(Resolved::Unsupported(why)) => {
                    let message = format!("{}: {}; it fails when the program runs", from, why);
                    if !warnings.contains(&message) {
                        warnings.push(message);
                    }
                    continue;
                }
                Err(err) => {
                    let message = format!("{}: {}", from, err);
                    if !errors.contains(&message) {
                        errors.push(message);
                    }
                    continue;
                }
            };
            let key = std::fs::canonicalize(&file).unwrap_or_else(|_| file.clone());
            let id = match ids.get(&key) {
                Some(&id) => id,
                None => match compile_file(&file, opt_level) {
                    Ok(program) => {
                        modules.push(ProjectModule {
                            path: file,
                            program,
                            names: Vec::new(),
                        });
                        ids.insert(key, modules.len() - 1);
                        modules.len() - 1
                    }
                    Err(err) => {
                        // Say which file asked for it, as the VM's error does
                        // for the module that fails.
                        errors.push(format!("{}: in module '{}': {}", from, spec, err));
                        continue;
                    }
                },
            };
            if !modules[id].names.contains(&spec) {
                modules[id].names.push(spec);
            }
        }
        if dynamic {
            warnings.push(format!(
                "{}: require() with an argument that is not a constant string; it can only \
                 load a module that is also required by name somewhere in the project, and \
                 fails when the program runs otherwise",
                from
            ));
        }
        next += 1;
    }
    if !errors.is_empty() {
        return Err(errors.join("\n"));
    }
    Ok(Project { modules, warnings })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn requires(source: &str) -> Vec<Required> {
        let program =
            compiler::compile_source_to_mir(source, "t", compiler::Options::RUN).expect("compiles");
        let mut found = Vec::new();
        scan_requires(&program.main, &mut found);
        found
    }

    #[test]
    fn test_scan_finds_constant_and_dynamic_requires_in_nested_functions() {
        let found = requires(
            "\
local a = require(\"a\")
local function f(name)
  local inner = function() return require(\"deep/b\") end
  return require(name), inner
end
local path = \"x\"
path = path .. \"y\"
print(a, f, require(path))",
        );
        let constants: Vec<&str> = found
            .iter()
            .filter_map(|r| match r {
                Required::Constant(s) => Some(s.as_str()),
                Required::Dynamic => None,
            })
            .collect();
        // The optimizer folds `path .. "y"` to the constant "xy".
        assert_eq!(constants, ["a", "xy", "deep/b"]);
        assert_eq!(found.iter().filter(|r| **r == Required::Dynamic).count(), 1);
    }

    #[test]
    fn test_a_local_named_require_is_not_the_builtin() {
        assert!(
            requires("local function require(x) return x end\nprint(require(\"a\"))").is_empty()
        );
    }

    #[test]
    fn test_entry_of_reads_the_manifest() {
        let dir = std::env::temp_dir().join(format!("neyuki-entry-of-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert!(entry_of(&dir).unwrap_err().contains("main.nyk"));
        std::fs::write(dir.join("start.nyk"), "").unwrap();
        std::fs::write(
            dir.join("neyuki.toml"),
            "[package]\nname = \"p\"\nversion = \"0.1.0\"\nentry = \"start.nyk\"\n",
        )
        .unwrap();
        assert_eq!(entry_of(&dir).unwrap(), dir.join("start.nyk"));
        assert_eq!(
            entry_of(&dir.join("start.nyk")).unwrap(),
            dir.join("start.nyk")
        );
        assert!(
            entry_of(&dir.join("gone"))
                .unwrap_err()
                .contains("does not exist")
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
