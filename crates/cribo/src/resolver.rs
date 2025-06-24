use std::{
    cell::RefCell,
    ffi::OsStr,
    path::{Path, PathBuf},
};

use anyhow::{Result, anyhow};
use indexmap::{IndexMap, IndexSet};
use log::{debug, warn};
use ruff_python_stdlib::sys;

use crate::config::Config;

/// Check if a module is part of the Python standard library using ruff_python_stdlib
fn is_stdlib_module(module_name: &str, python_version: u8) -> bool {
    // Check direct match using ruff_python_stdlib
    if sys::is_known_standard_library(python_version, module_name) {
        return true;
    }

    // Check if it's a submodule of a stdlib module
    if let Some(top_level) = module_name.split('.').next() {
        sys::is_known_standard_library(python_version, top_level)
    } else {
        false
    }
}

/// A scoped guard for safely setting and cleaning up the PYTHONPATH environment variable.
///
/// This guard ensures that the PYTHONPATH environment variable is properly restored
/// to its original value when the guard is dropped, even if a panic occurs during testing.
///
/// # Example
///
/// ```rust
/// use cribo::resolver::PythonPathGuard;
/// let _guard = PythonPathGuard::new("/tmp/test");
/// // PYTHONPATH is now set to "/tmp/test"
/// // When _guard goes out of scope, PYTHONPATH is restored to its original value
/// ```
#[must_use = "PythonPathGuard must be held in scope to ensure cleanup"]
pub struct PythonPathGuard {
    /// The original value of PYTHONPATH, if it was set
    /// None if PYTHONPATH was not set originally
    original_value: Option<String>,
}

/// A scoped guard for safely setting and cleaning up the VIRTUAL_ENV environment variable.
///
/// This guard ensures that the VIRTUAL_ENV environment variable is properly restored
/// to its original value when the guard is dropped, even if a panic occurs during testing.
///
/// # Example
///
/// ```rust
/// use cribo::resolver::VirtualEnvGuard;
/// let _guard = VirtualEnvGuard::new("/path/to/venv");
/// // VIRTUAL_ENV is now set to "/path/to/venv"
/// // When _guard goes out of scope, VIRTUAL_ENV is restored to its original value
/// ```
#[must_use = "VirtualEnvGuard must be held in scope to ensure cleanup"]
pub struct VirtualEnvGuard {
    /// The original value of VIRTUAL_ENV, if it was set
    /// None if VIRTUAL_ENV was not set originally
    original_value: Option<String>,
}

impl PythonPathGuard {
    /// Create a new PYTHONPATH guard with the given value.
    ///
    /// This will set the PYTHONPATH environment variable to the specified value
    /// and store the original value for restoration when the guard is dropped.
    pub fn new(new_value: &str) -> Self {
        let original_value = std::env::var("PYTHONPATH").ok();

        // SAFETY: This is safe in test contexts where we control the environment
        // and ensure proper cleanup via the Drop trait.
        unsafe {
            std::env::set_var("PYTHONPATH", new_value);
        }

        Self { original_value }
    }

    /// Create a new PYTHONPATH guard that ensures PYTHONPATH is unset.
    ///
    /// This will remove the PYTHONPATH environment variable and store the
    /// original value for restoration when the guard is dropped.
    pub fn unset() -> Self {
        let original_value = std::env::var("PYTHONPATH").ok();

        // SAFETY: This is safe in test contexts where we control the environment
        // and ensure proper cleanup via the Drop trait.
        unsafe {
            std::env::remove_var("PYTHONPATH");
        }

        Self { original_value }
    }
}

impl Drop for PythonPathGuard {
    fn drop(&mut self) {
        // Always attempt cleanup, even during panics - that's the whole point of a scope guard!
        // We catch and ignore any errors to prevent double panics, but we must try to clean up.
        #[allow(clippy::disallowed_methods)]
        // catch_unwind is necessary here to prevent double panics during cleanup
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            // SAFETY: This is safe as we're restoring the environment to its original state
            unsafe {
                match self.original_value.take() {
                    Some(original) => std::env::set_var("PYTHONPATH", original),
                    None => std::env::remove_var("PYTHONPATH"),
                }
            }
        }));
    }
}

impl VirtualEnvGuard {
    /// Create a new VIRTUAL_ENV guard with the given value.
    ///
    /// This will set the VIRTUAL_ENV environment variable to the specified value
    /// and store the original value for restoration when the guard is dropped.
    pub fn new(new_value: &str) -> Self {
        let original_value = std::env::var("VIRTUAL_ENV").ok();

        // SAFETY: This is safe in test contexts where we control the environment
        // and ensure proper cleanup via the Drop trait.
        unsafe {
            std::env::set_var("VIRTUAL_ENV", new_value);
        }

        Self { original_value }
    }

