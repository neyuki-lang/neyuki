//! Native filesystem primitives backing `@neyuki/fs`.
//!
//! Every call takes a path, so no file handle outlives a call. `lib/fs.nyk`
//! builds the `File` objects on top of these.

use std::fs::{self, OpenOptions};
use std::io::Write;

use std::time::UNIX_EPOCH;

use crate::vm::libs::args::{array, bool_arg, frozen, str_arg};
use crate::vm::machine::VM;
use crate::vm::value::{NativeDef, Value, VmTable};

pub(crate) const NATIVES: &[NativeDef] = &[
    NativeDef {
        name: "__fs_open",
        func: builtin_open,
    },
    NativeDef {
        name: "__fs_read",
        func: builtin_read,
    },
    NativeDef {
        name: "__fs_write",
        func: builtin_write,
    },
    NativeDef {
        name: "__fs_exists",
        func: builtin_exists,
    },
    NativeDef {
        name: "__fs_remove",
        func: builtin_remove,
    },
    NativeDef {
        name: "__fs_rename",
        func: builtin_rename,
    },
    NativeDef {
        name: "__fs_mkdir",
        func: builtin_mkdir,
    },
    NativeDef {
        name: "__fs_list",
        func: builtin_list,
    },
    NativeDef {
        name: "__fs_isdir",
        func: builtin_isdir,
    },
    NativeDef {
        name: "__fs_copy",
        func: builtin_copy,
    },
    NativeDef {
        name: "__fs_info",
        func: builtin_info,
    },
    NativeDef {
        name: "__fs_cwd",
        func: builtin_cwd,
    },
    NativeDef {
        name: "__fs_absolute",
        func: builtin_absolute,
    },
];

fn io_error(action: &str, path: &str, err: std::io::Error) -> String {
    format!("cannot {} `{}`: {}", action, path, err)
}

/// Checks that `path` can be opened in `mode` and prepares it: "r" requires
/// an existing file, "w" creates or truncates it, "a" creates it if missing.
fn builtin_open(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let path = str_arg(args, 0, "path")?;
    let mode = str_arg(args, 1, "mode")?;
    let result = match mode {
        "r" => fs::File::open(path).map(drop),
        "w" => fs::File::create(path).map(drop),
        "a" => OpenOptions::new()
            .append(true)
            .create(true)
            .open(path)
            .map(drop),
        _ => return Err(format!("invalid open mode `{}` (expected r, w or a)", mode)),
    };
    result.map_err(|err| io_error("open", path, err))?;
    Ok(vec![Value::Bool(true)])
}

/// Files at least this large are read on a worker thread when `read` is
/// called inside a task; smaller ones are not worth the hop.
const BACKGROUND_READ: u64 = 1024 * 1024;

fn builtin_read(vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let path = str_arg(args, 0, "path")?.to_string();
    let large = fs::metadata(&path).is_ok_and(|meta| meta.len() >= BACKGROUND_READ);
    if !large {
        let text = fs::read_to_string(&path).map_err(|err| io_error("read", &path, err))?;
        return Ok(vec![Value::string(text)]);
    }
    // Inside a task only that task waits; see `background::run`.
    crate::vm::libs::background::run(
        vm,
        move || fs::read_to_string(&path).map_err(|err| io_error("read", &path, err)),
        |_vm, text| Ok(vec![Value::string(text)]),
    )
}

fn builtin_write(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let path = str_arg(args, 0, "path")?;
    let text = str_arg(args, 1, "text")?;
    let append = bool_arg(args, 2, "append")?;
    let result = if append {
        OpenOptions::new()
            .append(true)
            .create(true)
            .open(path)
            .and_then(|mut file| file.write_all(text.as_bytes()))
    } else {
        fs::write(path, text)
    };
    result.map_err(|err| io_error("write", path, err))?;
    Ok(vec![Value::Nil])
}

fn builtin_exists(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let path = str_arg(args, 0, "path")?;
    Ok(vec![Value::Bool(fs::metadata(path).is_ok())])
}

