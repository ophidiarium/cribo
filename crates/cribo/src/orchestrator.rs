use std::{
    fs,
    path::{Path, PathBuf},
    sync::OnceLock,
};

use anyhow::{Context, Result, anyhow};
use indexmap::IndexSet;
use log::{debug, info, trace, warn};
use ruff_python_ast::{ModModule, visitor::Visitor};

use crate::{
    analyzers::types::{
        CircularDependencyAnalysis, CircularDependencyGroup, CircularDependencyType,
        ResolutionStrategy,
    },
    code_generator::Bundler,
    config::Config,
    cribo_graph::{CriboGraph, ModuleId},
    import_rewriter::{ImportDeduplicationStrategy, ImportRewriter},
    resolver::{ImportType, ModuleResolver},
    semantic_bundler::SemanticBundler,
    tree_shaking::TreeShaker,
    types::FxIndexMap,
    util::{module_name_from_relative, normalize_line_endings},
    visitors::{ImportDiscoveryVisitor, ImportLocation, ScopeElement},
};

/// Static empty parsed module for creating Stylist instances
static EMPTY_PARSED_MODULE: OnceLock<ruff_python_parser::Parsed<ModModule>> = OnceLock::new();

/// Immutable module information stored in the registry
#[derive(Debug, Clone)]
pub struct ModuleInfo {
    /// The unique module ID assigned by the dependency graph
    pub id: ModuleId,
    /// The canonical module name (e.g., "requests.compat")
    pub canonical_name: String,
    /// The resolved filesystem path
    pub resolved_path: PathBuf,
}

/// Central registry for module information
/// This is the single source of truth for module identity throughout the bundling process
pub struct ModuleRegistry {
    /// Map from `ModuleId` to complete module information
    modules: FxIndexMap<ModuleId, ModuleInfo>,
    /// Map from canonical name to `ModuleId` for fast lookups
    name_to_id: FxIndexMap<String, ModuleId>,
    /// Map from resolved path to `ModuleId` for fast lookups
    path_to_id: FxIndexMap<PathBuf, ModuleId>,
}

impl Default for ModuleRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl ModuleRegistry {
    /// Create a new empty module registry
    pub fn new() -> Self {
        Self {
            modules: FxIndexMap::default(),
            name_to_id: FxIndexMap::default(),
            path_to_id: FxIndexMap::default(),
        }
    }

    /// Add a module to the registry
    pub fn add_module(&mut self, info: ModuleInfo) {
        let id = info.id;
        let name = info.canonical_name.clone();
        let path = info.resolved_path.clone();

        // Check if module already exists and validate consistency
        if let Some(existing) = self.modules.get(&id) {
            assert!(
                !(existing.canonical_name != name || existing.resolved_path != path),
                "Attempting to register module {:?} with conflicting data. Existing: {} at {}, \
                 New: {} at {}",
                id,
                existing.canonical_name,
                existing.resolved_path.display(),
                name,
                path.display()
            );
            return; // Module already registered with same data
        }

        self.name_to_id.insert(name, id);
        self.path_to_id.insert(path, id);
        self.modules.insert(id, info);
    }

    /// Get module ID by canonical name
    pub fn get_id_by_name(&self, name: &str) -> Option<ModuleId> {
        self.name_to_id.get(name).copied()
    }
}

/// Get or create the empty parsed module for Stylist creation
fn get_empty_parsed_module() -> &'static ruff_python_parser::Parsed<ModModule> {
    EMPTY_PARSED_MODULE
        .get_or_init(|| ruff_python_parser::parse_module("").expect("Failed to parse empty module"))
}

/// Type alias for module processing queue
type ModuleQueue = Vec<(String, PathBuf)>;
/// Type alias for processed modules set
type ProcessedModules = IndexSet<String>;
/// Type alias for parsed module data with AST and source
type ParsedModuleData = (String, PathBuf, Vec<String>, ModModule, String);
/// Type alias for import extraction result
type ImportExtractionResult = Vec<(
    String,
    bool,
    Option<crate::visitors::ImportType>,
    Option<String>,
)>;

/// Parameters for discovery phase operations
struct DiscoveryParams<'a> {
    resolver: &'a ModuleResolver,
    modules_to_process: &'a mut ModuleQueue,
    processed_modules: &'a ProcessedModules,
    queued_modules: &'a mut IndexSet<String>,
}

