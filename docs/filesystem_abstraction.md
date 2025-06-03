# Filesystem Abstraction in Serpen

This document provides a comprehensive overview of the filesystem abstraction system implemented in Serpen, including its architecture, components, benefits, and usage patterns.

## 1. Overview and Architecture

The filesystem abstraction in Serpen provides a unified interface for interacting with files, allowing both physical and in-memory filesystem implementations. This approach offers several key benefits:

- **Testing Improvements**: Tests can run in isolation without depending on the physical filesystem
- **Cross-Platform Consistency**: Eliminates platform-specific filesystem behavior differences
- **Performance**: Significantly improves test execution speed by eliminating disk I/O
- **Flexibility**: Allows for different filesystem implementations to be used interchangeably
- **Isolation**: Tests operate in controlled environments, avoiding interference from other processes

### 1.1 Architecture Design

The filesystem abstraction is based on a trait-based design with these key components:

```
┌────────────────┐     ┌────────────────┐
│ System Trait   │     │ WritableSystem │
│ (Read-only)    │◄────┤ Trait          │
└────────────────┘     └────────────────┘
        ▲                      ▲
        │                      │
┌────────────────┐     ┌────────────────┐
│ PhysicalSystem │     │ MemorySystem   │
│ Implementation │     │ Implementation │
└────────────────┘     └────────────────┘
                              │
                      ┌────────────────┐
                      │ SharedMemory   │
                      │ System         │
                      └────────────────┘
```

The core components connect with the filesystem through these traits:

```
┌────────────────┐     ┌────────────────┐     ┌────────────────┐
│ ModuleResolver │     │ DependencyGraph│     │ Bundler        │
│ Fs             │────►│ Fs             │────►│ Fs             │
└────────────────┘     └────────────────┘     └────────────────┘
                                                      │
                                              ┌────────────────┐
                                              │ CodeEmitter    │
                                              │ Fs             │
                                              └────────────────┘
```

## 2. Core Filesystem Traits

### 2.1 System Trait

The `System` trait defines the core read-only filesystem operations:

```rust
pub trait System: Debug {
    /// Check if a path exists
    fn path_exists(&self, path: &Path) -> bool;

    /// Check if path is a directory
    fn is_directory(&self, path: &Path) -> bool;

    /// Check if path is a file
    fn is_file(&self, path: &Path) -> bool;

    /// Get metadata for a path
    fn path_metadata(&self, path: &Path) -> Result<FileMetadata>;

    /// Read file contents to string
    fn read_to_string(&self, path: &Path) -> Result<String>;

    /// Get current working directory
    fn current_directory(&self) -> &Path;

    /// List directory contents
    fn read_directory<'a>(
        &'a self,
        path: &Path,
    ) -> Result<Box<dyn Iterator<Item = Result<DirectoryEntry>> + 'a>>;

    /// Walk directory recursively
    fn walk_directory(&self, path: &Path) -> WalkDirectoryBuilder;

    /// Search for files matching a glob pattern
    fn glob(
        &self,
        pattern: &str,
    ) -> std::result::Result<
        Box<dyn Iterator<Item = std::result::Result<PathBuf, GlobError>> + '_>,
        PatternError,
    >;
}
```

### 2.2 WritableSystem Trait

The `WritableSystem` trait extends the `System` trait with write operations:

```rust
pub trait WritableSystem: System {
    /// Write content to a file
    fn write_file(&self, path: &Path, content: &str) -> Result<()>;

    /// Create directory and parents if needed
    fn create_directory_all(&self, path: &Path) -> Result<()>;
}
```

## 3. Filesystem Implementations

### 3.1 Physical Filesystem

The `PhysicalFileSystem` implementation wraps the standard library's filesystem operations:

```rust
pub struct PhysicalFileSystem {
    current_dir: PathBuf,
}

impl PhysicalFileSystem {
    pub fn new() -> Self {
        Self {
            current_dir: std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/")),
        }
    }
}

impl System for PhysicalFileSystem {
    // Implementation delegates to std::fs operations
    fn path_exists(&self, path: &Path) -> bool {
        path.exists()
    }

    fn is_file(&self, path: &Path) -> bool {
        path.is_file()
    }

    fn is_directory(&self, path: &Path) -> bool {
        path.is_dir()
    }

    fn read_to_string(&self, path: &Path) -> Result<String> {
        std::fs::read_to_string(path)
    }

    // Other method implementations...
}

impl WritableSystem for PhysicalFileSystem {
    fn write_file(&self, path: &Path, content: &str) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, content)
    }

    fn create_directory_all(&self, path: &Path) -> Result<()> {
        std::fs::create_dir_all(path)
    }
}
```