    /// Create a new VIRTUAL_ENV guard that ensures VIRTUAL_ENV is unset.
    ///
    /// This will remove the VIRTUAL_ENV environment variable and store the
    /// original value for restoration when the guard is dropped.
    pub fn unset() -> Self {
        let original_value = std::env::var("VIRTUAL_ENV").ok();

        // SAFETY: This is safe in test contexts where we control the environment
        // and ensure proper cleanup via the Drop trait.
        unsafe {
            std::env::remove_var("VIRTUAL_ENV");
        }

        Self { original_value }
    }
}

impl Drop for VirtualEnvGuard {
    fn drop(&mut self) {
        // Always attempt cleanup, even during panics - that's the whole point of a scope guard!
        // We catch and ignore any errors to prevent double panics, but we must try to clean up.
        #[allow(clippy::disallowed_methods)]
        // catch_unwind is necessary here to prevent double panics during cleanup
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            // SAFETY: This is safe as we're restoring the environment to its original state
            unsafe {
                match self.original_value.take() {
                    Some(original) => std::env::set_var("VIRTUAL_ENV", original),
                    None => std::env::remove_var("VIRTUAL_ENV"),
                }
            }
        }));
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ImportType {
    FirstParty,
    ThirdParty,
    StandardLibrary,
}

/// Module descriptor for import resolution
#[derive(Debug)]
struct ImportModuleDescriptor {
    /// Number of leading dots for relative imports
    leading_dots: usize,
    /// Module name parts (e.g., ["foo", "bar"] for "foo.bar")
    name_parts: Vec<String>,
}

impl ImportModuleDescriptor {
    fn from_module_name(name: &str) -> Self {
        let leading_dots = name.chars().take_while(|c| *c == '.').count();
        let name_parts = name
            .chars()
            .skip_while(|c| *c == '.')
            .collect::<String>()
            .split('.')
            .filter(|s| !s.is_empty())
            .map(String::from)
            .collect();

        Self {
            leading_dots,
            name_parts,
        }
    }
}

#[derive(Debug)]
pub struct ModuleResolver {
    config: Config,
    /// Cache of resolved module paths
    module_cache: IndexMap<String, Option<PathBuf>>,
    /// Cache of module classifications
    classification_cache: IndexMap<String, ImportType>,
    /// Cache of virtual environment packages to avoid repeated filesystem scans
    virtualenv_packages_cache: RefCell<Option<IndexSet<String>>>,
    /// Entry file's directory (first in search path)
    entry_dir: Option<PathBuf>,
    /// Python version for stdlib classification
    python_version: u8,
    /// PYTHONPATH override for testing
    pythonpath_override: Option<String>,
    /// VIRTUAL_ENV override for testing
    virtualenv_override: Option<String>,
}

impl ModuleResolver {
    /// Canonicalize a path, handling errors gracefully
    fn canonicalize_path(&self, path: PathBuf) -> PathBuf {
        match path.canonicalize() {
            Ok(canonical) => canonical,
            Err(e) => {
                // Log warning but don't fail - return the original path
                warn!("Failed to canonicalize path {}: {}", path.display(), e);
                path
            }
        }
    }

    pub fn new(config: Config) -> Result<Self> {
        Self::new_with_overrides(config, None, None)
    }

    /// Create a new ModuleResolver with optional PYTHONPATH override for testing
    pub fn new_with_pythonpath(config: Config, pythonpath_override: Option<&str>) -> Result<Self> {
        Self::new_with_overrides(config, pythonpath_override, None)
    }

    /// Create a new ModuleResolver with optional VIRTUAL_ENV override for testing
    pub fn new_with_virtualenv(config: Config, virtualenv_override: Option<&str>) -> Result<Self> {
        Self::new_with_overrides(config, None, virtualenv_override)
    }

    /// Create a new ModuleResolver with optional PYTHONPATH and VIRTUAL_ENV overrides for testing
    pub fn new_with_overrides(
        config: Config,
        pythonpath_override: Option<&str>,
        virtualenv_override: Option<&str>,
    ) -> Result<Self> {
        Ok(Self {
            config,
            module_cache: IndexMap::new(),
            classification_cache: IndexMap::new(),
            virtualenv_packages_cache: RefCell::new(None),
            entry_dir: None,
            python_version: 38, // Default to Python 3.8
            pythonpath_override: pythonpath_override.map(|s| s.to_string()),
            virtualenv_override: virtualenv_override.map(|s| s.to_string()),
        })
    }

    /// Set the entry file for the resolver
    /// This establishes the first search path directory
    pub fn set_entry_file(&mut self, entry_path: &Path) {
        if let Some(parent) = entry_path.parent() {
            self.entry_dir = Some(parent.to_path_buf());
            debug!("Set entry directory to: {:?}", self.entry_dir);
        }
    }

    /// Get all directories to search for modules
    /// Per docs/resolution.md: Entry file's directory is always first
    pub fn get_search_directories(&self) -> Vec<PathBuf> {
        let pythonpath = self.pythonpath_override.as_deref();
        let virtualenv = self.virtualenv_override.as_deref();
        self.get_search_directories_with_overrides(pythonpath, virtualenv)
    }