/// Parameters for static bundle emission
struct StaticBundleParams<'a> {
    sorted_modules: &'a [(String, PathBuf, Vec<String>)],
    parsed_modules: Option<&'a [ParsedModuleData]>, // Optional pre-parsed modules
    resolver: &'a ModuleResolver,
    entry_module_name: &'a str,
    graph: &'a CriboGraph,
    circular_dep_analysis: Option<&'a CircularDependencyAnalysis>,
    tree_shaker: Option<&'a TreeShaker>,
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
    resolver: &'a ModuleResolver,
    graph: &'a mut CriboGraph,
}

/// Result of the AST processing pipeline
#[derive(Clone)]
struct ProcessedModule {
    /// The transformed AST after all pipeline stages
    ast: ModModule,
    /// The original source code (needed for semantic analysis and code generation)
    source: String,
    /// Normalized imports map (alias -> canonical)
    normalized_imports: FxIndexMap<String, String>,
    /// Modules created by stdlib normalization (e.g., "abc", "collections")
    normalized_modules: crate::types::FxIndexSet<String>,
    /// Module ID if already added to dependency graph
    module_id: Option<crate::cribo_graph::ModuleId>,
}

pub struct BundleOrchestrator {
    config: Config,
    semantic_bundler: SemanticBundler,
    /// Central registry for module information
    module_registry: ModuleRegistry,
    /// Cache of processed modules to ensure we only parse and transform once
    module_cache: std::sync::Mutex<FxIndexMap<PathBuf, ProcessedModule>>,
}

impl BundleOrchestrator {
    pub fn new(config: Config) -> Self {
        Self {
            config,
            semantic_bundler: SemanticBundler::new(),
            module_registry: ModuleRegistry::new(),
            module_cache: std::sync::Mutex::new(FxIndexMap::default()),
        }
    }

    /// Single entry point for parsing and processing modules
    /// This is THE ONLY place where `ruff_python_parser::parse_module` should be called
    ///
    /// Pipeline:
    /// 1. Check cache
    /// 2. Read file and parse
    /// 3. Semantic analysis (on raw AST)
    /// 4. Stdlib normalization (transforms AST)
    /// 5. Cache result
    fn process_module(
        &mut self,
        module_path: &Path,
        module_name: &str,
        graph: Option<&mut CriboGraph>,
    ) -> Result<ProcessedModule> {
        // Canonicalize path for consistent caching
        let canonical_path = module_path
            .canonicalize()
            .unwrap_or_else(|_| module_path.to_path_buf());

        // Check cache first
        let cached_data = {
            let cache = self
                .module_cache
                .lock()
                .expect("Failed to acquire module cache lock");
            cache.get(&canonical_path).cloned()
        };

        if let Some(cached) = cached_data {
            debug!("Using cached module: {module_name}");

            // If graph is provided but cached module doesn't have module_id,
            // we need to add it to the graph
            let module_id = if let Some(graph) = graph {
                if cached.module_id.is_none() {
                    let module_id = graph.add_module(module_name.to_string(), module_path);

                    // Perform semantic analysis
                    self.semantic_bundler.analyze_module(
                        module_id,
                        &cached.ast,
                        &cached.source,
                        module_path,
                    )?;

                    // Add to module registry
                    let module_info = ModuleInfo {
                        id: module_id,
                        canonical_name: module_name.to_string(),
                        resolved_path: canonical_path.clone(),
                    };
                    self.module_registry.add_module(module_info);

                    Some(module_id)
                } else {
                    cached.module_id
                }
            } else {
                cached.module_id
            };

            return Ok(ProcessedModule {
                ast: cached.ast.clone(),
                source: cached.source.clone(),
                normalized_imports: cached.normalized_imports.clone(),
                normalized_modules: cached.normalized_modules.clone(),
                module_id,
            });
        }

        debug!(
            "Processing module: {module_name} from {}",
            module_path.display()
        );

        // Step 1: Read and parse (ONLY place where parse_module is called)
        let source = fs::read_to_string(module_path)
            .with_context(|| format!("Failed to read file: {}", module_path.display()))?;
        let source = normalize_line_endings(&source);

        let parsed = ruff_python_parser::parse_module(&source)
            .with_context(|| format!("Failed to parse Python file: {}", module_path.display()))?;
        let mut ast = parsed.into_syntax();

        // Step 2: Add to graph and perform semantic analysis (if graph provided)
        let module_id = if let Some(graph) = graph {
            let module_id = graph.add_module(module_name.to_string(), module_path);

            // Semantic analysis on raw AST
            self.semantic_bundler
                .analyze_module(module_id, &ast, &source, module_path)?;

            // Add to module registry
            let module_info = ModuleInfo {
                id: module_id,
                canonical_name: module_name.to_string(),
                resolved_path: canonical_path.clone(),
            };
            self.module_registry.add_module(module_info);

            Some(module_id)
        } else {
            None
        };

        // Step 3: Stdlib normalization (transforms AST)
        let normalization_result = crate::stdlib_normalization::normalize_stdlib_imports(&mut ast);
        debug!(
            "Normalized imports for {module_name}: {:?}",
            normalization_result.alias_to_canonical
        );
        debug!(
            "Normalized modules for {module_name}: {:?}",
            normalization_result.normalized_modules
        );

        // Step 4: Cache the result
        let processed = ProcessedModule {
            ast: ast.clone(),
            source: source.clone(),
            normalized_imports: normalization_result.alias_to_canonical.clone(),
            normalized_modules: normalization_result.normalized_modules.clone(),
            module_id,
        };

        {
            let mut cache = self
                .module_cache
                .lock()
                .expect("Failed to acquire module cache lock");
            cache.insert(canonical_path, processed.clone());
        }

        Ok(ProcessedModule {
            ast,
            source,
            normalized_imports: normalization_result.alias_to_canonical,
            normalized_modules: normalization_result.normalized_modules,
            module_id,
        })
    }