### 3.2 In-Memory Filesystem

The `MemoryFileSystem` provides an in-memory implementation for testing:

```rust
pub struct MemoryFileSystem {
    files: HashMap<PathBuf, FileData>,
    cwd: PathBuf,
    case_sensitive: bool,
}

struct FileData {
    content: Vec<u8>,
    file_type: FileType,
    modified: SystemTime,
}

impl MemoryFileSystem {
    pub fn new() -> Self {
        let mut fs = Self {
            files: HashMap::new(),
            cwd: PathBuf::from("/"),
            case_sensitive: true,
        };

        // Add root directory
        fs.files.insert(
            PathBuf::from("/"),
            FileData {
                content: Vec::new(),
                file_type: FileType::Directory,
                modified: SystemTime::now(),
            },
        );

        fs
    }

    pub fn with_case_sensitivity(mut self, case_sensitive: bool) -> Self {
        self.case_sensitive = case_sensitive;
        self
    }

    pub fn with_working_directory<P: AsRef<Path>>(mut self, path: P) -> Self {
        self.cwd = self.normalize_path(path);
        self
    }

    fn normalize_path<P: AsRef<Path>>(&self, path: P) -> PathBuf {
        // Implementation to handle . and .. components
    }
}

impl System for MemoryFileSystem {
    // Implementation methods...
}

impl WritableSystem for MemoryFileSystem {
    // Implementation methods...
}
```

### 3.3 Shared Memory Filesystem

For thread-safe operations, a `SharedMemoryFileSystem` wrapper is available:

```rust
pub struct SharedMemoryFileSystem {
    inner: RwLock<MemoryFileSystem>,
}

impl SharedMemoryFileSystem {
    pub fn new() -> Self {
        Self {
            inner: RwLock::new(MemoryFileSystem::new()),
        }
    }

    // Configuration methods...
}

impl System for SharedMemoryFileSystem {
    // Delegates to inner with read lock
}

impl WritableSystem for SharedMemoryFileSystem {
    // Delegates to inner with write lock
}
```

## 4. Test Benefits and Use Cases

### 4.1 Key Benefits for Testing

1. **Speed**: Tests run significantly faster without disk I/O overhead
2. **Reliability**: Tests are more deterministic and less prone to filesystem quirks
3. **Simplicity**: Test code is cleaner and more focused on what's being tested
4. **Isolation**: Tests don't interfere with each other or the host system
5. **Flexibility**: Tests can create complex file structures without worrying about cleanup
6. **Cross-platform consistency**: Tests behave the same on all platforms
7. **Better error handling**: Error scenarios can be tested more easily
8. **Simplified snapshot testing**: Better control over test input and output

### 4.2 High-Value Test Cases

The following test cases benefit most from the in-memory filesystem:

#### 4.2.1 Virtual Environment Tests

**Before**: Required complex temporary directory creation, environment variable manipulation, and serial test execution.

**After**:

```rust
#[test]
fn test_virtualenv_import_classification() {
    let mut fs = MemoryFileSystem::new();

    // Create virtual environment structure in memory
    fs.create_directory_all("/test_venv/lib/python3.11/site-packages")
        .unwrap();
    fs.create_directory_all("/src").unwrap();

    // Create test files
    fs.write_file(
        "/test_venv/lib/python3.11/site-packages/requests.py",
        "# This is a third-party module installed in virtual environment",
    )
    .unwrap();
    fs.create_directory_all("/test_venv/lib/python3.11/site-packages/numpy")
        .unwrap();
    fs.write_file(
        "/test_venv/lib/python3.11/site-packages/numpy/__init__.py",
        "# Third-party package",
    )
    .unwrap();
    fs.write_file("/src/mymodule.py", "# This is a first-party module")
        .unwrap();

    // Set up resolver with virtual filesystem
    let config = Config {
        src: vec![PathBuf::from("/src")],
        ..Default::default()
    };
    let resolver = ModuleResolver::new_with_virtualenv_fs(config, "/test_venv", &fs).unwrap();

    // Test import classifications
    assert_eq!(resolver.classify_import("mymodule"), ImportType::FirstParty);
    assert_eq!(resolver.classify_import("requests"), ImportType::ThirdParty);
    assert_eq!(resolver.classify_import("numpy"), ImportType::ThirdParty);
}
```

