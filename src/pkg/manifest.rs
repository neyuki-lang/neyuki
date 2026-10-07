// Package manifest (neyuki.toml) parser and serializer.

use std::collections::BTreeMap;
use std::str::FromStr;

use super::version::{Version, VersionReq};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PackageManifest {
    pub name: String,
    pub version: Version,
    pub description: Option<String>,
    pub entry: String,
    pub dependencies: BTreeMap<String, VersionReq>,
    pub dev_dependencies: BTreeMap<String, VersionReq>,
    pub scripts: BTreeMap<String, String>,
}

impl Default for PackageManifest {
    fn default() -> Self {
        Self {
            name: "unnamed_pkg".to_string(),
            version: Version::new(0, 1, 0),
            description: None,
            entry: "main.nyk".to_string(),
            dependencies: BTreeMap::new(),
            dev_dependencies: BTreeMap::new(),
            scripts: BTreeMap::new(),
        }
    }
}

impl PackageManifest {
    pub fn new(name: impl Into<String>, version: Version) -> Self {
        Self {
            name: name.into(),
            version,
            ..Default::default()
        }
    }

    pub fn add_dependency(&mut self, name: impl Into<String>, req: VersionReq) {
        self.dependencies.insert(name.into(), req);
    }

    pub fn parse(content: &str) -> Result<Self, String> {
        let mut manifest = Self::default();
        let mut current_section = "package";

        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }

            if trimmed.starts_with('[') && trimmed.ends_with(']') {
                let sec = &trimmed[1..trimmed.len() - 1].trim();
                current_section = match *sec {
                    "package" => "package",
                    "dependencies" => "dependencies",
                    "dev-dependencies" | "dev_dependencies" => "dev-dependencies",
                    "scripts" => "scripts",
                    _ => "unknown",
                };
                continue;
            }

            let Some((key, val_raw)) = trimmed.split_once('=') else {
                continue;
            };
            let key = key.trim();
            let val = val_raw.trim().trim_matches('"').trim_matches('\'');

            match current_section {
                "package" => match key {
                    "name" => manifest.name = val.to_string(),
                    "version" => manifest.version = Version::from_str(val)?,
                    "description" => manifest.description = Some(val.to_string()),
                    "entry" => manifest.entry = val.to_string(),
                    _ => {}
                },
                "dependencies" => {
                    let req = VersionReq::parse(val)?;
                    manifest.dependencies.insert(key.to_string(), req);
                }
                "dev-dependencies" => {
                    let req = VersionReq::parse(val)?;
                    manifest.dev_dependencies.insert(key.to_string(), req);
                }
                "scripts" => {
                    manifest.scripts.insert(key.to_string(), val.to_string());
                }
                _ => {}
            }
        }

        Ok(manifest)
    }

    pub fn serialize(&self) -> String {
        let mut out = String::new();
        out.push_str("[package]\n");
        out.push_str(&format!("name = \"{}\"\n", self.name));
        out.push_str(&format!("version = \"{}\"\n", self.version));
        if let Some(desc) = &self.description {
            out.push_str(&format!("description = \"{}\"\n", desc));
        }
        out.push_str(&format!("entry = \"{}\"\n\n", self.entry));

        if !self.dependencies.is_empty() {
            out.push_str("[dependencies]\n");
            for (k, v) in &self.dependencies {
                out.push_str(&format!("{} = \"{}\"\n", k, v));
            }
            out.push('\n');
        }

        if !self.dev_dependencies.is_empty() {
            out.push_str("[dev-dependencies]\n");
            for (k, v) in &self.dev_dependencies {
                out.push_str(&format!("{} = \"{}\"\n", k, v));
            }
            out.push('\n');
        }

        if !self.scripts.is_empty() {
            out.push_str("[scripts]\n");
            for (k, v) in &self.scripts {
                out.push_str(&format!("{} = \"{}\"\n", k, v));
            }
            out.push('\n');
        }

        out
    }
}
