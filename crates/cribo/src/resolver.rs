use std::{
    cell::RefCell,
    ffi::OsStr,
    fmt,
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
    sync::Mutex,
};

use anyhow::{Result, anyhow};
use cow_utils::CowUtils;
use indexmap::{IndexMap, IndexSet};
use log::{debug, info, warn};
use pep508_rs::PackageName;
use ruff_python_stdlib::sys;

use crate::{config::Config, types::FxIndexMap};

/// Unique identifier for a module in the dependency graph
/// The entry module ALWAYS has ID 0 - this is a fundamental invariant
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ModuleId(pub u32);

impl ModuleId {
    /// The entry point - always ID 0
    /// This is where bundling starts, the origin of our module universe
    pub const ENTRY: ModuleId = ModuleId(0);

    #[inline]
    pub const fn new(id: u32) -> Self {
        Self(id)
    }

    #[inline]
    pub const fn as_u32(self) -> u32 {
        self.0
    }

    /// Check if this is the entry module
    /// No more complex path detection or boolean flags!
    #[inline]
    pub const fn is_entry(self) -> bool {
        self.0 == 0
    }

    /// Format this `ModuleId` with the resolver to show the module name and path
    /// This is useful for debugging and error messages
    pub fn format_with_resolver(&self, resolver: &ModuleResolver) -> String {
        if let Some(name) = resolver.get_module_name(*self) {
            if let Some(path) = resolver.get_module_path(*self) {
                format!("ModuleId({})='{}' at '{}'", self.0, name, path.display())
            } else {
                format!("ModuleId({})='{}'", self.0, name)
            }
        } else {
            format!("ModuleId({})", self.0)
        }
    }
}

impl fmt::Display for ModuleId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "module#{}", self.0)
    }
}

impl From<u32> for ModuleId {
    fn from(value: u32) -> Self {
        ModuleId(value)
    }
}

impl From<ModuleId> for u32 {
    fn from(value: ModuleId) -> u32 {
        value.0
    }
}

/// Module metadata tracked by resolver
#[derive(Debug, Clone)]
pub struct ModuleMetadata {
    pub id: ModuleId,
    pub name: String,
    pub canonical_path: PathBuf,
    pub is_package: bool,
    pub kind: crate::python::module_path::ModuleKind,
}

/// Internal module registry for ID allocation
#[derive(Debug)]
struct ModuleRegistry {
    next_id: u32,
    by_id: FxIndexMap<ModuleId, ModuleMetadata>,
    by_name: FxIndexMap<String, ModuleId>,
    by_path: FxIndexMap<PathBuf, ModuleId>,
}

impl ModuleRegistry {
    fn new() -> Self {
        Self {
            next_id: 0, // Start at 0 - entry point gets this
            by_id: FxIndexMap::default(),
            by_name: FxIndexMap::default(),
            by_path: FxIndexMap::default(),
        }
    }

    fn register(&mut self, name: String, path: &Path) -> ModuleId {
        let canonical_path = path.canonicalize().unwrap_or_else(|_| path.to_owned());

        // Check for duplicates
        if let Some(&id) = self.by_name.get(&name)
            && self.by_id[&id].canonical_path == canonical_path
        {
            return id;
        }

        if let Some(&id) = self.by_path.get(&canonical_path) {
            self.by_name.insert(name, id);
            return id;
        }

        // Allocate ID - entry gets 0, others get sequential IDs
        let id = ModuleId::new(self.next_id);
        self.next_id += 1;

        // The beauty: first registered module (entry) automatically gets ID 0!
        debug_assert!(
            id != ModuleId::ENTRY || self.by_id.is_empty(),
            "Entry module must be registered first"
        );

        // Determine whether this path represents a package and its kind,
        // including support for PEP 420 namespace packages.
        let (is_package, kind) = if path.is_dir() {
            // A directory without __init__.py is a namespace package
            debug_assert!(
                !crate::python::module_path::is_package_dir_with_init(path),
                "register_module(name, path) should receive the __init__.py file for regular \
                 packages; got a directory: {}",
                path.display()
            );
            (
                true,
                crate::python::module_path::ModuleKind::NamespacePackageDir,
            )
        } else if path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(crate::python::module_path::is_init_file_name)
        {
            // A file named __init__.py (or equivalent) is a regular package init
            (true, crate::python::module_path::ModuleKind::PackageInit)
        } else if path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n == crate::python::constants::MAIN_FILE)
        {
            // A file named __main__.py
            (false, crate::python::module_path::ModuleKind::Main)
        } else {
            // Any other file is a regular module
            (false, crate::python::module_path::ModuleKind::RegularModule)
        };

