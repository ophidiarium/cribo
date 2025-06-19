use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, anyhow};
use indexmap::IndexSet;
use log::{debug, info, warn};
use ruff_python_ast::{ModModule, Stmt, StmtImportFrom, visitor::Visitor};

use crate::{
    code_generator::HybridStaticBundler,
    config::Config,
    cribo_graph::{
        CircularDependencyAnalysis, CircularDependencyGroup, CircularDependencyType, CriboGraph,
        ResolutionStrategy,
    },
    import_rewriter::{ImportDeduplicationStrategy, ImportRewriter},
    resolver::{ImportType, ModuleResolver},
    semantic_bundler::SemanticBundler,
    util::{module_name_from_relative, normalize_line_endings},
    visitors::{ImportDiscoveryVisitor, ImportLocation},
};

/// Type alias for module processing queue
type ModuleQueue = Vec<(String, PathBuf)>;
/// Type alias for processed modules set
type ProcessedModules = IndexSet<String>;
/// Type alias for parsed module data with AST and source
type ParsedModuleData = (String, PathBuf, Vec<String>, ModModule, String);

/// Context for import extraction operations
struct ImportExtractionContext<'a> {
    imports: &'a mut Vec<String>,
    file_path: &'a Path,
    resolver: Option<&'a mut ModuleResolver>,
}

/// Context for module import checking operations
struct ModuleImportContext<'a> {
    imports: &'a mut Vec<String>,
    base_module: &'a str,
    resolver: &'a mut ModuleResolver,
}

/// Parameters for discovery phase operations
struct DiscoveryParams<'a> {
    resolver: &'a mut ModuleResolver,
    modules_to_process: &'a mut ModuleQueue,
    processed_modules: &'a ProcessedModules,
    queued_modules: &'a mut IndexSet<String>,
}

/// Parameters for static bundle emission
struct StaticBundleParams<'a> {
    sorted_modules: &'a [(String, PathBuf, Vec<String>)],
    parsed_modules: Option<&'a [ParsedModuleData]>, // Optional pre-parsed modules
    _resolver: &'a ModuleResolver,                  // Unused but kept for future use
    entry_module_name: &'a str,
    graph: &'a CriboGraph,
    circular_dep_analysis: Option<&'a CircularDependencyAnalysis>,
}

/// Context for dependency building operations
struct DependencyContext<'a> {
    resolver: &'a ModuleResolver,
    graph: &'a mut CriboGraph,
    module_id_map: &'a indexmap::IndexMap<String, crate::cribo_graph::ModuleId>,
    current_module: &'a str,
    from_module_id: crate::cribo_graph::ModuleId,
}

/// Parameters for graph building operations
struct GraphBuildParams<'a> {
    entry_path: &'a Path,
    entry_module_name: &'a str,
    resolver: &'a mut ModuleResolver,
    graph: &'a mut CriboGraph,
}

pub struct BundleOrchestrator {
    config: Config,
    semantic_bundler: SemanticBundler,
}

impl BundleOrchestrator {
    pub fn new(config: Config) -> Self {
        Self {
            config,
            semantic_bundler: SemanticBundler::new(),
        }
    }

    /// Format error message for unresolvable cycles
    fn format_unresolvable_cycles_error(cycles: &[CircularDependencyGroup]) -> String {
        let mut error_msg = String::from("Unresolvable circular dependencies detected:\n\n");

        for (i, cycle) in cycles.iter().enumerate() {
            error_msg.push_str(&format!("Cycle {}: {}\n", i + 1, cycle.modules.join(" → ")));
            error_msg.push_str(&format!("  Type: {:?}\n", cycle.cycle_type));

            if let ResolutionStrategy::Unresolvable { reason } = &cycle.suggested_resolution {
                error_msg.push_str(&format!("  Reason: {reason}\n"));
            }
            error_msg.push('\n');
        }

        error_msg
    }

