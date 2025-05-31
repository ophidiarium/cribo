use anyhow::{anyhow, Context, Result};
use log::{debug, info, warn};
use regex::Regex;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use crate::config::Config;
use crate::dependency_graph::{DependencyGraph, ModuleNode};
use crate::emit::CodeEmitter;
use crate::resolver::{ImportType, ModuleResolver};

pub struct Bundler {
    config: Config,
}

impl Bundler {
    pub fn new(config: Config) -> Self {
        Self { config }
    }

    /// Main bundling function
    pub fn bundle(
        &mut self,
        entry_path: &Path,
        output_path: &Path,
        emit_requirements: bool,
    ) -> Result<()> {
        info!("Starting bundle process");
        debug!("Entry: {:?}, Output: {:?}", entry_path, output_path);

        // Auto-detect the entry point's directory as a source directory
        if let Some(entry_dir) = entry_path.parent() {
            let entry_dir = entry_dir.to_path_buf();
            if !self.config.src.contains(&entry_dir) {
                debug!("Adding entry directory to src paths: {:?}", entry_dir);
                self.config.src.insert(0, entry_dir);
            }
        }

        // Initialize resolver
        let mut resolver = ModuleResolver::new(self.config.clone())?;

        // Find the entry module name
        let entry_module_name = self.find_entry_module_name(entry_path, &resolver)?;
        info!("Entry module: {}", entry_module_name);

        // Build dependency graph
        let mut graph =
            self.build_dependency_graph(entry_path, &entry_module_name, &mut resolver)?;

        // Filter to only modules reachable from entry
        debug!(
            "Before filtering - graph has {} modules",
            graph.get_modules().len()
        );
        graph = graph.filter_reachable_from(&entry_module_name)?;
        debug!(
            "After filtering - graph has {} modules",
            graph.get_modules().len()
        );

        // Check for cycles
        if graph.has_cycles() {
            return Err(anyhow!(
                "Circular dependencies detected in the module graph"
            ));
        }

        // Get topologically sorted modules
        let sorted_modules = graph.topological_sort()?;
        info!("Found {} modules to bundle", sorted_modules.len());
        for (i, module) in sorted_modules.iter().enumerate() {
            debug!("Module {}: {} ({:?})", i, module.name, module.path);
        }

        // Generate bundled code
        let mut emitter = CodeEmitter::new(
            resolver,
            self.config.preserve_comments,
            self.config.preserve_type_hints,
        );

        let bundled_code = emitter.emit_bundle(&sorted_modules, &entry_module_name)?;

        // Write output file
        fs::write(output_path, bundled_code)
            .with_context(|| format!("Failed to write output file: {:?}", output_path))?;

        info!("Bundle written to: {:?}", output_path);

        // Generate requirements.txt if requested
        if emit_requirements {
            let requirements_content = emitter.generate_requirements(&sorted_modules)?;
            if !requirements_content.is_empty() {
                let requirements_path = output_path
                    .parent()
                    .unwrap_or_else(|| Path::new("."))
                    .join("requirements.txt");

                fs::write(&requirements_path, requirements_content).with_context(|| {
                    format!("Failed to write requirements file: {:?}", requirements_path)
                })?;

                info!("Requirements written to: {:?}", requirements_path);
            } else {
                info!("No third-party dependencies found, skipping requirements.txt");
            }
        }

        Ok(())
    }

    /// Find the module name for the entry script
    fn find_entry_module_name(
        &self,
        entry_path: &Path,
        resolver: &ModuleResolver,
    ) -> Result<String> {
        // Try to find which src directory contains the entry file
        for src_dir in &self.config.src {
            if let Ok(relative_path) = entry_path.strip_prefix(src_dir) {
                if let Some(module_name) = self.path_to_module_name(relative_path) {
                    return Ok(module_name);
                }
            }
        }

        // If not found in src directories, use the file stem as module name
        let module_name = entry_path
            .file_stem()
            .and_then(|name| name.to_str())
            .ok_or_else(|| {
                anyhow!(
                    "Cannot determine module name from entry path: {:?}",
                    entry_path
                )
            })?;

        Ok(module_name.to_string())
    }

    /// Convert a relative path to a module name
    fn path_to_module_name(&self, relative_path: &Path) -> Option<String> {
        let parts: Vec<String> = relative_path
            .components()
            .map(|c| c.as_os_str().to_string_lossy().to_string())
            .collect();

        if parts.is_empty() {
            return None;
        }

        let mut module_parts = parts;
        let last_part = module_parts.last_mut()?;

        // Remove .py extension
        if last_part.ends_with(".py") {
            *last_part = last_part[..last_part.len() - 3].to_string();
        }

        // Handle __init__.py
        if last_part == "__init__" {
            module_parts.pop();
        }

        if module_parts.is_empty() {
            return None;
        }

        Some(module_parts.join("."))
    }

