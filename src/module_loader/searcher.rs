// Pattern-based module searcher engine following standard package.path conventions.

use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearcherEngine {
    patterns: Vec<String>,
}

impl Default for SearcherEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl SearcherEngine {
    pub fn new() -> Self {
        Self {
            patterns: vec![
                "./?.nyk".to_string(),
                "./?/init.nyk".to_string(),
                "./?/mod.nyk".to_string(),
                "./modules/?.nyk".to_string(),
                "./modules/?/init.nyk".to_string(),
                "./packages/?.nyk".to_string(),
                "./packages/?/init.nyk".to_string(),
            ],
        }
    }

    pub fn with_patterns(patterns_str: &str) -> Self {
        let patterns = patterns_str
            .split(';')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        Self { patterns }
    }

    pub fn add_pattern(&mut self, pattern: impl Into<String>) {
        self.patterns.push(pattern.into());
    }

    pub fn patterns(&self) -> &[String] {
        &self.patterns
    }

    /// Searches for a module across all configured search patterns.
    pub fn search(&self, module_name: &str, base_dir: Option<&Path>) -> Option<PathBuf> {
        // In Lua/Neyuki, dots in module names can represent directory separators: `foo.bar` -> `foo/bar`
        let normalized_slash = module_name.replace('.', "/");
        let base = base_dir.unwrap_or_else(|| Path::new("."));

        for pattern in &self.patterns {
            let replaced = pattern.replace('?', &normalized_slash);
            let candidate = if let Some(stripped) = replaced.strip_prefix("./") {
                base.join(stripped)
            } else {
                base.join(&replaced)
            };

            if candidate.is_file() {
                return Some(candidate);
            }
        }

        None
    }
}