    /// Get all directories to search for modules with optional PYTHONPATH override
    pub fn get_search_directories_with_pythonpath(
        &self,
        pythonpath_override: Option<&str>,
    ) -> Vec<PathBuf> {
        let pythonpath = pythonpath_override.or(self.pythonpath_override.as_deref());
        let virtualenv = self.virtualenv_override.as_deref();
        self.get_search_directories_with_overrides(pythonpath, virtualenv)
    }

    /// Get all directories to search for modules with optional PYTHONPATH override
    /// Returns deduplicated, canonicalized paths
    fn get_search_directories_with_overrides(
        &self,
        pythonpath_override: Option<&str>,
        _virtualenv_override: Option<&str>,
    ) -> Vec<PathBuf> {
        let mut unique_dirs = IndexSet::new();

        // 1. Entry file's directory is ALWAYS first (per docs/resolution.md)
        if let Some(entry_dir) = &self.entry_dir {
            if let Ok(canonical) = entry_dir.canonicalize() {
                unique_dirs.insert(canonical);
            } else {
                unique_dirs.insert(entry_dir.clone());
            }
        }

        // 2. Add PYTHONPATH directories
        let pythonpath = pythonpath_override
            .map(|p| p.to_owned())
            .or_else(|| std::env::var("PYTHONPATH").ok());

        if let Some(pythonpath) = pythonpath {
            let separator = if cfg!(windows) { ';' } else { ':' };
            for path_str in pythonpath.split(separator) {
                self.add_pythonpath_directory(&mut unique_dirs, path_str);
            }
        }

        // 3. Add configured src directories
        for dir in &self.config.src {
            if let Ok(canonical) = dir.canonicalize() {
                unique_dirs.insert(canonical);
            } else {
                unique_dirs.insert(dir.clone());
            }
        }

        unique_dirs.into_iter().collect()
    }

    /// Helper method to add a PYTHONPATH directory to the unique set
    fn add_pythonpath_directory(&self, unique_dirs: &mut IndexSet<PathBuf>, path_str: &str) {
        if path_str.is_empty() {
            return;
        }

        let path = PathBuf::from(path_str);
        if !path.exists() || !path.is_dir() {
            return;
        }

        if let Ok(canonical) = path.canonicalize() {
            unique_dirs.insert(canonical);
        } else {
            unique_dirs.insert(path);
        }
    }

    /// Resolve a module to its file path using Python's resolution rules
    /// Per docs/resolution.md:
    /// 1. Check for package (foo/__init__.py)
    /// 2. Check for file module (foo.py)
    /// 3. Check for namespace package (foo/ directory without __init__.py)
    pub fn resolve_module_path(&mut self, module_name: &str) -> Result<Option<PathBuf>> {
        // For absolute imports, delegate to the context-aware version
        if !module_name.starts_with('.') {
            return self.resolve_module_path_with_context(module_name, None);
        }

        // Relative imports without context cannot be resolved
        // Don't cache this result since it might be resolvable with context
        warn!("Cannot resolve relative import '{module_name}' without module context");
        Ok(None)
    }

    /// Resolve a module with optional current module context for relative imports
    pub fn resolve_module_path_with_context(
        &mut self,
        module_name: &str,
        current_module_path: Option<&Path>,
    ) -> Result<Option<PathBuf>> {
        // Check cache first
        if let Some(cached_path) = self.module_cache.get(module_name) {
            return Ok(cached_path.clone());
        }

        let descriptor = ImportModuleDescriptor::from_module_name(module_name);

        // Handle relative imports
        if descriptor.leading_dots > 0 {
            if let Some(current_path) = current_module_path {
                let resolved = self.resolve_relative_import(&descriptor, current_path)?;
                // Don't cache relative imports as they depend on context
                // Different modules might resolve the same relative import differently
                return Ok(resolved);
            } else {
                // No context for relative import - don't cache this negative result
                warn!("Cannot resolve relative import '{module_name}' without module context");
                return Ok(None);
            }
        }

        // Try each search directory in order
        let search_dirs = self.get_search_directories();
        for search_dir in &search_dirs {
            if let Some(resolved_path) = self.resolve_in_directory(search_dir, &descriptor)? {
                self.module_cache
                    .insert(module_name.to_string(), Some(resolved_path.clone()));
                return Ok(Some(resolved_path));
            }
        }

        // Not found - cache the negative result
        self.module_cache.insert(module_name.to_string(), None);
        Ok(None)
    }

