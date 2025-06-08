use anyhow::Result;
use indexmap::{IndexMap, IndexSet};
use log::debug;
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

use crate::config::Config;
use ruff_python_stdlib::sys;

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

#[derive(Debug)]
pub struct ModuleResolver {
    config: Config,
    /// Cache of resolved module paths
    module_cache: IndexMap<String, Option<PathBuf>>,
    /// Set of all first-party modules discovered in src directories
    first_party_modules: IndexSet<String>,
    /// Cache of virtual environment packages to avoid repeated filesystem scans
    virtualenv_packages_cache: RefCell<Option<IndexSet<String>>>,
}

impl ModuleResolver {
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
        let mut resolver = Self {
            config,
            module_cache: IndexMap::new(),
            first_party_modules: IndexSet::new(),
            virtualenv_packages_cache: RefCell::new(None),
        };

        resolver.discover_first_party_modules_with_overrides(
            pythonpath_override,
            virtualenv_override,
        )?;
        Ok(resolver)
    }

    /// Get all directories to scan for modules (configured src + PYTHONPATH + VIRTUAL_ENV)
    /// Returns deduplicated, canonicalized paths
    pub fn get_scan_directories(&self) -> Vec<PathBuf> {
        self.get_scan_directories_with_overrides(None, None)
    }

    /// Get all directories to scan for modules with optional PYTHONPATH override
    /// Returns deduplicated, canonicalized paths
    pub fn get_scan_directories_with_pythonpath(
        &self,
        pythonpath_override: Option<&str>,
    ) -> Vec<PathBuf> {
        self.get_scan_directories_with_overrides(pythonpath_override, None)
    }

    /// Get all directories to scan for modules with optional VIRTUAL_ENV override
    /// Returns deduplicated, canonicalized paths
    pub fn get_scan_directories_with_virtualenv(
        &self,
        virtualenv_override: Option<&str>,
    ) -> Vec<PathBuf> {
        self.get_scan_directories_with_overrides(None, virtualenv_override)
    }

    /// Get all directories to scan for modules with optional PYTHONPATH override
    /// Returns deduplicated, canonicalized paths
    /// NOTE: VIRTUAL_ENV is NOT included here as it's used for third-party classification, not first-party discovery
    pub fn get_scan_directories_with_overrides(
        &self,
        pythonpath_override: Option<&str>,
        _virtualenv_override: Option<&str>,
    ) -> Vec<PathBuf> {
        let mut unique_dirs = IndexSet::new();

        // Add configured src directories
        for dir in &self.config.src {
            if let Ok(canonical) = dir.canonicalize() {
                unique_dirs.insert(canonical);
            } else {
                // If canonicalize fails (e.g., path doesn't exist), use the original path
                unique_dirs.insert(dir.clone());
            }
        }

        // Add PYTHONPATH directories (for first-party module discovery)
        let pythonpath = pythonpath_override
            .map(|p| p.to_owned())
            .or_else(|| std::env::var("PYTHONPATH").ok());

        if let Some(pythonpath) = pythonpath {
            // Use platform-appropriate path separator: ';' on Windows, ':' on Unix
            let separator = if cfg!(windows) { ';' } else { ':' };
            for path_str in pythonpath.split(separator) {
                self.add_pythonpath_directory(&mut unique_dirs, path_str);
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
            // If canonicalize fails but path exists, use the original path
            unique_dirs.insert(path);
        }
    }

    /// Detect common virtual environment directory names in the current working directory
    /// Returns a list of paths that appear to be virtual environments
    fn detect_fallback_virtualenv_paths(&self) -> Vec<PathBuf> {
        let current_dir = match std::env::current_dir() {
            Ok(dir) => dir,
            Err(_) => return Vec::new(),
        };

        self.scan_common_venv_names(current_dir.as_path())
    }

    /// Scan common virtual environment directory names in the given directory
    fn scan_common_venv_names(&self, current_dir: &Path) -> Vec<PathBuf> {
        let common_venv_names = [".venv", "venv", "env", ".virtualenv", "virtualenv"];
        let mut venv_paths = Vec::new();

        for venv_name in &common_venv_names {
            if let Some(venv_path) = self.validate_potential_venv(current_dir, venv_name) {
                venv_paths.push(venv_path);
            }
        }

        venv_paths
    }

    /// Validate if a potential virtual environment directory is actually a venv
    fn validate_potential_venv(&self, current_dir: &Path, venv_name: &str) -> Option<PathBuf> {
        let potential_venv = current_dir.join(venv_name);

        if !self.is_valid_directory(&potential_venv) {
            return None;
        }

        let site_packages_dirs =
            self.get_virtualenv_site_packages_directories(&potential_venv.to_string_lossy());

        if site_packages_dirs.is_empty() {
            None
        } else {
            Some(potential_venv)
        }
    }

    /// Check if a path is a valid existing directory
    fn is_valid_directory(&self, path: &Path) -> bool {
        path.exists() && path.is_dir()
    }

    /// Get site-packages directories from a virtual environment path
    /// This is used for third-party dependency detection, not first-party module discovery
    fn get_virtualenv_site_packages_directories(&self, virtualenv_path: &str) -> Vec<PathBuf> {
        let venv_root = PathBuf::from(virtualenv_path);

        if !venv_root.exists() || !venv_root.is_dir() {
            return Vec::new();
        }

        if cfg!(windows) {
            self.get_windows_site_packages(&venv_root)
        } else {
            self.get_unix_site_packages(&venv_root)
        }
    }

    /// Get site-packages directories for Windows virtual environments
    fn get_windows_site_packages(&self, venv_root: &Path) -> Vec<PathBuf> {
        let site_packages = venv_root.join("Lib").join("site-packages");

        if site_packages.exists() && site_packages.is_dir() {
            vec![site_packages]
        } else {
            Vec::new()
        }
    }

    /// Get site-packages directories for Unix-like virtual environments
    fn get_unix_site_packages(&self, venv_root: &Path) -> Vec<PathBuf> {
        let lib_dir = venv_root.join("lib");

        if !lib_dir.exists() || !lib_dir.is_dir() {
            return Vec::new();
        }

        self.scan_lib_directory_for_python_versions(&lib_dir)
    }

    /// Scan lib directory for Python version directories containing site-packages
    fn scan_lib_directory_for_python_versions(&self, lib_dir: &Path) -> Vec<PathBuf> {
        let mut site_packages_dirs = Vec::new();

        let Ok(entries) = std::fs::read_dir(lib_dir) else {
            return site_packages_dirs;
        };

        for entry in entries.flatten() {
            if let Some(site_packages) = self.check_python_version_directory(&entry.path()) {
                site_packages_dirs.push(site_packages);
            }
        }

        site_packages_dirs
    }

    /// Check if a directory is a Python version directory with site-packages
    fn check_python_version_directory(&self, path: &Path) -> Option<PathBuf> {
        if !path.is_dir() {
            return None;
        }

        let name = path.file_name().and_then(|n| n.to_str())?;

        if !name.starts_with("python") {
            return None;
        }

        let site_packages = path.join("site-packages");
        if site_packages.exists() && site_packages.is_dir() {
            Some(site_packages)
        } else {
            None
        }
    }

    /// Get the set of third-party packages installed in the virtual environment
    /// Used for improving import classification accuracy
    fn get_virtualenv_packages(&self, virtualenv_override: Option<&str>) -> IndexSet<String> {
        // If we have a cached result and no override is specified, return it
        if virtualenv_override.is_none() {
            if let Some(cached_packages) = self.get_cached_virtualenv_packages() {
                return cached_packages;
            }
        }

        // Compute the packages
        self.compute_virtualenv_packages(virtualenv_override)
    }

    /// Get cached virtualenv packages if available
    fn get_cached_virtualenv_packages(&self) -> Option<IndexSet<String>> {
        let cache_ref = self.virtualenv_packages_cache.try_borrow().ok()?;
        cache_ref.as_ref().cloned()
    }

    /// Compute virtualenv packages by scanning the filesystem
    fn compute_virtualenv_packages(&self, virtualenv_override: Option<&str>) -> IndexSet<String> {
        let mut packages = IndexSet::new();

        // First, try to get explicit VIRTUAL_ENV (either override or environment variable)
        let explicit_virtualenv = virtualenv_override
            .map(|v| v.to_owned())
            .or_else(|| std::env::var("VIRTUAL_ENV").ok());

        let virtualenv_paths = if let Some(virtualenv_path) = explicit_virtualenv {
            // Use explicit VIRTUAL_ENV if provided
            vec![PathBuf::from(virtualenv_path)]
        } else {
            // Fallback: detect common virtual environment directory names
            self.detect_fallback_virtualenv_paths()
        };

        // Scan all discovered virtual environment paths
        for venv_path in virtualenv_paths {
            let virtualenv_path_str = venv_path.to_string_lossy();
            for site_packages_dir in
                self.get_virtualenv_site_packages_directories(&virtualenv_path_str)
            {
                self.scan_site_packages_directory(&site_packages_dir, &mut packages);
            }
        }

        // Cache the result if no override was specified (for subsequent calls)
        if virtualenv_override.is_none() {
            if let Ok(mut cache_ref) = self.virtualenv_packages_cache.try_borrow_mut() {
                *cache_ref = Some(packages.clone());
            }
        }

        packages
    }

    /// Scan a site-packages directory and add found packages to the set
    fn scan_site_packages_directory(
        &self,
        site_packages_dir: &PathBuf,
        packages: &mut IndexSet<String>,
    ) {
        let Ok(entries) = std::fs::read_dir(site_packages_dir) else {
            return;
        };

        for entry in entries.flatten() {
            let path = entry.path();
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
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
    /// Used for improving import classification accuracy
    fn is_virtualenv_package(&self, module_name: &str) -> bool {
        let virtualenv_packages = self.get_virtualenv_packages(None);

        // Check for exact match
        if virtualenv_packages.contains(module_name) {
            return true;
        }

        // Check if this is a submodule of a virtual environment package
        // e.g., for "requests.auth", check if "requests" is in virtualenv
        if let Some(root_module) = module_name.split('.').next() {
            if virtualenv_packages.contains(root_module) {
                return true;
            }
        }

        false
    }

    /// Discover first-party modules with optional PYTHONPATH override
    /// This method is useful for testing to avoid environment variable pollution
    #[allow(dead_code)]
    fn discover_first_party_modules_with_pythonpath(
        &mut self,
        pythonpath_override: Option<&str>,
    ) -> Result<()> {
        self.discover_first_party_modules_with_overrides(pythonpath_override, None)
    }

    /// Discover first-party modules with optional PYTHONPATH and VIRTUAL_ENV overrides
    /// This method is useful for testing to avoid environment variable pollution
    fn discover_first_party_modules_with_overrides(
        &mut self,
        pythonpath_override: Option<&str>,
        virtualenv_override: Option<&str>,
    ) -> Result<()> {
        let directories_to_scan =
            self.get_scan_directories_with_overrides(pythonpath_override, virtualenv_override);

        for src_dir in &directories_to_scan {
            self.scan_directory_for_modules(src_dir)?;
        }

        // Add configured known first-party modules
        for module_name in &self.config.known_first_party {
            self.first_party_modules.insert(module_name.clone());
        }

        Ok(())
    }

    /// Scan a single directory for Python modules
    fn scan_directory_for_modules(&mut self, src_dir: &Path) -> Result<()> {
        if !src_dir.exists() {
            return Ok(());
        }

        debug!("Scanning source directory: {:?}", src_dir);

        let entries = WalkDir::new(src_dir)
            .follow_links(false)
            .into_iter()
            .filter_map(|e| e.ok());

        for entry in entries {
            self.process_directory_entry(src_dir, entry.path());
        }

        Ok(())
    }

    /// Process a single directory entry and add it as a module if it's a Python file
    fn process_directory_entry(&mut self, src_dir: &Path, path: &Path) {
        if !self.is_python_file(path) {
            return;
        }

        if let Some(module_name) = self.path_to_module_name(src_dir, path) {
            debug!("Found first-party module: {}", module_name);
            self.first_party_modules.insert(module_name.clone());
            self.module_cache
                .insert(module_name, Some(path.to_path_buf()));
        }
    }

    /// Check if a path is a Python file
    fn is_python_file(&self, path: &Path) -> bool {
        path.is_file()
            && path
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("py"))
    }

    /// Convert a file path to a Python module name
    fn path_to_module_name(&self, src_dir: &Path, file_path: &Path) -> Option<String> {
        // Handle root __init__.py specially
        if let Ok(relative) = file_path.strip_prefix(src_dir) {
            if relative.components().count() == 1
                && relative.file_name().and_then(|n| n.to_str()) == Some("__init__.py")
            {
                return src_dir
                    .file_name()
                    .and_then(|os| os.to_str())
                    .map(|s| s.to_owned());
            }
        }
        crate::util::path_to_module_name(src_dir, file_path)
    }

    /// Classify an import as first-party, third-party, or standard library
    pub fn classify_import(&self, module_name: &str) -> ImportType {
        // Check if it's a relative import (starts with a dot)
        if module_name.starts_with('.') {
            return ImportType::FirstParty;
        }

        // Check if it's a standard library module
        if let Ok(python_version) = self.config.python_version() {
            if is_stdlib_module(module_name, python_version) {
                return ImportType::StandardLibrary;
            }
        }

        // Check if it's explicitly configured as third-party
        if self.config.known_third_party.contains(module_name) {
            return ImportType::ThirdParty;
        }

        // Check if it's a first-party module (exact match or parent)
        if self.is_first_party_module(module_name) {
            return ImportType::FirstParty;
        }

        // Use VIRTUAL_ENV to improve third-party vs standard library classification
        // If the module exists in the virtual environment, it's definitely third-party
        if self.is_virtualenv_package(module_name) {
            return ImportType::ThirdParty;
        }

        // Default to third-party
        ImportType::ThirdParty
    }

    /// Check if a module is first-party
    fn is_first_party_module(&self, module_name: &str) -> bool {
        // Exact match
        if self.first_party_modules.contains(module_name) {
            return true;
        }

        // Check if any discovered module starts with this name (submodule)
        for first_party_module in &self.first_party_modules {
            if first_party_module.starts_with(&format!("{}.", module_name)) {
                return true;
            }
        }

        // Check if this is a submodule of any first-party module
        let parts: Vec<&str> = module_name.split('.').collect();
        for i in 1..parts.len() {
            let parent_module = parts[..i].join(".");
            if self.first_party_modules.contains(&parent_module) {
                return true;
            }
        }

        false
    }

    /// Resolve a module name to its file path (for first-party modules only)
    pub fn resolve_module_path(&mut self, module_name: &str) -> Result<Option<PathBuf>> {
        // Check cache first
        if let Some(cached_path) = self.module_cache.get(module_name) {
            return Ok(cached_path.clone());
        }

        // Only resolve first-party modules
        if !self.is_first_party_module(module_name) {
            self.module_cache.insert(module_name.to_owned(), None);
            return Ok(None);
        }

        // Try to find the module file in all directories
        let directories_to_search = self.get_scan_directories();
        for src_dir in &directories_to_search {
            if let Some(path) = self.find_module_file(src_dir, module_name)? {
                self.module_cache
                    .insert(module_name.to_owned(), Some(path.clone()));
                return Ok(Some(path));
            }
        }

        self.module_cache.insert(module_name.to_owned(), None);
        Ok(None)
    }

    /// Find the file for a given module name in a source directory
    fn find_module_file(&self, src_dir: &Path, module_name: &str) -> Result<Option<PathBuf>> {
        let parts: Vec<&str> = module_name.split('.').collect();
        let mut file_path = src_dir.to_path_buf();

        // Build the directory path up to the final part
        for part in parts.iter().take(parts.len().saturating_sub(1)) {
            file_path.push(part);
        }

        // Try to resolve the final part as either a file or package
        if let Some(final_part) = parts.last() {
            if let Some(found_path) = self.try_resolve_final_part(&mut file_path, final_part) {
                return Ok(Some(found_path));
            }
        }

        Ok(None)
    }

    /// Try to resolve the final part of a module path as either a file or package
    fn try_resolve_final_part(&self, file_path: &mut PathBuf, part: &str) -> Option<PathBuf> {
        // Try as a .py file
        file_path.push(format!("{}.py", part));
        if file_path.exists() {
            return Some(file_path.clone());
        }

        // Try as a package with __init__.py
        file_path.pop();
        file_path.push(part);
        file_path.push("__init__.py");
        if file_path.exists() {
            Some(file_path.clone())
        } else {
            None
        }
    }

    /// Get all discovered first-party modules
    pub fn get_first_party_modules(&self) -> &IndexSet<String> {
        &self.first_party_modules
    }

    /// Get a reference to the configuration
    pub fn config(&self) -> &Config {
        &self.config
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use std::path::Path;

    #[test]
    fn test_root_init_py_module_name() {
        // Root __init__.py should map to its directory name
        let src_dir = Path::new("/path/to/mypkg");
        let file_path = Path::new("/path/to/mypkg/__init__.py");
        let resolver = ModuleResolver {
            config: Config::default(),
            module_cache: IndexMap::new(),
            first_party_modules: IndexSet::new(),
            virtualenv_packages_cache: RefCell::new(None),
        };
        assert_eq!(
            resolver.path_to_module_name(src_dir, file_path),
            Some("mypkg".to_owned())
        );
    }

    #[test]
    fn test_get_scan_directories_with_pythonpath() {
        let config = Config {
            src: vec![PathBuf::from("/src1"), PathBuf::from("/src2")],
            ..Config::default()
        };
        let resolver = ModuleResolver {
            config,
            module_cache: IndexMap::new(),
            first_party_modules: IndexSet::new(),
            virtualenv_packages_cache: RefCell::new(None),
        };

        // Use scope guard to safely set PYTHONPATH for testing
        let separator = if cfg!(windows) { ';' } else { ':' };
        let pythonpath_value = format!(
            "/pythonpath1{}/pythonpath2{}/nonexistent",
            separator, separator
        );
        let _guard = PythonPathGuard::new(&pythonpath_value);

        let scan_dirs = resolver.get_scan_directories();

        // Should contain configured src directories
        assert!(scan_dirs.contains(&PathBuf::from("/src1")));
        assert!(scan_dirs.contains(&PathBuf::from("/src2")));

        // Note: PYTHONPATH directories are only included if they exist,
        // so we can't test for their presence without creating actual directories

        // No manual cleanup needed - guard handles it automatically
    }

    #[test]
    fn test_get_scan_directories_without_pythonpath() {
        let config = Config {
            src: vec![PathBuf::from("/src1"), PathBuf::from("/src2")],
            ..Config::default()
        };
        let resolver = ModuleResolver {
            config,
            module_cache: IndexMap::new(),
            first_party_modules: IndexSet::new(),
            virtualenv_packages_cache: RefCell::new(None),
        };

        // Use scope guard to ensure PYTHONPATH is not set
        let _guard = PythonPathGuard::unset();

        let scan_dirs = resolver.get_scan_directories();

        // Should only contain configured src directories
        assert_eq!(scan_dirs.len(), 2);
        assert!(scan_dirs.contains(&PathBuf::from("/src1")));
        assert!(scan_dirs.contains(&PathBuf::from("/src2")));
    }
}