        let metadata = ModuleMetadata {
            id,
            name: name.clone(),
            canonical_path: canonical_path.clone(),
            is_package,
            kind,
        };

        self.by_id.insert(id, metadata);
        self.by_name.insert(name, id);
        self.by_path.insert(canonical_path, id);

        id
    }

    fn get_metadata(&self, id: ModuleId) -> Option<&ModuleMetadata> {
        self.by_id.get(&id)
    }

    fn get_id_by_name(&self, name: &str) -> Option<&ModuleId> {
        self.by_name.get(name)
    }
}

/// Resolve a relative import based on module name (standalone utility)
///
/// This is a pure function that resolves relative imports based on module names alone,
/// without requiring a resolver instance. Used by both `ModuleResolver` and `ImportAnalyzer`.
///
/// # Arguments
/// * `level` - The number of leading dots in the relative import
/// * `name` - The module name after the dots (if any)
/// * `current_module_name` - The name of the module performing the import
///
/// # Returns
/// The resolved absolute module name
pub fn resolve_relative_import_from_name(
    level: u32,
    name: Option<&str>,
    current_module_name: &str,
) -> String {
    let mut package_parts: Vec<&str> = current_module_name.split('.').collect();

    // For modules (not packages), we need to remove the module itself first
    // then go up additional levels
    // Check if this is likely a package (__init__) or a regular module
    let is_likely_package = package_parts
        .last()
        .is_some_and(|part| *part == crate::python::constants::INIT_STEM);

    if !is_likely_package && package_parts.len() > 1 {
        // Remove the module name itself for regular modules
        package_parts.pop();
    }

    // Go up additional levels based on the import level
    // Level 1 means current package, level 2 means parent, etc.
    for _ in 1..level {
        if package_parts.is_empty() {
            break; // Can't go up any further
        }
        package_parts.pop();
    }

    // Append the name part if provided
    if let Some(name_part) = name
        && !name_part.is_empty()
    {
        package_parts.push(name_part);
    }

    package_parts.join(".")
}