    /// Resolve a relative import given the current module's path
    fn resolve_relative_import(
        &self,
        descriptor: &ImportModuleDescriptor,
        current_module_path: &Path,
    ) -> Result<Option<PathBuf>> {
        // Determine the base directory for the relative import
        let mut base_dir = if current_module_path.is_file() {
            // If current module is a file, start from its parent directory
            current_module_path.parent().ok_or_else(|| {
                anyhow!("Cannot get parent directory of {:?}", current_module_path)
            })?
        } else {
            // If current module is a package directory, start from the directory itself
            current_module_path
        };

        // Go up directories based on the number of dots
        // One dot = current directory, two dots = parent directory, etc.
        for _ in 1..descriptor.leading_dots {
            base_dir = base_dir
                .parent()
                .ok_or_else(|| anyhow!("Too many dots in relative import - went above root"))?;
        }

        // If there are no name parts, we're importing the parent package itself
        if descriptor.name_parts.is_empty() {
            // Check if it's a package directory with __init__.py
            let init_path = base_dir.join("__init__.py");
            if init_path.exists() {
                let canonical = self.canonicalize_path(init_path);
                return Ok(Some(canonical));
            }
            // Otherwise, it might be a namespace package
            if base_dir.is_dir() {
                let canonical = self.canonicalize_path(base_dir.to_path_buf());
                return Ok(Some(canonical));
            }
            return Ok(None);
        }

        // Build the target path from the name parts
        let target_path = descriptor
            .name_parts
            .iter()
            .fold(base_dir.to_path_buf(), |path, part| path.join(part));

        // Try the standard resolution order
        // 1. Check for package (__init__.py)
        let init_path = target_path.join("__init__.py");
        if init_path.exists() {
            let canonical = self.canonicalize_path(init_path);
            return Ok(Some(canonical));
        }

        // 2. Check for module file (.py)
        let py_path = target_path.with_extension("py");
        if py_path.exists() {
            let canonical = self.canonicalize_path(py_path);
            return Ok(Some(canonical));
        }

        // 3. Check for namespace package (directory without __init__.py)
        if target_path.is_dir() {
            let canonical = self.canonicalize_path(target_path);
            return Ok(Some(canonical));
        }

        Ok(None)
    }

    /// Resolve an ImportlibStatic import that may have invalid Python identifiers
    /// This handles cases like importlib.import_module("data-processor")
    pub fn resolve_importlib_static(&mut self, module_name: &str) -> Result<Option<PathBuf>> {
        self.resolve_importlib_static_with_context(module_name, None)
            .map(|opt| opt.map(|(_, path)| path))
    }

    /// Resolve ImportlibStatic imports with optional package context for relative imports
    /// Returns a tuple of (resolved_module_name, path)
    pub fn resolve_importlib_static_with_context(
        &mut self,
        module_name: &str,
        package_context: Option<&str>,
    ) -> Result<Option<(String, PathBuf)>> {
        // Handle relative imports with package context
        let resolved_name = if let Some(package) = package_context {
            if module_name.starts_with('.') {
                // Count the number of leading dots
                let level = module_name.chars().take_while(|&c| c == '.').count();
                let name_part = module_name.trim_start_matches('.');

                // Split the package to handle parent navigation
                let mut package_parts: Vec<&str> = package.split('.').collect();

                // Go up 'level - 1' levels (one dot means current package)
                if level > 1 && package_parts.len() >= level - 1 {
                    package_parts.truncate(package_parts.len() - (level - 1));
                }

                // Append the name part if it's not empty
                if !name_part.is_empty() {
                    package_parts.push(name_part);
                }

                package_parts.join(".")
            } else {
                // Absolute import, use as-is
                module_name.to_string()
            }
        } else {
            module_name.to_string()
        };

        debug!(
            "Resolving ImportlibStatic: '{}' with package '{}' -> '{}'",
            module_name,
            package_context.unwrap_or("None"),
            resolved_name
        );

        // For ImportlibStatic imports, we look for files with the exact name
        // (including hyphens and other invalid Python identifier characters)
        let search_dirs = self.get_search_directories();

        for search_dir in &search_dirs {
            // Convert module name to file path (replace dots with slashes)
            let path_components: Vec<&str> = resolved_name.split('.').collect();

            if path_components.len() == 1 {
                // Single component - try as direct file
                let file_path = search_dir.join(format!("{resolved_name}.py"));
                if file_path.is_file() {
                    debug!("Found ImportlibStatic module at: {file_path:?}");
                    let canonical = self.canonicalize_path(file_path);
                    return Ok(Some((resolved_name.clone(), canonical)));
                }
            }

            // Try as a nested module path
            let mut module_path = search_dir.clone();
            for (i, component) in path_components.iter().enumerate() {
                if i == path_components.len() - 1 {
                    // Last component - try as file
                    let file_path = module_path.join(format!("{component}.py"));
                    if file_path.is_file() {
                        debug!("Found ImportlibStatic module at: {file_path:?}");
                        let canonical = self.canonicalize_path(file_path);
                        return Ok(Some((resolved_name.clone(), canonical)));
                    }
                }
                module_path = module_path.join(component);
            }

            // Try as a package directory with __init__.py
            let init_path = module_path.join("__init__.py");
            if init_path.is_file() {
                debug!("Found ImportlibStatic package at: {init_path:?}");
                let canonical = self.canonicalize_path(init_path);
                return Ok(Some((resolved_name.clone(), canonical)));
            }
        }

        // Not found
        Ok(None)
    }

