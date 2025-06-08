# In-Memory Filesystem Implementation Plan for Cribo

This document outlines a detailed step-by-step plan for implementing an in-memory filesystem for Cribo, based on our analysis of Ruff's implementation and our test case review.

## Phase 1: Core Filesystem Trait Design

### Step 1: Define Core Traits

Create a new file `crates/cribo/src/filesystem.rs` with the following trait definitions:

```rust
use std::io;
use std::path::{Path, PathBuf};

/// Core trait for read-only filesystem operations
pub trait System {
    /// Check if a file exists at the given path
    fn file_exists<P: AsRef<Path>>(&self, path: P) -> bool;

    /// Check if a directory exists at the given path
    fn directory_exists<P: AsRef<Path>>(&self, path: P) -> bool;

    /// Read the contents of a file as bytes
    fn read_file<P: AsRef<Path>>(&self, path: P) -> io::Result<Vec<u8>>;

    /// Read the contents of a file as a UTF-8 string
    fn read_file_str<P: AsRef<Path>>(&self, path: P) -> io::Result<String> {
        let bytes = self.read_file(path)?;
        String::from_utf8(bytes).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
    }

    /// List entries in a directory
    fn read_directory<P: AsRef<Path>>(&self, path: P) -> io::Result<Vec<PathBuf>>;

    /// Canonicalize a path, resolving any symlinks and relative components
    fn canonicalize<P: AsRef<Path>>(&self, path: P) -> io::Result<PathBuf>;

    /// Join a path with a base directory, ensuring the result is within the base
    fn join_path<P1: AsRef<Path>, P2: AsRef<Path>>(&self, base: P1, path: P2) -> PathBuf {
        let base = base.as_ref();
        let joined = base.join(path);
        joined
    }
}

/// Extended trait for writable filesystem operations
pub trait WritableSystem: System {
    /// Create a directory and any parent directories that don't exist
    fn create_directory<P: AsRef<Path>>(&mut self, path: P) -> io::Result<()>;

    /// Write data to a file, creating the file if it doesn't exist
    fn write_file<P: AsRef<Path>, C: AsRef<[u8]>>(
        &mut self,
        path: P,
        contents: C,
    ) -> io::Result<()>;

    /// Delete a file
    fn remove_file<P: AsRef<Path>>(&mut self, path: P) -> io::Result<()>;

    /// Delete a directory and all its contents recursively
    fn remove_directory<P: AsRef<Path>>(&mut self, path: P) -> io::Result<()>;
}
```

### Step 2: Implement Physical Filesystem

Create a wrapper for the physical filesystem:

```rust
pub struct PhysicalFileSystem;

impl System for PhysicalFileSystem {
    fn file_exists<P: AsRef<Path>>(&self, path: P) -> bool {
        path.as_ref().is_file()
    }

    fn directory_exists<P: AsRef<Path>>(&self, path: P) -> bool {
        path.as_ref().is_dir()
    }

    fn read_file<P: AsRef<Path>>(&self, path: P) -> io::Result<Vec<u8>> {
        std::fs::read(path)
    }

    fn read_directory<P: AsRef<Path>>(&self, path: P) -> io::Result<Vec<PathBuf>> {
        let entries = std::fs::read_dir(path)?;
        let mut paths = Vec::new();

        for entry in entries {
            let entry = entry?;
            paths.push(entry.path());
        }

        Ok(paths)
    }

    fn canonicalize<P: AsRef<Path>>(&self, path: P) -> io::Result<PathBuf> {
        std::fs::canonicalize(path)
    }
}

impl WritableSystem for PhysicalFileSystem {
    fn create_directory<P: AsRef<Path>>(&mut self, path: P) -> io::Result<()> {
        std::fs::create_dir_all(path)
    }

    fn write_file<P: AsRef<Path>, C: AsRef<[u8]>>(
        &mut self,
        path: P,
        contents: C,
    ) -> io::Result<()> {
        // Create parent directory if it doesn't exist
        if let Some(parent) = path.as_ref().parent() {
            if !parent.exists() {
                std::fs::create_dir_all(parent)?;
            }
        }
        std::fs::write(path, contents)
    }

    fn remove_file<P: AsRef<Path>>(&mut self, path: P) -> io::Result<()> {
        std::fs::remove_file(path)
    }

    fn remove_directory<P: AsRef<Path>>(&mut self, path: P) -> io::Result<()> {
        std::fs::remove_dir_all(path)
    }
}
```

