#![allow(clippy::disallowed_methods)]

use std::fs;

use cribo::{
    config::Config,
    resolver::{ImportType, ModuleResolver},
};
use tempfile::TempDir;

#[test]
fn test_pythonpath_module_discovery() {
    // Create temporary directories for testing
    let temp_dir = TempDir::new().unwrap();
    let pythonpath_dir = temp_dir.path().join("pythonpath_modules");
    let src_dir = temp_dir.path().join("src");

    // Create directory structures
    fs::create_dir_all(&pythonpath_dir).unwrap();
    fs::create_dir_all(&src_dir).unwrap();

    // Create a module in PYTHONPATH directory
    let pythonpath_module = pythonpath_dir.join("pythonpath_module.py");
    fs::write(
        &pythonpath_module,
        "# This is a PYTHONPATH module\ndef hello():\n    return 'Hello from PYTHONPATH'",
    )
    .unwrap();

    // Create a package in PYTHONPATH directory
    let pythonpath_pkg = pythonpath_dir.join("pythonpath_pkg");
    fs::create_dir_all(&pythonpath_pkg).unwrap();
    let pythonpath_pkg_init = pythonpath_pkg.join("__init__.py");
    fs::write(&pythonpath_pkg_init, "# PYTHONPATH package").unwrap();
    let pythonpath_pkg_module = pythonpath_pkg.join("submodule.py");
    fs::write(&pythonpath_pkg_module, "# PYTHONPATH submodule").unwrap();

    // Create a module in src directory
    let src_module = src_dir.join("src_module.py");
    fs::write(&src_module, "# This is a src module").unwrap();

    // Set up config with src directory
    let config = Config {
        src: vec![src_dir.clone()],
        ..Default::default()
    };

    // Create resolver with PYTHONPATH override
    let pythonpath_str = pythonpath_dir.to_string_lossy();
    let mut resolver = ModuleResolver::new_with_pythonpath(config, Some(&pythonpath_str)).unwrap();

    // Test that modules can be resolved from both src and PYTHONPATH
    assert!(
        resolver
            .resolve_module_path("src_module")
            .unwrap()
            .is_some(),
        "Should resolve modules from configured src directories"
    );
    assert!(
        resolver
            .resolve_module_path("pythonpath_module")
            .unwrap()
            .is_some(),
        "Should resolve modules from PYTHONPATH directories"
    );
    assert!(
        resolver
            .resolve_module_path("pythonpath_pkg")
            .unwrap()
            .is_some(),
        "Should resolve packages from PYTHONPATH directories"
    );
    assert!(
        resolver
            .resolve_module_path("pythonpath_pkg.submodule")
            .unwrap()
            .is_some(),
        "Should resolve submodules from PYTHONPATH packages"
    );

    // Also verify classification
    assert_eq!(
        resolver.classify_import("src_module"),
        ImportType::FirstParty,
        "Should classify src_module as first-party"
    );
    assert_eq!(
        resolver.classify_import("pythonpath_module"),
        ImportType::FirstParty,
        "Should classify pythonpath_module as first-party"
    );
    assert_eq!(
        resolver.classify_import("pythonpath_pkg"),
        ImportType::FirstParty,
        "Should classify pythonpath_pkg as first-party"
    );
    assert_eq!(
        resolver.classify_import("pythonpath_pkg.submodule"),
        ImportType::FirstParty,
        "Should classify pythonpath_pkg.submodule as first-party"
    );
}

