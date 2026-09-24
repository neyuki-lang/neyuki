// Neyuki Package Manager (pkg) - Comprehensive dependency and package engine.

pub mod cli;
pub mod lockfile;
pub mod manifest;
pub mod resolver;
pub mod store;
pub mod version;

pub use cli::run_pkg_cli;
pub use lockfile::{LockedPackage, Lockfile};
pub use manifest::PackageManifest;
pub use resolver::{AvailablePackage, DependencyResolver};
pub use store::PackageStore;
pub use version::{Version, VersionReq};

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn test_semver_comparisons_and_caret() {
        let v1 = Version::new(1, 2, 3);
        let v2 = Version::new(1, 3, 0);
        let v3 = Version::new(2, 0, 0);

        assert!(v1 < v2);
        assert!(v2 < v3);

        let req_caret = VersionReq::parse("^1.2.0").unwrap();
        assert!(req_caret.matches(&v1));
        assert!(req_caret.matches(&v2));
        assert!(!req_caret.matches(&v3));

        let req_tilde = VersionReq::parse("~1.2.0").unwrap();
        assert!(req_tilde.matches(&v1));
        assert!(!req_tilde.matches(&v2));
    }

    #[test]
    fn test_manifest_parse_and_serialize() {
        let toml_str = r#"
[package]
name = "demo_service"
version = "1.0.4"
description = "A demo web service"
entry = "server.nyk"

[dependencies]
http = "^0.5.0"
json = "=1.0.0"

[scripts]
test = "neyuki test"
"#;
        let manifest = PackageManifest::parse(toml_str).unwrap();
        assert_eq!(manifest.name, "demo_service");
        assert_eq!(manifest.version, Version::new(1, 0, 4));
        assert_eq!(manifest.dependencies.len(), 2);
        assert!(manifest.dependencies.contains_key("http"));

        let serialized = manifest.serialize();
        assert!(serialized.contains("name = \"demo_service\""));
        assert!(serialized.contains("version = \"1.0.4\""));
    }

    #[test]
    fn test_lockfile_roundtrip() {
        let mut lockfile = Lockfile::new();
        lockfile.add_package(LockedPackage {
            name: "crypto_utils".to_string(),
            version: Version::new(2, 1, 0),
            checksum: "a1b2c3d4e5f6".to_string(),
            dependencies: vec!["buffer".to_string()],
        });

        let serialized = lockfile.serialize();
        let parsed = Lockfile::parse(&serialized).unwrap();
        assert_eq!(parsed.packages.len(), 1);
        assert_eq!(parsed.packages[0].name, "crypto_utils");
        assert_eq!(parsed.packages[0].version, Version::new(2, 1, 0));
        assert_eq!(parsed.packages[0].checksum, "a1b2c3d4e5f6");
    }

    #[test]
    fn test_dependency_resolver_multi_version() {
        let mut resolver = DependencyResolver::new();
        resolver.register_candidate(AvailablePackage {
            name: "math_lib".to_string(),
            version: Version::new(1, 0, 0),
            dependencies: HashMap::new(),
            checksum: "h1".to_string(),
        });
        resolver.register_candidate(AvailablePackage {
            name: "math_lib".to_string(),
            version: Version::new(1, 5, 2),
            dependencies: HashMap::new(),
            checksum: "h2".to_string(),
        });

        let mut manifest = PackageManifest::new("app", Version::new(0, 1, 0));
        manifest.add_dependency("math_lib", VersionReq::parse("^1.0.0").unwrap());

        let lock = resolver.resolve(&manifest).unwrap();
        assert_eq!(lock.packages.len(), 1);
        assert_eq!(lock.packages[0].name, "math_lib");
        // Should resolve to the highest matching version 1.5.2
        assert_eq!(lock.packages[0].version, Version::new(1, 5, 2));
    }
}