## Phase 2: In-Memory Filesystem Implementation

### Step 3: Define Core Data Structures

```rust
use std::collections::HashMap;
use std::io;
use std::path::{Component, Path, PathBuf};
use std::sync::RwLock;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone)]
pub enum FileEntry {
    File {
        content: Vec<u8>,
        created: SystemTime,
        modified: SystemTime,
    },
    Directory {
        created: SystemTime,
        modified: SystemTime,
    },
}

impl FileEntry {
    fn new_file(content: Vec<u8>) -> Self {
        let now = SystemTime::now();
        FileEntry::File {
            content,
            created: now,
            modified: now,
        }
    }

    fn new_directory() -> Self {
        let now = SystemTime::now();
        FileEntry::Directory {
            created: now,
            modified: now,
        }
    }

    fn is_file(&self) -> bool {
        matches!(self, FileEntry::File { .. })
    }

    fn is_directory(&self) -> bool {
        matches!(self, FileEntry::Directory { .. })
    }
}

pub struct MemoryFileSystem {
    entries: HashMap<PathBuf, FileEntry>,
    working_directory: PathBuf,
    case_sensitive: bool,
}
```

### Step 4: Implement Basic In-Memory Functionality

```rust
impl MemoryFileSystem {
    pub fn new() -> Self {
        let mut fs = MemoryFileSystem {
            entries: HashMap::new(),
            working_directory: PathBuf::from("/"),
            case_sensitive: true,
        };

        // Add root directory
        fs.entries
            .insert(PathBuf::from("/"), FileEntry::new_directory());
        fs
    }

    pub fn with_case_sensitivity(mut self, case_sensitive: bool) -> Self {
        self.case_sensitive = case_sensitive;
        self
    }

    pub fn with_working_directory<P: AsRef<Path>>(mut self, path: P) -> Self {
        self.working_directory = self.normalize_path(path.as_ref().to_path_buf());
        self
    }

    fn normalize_path(&self, path: PathBuf) -> PathBuf {
        let path_str = path.to_string_lossy();

        // Handle absolute vs relative paths
        let path = if path_str.starts_with('/') {
            path
        } else {
            self.working_directory.join(path)
        };

        // Normalize by resolving . and .. components
        let mut normalized = PathBuf::new();

        for component in path.components() {
            match component {
                Component::Prefix(_) => {}
                Component::RootDir => normalized.push("/"),
                Component::CurDir => {}
                Component::ParentDir => {
                    normalized.pop();
                }
                Component::Normal(name) => normalized.push(name),
            }
        }

        if normalized.as_os_str().is_empty() {
            PathBuf::from("/")
        } else {
            normalized
        }
    }

    fn get_normalized_path_key<P: AsRef<Path>>(&self, path: P) -> PathBuf {
        let normalized = self.normalize_path(path.as_ref().to_path_buf());

        if self.case_sensitive {
            normalized
        } else {
            // For case-insensitive systems, convert to lowercase
            PathBuf::from(normalized.to_string_lossy().to_lowercase())
        }
    }

    fn get_parent_dirs(&self, path: &Path) -> Vec<PathBuf> {
        let mut parent_dirs = Vec::new();
        let mut current = path.to_path_buf();

        while let Some(parent) = current.parent() {
            if parent.as_os_str().is_empty() {
                break;
            }

            parent_dirs.push(parent.to_path_buf());
            current = parent.to_path_buf();
        }

        parent_dirs
    }

    fn ensure_parent_directories<P: AsRef<Path>>(&mut self, path: P) -> io::Result<()> {
        let path = path.as_ref();

        if let Some(parent) = path.parent() {
            if parent.as_os_str().is_empty() {
                return Ok(());
            }

            let normalized_parent = self.normalize_path(parent.to_path_buf());

            if !self.directory_exists(&normalized_parent) {
                let parents = self.get_parent_dirs(&normalized_parent);

                // Create parent directories in reverse order (from root to leaf)
                for parent_dir in parents.iter().rev() {
                    if !self.directory_exists(parent_dir) {
                        self.entries.insert(
                            self.get_normalized_path_key(parent_dir),
                            FileEntry::new_directory(),
                        );
                    }
                }

                self.entries.insert(
                    self.get_normalized_path_key(normalized_parent),
                    FileEntry::new_directory(),
                );
            }
        }

        Ok(())
    }
}
```