#[test]
fn test_pythonpath_module_classification() {
    // Create temporary directories for testing
    let temp_dir = TempDir::new().unwrap();
    let pythonpath_dir = temp_dir.path().join("pythonpath_modules");
    let src_dir = temp_dir.path().join("src");

    // Create directory structures
    fs::create_dir_all(&pythonpath_dir).unwrap();
    fs::create_dir_all(&src_dir).unwrap();

    // Create a module in PYTHONPATH directory
    let pythonpath_module = pythonpath_dir.join("pythonpath_module.py");
    fs::write(&pythonpath_module, "# This is a PYTHONPATH module").unwrap();

    // Set up config
    let config = Config {
        src: vec![src_dir.clone()],
        ..Default::default()
    };

    // Create resolver with PYTHONPATH override
    let pythonpath_str = pythonpath_dir.to_string_lossy();
    let resolver = ModuleResolver::new_with_pythonpath(config, Some(&pythonpath_str)).unwrap();

    // Test that PYTHONPATH modules are classified as first-party
    use cribo::resolver::ImportType;
    assert_eq!(
        resolver.classify_import("pythonpath_module"),
        ImportType::FirstParty,
        "PYTHONPATH modules should be classified as first-party"
    );

    // Test that unknown modules are still classified as third-party
    assert_eq!(
        resolver.classify_import("unknown_module"),
        ImportType::ThirdParty,
        "Unknown modules should still be classified as third-party"
    );
}

#[test]
fn test_pythonpath_multiple_directories() {
    // Create temporary directories for testing
    let temp_dir = TempDir::new().unwrap();
    let pythonpath_dir1 = temp_dir.path().join("pythonpath1");
    let pythonpath_dir2 = temp_dir.path().join("pythonpath2");
    let src_dir = temp_dir.path().join("src");

    // Create directory structures
    fs::create_dir_all(&pythonpath_dir1).unwrap();
    fs::create_dir_all(&pythonpath_dir2).unwrap();
    fs::create_dir_all(&src_dir).unwrap();

    // Create modules in different PYTHONPATH directories
    let module1 = pythonpath_dir1.join("module1.py");
    fs::write(&module1, "# Module in pythonpath1").unwrap();

    let module2 = pythonpath_dir2.join("module2.py");
    fs::write(&module2, "# Module in pythonpath2").unwrap();

    // Set up config
    let config = Config {
        src: vec![src_dir.clone()],
        ..Default::default()
    };

    // Create resolver with PYTHONPATH override (multiple directories separated by
    // platform-appropriate separator)
    let separator = if cfg!(windows) { ';' } else { ':' };
    let pythonpath_str = format!(
        "{}{}{}",
        pythonpath_dir1.to_string_lossy(),
        separator,
        pythonpath_dir2.to_string_lossy()
    );
    let mut resolver = ModuleResolver::new_with_pythonpath(config, Some(&pythonpath_str)).unwrap();

    // Test that modules from both PYTHONPATH directories can be resolved
    assert!(
        resolver.resolve_module_path("module1").unwrap().is_some(),
        "Should resolve modules from first PYTHONPATH directory"
    );
    assert!(
        resolver.resolve_module_path("module2").unwrap().is_some(),
        "Should resolve modules from second PYTHONPATH directory"
    );

    // Also verify classification
    assert_eq!(
        resolver.classify_import("module1"),
        ImportType::FirstParty,
        "Should classify module1 as first-party"
    );
    assert_eq!(
        resolver.classify_import("module2"),
        ImportType::FirstParty,
        "Should classify module2 as first-party"
    );
}

#[test]
fn test_pythonpath_empty_or_nonexistent() {
    // Create a temporary directory for testing
    let temp_dir = TempDir::new().unwrap();
    let src_dir = temp_dir.path().join("src");
    fs::create_dir_all(&src_dir).unwrap();

    // Create a test module
    let test_module = src_dir.join("test_module.py");
    fs::write(&test_module, "# Test module").unwrap();

    let config = Config {
        src: vec![src_dir.clone()],
        ..Default::default()
    };

    // Test with empty PYTHONPATH
    let mut resolver1 = ModuleResolver::new_with_pythonpath(config.clone(), Some("")).unwrap();

    // Should be able to resolve module from src directory
    assert!(
        resolver1
            .resolve_module_path("test_module")
            .unwrap()
            .is_some(),
        "Should resolve module from src directory with empty PYTHONPATH"
    );

    // Test with no PYTHONPATH
    let mut resolver2 = ModuleResolver::new_with_pythonpath(config.clone(), None).unwrap();

    // Should be able to resolve module from src directory
    assert!(
        resolver2
            .resolve_module_path("test_module")
            .unwrap()
            .is_some(),
        "Should resolve module from src directory with no PYTHONPATH"
    );

    // Test with nonexistent directories in PYTHONPATH
    let separator = if cfg!(windows) { ';' } else { ':' };
    let nonexistent_pythonpath = format!("/nonexistent1{separator}/nonexistent2");
    let mut resolver3 =
        ModuleResolver::new_with_pythonpath(config, Some(&nonexistent_pythonpath)).unwrap();

    // Should still be able to resolve module from src directory
    assert!(
        resolver3
            .resolve_module_path("test_module")
            .unwrap()
            .is_some(),
        "Should resolve module from src directory even with nonexistent PYTHONPATH"
    );

    // Non-existent modules should not be found
    assert!(
        resolver3
            .resolve_module_path("nonexistent_module")
            .unwrap()
            .is_none(),
        "Should not find nonexistent modules"
    );
}