#### 4.2.2 PYTHONPATH Support Tests

**Before**: Created numerous temporary directories, with complex path handling across platforms.

**After**:

```rust
#[test]
fn test_pythonpath_module_discovery() {
    let mut fs = MemoryFileSystem::new();

    // Create directory structure in memory
    fs.create_directory_all("/pythonpath_modules").unwrap();
    fs.create_directory_all("/src").unwrap();

    // Create test files
    fs.write_file(
        "/pythonpath_modules/pythonpath_module.py",
        "# This is a PYTHONPATH module\ndef hello():\n    return 'Hello from PYTHONPATH'",
    )
    .unwrap();
    fs.create_directory_all("/pythonpath_modules/pythonpath_pkg")
        .unwrap();
    fs.write_file(
        "/pythonpath_modules/pythonpath_pkg/__init__.py",
        "# PYTHONPATH package",
    )
    .unwrap();
    fs.write_file(
        "/pythonpath_modules/pythonpath_pkg/submodule.py",
        "# PYTHONPATH submodule",
    )
    .unwrap();
    fs.write_file("/src/src_module.py", "# This is a src module")
        .unwrap();

    // Set up resolver with virtual filesystem
    let config = Config {
        src: vec![PathBuf::from("/src")],
        ..Default::default()
    };
    let resolver =
        ModuleResolver::new_with_pythonpath_fs(config, "/pythonpath_modules", &fs).unwrap();

    // Test module discovery
    let first_party_modules = resolver.get_first_party_modules();
    assert!(first_party_modules.contains("src_module"));
    assert!(first_party_modules.contains("pythonpath_module"));
    assert!(first_party_modules.contains("pythonpath_pkg"));
    assert!(first_party_modules.contains("pythonpath_pkg.submodule"));
}
```

#### 4.2.3 Stickytape Compatibility Tests

**Before**: Required numerous fixtures and executing bundled scripts with the real Python interpreter.

**After**:

```rust
#[test]
fn test_single_file_script_still_works() {
    let mut fs = MemoryFileSystem::new();

    // Create test script structure in memory
    fs.create_directory_all("/test_scripts/single_file")
        .unwrap();
    fs.write_file("/test_scripts/single_file/hello", "print('Hello')")
        .unwrap();

    // Configure bundler with in-memory filesystem
    let config = Config {
        src: vec![PathBuf::from("/test_scripts/single_file")],
        ..Default::default()
    };
    let mut bundler = Bundler::new_with_filesystem(config, &fs);

    // Bundle the script
    let output_path = PathBuf::from("/bundled_script.py");
    bundler
        .bundle(
            &PathBuf::from("/test_scripts/single_file/hello"),
            &output_path,
            false,
        )
        .unwrap();

    // Verify bundled content
    let bundled_content = fs.read_to_string(&output_path).unwrap();
    assert!(bundled_content.contains("print('Hello')"));

    // Create snapshot for the bundled content
    assert_snapshot!("bundled_single_file", bundled_content);
}
```

#### 4.2.4 Relative Import Tests

**Before**: Required creating temporary directory structures and real paths for import resolution testing.

**After**:

```rust
#[test]
fn test_relative_import_resolution() {
    let mut fs = MemoryFileSystem::new();

    // Create test package structure in memory
    fs.create_directory_all("/test_package").unwrap();
    fs.create_directory_all("/test_package/subpackage").unwrap();

    // Create test files
    fs.write_file(
        "/test_package/main.py",
        r#"# Test file with relative imports
from . import utils  # Should resolve to 'test_package.utils'
from .utils import helper_function  # Should resolve to 'test_package.utils'

def main():
    pass
"#,
    )
    .unwrap();

    fs.write_file("/test_package/utils.py", "def helper_function(): pass")
        .unwrap();

    fs.write_file(
        "/test_package/subpackage/module.py",
        r#"# Test file with relative imports in subpackage
from .. import main  # Should resolve to 'test_package.main'

def sub_function():
    pass
"#,
    )
    .unwrap();

    // Configure bundler with in-memory filesystem
    let config = Config {
        src: vec![PathBuf::from("/")],
        ..Default::default()
    };
    let bundler = Bundler::new_with_filesystem(config, &fs);

    // Test main.py imports
    let imports = bundler
        .extract_imports(&PathBuf::from("/test_package/main.py"))
        .unwrap();
    assert!(!imports.is_empty());
    assert!(imports.contains(&"test_package.utils".to_string()));

    // Test subpackage module imports
    let imports = bundler
        .extract_imports(&PathBuf::from("/test_package/subpackage/module.py"))
        .unwrap();
    assert!(imports.contains(&"test_package.main".to_string()));
}
```