### Step 5: Implement System Trait for Memory Filesystem

```rust
impl System for MemoryFileSystem {
    fn file_exists<P: AsRef<Path>>(&self, path: P) -> bool {
        let key = self.get_normalized_path_key(path);
        self.entries
            .get(&key)
            .map_or(false, |entry| entry.is_file())
    }

    fn directory_exists<P: AsRef<Path>>(&self, path: P) -> bool {
        let key = self.get_normalized_path_key(path);
        self.entries
            .get(&key)
            .map_or(false, |entry| entry.is_directory())
    }

    fn read_file<P: AsRef<Path>>(&self, path: P) -> io::Result<Vec<u8>> {
        let key = self.get_normalized_path_key(path.as_ref());

        match self.entries.get(&key) {
            Some(FileEntry::File { content, .. }) => Ok(content.clone()),
            Some(FileEntry::Directory { .. }) => Err(io::Error::new(
                io::ErrorKind::IsADirectory,
                "Is a directory",
            )),
            None => Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("File not found: {}", path.as_ref().display()),
            )),
        }
    }

    fn read_directory<P: AsRef<Path>>(&self, path: P) -> io::Result<Vec<PathBuf>> {
        let normalized_path = self.normalize_path(path.as_ref().to_path_buf());
        let key = self.get_normalized_path_key(&normalized_path);

        if !self.directory_exists(&key) {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("Directory not found: {}", path.as_ref().display()),
            ));
        }

        let path_str = normalized_path.to_string_lossy().to_string();
        let path_prefix = if path_str.ends_with('/') {
            path_str
        } else {
            format!("{}/", path_str)
        };

        let mut entries = Vec::new();

        for entry_path in self.entries.keys() {
            let entry_str = entry_path.to_string_lossy();

            // If entry is directly under the requested directory
            if entry_str.starts_with(&path_prefix) {
                let remaining = &entry_str[path_prefix.len()..];
                if !remaining.is_empty() && !remaining.contains('/') {
                    entries.push(entry_path.clone());
                }
            }
        }

        Ok(entries)
    }

    fn canonicalize<P: AsRef<Path>>(&self, path: P) -> io::Result<PathBuf> {
        let normalized = self.normalize_path(path.as_ref().to_path_buf());

        if self.file_exists(&normalized) || self.directory_exists(&normalized) {
            Ok(normalized)
        } else {
            Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("Path not found: {}", path.as_ref().display()),
            ))
        }
    }
}
```

### Step 6: Implement WritableSystem Trait

