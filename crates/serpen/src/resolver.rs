use anyhow::{Context, Result};
use log::debug;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

use crate::config::Config;
use crate::python_stdlib::is_stdlib_module;

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
        let mut resolver = Self {
            config,
            module_cache: HashMap::new(),
            first_party_modules: HashSet::new(),
        };

        resolver.discover_first_party_modules()?;
        Ok(resolver)
    }

    /// Scan src directories to discover all first-party Python modules
    fn discover_first_party_modules(&mut self) -> Result<()> {
        for src_dir in &self.config.src {
            if !src_dir.exists() {
                continue;
            }

            debug!("Scanning source directory: {:?}", src_dir);

            for entry in WalkDir::new(src_dir)
                .follow_links(false)
                .into_iter()
                .filter_map(|e| e.ok())
            {
                let path = entry.path();

                // Skip non-Python files
                if !path.is_file()
                    || path
                        .extension()
                        .is_none_or(|ext| !ext.eq_ignore_ascii_case("py"))
                {
                    continue;
                }

                // Convert file path to module name
                if let Some(module_name) = self.path_to_module_name(src_dir, path) {
                    debug!("Found first-party module: {}", module_name);
                    self.first_party_modules.insert(module_name.clone());
                    self.module_cache
                        .insert(module_name, Some(path.to_path_buf()));
                }
            }
        }

        // Add configured known first-party modules
        for module_name in &self.config.known_first_party {
            self.first_party_modules.insert(module_name.clone());
        }

        Ok(())
    }

    /// Convert a file path to a Python module name
    fn path_to_module_name(&self, src_dir: &Path, file_path: &Path) -> Option<String> {
        let relative_path = file_path.strip_prefix(src_dir).ok()?;

        let module_parts: Vec<String> = relative_path
            .components()
            .map(|component| component.as_os_str().to_string_lossy().to_string())
            .collect();

        if module_parts.is_empty() {
            return None;
        }

        let mut parts = module_parts;
        let last_part = parts.last_mut()?;

        // Remove .py extension
        if last_part.ends_with(".py") {
            *last_part = last_part[..last_part.len() - 3].to_string();
        }

        // Handle __init__.py files
        if last_part == "__init__" {
            parts.pop();
            // Root __init__.py at source directory: use directory name as module name
            if parts.is_empty() {
                if let Some(dir_name) = src_dir.file_name().and_then(|os| os.to_str()) {
                    return Some(dir_name.to_string());
                } else {
                    return None;
                }
            }
        }

        // Skip files that don't map to a module (e.g., empty parts)
        if parts.is_empty() {
            return None;
        }

        Some(parts.join("."))
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

        // Try to find the module file
        for src_dir in &self.config.src {
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
    fn find_module_file(&self, src_dir: &Path, module_name: &str) -> Result<Option<PathBuf>> {
        let parts: Vec<&str> = module_name.split('.').collect();

        // Try as a regular module file
        let mut file_path = src_dir.to_path_buf();
        for (i, part) in parts.iter().enumerate() {
            if i == parts.len() - 1 {
                // Last part - could be a .py file
                file_path.push(format!("{}.py", part));
                if file_path.exists() {
                    return Ok(Some(file_path));
                }

                // Or could be a package with __init__.py
                file_path.pop();
                file_path.push(part);
                file_path.push("__init__.py");
                if file_path.exists() {
                    return Ok(Some(file_path));
                }
            } else {
                file_path.push(part);
            }
        }

        Ok(None)
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
}