    /// Resolve a module within a specific directory
    /// Implements the resolution algorithm from docs/resolution.md
    fn resolve_in_directory(
        &self,
        root: &Path,
        descriptor: &ImportModuleDescriptor,
    ) -> Result<Option<PathBuf>> {
        if descriptor.name_parts.is_empty() {
            // Edge case: empty import (shouldn't happen in practice)
            return Ok(None);
        }

        let mut current_path = root.to_path_buf();
        let mut resolved_paths = Vec::new();

        // Process all parts except the last one
        for (i, part) in descriptor.name_parts.iter().enumerate() {
            let is_last = i == descriptor.name_parts.len() - 1;

            if is_last {
                // For the last part, check in order:
                // 1. Package (foo/__init__.py)
                // 2. Module file (foo.py)
                // 3. Namespace package (foo/ directory)

                // Check for package first
                let package_init = current_path.join(part).join("__init__.py");
                if package_init.is_file() {
                    debug!("Found package at: {package_init:?}");
                    let canonical = self.canonicalize_path(package_init);
                    return Ok(Some(canonical));
                }

                // Check for module file
                let module_file = current_path.join(format!("{part}.py"));
                if module_file.is_file() {
                    debug!("Found module file at: {module_file:?}");
                    let canonical = self.canonicalize_path(module_file);
                    return Ok(Some(canonical));
                }

                // Check for namespace package (directory without __init__.py)
                let namespace_dir = current_path.join(part);
                if namespace_dir.is_dir() {
                    debug!("Found namespace package at: {namespace_dir:?}");
                    // Return the directory path to indicate this is a namespace package
                    let canonical = self.canonicalize_path(namespace_dir);
                    return Ok(Some(canonical));
                }
            } else {
                // For intermediate parts, they must be packages
                let package_dir = current_path.join(part);
                let package_init = package_dir.join("__init__.py");

                if package_init.is_file() {
                    resolved_paths.push(package_init);
                    current_path = package_dir;
                } else if package_dir.is_dir() {
                    // Namespace package - continue but don't add to resolved paths
                    current_path = package_dir;
                } else {
                    // Not found
                    return Ok(None);
                }
            }
        }

        Ok(None)
    }

    /// Classify an import as first-party, third-party, or standard library
    pub fn classify_import(&mut self, module_name: &str) -> ImportType {
        // Check cache first
        if let Some(cached_type) = self.classification_cache.get(module_name) {
            return cached_type.clone();
        }

        // Check if it's a relative import (starts with a dot)
        if module_name.starts_with('.') {
            let import_type = ImportType::FirstParty;
            self.classification_cache
                .insert(module_name.to_string(), import_type.clone());
            return import_type;
        }

        // Check explicit classifications from config
        if self.config.known_first_party.contains(module_name) {
            let import_type = ImportType::FirstParty;
            self.classification_cache
                .insert(module_name.to_string(), import_type.clone());
            return import_type;
        }
        if self.config.known_third_party.contains(module_name) {
            let import_type = ImportType::ThirdParty;
            self.classification_cache
                .insert(module_name.to_string(), import_type.clone());
            return import_type;
        }

        // Check if it's a standard library module
        if is_stdlib_module(module_name, self.python_version) {
            let import_type = ImportType::StandardLibrary;
            self.classification_cache
                .insert(module_name.to_string(), import_type.clone());
            return import_type;
        }

        // Try to resolve the module to determine if it's first-party
        let search_dirs = self.get_search_directories();
        let descriptor = ImportModuleDescriptor::from_module_name(module_name);

        for search_dir in &search_dirs {
            if let Ok(Some(_)) = self.resolve_in_directory(search_dir, &descriptor) {
                let import_type = ImportType::FirstParty;
                self.classification_cache
                    .insert(module_name.to_string(), import_type.clone());
                return import_type;
            }
        }

        // If the full module wasn't found, check if it's a submodule of a first-party module
        // For example, if "requests.auth" isn't found, check if "requests" is first-party
        if module_name.contains('.') {
            let parts: Vec<&str> = module_name.split('.').collect();
            if !parts.is_empty() {
                let parent_module = parts[0];
                // Recursively classify the parent module
                let parent_classification = self.classify_import(parent_module);
                if parent_classification == ImportType::FirstParty {
                    // If the parent is first-party, the submodule is too
                    let import_type = ImportType::FirstParty;
                    self.classification_cache
                        .insert(module_name.to_string(), import_type.clone());
                    return import_type;
                }
            }
        }

        // Check if it's in the virtual environment (third-party)
        if self.is_virtualenv_package(module_name) {
            let import_type = ImportType::ThirdParty;
            self.classification_cache
                .insert(module_name.to_string(), import_type.clone());
            return import_type;
        }

        // Default to third-party if we can't determine otherwise
        let import_type = ImportType::ThirdParty;
        self.classification_cache
            .insert(module_name.to_string(), import_type.clone());
        import_type
    }