```rust
impl WritableSystem for MemoryFileSystem {
    fn create_directory<P: AsRef<Path>>(&mut self, path: P) -> io::Result<()> {
        let path = path.as_ref();
        let normalized = self.normalize_path(path.to_path_buf());
        let key = self.get_normalized_path_key(&normalized);

        if self.file_exists(&key) {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "A file with this name already exists",
            ));
        }

        if !self.directory_exists(&key) {
            self.ensure_parent_directories(&normalized)?;
            self.entries.insert(key, FileEntry::new_directory());
        }

        Ok(())
    }

    fn write_file<P: AsRef<Path>, C: AsRef<[u8]>>(
        &mut self,
        path: P,
        contents: C,
    ) -> io::Result<()> {
        let path = path.as_ref();
        let normalized = self.normalize_path(path.to_path_buf());
        let key = self.get_normalized_path_key(&normalized);

        if self.directory_exists(&key) {
            return Err(io::Error::new(
                io::ErrorKind::IsADirectory,
                "Cannot write to a directory",
            ));
        }

        self.ensure_parent_directories(&normalized)?;

        self.entries
            .insert(key, FileEntry::new_file(contents.as_ref().to_vec()));

        Ok(())
    }

    fn remove_file<P: AsRef<Path>>(&mut self, path: P) -> io::Result<()> {
        let path = path.as_ref();
        let key = self.get_normalized_path_key(path);

        if !self.file_exists(&key) {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("File not found: {}", path.display()),
            ));
        }

        self.entries.remove(&key);
        Ok(())
    }

    fn remove_directory<P: AsRef<Path>>(&mut self, path: P) -> io::Result<()> {
        let path = path.as_ref();
        let normalized = self.normalize_path(path.to_path_buf());
        let key = self.get_normalized_path_key(&normalized);

        if !self.directory_exists(&key) {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("Directory not found: {}", path.display()),
            ));
        }

        let path_str = normalized.to_string_lossy().to_string();
        let path_prefix = if path_str.ends_with('/') {
            path_str
        } else {
            format!("{}/", path_str)
        };

        // Remove all entries under this directory
        let keys_to_remove: Vec<PathBuf> = self
            .entries
            .keys()
            .filter(|entry_path| {
                let entry_str = entry_path.to_string_lossy();
                entry_str == path_str || entry_str.starts_with(&path_prefix)
            })
            .cloned()
            .collect();

        for key in keys_to_remove {
            self.entries.remove(&key);
        }

        Ok(())
    }
}
```

## Phase 3: Utility Functions and Helpers

### Step 7: Add Helper Functions for Test Setup

```rust
impl MemoryFileSystem {
    /// Create a fixture directory structure with files and their contents
    pub fn create_fixture<P: AsRef<Path>>(
        &mut self,
        base_path: P,
        fixtures: &[(&str, Option<&str>)],
    ) -> io::Result<()> {
        let base = base_path.as_ref();

        for (rel_path, content) in fixtures {
            let path = base.join(rel_path);

            if let Some(content) = content {
                // It's a file with content
                self.write_file(path, content.as_bytes())?;
            } else {
                // It's a directory
                self.create_directory(path)?;
            }
        }

        Ok(())
    }

    /// Create a Python package structure with __init__.py files
    pub fn create_python_package<P: AsRef<Path>>(
        &mut self,
        base_path: P,
        package_paths: &[&str],
    ) -> io::Result<()> {
        let base = base_path.as_ref();

        for pkg_path in package_paths {
            let path = base.join(pkg_path);
            self.create_directory(&path)?;

            // Add __init__.py file
            let init_path = path.join("__init__.py");
            self.write_file(init_path, b"")?;
        }

        Ok(())
    }

    /// Create a virtual environment structure
    pub fn create_virtual_environment<P: AsRef<Path>>(
        &mut self,
        venv_path: P,
        python_version: &str,
    ) -> io::Result<()> {
        let venv = venv_path.as_ref();

        // Create base venv directories
        for dir in &["bin", "include", "lib", "lib/python"] {
            self.create_directory(venv.join(dir))?;
        }

        // Create lib/python3.x directory
        let python_lib_dir = format!("lib/python{}", python_version);
        self.create_directory(venv.join(&python_lib_dir))?;

        // Create site-packages directory
        let site_packages = format!("{}/site-packages", python_lib_dir);
        self.create_directory(venv.join(&site_packages))?;

        // Create python executable in bin
        #[cfg(unix)]
        self.write_file(
            venv.join("bin/python"),
            b"#!/bin/sh\necho Python executable",
        )?;
        #[cfg(unix)]
        self.write_file(
            venv.join("bin/python3"),
            b"#!/bin/sh\necho Python executable",
        )?;

        #[cfg(windows)]
        self.write_file(
            venv.join("Scripts/python.exe"),
            b"Windows Python executable",
        )?;
        #[cfg(windows)]
        self.write_file(
            venv.join("Scripts/python3.exe"),
            b"Windows Python executable",
        )?;

        // Create activation scripts
        #[cfg(unix)]
        self.write_file(
            venv.join("bin/activate"),
            format!(
                "export VIRTUAL_ENV=\"{}\"\nPATH=\"$VIRTUAL_ENV/bin:$PATH\"\n",
                venv.display()
            )
            .as_bytes(),
        )?;

        #[cfg(windows)]
        self.write_file(
            venv.join("Scripts/activate.bat"),
            format!(
                "@echo off\nset \"VIRTUAL_ENV={}\"\nset \"PATH=%VIRTUAL_ENV%\\Scripts;%PATH%\"\n",
                venv.display()
            )
            .as_bytes(),
        )?;

        Ok(())
    }
}
```

