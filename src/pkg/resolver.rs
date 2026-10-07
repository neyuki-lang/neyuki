// Dependency graph resolver and version constraint solver.

use std::collections::{HashMap, HashSet};

use super::lockfile::{LockedPackage, Lockfile};
use super::manifest::PackageManifest;
use super::version::{Version, VersionReq};

#[derive(Clone, Debug)]
pub struct AvailablePackage {
    pub name: String,
    pub version: Version,
    pub dependencies: HashMap<String, VersionReq>,
    pub checksum: String,
}

pub struct DependencyResolver {
    available: HashMap<String, Vec<AvailablePackage>>,
}

impl Default for DependencyResolver {
    fn default() -> Self {
        Self::new()
    }
}

impl DependencyResolver {
    pub fn new() -> Self {
        Self {
            available: HashMap::new(),
        }
    }

    pub fn register_candidate(&mut self, pkg: AvailablePackage) {
        self.available
            .entry(pkg.name.clone())
            .or_default()
            .push(pkg);
    }

    /// Resolves dependencies declared in manifest into a complete `Lockfile`.
    pub fn resolve(&self, manifest: &PackageManifest) -> Result<Lockfile, String> {
        let mut resolved = Lockfile::new();
        let mut visited = HashSet::new();

        for (dep_name, req) in &manifest.dependencies {
            self.resolve_recursive(dep_name, req, &mut resolved, &mut visited)?;
        }

        Ok(resolved)
    }

    fn resolve_recursive(
        &self,
        name: &str,
        req: &VersionReq,
        lockfile: &mut Lockfile,
        visited: &mut HashSet<String>,
    ) -> Result<(), String> {
        if visited.contains(name) {
            return Ok(());
        }

        let candidates = self
            .available
            .get(name)
            .ok_or_else(|| format!("package '{}' not found in registry", name))?;

        // Find highest version satisfying the requirement
        let mut matched: Vec<&AvailablePackage> = candidates
            .iter()
            .filter(|c| req.matches(&c.version))
            .collect();
        matched.sort_by(|a, b| b.version.cmp(&a.version));

        let chosen = matched.first().ok_or_else(|| {
            format!(
                "no matching version for '{}' with constraint '{}'",
                name, req
            )
        })?;

        visited.insert(name.to_string());

        let dep_names: Vec<String> = chosen.dependencies.keys().cloned().collect();

        // Recursively resolve sub-dependencies
        for (sub_name, sub_req) in &chosen.dependencies {
            self.resolve_recursive(sub_name, sub_req, lockfile, visited)?;
        }

        lockfile.add_package(LockedPackage {
            name: chosen.name.clone(),
            version: chosen.version.clone(),
            checksum: chosen.checksum.clone(),
            dependencies: dep_names,
        });

        Ok(())
    }
}