#[test]
fn test_directory_deduplication() {
    // Create temporary directories for testing
    let temp_dir = TempDir::new().unwrap();
    let src_dir = temp_dir.path().join("src");
    let other_dir = temp_dir.path().join("other");

    // Create directory structures
    fs::create_dir_all(&src_dir).unwrap();
    fs::create_dir_all(&other_dir).unwrap();

    // Create modules
    let src_module = src_dir.join("src_module.py");
    fs::write(&src_module, "# Source module").unwrap();
    let other_module = other_dir.join("other_module.py");
    fs::write(&other_module, "# Other module").unwrap();

    // Set up config with src directory
    let config = Config {
        src: vec![src_dir.clone()],
        ..Default::default()
    };

    // Create resolver with PYTHONPATH override that includes the same src directory plus another
    // directory
    let separator = if cfg!(windows) { ';' } else { ':' };
    let pythonpath_str = format!(
        "{}{}{}",
        src_dir.to_string_lossy(),
        separator,
        other_dir.to_string_lossy()
    );
    let mut resolver = ModuleResolver::new_with_pythonpath(config, Some(&pythonpath_str)).unwrap();

    // Test that deduplication works - both modules should be resolvable
    assert!(
        resolver
            .resolve_module_path("src_module")
            .unwrap()
            .is_some(),
        "Should resolve src_module"
    );
    assert!(
        resolver
            .resolve_module_path("other_module")
            .unwrap()
            .is_some(),
        "Should resolve other_module"
    );

    // Both should be classified as first-party
    assert_eq!(
        resolver.classify_import("src_module"),
        ImportType::FirstParty,
        "Should classify src_module as first-party"
    );
    assert_eq!(
        resolver.classify_import("other_module"),
        ImportType::FirstParty,
        "Should classify other_module as first-party"
    );
}

#[test]
fn test_path_canonicalization() {
    // Create temporary directories for testing
    let temp_dir = TempDir::new().unwrap();
    let src_dir = temp_dir.path().join("src");
    fs::create_dir_all(&src_dir).unwrap();

    // Create a module
    let module_file = src_dir.join("test_module.py");
    fs::write(&module_file, "# Test module").unwrap();

    // Set up config with the src directory
    let config = Config {
        src: vec![src_dir.clone()],
        ..Default::default()
    };

    // Create resolver with PYTHONPATH override using a relative path with .. components
    // This creates a different string representation of the same directory
    let parent_dir = src_dir.parent().unwrap();
    let relative_path = parent_dir.join("src/../src"); // This resolves to the same directory
    let pythonpath_str = relative_path.to_string_lossy();
    let mut resolver = ModuleResolver::new_with_pythonpath(config, Some(&pythonpath_str)).unwrap();

    // Test that the module can be resolved despite path canonicalization differences
    assert!(
        resolver
            .resolve_module_path("test_module")
            .unwrap()
            .is_some(),
        "Should resolve module even with different path representations"
    );

    // Should be classified as first-party
    assert_eq!(
        resolver.classify_import("test_module"),
        ImportType::FirstParty,
        "Should classify test_module as first-party"
    );
}