### Step 8: Add Thread-Safe Shared Filesystem

```rust
/// Thread-safe shared memory filesystem
pub struct SharedMemoryFileSystem {
    inner: RwLock<MemoryFileSystem>,
}

impl SharedMemoryFileSystem {
    pub fn new() -> Self {
        SharedMemoryFileSystem {
            inner: RwLock::new(MemoryFileSystem::new()),
        }
    }

    pub fn with_case_sensitivity(self, case_sensitive: bool) -> Self {
        let mut fs = self.inner.write().unwrap();
        *fs = fs.clone().with_case_sensitivity(case_sensitive);
        self
    }

    pub fn with_working_directory<P: AsRef<Path>>(self, path: P) -> Self {
        let mut fs = self.inner.write().unwrap();
        *fs = fs.clone().with_working_directory(path);
        self
    }
}

impl System for SharedMemoryFileSystem {
    fn file_exists<P: AsRef<Path>>(&self, path: P) -> bool {
        self.inner.read().unwrap().file_exists(path)
    }

    fn directory_exists<P: AsRef<Path>>(&self, path: P) -> bool {
        self.inner.read().unwrap().directory_exists(path)
    }

    fn read_file<P: AsRef<Path>>(&self, path: P) -> io::Result<Vec<u8>> {
        self.inner.read().unwrap().read_file(path)
    }

    fn read_directory<P: AsRef<Path>>(&self, path: P) -> io::Result<Vec<PathBuf>> {
        self.inner.read().unwrap().read_directory(path)
    }

    fn canonicalize<P: AsRef<Path>>(&self, path: P) -> io::Result<PathBuf> {
        self.inner.read().unwrap().canonicalize(path)
    }
}

impl WritableSystem for SharedMemoryFileSystem {
    fn create_directory<P: AsRef<Path>>(&mut self, path: P) -> io::Result<()> {
        self.inner.write().unwrap().create_directory(path)
    }

    fn write_file<P: AsRef<Path>, C: AsRef<[u8]>>(
        &mut self,
        path: P,
        contents: C,
    ) -> io::Result<()> {
        self.inner.write().unwrap().write_file(path, contents)
    }

    fn remove_file<P: AsRef<Path>>(&mut self, path: P) -> io::Result<()> {
        self.inner.write().unwrap().remove_file(path)
    }

    fn remove_directory<P: AsRef<Path>>(&mut self, path: P) -> io::Result<()> {
        self.inner.write().unwrap().remove_directory(path)
    }
}
```

## Phase 4: Refactoring Cribo for Filesystem Abstraction

### Step 9: Update Dependency Graph and Resolver

Update `dependency_graph.rs` and `resolver.rs` to use the filesystem abstraction:

```rust
// dependency_graph.rs

use crate::filesystem::System;

pub struct DependencyGraph<'a, FS: System> {
    filesystem: &'a FS,
    // Other fields...
}

impl<'a, FS: System> DependencyGraph<'a, FS> {
    pub fn new(filesystem: &'a FS /* other params */) -> Self {
        Self {
            filesystem,
            // Initialize other fields...
        }
    }

    // Update methods to use filesystem...
}
```

```rust
// resolver.rs

use crate::filesystem::System;

pub struct ModuleResolver<'a, FS: System> {
    filesystem: &'a FS,
    // Other fields...
}

impl<'a, FS: System> ModuleResolver<'a, FS> {
    pub fn new(filesystem: &'a FS /* other params */) -> Self {
        Self {
            filesystem,
            // Initialize other fields...
        }
    }

    // Update file existence checks and reading
    pub fn resolve_module(
        &self,
        name: &str, /* other params */
    ) -> Result<ModuleResolution, ResolverError> {
        // Instead of using std::fs directly:
        if self.filesystem.file_exists(possible_path) {
            let content = self.filesystem.read_file_str(possible_path)?;
            // Process content...
        }

        // Rest of implementation...
    }

    // Other methods...
}
```

