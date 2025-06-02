use std::fs;
use std::path::PathBuf;
use tempfile::TempDir;

use serpen::config::Config;
use serpen::resolver::ModuleResolver;

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
    let resolver = ModuleResolver::new_with_pythonpath(config, Some(&pythonpath_str)).unwrap();
    let first_party_modules = resolver.get_first_party_modules();

    // Verify that modules from both src and PYTHONPATH are discovered
    assert!(
        first_party_modules.contains("src_module"),
        "Should discover modules from configured src directories"
    );
    assert!(
        first_party_modules.contains("pythonpath_module"),
        "Should discover modules from PYTHONPATH directories"
    );
    assert!(
        first_party_modules.contains("pythonpath_pkg"),
        "Should discover packages from PYTHONPATH directories"
    );
    assert!(
        first_party_modules.contains("pythonpath_pkg.submodule"),
        "Should discover submodules from PYTHONPATH packages"
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
    use serpen::resolver::ImportType;
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

    // Create resolver with PYTHONPATH override (multiple directories separated by platform-appropriate separator)
    let separator = if cfg!(windows) { ';' } else { ':' };
    let pythonpath_str = format!(
        "{}{}{}",
        pythonpath_dir1.to_string_lossy(),
        separator,
        pythonpath_dir2.to_string_lossy()
    );
    let resolver = ModuleResolver::new_with_pythonpath(config, Some(&pythonpath_str)).unwrap();
    let first_party_modules = resolver.get_first_party_modules();

    // Verify that modules from both PYTHONPATH directories are discovered
    assert!(
        first_party_modules.contains("module1"),
        "Should discover modules from first PYTHONPATH directory"
    );
    assert!(
        first_party_modules.contains("module2"),
        "Should discover modules from second PYTHONPATH directory"
    );
}

#[test]
fn test_pythonpath_empty_or_nonexistent() {
    // Set up config
    let src_path = PathBuf::from("tests/fixtures/simple_project");
    let config = Config {
        src: vec![src_path.clone()],
        ..Default::default()
    };

    // Test with empty PYTHONPATH
    let resolver1 = ModuleResolver::new_with_pythonpath(config.clone(), Some("")).unwrap();
    let scan_dirs1 = resolver1.get_scan_directories_with_pythonpath(Some(""));

    // Should only contain configured src directories
    assert_eq!(scan_dirs1.len(), 1);
    let expected_path = src_path.canonicalize().unwrap_or(src_path.clone());
    assert!(
        scan_dirs1.contains(&expected_path),
        "Expected {:?} in {:?}",
        expected_path,
        scan_dirs1
    );

    // Test with no PYTHONPATH
    let resolver2 = ModuleResolver::new_with_pythonpath(config.clone(), None).unwrap();
    let scan_dirs2 = resolver2.get_scan_directories_with_pythonpath(None);

    // Should only contain configured src directories
    assert_eq!(scan_dirs2.len(), 1);
    assert!(
        scan_dirs2.contains(&expected_path),
        "Expected {:?} in {:?}",
        expected_path,
        scan_dirs2
    );

    // Test with nonexistent directories in PYTHONPATH
    let separator = if cfg!(windows) { ';' } else { ':' };
    let nonexistent_pythonpath = format!("/nonexistent1{}/nonexistent2", separator);
    let resolver3 =
        ModuleResolver::new_with_pythonpath(config, Some(&nonexistent_pythonpath)).unwrap();
    let scan_dirs3 = resolver3.get_scan_directories_with_pythonpath(Some(&nonexistent_pythonpath));

    // Should only contain configured src directories (nonexistent dirs filtered out)
    assert_eq!(scan_dirs3.len(), 1);
    assert!(
        scan_dirs3.contains(&expected_path),
        "Expected {:?} in {:?}",
        expected_path,
        scan_dirs3
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

    // Create resolver with PYTHONPATH override that includes the same src directory plus another directory
    let separator = if cfg!(windows) { ';' } else { ':' };
    let pythonpath_str = format!(
        "{}{}{}",
        src_dir.to_string_lossy(),
        separator,
        other_dir.to_string_lossy()
    );
    let resolver = ModuleResolver::new_with_pythonpath(config, Some(&pythonpath_str)).unwrap();
    let scan_dirs = resolver.get_scan_directories_with_pythonpath(Some(&pythonpath_str));

    // Should only have 2 unique directories, even though src_dir appears in both config.src and PYTHONPATH
    assert_eq!(
        scan_dirs.len(),
        2,
        "Should deduplicate directories: got {:?}",
        scan_dirs
    );

    // Convert to canonical paths for comparison
    let expected_src = src_dir.canonicalize().unwrap_or(src_dir);
    let expected_other = other_dir.canonicalize().unwrap_or(other_dir);

    assert!(
        scan_dirs.contains(&expected_src),
        "Should contain src directory"
    );
    assert!(
        scan_dirs.contains(&expected_other),
        "Should contain other directory"
    );

    // Verify modules are discovered correctly
    let first_party_modules = resolver.get_first_party_modules();
    assert!(
        first_party_modules.contains("src_module"),
        "Should discover src_module"
    );
    assert!(
        first_party_modules.contains("other_module"),
        "Should discover other_module"
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
    let resolver = ModuleResolver::new_with_pythonpath(config, Some(&pythonpath_str)).unwrap();
    let scan_dirs = resolver.get_scan_directories_with_pythonpath(Some(&pythonpath_str));

    // Should deduplicate even with different path representations
    assert_eq!(
        scan_dirs.len(),
        1,
        "Should deduplicate paths even with different representations: got {:?}",
        scan_dirs
    );

    // The path should be canonicalized
    let canonical_src = src_dir.canonicalize().unwrap_or(src_dir);
    assert!(
        scan_dirs.contains(&canonical_src),
        "Should contain canonicalized src directory: expected {:?} in {:?}",
        canonical_src,
        scan_dirs
    );
}