    /// Get the set of third-party packages installed in the virtual environment
    fn get_virtualenv_packages(&self, virtualenv_override: Option<&str>) -> IndexSet<String> {
        let override_to_use = virtualenv_override.or(self.virtualenv_override.as_deref());

        // If we have a cached result and the same override (or lack thereof), return it
        if override_to_use == self.virtualenv_override.as_deref()
            && let Ok(cache_ref) = self.virtualenv_packages_cache.try_borrow()
            && let Some(cached_packages) = cache_ref.as_ref()
        {
            return cached_packages.clone();
        }

        // Compute the packages
        self.compute_virtualenv_packages(override_to_use)
    }

    /// Compute virtualenv packages by scanning the filesystem
    fn compute_virtualenv_packages(&self, virtualenv_override: Option<&str>) -> IndexSet<String> {
        let mut packages = IndexSet::new();

        // Try to get explicit VIRTUAL_ENV
        let explicit_virtualenv = virtualenv_override
            .map(|v| v.to_owned())
            .or_else(|| std::env::var("VIRTUAL_ENV").ok());

        let virtualenv_paths = if let Some(virtualenv_path) = explicit_virtualenv {
            vec![PathBuf::from(virtualenv_path)]
        } else {
            // Fallback: detect common virtual environment directory names
            self.detect_fallback_virtualenv_paths()
        };

        // Scan all discovered virtual environment paths
        for venv_path in virtualenv_paths {
            for site_packages_dir in self.get_virtualenv_site_packages_directories(&venv_path) {
                self.scan_site_packages_directory(&site_packages_dir, &mut packages);
            }
        }

        // Cache the result if it matches our stored override
        if virtualenv_override == self.virtualenv_override.as_deref()
            && let Ok(mut cache_ref) = self.virtualenv_packages_cache.try_borrow_mut()
        {
            *cache_ref = Some(packages.clone());
        }

        packages
    }

    /// Detect common virtual environment directory names
    fn detect_fallback_virtualenv_paths(&self) -> Vec<PathBuf> {
        let current_dir = match std::env::current_dir() {
            Ok(dir) => dir,
            Err(_) => return Vec::new(),
        };

        let common_venv_names = [".venv", "venv", "env", ".virtualenv", "virtualenv"];
        let mut venv_paths = Vec::new();

        for venv_name in &common_venv_names {
            let venv_path = current_dir.join(venv_name);
            if venv_path.is_dir() {
                // Check if it looks like a virtual environment
                let has_bin = venv_path.join("bin").is_dir() || venv_path.join("Scripts").is_dir();
                let has_lib = venv_path.join("lib").is_dir();

                if has_bin || has_lib {
                    venv_paths.push(venv_path);
                }
            }
        }

        venv_paths
    }

