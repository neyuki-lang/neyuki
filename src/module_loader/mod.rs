// Modular external module loader engine for Neyuki.
// Supports requires outside of bundles, relative directory imports,
// init.nyk resolution, package.path searchers, and circular dependency protection.

pub mod cache;
pub mod path;
pub mod searcher;

pub use cache::ModuleCache;
pub use path::{PathEngine, PathResolutionError};
pub use searcher::SearcherEngine;

use std::path::{Path, PathBuf};
use std::rc::Rc;

use crate::bytecode::proto::Proto;

#[derive(Clone, Debug)]
pub struct ModuleLoader {
    searcher: SearcherEngine,
    cache: ModuleCache,
    base_dir: Option<PathBuf>,
}

impl Default for ModuleLoader {
    fn default() -> Self {
        Self::new()
    }
}

impl ModuleLoader {
    pub fn new() -> Self {
        Self {
            searcher: SearcherEngine::new(),
            cache: ModuleCache::new(),
            base_dir: None,
        }
    }

    pub fn with_base_dir(mut self, base_dir: impl Into<PathBuf>) -> Self {
        self.base_dir = Some(base_dir.into());
        self
    }

    pub fn searcher(&self) -> &SearcherEngine {
        &self.searcher
    }

    pub fn searcher_mut(&mut self) -> &mut SearcherEngine {
        &mut self.searcher
    }

    pub fn cache(&self) -> &ModuleCache {
        &self.cache
    }

    pub fn cache_mut(&mut self) -> &mut ModuleCache {
        &mut self.cache
    }

    /// Resolves an external module path request using searchers or relative paths.
    pub fn resolve_module(&self, requested: &str) -> Result<PathBuf, String> {
        let base = self.base_dir.as_deref();

        // 1. Try searcher patterns first (e.g. ./?.nyk, ./modules/?.nyk)
        if let Some(found) = self.searcher.search(requested, base) {
            return Ok(found);
        }

        // 2. Try direct relative path resolution (e.g. ./sub/utils or utils/math)
        match PathEngine::resolve_candidate(requested, base) {
            Ok(found) => Ok(found),
            Err(e) => Err(e.to_string()),
        }
    }

    /// Resolves and reads module source or bytecode from disk.
    pub fn read_module(&self, requested: &str) -> Result<(PathBuf, Vec<u8>), String> {
        let resolved = self.resolve_module(requested)?;
        let bytes = std::fs::read(&resolved)
            .map_err(|e| format!("cannot read module '{}': {}", resolved.display(), e))?;
        Ok((resolved, bytes))
    }

    /// Compiles module bytes into a verified `Proto`.
    pub fn compile_bytes(
        &mut self,
        path: &Path,
        bytes: &[u8],
        requested_name: &str,
    ) -> Result<Rc<Proto>, String> {
        let path_buf = path.to_path_buf();
        if let Some(cached) = self.cache.get_proto(&path_buf) {
            return Ok(cached);
        }

        let proto = if bytes.starts_with(crate::bytecode::MAGIC)
            || path.extension().is_some_and(|ext| ext == "nykb")
        {
            let p = crate::bytecode::deserialize(bytes)?;
            crate::bytecode::verify_proto(&p)
                .map_err(|e| format!("bytecode verification failed: {}", e))?;
            p
        } else {
            let src = std::str::from_utf8(bytes)
                .map_err(|_| format!("cannot read module '{}': invalid UTF-8", requested_name))?;
            let stmts = crate::compiler::compile_source(src)?;
            let diags = crate::sema::analyze(&stmts, src);
            if let Some(err) = diags
                .iter()
                .find(|d| d.severity == crate::diagnostics::severity::Severity::Error)
            {
                return Err(format!(
                    "semantic error in module '{}': {}",
                    requested_name, err.message
                ));
            }
            crate::compiler::try_compile_to_proto_via_ir(&stmts)?
        };

        let rc = Rc::new(proto);
        self.cache.insert_proto(path_buf, rc.clone());
        Ok(rc)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_searcher_dot_conversion() {
        let searcher = SearcherEngine::new();
        assert!(searcher.patterns().contains(&"./?.nyk".to_string()));
        assert!(searcher.patterns().contains(&"./?/init.nyk".to_string()));
    }

    #[test]
    fn test_module_cache_cycle_detection() {
        let mut cache = ModuleCache::new();
        assert!(cache.begin_loading("a").is_ok());
        assert!(cache.begin_loading("b").is_ok());
        let cycle_err = cache.begin_loading("a").unwrap_err();
        assert!(cycle_err.contains("circular dependency detected"));
        cache.finish_loading("b");
        cache.finish_loading("a");
        assert!(cache.begin_loading("a").is_ok());
    }

    #[test]
    fn test_path_resolution_candidate_extensions() {
        // Non-existent module returns clean error
        let err = PathEngine::resolve_candidate("non_existent_12345", None).unwrap_err();
        assert!(matches!(err, PathResolutionError::FileNotFound(_)));
    }
}