/// Check if a module is part of the Python standard library using `ruff_python_stdlib`
pub fn is_stdlib_module(module_name: &str, python_version: u8) -> bool {
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
    /// Module registry for ID allocation - the single source of truth for module identity
    registry: Mutex<ModuleRegistry>,
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

    pub fn new(config: Config) -> Self {
        Self::new_with_overrides(config, None, None)
    }

    /// Create a new `ModuleResolver` with optional PYTHONPATH and `VIRTUAL_ENV` overrides for
    /// testing
    pub fn new_with_overrides(
        config: Config,
        pythonpath_override: Option<&str>,
        virtualenv_override: Option<&str>,
    ) -> Self {
        Self {
            config,
            registry: Mutex::new(ModuleRegistry::new()),
            module_cache: RefCell::new(IndexMap::new()),
            classification_cache: RefCell::new(IndexMap::new()),
            virtualenv_packages_cache: RefCell::new(None),
            entry_dir: None,
            python_version: 38, // Default to Python 3.8
            pythonpath_override: pythonpath_override.map(std::string::ToString::to_string),
            virtualenv_override: virtualenv_override.map(std::string::ToString::to_string),
        }
    }

    /// Set the entry file for the resolver
    /// This establishes the first search path directory
    pub fn set_entry_file(&mut self, entry_path: &Path, original_entry_path: &Path) {
        // If the original entry was a directory that became __init__.py,
        // use the parent of the directory as the search root
        if original_entry_path.is_dir()
            && entry_path.file_name()
                == Some(std::ffi::OsStr::new(crate::python::constants::INIT_FILE))
        {
            // For package directories, use the parent of the package dir as search root
            if let Some(package_parent) = original_entry_path.parent() {
                self.entry_dir = Some(package_parent.to_path_buf());
                debug!(
                    "Set entry directory to package parent: {}",
                    package_parent.display()
                );
            } else {
                // Fallback to the package directory itself if no parent
                self.entry_dir = Some(original_entry_path.to_path_buf());
                debug!(
                    "Set entry directory to package dir: {}",
                    original_entry_path.display()
                );
            }
        } else if let Some(parent) = entry_path.parent() {
            self.entry_dir = Some(parent.to_path_buf());
            debug!("Set entry directory to: {}", parent.display());
        }
    }

    /// Get module name by ID
    pub fn get_module_name(&self, id: ModuleId) -> Option<String> {
        let registry = self.registry.lock().expect("Module registry lock poisoned");
        registry.get_metadata(id).map(|m| m.name.clone())
    }

    /// Get module kind by ID (post-registration truth source)
    pub fn get_module_kind(&self, id: ModuleId) -> Option<crate::python::module_path::ModuleKind> {
        let registry = self.registry.lock().expect("Module registry lock poisoned");
        registry.get_metadata(id).map(|m| m.kind)
    }

    /// Returns true if the module is a package initializer (__init__.py)
    pub fn is_package_init(&self, id: ModuleId) -> bool {
        matches!(
            self.get_module_kind(id),
            Some(crate::python::module_path::ModuleKind::PackageInit)
        )
    }

    /// Returns true if the module is a namespace package (directory without __init__.py)
    pub fn is_namespace_package(&self, id: ModuleId) -> bool {
        matches!(
            self.get_module_kind(id),
            Some(crate::python::module_path::ModuleKind::NamespacePackageDir)
        )
    }

    /// Get module path by ID
    pub fn get_module_path(&self, id: ModuleId) -> Option<PathBuf> {
        let registry = self.registry.lock().expect("Module registry lock poisoned");
        registry.get_metadata(id).map(|m| m.canonical_path.clone())
    }

    /// Check if the entry module is a package
    pub fn is_entry_package(&self) -> bool {
        let registry = self.registry.lock().expect("Module registry lock poisoned");
        registry.get_metadata(ModuleId::ENTRY).is_some_and(|m| {
            matches!(
                m.kind,
                crate::python::module_path::ModuleKind::PackageInit
                    | crate::python::module_path::ModuleKind::NamespacePackageDir
            )
        })
    }

    /// Get module metadata by ID
    pub fn get_module(&self, id: ModuleId) -> Option<ModuleMetadata> {
        let registry = self.registry.lock().expect("Module registry lock poisoned");
        registry.get_metadata(id).cloned()
    }

    /// Get module ID by name (reverse lookup)
    pub fn get_module_id_by_name(&self, name: &str) -> Option<ModuleId> {
        let registry = self.registry.lock().expect("Module registry lock poisoned");
        registry.get_id_by_name(name).copied()
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
            if let Some(resolved_path) = self.resolve_in_directory(search_dir, &descriptor) {
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
            if let Some(resolved_path) = self.resolve_in_directory(search_dir, &absolute_descriptor)
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
    ) -> Option<(String, PathBuf)> {
        // Handle relative imports with package context
        let resolved_name = if let Some(package) = package_context {
            if module_name.starts_with('.') {
                // Count the number of leading dots
                let level = module_name.chars().take_while(|&c| c == '.').count() as u32;
                let name_part = module_name.trim_start_matches('.');

                // Use the centralized helper for relative import resolution
                self.resolve_relative_import_from_package_name(
                    level,
                    if name_part.is_empty() {
                        None
                    } else {
                        Some(name_part)
                    },
                    package,
                )
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
                    return Some((resolved_name.clone(), canonical));
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
                        return Some((resolved_name.clone(), canonical));
                    }
                }
                module_path = module_path.join(component);
            }

            // Try as a package directory with __init__.py
            let init_path = module_path.join(crate::python::constants::INIT_FILE);
            if init_path.is_file() {
                debug!("Found ImportlibStatic package at: {}", init_path.display());
                let canonical = self.canonicalize_path(init_path);
                return Some((resolved_name.clone(), canonical));
            }
        }

        // Not found
        None
    }

    /// Resolve a module within a specific directory
    /// Implements the resolution algorithm from docs/resolution.md
    fn resolve_in_directory(
        &self,
        root: &Path,
        descriptor: &ImportModuleDescriptor,
    ) -> Option<PathBuf> {
        if descriptor.name_parts.is_empty() {
            // Edge case: empty import (shouldn't happen in practice)
            return None;
        }

        let mut current_path = root.to_path_buf();

        // Process all parts except the last one
        for (i, part) in descriptor.name_parts.iter().enumerate() {
            let is_last = i == descriptor.name_parts.len() - 1;

            if is_last {
                // For the last part, check in order:
                // 1. Package (foo/__init__.py)
                // 2. Module file (foo.py)
                // 3. C extension (foo.so, foo.pyd, etc.)
                // 4. Namespace package (foo/ directory)

                // Check for package first
                let package_init = current_path
                    .join(part)
                    .join(crate::python::constants::INIT_FILE);
                if package_init.is_file() {
                    debug!("Found package at: {}", package_init.display());
                    let canonical = self.canonicalize_path(package_init);
                    return Some(canonical);
                }

                // Check for module file
                let module_file = current_path.join(format!("{part}.py"));
                if module_file.is_file() {
                    debug!("Found module file at: {}", module_file.display());
                    let canonical = self.canonicalize_path(module_file);
                    return Some(canonical);
                }

                // Check for namespace package (directory without __init__.py)
                let namespace_dir = current_path.join(part);
                if crate::python::module_path::is_namespace_package_dir(&namespace_dir) {
                    debug!("Found namespace package at: {}", namespace_dir.display());
                    // Return the directory path to indicate this is a namespace package
                    let canonical = self.canonicalize_path(namespace_dir);
                    return Some(canonical);
                }
            } else {
                // For intermediate parts, they must be packages
                let package_dir = current_path.join(part);
                let package_init = package_dir.join(crate::python::constants::INIT_FILE);

                if package_init.is_file() {
                    current_path = package_dir;
                } else if crate::python::module_path::is_namespace_package_dir(&package_dir) {
                    // Namespace package - continue but don't add to resolved paths
                    current_path = package_dir;
                } else {
                    // Not found
                    return None;
                }
            }
        }

        None
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
            if self.resolve_in_directory(search_dir, &descriptor).is_some() {
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
                    // Before assuming the submodule is first-party, try to resolve it
                    // If we can't find it as a source file, treat it as third-party
                    // This handles cases where submodules are C extensions or otherwise not
                    // available as source files
                    let descriptor = ImportModuleDescriptor::from_module_name(module_name);
                    let mut found_as_source = false;
                    for search_dir in &search_dirs {
                        if self.resolve_in_directory(search_dir, &descriptor).is_some() {
                            found_as_source = true;
                            break;
                        }
                    }

                    if found_as_source {
                        // Found as source file, it's first-party
                        let import_type = ImportType::FirstParty;
                        self.classification_cache
                            .borrow_mut()
                            .insert(module_name.to_string(), import_type.clone());
                        return import_type;
                    } else {
                        // Check if the parent module is a package
                        // If parent is NOT a package (just a .py file), then submodules can't exist
                        // This preserves Python's shadowing behavior

                        // First, try to resolve the parent module to get its path
                        let parent_descriptor =
                            ImportModuleDescriptor::from_module_name(parent_module);
                        let mut parent_is_package = false;
                        let mut parent_found = false;

                        for search_dir in &search_dirs {
                            if let Some(parent_path) =
                                self.resolve_in_directory(search_dir, &parent_descriptor)
                            {
                                parent_found = true;
                                // Check if it's a package (__init__.py) or a module (.py file)
                                parent_is_package = parent_path
                                    .file_name()
                                    .and_then(|n| n.to_str())
                                    .is_some_and(crate::python::module_path::is_init_file_name);
                                break;
                            }
                        }

                        if parent_found && !parent_is_package {
                            // Parent is a module file, not a package - submodules can't exist
                            // This mimics Python's behavior where a .py file shadows a package
                            debug!(
                                "Module '{module_name}' cannot exist - parent '{parent_module}' \
                                 is a module file, not a package (shadowing behavior)"
                            );
                            // Return FirstParty to trigger an error during bundling
                            // (the module won't be found and will cause an appropriate error)
                            let import_type = ImportType::FirstParty;
                            self.classification_cache
                                .borrow_mut()
                                .insert(module_name.to_string(), import_type.clone());
                            return import_type;
                        }

                        // Can't find source file, treat as third-party
                        // This could be a C extension or dynamically available module
                        debug!(
                            "Module '{module_name}' has first-party parent '{parent_module}' but \
                             no source file found - treating as third-party"
                        );
                        let import_type = ImportType::ThirdParty;
                        self.classification_cache
                            .borrow_mut()
                            .insert(module_name.to_string(), import_type.clone());
                        return import_type;
                    }
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

    /// Map an import name to its package name by checking dist-info metadata in the virtual
    /// environment For example: "`markdown_it`" -> "markdown-it-py"
    pub fn map_import_to_package_name(&self, import_name: &str) -> String {
        // Extract the root module name (e.g., "markdown_it" from "markdown_it.parser")
        let root_import = import_name.split('.').next().unwrap_or(import_name);

        debug!("Attempting to map import '{import_name}' (root: '{root_import}') to package name");

        // Check if we have a virtual environment
        let explicit_virtualenv = self
            .virtualenv_override
            .as_deref()
            .map(std::borrow::ToOwned::to_owned)
            .or_else(|| std::env::var("VIRTUAL_ENV").ok());

        let virtualenv_paths = if let Some(virtualenv_path) = explicit_virtualenv {
            vec![PathBuf::from(virtualenv_path)]
        } else {
            // Fallback: detect common virtual environment directory names
            self.detect_fallback_virtualenv_paths()
        };

        // Try to find the package name from dist-info
        for venv_path in virtualenv_paths {
            debug!("Checking venv path: {}", venv_path.display());
            for site_packages_dir in self.get_virtualenv_site_packages_directories(&venv_path) {
                debug!("Checking site-packages: {}", site_packages_dir.display());
                if let Some(package_name) =
                    self.find_package_name_in_site_packages(&site_packages_dir, root_import)
                {
                    debug!("Mapped import '{root_import}' to package '{package_name}'");
                    return package_name;
                }
            }
        }

        // If no mapping found, return the import name as-is
        debug!("No package mapping found for '{root_import}', using import name as-is");
        root_import.to_string()
    }

    /// Normalize a package name according to PEP 503 using `pep508_rs`
    fn normalize_package_name(name: &str) -> String {
        // Use pep508_rs::PackageName for proper PEP 503 normalization
        if let Ok(package_name) = PackageName::new(name.to_string()) {
            package_name.to_string()
        } else {
            // If normalization fails (shouldn't happen for valid package names),
            // fall back to simple lowercase
            debug!("Failed to normalize package name '{name}', using lowercase");
            name.cow_to_lowercase().into_owned()
        }
    }

    /// Find the package name for an import by scanning dist-info directories
    fn find_package_name_in_site_packages(
        &self,
        site_packages_dir: &Path,
        import_name: &str,
    ) -> Option<String> {
        // Look for corresponding dist-info directory
        // Note: We don't check if the import exists first because:
        // - Single-file modules (foo.py)
        // - Compiled extensions (foo.cpython-312-darwin.so)
        // - Namespace packages
        // may not have a directory
        let Ok(entries) = std::fs::read_dir(site_packages_dir) else {
            return None;
        };

        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }

            let Some(dir_name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };

            // Check if this is a dist-info directory
            if !dir_name.ends_with(".dist-info") {
                continue;
            }

            // Check if this dist-info might be related to our import
            // by checking the RECORD file for the import directory
            let record_file = path.join("RECORD");
            if record_file.exists()
                && let Ok(file) = std::fs::File::open(&record_file)
            {
                let reader = BufReader::new(file);
                let mut matches_import = false;
                for line in reader.lines().map_while(Result::ok) {
                    // RECORD entries are CSV; the first field is the path
                    let path_part = line.split(',').next().unwrap_or("");
                    // Normalize separators to forward slash for matching
                    let path_norm = path_part.cow_replace('\\', "/").into_owned();
                    if path_norm == format!("{import_name}.py")
                        || path_norm.starts_with(&format!("{import_name}/"))
                        // Handle compiled extension modules (e.g., ujson.cpython-312-darwin.so)
                        || (path_norm.starts_with(&format!("{import_name}.")) && !path_norm.contains('/'))
                    {
                        matches_import = true;
                        break;
                    }
                }
                if matches_import {
                    // Found the right dist-info, now extract package name from METADATA
                    let metadata_file = path.join("METADATA");
                    if metadata_file.exists()
                        && let Ok(metadata) = std::fs::read_to_string(&metadata_file)
                    {
                        for line in metadata.lines() {
                            if let Some(name) = line.strip_prefix("Name: ") {
                                let normalized = Self::normalize_package_name(name.trim());
                                return Some(normalized);
                            }
                        }
                    }
                }
            }
        }

        None
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

        // If name is provided, split it and append. Trim any leading dots to avoid
        // accidental empty components (e.g., "._types").
        if let Some(raw_name) = name {
            let cleaned = raw_name.trim_start_matches('.');
            if !cleaned.is_empty() {
                let name_parts: Vec<&str> = cleaned.split('.').filter(|s| !s.is_empty()).collect();
                current_parts.extend(name_parts.into_iter().map(std::string::ToString::to_string));
            }
        }

        if current_parts.is_empty() {
            // If we're at the root after applying relative levels, return empty string
            // This will be handled by the caller to construct the full import name
            Some(String::new())
        } else {
            Some(current_parts.join("."))
        }
    }

    /// Resolve a relative import given a module or package name (not path)
    /// This is used when we have a module string like "foo.bar.baz" instead of a file path
    ///
    /// For relative imports:
    /// - level=1 (from . import x): Import from the same package as the current module
    /// - level=2 (from .. import x): Import from the parent package
    /// - level=3 (from ... import x): Import from the grandparent package, etc.
    pub fn resolve_relative_import_from_package_name(
        &self,
        level: u32,
        name: Option<&str>,
        current_module_name: &str,
    ) -> String {
        // Determine if current_module_name is a package (__init__ or namespace)
        let mut parts: Vec<&str> = current_module_name.split('.').collect();
        let current_is_package = self.get_module_id_by_name(current_module_name).map_or_else(
            || parts.len() == 1,
            |id| self.is_package_init(id) || self.is_namespace_package(id),
        );

        // For regular modules, drop the last segment; for packages, keep it
        if !current_is_package && parts.len() > 1 {
            parts.pop();
        }

        for _ in 1..level {
            if parts.is_empty() {
                break;
            }
            parts.pop();
        }

        if let Some(name_part) = name.filter(|s| !s.is_empty()) {
            parts.push(name_part);
        }

        let result = parts.join(".");
        debug!(
            "Resolved relative import: level={level}, name={name:?}, from '{current_module_name}' \
             → '{result}'"
        );
        result
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

        // Convert current_dir to absolute path if it's relative and canonicalize it
        let absolute_current_dir = if current_dir.is_absolute() {
            current_dir
                .canonicalize()
                .unwrap_or_else(|_| current_dir.to_path_buf())
        } else {
            let current_working_dir = std::env::current_dir().ok()?;
            let joined = current_working_dir.join(current_dir);
            joined.canonicalize().unwrap_or(joined)
        };

        // Find which search directory (entry dir, PYTHONPATH, or src) contains this file
        let search_dirs = self.get_search_directories();
        let relative_dir = search_dirs.iter().find_map(|dir| {
            // The search directories are already canonicalized/absolute from get_search_directories
            absolute_current_dir.strip_prefix(dir).ok()
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

    /// Register a module - entry gets 0, others get sequential IDs
    pub fn register_module(&self, name: String, path: &Path) -> ModuleId {
        let mut registry = self.registry.lock().expect("Module registry lock poisoned");

        let id = registry.register(name.clone(), path);

        if id.is_entry() {
            info!("Registered ENTRY module '{name}' at the origin (ID 0)");
        } else {
            debug!(
                "Registered module '{}' with ID {} (package: {})",
                name,
                id.as_u32(),
                registry.get_metadata(id).is_some_and(|m| m.is_package)
            );
        }

        id
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
        create_test_file(
            &root.join(format!("foo/{}", crate::python::constants::INIT_FILE)),
            "# Package",
        )?;
        create_test_file(&root.join("foo.py"), "# Module")?;

        let config = Config {
            src: vec![root.to_path_buf()],
            ..Default::default()
        };
        let resolver = ModuleResolver::new(config);

        // Resolve foo - should prefer foo/__init__.py
        let result = resolver.resolve_module_path("foo")?;
        let expected = root
            .join(format!("foo/{}", crate::python::constants::INIT_FILE))
            .canonicalize()?;
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
        let mut resolver = ModuleResolver::new(config);
        resolver.set_entry_file(&entry_file, &entry_file);

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
        create_test_file(
            &root.join(format!("myapp/{}", crate::python::constants::INIT_FILE)),
            "",
        )?;
        create_test_file(
            &root.join(format!(
                "myapp/utils/{}",
                crate::python::constants::INIT_FILE
            )),
            "",
        )?;
        create_test_file(&root.join("myapp/utils/helpers.py"), "")?;

        let config = Config {
            src: vec![root.to_path_buf()],
            ..Default::default()
        };
        let resolver = ModuleResolver::new(config);

        // Test various imports
        assert_eq!(
            resolver.resolve_module_path("myapp")?.map(|p| p
                .canonicalize()
                .expect("failed to canonicalize resolved path")),
            Some(
                root.join(format!("myapp/{}", crate::python::constants::INIT_FILE))
                    .canonicalize()?
            )
        );
        assert_eq!(
            resolver.resolve_module_path("myapp.utils")?.map(|p| p
                .canonicalize()
                .expect("failed to canonicalize resolved path")),
            Some(
                root.join(format!(
                    "myapp/utils/{}",
                    crate::python::constants::INIT_FILE
                ))
                .canonicalize()?
            )
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
        let resolver = ModuleResolver::new(config);

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
        let resolver = ModuleResolver::new(config);

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
        create_test_file(
            &root.join(format!("mypackage/{}", crate::python::constants::INIT_FILE)),
            "# Package init",
        )?;
        create_test_file(&root.join("mypackage/module1.py"), "# Module 1")?;
        create_test_file(
            &root.join(format!(
                "mypackage/subpackage/{}",
                crate::python::constants::INIT_FILE
            )),
            "# Subpackage init",
        )?;
        create_test_file(&root.join("mypackage/subpackage/module2.py"), "# Module 2")?;
        create_test_file(
            &root.join(format!(
                "mypackage/subpackage/deeper/{}",
                crate::python::constants::INIT_FILE
            )),
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
        let resolver = ModuleResolver::new(config);

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
                root.join(format!(
                    "mypackage/subpackage/deeper/{}",
                    crate::python::constants::INIT_FILE
                ))
                .canonicalize()?
            )
        );

        // Test "from .. import" (parent package)
        assert_eq!(
            resolver.resolve_module_path_with_context("..", Some(&module3_path))?,
            Some(
                root.join(format!(
                    "mypackage/subpackage/{}",
                    crate::python::constants::INIT_FILE
                ))
                .canonicalize()?
            )
        );

        // Test relative import from a package __init__.py
        let subpackage_init = root.join(format!(
            "mypackage/subpackage/{}",
            crate::python::constants::INIT_FILE
        ));

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
        let pythonpath_pkg_init = pythonpath_pkg.join(crate::python::constants::INIT_FILE);
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
        let resolver = ModuleResolver::new_with_overrides(config, Some(&pythonpath_str), None);

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
        let resolver = ModuleResolver::new_with_overrides(config, Some(&pythonpath_str), None);

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
        let resolver = ModuleResolver::new_with_overrides(config, Some(&pythonpath_str), None);

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
        let resolver1 = ModuleResolver::new_with_overrides(config.clone(), Some(""), None);

        // Should be able to resolve module from src directory
        assert!(
            resolver1.resolve_module_path("test_module")?.is_some(),
            "Should resolve module from src directory with empty PYTHONPATH"
        );

        // Test with no PYTHONPATH
        let resolver2 = ModuleResolver::new_with_overrides(config.clone(), None, None);

        // Should be able to resolve module from src directory
        assert!(
            resolver2.resolve_module_path("test_module")?.is_some(),
            "Should resolve module from src directory with no PYTHONPATH"
        );

        // Test with nonexistent directories in PYTHONPATH
        let separator = if cfg!(windows) { ';' } else { ':' };
        let nonexistent_pythonpath = format!("/nonexistent1{separator}/nonexistent2");
        let resolver3 =
            ModuleResolver::new_with_overrides(config, Some(&nonexistent_pythonpath), None);

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
        let resolver = ModuleResolver::new_with_overrides(config, Some(&pythonpath_str), None);

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
        let resolver = ModuleResolver::new_with_overrides(config, Some(&pythonpath_str), None);

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