    /// Core bundling logic shared between file and string output modes
    /// Returns the entry module name, parsed modules, and circular dependency analysis, with graph
    /// and resolver populated via mutable references
    fn bundle_core(
        &mut self,
        entry_path: &Path,
        graph: &mut CriboGraph,
        resolver_opt: &mut Option<ModuleResolver>,
    ) -> Result<(
        String,
        Vec<ParsedModuleData>,
        Option<CircularDependencyAnalysis>,
    )> {
        debug!("Entry: {entry_path:?}");
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
                    // Fall back to the original path if canonicalization fails (e.g., path doesn't
                    // exist)
                    entry_dir.to_path_buf()
                }
            };
            if !self.config.src.contains(&entry_dir) {
                debug!("Adding entry directory to src paths: {entry_dir:?}");
                self.config.src.insert(0, entry_dir);
            }
        }

        // Initialize resolver with the updated config
        let mut resolver = ModuleResolver::new(self.config.clone())?;

        // Set the entry file to establish the primary search path
        resolver.set_entry_file(entry_path);

        // Find the entry module name
        let entry_module_name = self.find_entry_module_name(entry_path, &resolver)?;
        info!("Entry module: {entry_module_name}");

        // Build dependency graph
        let mut build_params = GraphBuildParams {
            entry_path,
            entry_module_name: &entry_module_name,
            resolver: &mut resolver,
            graph,
        };
        let parsed_modules = self.build_dependency_graph(&mut build_params)?;

        // In CriboGraph, we track all modules but focus on reachable ones
        debug!("Graph has {} modules", graph.modules.len());

        // Enhanced circular dependency detection and analysis
        let mut circular_dep_analysis = None;
        if graph.has_cycles() {
            let analysis = graph.analyze_circular_dependencies();

            if !analysis.unresolvable_cycles.is_empty() {
                let error_msg =
                    Self::format_unresolvable_cycles_error(&analysis.unresolvable_cycles);
                return Err(anyhow!(error_msg));
            }

            // Check if we can resolve the circular dependencies
            let all_resolvable = analysis
                .resolvable_cycles
                .iter()
                .all(|cycle| matches!(cycle.cycle_type, CircularDependencyType::FunctionLevel));

            if all_resolvable && analysis.unresolvable_cycles.is_empty() {
                // All cycles are function-level and resolvable - proceed with bundling
                warn!(
                    "Detected {} resolvable circular dependencies - proceeding with bundling",
                    analysis.resolvable_cycles.len()
                );

                Self::log_resolvable_cycles(&analysis.resolvable_cycles);
                circular_dep_analysis = Some(analysis);
            } else {
                // We have unresolvable cycles or complex resolvable cycles - still fail for now
                warn!(
                    "Detected {} circular dependencies (including {} potentially resolvable)",
                    analysis.total_cycles_detected,
                    analysis.resolvable_cycles.len()
                );

                let error_msg = Self::build_cycle_error_message(&analysis);
                return Err(anyhow!(error_msg));
            }
        }

        // Set the resolver for the caller to use
        *resolver_opt = Some(resolver);

        Ok((entry_module_name, parsed_modules, circular_dep_analysis))
    }

    /// Helper to get sorted modules from graph
    fn get_sorted_modules_from_graph(
        &self,
        graph: &CriboGraph,
        circular_dep_analysis: Option<&CircularDependencyAnalysis>,
    ) -> Result<Vec<(String, PathBuf, Vec<String>)>> {
        let module_ids = if let Some(analysis) = circular_dep_analysis {
            // We already have the analysis from bundle_core
            let all_resolvable = analysis
                .resolvable_cycles
                .iter()
                .all(|cycle| matches!(cycle.cycle_type, CircularDependencyType::FunctionLevel))
                && analysis.unresolvable_cycles.is_empty();

            if all_resolvable {
                // For resolvable cycles, use a custom ordering that breaks cycles
                self.get_modules_with_cycle_resolution(graph, analysis)?
            } else {
                // This should have been caught earlier, but be safe
                return Err(anyhow!("Unresolvable circular dependencies detected"));
            }
        } else {
            graph.topological_sort()?
        };
        // Convert module IDs to module data tuples
        let mut sorted_modules = Vec::new();
        for module_id in module_ids {
            if let Some(module) = graph.modules.get(&module_id) {
                let name = module.module_name.clone();
                let path = graph
                    .module_paths
                    .iter()
                    .find(|(_, id)| **id == module_id)
                    .map(|(p, _)| p.clone())
                    .unwrap_or_else(|| {
                        warn!("Module path not found for {name}, using name as fallback");
                        PathBuf::from(&name)
                    });

                // Extract imports from module items
                let imports = self.extract_imports_from_module_items(&module.items);

                debug!("Module '{name}' has imports: {imports:?}");

                sorted_modules.push((name, path, imports));
            }
        }

        info!("Found {} modules to bundle", sorted_modules.len());
        debug!("=== DEPENDENCY GRAPH DEBUG ===");
        for (module_id, module) in &graph.modules {
            let deps = graph.get_dependencies(*module_id);
            if !deps.is_empty() {
                let dep_names: Vec<String> = deps
                    .iter()
                    .filter_map(|dep_id| graph.modules.get(dep_id).map(|m| m.module_name.clone()))
                    .collect();
                debug!(
                    "Module '{}' depends on: {:?}",
                    module.module_name, dep_names
                );
            }
        }
        debug!("=== TOPOLOGICAL SORT ORDER ===");
        for (i, (name, path, _)) in sorted_modules.iter().enumerate() {
            debug!("Module {i}: {name} ({path:?})");
        }
        debug!("=== END DEBUG ===");
        Ok(sorted_modules)
    }

    /// Bundle to string for stdout output
    pub fn bundle_to_string(
        &mut self,
        entry_path: &Path,
        emit_requirements: bool,
    ) -> Result<String> {
        info!("Starting bundle process for stdout output");

        // Initialize empty graph - resolver will be created in bundle_core
        let mut graph = CriboGraph::new();
        let mut resolver_opt = None;

        // Perform core bundling logic
        let (entry_module_name, parsed_modules, circular_dep_analysis) =
            self.bundle_core(entry_path, &mut graph, &mut resolver_opt)?;

        // Extract the resolver (it's guaranteed to be Some after bundle_core)
        let resolver = resolver_opt.expect("Resolver should be initialized by bundle_core");

        let sorted_modules =
            self.get_sorted_modules_from_graph(&graph, circular_dep_analysis.as_ref())?;

        // Extract module data from sorted_modules
        let module_data = sorted_modules
            .iter()
            .map(|(name, path, imports)| (name.clone(), path.clone(), imports.clone()))
            .collect::<Vec<_>>();

        // Generate bundled code
        info!("Using hybrid static bundler");
        let bundled_code = self.emit_static_bundle(StaticBundleParams {
            sorted_modules: &module_data,
            parsed_modules: Some(&parsed_modules),
            _resolver: &resolver,
            entry_module_name: &entry_module_name,
            graph: &graph,
            circular_dep_analysis: circular_dep_analysis.as_ref(),
        })?;

        // Generate requirements.txt if requested
        if emit_requirements {
            self.write_requirements_file_for_stdout(&module_data, &resolver)?;
        }

        Ok(bundled_code)
    }

    /// Main bundling function
    pub fn bundle(
        &mut self,
        entry_path: &Path,
        output_path: &Path,
        emit_requirements: bool,
    ) -> Result<()> {
        info!("Starting bundle process");
        debug!("Output: {output_path:?}");

        // Initialize empty graph - resolver will be created in bundle_core
        let mut graph = CriboGraph::new();
        let mut resolver_opt = None;

        // Perform core bundling logic
        let (entry_module_name, parsed_modules, circular_dep_analysis) =
            self.bundle_core(entry_path, &mut graph, &mut resolver_opt)?;

        // Extract the resolver (it's guaranteed to be Some after bundle_core)
        let resolver = resolver_opt.expect("Resolver should be initialized by bundle_core");

        let sorted_modules =
            self.get_sorted_modules_from_graph(&graph, circular_dep_analysis.as_ref())?;

        // Generate bundled code
        info!("Using hybrid static bundler");
        let bundled_code = self.emit_static_bundle(StaticBundleParams {
            sorted_modules: &sorted_modules,
            parsed_modules: Some(&parsed_modules), // Use pre-parsed modules to avoid double parsing
            _resolver: &resolver,
            entry_module_name: &entry_module_name,
            graph: &graph,
            circular_dep_analysis: circular_dep_analysis.as_ref(),
        })?;

        // Generate requirements.txt if requested
        if emit_requirements {
            self.write_requirements_file(&sorted_modules, &resolver, output_path)?;
        }

        // Write output file
        fs::write(output_path, bundled_code)
            .with_context(|| format!("Failed to write output file: {output_path:?}"))?;

        info!("Bundle written to: {output_path:?}");

        Ok(())
    }

    /// Get modules in a valid order for bundling when there are resolvable circular dependencies
    fn get_modules_with_cycle_resolution(
        &self,
        graph: &CriboGraph,
        analysis: &crate::cribo_graph::CircularDependencyAnalysis,
    ) -> Result<Vec<crate::cribo_graph::ModuleId>> {
        // For simple function-level cycles, we can use a modified topological sort
        // that breaks cycles by removing edges within strongly connected components

        // Get all module IDs
        let all_module_ids: Vec<_> = graph.modules.keys().cloned().collect();

        // Collect all modules that are part of circular dependencies
        let mut cycle_module_names = IndexSet::new();
        for cycle in &analysis.resolvable_cycles {
            for module_name in &cycle.modules {
                cycle_module_names.insert(module_name.as_str());
            }
        }

        // Split modules into non-cycle and cycle modules
        let (mut cycle_ids, non_cycle_ids): (Vec<_>, Vec<_>) =
            all_module_ids.into_iter().partition(|&module_id| {
                if let Some(module) = graph.modules.get(&module_id) {
                    cycle_module_names.contains(module.module_name.as_str())
                } else {
                    false
                }
            });

        // For non-cycle modules, we can still use topological sorting on the subgraph
        let mut result = Vec::new();

        // Add non-cycle modules first (they should sort topologically)
        result.extend(non_cycle_ids);

        // For cycle modules, try to maintain dependency order where possible
        // Sort cycle modules by name to get deterministic output
        cycle_ids.sort_by(|&a_id, &b_id| {
            let a_name = &graph.modules[&a_id].module_name;
            let b_name = &graph.modules[&b_id].module_name;

            // For package hierarchies like mypackage.utils vs mypackage,
            // put the deeper/more specific modules first (dependencies before dependents)
            let a_depth = a_name.matches('.').count();
            let b_depth = b_name.matches('.').count();

            // If one is a submodule of the other, put the submodule first
            if a_name.starts_with(&format!("{b_name}.")) {
                std::cmp::Ordering::Less // a (submodule) before b (parent)
            } else if b_name.starts_with(&format!("{a_name}.")) {
                std::cmp::Ordering::Greater // b (submodule) before a (parent)
            } else {
                // Otherwise sort by depth (deeper modules first), then by name
                match a_depth.cmp(&b_depth) {
                    std::cmp::Ordering::Equal => a_name.cmp(b_name),
                    other => other.reverse(), // Deeper modules first
                }
            }
        });

        result.extend(cycle_ids);

        Ok(result)
    }

    /// Extract imports from module items
    fn extract_imports_from_module_items(
        &self,
        items: &rustc_hash::FxHashMap<crate::cribo_graph::ItemId, crate::cribo_graph::ItemData>,
    ) -> Vec<String> {
        let mut imports = Vec::new();
        for item_data in items.values() {
            match &item_data.item_type {
                crate::cribo_graph::ItemType::Import { module, alias: _ } => {
                    imports.push(module.clone());
                }
                crate::cribo_graph::ItemType::FromImport { module, .. } => {
                    imports.push(module.clone());
                }
                _ => {}
            }
        }
        imports
    }

    /// Helper method to find module name in source directories
    fn find_module_in_src_dirs(&self, entry_path: &Path) -> Option<String> {
        for src_dir in &self.config.src {
            let relative_path = match entry_path.strip_prefix(src_dir) {
                Ok(path) => path,
                Err(_) => continue,
            };
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

        Ok(module_name.to_owned())
    }

    /// Convert a relative path to a module name
    fn path_to_module_name(&self, relative_path: &Path) -> Option<String> {
        module_name_from_relative(relative_path)
    }

    /// Build the complete dependency graph starting from the entry module
    /// Returns the parsed modules to avoid re-parsing
    fn build_dependency_graph(
        &mut self,
        params: &mut GraphBuildParams<'_>,
    ) -> Result<Vec<ParsedModuleData>> {
        let mut processed_modules = ProcessedModules::new();
        let mut queued_modules = IndexSet::new();
        let mut modules_to_process = ModuleQueue::new();
        modules_to_process.push((
            params.entry_module_name.to_owned(),
            params.entry_path.to_path_buf(),
        ));
        queued_modules.insert(params.entry_module_name.to_owned());

        // Store module data for phase 2
        type DiscoveryData = (String, PathBuf, Vec<String>); // (name, path, imports) for discovery phase
        let mut discovered_modules: Vec<DiscoveryData> = Vec::new();

        // PHASE 1: Discover and collect all modules
        info!("Phase 1: Discovering all modules...");
        while let Some((module_name, module_path)) = modules_to_process.pop() {
            debug!("Discovering module: {module_name} ({module_path:?})");
            if processed_modules.contains(&module_name) {
                debug!("Module {module_name} already discovered, skipping");
                continue;
            }

            // Check if this is a namespace package (directory without __init__.py)
            if module_path.is_dir() {
                debug!(
                    "Module {module_name} is a namespace package (directory), marking as processed"
                );
                processed_modules.insert(module_name.clone());
                // Don't add namespace packages to discovered_modules since they have no code
                continue;
            }

            // Parse the module and extract imports (including module imports)
            let imports = self.extract_all_imports(&module_path, Some(params.resolver))?;
            debug!("Extracted imports from {module_name}: {imports:?}");

            // Store module data for later processing
            discovered_modules.push((module_name.clone(), module_path.clone(), imports.clone()));
            processed_modules.insert(module_name.clone());

            // Find and queue first-party imports for discovery
            for import in imports {
                let mut discovery_params = DiscoveryParams {
                    resolver: params.resolver,
                    modules_to_process: &mut modules_to_process,
                    processed_modules: &processed_modules,
                    queued_modules: &mut queued_modules,
                };
                self.process_import_for_discovery(&import, &mut discovery_params);
            }
        }

        info!(
            "Phase 1 complete: discovered {} modules",
            discovered_modules.len()
        );

        // PHASE 2: Add all modules to graph and create dependency edges
        info!("Phase 2: Adding modules to graph...");

        // First, add all modules to the graph and parse them
        let mut module_id_map = indexmap::IndexMap::new();
        let mut parsed_modules: Vec<ParsedModuleData> = Vec::new();

        for (module_name, module_path, imports) in discovered_modules.iter() {
            let module_id = params
                .graph
                .add_module(module_name.clone(), module_path.clone());
            module_id_map.insert(module_name.clone(), module_id);
            debug!("Added module to graph: {module_name} with ID {module_id:?}");

            // Parse the module AST and build detailed graph
            let source = fs::read_to_string(module_path)
                .with_context(|| format!("Failed to read file: {module_path:?}"))?;
            let source = crate::util::normalize_line_endings(source);
            let parsed = ruff_python_parser::parse_module(&source)
                .with_context(|| format!("Failed to parse Python file: {module_path:?}"))?;

            let ast = parsed.into_syntax();

            // Perform semantic analysis on this module
            self.semantic_bundler
                .analyze_module(module_id, &ast, &source, module_path)?;

            // Build dependency graph BEFORE no-ops removal
            if let Some(module) = params.graph.get_module_by_name_mut(module_name) {
                let mut builder = crate::graph_builder::GraphBuilder::new(module);
                builder.build_from_ast(&ast)?;
            }

            // Store parsed module data for later use
            parsed_modules.push((
                module_name.clone(),
                module_path.clone(),
                imports.clone(),
                ast,
                source,
            ));
        }

        info!("Added {} modules to graph", params.graph.modules.len());

        // Then, add all dependency edges
        info!("Phase 2: Creating dependency edges...");
        for (module_name, _module_path, imports, _ast, _source) in &parsed_modules {
            let from_id = module_id_map.get(module_name).cloned();
            for import in imports {
                if let Some(from_module_id) = from_id {
                    let mut context = DependencyContext {
                        resolver: params.resolver,
                        graph: params.graph,
                        module_id_map: &module_id_map,
                        current_module: module_name,
                        from_module_id,
                    };
                    self.process_import_for_dependency(import, &mut context);
                }
            }
        }

        info!(
            "Phase 2 complete: dependency graph built with {} modules",
            params.graph.modules.len()
        );
        Ok(parsed_modules)
    }

    /// Extract import statements from a Python file using AST parsing
    /// This handles all import variations including multi-line, aliased, relative, and
    /// parenthesized imports
    pub fn extract_imports(
        &self,
        file_path: &Path,
        resolver: Option<&mut ModuleResolver>,
    ) -> Result<Vec<String>> {
        let source = fs::read_to_string(file_path)
            .with_context(|| format!("Failed to read file: {file_path:?}"))?;
        let source = normalize_line_endings(source);

        let parsed = ruff_python_parser::parse_module(&source)
            .with_context(|| format!("Failed to parse Python file: {file_path:?}"))?;

        let mut imports = Vec::new();

        let mut context = ImportExtractionContext {
            imports: &mut imports,
            file_path,
            resolver,
        };

        for stmt in parsed.syntax().body.iter() {
            self.extract_imports_from_statement(stmt, &mut context);
        }

        Ok(imports)
    }

    /// Extract ALL imports from a Python file, including those nested in functions and classes
    pub fn extract_all_imports(
        &self,
        file_path: &Path,
        mut resolver: Option<&mut ModuleResolver>,
    ) -> Result<Vec<String>> {
        let source = fs::read_to_string(file_path)
            .with_context(|| format!("Failed to read file: {file_path:?}"))?;
        let source = normalize_line_endings(source);

        let parsed = ruff_python_parser::parse_module(&source)
            .with_context(|| format!("Failed to parse Python file: {file_path:?}"))?;

        // Use the visitor to discover all imports
        // Note: For simple import discovery, we don't need semantic analysis
        let mut visitor = ImportDiscoveryVisitor::new();
        for stmt in &parsed.syntax().body {
            visitor.visit_stmt(stmt);
        }

        let discovered_imports = visitor.into_imports();
        let mut imports_set = IndexSet::new();

        // Convert discovered imports to module names, handling relative imports
        for import in &discovered_imports {
            // All import locations are valid for discovery
            if import.level > 0 {
                self.process_relative_import_set(import, file_path, &mut imports_set);
            } else if let Some(ref module_name) = import.module_name {
                // Absolute imports
                imports_set.insert(module_name.clone());

                // Check if any imported names are actually submodules
                // Only do this for non-relative imports to avoid issues
                self.check_submodule_imports_set(
                    module_name,
                    import,
                    &mut resolver,
                    &mut imports_set,
                );
            } else if import.names.len() == 1 {
                self.process_single_name_import_set(import, &mut resolver, &mut imports_set);
            }
        }

        // Convert IndexSet to Vec to maintain the existing API
        let imports: Vec<String> = imports_set.into_iter().collect();

        // Log discovery of function-scoped imports for debugging
        for import in &discovered_imports {
            if !matches!(import.location, ImportLocation::Module) {
                debug!(
                    "Found nested import: {:?} at {:?}",
                    import
                        .module_name
                        .as_ref()
                        .unwrap_or(&"<names>".to_string()),
                    import.location
                );
            }
        }

        Ok(imports)
    }

    /// Process relative imports and add to IndexSet
    fn process_relative_import_set(
        &self,
        import: &crate::visitors::DiscoveredImport,
        file_path: &Path,
        imports: &mut IndexSet<String>,
    ) {
        let base_module = match self.resolve_relative_import(file_path, import.level) {
            Some(module) => module,
            None => {
                debug!(
                    "Could not resolve relative import with level {}",
                    import.level
                );
                return;
            }
        };

        if import.names.is_empty() {
            if let Some(ref module_name) = import.module_name {
                let full_module = if base_module.is_empty() {
                    module_name.clone()
                } else {
                    format!("{base_module}.{module_name}")
                };
                imports.insert(full_module);
            }
        } else if let Some(ref module_name) = import.module_name {
            let full_module = if base_module.is_empty() {
                module_name.clone()
            } else {
                format!("{base_module}.{module_name}")
            };
            imports.insert(full_module);
        } else if !import.names.is_empty() && !base_module.is_empty() {
            // Add the base module
            imports.insert(base_module.clone());

            // For "from . import X", check if X is actually a submodule
            for (name, _) in &import.names {
                let potential_submodule = format!("{base_module}.{name}");
                imports.insert(potential_submodule);
                debug!("Added potential submodule from relative import: {name}");
            }
        }
    }

    /// Process a single name import that might be a submodule (IndexSet version)
    fn process_single_name_import_set(
        &self,
        import: &crate::visitors::DiscoveredImport,
        resolver: &mut Option<&mut ModuleResolver>,
        imports: &mut IndexSet<String>,
    ) {
        if let Some(resolver) = resolver {
            let (name, _) = &import.names[0];
            match resolver.classify_import(name) {
                ImportType::StandardLibrary | ImportType::ThirdParty | ImportType::FirstParty => {
                    imports.insert(name.clone());
                }
            }
        }
    }

    /// Check if any imported names are actually submodules (IndexSet version)
    fn check_submodule_imports_set(
        &self,
        module_name: &str,
        import: &crate::visitors::DiscoveredImport,
        resolver: &mut Option<&mut ModuleResolver>,
        imports: &mut IndexSet<String>,
    ) {
        let Some(resolver) = resolver else { return };

        for (name, _) in &import.names {
            let full_module_name = format!("{module_name}.{name}");
            // Try to resolve the full module name to see if it's a module
            if resolver
                .resolve_module_path(&full_module_name)
                .is_ok_and(|path| path.is_some())
            {
                imports.insert(full_module_name);
                debug!("Detected submodule import: {name} from {module_name}");
            }
        }
    }

    /// Extract import module names from a single AST statement
    fn extract_imports_from_statement(
        &self,
        stmt: &Stmt,
        context: &mut ImportExtractionContext<'_>,
    ) {
        if let Stmt::Import(import_stmt) = stmt {
            for alias in &import_stmt.names {
                // For dotted imports like "xml.etree.ElementTree", we need to extract
                // the root module name "xml" for classification purposes, but we should
                // preserve the full import path for the output
                #[allow(clippy::disallowed_methods)]
                let module_name = alias.name.id.to_string();
                context.imports.push(module_name);
            }
        } else if let Stmt::ImportFrom(import_from_stmt) = stmt {
            self.process_import_from_statement(import_from_stmt, context);
        }
    }

    /// Process a "from ... import ..." statement to extract module names
    fn process_import_from_statement(
        &self,
        import_from_stmt: &StmtImportFrom,
        context: &mut ImportExtractionContext<'_>,
    ) {
        let level = import_from_stmt.level;

        if level == 0 {
            if let Some(ref mut resolver) = context.resolver {
                self.process_absolute_import_with_resolver(
                    import_from_stmt,
                    context.imports,
                    resolver,
                );
            } else {
                self.process_absolute_import(import_from_stmt, context.imports);
            }
            return;
        }

        // Handle relative imports
        // TODO: Consider extending resolver support to relative imports as well
        if let Some(base_module) = self.resolve_relative_import(context.file_path, level) {
            self.process_resolved_relative_import(import_from_stmt, context.imports, &base_module);
        } else {
            self.process_fallback_relative_import(import_from_stmt, context.imports, level);
        }
    }

    /// Process absolute imports (level == 0)
    fn process_absolute_import(
        &self,
        import_from_stmt: &StmtImportFrom,
        imports: &mut Vec<String>,
    ) {
        if let Some(ref module) = import_from_stmt.module {
            #[allow(clippy::disallowed_methods)]
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
            #[allow(clippy::disallowed_methods)]
            let m = module.id.to_string();
            // Add the package/module being imported from
            if !imports.contains(&m) {
                imports.push(m.clone());
            }

            // Check if any of the imported names are actually modules
            let mut module_context = ModuleImportContext {
                imports,
                base_module: &m,
                resolver,
            };
            self.check_and_add_module_imports(import_from_stmt, &mut module_context);
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
            target_module.to_owned()
        } else {
            format!("{base_module}.{target_module}")
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
            format!("{dots}{module}")
        } else {
            module.to_owned()
        }
    }

    /// Resolve a relative import to its absolute module name
    /// Returns None if the relative import goes beyond the module hierarchy
    #[allow(clippy::manual_ok_err)]
    fn resolve_relative_import(&self, file_path: &Path, level: u32) -> Option<String> {
        // Get the directory containing the current file
        let current_dir = file_path.parent()?;

        // Convert current_dir to absolute path if it's relative
        let absolute_current_dir = if current_dir.is_absolute() {
            current_dir.to_path_buf()
        } else {
            let current_working_dir = match std::env::current_dir() {
                Ok(dir) => dir,
                Err(_) => return None,
            };
            current_working_dir.join(current_dir)
        };

        // Find which source directory contains this file
        let relative_dir = self.config.src.iter().find_map(|src_dir| {
            // Handle case where paths might be relative vs absolute
            if src_dir.is_absolute() {
                match absolute_current_dir.strip_prefix(src_dir) {
                    Ok(path) => Some(path),
                    Err(_) => None,
                }
            } else {
                // For relative source directories, we need to resolve them relative to the current
                // working directory
                let current_working_dir = match std::env::current_dir() {
                    Ok(dir) => dir,
                    Err(_) => return None,
                };
                let absolute_src_dir = current_working_dir.join(src_dir);
                match absolute_current_dir.strip_prefix(&absolute_src_dir) {
                    Ok(path) => Some(path),
                    Err(_) => None,
                }
            }
        })?;

        // Convert directory path to module path components
        let module_parts: Vec<String> = if relative_dir == Path::new("") {
            // If relative_dir is empty, we're at the root of the source directory
            Vec::new()
        } else {
            relative_dir
                .components()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
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
            debug!("Adding '{import}' to discovery queue");
            discovery_params
                .modules_to_process
                .push((import.to_owned(), import_path));
            discovery_params.queued_modules.insert(import.to_owned());
        } else {
            debug!("Module '{import}' already processed or queued, skipping");
        }
    }

    /// Add parent packages to discovery queue to ensure __init__.py files are included
    /// For example, if importing "greetings.irrelevant", also add "greetings"
    fn add_parent_packages_to_discovery(&self, import: &str, params: &mut DiscoveryParams) {
        let parts: Vec<&str> = import.split('.').collect();

        // For each parent package level, try to add it to discovery
        for i in 1..parts.len() {
            let parent_module = parts[..i].join(".");
            self.try_add_parent_package_to_discovery(&parent_module, import, params);
        }
    }

    /// Try to add a single parent package to discovery if it's first-party
    fn try_add_parent_package_to_discovery(
        &self,
        parent_module: &str,
        import: &str,
        params: &mut DiscoveryParams,
    ) {
        match params.resolver.classify_import(parent_module) {
            ImportType::FirstParty => {
                if let Ok(Some(parent_path)) = params.resolver.resolve_module_path(parent_module) {
                    debug!(
                        "Adding parent package '{parent_module}' to discovery queue for import \
                         '{import}'"
                    );
                    self.add_to_discovery_queue_if_new(parent_module, parent_path, params);
                }
            }
            _ => {
                // Parent is not first-party, processing stops here
            }
        }
    }

    /// Process an import during discovery phase
    fn process_import_for_discovery(&self, import: &str, params: &mut DiscoveryParams) {
        match params.resolver.classify_import(import) {
            ImportType::FirstParty => {
                debug!("'{import}' classified as FirstParty");
                if let Ok(Some(import_path)) = params.resolver.resolve_module_path(import) {
                    debug!("Resolved '{import}' to path: {import_path:?}");
                    self.add_to_discovery_queue_if_new(import, import_path, params);

                    // Also add parent packages for submodules to ensure __init__.py files are
                    // included For example, if importing
                    // "greetings.irrelevant", also add "greetings"
                    self.add_parent_packages_to_discovery(import, params);
                } else {
                    warn!("Failed to resolve path for first-party module: {import}");
                }
            }
            ImportType::ThirdParty | ImportType::StandardLibrary => {
                debug!("'{import}' classified as external (preserving)");
            }
        }
    }

    /// Process an import during dependency graph creation phase
    fn process_import_for_dependency(&self, import: &str, context: &mut DependencyContext<'_>) {
        match context.resolver.classify_import(import) {
            ImportType::FirstParty => {
                // Add dependency edge if the imported module exists
                if let Some(&to_module_id) = context.module_id_map.get(import) {
                    debug!(
                        "Adding dependency edge: {} -> {}",
                        import, context.current_module
                    );
                    // TODO: Properly track TYPE_CHECKING information from ImportDiscoveryVisitor
                    // For now, we use the default (is_type_checking_only = false)
                    // This should be updated to use the actual is_type_checking_only flag from
                    // the DiscoveredImport when we refactor to preserve that information
                    context
                        .graph
                        .add_module_dependency(context.from_module_id, to_module_id);
                    debug!(
                        "Successfully added dependency edge: {} -> {}",
                        import, context.current_module
                    );
                } else {
                    debug!("Module {import} not found in graph, skipping dependency edge");
                }

                // Also add dependency edges for parent packages
                // For example, if importing "greetings.irrelevant", also add dependency on
                // "greetings"
                self.add_parent_package_dependencies(import, context);
            }
            ImportType::ThirdParty | ImportType::StandardLibrary => {
                // These will be preserved in the output, not inlined
            }
        }
    }

    /// Add dependency edges for parent packages to ensure proper ordering
    fn add_parent_package_dependencies(&self, import: &str, context: &mut DependencyContext<'_>) {
        let parts: Vec<&str> = import.split('.').collect();

        // For each parent package level, add a dependency edge
        for i in 1..parts.len() {
            let parent_module = parts[..i].join(".");
            self.try_add_parent_dependency(&parent_module, context);
        }
    }

    /// Try to add a dependency edge for a parent package
    fn try_add_parent_dependency(&self, parent_module: &str, context: &mut DependencyContext<'_>) {
        // Skip if parent_module is the same as module_name to avoid self-dependencies
        if parent_module == context.current_module {
            debug!(
                "Skipping self-dependency: {} -> {}",
                parent_module, context.current_module
            );
            return;
        }

        if context.resolver.classify_import(parent_module) == ImportType::FirstParty
            && let Some(&parent_module_id) = context.module_id_map.get(parent_module)
        {
            debug!(
                "Adding parent package dependency edge: {} -> {}",
                parent_module, context.current_module
            );
            // TODO: Inherit TYPE_CHECKING information from child import
            context
                .graph
                .add_module_dependency(context.from_module_id, parent_module_id);
        }
    }

    /// Write requirements.txt file for stdout mode (current directory)
    fn write_requirements_file_for_stdout(
        &self,
        sorted_modules: &[(String, PathBuf, Vec<String>)],
        resolver: &ModuleResolver,
    ) -> Result<()> {
        let requirements_content = self.generate_requirements(sorted_modules, resolver)?;
        if !requirements_content.is_empty() {
            let requirements_path = Path::new("requirements.txt");

            fs::write(requirements_path, requirements_content).with_context(|| {
                format!("Failed to write requirements file: {requirements_path:?}")
            })?;

            info!("Requirements written to: {requirements_path:?}");
        } else {
            info!("No third-party dependencies found, skipping requirements.txt");
        }
        Ok(())
    }

    /// Write requirements.txt file if there are dependencies
    fn write_requirements_file(
        &self,
        sorted_modules: &[(String, PathBuf, Vec<String>)],
        resolver: &ModuleResolver,
        output_path: &Path,
    ) -> Result<()> {
        let requirements_content = self.generate_requirements(sorted_modules, resolver)?;
        if !requirements_content.is_empty() {
            let requirements_path = output_path
                .parent()
                .unwrap_or_else(|| Path::new("."))
                .join("requirements.txt");

            fs::write(&requirements_path, requirements_content).with_context(|| {
                format!("Failed to write requirements file: {requirements_path:?}")
            })?;

            info!("Requirements written to: {requirements_path:?}");
        } else {
            info!("No third-party dependencies found, skipping requirements.txt");
        }
        Ok(())
    }

    /// Helper method to build detailed error message for cycles with resolvable cycles handling
    fn build_cycle_error_message(
        analysis: &crate::cribo_graph::CircularDependencyAnalysis,
    ) -> String {
        let mut error_msg = String::from("Circular dependencies detected in the module graph:\n\n");

        // First, show unresolvable cycles if any
        if !analysis.unresolvable_cycles.is_empty() {
            error_msg.push_str("UNRESOLVABLE CYCLES:\n");
            Self::append_unresolvable_cycles_to_error(
                &mut error_msg,
                &analysis.unresolvable_cycles,
            );
        }

        // Then show resolvable cycles that are not yet supported
        if !analysis.resolvable_cycles.is_empty() {
            Self::append_resolvable_cycles_to_error(
                &mut error_msg,
                &analysis.resolvable_cycles,
                &analysis.unresolvable_cycles,
            );
        }

        error_msg
    }

    /// Helper method to append unresolvable cycles to error message
    fn append_unresolvable_cycles_to_error(
        error_msg: &mut String,
        unresolvable_cycles: &[CircularDependencyGroup],
    ) {
        for (i, cycle) in unresolvable_cycles.iter().enumerate() {
            error_msg.push_str(&format!("Cycle {}: {}\n", i + 1, cycle.modules.join(" → ")));
            error_msg.push_str(&format!("  Type: {:?}\n", cycle.cycle_type));
            if let ResolutionStrategy::Unresolvable { reason } = &cycle.suggested_resolution {
                error_msg.push_str(&format!("  Reason: {reason}\n"));
            }
            error_msg.push('\n');
        }
    }

    /// Helper method to append resolvable cycles to error message
    fn append_resolvable_cycles_to_error(
        error_msg: &mut String,
        resolvable_cycles: &[CircularDependencyGroup],
        unresolvable_cycles: &[CircularDependencyGroup],
    ) {
        if !unresolvable_cycles.is_empty() {
            error_msg.push_str("RESOLVABLE CYCLES (not yet implemented):\n");
        }
        for (i, cycle) in resolvable_cycles.iter().enumerate() {
            let cycle_num = i + 1 + unresolvable_cycles.len();
            error_msg.push_str(&format!(
                "Cycle {}: {}\n",
                cycle_num,
                cycle.modules.join(" → ")
            ));
            error_msg.push_str(&format!("  Type: {:?}\n", cycle.cycle_type));

            Self::append_cycle_resolution_suggestions(error_msg, &cycle.suggested_resolution);
            error_msg.push('\n');
        }
    }

    /// Helper method to append cycle resolution suggestions to error message
    fn append_cycle_resolution_suggestions(
        error_msg: &mut String,
        suggested_resolution: &ResolutionStrategy,
    ) {
        match suggested_resolution {
            ResolutionStrategy::LazyImport { modules: _ } => {
                error_msg.push_str(
                    "  Suggestion: Move imports inside functions to enable lazy loading\n",
                );
            }
            ResolutionStrategy::FunctionScopedImport { import_statements } => {
                error_msg.push_str("  Suggestions:\n");
                for suggestion in import_statements {
                    error_msg.push_str(&format!("    {suggestion}\n"));
                }
            }
            ResolutionStrategy::ModuleSplit { suggestions } => {
                error_msg.push_str("  Suggestions:\n");
                for suggestion in suggestions {
                    error_msg.push_str(&format!("    {suggestion}\n"));
                }
            }
            ResolutionStrategy::Unresolvable { .. } => {
                // This shouldn't happen in resolvable cycles
            }
        }
    }

    /// Helper method to log resolvable cycles
    fn log_resolvable_cycles(cycles: &[CircularDependencyGroup]) {
        for cycle in cycles {
            info!("Resolving cycle: {}", cycle.modules.join(" → "));
        }
    }

    /// Helper method to check and add module imports to reduce nesting
    fn check_and_add_module_imports(
        &self,
        import_from_stmt: &StmtImportFrom,
        context: &mut ModuleImportContext<'_>,
    ) {
        for alias in &import_from_stmt.names {
            #[allow(clippy::disallowed_methods)]
            let imported_name = alias.name.id.to_string();
            let full_module_name = format!("{}.{}", context.base_module, imported_name);

            // Try to resolve the full module name to see if it's a module
            let Ok(Some(_)) = context.resolver.resolve_module_path(&full_module_name) else {
                continue;
            };

            // This is a module import (e.g., from greetings import greeting)
            if context.imports.contains(&full_module_name) {
                continue;
            }
            context.imports.push(full_module_name);
            debug!(
                "Detected module import: {} from {}",
                imported_name, context.base_module
            );
        }
    }

    /// Emit bundle using static bundler (no exec calls)
    fn emit_static_bundle(&mut self, params: StaticBundleParams<'_>) -> Result<String> {
        // First, detect and resolve conflicts after all modules have been analyzed
        let conflicts = self.semantic_bundler.detect_and_resolve_conflicts();
        if !conflicts.is_empty() {
            info!(
                "Detected {} symbol conflicts across modules, applying renaming strategy",
                conflicts.len()
            );
            for conflict in &conflicts {
                debug!(
                    "Symbol '{}' conflicts across modules: {:?}",
                    conflict.symbol, conflict.modules
                );
            }
        }

        let mut static_bundler = HybridStaticBundler::new();

        // Parse all modules and prepare them for bundling
        let mut module_asts = Vec::new();

        // Check if we have pre-parsed modules
        if let Some(parsed_modules) = params.parsed_modules {
            // Use pre-parsed modules to avoid double parsing
            for (module_name, module_path, _imports, ast, source) in parsed_modules {
                // Calculate content hash for deterministic module naming
                use sha2::{Digest, Sha256};
                let mut hasher = Sha256::new();
                hasher.update(source.as_bytes());
                let hash = hasher.finalize();
                let content_hash = format!("{hash:x}");

                module_asts.push((
                    module_name.clone(),
                    ast.clone(),
                    module_path.clone(),
                    content_hash,
                ));
            }
        } else {
            // Fall back to parsing modules if not pre-parsed
            for (module_name, module_path, _imports) in params.sorted_modules {
                let source = fs::read_to_string(module_path)
                    .with_context(|| format!("Failed to read module file: {module_path:?}"))?;
                let source = crate::util::normalize_line_endings(source);
                // Calculate content hash for deterministic module naming
                use sha2::{Digest, Sha256};
                let mut hasher = Sha256::new();
                hasher.update(source.as_bytes());
                let hash = hasher.finalize();
                let content_hash = format!("{hash:x}");

                // Parse into AST
                let ast = ruff_python_parser::parse_module(&source)
                    .with_context(|| format!("Failed to parse module: {module_path:?}"))?;

                module_asts.push((
                    module_name.clone(),
                    ast.into_syntax(),
                    module_path.clone(),
                    content_hash,
                ));
            }
        }

        // Apply import rewriting if we have resolvable circular dependencies
        if let Some(analysis) = params.circular_dep_analysis
            && !analysis.resolvable_cycles.is_empty()
        {
            info!("Applying function-scoped import rewriting to resolve circular dependencies");

            // Create import rewriter
            let mut import_rewriter =
                ImportRewriter::new(ImportDeduplicationStrategy::FunctionStart);

            // Prepare module ASTs for semantic analysis
            let module_ast_pairs: Vec<(String, ModModule)> = module_asts
                .iter()
                .map(|(name, ast, _, _)| (name.clone(), ast.clone()))
                .collect();

            // Analyze movable imports using semantic analysis
            let movable_imports = import_rewriter.analyze_movable_imports_semantic(
                params.graph,
                &analysis.resolvable_cycles,
                &self.semantic_bundler,
                &module_ast_pairs,
            )?;

            debug!(
                "Found {} imports that can be moved to function scope using semantic analysis",
                movable_imports.len()
            );

            // Apply rewriting to each module AST
            for (module_name, ast, _, _) in &mut module_asts {
                import_rewriter.rewrite_module(ast, &movable_imports, module_name)?;
            }
        }

        // Bundle all modules using static bundler
        let bundled_ast =
            static_bundler.bundle_modules(crate::code_generator::BundleParams {
                modules: module_asts,
                sorted_modules: params.sorted_modules,
                entry_module_name: params.entry_module_name,
                graph: params.graph,
                semantic_bundler: &self.semantic_bundler,
                circular_dep_analysis: params.circular_dep_analysis,
            })?;

        // Generate Python code from AST
        let empty_parsed = ruff_python_parser::parse_module("")?;
        let stylist = ruff_python_codegen::Stylist::from_tokens(empty_parsed.tokens(), "");

        let mut code_parts = Vec::new();
        for stmt in &bundled_ast.body {
            let generator = ruff_python_codegen::Generator::from(&stylist);
            let stmt_code = generator.stmt(stmt);
            code_parts.push(stmt_code);
        }

        // Add shebang and header
        let mut final_output = vec![
            "#!/usr/bin/env python3".to_string(),
            "# Generated by Cribo - Python Source Bundler".to_string(),
            "# https://github.com/ophidiarium/cribo".to_string(),
            String::new(), // Empty line
        ];
        final_output.extend(code_parts);

        Ok(final_output.join("\n"))
    }

    /// Generate requirements.txt content from third-party imports
    fn generate_requirements(
        &self,
        modules: &[(String, PathBuf, Vec<String>)],
        resolver: &ModuleResolver,
    ) -> Result<String> {
        let mut third_party_imports = IndexSet::new();

        // TODO: Use TYPE_CHECKING information from the dependency graph to filter out
        // dependencies that are only used for type checking. These could be placed
        // in a separate section or excluded entirely based on configuration.
        // For now, all third-party imports are included.
        for (_module_name, _module_path, imports) in modules {
            for import in imports {
                debug!("Checking import '{import}' for requirements");
                if let ImportType::ThirdParty = resolver.classify_import(import) {
                    // Extract top-level package name
                    let package_name = import.split('.').next().unwrap_or(import);
                    debug!("Adding '{package_name}' to requirements (from '{import}')");
                    third_party_imports.insert(package_name.to_string());
                }
            }
        }

        let mut requirements: Vec<String> = third_party_imports.into_iter().collect();
        requirements.sort();

        Ok(requirements.join("\n"))
    }
}