    /// Build the complete dependency graph starting from the entry module
    fn build_dependency_graph(
        &self,
        entry_path: &Path,
        entry_module_name: &str,
        resolver: &mut ModuleResolver,
    ) -> Result<DependencyGraph> {
        let mut graph = DependencyGraph::new();
        let mut processed_modules = HashSet::new();
        let mut modules_to_process =
            vec![(entry_module_name.to_string(), entry_path.to_path_buf())];

        // Store module data for phase 2
        let mut all_modules: Vec<(String, PathBuf, Vec<String>)> = Vec::new();

        // PHASE 1: Discover and collect all modules
        info!("Phase 1: Discovering all modules...");
        while let Some((module_name, module_path)) = modules_to_process.pop() {
            debug!("Discovering module: {} ({:?})", module_name, module_path);
            if processed_modules.contains(&module_name) {
                debug!("Module {} already discovered, skipping", module_name);
                continue;
            }

            // Parse the module and extract imports
            let imports = self.extract_imports(&module_path)?;
            debug!("Extracted imports from {}: {:?}", module_name, imports);

            // Store module data for later processing
            all_modules.push((module_name.clone(), module_path.clone(), imports.clone()));
            processed_modules.insert(module_name.clone());

            // Find and queue first-party imports for discovery
            for import in imports {
                match resolver.classify_import(&import) {
                    ImportType::FirstParty => {
                        debug!("'{}' classified as FirstParty", import);
                        if let Some(import_path) = resolver.resolve_module_path(&import)? {
                            debug!("Resolved '{}' to path: {:?}", import, import_path);

                            // Add to processing queue if not already processed
                            if !processed_modules.contains(&import) {
                                debug!("Adding '{}' to discovery queue", import);
                                modules_to_process.push((import, import_path));
                            }
                        } else {
                            warn!("Failed to resolve path for first-party module: {}", import);
                        }
                    }
                    ImportType::ThirdParty | ImportType::StandardLibrary => {
                        debug!("'{}' classified as external (preserving)", import);
                    }
                }
            }
        }

        info!("Phase 1 complete: discovered {} modules", all_modules.len());

        // PHASE 2: Add all modules to graph and create dependency edges
        info!("Phase 2: Adding modules to graph...");

        // First, add all modules to the graph
        for (module_name, module_path, imports) in &all_modules {
            let module_node = ModuleNode {
                name: module_name.clone(),
                path: module_path.clone(),
                imports: imports.clone(),
            };
            debug!("Adding module to graph: {}", module_node.name);
            graph.add_module(module_node);
        }

        info!("Added {} modules to graph", graph.get_modules().len());

        // Then, add all dependency edges
        info!("Phase 2: Creating dependency edges...");
        for (module_name, _module_path, imports) in &all_modules {
            for import in imports {
                match resolver.classify_import(import) {
                    ImportType::FirstParty => {
                        // Add dependency edge (dependency -> dependent)
                        // This means the imported module must come before the importing module
                        debug!("Adding dependency edge: {} -> {}", import, module_name);
                        if let Err(e) = graph.add_dependency(import, module_name) {
                            debug!("Failed to add dependency edge: {}", e);
                            warn!("Failed to add dependency edge: {}", e);
                        } else {
                            debug!(
                                "Successfully added dependency edge: {} -> {}",
                                import, module_name
                            );
                        }
                    }
                    ImportType::ThirdParty | ImportType::StandardLibrary => {
                        // These will be preserved in the output, not inlined
                    }
                }
            }
        }

        info!(
            "Phase 2 complete: dependency graph built with {} modules",
            graph.get_modules().len()
        );
        Ok(graph)
    }

    /// Extract import statements from a Python file using regex parsing
    fn extract_imports(&self, file_path: &Path) -> Result<Vec<String>> {
        let source = fs::read_to_string(file_path)
            .with_context(|| format!("Failed to read file: {:?}", file_path))?;

        let mut imports = Vec::new();

        // Regex patterns for different import types
        let import_re =
            Regex::new(r"^import\s+([a-zA-Z_][a-zA-Z0-9_]*(?:\.[a-zA-Z_][a-zA-Z0-9_]*)*)")
                .expect("Invalid regex");
        let from_import_re =
            Regex::new(r"^from\s+([a-zA-Z_][a-zA-Z0-9_]*(?:\.[a-zA-Z_][a-zA-Z0-9_]*)*)\s+import")
                .expect("Invalid regex");

        for line in source.lines() {
            let trimmed = line.trim();

            // Skip comments and empty lines
            if trimmed.starts_with('#') || trimmed.is_empty() {
                continue;
            }

            // Match "import module" statements
            if let Some(caps) = import_re.captures(trimmed) {
                if let Some(module) = caps.get(1) {
                    imports.push(module.as_str().to_string());
                }
            }

            // Match "from module import ..." statements
            if let Some(caps) = from_import_re.captures(trimmed) {
                if let Some(module) = caps.get(1) {
                    imports.push(module.as_str().to_string());
                }
            }
        }

        Ok(imports)
    }
}

#[cfg(feature = "python")]
use pyo3::prelude::*;

#[cfg(feature = "python")]
#[pyclass]
pub struct PyBundler {
    bundler: Bundler,
}

#[cfg(feature = "python")]
#[pymethods]
impl PyBundler {
    #[new]
    fn new() -> PyResult<Self> {
        let config = Config::default();
        Ok(PyBundler {
            bundler: Bundler::new(config),
        })
    }

    fn bundle(
        &mut self,
        entry_path: &str,
        output_path: &str,
        emit_requirements: bool,
    ) -> PyResult<()> {
        let entry = PathBuf::from(entry_path);
        let output = PathBuf::from(output_path);

        self.bundler
            .bundle(&entry, &output, emit_requirements)
            .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(e.to_string()))
    }
}