### Step 10: Update Bundler and Main Entry Points

```rust
// bundler.rs

use crate::filesystem::{PhysicalFileSystem, System};

pub struct Bundler<FS: System = PhysicalFileSystem> {
    filesystem: FS,
    // Other fields...
}

impl<FS: System> Bundler<FS> {
    pub fn new(filesystem: FS /* other params */) -> Self {
        Self {
            filesystem,
            // Initialize other fields...
        }
    }

    // Update methods to use filesystem...
}

// Default implementation using physical filesystem
impl Default for Bundler<PhysicalFileSystem> {
    fn default() -> Self {
        Self::new(PhysicalFileSystem /* default params */)
    }
}
```

```rust
// main.rs

use crate::filesystem::PhysicalFileSystem;

fn main() {
    // Use physical filesystem by default for CLI
    let filesystem = PhysicalFileSystem;
    let bundler = Bundler::new(filesystem /* params */);

    // Rest of implementation...
}
```

## Phase 5: Test Infrastructure Updates

### Step 11: Create Test Utilities Module

Create `crates/cribo/src/test_utils.rs`:

```rust
use crate::filesystem::{MemoryFileSystem, WritableSystem};
use std::path::{Path, PathBuf};

pub fn setup_test_filesystem() -> MemoryFileSystem {
    MemoryFileSystem::new()
}

pub fn setup_simple_project(fs: &mut MemoryFileSystem) -> PathBuf {
    let project_path = PathBuf::from("/simple_project");

    // Create basic project structure
    fs.create_directory(&project_path).unwrap();
    fs.create_directory(project_path.join("models")).unwrap();
    fs.create_directory(project_path.join("utils")).unwrap();

    // Create __init__.py files
    fs.write_file(project_path.join("__init__.py"), b"")
        .unwrap();
    fs.write_file(project_path.join("models/__init__.py"), b"")
        .unwrap();
    fs.write_file(project_path.join("utils/__init__.py"), b"")
        .unwrap();

    // Create main.py
    fs.write_file(
        project_path.join("main.py"),
        b"from models.user import User\nfrom utils.helpers import format_name\n\ndef main():\n    user = User('John', 'Doe')\n    print(format_name(user))\n\nif __name__ == '__main__':\n    main()\n"
    ).unwrap();

    // Create models/user.py
    fs.write_file(
        project_path.join("models/user.py"),
        b"class User:\n    def __init__(self, first_name, last_name):\n        self.first_name = first_name\n        self.last_name = last_name\n"
    ).unwrap();

    // Create utils/helpers.py
    fs.write_file(
        project_path.join("utils/helpers.py"),
        b"def format_name(user):\n    return f\"{user.first_name} {user.last_name}\"\n",
    )
    .unwrap();

    project_path
}

pub fn setup_virtual_env(fs: &mut MemoryFileSystem) -> PathBuf {
    let venv_path = PathBuf::from("/venv");

    // Create virtual environment structure
    fs.create_virtual_environment(&venv_path, "3.9").unwrap();

    // Add some site-packages
    let site_packages = venv_path.join("lib/python3.9/site-packages");

    // Add a third-party package
    fs.create_directory(site_packages.join("requests")).unwrap();
    fs.write_file(site_packages.join("requests/__init__.py"), b"")
        .unwrap();
    fs.write_file(
        site_packages.join("requests/api.py"),
        b"def get(url, **kwargs):\n    pass\n",
    )
    .unwrap();

    venv_path
}
```

### Step 12: Update Test Modules to Use In-Memory Filesystem

Update `crates/cribo/tests/test_virtualenv_support.rs`:

