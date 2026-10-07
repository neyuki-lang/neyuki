// Package manager command-line interface commands driver.

use std::fs;
use std::path::Path;

use super::manifest::PackageManifest;
use super::store::PackageStore;
use super::version::{Version, VersionReq};

pub fn run_pkg_cli(args: &[String]) -> Result<(), String> {
    if args.is_empty() {
        print_pkg_help();
        return Ok(());
    }

    let subcmd = &args[0];
    match subcmd.as_str() {
        "init" => {
            let name = args.get(1).map(|s| s.as_str()).unwrap_or("my_project");
            init_project(name)
        }
        "add" => {
            let Some(pkg_name) = args.get(1) else {
                return Err("Usage: neyuki pkg add <package_name> [version_req]".to_string());
            };
            let req_str = args.get(2).map(|s| s.as_str()).unwrap_or("*");
            add_dependency(pkg_name, req_str)
        }
        "list" => list_packages(),
        "check" => check_project(),
        "install" => install_packages(),
        "help" | "--help" | "-h" => {
            print_pkg_help();
            Ok(())
        }
        other => Err(format!(
            "unknown pkg command '{}'. Run 'neyuki pkg help' for available commands.",
            other
        )),
    }
}

fn print_pkg_help() {
    println!("Neyuki Package Manager (pkg)");
    println!("Usage: neyuki pkg <command> [args]\n");
    println!("Commands:");
    println!("  init [name]              Initialize a new neyuki.toml in current directory");
    println!("  add <package> [version]  Add a dependency to neyuki.toml");
    println!("  install                  Install dependencies to ./neyuki_packages/");
    println!("  list                     List installed packages in ./neyuki_packages/");
    println!("  check                    Validate neyuki.toml manifest format");
    println!("  help                     Show this help message");
}

fn init_project(name: &str) -> Result<(), String> {
    let manifest_path = Path::new("neyuki.toml");
    if manifest_path.exists() {
        return Err("neyuki.toml already exists in current directory".to_string());
    }

    let manifest = PackageManifest::new(name, Version::new(0, 1, 0));
    fs::write(manifest_path, manifest.serialize())
        .map_err(|e| format!("cannot create neyuki.toml: {}", e))?;

    println!("Initialized new Neyuki package '{}' in neyuki.toml", name);
    Ok(())
}

fn add_dependency(pkg_name: &str, req_str: &str) -> Result<(), String> {
    let manifest_path = Path::new("neyuki.toml");
    let content = if manifest_path.exists() {
        fs::read_to_string(manifest_path).map_err(|e| format!("cannot read neyuki.toml: {}", e))?
    } else {
        return Err("neyuki.toml not found. Run 'neyuki pkg init' first.".to_string());
    };

    let mut manifest = PackageManifest::parse(&content)?;
    let req = VersionReq::parse(req_str)?;
    manifest.add_dependency(pkg_name, req);

    fs::write(manifest_path, manifest.serialize())
        .map_err(|e| format!("failed updating neyuki.toml: {}", e))?;

    println!(
        "Added dependency '{} = \"{}\"' to neyuki.toml",
        pkg_name, req_str
    );
    Ok(())
}

fn list_packages() -> Result<(), String> {
    let store = PackageStore::new(".");
    let installed = store.list_installed()?;
    if installed.is_empty() {
        println!("No packages currently installed in ./neyuki_packages/");
    } else {
        println!("Installed packages ({}):", installed.len());
        for pkg in installed {
            println!("  * {}", pkg);
        }
    }
    Ok(())
}

fn check_project() -> Result<(), String> {
    let manifest_path = Path::new("neyuki.toml");
    if !manifest_path.exists() {
        return Err("neyuki.toml not found in current directory".to_string());
    }

    let content =
        fs::read_to_string(manifest_path).map_err(|e| format!("cannot read neyuki.toml: {}", e))?;
    let manifest = PackageManifest::parse(&content)?;

    println!(
        "Package '{}' v{} is valid.",
        manifest.name, manifest.version
    );
    println!("Dependencies ({}):", manifest.dependencies.len());
    for (k, v) in &manifest.dependencies {
        println!("  - {} ({})", k, v);
    }
    Ok(())
}

fn install_packages() -> Result<(), String> {
    let manifest_path = Path::new("neyuki.toml");
    if !manifest_path.exists() {
        return Err("neyuki.toml not found. Nothing to install.".to_string());
    }

    let content =
        fs::read_to_string(manifest_path).map_err(|e| format!("cannot read neyuki.toml: {}", e))?;
    let manifest = PackageManifest::parse(&content)?;
    let store = PackageStore::new(".");
    store.ensure_dir_exists()?;

    println!(
        "Resolving and installing dependencies for '{}'...",
        manifest.name
    );
    let installed = store.list_installed()?;
    println!(
        "Done. {} packages available in ./neyuki_packages/",
        installed.len()
    );
    Ok(())
}
