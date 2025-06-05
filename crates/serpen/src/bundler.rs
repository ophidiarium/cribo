use anyhow::{Context, Result, anyhow};
use log::{debug, info, warn};
use ruff_python_ast::{Stmt, StmtImportFrom};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use crate::config::Config;
use crate::dependency_graph::{DependencyGraph, ModuleNode};
use crate::emit::CodeEmitter;
use crate::resolver::{ImportType, ModuleResolver};
use crate::util::module_name_from_relative;

/// Type alias for module processing queue
type ModuleQueue = Vec<(String, PathBuf)>;
/// Type alias for processed modules set
type ProcessedModules = HashSet<String>;

/// Parameters for discovery phase operations
struct DiscoveryParams<'a> {
    resolver: &'a mut ModuleResolver,
    modules_to_process: &'a mut ModuleQueue,
    processed_modules: &'a ProcessedModules,
    queued_modules: &'a mut HashSet<String>,
}

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
        debug!(
            "Using target Python version: {} (Python 3.{})",
            self.config.target_version,
            self.config.python_version().unwrap_or(10)
        );

        // Auto-detect the entry point's directory as a source directory
        if let Some(entry_dir) = entry_path.parent() {
            // Canonicalize the path to avoid duplicates due to different lexical representations
            let entry_dir = match entry_dir.canonicalize() {
                Ok(canonical_path) => canonical_path,
                Err(_) => {
                    // Fall back to the original path if canonicalization fails (e.g., path doesn't exist)
                    entry_dir.to_path_buf()
                }
            };
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

        // Enhanced circular dependency detection and analysis
        if graph.has_cycles() {
            let analysis = graph.classify_circular_dependencies();

            if !analysis.unresolvable_cycles.is_empty() {
                // Format detailed error message for unresolvable cycles
                let mut error_msg =
                    String::from("Unresolvable circular dependencies detected:\n\n");

                for (i, cycle) in analysis.unresolvable_cycles.iter().enumerate() {
                    error_msg.push_str(&format!(
                        "Cycle {}: {}\n",
                        i + 1,
                        cycle.modules.join(" → ")
                    ));
                    error_msg.push_str(&format!("  Type: {:?}\n", cycle.cycle_type));

                    if let crate::dependency_graph::ResolutionStrategy::Unresolvable { reason } =
                        &cycle.suggested_resolution
                    {
                        error_msg.push_str(&format!("  Reason: {}\n", reason));
                    }
                    error_msg.push('\n');
                }

                return Err(anyhow!(error_msg));
            }

            // Check if we can resolve the circular dependencies
            let all_resolvable = analysis.resolvable_cycles.iter().all(|cycle| {
                matches!(
                    cycle.cycle_type,
                    crate::dependency_graph::CircularDependencyType::FunctionLevel
                )
            });

            if all_resolvable && analysis.unresolvable_cycles.is_empty() {
                // All cycles are function-level and resolvable - proceed with bundling
                warn!(
                    "Detected {} resolvable circular dependencies - proceeding with bundling",
                    analysis.resolvable_cycles.len()
                );

                for cycle in &analysis.resolvable_cycles {
                    info!("Resolving cycle: {}", cycle.modules.join(" → "));
                }
            } else {
                // We have unresolvable cycles or complex resolvable cycles - still fail for now
                warn!(
                    "Detected {} circular dependencies (including {} potentially resolvable)",
                    analysis.total_cycles_detected,
                    analysis.resolvable_cycles.len()
                );

                let mut error_msg =
                    String::from("Circular dependencies detected in the module graph:\n\n");

                // First, show unresolvable cycles if any
                if !analysis.unresolvable_cycles.is_empty() {
                    error_msg.push_str("UNRESOLVABLE CYCLES:\n");
                    for (i, cycle) in analysis.unresolvable_cycles.iter().enumerate() {
                        error_msg.push_str(&format!(
                            "Cycle {}: {}\n",
                            i + 1,
                            cycle.modules.join(" → ")
                        ));
                        error_msg.push_str(&format!("  Type: {:?}\n", cycle.cycle_type));
                        if let crate::dependency_graph::ResolutionStrategy::Unresolvable {
                            reason,
                        } = &cycle.suggested_resolution
                        {
                            error_msg.push_str(&format!("  Reason: {}\n", reason));
                        }
                        error_msg.push('\n');
                    }
                }

                // Then show resolvable cycles that are not yet supported
                if !analysis.resolvable_cycles.is_empty() {
                    if !analysis.unresolvable_cycles.is_empty() {
                        error_msg.push_str("RESOLVABLE CYCLES (not yet implemented):\n");
                    }
                    for (i, cycle) in analysis.resolvable_cycles.iter().enumerate() {
                        let cycle_num = i + 1 + analysis.unresolvable_cycles.len();
                        error_msg.push_str(&format!(
                            "Cycle {}: {}\n",
                            cycle_num,
                            cycle.modules.join(" → ")
                        ));
                        error_msg.push_str(&format!("  Type: {:?}\n", cycle.cycle_type));

                        match &cycle.suggested_resolution {
                            crate::dependency_graph::ResolutionStrategy::LazyImport {
                                modules: _,
                            } => {
                                error_msg.push_str("  Suggestion: Move imports inside functions to enable lazy loading\n");
                            }
                            crate::dependency_graph::ResolutionStrategy::FunctionScopedImport {
                                import_statements,
                            } => {
                                error_msg.push_str("  Suggestions:\n");
                                for suggestion in import_statements {
                                    error_msg.push_str(&format!("    {}\n", suggestion));
                                }
                            }
                            crate::dependency_graph::ResolutionStrategy::ModuleSplit {
                                suggestions,
                            } => {
                                error_msg.push_str("  Suggestions:\n");
                                for suggestion in suggestions {
                                    error_msg.push_str(&format!("    {}\n", suggestion));
                                }
                            }
                            crate::dependency_graph::ResolutionStrategy::Unresolvable {
                                ..
                            } => {
                                // This shouldn't happen in resolvable cycles
                            }
                        }
                        error_msg.push('\n');
                    }
                }

                return Err(anyhow!(error_msg));
            }
        }

        // Get topologically sorted modules
        // If we have cycles but they're all resolvable, we need a different approach
        let sorted_modules = if graph.has_cycles() {
            let analysis = graph.classify_circular_dependencies();
            let all_resolvable = analysis.resolvable_cycles.iter().all(|cycle| {
                matches!(
                    cycle.cycle_type,
                    crate::dependency_graph::CircularDependencyType::FunctionLevel
                )
            }) && analysis.unresolvable_cycles.is_empty();

            if all_resolvable {
                // For resolvable cycles, use a custom ordering that breaks cycles
                self.get_modules_with_cycle_resolution(&graph, &analysis)?
            } else {
                // This should have been caught earlier, but be safe
                return Err(anyhow!("Unresolvable circular dependencies detected"));
            }
        } else {
            graph.topological_sort()?
        };
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
            self.write_requirements_file(&sorted_modules, &mut emitter, output_path)?;
        }

        Ok(())
    }

    /// Get modules in a valid order for bundling when there are resolvable circular dependencies
    fn get_modules_with_cycle_resolution<'a>(
        &self,
        graph: &'a DependencyGraph,
        analysis: &crate::dependency_graph::CircularDependencyAnalysis,
    ) -> Result<Vec<&'a crate::dependency_graph::ModuleNode>> {
        // For simple function-level cycles, we can use a modified topological sort
        // that breaks cycles by removing edges within strongly connected components

        // Get all modules
        let modules = graph.get_modules();

        // For now, use a simple approach: put all modules from cycles at the end
        // and sort the rest topologically
        let mut cycle_modules = std::collections::HashSet::new();
        for cycle in &analysis.resolvable_cycles {
            for module_name in &cycle.modules {
                cycle_modules.insert(module_name.as_str());
            }
        }

        // Split modules into non-cycle and cycle modules
        let (cycle_mods, non_cycle_mods): (Vec<_>, Vec<_>) = modules
            .into_iter()
            .partition(|module| cycle_modules.contains(module.name.as_str()));

        // For non-cycle modules, we can still use topological sorting on the subgraph
        let mut result = Vec::new();

        // Add non-cycle modules first (they should sort topologically)
        result.extend(non_cycle_mods);

        // Add cycle modules at the end - the order doesn't matter much for function-level cycles
        result.extend(cycle_mods);

        Ok(result)
    }

    /// Helper method to find module name in source directories
    fn find_module_in_src_dirs(&self, entry_path: &Path) -> Option<String> {
        for src_dir in &self.config.src {
            let relative_path = entry_path.strip_prefix(src_dir).ok()?;
            if let Some(module_name) = self.path_to_module_name(relative_path) {
                return Some(module_name);
            }
        }
        None
    }

    /// Find the module name for the entry script
    fn find_entry_module_name(
        &self,
        entry_path: &Path,
        _resolver: &ModuleResolver,
    ) -> Result<String> {
        // Try to find which src directory contains the entry file
        if let Some(module_name) = self.find_module_in_src_dirs(entry_path) {
            return Ok(module_name);
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
        module_name_from_relative(relative_path)
    }

    /// Build the complete dependency graph starting from the entry module
    fn build_dependency_graph(
        &self,
        entry_path: &Path,
        entry_module_name: &str,
        resolver: &mut ModuleResolver,
    ) -> Result<DependencyGraph> {
        let mut graph = DependencyGraph::new();
        let mut processed_modules = ProcessedModules::new();
        let mut queued_modules = HashSet::new();
        let mut modules_to_process = ModuleQueue::new();
        modules_to_process.push((entry_module_name.to_string(), entry_path.to_path_buf()));
        queued_modules.insert(entry_module_name.to_string());

        // Store module data for phase 2
        type ModuleData = (String, PathBuf, Vec<String>);
        let mut all_modules: Vec<ModuleData> = Vec::new();

        // PHASE 1: Discover and collect all modules
        info!("Phase 1: Discovering all modules...");
        while let Some((module_name, module_path)) = modules_to_process.pop() {
            debug!("Discovering module: {} ({:?})", module_name, module_path);
            if processed_modules.contains(&module_name) {
                debug!("Module {} already discovered, skipping", module_name);
                continue;
            }

            // Parse the module and extract imports (including module imports)
            let imports = self.extract_imports(&module_path, Some(resolver))?;
            debug!("Extracted imports from {}: {:?}", module_name, imports);

            // Store module data for later processing
            all_modules.push((module_name.clone(), module_path.clone(), imports.clone()));
            processed_modules.insert(module_name.clone());

            // Find and queue first-party imports for discovery
            for import in imports {
                let mut params = DiscoveryParams {
                    resolver,
                    modules_to_process: &mut modules_to_process,
                    processed_modules: &processed_modules,
                    queued_modules: &mut queued_modules,
                };
                self.process_import_for_discovery(&import, &mut params);
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
                self.process_import_for_dependency(import, module_name, (resolver, &mut graph));
            }
        }

        info!(
            "Phase 2 complete: dependency graph built with {} modules",
            graph.get_modules().len()
        );
        Ok(graph)
    }

    /// Extract import statements from a Python file using AST parsing
    /// This handles all import variations including multi-line, aliased, relative, and parenthesized imports
    pub fn extract_imports(
        &self,
        file_path: &Path,
        mut resolver: Option<&mut ModuleResolver>,
    ) -> Result<Vec<String>> {
        let source = fs::read_to_string(file_path)
            .with_context(|| format!("Failed to read file: {:?}", file_path))?;

        let parsed = ruff_python_parser::parse_module(&source)
            .with_context(|| format!("Failed to parse Python file: {:?}", file_path))?;

        let mut imports = Vec::new();

        for stmt in parsed.syntax().body.iter() {
            self.extract_imports_from_statement(
                stmt,
                &mut imports,
                file_path,
                resolver.as_deref_mut(),
            );
        }

        Ok(imports)
    }

    /// Extract import module names from a single AST statement
    fn extract_imports_from_statement(
        &self,
        stmt: &Stmt,
        imports: &mut Vec<String>,
        file_path: &Path,
        resolver: Option<&mut ModuleResolver>,
    ) {
        if let Stmt::Import(import_stmt) = stmt {
            for alias in &import_stmt.names {
                // For dotted imports like "xml.etree.ElementTree", we need to extract
                // the root module name "xml" for classification purposes, but we should
                // preserve the full import path for the output
                let module_name = alias.name.id.to_string();
                imports.push(module_name);
            }
        } else if let Stmt::ImportFrom(import_from_stmt) = stmt {
            self.process_import_from_statement(import_from_stmt, imports, file_path, resolver);
        }
    }

    /// Process a "from ... import ..." statement to extract module names
    fn process_import_from_statement(
        &self,
        import_from_stmt: &StmtImportFrom,
        imports: &mut Vec<String>,
        file_path: &Path,
        mut resolver: Option<&mut ModuleResolver>,
    ) {
        let level = import_from_stmt.level;

        if level == 0 {
            if let Some(ref mut resolver) = resolver {
                self.process_absolute_import_with_resolver(import_from_stmt, imports, resolver);
            } else {
                self.process_absolute_import(import_from_stmt, imports);
            }
            return;
        }

        // Handle relative imports
        // TODO: Consider extending resolver support to relative imports as well
        if let Some(base_module) = self.resolve_relative_import(file_path, level) {
            self.process_resolved_relative_import(import_from_stmt, imports, &base_module);
        } else {
            self.process_fallback_relative_import(import_from_stmt, imports, level);
        }
    }

    /// Process absolute imports (level == 0)
    fn process_absolute_import(
        &self,
        import_from_stmt: &StmtImportFrom,
        imports: &mut Vec<String>,
    ) {
        if let Some(ref module) = import_from_stmt.module {
            let m = module.id.to_string();
            // Avoid duplicate absolute imports (e.g., import importlib + from importlib import)
            if !imports.contains(&m) {
                imports.push(m);
            }
        }
    }

    /// Enhanced version that can detect module imports using a resolver
    fn process_absolute_import_with_resolver(
        &self,
        import_from_stmt: &StmtImportFrom,
        imports: &mut Vec<String>,
        resolver: &mut ModuleResolver,
    ) {
        if let Some(ref module) = import_from_stmt.module {
            let m = module.id.to_string();
            // Add the package/module being imported from
            if !imports.contains(&m) {
                imports.push(m.clone());
            }

            // Check if any of the imported names are actually modules
            for alias in &import_from_stmt.names {
                let imported_name = alias.name.id.to_string();
                let full_module_name = format!("{}.{}", m, imported_name);

                // Try to resolve the full module name to see if it's a module
                if let Ok(Some(_)) = resolver.resolve_module_path(&full_module_name) {
                    // This is a module import (e.g., from greetings import greeting)
                    if !imports.contains(&full_module_name) {
                        imports.push(full_module_name);
                        debug!("Detected module import: {} from {}", imported_name, m);
                    }
                }
            }
        }
    }

    /// Process relative imports that were successfully resolved
    fn process_resolved_relative_import(
        &self,
        import_from_stmt: &StmtImportFrom,
        imports: &mut Vec<String>,
        base_module: &str,
    ) {
        if let Some(ref module) = import_from_stmt.module {
            // Relative import with explicit module name: from .module import something
            let full_module = self.build_full_module_name(base_module, &module.id);
            imports.push(full_module);
        } else {
            // Relative import without explicit module: from . import something
            // Add each imported name as a full module name
            for alias in &import_from_stmt.names {
                let full_import = self.build_full_module_name(base_module, &alias.name.id);
                imports.push(full_import);
            }
        }
    }

    /// Build a full module name by combining base module and target module
    fn build_full_module_name(&self, base_module: &str, target_module: &str) -> String {
        if base_module.is_empty() {
            target_module.to_string()
        } else {
            format!("{}.{}", base_module, target_module)
        }
    }

    /// Process relative imports when resolution fails (fallback behavior)
    fn process_fallback_relative_import(
        &self,
        import_from_stmt: &StmtImportFrom,
        imports: &mut Vec<String>,
        level: u32,
    ) {
        if let Some(ref module) = import_from_stmt.module {
            let module_name = self.format_module_name(&module.id, level);
            imports.push(module_name);
        } else {
            let dots = ".".repeat(level as usize);
            imports.push(dots);
        }
    }

    /// Format module name based on relative import level
    /// Assumes module is always non-empty when called
    fn format_module_name(&self, module: &str, level: u32) -> String {
        if level > 0 {
            let dots = ".".repeat(level as usize);
            format!("{}{}", dots, module)
        } else {
            module.to_string()
        }
    }

    /// Resolve a relative import to its absolute module name
    /// Returns None if the relative import goes beyond the module hierarchy
    fn resolve_relative_import(&self, file_path: &Path, level: u32) -> Option<String> {
        // Get the directory containing the current file
        let current_dir = file_path.parent()?;

        // Convert current_dir to absolute path if it's relative
        let absolute_current_dir = if current_dir.is_absolute() {
            current_dir.to_path_buf()
        } else {
            std::env::current_dir().ok()?.join(current_dir)
        };

        // Find which source directory contains this file
        let relative_dir = self.config.src.iter().find_map(|src_dir| {
            // Handle case where paths might be relative vs absolute
            if src_dir.is_absolute() {
                absolute_current_dir.strip_prefix(src_dir).ok()
            } else {
                // For relative source directories, we need to resolve them relative to the current working directory
                let current_working_dir = std::env::current_dir().ok()?;
                let absolute_src_dir = current_working_dir.join(src_dir);
                absolute_current_dir.strip_prefix(&absolute_src_dir).ok()
            }
        })?;

        // Convert directory path to module path components
        let module_parts: Vec<String> = if relative_dir == Path::new("") {
            // If relative_dir is empty, we're at the root of the source directory
            Vec::new()
        } else {
            relative_dir
                .components()
                .map(|c| c.as_os_str().to_string_lossy().to_string())
                .collect()
        };

        // Apply relative import logic
        let mut current_parts = module_parts;

        // Remove 'level' number of components from the end
        // For level=1 (.), we stay in current package
        // For level=2 (..), we go to parent package, etc.
        if level as usize > current_parts.len() + 1 {
            // Cannot go beyond the root of the project
            return None;
        }

        // Remove (level - 1) components since level=1 means current package
        for _ in 0..(level.saturating_sub(1)) {
            current_parts.pop();
        }

        if current_parts.is_empty() {
            // If we're at the root after applying relative levels, return empty string
            // This will be handled by the caller to construct the full import name
            Some(String::new())
        } else {
            Some(current_parts.join("."))
        }
    }

    /// Helper method to add module to discovery queue if not already processed or queued
    fn add_to_discovery_queue_if_new(
        &self,
        import: &str,
        import_path: PathBuf,
        discovery_params: &mut DiscoveryParams,
    ) {
        if !discovery_params.processed_modules.contains(import)
            && !discovery_params.queued_modules.contains(import)
        {
            debug!("Adding '{}' to discovery queue", import);
            discovery_params
                .modules_to_process
                .push((import.to_string(), import_path));
            discovery_params.queued_modules.insert(import.to_string());
        } else {
            debug!("Module '{}' already processed or queued, skipping", import);
        }
    }

    /// Process an import during discovery phase
    fn process_import_for_discovery(&self, import: &str, params: &mut DiscoveryParams) {
        match params.resolver.classify_import(import) {
            ImportType::FirstParty => {
                debug!("'{}' classified as FirstParty", import);
                if let Ok(Some(import_path)) = params.resolver.resolve_module_path(import) {
                    debug!("Resolved '{}' to path: {:?}", import, import_path);
                    self.add_to_discovery_queue_if_new(import, import_path, params);
                } else {
                    warn!("Failed to resolve path for first-party module: {}", import);
                }
            }
            ImportType::ThirdParty | ImportType::StandardLibrary => {
                debug!("'{}' classified as external (preserving)", import);
            }
        }
    }

    /// Process an import during dependency graph creation phase
    fn process_import_for_dependency(
        &self,
        import: &str,
        module_name: &str,
        resolver_and_graph: (&ModuleResolver, &mut DependencyGraph),
    ) {
        let (resolver, graph) = resolver_and_graph;
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

    /// Write requirements.txt file if there are dependencies
    fn write_requirements_file(
        &self,
        sorted_modules: &[&ModuleNode],
        emitter: &mut CodeEmitter,
        output_path: &Path,
    ) -> Result<()> {
        let requirements_content = emitter.generate_requirements(sorted_modules)?;
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
        Ok(())
    }
}
