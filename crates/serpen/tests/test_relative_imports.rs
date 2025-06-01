use serpen::bundler::Bundler;
use serpen::config::Config;
use std::fs;
use tempfile::TempDir;

/// Integration test for relative import resolution using temporary test files
#[test]
fn test_relative_import_resolution() {
    // Initialize logger for debugging
    let _ = env_logger::try_init();

    // Create temporary directory structure for testing
    let temp_dir = TempDir::new().expect("Failed to create temp directory");
    let temp_path = temp_dir.path();

    // Create test package structure
    let package_dir = temp_path.join("test_package");
    fs::create_dir_all(&package_dir).expect("Failed to create package directory");

    let subpackage_dir = package_dir.join("subpackage");
    fs::create_dir_all(&subpackage_dir).expect("Failed to create subpackage directory");

    // Create main.py with relative imports
    let main_py_content = r#"# Test file with relative imports
from . import utils  # Should resolve to 'test_package.utils'
from .utils import helper_function  # Should resolve to 'test_package.utils'

def main():
    pass
"#;
    fs::write(package_dir.join("main.py"), main_py_content).expect("Failed to write main.py");

    // Create utils.py
    fs::write(package_dir.join("utils.py"), "def helper_function(): pass")
        .expect("Failed to write utils.py");

    // Create subpackage/module.py with relative imports
    let module_py_content = r#"# Test file with relative imports in subpackage
from .. import main  # Should resolve to 'test_package.main'

def sub_function():
    pass
"#;
    fs::write(subpackage_dir.join("module.py"), module_py_content)
        .expect("Failed to write subpackage/module.py");

    // Create a config that includes the temp package as a source directory
    let config = Config {
        src: vec![temp_path.to_path_buf()],
        ..Config::default()
    };

    let bundler = Bundler::new(config);

    // Test the main.py file
    let file_path = package_dir.join("main.py");
    let imports = bundler
        .extract_imports(&file_path)
        .expect("Failed to extract imports from main.py");

    println!("Imports from main.py: {:?}", imports);

    // Verify that relative imports are resolved correctly
    // main.py contains: "from . import utils" and "from .utils import helper_function"
    // Both should resolve to the same module: "test_package.utils"
    assert!(!imports.is_empty(), "Should have extracted some imports");
    assert!(
        imports.contains(&"test_package.utils".to_string()),
        "Should contain resolved 'test_package.utils' module"
    );
    assert_eq!(
        imports.len(),
        2,
        "Should have exactly 2 imports (both resolve to 'test_package.utils')"
    );
    assert!(
        imports.iter().all(|imp| imp == "test_package.utils"),
        "All imports should resolve to 'test_package.utils'"
    );

    // Test the subpackage module
    let file_path = subpackage_dir.join("module.py");
    let imports = bundler
        .extract_imports(&file_path)
        .expect("Failed to extract imports from subpackage/module.py");

    println!("Imports from subpackage/module.py: {:?}", imports);

    // Verify that relative imports are resolved correctly
    // subpackage/module.py contains: "from .. import main"
    // This should resolve to "test_package.main" (the parent package's main module)
    assert!(!imports.is_empty(), "Should have extracted some imports");
    assert!(
        imports.contains(&"test_package.main".to_string()),
        "Should contain resolved 'test_package.main' module"
    );
    assert_eq!(imports.len(), 1, "Should have exactly 1 import");
    assert_eq!(
        imports[0], "test_package.main",
        "Import should resolve to 'test_package.main'"
    );
}