#### 4.2.5 Integration Tests for Bundling

**Before**: Relied on existing test fixtures on disk and complex temporary directory management.

**After**:

```rust
#[test]
fn test_simple_project_bundling() {
    let mut fs = MemoryFileSystem::new();

    // Create test project structure in memory
    fs.create_directory_all("/simple_project/models").unwrap();
    fs.create_directory_all("/simple_project/utils").unwrap();

    // Create project files
    fs.write_file(
        "/simple_project/main.py",
        r#"
from models.user import User
from utils.helpers import format_name

def main():
    user = User("John", "Doe")
    print(format_name(user))

if __name__ == "__main__":
    main()
"#,
    )
    .unwrap();

    fs.write_file("/simple_project/models/__init__.py", "")
        .unwrap();
    fs.write_file(
        "/simple_project/models/user.py",
        r#"
class User:
    def __init__(self, first_name, last_name):
        self.first_name = first_name
        self.last_name = last_name
"#,
    )
    .unwrap();

    fs.write_file("/simple_project/utils/__init__.py", "")
        .unwrap();
    fs.write_file(
        "/simple_project/utils/helpers.py",
        r#"
def format_name(user):
    return f"{user.first_name} {user.last_name}"
"#,
    )
    .unwrap();

    // Configure bundler with in-memory filesystem
    let config = Config {
        src: vec![PathBuf::from("/simple_project")],
        ..Default::default()
    };
    let mut bundler = Bundler::new_with_filesystem(config, &fs);

    // Bundle the project
    let entry_path = PathBuf::from("/simple_project/main.py");
    let output_path = PathBuf::from("/bundle.py");
    bundler.bundle(&entry_path, &output_path, false).unwrap();

    // Verify bundled content
    let content = fs.read_to_string(&output_path).unwrap();
    assert!(content.contains("class User"));
    assert!(content.contains("def format_name"));

    // Create snapshot
    assert_snapshot!("simple_project_bundle", content);
}
```

## 5. Current Limitations and Future Improvements

### 5.1 Current Limitations

1. **Compatibility with Existing Code**:
   - Not all components are fully integrated with the filesystem abstraction
   - Some code still directly uses the standard library filesystem operations

2. **AST Handling**:
   - The rustpython-parser API has changed, requiring adaptations to work with the new API
   - Need to properly handle conversions between AST types

3. **Path Handling**:
   - Path handling and module resolution need improvements for relative imports
   - Case sensitivity handling could be improved for cross-platform consistency

4. **Performance Optimizations**:
   - The in-memory filesystem implementation could be optimized for large file structures
   - Path normalization could be improved for better performance

### 5.2 Future Improvements

1. **Complete Integration**:
   - Update all components to use the filesystem abstraction consistently
   - Create filesystem-compatible versions of all core modules

2. **Enhanced Implementations**:
   - Add a remote filesystem implementation for accessing files over SSH or HTTP
   - Add a caching filesystem implementation for improved performance
   - Add a filtered filesystem implementation for security or sandboxing

3. **Testing Improvements**:
   - Expand test helpers for common testing patterns
   - Add more sophisticated project templates for testing

4. **Documentation**:
   - Improve API documentation for the filesystem abstraction
   - Add more examples and tutorials

5. **Performance Enhancements**:
   - Optimize path normalization and resolution
   - Add caching for frequently accessed files and directories

## 6. Example Usage Patterns

### 6.1 Basic Usage