    /// Get site-packages directories for a virtual environment
    fn get_virtualenv_site_packages_directories(&self, venv_path: &Path) -> Vec<PathBuf> {
        let mut site_packages_dirs = Vec::new();

        // Unix-style virtual environment
        let lib_dir = venv_path.join("lib");
        if lib_dir.is_dir()
            && let Ok(entries) = std::fs::read_dir(&lib_dir)
        {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    let site_packages = path.join("site-packages");
                    if site_packages.is_dir() {
                        site_packages_dirs.push(site_packages);
                    }
                }
            }
        }

        // Windows-style virtual environment
        let lib_site_packages = venv_path.join("Lib").join("site-packages");
        if lib_site_packages.is_dir() {
            site_packages_dirs.push(lib_site_packages);
        }

        site_packages_dirs
    }

    /// Scan a site-packages directory and add found packages to the set
    fn scan_site_packages_directory(
        &self,
        site_packages_dir: &Path,
        packages: &mut IndexSet<String>,
    ) {
        let Ok(entries) = std::fs::read_dir(site_packages_dir) else {
            return;
        };

        for entry in entries.flatten() {
            let path = entry.path();
            let Some(name) = path.file_name().and_then(OsStr::to_str) else {
                continue;
            };

            // Skip common non-package entries
            if name.starts_with('_') || name.contains("-info") || name.contains(".dist-info") {
                continue;
            }

            // For directories, use the directory name as package name
            if path.is_dir() {
                packages.insert(name.to_owned());
            }
            // For .py files, use the filename without extension
            else if let Some(package_name) = name.strip_suffix(".py") {
                packages.insert(package_name.to_owned());
            }
        }
    }

    /// Check if a module name exists in the virtual environment packages
    fn is_virtualenv_package(&self, module_name: &str) -> bool {
        let virtualenv_packages = self.get_virtualenv_packages(None);

        // Check for exact match
        if virtualenv_packages.contains(module_name) {
            return true;
        }

        // Check if this is a submodule of a virtual environment package
        if let Some(root_module) = module_name.split('.').next()
            && virtualenv_packages.contains(root_module)
        {
            return true;
        }

        false
    }

    /// Get a reference to the configuration
    pub fn config(&self) -> &Config {
        &self.config
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::TempDir;

    use super::*;
    use crate::config::Config;

    fn create_test_file(path: &Path, content: &str) -> Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, content)?;
        Ok(())
    }

    #[test]
    fn test_module_first_resolution() -> Result<()> {
        // Test that foo/__init__.py is preferred over foo.py
        let temp_dir = TempDir::new()?;
        let root = temp_dir.path();

        // Create both foo/__init__.py and foo.py
        create_test_file(&root.join("foo/__init__.py"), "# Package")?;
        create_test_file(&root.join("foo.py"), "# Module")?;

        let config = Config {
            src: vec![root.to_path_buf()],
            ..Default::default()
        };
        let mut resolver = ModuleResolver::new(config)?;

        // Resolve foo - should prefer foo/__init__.py
        let result = resolver.resolve_module_path("foo")?;
        let expected = root.join("foo/__init__.py").canonicalize()?;
        assert_eq!(
            result.map(|p| p
                .canonicalize()
                .expect("failed to canonicalize resolved path")),
            Some(expected)
        );

        Ok(())
    }

    #[test]
    fn test_entry_dir_first_in_search_path() -> Result<()> {
        let temp_dir = TempDir::new()?;
        let root = temp_dir.path();

        // Create entry file and module in entry dir
        let entry_dir = root.join("src/app");
        let entry_file = entry_dir.join("main.py");
        create_test_file(&entry_file, "# Main")?;
        create_test_file(&entry_dir.join("helper.py"), "# Helper")?;

        // Create a different helper in configured src
        let other_src = root.join("lib");
        create_test_file(&other_src.join("helper.py"), "# Other helper")?;

        let config = Config {
            src: vec![other_src.clone()],
            ..Default::default()
        };
        let mut resolver = ModuleResolver::new(config)?;
        resolver.set_entry_file(&entry_file);

        // Resolve helper - should find the one in entry dir, not lib
        let result = resolver.resolve_module_path("helper")?;
        let expected = entry_dir.join("helper.py").canonicalize()?;
        assert_eq!(
            result.map(|p| p
                .canonicalize()
                .expect("failed to canonicalize resolved path")),
            Some(expected)
        );

        // Verify search path order
        let search_dirs = resolver.get_search_directories();
        assert!(!search_dirs.is_empty());
        // First dir should be the entry dir
        assert_eq!(search_dirs[0], entry_dir.canonicalize()?);

        Ok(())
    }

    #[test]
    fn test_package_resolution() -> Result<()> {
        let temp_dir = TempDir::new()?;
        let root = temp_dir.path();

        // Create nested package structure
        create_test_file(&root.join("myapp/__init__.py"), "")?;
        create_test_file(&root.join("myapp/utils/__init__.py"), "")?;
        create_test_file(&root.join("myapp/utils/helpers.py"), "")?;

        let config = Config {
            src: vec![root.to_path_buf()],
            ..Default::default()
        };
        let mut resolver = ModuleResolver::new(config)?;

        // Test various imports
        assert_eq!(
            resolver.resolve_module_path("myapp")?.map(|p| p
                .canonicalize()
                .expect("failed to canonicalize resolved path")),
            Some(root.join("myapp/__init__.py").canonicalize()?)
        );
        assert_eq!(
            resolver.resolve_module_path("myapp.utils")?.map(|p| p
                .canonicalize()
                .expect("failed to canonicalize resolved path")),
            Some(root.join("myapp/utils/__init__.py").canonicalize()?)
        );
        assert_eq!(
            resolver
                .resolve_module_path("myapp.utils.helpers")?
                .map(|p| p
                    .canonicalize()
                    .expect("failed to canonicalize resolved path")),
            Some(root.join("myapp/utils/helpers.py").canonicalize()?)
        );

        Ok(())
    }

    #[test]
    fn test_classification() -> Result<()> {
        let temp_dir = TempDir::new()?;
        let root = temp_dir.path();

        // Create a first-party module
        create_test_file(&root.join("mymodule.py"), "")?;

        let config = Config {
            src: vec![root.to_path_buf()],
            known_first_party: IndexSet::from(["known_first".to_string()]),
            known_third_party: IndexSet::from(["requests".to_string()]),
            ..Default::default()
        };
        let mut resolver = ModuleResolver::new(config)?;

        // Test classifications
        assert_eq!(resolver.classify_import("os"), ImportType::StandardLibrary);
        assert_eq!(resolver.classify_import("sys"), ImportType::StandardLibrary);
        assert_eq!(resolver.classify_import("mymodule"), ImportType::FirstParty);
        assert_eq!(
            resolver.classify_import("known_first"),
            ImportType::FirstParty
        );
        assert_eq!(resolver.classify_import("requests"), ImportType::ThirdParty);
        assert_eq!(
            resolver.classify_import(".relative"),
            ImportType::FirstParty
        );
        assert_eq!(
            resolver.classify_import("unknown_module"),
            ImportType::ThirdParty
        );

        Ok(())
    }

    #[test]
    fn test_pythonpath_guard() {
        // Save original value
        let original = std::env::var("PYTHONPATH").ok();

        {
            let _guard = PythonPathGuard::new("/test/path");
            assert_eq!(
                std::env::var("PYTHONPATH").expect("PYTHONPATH should be set"),
                "/test/path"
            );
        }

        // Should be restored
        match (original, std::env::var("PYTHONPATH").ok()) {
            (None, None) => (), // Good - was unset, still unset
            (Some(orig), Some(current)) => assert_eq!(orig, current),
            _ => panic!("PYTHONPATH not properly restored"),
        }
    }

    #[test]
    fn test_namespace_package() -> Result<()> {
        let temp_dir = TempDir::new()?;
        let root = temp_dir.path();

        // Create namespace package (directory without __init__.py)
        fs::create_dir_all(root.join("namespace_pkg/subpkg"))?;
        create_test_file(&root.join("namespace_pkg/subpkg/module.py"), "")?;

        let config = Config {
            src: vec![root.to_path_buf()],
            ..Default::default()
        };
        let mut resolver = ModuleResolver::new(config)?;

        // Namespace packages should be resolved to the directory
        let result = resolver.resolve_module_path("namespace_pkg")?;
        assert!(result.is_some());
        let resolved_path = result.expect("namespace_pkg should resolve to a path");
        assert!(resolved_path.is_dir());
        let expected = root.join("namespace_pkg").canonicalize()?;
        assert_eq!(resolved_path.canonicalize()?, expected);

        // Should be classified as first-party
        assert_eq!(
            resolver.classify_import("namespace_pkg"),
            ImportType::FirstParty
        );

        Ok(())
    }

    #[test]
    fn test_relative_import_resolution() -> Result<()> {
        let temp_dir = TempDir::new()?;
        let root = temp_dir.path();

        // Create a package structure:
        // mypackage/
        //   __init__.py
        //   module1.py
        //   subpackage/
        //     __init__.py
        //     module2.py
        //     deeper/
        //       __init__.py
        //       module3.py

        fs::create_dir_all(root.join("mypackage/subpackage/deeper"))?;
        create_test_file(&root.join("mypackage/__init__.py"), "# Package init")?;
        create_test_file(&root.join("mypackage/module1.py"), "# Module 1")?;
        create_test_file(
            &root.join("mypackage/subpackage/__init__.py"),
            "# Subpackage init",
        )?;
        create_test_file(&root.join("mypackage/subpackage/module2.py"), "# Module 2")?;
        create_test_file(
            &root.join("mypackage/subpackage/deeper/__init__.py"),
            "# Deeper init",
        )?;
        create_test_file(
            &root.join("mypackage/subpackage/deeper/module3.py"),
            "# Module 3",
        )?;

        let config = Config {
            src: vec![root.to_path_buf()],
            ..Default::default()
        };
        let mut resolver = ModuleResolver::new(config)?;

        // Test relative import from module3.py
        let module3_path = root.join("mypackage/subpackage/deeper/module3.py");

        // Test "from . import module3" (same directory)
        assert_eq!(
            resolver.resolve_module_path_with_context(".module3", Some(&module3_path))?,
            Some(
                root.join("mypackage/subpackage/deeper/module3.py")
                    .canonicalize()?
            )
        );

        // Test "from .. import module2" (parent directory)
        assert_eq!(
            resolver.resolve_module_path_with_context("..module2", Some(&module3_path))?,
            Some(
                root.join("mypackage/subpackage/module2.py")
                    .canonicalize()?
            )
        );

        // Test "from ... import module1" (grandparent directory)
        assert_eq!(
            resolver.resolve_module_path_with_context("...module1", Some(&module3_path))?,
            Some(root.join("mypackage/module1.py").canonicalize()?)
        );

        // Test "from . import" (current package)
        assert_eq!(
            resolver.resolve_module_path_with_context(".", Some(&module3_path))?,
            Some(
                root.join("mypackage/subpackage/deeper/__init__.py")
                    .canonicalize()?
            )
        );

        // Test "from .. import" (parent package)
        assert_eq!(
            resolver.resolve_module_path_with_context("..", Some(&module3_path))?,
            Some(
                root.join("mypackage/subpackage/__init__.py")
                    .canonicalize()?
            )
        );

        // Test relative import from a package __init__.py
        let subpackage_init = root.join("mypackage/subpackage/__init__.py");

        // Test "from . import module2" from __init__.py
        assert_eq!(
            resolver.resolve_module_path_with_context(".module2", Some(&subpackage_init))?,
            Some(
                root.join("mypackage/subpackage/module2.py")
                    .canonicalize()?
            )
        );

        // Test "from .deeper import module3"
        assert_eq!(
            resolver.resolve_module_path_with_context(".deeper.module3", Some(&subpackage_init))?,
            Some(
                root.join("mypackage/subpackage/deeper/module3.py")
                    .canonicalize()?
            )
        );

        // Test error case: too many dots
        let result =
            resolver.resolve_module_path_with_context("....toomanydots", Some(&module3_path));
        assert!(result.is_err() || result.expect("result should be Ok").is_none());

        Ok(())
    }
}
