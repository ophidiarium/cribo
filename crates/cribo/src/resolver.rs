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

/// Check if a module is part of the Python standard library using `ruff_python_stdlib`
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
    module_cache: RefCell<IndexMap<String, Option<PathBuf>>>,
    /// Cache of module classifications
    classification_cache: RefCell<IndexMap<String, ImportType>>,
    /// Cache of virtual environment packages to avoid repeated filesystem scans
    virtualenv_packages_cache: RefCell<Option<IndexSet<String>>>,
    /// Entry file's directory (first in search path)
    entry_dir: Option<PathBuf>,
    /// Python version for stdlib classification
    python_version: u8,
    /// PYTHONPATH override for testing
    pythonpath_override: Option<String>,
    /// `VIRTUAL_ENV` override for testing
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

    /// Create a new `ModuleResolver` with optional PYTHONPATH and `VIRTUAL_ENV` overrides for
    /// testing
    pub fn new_with_overrides(
        config: Config,
        pythonpath_override: Option<&str>,
        virtualenv_override: Option<&str>,
    ) -> Result<Self> {
        Ok(Self {
            config,
            module_cache: RefCell::new(IndexMap::new()),
            classification_cache: RefCell::new(IndexMap::new()),
            virtualenv_packages_cache: RefCell::new(None),
            entry_dir: None,
            python_version: 38, // Default to Python 3.8
            pythonpath_override: pythonpath_override.map(std::string::ToString::to_string),
            virtualenv_override: virtualenv_override.map(std::string::ToString::to_string),
        })
    }

    /// Set the entry file for the resolver
    /// This establishes the first search path directory
    pub fn set_entry_file(&mut self, entry_path: &Path) {
        if let Some(parent) = entry_path.parent() {
            self.entry_dir = Some(parent.to_path_buf());
            debug!("Set entry directory to: {}", parent.display());
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
            .map(std::borrow::ToOwned::to_owned)
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
    pub fn resolve_module_path(&self, module_name: &str) -> Result<Option<PathBuf>> {
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
        &self,
        module_name: &str,
        current_module_path: Option<&Path>,
    ) -> Result<Option<PathBuf>> {
        // Check cache first
        if let Some(cached_path) = self.module_cache.borrow().get(module_name) {
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
            }
            // No context for relative import - don't cache this negative result
            warn!("Cannot resolve relative import '{module_name}' without module context");
            return Ok(None);
        }

        // Try each search directory in order
        let search_dirs = self.get_search_directories();
        for search_dir in &search_dirs {
            if let Some(resolved_path) = self.resolve_in_directory(search_dir, &descriptor)? {
                self.module_cache
                    .borrow_mut()
                    .insert(module_name.to_string(), Some(resolved_path.clone()));
                return Ok(Some(resolved_path));
            }
        }

        // Not found - cache the negative result
        self.module_cache
            .borrow_mut()
            .insert(module_name.to_string(), None);
        Ok(None)
    }

    /// Resolve a relative import given the current module's path
    fn resolve_relative_import(
        &self,
        descriptor: &ImportModuleDescriptor,
        current_module_path: &Path,
    ) -> Result<Option<PathBuf>> {
        // First resolve to absolute module name
        let name_string = if descriptor.name_parts.is_empty() {
            None
        } else {
            Some(descriptor.name_parts.join("."))
        };
        let name = name_string.as_deref();

        let level = u32::try_from(descriptor.leading_dots).map_err(|_| {
            anyhow!(
                "Relative import level {} is too large (max: {})",
                descriptor.leading_dots,
                u32::MAX
            )
        })?;

        let absolute_module_name = self
            .resolve_relative_to_absolute_module_name(level, name, current_module_path)
            .ok_or_else(|| anyhow!("Failed to resolve relative import"))?;

        // Now resolve the absolute module name to a path
        // Create a new descriptor for the absolute import
        let absolute_descriptor = ImportModuleDescriptor::from_module_name(&absolute_module_name);

        // Use the existing resolution logic for absolute imports
        let search_dirs = self.get_search_directories();
        for search_dir in &search_dirs {
            if let Some(resolved_path) =
                self.resolve_in_directory(search_dir, &absolute_descriptor)?
            {
                return Ok(Some(resolved_path));
            }
        }

        Ok(None)
    }

    /// Resolve an `ImportlibStatic` import that may have invalid Python identifiers
    /// This handles cases like importlib.import_module("data-processor")
    /// Resolve `ImportlibStatic` imports with optional package context for relative imports
    /// Returns a tuple of (`resolved_module_name`, path)
    pub fn resolve_importlib_static_with_context(
        &self,
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
                    debug!("Found ImportlibStatic module at: {}", file_path.display());
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
                        debug!("Found ImportlibStatic module at: {}", file_path.display());
                        let canonical = self.canonicalize_path(file_path);
                        return Ok(Some((resolved_name.clone(), canonical)));
                    }
                }
                module_path = module_path.join(component);
            }

            // Try as a package directory with __init__.py
            let init_path = module_path.join("__init__.py");
            if init_path.is_file() {
                debug!("Found ImportlibStatic package at: {}", init_path.display());
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
                    debug!("Found package at: {}", package_init.display());
                    let canonical = self.canonicalize_path(package_init);
                    return Ok(Some(canonical));
                }

                // Check for module file
                let module_file = current_path.join(format!("{part}.py"));
                if module_file.is_file() {
                    debug!("Found module file at: {}", module_file.display());
                    let canonical = self.canonicalize_path(module_file);
                    return Ok(Some(canonical));
                }

                // Check for namespace package (directory without __init__.py)
                let namespace_dir = current_path.join(part);
                if namespace_dir.is_dir() {
                    debug!("Found namespace package at: {}", namespace_dir.display());
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
    pub fn classify_import(&self, module_name: &str) -> ImportType {
        // Check cache first
        if let Some(cached_type) = self.classification_cache.borrow().get(module_name) {
            return cached_type.clone();
        }

        // Check if it's a relative import (starts with a dot)
        if module_name.starts_with('.') {
            let import_type = ImportType::FirstParty;
            self.classification_cache
                .borrow_mut()
                .insert(module_name.to_string(), import_type.clone());
            return import_type;
        }

        // Check explicit classifications from config
        if self.config.known_first_party.contains(module_name) {
            let import_type = ImportType::FirstParty;
            self.classification_cache
                .borrow_mut()
                .insert(module_name.to_string(), import_type.clone());
            return import_type;
        }
        if self.config.known_third_party.contains(module_name) {
            let import_type = ImportType::ThirdParty;
            self.classification_cache
                .borrow_mut()
                .insert(module_name.to_string(), import_type.clone());
            return import_type;
        }

        // Check if it's a standard library module
        if is_stdlib_module(module_name, self.python_version) {
            let import_type = ImportType::StandardLibrary;
            self.classification_cache
                .borrow_mut()
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
                    .borrow_mut()
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
                        .borrow_mut()
                        .insert(module_name.to_string(), import_type.clone());
                    return import_type;
                }
            }
        }

        // Check if it's in the virtual environment (third-party)
        if self.is_virtualenv_package(module_name) {
            let import_type = ImportType::ThirdParty;
            self.classification_cache
                .borrow_mut()
                .insert(module_name.to_string(), import_type.clone());
            return import_type;
        }

        // Default to third-party if we can't determine otherwise
        let import_type = ImportType::ThirdParty;
        self.classification_cache
            .borrow_mut()
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
            .map(std::borrow::ToOwned::to_owned)
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
        let Ok(current_dir) = std::env::current_dir() else {
            return Vec::new();
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

    /// Resolves a relative import to an absolute module name.
    ///
    /// # Arguments
    /// * `level` - The number of leading dots (e.g., 1 for `.`, 2 for `..`). Must be > 0.
    /// * `name` - The module being imported, if any (e.g., `Some("bar")` for `from . import bar`).
    /// * `current_module_path` - The filesystem path of the module performing the import.
    ///
    /// # Returns
    /// An `Option<String>` containing the absolute module name if resolution is successful.
    pub fn resolve_relative_to_absolute_module_name(
        &self,
        level: u32,
        name: Option<&str>,
        current_module_path: &Path,
    ) -> Option<String> {
        // Get the absolute module path parts for the current file
        let module_parts = self.path_to_module_parts(current_module_path)?;

        // Apply relative import logic
        let mut current_parts = module_parts;

        // Remove 'level - 1' components since level=1 means current package
        // Cannot go beyond the root of the project
        if level as usize > current_parts.len() + 1 {
            return None;
        }

        for _ in 0..(level.saturating_sub(1)) {
            current_parts.pop();
        }

        // If name is provided, split it and append
        if let Some(name) = name
            && !name.is_empty()
        {
            let name_parts: Vec<&str> = name.split('.').collect();
            current_parts.extend(name_parts.into_iter().map(std::string::ToString::to_string));
        }

        if current_parts.is_empty() {
            // If we're at the root after applying relative levels, return empty string
            // This will be handled by the caller to construct the full import name
            Some(String::new())
        } else {
            Some(current_parts.join("."))
        }
    }

    /// Resolve a relative import given a package name (not path)
    /// This is used when we have a package string like "foo.bar" instead of a file path
    pub fn resolve_relative_import_from_package_name(
        &self,
        level: u32,
        name: Option<&str>,
        package_name: &str,
    ) -> String {
        let mut package_parts: Vec<&str> = package_name.split('.').collect();

        // Go up 'level - 1' levels (one dot means current package)
        if level > 1 && package_parts.len() >= (level as usize) - 1 {
            package_parts.truncate(package_parts.len() - ((level as usize) - 1));
        }

        // Append the name part if provided
        if let Some(name_part) = name
            && !name_part.is_empty()
        {
            package_parts.push(name_part);
        }

        package_parts.join(".")
    }

    /// Convert a filesystem path to module path components
    fn path_to_module_parts(&self, file_path: &Path) -> Option<Vec<String>> {
        // Get the directory containing the current file
        let current_dir = if file_path.is_file() {
            file_path.parent()?
        } else {
            // If it's a directory (e.g., namespace package), use it directly
            file_path
        };

        // Convert current_dir to absolute path if it's relative
        let absolute_current_dir = if current_dir.is_absolute() {
            current_dir.to_path_buf()
        } else {
            let current_working_dir = std::env::current_dir().ok()?;
            current_working_dir.join(current_dir)
        };

        // Find which source directory contains this file
        let relative_dir = self.config.src.iter().find_map(|src_dir| {
            // Handle case where paths might be relative vs absolute
            if src_dir.is_absolute() {
                absolute_current_dir.strip_prefix(src_dir).ok()
            } else {
                // For relative source directories, resolve them relative to current working
                // directory
                let current_working_dir = std::env::current_dir().ok()?;
                let absolute_src_dir = current_working_dir.join(src_dir);
                absolute_current_dir.strip_prefix(&absolute_src_dir).ok()
            }
        })?;

        // Convert directory path to module path components
        if relative_dir == Path::new("") {
            // If relative_dir is empty, we're at the root of the source directory
            Some(Vec::new())
        } else {
            Some(
                relative_dir
                    .components()
                    .map(|c| c.as_os_str().to_string_lossy().into_owned())
                    .collect(),
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use anyhow::Result;
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
        let resolver = ModuleResolver::new(config)?;

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
        let resolver = ModuleResolver::new(config)?;

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
        let resolver = ModuleResolver::new(config)?;

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
        let resolver = ModuleResolver::new(config)?;

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
        let resolver = ModuleResolver::new(config)?;

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

    #[test]
    fn test_pythonpath_module_discovery() -> Result<()> {
        // Create temporary directories for testing
        let temp_dir = TempDir::new()?;
        let pythonpath_dir = temp_dir.path().join("pythonpath_modules");
        let src_dir = temp_dir.path().join("src");

        // Create directory structures
        fs::create_dir_all(&pythonpath_dir)?;
        fs::create_dir_all(&src_dir)?;

        // Create a module in PYTHONPATH directory
        let pythonpath_module = pythonpath_dir.join("pythonpath_module.py");
        fs::write(
            &pythonpath_module,
            "# This is a PYTHONPATH module\ndef hello():\n    return 'Hello from PYTHONPATH'",
        )?;

        // Create a package in PYTHONPATH directory
        let pythonpath_pkg = pythonpath_dir.join("pythonpath_pkg");
        fs::create_dir_all(&pythonpath_pkg)?;
        let pythonpath_pkg_init = pythonpath_pkg.join("__init__.py");
        fs::write(&pythonpath_pkg_init, "# PYTHONPATH package")?;
        let pythonpath_pkg_module = pythonpath_pkg.join("submodule.py");
        fs::write(&pythonpath_pkg_module, "# PYTHONPATH submodule")?;

        // Create a module in src directory
        let src_module = src_dir.join("src_module.py");
        fs::write(&src_module, "# This is a src module")?;

        // Set up config with src directory
        let config = Config {
            src: vec![src_dir.clone()],
            ..Default::default()
        };

        // Create resolver with PYTHONPATH override
        let pythonpath_str = pythonpath_dir.to_string_lossy();
        let resolver = ModuleResolver::new_with_overrides(config, Some(&pythonpath_str), None)?;

        // Test that modules can be resolved from both src and PYTHONPATH
        assert!(
            resolver.resolve_module_path("src_module")?.is_some(),
            "Should resolve modules from configured src directories"
        );
        assert!(
            resolver.resolve_module_path("pythonpath_module")?.is_some(),
            "Should resolve modules from PYTHONPATH directories"
        );
        assert!(
            resolver.resolve_module_path("pythonpath_pkg")?.is_some(),
            "Should resolve packages from PYTHONPATH directories"
        );
        assert!(
            resolver
                .resolve_module_path("pythonpath_pkg.submodule")?
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

        Ok(())
    }

    #[test]
    fn test_pythonpath_module_classification() -> Result<()> {
        // Create temporary directories for testing
        let temp_dir = TempDir::new()?;
        let pythonpath_dir = temp_dir.path().join("pythonpath_modules");
        let src_dir = temp_dir.path().join("src");

        // Create directory structures
        fs::create_dir_all(&pythonpath_dir)?;
        fs::create_dir_all(&src_dir)?;

        // Create a module in PYTHONPATH directory
        let pythonpath_module = pythonpath_dir.join("pythonpath_module.py");
        fs::write(&pythonpath_module, "# This is a PYTHONPATH module")?;

        // Set up config
        let config = Config {
            src: vec![src_dir.clone()],
            ..Default::default()
        };

        // Create resolver with PYTHONPATH override
        let pythonpath_str = pythonpath_dir.to_string_lossy();
        let resolver = ModuleResolver::new_with_overrides(config, Some(&pythonpath_str), None)?;

        // Test that PYTHONPATH modules are classified as first-party
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

        Ok(())
    }

    #[test]
    fn test_pythonpath_multiple_directories() -> Result<()> {
        // Create temporary directories for testing
        let temp_dir = TempDir::new()?;
        let pythonpath_dir1 = temp_dir.path().join("pythonpath1");
        let pythonpath_dir2 = temp_dir.path().join("pythonpath2");
        let src_dir = temp_dir.path().join("src");

        // Create directory structures
        fs::create_dir_all(&pythonpath_dir1)?;
        fs::create_dir_all(&pythonpath_dir2)?;
        fs::create_dir_all(&src_dir)?;

        // Create modules in different PYTHONPATH directories
        let module1 = pythonpath_dir1.join("module1.py");
        fs::write(&module1, "# Module in pythonpath1")?;

        let module2 = pythonpath_dir2.join("module2.py");
        fs::write(&module2, "# Module in pythonpath2")?;

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
        let resolver = ModuleResolver::new_with_overrides(config, Some(&pythonpath_str), None)?;

        // Test that modules from both PYTHONPATH directories can be resolved
        assert!(
            resolver.resolve_module_path("module1")?.is_some(),
            "Should resolve modules from first PYTHONPATH directory"
        );
        assert!(
            resolver.resolve_module_path("module2")?.is_some(),
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

        Ok(())
    }

    #[test]
    fn test_pythonpath_empty_or_nonexistent() -> Result<()> {
        // Create a temporary directory for testing
        let temp_dir = TempDir::new()?;
        let src_dir = temp_dir.path().join("src");
        fs::create_dir_all(&src_dir)?;

        // Create a test module
        let test_module = src_dir.join("test_module.py");
        fs::write(&test_module, "# Test module")?;

        let config = Config {
            src: vec![src_dir.clone()],
            ..Default::default()
        };

        // Test with empty PYTHONPATH
        let resolver1 = ModuleResolver::new_with_overrides(config.clone(), Some(""), None)?;

        // Should be able to resolve module from src directory
        assert!(
            resolver1.resolve_module_path("test_module")?.is_some(),
            "Should resolve module from src directory with empty PYTHONPATH"
        );

        // Test with no PYTHONPATH
        let resolver2 = ModuleResolver::new_with_overrides(config.clone(), None, None)?;

        // Should be able to resolve module from src directory
        assert!(
            resolver2.resolve_module_path("test_module")?.is_some(),
            "Should resolve module from src directory with no PYTHONPATH"
        );

        // Test with nonexistent directories in PYTHONPATH
        let separator = if cfg!(windows) { ';' } else { ':' };
        let nonexistent_pythonpath = format!("/nonexistent1{separator}/nonexistent2");
        let resolver3 =
            ModuleResolver::new_with_overrides(config, Some(&nonexistent_pythonpath), None)?;

        // Should still be able to resolve module from src directory
        assert!(
            resolver3.resolve_module_path("test_module")?.is_some(),
            "Should resolve module from src directory even with nonexistent PYTHONPATH"
        );

        // Non-existent modules should not be found
        assert!(
            resolver3
                .resolve_module_path("nonexistent_module")?
                .is_none(),
            "Should not find nonexistent modules"
        );

        Ok(())
    }

    #[test]
    fn test_directory_deduplication() -> Result<()> {
        // Create temporary directories for testing
        let temp_dir = TempDir::new()?;
        let src_dir = temp_dir.path().join("src");
        let other_dir = temp_dir.path().join("other");

        // Create directory structures
        fs::create_dir_all(&src_dir)?;
        fs::create_dir_all(&other_dir)?;

        // Create modules
        let src_module = src_dir.join("src_module.py");
        fs::write(&src_module, "# Source module")?;
        let other_module = other_dir.join("other_module.py");
        fs::write(&other_module, "# Other module")?;

        // Set up config with src directory
        let config = Config {
            src: vec![src_dir.clone()],
            ..Default::default()
        };

        // Create resolver with PYTHONPATH override that includes the same src directory plus
        // another directory
        let separator = if cfg!(windows) { ';' } else { ':' };
        let pythonpath_str = format!(
            "{}{}{}",
            src_dir.to_string_lossy(),
            separator,
            other_dir.to_string_lossy()
        );
        let resolver = ModuleResolver::new_with_overrides(config, Some(&pythonpath_str), None)?;

        // Test that deduplication works - both modules should be resolvable
        assert!(
            resolver.resolve_module_path("src_module")?.is_some(),
            "Should resolve src_module"
        );
        assert!(
            resolver.resolve_module_path("other_module")?.is_some(),
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

        Ok(())
    }

    #[test]
    fn test_path_canonicalization() -> Result<()> {
        // Create temporary directories for testing
        let temp_dir = TempDir::new()?;
        let src_dir = temp_dir.path().join("src");
        fs::create_dir_all(&src_dir)?;

        // Create a module
        let module_file = src_dir.join("test_module.py");
        fs::write(&module_file, "# Test module")?;

        // Set up config with the src directory
        let config = Config {
            src: vec![src_dir.clone()],
            ..Default::default()
        };

        // Create resolver with PYTHONPATH override using a relative path with .. components
        // This creates a different string representation of the same directory
        let parent_dir = src_dir
            .parent()
            .expect("test source directory should have a parent");
        let relative_path = parent_dir.join("src/../src"); // This resolves to the same directory
        let pythonpath_str = relative_path.to_string_lossy();
        let resolver = ModuleResolver::new_with_overrides(config, Some(&pythonpath_str), None)?;

        // Test that the module can be resolved despite path canonicalization differences
        assert!(
            resolver.resolve_module_path("test_module")?.is_some(),
            "Should resolve module even with different path representations"
        );

        // Should be classified as first-party
        assert_eq!(
            resolver.classify_import("test_module"),
            ImportType::FirstParty,
            "Should classify test_module as first-party"
        );

        Ok(())
    }
}
