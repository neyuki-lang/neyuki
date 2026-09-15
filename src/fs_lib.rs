//! Native filesystem primitives backing `@neyuki/fs`.
//!
//! The natives are deliberately thin and stateless: every call takes a path,
//! so file handles never outlive the runtime. `lib/fs.nyk` builds the `File`
//! objects and their methods on top of these.

use std::fs::{self, OpenOptions};
use std::io::Write;

use crate::runtime::{Value, new_table};

pub(crate) const NATIVES: &[(&str, crate::runtime::Native)] = &[
    ("__fs_open", builtin_open),
    ("__fs_read", builtin_read),
    ("__fs_write", builtin_write),
    ("__fs_exists", builtin_exists),
    ("__fs_remove", builtin_remove),
    ("__fs_rename", builtin_rename),
    ("__fs_mkdir", builtin_mkdir),
    ("__fs_list", builtin_list),
    ("__fs_isdir", builtin_isdir),
];

fn string_arg(args: &[Value], index: usize, name: &str) -> Result<String, String> {
    match args.get(index) {
        Some(Value::String(value)) => Ok(value.clone()),
        Some(value) => Err(format!(
            "{} must be a string, got {}",
            name,
            value.type_name()
        )),
        None => Err(format!("{} must be provided", name)),
    }
}

fn bool_arg(args: &[Value], index: usize, name: &str) -> Result<bool, String> {
    match args.get(index) {
        Some(Value::Bool(value)) => Ok(*value),
        None | Some(Value::Nil) => Ok(false),
        Some(value) => Err(format!(
            "{} must be a boolean, got {}",
            name,
            value.type_name()
        )),
    }
}

fn io_error(action: &str, path: &str, err: std::io::Error) -> String {
    format!("cannot {} `{}`: {}", action, path, err)
}

/// Checks that `path` can be opened in `mode` and prepares it: "r" requires
/// an existing file, "w" creates or truncates it, "a" creates it if missing.
fn builtin_open(args: Vec<Value>) -> Result<Vec<Value>, String> {
    let path = string_arg(&args, 0, "path")?;
    let mode = string_arg(&args, 1, "mode")?;
    let result = match mode.as_str() {
        "r" => fs::File::open(&path).map(drop),
        "w" => fs::File::create(&path).map(drop),
        "a" => OpenOptions::new()
            .append(true)
            .create(true)
            .open(&path)
            .map(drop),
        _ => return Err(format!("invalid open mode `{}` (expected r, w or a)", mode)),
    };
    result.map_err(|err| io_error("open", &path, err))?;
    Ok(vec![Value::Bool(true)])
}

fn builtin_read(args: Vec<Value>) -> Result<Vec<Value>, String> {
    let path = string_arg(&args, 0, "path")?;
    let text = fs::read_to_string(&path).map_err(|err| io_error("read", &path, err))?;
    Ok(vec![Value::String(text)])
}

fn builtin_write(args: Vec<Value>) -> Result<Vec<Value>, String> {
    let path = string_arg(&args, 0, "path")?;
    let text = string_arg(&args, 1, "text")?;
    let append = bool_arg(&args, 2, "append")?;
    let result = if append {
        OpenOptions::new()
            .append(true)
            .create(true)
            .open(&path)
            .and_then(|mut file| file.write_all(text.as_bytes()))
    } else {
        fs::write(&path, text)
    };
    result.map_err(|err| io_error("write", &path, err))?;
    Ok(vec![Value::Nil])
}

fn builtin_exists(args: Vec<Value>) -> Result<Vec<Value>, String> {
    let path = string_arg(&args, 0, "path")?;
    Ok(vec![Value::Bool(fs::metadata(path).is_ok())])
}

fn builtin_remove(args: Vec<Value>) -> Result<Vec<Value>, String> {
    let path = string_arg(&args, 0, "path")?;
    let metadata = fs::metadata(&path).map_err(|err| io_error("remove", &path, err))?;
    let result = if metadata.is_dir() {
        fs::remove_dir(&path)
    } else {
        fs::remove_file(&path)
    };
    result.map_err(|err| io_error("remove", &path, err))?;
    Ok(vec![Value::Nil])
}

fn builtin_rename(args: Vec<Value>) -> Result<Vec<Value>, String> {
    let from = string_arg(&args, 0, "from")?;
    let to = string_arg(&args, 1, "to")?;
    fs::rename(&from, &to)
        .map_err(|err| format!("cannot rename `{}` to `{}`: {}", from, to, err))?;
    Ok(vec![Value::Nil])
}

fn builtin_mkdir(args: Vec<Value>) -> Result<Vec<Value>, String> {
    let path = string_arg(&args, 0, "path")?;
    let recursive = bool_arg(&args, 1, "recursive")?;
    let result = if recursive {
        fs::create_dir_all(&path)
    } else {
        fs::create_dir(&path)
    };
    result.map_err(|err| io_error("create directory", &path, err))?;
    Ok(vec![Value::Nil])
}

fn builtin_list(args: Vec<Value>) -> Result<Vec<Value>, String> {
    let path = string_arg(&args, 0, "path")?;
    let mut names = Vec::new();
    for entry in fs::read_dir(&path).map_err(|err| io_error("list", &path, err))? {
        let entry = entry.map_err(|err| io_error("list", &path, err))?;
        names.push(entry.file_name().to_string_lossy().into_owned());
    }
    names.sort();
    Ok(vec![new_table(
        names.into_iter().map(Value::String).collect(),
    )])
}

fn builtin_isdir(args: Vec<Value>) -> Result<Vec<Value>, String> {
    let path = string_arg(&args, 0, "path")?;
    Ok(vec![Value::Bool(
        fs::metadata(path).is_ok_and(|metadata| metadata.is_dir()),
    )])
}
