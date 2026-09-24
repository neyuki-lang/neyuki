// Safe path resolution and sandbox validation engine for external module requires.

use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PathResolutionError {
    NullByteDetected,
    PathTraversalForbidden(String),
    FileNotFound(String),
    SecurityViolation(String),
}

impl std::fmt::Display for PathResolutionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NullByteDetected => write!(f, "security error: null byte in module path"),
            Self::PathTraversalForbidden(p) => {
                write!(
                    f,
                    "security error: path traversal forbidden in require: '{}'",
                    p
                )
            }
            Self::FileNotFound(p) => write!(f, "module not found: '{}'", p),
            Self::SecurityViolation(msg) => write!(f, "security violation: {}", msg),
        }
    }
}

pub struct PathEngine;

impl PathEngine {
    /// Validates and resolves an external module request to a concrete file on disk.
    pub fn resolve_candidate(
        requested: &str,
        base_dir: Option<&Path>,
    ) -> Result<PathBuf, PathResolutionError> {
        if requested.contains('\0') {
            return Err(PathResolutionError::NullByteDetected);
        }

        let base = base_dir.unwrap_or_else(|| Path::new("."));
        let initial_path = base.join(requested);

        // Candidates to probe in priority order
        let mut candidates = Vec::new();
        if requested.ends_with(".nyk") || requested.ends_with(".nykb") {
            candidates.push(initial_path.clone());
        } else {
            let mut with_nyk = initial_path.clone();
            with_nyk.set_extension("nyk");
            candidates.push(with_nyk);

            let mut with_nykb = initial_path.clone();
            with_nykb.set_extension("nykb");
            candidates.push(with_nykb);

            candidates.push(initial_path.join("init.nyk"));
            candidates.push(initial_path.join("mod.nyk"));
            candidates.push(initial_path.join("index.nyk"));
        }

        for candidate in candidates {
            if candidate.is_file() {
                return Ok(candidate);
            }
        }

        Err(PathResolutionError::FileNotFound(requested.to_string()))
    }

    /// Verifies that a resolved path stays within the permitted root directory.
    pub fn verify_sandbox(
        resolved_path: &Path,
        root_dir: &Path,
    ) -> Result<(), PathResolutionError> {
        let canonical_root = root_dir
            .canonicalize()
            .map_err(|e| PathResolutionError::SecurityViolation(e.to_string()))?;
        let canonical_target = resolved_path
            .canonicalize()
            .map_err(|e| PathResolutionError::SecurityViolation(e.to_string()))?;

        if canonical_target.starts_with(&canonical_root) {
            Ok(())
        } else {
            Err(PathResolutionError::PathTraversalForbidden(
                resolved_path.display().to_string(),
            ))
        }
    }
}
