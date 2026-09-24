// Local package installation store and directory manager.

use std::fs;
use std::path::{Path, PathBuf};

use super::lockfile::LockedPackage;

pub struct PackageStore {
    root_dir: PathBuf,
}

impl Default for PackageStore {
    fn default() -> Self {
        Self::new(".")
    }
}

impl PackageStore {
    pub fn new(project_root: impl Into<PathBuf>) -> Self {
        let p: PathBuf = project_root.into();
        Self {
            root_dir: p.join("neyuki_packages"),
        }
    }

    pub fn packages_dir(&self) -> &Path {
        &self.root_dir
    }

    pub fn ensure_dir_exists(&self) -> Result<(), String> {
        if !self.root_dir.exists() {
            fs::create_dir_all(&self.root_dir)
                .map_err(|e| format!("cannot create neyuki_packages directory: {}", e))?;
        }
        Ok(())
    }

    pub fn install_package(
        &self,
        pkg: &LockedPackage,
        files: &[(&str, &[u8])],
    ) -> Result<PathBuf, String> {
        self.ensure_dir_exists()?;
        let pkg_dir = self.root_dir.join(&pkg.name);
        fs::create_dir_all(&pkg_dir)
            .map_err(|e| format!("cannot create package directory for '{}': {}", pkg.name, e))?;

        for (filename, content) in files {
            let file_path = pkg_dir.join(filename);
            if let Some(parent) = file_path.parent() {
                let _ = fs::create_dir_all(parent);
            }
            fs::write(&file_path, content).map_err(|e| {
                format!(
                    "failed writing package file '{}': {}",
                    file_path.display(),
                    e
                )
            })?;
        }

        Ok(pkg_dir)
    }

    pub fn list_installed(&self) -> Result<Vec<String>, String> {
        if !self.root_dir.exists() {
            return Ok(Vec::new());
        }

        let mut out = Vec::new();
        let entries = fs::read_dir(&self.root_dir)
            .map_err(|e| format!("failed reading neyuki_packages: {}", e))?;

        for entry in entries.flatten() {
            let is_dir = entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false);
            if let (true, Some(name)) = (is_dir, entry.file_name().to_str()) {
                out.push(name.to_string());
            }
        }

        out.sort();
        Ok(out)
    }
}