    /// Format error message for unresolvable cycles
    fn format_unresolvable_cycles_error(cycles: &[CircularDependencyGroup]) -> String {
        use std::fmt::Write;
        let mut error_msg = String::from("Unresolvable circular dependencies detected:\n\n");

        for (i, cycle) in cycles.iter().enumerate() {
            let _ = writeln!(error_msg, "Cycle {}: {}", i + 1, cycle.modules.join(" → "));
            let _ = writeln!(error_msg, "  Type: {:?}", cycle.cycle_type);

            if let ResolutionStrategy::Unresolvable { reason } = &cycle.suggested_resolution {
                let _ = writeln!(error_msg, "  Reason: {reason}");
            }
            error_msg.push('\n');
        }

        error_msg
    }

    /// Core bundling logic shared between file and string output modes
    /// Returns the entry module name, parsed modules, circular dependency analysis, and optional
    /// tree shaker, with graph and resolver populated via mutable references
    fn bundle_core(
        &mut self,
        entry_path: &Path,
        graph: &mut CriboGraph,
        resolver_opt: &mut Option<ModuleResolver>,
    ) -> Result<(
        String,
        Vec<ParsedModuleData>,
        Option<CircularDependencyAnalysis>,
        Option<TreeShaker>,
    )> {
        // Handle directory as entry point
        let entry_path = if entry_path.is_dir() {
            // Check for __main__.py first
            let main_py = entry_path.join("__main__.py");
            if main_py.exists() && main_py.is_file() {
                info!(
                    "Using __main__.py as entry point from directory: {}",
                    entry_path.display()
                );
                main_py
            } else {
                // Check for __init__.py
                let init_py = entry_path.join("__init__.py");
                if init_py.exists() && init_py.is_file() {
                    info!(
                        "Using __init__.py as entry point from directory: {}",
                        entry_path.display()
                    );
                    init_py
                } else {
                    return Err(anyhow!(
                        "Directory {} does not contain __main__.py or __init__.py",
                        entry_path.display()
                    ));
                }
            }
        } else if entry_path.is_file() {
            entry_path.to_path_buf()
        } else {
            return Err(anyhow!(
                "Entry path {} does not exist or is not a file or directory",
                entry_path.display()
            ));
        };

        // Use a reference to the resolved entry_path for the rest of the function
        let entry_path = &entry_path;

        debug!("Entry: {}", entry_path.display());
        debug!(
            "Using target Python version: {} (Python 3.{})",
            self.config.target_version,
            self.config.python_version().unwrap_or(10)
        );

        // Auto-detect the entry point's directory as a source directory
        if let Some(entry_dir) = entry_path.parent() {
            // Check if this is a package __init__.py or __main__.py file
            let filename = entry_path
                .file_name()
                .and_then(|f| f.to_str())
                .unwrap_or("");
            let is_package_entry = filename == "__init__.py" || filename == "__main__.py";

            // If it's __init__.py or __main__.py, use the parent's parent as the src directory
            // to preserve the package structure
            let src_dir = if is_package_entry {
                entry_dir.parent().unwrap_or(entry_dir)
            } else {
                entry_dir
            };

            // Canonicalize the path to avoid duplicates due to different lexical representations
            let src_dir = match src_dir.canonicalize() {
                Ok(canonical_path) => canonical_path,
                Err(_) => {
                    // Fall back to the original path if canonicalization fails (e.g., path doesn't
                    // exist)
                    src_dir.to_path_buf()
                }
            };
            if !self.config.src.contains(&src_dir) {
                debug!("Adding entry directory to src paths: {}", src_dir.display());
                self.config.src.insert(0, src_dir);
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
            resolver: &resolver,
            graph,
        };
        let parsed_modules = self.build_dependency_graph(&mut build_params)?;

        // In CriboGraph, we track all modules but focus on reachable ones
        debug!("Graph has {} modules", graph.modules.len());

        // Enhanced circular dependency detection and analysis
        let mut circular_dep_analysis = None;
        if graph.has_cycles() {
            let analysis = crate::analyzers::dependency_analyzer::DependencyAnalyzer::analyze_circular_dependencies(graph);

            // Check if we have unresolvable cycles - these we must fail on
            if !analysis.unresolvable_cycles.is_empty() {
                let error_msg =
                    Self::format_unresolvable_cycles_error(&analysis.unresolvable_cycles);
                return Err(anyhow!(error_msg));
            }

            // For resolvable cycles, warn but proceed
            if !analysis.resolvable_cycles.is_empty() {
                warn!(
                    "Detected {} potentially resolvable circular dependencies",
                    analysis.resolvable_cycles.len()
                );

                // Log details about each resolvable cycle
                for (i, cycle) in analysis.resolvable_cycles.iter().enumerate() {
                    warn!(
                        "Cycle {}: {} (Type: {:?})",
                        i + 1,
                        cycle.modules.join(" → "),
                        cycle.cycle_type
                    );

                    // Provide specific warnings for non-function-level cycles
                    match cycle.cycle_type {
                        CircularDependencyType::ClassLevel => {
                            warn!(
                                "  ⚠️  ClassLevel cycle detected - bundling may fail if imports \
                                 are used before definition"
                            );
                            warn!(
                                "  Suggestion: Consider refactoring to avoid module-level \
                                 circular imports"
                            );
                        }
                        CircularDependencyType::ModuleConstants => {
                            warn!(
                                "  ⚠️  ModuleConstants cycle detected - likely unresolvable due \
                                 to temporal paradox"
                            );
                        }
                        CircularDependencyType::ImportTime => {
                            warn!("  ⚠️  ImportTime cycle detected - depends on execution order");
                        }
                        CircularDependencyType::FunctionLevel => {
                            info!("  ✓ FunctionLevel cycle - should be safely resolvable");
                        }
                    }
                }

                warn!(
                    "Proceeding with bundling despite circular dependencies - output may require \
                     manual verification"
                );
                circular_dep_analysis = Some(analysis);
            }
        }

        // Run tree-shaking if enabled
        let tree_shaker = if self.config.tree_shake {
            info!("Running tree-shaking analysis...");
            let mut shaker = TreeShaker::from_graph(graph);

            // Check which modules can be tree-shaken (no side effects)
            let mut modules_with_side_effects = Vec::new();
            let mut modules_for_tree_shaking = Vec::new();

            for module in graph.modules.values() {
                if shaker.module_has_side_effects(&module.module_name) {
                    modules_with_side_effects.push(&module.module_name);
                } else {
                    modules_for_tree_shaking.push(&module.module_name);
                }
            }

            if !modules_with_side_effects.is_empty() {
                debug!(
                    "Modules with side effects (excluded from tree-shaking): \
                     {modules_with_side_effects:?}"
                );
            }

            if !modules_for_tree_shaking.is_empty() {
                debug!("Modules eligible for tree-shaking: {modules_for_tree_shaking:?}");
            }

            // Analyze from entry module
            shaker.analyze(&entry_module_name)?;

            // Log tree-shaking results
            for module_name in modules_for_tree_shaking {
                let unused = shaker.get_unused_symbols_for_module(module_name);
                if !unused.is_empty() {
                    info!(
                        "Tree-shaking will remove {} unused symbols from module '{}': {:?}",
                        unused.len(),
                        module_name,
                        unused
                    );
                }
            }

            Some(shaker)
        } else {
            None
        };

        // Set the resolver for the caller to use
        *resolver_opt = Some(resolver);

        Ok((
            entry_module_name,
            parsed_modules,
            circular_dep_analysis,
            tree_shaker,
        ))
    }

    /// Helper to get sorted modules from graph
    fn get_sorted_modules_from_graph(
        &self,
        graph: &CriboGraph,
        circular_dep_analysis: Option<&CircularDependencyAnalysis>,
    ) -> Result<Vec<(String, PathBuf, Vec<String>)>> {
        let module_ids = if let Some(analysis) = circular_dep_analysis {
            // We have circular dependencies but they're potentially resolvable
            // Use a custom ordering that attempts to break cycles
            self.get_modules_with_cycle_resolution(graph, analysis)?
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
                    .map_or_else(
                        || {
                            warn!("Module path not found for {name}, using name as fallback");
                            PathBuf::from(&name)
                        },
                        |(p, _)| p.clone(),
                    );

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
            debug!("Module {i}: {name} ({})", path.display());
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
        let (entry_module_name, parsed_modules, circular_dep_analysis, tree_shaker) =
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
        let bundled_code = self.emit_static_bundle(&StaticBundleParams {
            sorted_modules: &module_data,
            parsed_modules: Some(&parsed_modules),
            resolver: &resolver,
            entry_module_name: &entry_module_name,
            graph: &graph,
            circular_dep_analysis: circular_dep_analysis.as_ref(),
            tree_shaker: tree_shaker.as_ref(),
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
        debug!("Output: {}", output_path.display());

        // Initialize empty graph - resolver will be created in bundle_core
        let mut graph = CriboGraph::new();
        let mut resolver_opt = None;

        // Perform core bundling logic
        let (entry_module_name, parsed_modules, circular_dep_analysis, tree_shaker) =
            self.bundle_core(entry_path, &mut graph, &mut resolver_opt)?;

        // Extract the resolver (it's guaranteed to be Some after bundle_core)
        let resolver = resolver_opt.expect("Resolver should be initialized by bundle_core");

        let sorted_modules =
            self.get_sorted_modules_from_graph(&graph, circular_dep_analysis.as_ref())?;

        // Generate bundled code
        info!("Using hybrid static bundler");
        let bundled_code = self.emit_static_bundle(&StaticBundleParams {
            sorted_modules: &sorted_modules,
            parsed_modules: Some(&parsed_modules), // Use pre-parsed modules to avoid double parsing
            resolver: &resolver,
            entry_module_name: &entry_module_name,
            graph: &graph,
            circular_dep_analysis: circular_dep_analysis.as_ref(),
            tree_shaker: tree_shaker.as_ref(),
        })?;

        // Generate requirements.txt if requested
        if emit_requirements {
            self.write_requirements_file(&sorted_modules, &resolver, output_path)?;
        }

        // Write output file
        fs::write(output_path, bundled_code)
            .with_context(|| format!("Failed to write output file: {}", output_path.display()))?;

        info!("Bundle written to: {}", output_path.display());

        Ok(())
    }

    /// Get modules in a valid order for bundling when there are resolvable circular dependencies
    fn get_modules_with_cycle_resolution(
        &self,
        graph: &CriboGraph,
        analysis: &crate::analyzers::types::CircularDependencyAnalysis,
    ) -> Result<Vec<crate::cribo_graph::ModuleId>> {
        // For simple function-level cycles, we can use a modified topological sort
        // that breaks cycles by removing edges within strongly connected components

        // Get all module IDs
        let all_module_ids: Vec<_> = graph.modules.keys().copied().collect();

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
                crate::cribo_graph::ItemType::Import { module, .. }
                | crate::cribo_graph::ItemType::FromImport { module, .. } => {
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
            let Ok(relative_path) = entry_path.strip_prefix(src_dir) else {
                continue;
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

        // Store module data for phase 2 including parsed AST
        type DiscoveryData = (String, PathBuf, Vec<String>, ModModule, String); // (name, path, imports, ast, source) for discovery phase
        let mut discovered_modules: Vec<DiscoveryData> = Vec::new();

        // PHASE 1: Discover and collect all modules
        info!("Phase 1: Discovering all modules...");
        while let Some((module_name, module_path)) = modules_to_process.pop() {
            debug!(
                "Discovering module: {module_name} ({})",
                module_path.display()
            );
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

            // Process module through the pipeline (parse, semantic analysis, normalization)
            let processed = self.process_module(&module_path, &module_name, None)?;

            // Extract imports from the processed AST
            let imports_with_context =
                self.extract_imports_from_ast(&processed.ast, &module_path, Some(params.resolver))?;
            let imports: Vec<String> = imports_with_context
                .iter()
                .map(|(m, _, _, _)| m.clone())
                .collect();
            debug!("Extracted imports from {module_name}: {imports:?}");

            // Store module data including parsed AST for later processing
            discovered_modules.push((
                module_name.clone(),
                module_path.clone(),
                imports.clone(),
                processed.ast,
                processed.source,
            ));
            processed_modules.insert(module_name.clone());

            // Find and queue first-party imports for discovery
            for (import, is_in_error_handler, import_type, package_context) in imports_with_context
            {
                let mut discovery_params = DiscoveryParams {
                    resolver: params.resolver,
                    modules_to_process: &mut modules_to_process,
                    processed_modules: &processed_modules,
                    queued_modules: &mut queued_modules,
                };
                self.process_import_for_discovery_with_context(
                    &import,
                    is_in_error_handler,
                    import_type,
                    &package_context,
                    &mut discovery_params,
                )?;
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

        for (module_name, module_path, imports, _ast, _source) in discovered_modules {
            debug!("Phase 2: Processing module '{module_name}'");

            // Re-process the module WITH graph context this time
            // This will use cache but also add to graph and do semantic analysis
            let processed = self.process_module(&module_path, &module_name, Some(params.graph))?;

            let module_id = processed
                .module_id
                .expect("module_id should be set when graph provided");
            module_id_map.insert(module_name.clone(), module_id);
            debug!("Added module to graph: {module_name} with ID {module_id:?}");

            // Build dependency graph BEFORE no-ops removal
            if let Some(module) = params.graph.get_module_by_name_mut(&module_name) {
                let mut builder = crate::graph_builder::GraphBuilder::new(module);
                builder
                    .set_normalized_modules(processed.normalized_modules.iter().cloned().collect());
                builder.build_from_ast(&processed.ast)?;
            }

            // Store parsed module data for later use
            parsed_modules.push((
                module_name.clone(),
                module_path.clone(),
                imports.clone(),
                processed.ast,
                processed.source,
            ));
        }

        info!("Added {} modules to graph", params.graph.modules.len());

        // Then, add all dependency edges
        info!("Phase 2: Creating dependency edges...");
        for (module_name, _module_path, imports, _ast, _source) in &parsed_modules {
            let from_id = module_id_map.get(module_name).copied();
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

        // Aggregate __all__ access information from all modules
        let mut all_accesses = Vec::new();
        for module_graph in params.graph.modules.values() {
            for item in module_graph.items.values() {
                // Check attribute accesses for __all__
                for (base_name, attributes) in &item.attribute_accesses {
                    if attributes.contains("__all__") {
                        // This module accesses base_name.__all__
                        all_accesses.push((base_name.clone(), module_graph.module_name.clone()));
                        log::debug!(
                            "Module '{}' accesses {base_name}.__all__",
                            module_graph.module_name
                        );
                    }
                }
            }
        }

        // Now update the graph with the collected accesses
        for (base_name, accessing_module) in all_accesses {
            params
                .graph
                .add_module_accessing_all(base_name, accessing_module);
        }

        info!(
            "Phase 2 complete: dependency graph built with {} modules",
            params.graph.modules.len()
        );
        Ok(parsed_modules)
    }

    /// Extract imports from an already-parsed AST with full context information
    fn extract_imports_from_ast(
        &self,
        ast: &ModModule,
        file_path: &Path,
        mut resolver: Option<&ModuleResolver>,
    ) -> Result<ImportExtractionResult> {
        let mut visitor = ImportDiscoveryVisitor::new();
        for stmt in &ast.body {
            visitor.visit_stmt(stmt);
        }

        let discovered_imports = visitor.into_imports();
        debug!(
            "ImportDiscoveryVisitor found {} imports",
            discovered_imports.len()
        );
        if log::log_enabled!(log::Level::Trace) {
            for (i, import) in discovered_imports.iter().enumerate() {
                trace!(
                    "Import {}: type={:?}, module={:?}",
                    i, import.import_type, import.module_name
                );
            }
        }
        let mut imports_with_context = Vec::new();

        // Process each import and track if it's in an error-handling context
        for import in &discovered_imports {
            let is_in_error_handler = Self::is_import_in_error_handler(&import.location);

            // Handle ImportlibStatic imports
            if matches!(
                import.import_type,
                crate::visitors::ImportType::ImportlibStatic
            ) {
                let mut temp_set = IndexSet::new();
                self.process_importlib_static_import(import, &mut temp_set);
                for module_name in temp_set {
                    imports_with_context.push((
                        module_name,
                        is_in_error_handler,
                        Some(import.import_type),
                        import.package_context.clone(),
                    ));
                }
            } else if import.level > 0 {
                // Handle relative imports
                let mut imports_set = IndexSet::new();
                self.process_relative_import_set(
                    import,
                    file_path,
                    &mut resolver,
                    &mut imports_set,
                );
                for module in imports_set {
                    imports_with_context.push((module, is_in_error_handler, None, None));
                }
            } else if let Some(ref module_name) = import.module_name {
                // Absolute imports
                imports_with_context.push((module_name.clone(), is_in_error_handler, None, None));

                // Check if any imported names are actually submodules
                let mut imports_set = IndexSet::new();
                self.check_submodule_imports_set(
                    module_name,
                    import,
                    &mut resolver,
                    &mut imports_set,
                );
                for module in imports_set {
                    if module != *module_name {
                        imports_with_context.push((module, is_in_error_handler, None, None));
                    }
                }
            } else if import.names.len() == 1 {
                let mut imports_set = IndexSet::new();
                self.process_single_name_import_set(import, &mut resolver, &mut imports_set);
                for module in imports_set {
                    imports_with_context.push((module, is_in_error_handler, None, None));
                }
            }
        }

        Ok(imports_with_context)
    }

    /// Check if an import is in an error-handling context (try/except or with suppress)
    fn is_import_in_error_handler(location: &ImportLocation) -> bool {
        match location {
            ImportLocation::Nested(scopes) => {
                for scope in scopes {
                    match scope {
                        ScopeElement::Try => return true,
                        ScopeElement::With => {
                            // TODO: Ideally we'd check if it's specifically "with suppress"
                            // For now, assume any import in a with block might be suppressed
                            return true;
                        }
                        _ => {}
                    }
                }
                false
            }
            _ => false,
        }
    }

    /// Helper to process `ImportlibStatic` imports
    fn process_importlib_static_import(
        &self,
        import: &crate::visitors::DiscoveredImport,
        imports_set: &mut IndexSet<String>,
    ) {
        if let Some(ref module_name) = import.module_name {
            debug!("Found ImportlibStatic import: {module_name}");
            imports_set.insert(module_name.clone());
        }
    }

    /// Process relative imports and add to `IndexSet`
    fn process_relative_import_set(
        &self,
        import: &crate::visitors::DiscoveredImport,
        file_path: &Path,
        resolver: &mut Option<&ModuleResolver>,
        imports: &mut IndexSet<String>,
    ) {
        // Get resolver reference
        let resolver_ref = if let Some(resolver) = resolver {
            resolver
        } else {
            debug!("No resolver available for relative import resolution");
            return;
        };

        let base_module = match resolver_ref.resolve_relative_to_absolute_module_name(
            import.level,
            None, // Don't include module_name here, we'll handle it separately
            file_path,
        ) {
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
            // For "from . import X", check if X is actually a submodule
            // Note: We don't add the base module itself to avoid self-imports
            if let Some(resolver) = resolver {
                for (name, _) in &import.names {
                    let potential_submodule = format!("{base_module}.{name}");
                    // Only add if it's actually resolvable as a module
                    if resolver
                        .resolve_module_path(&potential_submodule)
                        .is_ok_and(|path| path.is_some())
                    {
                        imports.insert(potential_submodule);
                        debug!("Added verified submodule from relative import: {name}");
                    }
                }
            }
        }
    }

    /// Process a single name import that might be a submodule (`IndexSet` version)
    fn process_single_name_import_set(
        &self,
        import: &crate::visitors::DiscoveredImport,
        resolver: &mut Option<&ModuleResolver>,
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

    /// Check if any imported names are actually submodules (`IndexSet` version)
    fn check_submodule_imports_set(
        &self,
        module_name: &str,
        import: &crate::visitors::DiscoveredImport,
        resolver: &mut Option<&ModuleResolver>,
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
        if params.resolver.classify_import(parent_module) == ImportType::FirstParty {
            if let Ok(Some(parent_path)) = params.resolver.resolve_module_path(parent_module) {
                debug!(
                    "Adding parent package '{parent_module}' to discovery queue for import \
                     '{import}'"
                );
                self.add_to_discovery_queue_if_new(parent_module, parent_path, params);
            }
        } else {
            // Parent is not first-party, processing stops here
        }
    }

    /// Process an import during discovery phase with error handling context
    fn process_import_for_discovery_with_context(
        &self,
        import: &str,
        is_in_error_handler: bool,
        import_type: Option<crate::visitors::ImportType>,
        package_context: &Option<String>,
        params: &mut DiscoveryParams,
    ) -> Result<()> {
        // Special handling for ImportlibStatic imports that might have invalid Python identifiers
        if let Some(crate::visitors::ImportType::ImportlibStatic) = import_type {
            debug!("Processing ImportlibStatic import: {import}");

            // Try to resolve ImportlibStatic with package context
            if let Ok(Some((resolved_name, import_path))) = params
                .resolver
                .resolve_importlib_static_with_context(import, package_context.as_deref())
            {
                debug!(
                    "Resolved ImportlibStatic '{import}' to module '{resolved_name}' at path: {}",
                    import_path.display()
                );
                // Use the resolved name instead of the original import
                self.add_to_discovery_queue_if_new(&resolved_name, import_path, params);
            } else {
                // Try normal resolution in case it's a valid Python identifier
                match params.resolver.classify_import(import) {
                    ImportType::FirstParty => {
                        if let Ok(Some(import_path)) = params.resolver.resolve_module_path(import) {
                            debug!(
                                "Resolved ImportlibStatic '{import}' to path: {}",
                                import_path.display()
                            );
                            self.add_to_discovery_queue_if_new(import, import_path, params);
                        } else if !is_in_error_handler {
                            return Err(anyhow!(
                                "Failed to resolve ImportlibStatic module '{}'. \nThis import \
                                 would fail at runtime with: ModuleNotFoundError: No module named \
                                 '{}'",
                                import,
                                import
                            ));
                        }
                    }
                    _ => {
                        debug!("ImportlibStatic '{import}' classified as external (preserving)");
                    }
                }
            }
        } else {
            // Normal import handling
            match params.resolver.classify_import(import) {
                ImportType::FirstParty => {
                    debug!("'{import}' classified as FirstParty");
                    if let Ok(Some(import_path)) = params.resolver.resolve_module_path(import) {
                        debug!("Resolved '{import}' to path: {}", import_path.display());
                        self.add_to_discovery_queue_if_new(import, import_path, params);

                        // Also add parent packages for submodules to ensure __init__.py files are
                        // included For example, if importing
                        // "greetings.irrelevant", also add "greetings"
                        self.add_parent_packages_to_discovery(import, params);
                    } else {
                        // If the import is not in an error handler, this is a fatal error
                        if is_in_error_handler {
                            debug!(
                                "Failed to resolve first-party module '{import}' but it's in an \
                                 error handler (try/except or with suppress)"
                            );
                        } else {
                            return Err(anyhow!(
                                "Failed to resolve first-party module '{}'. \nThis import would \
                                 fail at runtime with: ModuleNotFoundError: No module named '{}'",
                                import,
                                import
                            ));
                        }
                    }
                }
                ImportType::ThirdParty | ImportType::StandardLibrary => {
                    debug!("'{import}' classified as external (preserving)");
                }
            }
        }
        Ok(())
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
        if requirements_content.is_empty() {
            info!("No third-party dependencies found, skipping requirements.txt");
        } else {
            let requirements_path = Path::new("requirements.txt");

            fs::write(requirements_path, requirements_content).with_context(|| {
                format!(
                    "Failed to write requirements file: {}",
                    requirements_path.display()
                )
            })?;

            info!("Requirements written to: {}", requirements_path.display());
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
        if requirements_content.is_empty() {
            info!("No third-party dependencies found, skipping requirements.txt");
        } else {
            let requirements_path = output_path
                .parent()
                .unwrap_or_else(|| Path::new("."))
                .join("requirements.txt");

            fs::write(&requirements_path, requirements_content).with_context(|| {
                format!(
                    "Failed to write requirements file: {}",
                    requirements_path.display()
                )
            })?;

            info!("Requirements written to: {}", requirements_path.display());
        }
        Ok(())
    }

    /// Emit bundle using static bundler (no exec calls)
    fn emit_static_bundle(&mut self, params: &StaticBundleParams<'_>) -> Result<String> {
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

        let mut static_bundler = Bundler::new(Some(&self.module_registry), params.resolver);

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
            // This fallback path should never be reached since we always pass pre-parsed modules
            return Err(anyhow!(
                "emit_static_bundle called without pre-parsed modules. This is a bug - all code \
                 paths should provide parsed_modules"
            ));
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
            let module_ast_pairs: Vec<(String, &ModModule)> = module_asts
                .iter()
                .map(|(name, ast, _, _)| (name.clone(), ast))
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
        let bundled_ast = static_bundler.bundle_modules(&crate::code_generator::BundleParams {
            modules: &module_asts,
            sorted_modules: params.sorted_modules,
            entry_module_name: params.entry_module_name,
            graph: params.graph,
            semantic_bundler: &self.semantic_bundler,
            circular_dep_analysis: params.circular_dep_analysis,
            tree_shaker: params.tree_shaker,
            python_version: self.config.python_version().unwrap_or(10),
        })?;

        // Generate Python code from AST
        let empty_parsed = get_empty_parsed_module();
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
