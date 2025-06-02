use anyhow::Result;
use log::debug;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

use crate::config::Config;
use crate::python_stdlib::is_stdlib_module;

/// A scoped guard for safely setting and cleaning up the PYTHONPATH environment variable.
///
/// This guard ensures that the PYTHONPATH environment variable is properly restored
/// to its original value when the guard is dropped, even if a panic occurs during testing.
///
/// # Example
///
/// ```rust
/// use serpen::resolver::PythonPathGuard;
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
    module_cache: HashMap<String, Option<PathBuf>>,
    /// Set of all first-party modules discovered in src directories
    first_party_modules: HashSet<String>,
}

impl ModuleResolver {
    pub fn new(config: Config) -> Result<Self> {
        Self::new_with_pythonpath(config, None)
    }

    /// Create a new ModuleResolver with optional PYTHONPATH override for testing
    pub fn new_with_pythonpath(config: Config, pythonpath_override: Option<&str>) -> Result<Self> {
        let mut resolver = Self {
            config,
            module_cache: HashMap::new(),
            first_party_modules: HashSet::new(),
        };

        resolver.discover_first_party_modules_with_pythonpath(pythonpath_override)?;
        Ok(resolver)
    }

    /// Get all directories to scan for modules (configured src + PYTHONPATH)
    /// Returns deduplicated, canonicalized paths
    pub fn get_scan_directories(&self) -> Vec<PathBuf> {
        self.get_scan_directories_with_pythonpath(None)
    }

    /// Get all directories to scan for modules with optional PYTHONPATH override
    /// Returns deduplicated, canonicalized paths
    pub fn get_scan_directories_with_pythonpath(
        &self,
        pythonpath_override: Option<&str>,
    ) -> Vec<PathBuf> {
        let mut unique_dirs = HashSet::new();

        // Add configured src directories
        for dir in &self.config.src {
            if let Ok(canonical) = dir.canonicalize() {
                unique_dirs.insert(canonical);
            } else {
                // If canonicalize fails (e.g., path doesn't exist), use the original path
                unique_dirs.insert(dir.clone());
            }
        }

        // Add PYTHONPATH directories
        let pythonpath = pythonpath_override
            .map(|p| p.to_string())
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
    fn add_pythonpath_directory(&self, unique_dirs: &mut HashSet<PathBuf>, path_str: &str) {
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

    /// Discover first-party modules with optional PYTHONPATH override
    /// This method is useful for testing to avoid environment variable pollution
    fn discover_first_party_modules_with_pythonpath(
        &mut self,
        pythonpath_override: Option<&str>,
    ) -> Result<()> {
        let directories_to_scan = self.get_scan_directories_with_pythonpath(pythonpath_override);

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
                    .map(|s| s.to_string());
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
        if is_stdlib_module(module_name) {
            return ImportType::StandardLibrary;
        }

        // Check if it's explicitly configured as third-party
        if self.config.known_third_party.contains(module_name) {
            return ImportType::ThirdParty;
        }

        // Check if it's a first-party module (exact match or parent)
        if self.is_first_party_module(module_name) {
            return ImportType::FirstParty;
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
            self.module_cache.insert(module_name.to_string(), None);
            return Ok(None);
        }

        // Try to find the module file in all directories
        let directories_to_search = self.get_scan_directories();
        for src_dir in &directories_to_search {
            if let Some(path) = self.find_module_file(src_dir, module_name)? {
                self.module_cache
                    .insert(module_name.to_string(), Some(path.clone()));
                return Ok(Some(path));
            }
        }

        self.module_cache.insert(module_name.to_string(), None);
        Ok(None)
    }

    /// Find the file for a given module name in a source directory
    #[allow(clippy::excessive_nesting)]
    fn find_module_file(&self, src_dir: &Path, module_name: &str) -> Result<Option<PathBuf>> {
        let parts: Vec<&str> = module_name.split('.').collect();

        // Try as a regular module file
        let mut file_path = src_dir.to_path_buf();
        for (i, part) in parts.iter().enumerate() {
            if i == parts.len() - 1 {
                // Last part - try both module file and package
                if let Some(found_path) = self.try_resolve_final_part(&mut file_path, part) {
                    return Ok(Some(found_path));
                }
            } else {
                file_path.push(part);
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
    pub fn get_first_party_modules(&self) -> &HashSet<String> {
        &self.first_party_modules
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use std::collections::{HashMap, HashSet};
    use std::path::Path;

    #[test]
    fn test_root_init_py_module_name() {
        // Root __init__.py should map to its directory name
        let src_dir = Path::new("/path/to/mypkg");
        let file_path = Path::new("/path/to/mypkg/__init__.py");
        let resolver = ModuleResolver {
            config: Config::default(),
            module_cache: HashMap::new(),
            first_party_modules: HashSet::new(),
        };
        assert_eq!(
            resolver.path_to_module_name(src_dir, file_path),
            Some("mypkg".to_string())
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
            module_cache: HashMap::new(),
            first_party_modules: HashSet::new(),
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
            module_cache: HashMap::new(),
            first_party_modules: HashSet::new(),
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