```rust
use serpen::{BundlerFs, Config, MemoryFileSystem, System, WritableSystem};
use std::path::PathBuf;

// Create in-memory filesystem
let mut fs = MemoryFileSystem::new();

// Set up a project
fs.create_directory("/project")?;
fs.write_file("/project/main.py", "print('Hello, world!')")?;

// Create bundler with in-memory filesystem
let config = Config {
    src: vec![PathBuf::from("/project")],
    ..Config::default()
};
let mut bundler = BundlerFs::new(fs.clone(), config);

// Bundle the project
let entry_path = PathBuf::from("/project/main.py");
let output_path = PathBuf::from("/bundled.py");
bundler.bundle(&entry_path, &output_path, &mut fs, false)?;

// Verify the output
assert!(fs.file_exists(&output_path));
```

### 6.2 Test Fixtures Setup

```rust
// Create in-memory filesystem
let mut fs = MemoryFileSystem::new();

// Create a virtual environment structure
fs.create_directory_all("/test_venv/lib/python3.11/site-packages")?;
fs.create_directory_all("/test_venv/bin")?;
fs.write_file("/test_venv/bin/python", "#!/bin/sh\necho Python")?;

// Create a project structure
let project_path = PathBuf::from("/project");
fs.create_directory(&project_path)?;
fs.create_directory(project_path.join("src"))?;
fs.create_directory(project_path.join("tests"))?;

// Create Python package structure
fs.create_directory(project_path.join("src/mypackage"))?;
fs.write_file(project_path.join("src/mypackage/__init__.py"), "")?;
fs.write_file(project_path.join("src/mypackage/module.py"), "def hello(): return 'Hello'")?;

// Create test module
fs.write_file(project_path.join("tests/test_module.py"), 
    "from mypackage.module import hello\n\ndef test_hello():\n    assert hello() == 'Hello'")?;
```

### 6.3 Integration with Existing Code

```rust
// Create physical filesystem for normal operation
let physical_fs = PhysicalFileSystem::new();

// Create in-memory filesystem for testing
let memory_fs = MemoryFileSystem::new();

// Function that works with either filesystem
fn process_files<FS: System>(fs: &FS, path: &Path) -> Result<()> {
    if !fs.path_exists(path) {
        return Err(anyhow::anyhow!("Path does not exist"));
    }
    
    if fs.is_directory(path) {
        for entry in fs.read_directory(path)? {
            let entry = entry?;
            process_files(fs, &entry.path())?;
        }
    } else if fs.is_file(path) {
        let content = fs.read_to_string(path)?;
        println!("Processing {}: {} bytes", path.display(), content.len());
    }
    
    Ok(())
}

// Use with physical filesystem in production
process_files(&physical_fs, Path::new("/path/to/project")).unwrap();

// Use with memory filesystem in tests
memory_fs.write_file("/test/file.txt", "Test content").unwrap();
process_files(&memory_fs, Path::new("/test")).unwrap();
```

### 6.4 Helper Functions for Testing

```rust
/// Set up a standard test environment with project structure
fn setup_test_environment() -> MemoryFileSystem {
    let mut fs = MemoryFileSystem::new();

    // Create project structure
    fs.create_directory_all("/project/src").unwrap();
    fs.create_directory_all("/project/tests").unwrap();

    // Create main module
    fs.write_file(
        "/project/src/main.py",
        r#"
def main():
    print("Hello, world!")

if __name__ == "__main__":
    main()
"#,
    )
    .unwrap();

    // Create test module
    fs.write_file(
        "/project/tests/test_main.py",
        r#"
from src.main import main

def test_main():
    # Just make sure it runs without error
    main()
"#,
    )
    .unwrap();

    fs
}

#[test]
fn test_bundling() {
    let fs = setup_test_environment();

    // Use the pre-configured filesystem for testing
    let bundler = BundlerFs::new(fs.clone(), Config::default());
    // Test bundling operations...
}
```

## Conclusion

The filesystem abstraction layer in Serpen provides a powerful foundation for testing and extensibility. By separating filesystem operations from business logic, it enables more reliable, faster tests and a more flexible architecture. The in-memory filesystem implementation is particularly valuable for test scenarios, offering significant improvements in speed, reliability, and cross-platform consistency.

While there are still areas for improvement, the current implementation delivers substantial benefits for testing and opens up possibilities for future extensions like remote filesystem support. Continued development of this abstraction will further enhance Serpen's architecture and testing capabilities.