```rust
use cribo::filesystem::MemoryFileSystem;
use cribo::test_utils::{setup_test_filesystem, setup_virtual_env};
use std::path::PathBuf;

#[test]
fn test_virtualenv_detection() {
    let mut fs = setup_test_filesystem();
    let venv_path = setup_virtual_env(&mut fs);

    // Set up a project that uses the virtualenv
    let project_path = PathBuf::from("/project");
    fs.create_directory(&project_path).unwrap();
    fs.write_file(
        project_path.join("main.py"),
        b"import requests\n\nrequests.get('https://example.com')\n",
    )
    .unwrap();

    // Test virtualenv detection logic with the filesystem
    let bundler = cribo::bundler::Bundler::new(fs /* params */);

    // Set VIRTUAL_ENV environment variable in the test
    std::env::set_var("VIRTUAL_ENV", venv_path.to_string_lossy().to_string());

    // Rest of test...
}
```

## Phase 6: Documentation and Examples

### Step 13: Add Usage Documentation

Create a documentation file explaining how to use the in-memory filesystem:

````markdown
# Using the In-Memory Filesystem in Cribo

The in-memory filesystem provides a way to test Cribo's functionality without relying on the physical filesystem. This is particularly useful for:

- Unit testing that needs to be fast and isolated
- Testing edge cases that are difficult to set up with real files
- Creating reproducible test environments across different platforms

## Basic Usage

```rust
use cribo::filesystem::{MemoryFileSystem, System, WritableSystem};
use std::path::PathBuf;

// Create a new in-memory filesystem
let mut fs = MemoryFileSystem::new();

// Create directories
fs.create_directory("/project").unwrap();
fs.create_directory("/project/src").unwrap();

// Write files
fs.write_file("/project/src/main.py", "print('Hello, world!')").unwrap();

// Read files
let content = fs.read_file_str("/project/src/main.py").unwrap();
assert_eq!(content, "print('Hello, world!')");

// Use with Cribo's bundler
let bundler = cribo::Bundler::new(fs, /* other params */);
let result = bundler.bundle("/project/src/main.py").unwrap();
```
````

## Setting Up Test Fixtures

The `MemoryFileSystem` provides helper methods for setting up common test fixtures:

```rust
// Create a virtual environment
let venv_path = PathBuf::from("/venv");
fs.create_virtual_environment(&venv_path, "3.9").unwrap();

// Create a Python package structure
fs.create_python_package("/project", &["models", "utils"]).unwrap();

// Create multiple files at once
fs.create_fixture("/project", &[
    ("main.py", Some("print('Hello, world!')")),
    ("models/__init__.py", Some("")),
    ("models/user.py", Some("class User:\n    pass")),
    ("utils/__init__.py", Some("")),
    ("utils/helpers.py", Some("def helper():\n    pass")),
]).unwrap();
```

## Thread-Safe Usage

For concurrent tests, use the `SharedMemoryFileSystem`:

```rust
use cribo::filesystem::SharedMemoryFileSystem;

let fs = SharedMemoryFileSystem::new();

// Can be safely shared between threads
let fs_clone = fs.clone();
let handle = std::thread::spawn(move || {
    // Use fs_clone in this thread
});
```

```
## Conclusion and Migration Strategy

This implementation plan outlines a comprehensive approach to adding an in-memory filesystem to Cribo. The key benefits of this approach include:

1. **Abstraction Layer**: By introducing filesystem traits, we separate the concerns of file operations from business logic.

2. **Test Isolation**: Tests can run in a completely isolated environment without side effects or dependencies on the local filesystem.

3. **Cross-Platform Consistency**: In-memory filesystem behavior is consistent across operating systems, avoiding issues with path separators or case-sensitivity.

4. **Performance**: Tests will run significantly faster without disk I/O overhead.

5. **Flexibility**: The modular design allows for easy extension to support other filesystem implementations (e.g., remote filesystems, encrypted filesystems).

### Migration Strategy

1. Start by implementing the core filesystem traits and the in-memory implementation
2. Add the PhysicalFileSystem implementation to ensure compatibility with existing code
3. Update core modules one by one to use the filesystem abstraction
4. Refactor tests to use the in-memory filesystem, starting with the test cases identified as having the most benefit
5. Update documentation and examples to reflect the new approach

By following this phased approach, we can ensure a smooth transition to the new architecture while maintaining compatibility with existing code.
```