/// `(path, recursive)`: removes a file, an empty directory, or with
/// `recursive` a directory and everything in it.
fn builtin_remove(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let path = str_arg(args, 0, "path")?;
    let recursive = bool_arg(args, 1, "recursive")?;
    let metadata = fs::symlink_metadata(path).map_err(|err| io_error("remove", path, err))?;
    let result = if metadata.is_dir() && recursive {
        fs::remove_dir_all(path)
    } else if metadata.is_dir() {
        fs::remove_dir(path)
    } else {
        fs::remove_file(path)
    };
    result.map_err(|err| io_error("remove", path, err))?;
    Ok(vec![Value::Nil])
}

fn builtin_rename(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let from = str_arg(args, 0, "from")?;
    let to = str_arg(args, 1, "to")?;
    fs::rename(from, to).map_err(|err| format!("cannot rename `{}` to `{}`: {}", from, to, err))?;
    Ok(vec![Value::Nil])
}

fn builtin_mkdir(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let path = str_arg(args, 0, "path")?;
    let recursive = bool_arg(args, 1, "recursive")?;
    let result = if recursive {
        fs::create_dir_all(path)
    } else {
        fs::create_dir(path)
    };
    result.map_err(|err| io_error("create directory", path, err))?;
    Ok(vec![Value::Nil])
}

fn builtin_list(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let path = str_arg(args, 0, "path")?;
    let mut names = Vec::new();
    for entry in fs::read_dir(path).map_err(|err| io_error("list", path, err))? {
        let entry = entry.map_err(|err| io_error("list", path, err))?;
        names.push(entry.file_name().to_string_lossy().into_owned());
    }
    names.sort();
    Ok(vec![array(names.into_iter().map(Value::string).collect())])
}

fn builtin_isdir(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let path = str_arg(args, 0, "path")?;
    Ok(vec![Value::Bool(
        fs::metadata(path).is_ok_and(|metadata| metadata.is_dir()),
    )])
}

/// `(from, to)`: copies a file, replacing `to`, and returns the bytes copied.
fn builtin_copy(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let from = str_arg(args, 0, "from")?;
    let to = str_arg(args, 1, "to")?;
    let bytes =
        fs::copy(from, to).map_err(|err| format!("cannot copy `{}` to `{}`: {}", from, to, err))?;
    Ok(vec![Value::Int(bytes as i64)])
}

fn epoch_seconds(time: std::io::Result<std::time::SystemTime>) -> Value {
    time.ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map_or(Value::Nil, |since| Value::Float(since.as_secs_f64()))
}

/// `(path)`: a table describing the file or directory, or nil when nothing
/// is there. Symbolic links are followed.
fn builtin_info(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let path = str_arg(args, 0, "path")?;
    let Ok(metadata) = fs::metadata(path) else {
        return Ok(vec![Value::Nil]);
    };
    let mut table = VmTable::new();
    table.set_str("size", Value::Int(metadata.len() as i64));
    table.set_str("isDir", Value::Bool(metadata.is_dir()));
    table.set_str("isFile", Value::Bool(metadata.is_file()));
    table.set_str("readonly", Value::Bool(metadata.permissions().readonly()));
    table.set_str("modified", epoch_seconds(metadata.modified()));
    table.set_str("created", epoch_seconds(metadata.created()));
    Ok(vec![frozen(table)])
}

fn builtin_cwd(_vm: &mut VM, _args: &[Value]) -> Result<Vec<Value>, String> {
    let dir = std::env::current_dir()
        .map_err(|err| format!("cannot read the working directory: {}", err))?;
    Ok(vec![Value::string(dir.to_string_lossy().into_owned())])
}

/// `(path)`: `path` made absolute against the working directory, without
/// touching the filesystem or resolving links.
fn builtin_absolute(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
    let path = str_arg(args, 0, "path")?;
    let absolute = std::path::absolute(path).map_err(|err| io_error("resolve", path, err))?;
    Ok(vec![Value::string(absolute.to_string_lossy().into_owned())])
}
