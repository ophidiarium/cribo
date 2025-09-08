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
    cribo_graph::CriboGraph,
    import_rewriter::{ImportDeduplicationStrategy, ImportRewriter},
    resolver::{ImportType, ModuleId, ModuleResolver},
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

        // Check if module already exists
        if let Some(existing) = self.modules.get(&id) {
            // For the same ModuleId, we allow different canonical names
            // (e.g., "__init__" and "yaml" for the same file)
            // but the path must be the same
            assert!(
                existing.resolved_path == path,
                "Attempting to register module {:?} with conflicting paths. Existing: {} at {}, \
                 New: {} at {}",
                id,
                existing.canonical_name,
                existing.resolved_path.display(),
                name,
                path.display()
            );

            // Add this new name as an alias for the same ModuleId
            self.name_to_id.insert(name, id);
            return; // Module already registered, just added new name mapping
        }

        self.name_to_id.insert(name, id);
        self.path_to_id.insert(path, id);
        self.modules.insert(id, info);
    }

    /// Get module ID by canonical name
    pub fn get_id_by_name(&self, name: &str) -> Option<ModuleId> {
        self.name_to_id.get(name).copied()
    }

    /// Check if a module exists by `ModuleId`
    pub fn contains_module(&self, id: &ModuleId) -> bool {
        self.modules.contains_key(id)
    }
}

/// Get or create the empty parsed module for Stylist creation
fn get_empty_parsed_module() -> &'static ruff_python_parser::Parsed<ModModule> {
    EMPTY_PARSED_MODULE
        .get_or_init(|| ruff_python_parser::parse_module("").expect("Failed to parse empty module"))
}

/// Type alias for module processing queue
type ModuleQueue = Vec<(ModuleId, PathBuf)>;
/// Type alias for processed modules set
type ProcessedModules = IndexSet<ModuleId>;
/// Type alias for parsed module data with AST and source
/// (`module_id`, imports, ast, source)
type ParsedModuleData = (crate::resolver::ModuleId, Vec<String>, ModModule, String);
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
    queued_modules: &'a mut IndexSet<ModuleId>,
}

/// Parameters for static bundle emission
struct StaticBundleParams<'a> {
    sorted_module_ids: &'a [crate::resolver::ModuleId],
    parsed_modules: Option<&'a [ParsedModuleData]>, // Optional pre-parsed modules
    resolver: &'a ModuleResolver,
    graph: &'a CriboGraph,
    circular_dep_analysis: Option<&'a CircularDependencyAnalysis>,
    tree_shaker: Option<&'a TreeShaker<'a>>,
}

/// Context for dependency building operations
struct DependencyContext<'a> {
    resolver: &'a ModuleResolver,
    graph: &'a mut CriboGraph,
    current_module_id: crate::resolver::ModuleId,
}

/// Parameters for graph building operations
struct GraphBuildParams<'a> {
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
    /// Module ID if already added to dependency graph
    module_id: Option<crate::resolver::ModuleId>,
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
        resolver: Option<&ModuleResolver>,
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
                    // Get or register module ID with resolver (required when graph is Some)
                    let resolver = resolver.expect("Resolver must be provided when graph is Some");
                    let module_id = resolver.register_module(module_name.to_string(), module_path);

                    graph.add_module(module_id, module_name.to_string(), module_path);

                    // Perform semantic analysis
                    self.semantic_bundler
                        .analyze_module(module_id, &cached.ast, module_path)?;

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
        let ast = parsed.into_syntax();

        // Step 2: Add to graph and perform semantic analysis (if graph provided)
        let module_id = if let Some(graph) = graph {
            // Get or register module ID with resolver (required when graph is Some)
            let resolver = resolver.expect("Resolver must be provided when graph is Some");
            let module_id = resolver.register_module(module_name.to_string(), module_path);

            graph.add_module(module_id, module_name.to_string(), module_path);

            // Semantic analysis on raw AST
            self.semantic_bundler
                .analyze_module(module_id, &ast, module_path)?;

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

        // Step 3: Cache the result
        let processed = ProcessedModule {
            ast: ast.clone(),
            source: source.clone(),
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
            module_id,
        })
    }

    /// Format error message for unresolvable cycles
    fn format_unresolvable_cycles_error(
        cycles: &[CircularDependencyGroup],
        resolver: &ModuleResolver,
    ) -> String {
        let mut error_msg = String::from("Unresolvable circular dependencies detected:\n\n");

        for (i, cycle) in cycles.iter().enumerate() {
            // Convert ModuleIds to names for display
            let module_names: Vec<String> = cycle
                .modules
                .iter()
                .filter_map(|id| resolver.get_module_name(*id))
                .collect();

            error_msg.push_str(&format!("Cycle {}: {}\n", i + 1, module_names.join(" → ")));
            error_msg.push_str(&format!("  Type: {:?}\n", cycle.cycle_type));

            if let ResolutionStrategy::Unresolvable { reason } = &cycle.suggested_resolution {
                error_msg.push_str(&format!("  Reason: {reason}\n"));
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
    )> {
        // Store the original entry path before transformation
        let original_entry_path = entry_path.to_path_buf();

        // Handle directory as entry point
        let entry_path = if entry_path.is_dir() {
            // Check for __main__.py first
            let main_py = entry_path.join(crate::python::constants::MAIN_FILE);
            if main_py.exists() && main_py.is_file() {
                info!(
                    "Using {} as entry point from directory: {}",
                    crate::python::constants::MAIN_FILE,
                    entry_path.display()
                );
                main_py
            } else {
                // Check for __init__.py
                let init_py = entry_path.join(crate::python::constants::INIT_FILE);
                if init_py.exists() && init_py.is_file() {
                    info!(
                        "Using {} as entry point from directory: {}",
                        crate::python::constants::INIT_FILE,
                        entry_path.display()
                    );
                    init_py
                } else {
                    return Err(anyhow!(
                        "Directory {} does not contain {} or {}",
                        entry_path.display(),
                        crate::python::constants::MAIN_FILE,
                        crate::python::constants::INIT_FILE
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
            let is_package_entry = crate::python::module_path::is_special_entry_file_name(filename);

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
        let mut resolver = ModuleResolver::new(self.config.clone());

        // Set the entry file to establish the primary search path
        resolver.set_entry_file(entry_path, &original_entry_path);

        // Find the entry module name
        let entry_module_name = self.find_entry_module_name(entry_path, &resolver)?;
        info!("Entry module: {entry_module_name}");

        // CRITICAL: Register the entry module FIRST to guarantee it gets ID 0
        // This is a fundamental invariant of our architecture
        let entry_id = resolver.register_module(entry_module_name.clone(), entry_path);
        assert_eq!(
            entry_id,
            crate::resolver::ModuleId::ENTRY,
            "Entry module must be ID 0 - bundling starts here"
        );

        // Build dependency graph
        let mut build_params = GraphBuildParams {
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
                let error_msg = Self::format_unresolvable_cycles_error(
                    &analysis.unresolvable_cycles,
                    &resolver,
                );
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
                    // Convert ModuleIds to module names for display
                    let module_names: Vec<String> = cycle
                        .modules
                        .iter()
                        .filter_map(|id| graph.modules.get(id).map(|m| m.module_name.clone()))
                        .collect();
                    warn!(
                        "Cycle {}: {} (Type: {:?})",
                        i + 1,
                        module_names.join(" → "),
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

        // Set the resolver for the caller to use
        *resolver_opt = Some(resolver);

        Ok((entry_module_name, parsed_modules, circular_dep_analysis))
    }

    /// Helper to get sorted modules from graph
    fn get_sorted_modules_from_graph(
        &self,
        graph: &CriboGraph,
        circular_dep_analysis: Option<&CircularDependencyAnalysis>,
    ) -> Result<Vec<crate::resolver::ModuleId>> {
        debug!(
            "get_sorted_modules_from_graph called with circular_dep_analysis: {}",
            circular_dep_analysis.is_some()
        );

        let module_ids = if let Some(analysis) = circular_dep_analysis {
            // We have circular dependencies but they're potentially resolvable
            // Use a custom ordering that attempts to break cycles
            debug!(
                "Using custom cycle resolution for {} cycles",
                analysis.resolvable_cycles.len() + analysis.unresolvable_cycles.len()
            );
            self.get_modules_with_cycle_resolution(graph, analysis)
        } else {
            debug!("Using standard topological sort");
            graph.topological_sort()?
        };

        // The topological sort already gives us the correct order for bundling:
        // dependencies come before dependents (modules are defined before they're used).
        // We do NOT need to reverse the order.

        debug!("Final module order (topologically sorted):");
        for &module_id in &module_ids {
            if let Some(module) = graph.modules.get(&module_id) {
                debug!("  - {}", module.module_name);
            }
        }

        info!("Found {} modules to bundle", module_ids.len());
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
        for (i, module_id) in module_ids.iter().enumerate() {
            if let Some(module) = graph.modules.get(module_id) {
                debug!(
                    "Position {}: {} (ModuleId({}))",
                    i, module.module_name, module_id.0
                );
            } else {
                debug!(
                    "Position {}: ModuleId({}) - NOT FOUND IN GRAPH",
                    i, module_id.0
                );
            }
        }
        debug!("=== END DEBUG ===");
        Ok(module_ids)
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
        let (_entry_module_name, parsed_modules, circular_dep_analysis) =
            self.bundle_core(entry_path, &mut graph, &mut resolver_opt)?;

        // Extract the resolver (it's guaranteed to be Some after bundle_core)
        let resolver = resolver_opt.expect("Resolver should be initialized by bundle_core");

        let sorted_module_ids =
            self.get_sorted_modules_from_graph(&graph, circular_dep_analysis.as_ref())?;

        // Optional: run tree-shaking after resolver is available
        let tree_shaker = if self.config.tree_shake {
            info!("Running tree-shaking analysis...");
            let mut shaker = crate::tree_shaking::TreeShaker::from_graph(&graph, &resolver);

            // Analyze from entry module (resolver guarantees ENTRY name is registered)
            // We use resolver to fetch it for logging and correctness where needed
            let entry_name = resolver
                .get_module_name(crate::resolver::ModuleId::ENTRY)
                .unwrap_or_else(|| "__main__".to_string());
            shaker.analyze(&entry_name);

            Some(shaker)
        } else {
            None
        };

        // Generate bundled code
        info!("Using hybrid static bundler");
        let bundled_code = self.emit_static_bundle(&StaticBundleParams {
            sorted_module_ids: &sorted_module_ids,
            parsed_modules: Some(&parsed_modules),
            resolver: &resolver,
            graph: &graph,
            circular_dep_analysis: circular_dep_analysis.as_ref(),
            tree_shaker: tree_shaker.as_ref(),
        })?;

        // Generate requirements.txt if requested
        if emit_requirements {
            self.write_requirements_file_for_stdout(&sorted_module_ids, &resolver, &graph)?;
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
        let (_entry_module_name, parsed_modules, circular_dep_analysis) =
            self.bundle_core(entry_path, &mut graph, &mut resolver_opt)?;

        // Extract the resolver (it's guaranteed to be Some after bundle_core)
        let resolver = resolver_opt.expect("Resolver should be initialized by bundle_core");

        let sorted_module_ids =
            self.get_sorted_modules_from_graph(&graph, circular_dep_analysis.as_ref())?;

        // Optional: run tree-shaking after resolver is available
        let tree_shaker = if self.config.tree_shake {
            info!("Running tree-shaking analysis...");
            let mut shaker = crate::tree_shaking::TreeShaker::from_graph(&graph, &resolver);

            let entry_name = resolver
                .get_module_name(crate::resolver::ModuleId::ENTRY)
                .unwrap_or_else(|| "__main__".to_string());
            shaker.analyze(&entry_name);

            Some(shaker)
        } else {
            None
        };

        // Generate bundled code
        info!("Using hybrid static bundler");
        let bundled_code = self.emit_static_bundle(&StaticBundleParams {
            sorted_module_ids: &sorted_module_ids,
            parsed_modules: Some(&parsed_modules), // Use pre-parsed modules to avoid double parsing
            resolver: &resolver,
            graph: &graph,
            circular_dep_analysis: circular_dep_analysis.as_ref(),
            tree_shaker: tree_shaker.as_ref(),
        })?;

        // Generate requirements.txt if requested
        if emit_requirements {
            self.write_requirements_file(&sorted_module_ids, &resolver, &graph, output_path)?;
        }

        // Write output file
        fs::write(output_path, bundled_code)
            .with_context(|| format!("Failed to write output file: {}", output_path.display()))?;

        info!("Bundle written to: {}", output_path.display());

        Ok(())
    }

    // Removed: unused helper `topologically_sort_modules` — we now sort via full-graph SCC
    // condensation.

    /// Get modules in a valid order for bundling when there are resolvable circular dependencies
    fn get_modules_with_cycle_resolution(
        &self,
        graph: &CriboGraph,
        _analysis: &crate::analyzers::types::CircularDependencyAnalysis,
    ) -> Vec<crate::resolver::ModuleId> {
        debug!("get_modules_with_cycle_resolution: computing SCC condensation over full graph");

        use petgraph::{
            algo::{tarjan_scc, toposort},
            graph::DiGraph,
            visit::{DfsPostOrder, EdgeRef},
        };

        // Build subgraph over ALL modules to maintain deps-before-dependents globally
        let all_module_ids: Vec<_> = graph.modules.keys().copied().collect();

        let mut subgraph: DiGraph<crate::resolver::ModuleId, ()> = DiGraph::new();
        let mut node_map: FxIndexMap<crate::resolver::ModuleId, petgraph::graph::NodeIndex> =
            FxIndexMap::default();
        for &id in &all_module_ids {
            node_map.insert(id, subgraph.add_node(id));
        }
        for &id in &all_module_ids {
            for dep in graph.get_dependencies(id) {
                if let (Some(&from), Some(&to)) = (node_map.get(&dep), node_map.get(&id))
                    && !subgraph.contains_edge(from, to)
                {
                    subgraph.add_edge(from, to, ());
                }
            }
        }

        // Compute SCCs on the full subgraph
        let sccs = tarjan_scc(&subgraph);

        // Map each node to its SCC index
        let mut node_to_scc: FxIndexMap<petgraph::graph::NodeIndex, usize> = FxIndexMap::default();
        for (i, comp) in sccs.iter().enumerate() {
            for &n in comp {
                node_to_scc.insert(n, i);
            }
        }

        // Deterministic rank using discovery order from all_module_ids
        let mut rank: FxIndexMap<crate::resolver::ModuleId, usize> = FxIndexMap::default();
        for (i, &mid) in all_module_ids.iter().enumerate() {
            rank.insert(mid, i);
        }

        // Build condensation DAG of components
        let mut comp_graph: DiGraph<usize, ()> = DiGraph::new();
        let mut comp_node_map: FxIndexMap<usize, petgraph::graph::NodeIndex> =
            FxIndexMap::default();

        // Insert components in deterministic order by minimal member rank
        let mut comp_indices: Vec<usize> = (0..sccs.len()).collect();
        comp_indices.sort_by_key(|&cid| {
            sccs[cid]
                .iter()
                .map(|&nx| rank.get(&subgraph[nx]).copied().unwrap_or(usize::MAX))
                .min()
                .unwrap_or(usize::MAX)
        });
        for cid in comp_indices.iter().copied() {
            comp_node_map.insert(cid, comp_graph.add_node(cid));
        }

        // Add edges between components (dependency -> dependent)
        for edge in subgraph.edge_references() {
            let u = edge.source();
            let v = edge.target();
            let cu = node_to_scc[&u];
            let cv = node_to_scc[&v];
            if cu != cv {
                let from = comp_node_map[&cu];
                let to = comp_node_map[&cv];
                if !comp_graph.contains_edge(from, to) {
                    comp_graph.add_edge(from, to, ());
                }
            }
        }

        // Topologically order components
        let comp_order = match toposort(&comp_graph, None) {
            Ok(nodes) => nodes.into_iter().map(|n| comp_graph[n]).collect::<Vec<_>>(),
            Err(_) => {
                // Condensation should be a DAG; if not, fall back to insertion order
                comp_indices
            }
        };

        // Emit modules: singleton SCCs directly; multi-node SCCs with stable DFS-post-order
        let mut visited = IndexSet::new();
        let mut result = Vec::with_capacity(all_module_ids.len());
        for cid in comp_order {
            let comp_nodes = &sccs[cid];
            if comp_nodes.len() == 1 {
                let mid = subgraph[comp_nodes[0]];
                if visited.insert(mid) {
                    result.push(mid);
                }
                continue;
            }

            // Build a mini-subgraph containing only nodes in this component
            let mut mini: DiGraph<crate::resolver::ModuleId, ()> = DiGraph::new();
            let mut mini_map: FxIndexMap<petgraph::graph::NodeIndex, petgraph::graph::NodeIndex> =
                FxIndexMap::default();

            // Add nodes with deterministic order (by rank)
            let mut comp_sorted = comp_nodes.clone();
            comp_sorted.sort_by_key(|&nx| rank.get(&subgraph[nx]).copied().unwrap_or(usize::MAX));
            for &nx in &comp_sorted {
                let mid = subgraph[nx];
                let idx = mini.add_node(mid);
                mini_map.insert(nx, idx);
            }

            // Add edges among component nodes
            for &nx in &comp_sorted {
                let from_idx = mini_map[&nx];
                for edge in subgraph.edges(nx) {
                    let tgt = edge.target();
                    if let Some(&to_idx) = mini_map.get(&tgt)
                        && !mini.contains_edge(to_idx, from_idx)
                    {
                        // Reverse orientation: dependent -> dependency
                        // so DfsPostOrder emits dependencies before dependents
                        mini.add_edge(to_idx, from_idx, ());
                    }
                }
            }

            // Traverse the mini-subgraph to ensure dependency-first order within the SCC
            for &nx in &comp_sorted {
                if let Some(&start) = mini_map.get(&nx) {
                    let mut dfs = DfsPostOrder::new(&mini, start);
                    while let Some(nid) = dfs.next(&mini) {
                        let mid = mini[nid];
                        if visited.insert(mid) {
                            result.push(mid);
                        }
                    }
                }
            }
        }

        // Debug log resulting order
        debug!("Resolved module order (including cycles):");
        for &module_id in &result {
            if let Some(module) = graph.modules.get(&module_id) {
                debug!("  - {}", module.module_name);
            }
        }

        result
    }

    /// Extract imports from module items
    fn extract_imports_from_module_items(
        &self,
        items: &crate::types::FxIndexMap<crate::cribo_graph::ItemId, crate::cribo_graph::ItemData>,
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
        log::debug!("find_module_in_src_dirs: src dirs = {:?}", self.config.src);
        for src_dir in &self.config.src {
            log::debug!("Checking if {entry_path:?} starts with {src_dir:?}");
            let Ok(relative_path) = entry_path.strip_prefix(src_dir) else {
                continue;
            };
            log::debug!("Relative path: {relative_path:?}");
            if let Some(module_name) = self.path_to_module_name(relative_path) {
                log::debug!("Module name from relative path: {module_name}");
                return Some(module_name);
            }
        }
        log::debug!("No module name found in src dirs");
        None
    }

    /// Find the module name for the entry script
    fn find_entry_module_name(
        &self,
        entry_path: &Path,
        _resolver: &ModuleResolver,
    ) -> Result<String> {
        log::debug!("find_entry_module_name: entry_path = {entry_path:?}");

        // Special case: If the entry is __init__.py, always use __init__ as the module name
        // to avoid conflicts with wrapper modules that might have the same name as the package
        if entry_path
            .file_name()
            .and_then(|f| f.to_str())
            .is_some_and(crate::python::module_path::is_init_file_name)
        {
            log::debug!(
                "Entry is {}, using '{}' as module name",
                crate::python::constants::INIT_FILE,
                crate::python::constants::INIT_STEM
            );
            return Ok(crate::python::constants::INIT_STEM.to_string());
        }

        // Try to find which src directory contains the entry file
        if let Some(module_name) = self.find_module_in_src_dirs(entry_path) {
            log::debug!("Found module name from src dirs: {module_name}");
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

        log::debug!("Using file stem as module name: {module_name}");
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
        // Get entry module information from resolver
        let entry_path = params
            .resolver
            .get_module_path(ModuleId::ENTRY)
            .expect("Entry module must have a path");

        let mut queued_modules = IndexSet::new();
        let mut modules_to_process = ModuleQueue::new();
        modules_to_process.push((ModuleId::ENTRY, entry_path));
        queued_modules.insert(ModuleId::ENTRY);

        // Store module data for phase 2 including parsed AST
        type DiscoveryData = (ModuleId, PathBuf, Vec<String>, ModModule, String); // (id, path, imports, ast, source) for discovery phase
        let mut discovered_modules: Vec<DiscoveryData> = Vec::new();

        // PHASE 1: Discover and collect all modules
        info!("Phase 1: Discovering all modules...");
        while let Some((module_id, module_path)) = modules_to_process.pop() {
            let module_name = params
                .resolver
                .get_module_name(module_id)
                .unwrap_or_else(|| format!("module_{}", module_id.as_u32()));
            debug!(
                "Discovering module: {module_name} ({})",
                module_path.display()
            );

            // Check if this is a namespace package (directory without __init__.py)
            if module_path.is_dir() {
                debug!("Module {module_name} is a namespace package (directory), skipping");
                // Don't track namespace packages as they have no code
                continue;
            }

            // Process module through the pipeline (parse, semantic analysis, normalization)
            let processed =
                self.process_module(&module_path, &module_name, None, Some(params.resolver))?;

            // Extract imports from the processed AST
            let imports_with_context =
                self.extract_imports_from_ast(&processed.ast, &module_path, Some(params.resolver));
            let imports: Vec<String> = imports_with_context
                .iter()
                .map(|(m, _, _, _)| m.clone())
                .collect();
            debug!("Extracted imports from {module_name}: {imports:?}");

            // Store module data including parsed AST for later processing
            discovered_modules.push((
                module_id,
                module_path.clone(),
                imports.clone(),
                processed.ast,
                processed.source,
            ));
            processed_modules.insert(module_id);

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
        let mut parsed_modules: Vec<ParsedModuleData> = Vec::new();

        for (discovered_module_id, module_path, imports, _ast, _source) in discovered_modules {
            let module_name = params
                .resolver
                .get_module_name(discovered_module_id)
                .unwrap_or_else(|| format!("module_{}", discovered_module_id.as_u32()));
            debug!("Phase 2: Processing module '{module_name}'");

            // Re-process the module WITH graph context this time
            // This will use cache but also add to graph and do semantic analysis
            let processed = self.process_module(
                &module_path,
                &module_name,
                Some(params.graph),
                Some(params.resolver),
            )?;

            let module_id = processed
                .module_id
                .expect("module_id should be set when graph provided");
            debug!("Added module to graph: {module_name} with ID {module_id:?}");

            // Build dependency graph BEFORE no-ops removal
            if let Some(module) = params.graph.get_module_by_name_mut(&module_name) {
                let python_version = self.config.python_version().unwrap_or(10);
                let mut builder = crate::graph_builder::GraphBuilder::new(module, python_version);
                // No longer setting normalized_modules as we handle stdlib normalization later
                builder.build_from_ast(&processed.ast)?;
            }

            // Store parsed module data for later use
            parsed_modules.push((module_id, imports.clone(), processed.ast, processed.source));
        }

        info!("Added {} modules to graph", params.graph.modules.len());

        // Then, add all dependency edges
        info!("Phase 2: Creating dependency edges...");
        for (module_id, imports, _ast, _source) in &parsed_modules {
            for import in imports {
                let mut context = DependencyContext {
                    resolver: params.resolver,
                    graph: params.graph,
                    current_module_id: *module_id,
                };
                self.process_import_for_dependency(import, &mut context);
            }
        }

        // Aggregate __all__ access information from all modules
        let mut all_accesses = Vec::new();
        for (accessing_module_id, module_graph) in &params.graph.modules {
            for item in module_graph.items.values() {
                // Check attribute accesses for __all__
                for (base_name, attributes) in &item.attribute_accesses {
                    if attributes.contains("__all__") {
                        // Resolve the base_name to the actual module if it's an alias
                        let resolved_module_name = module_graph
                            .items
                            .values()
                            .find_map(|i| match &i.item_type {
                                crate::cribo_graph::ItemType::Import { module, alias }
                                    if alias.as_deref() == Some(base_name) =>
                                {
                                    Some(module.clone())
                                }
                                _ => None,
                            })
                            .unwrap_or_else(|| base_name.clone());

                        // Try to resolve the accessed module name to a ModuleId
                        if let Some(accessed_module) =
                            params.graph.get_module_by_name(&resolved_module_name)
                        {
                            // This module accesses resolved_module.__all__
                            all_accesses.push((*accessing_module_id, accessed_module.module_id));
                            log::debug!(
                                "Module '{}' (ID {:?}) accesses {}.__all__ (ID {:?}, resolved \
                                 from alias '{base_name}')",
                                module_graph.module_name,
                                accessing_module_id,
                                resolved_module_name,
                                accessed_module.module_id
                            );
                        } else {
                            log::debug!(
                                "Could not resolve module '{}' to ID when tracking __all__ access \
                                 from '{}'",
                                resolved_module_name,
                                module_graph.module_name
                            );
                        }
                    }
                }

                // Note: Do not treat wildcard imports as implicit __all__ access globally.
                // Runtime reflection patterns are handled locally in namespace population
                // via heuristics (wildcard import + setattr), avoiding unnecessary __all__
                // assignments that cause snapshot churn.
            }
        }

        // Now update the graph with the collected accesses
        for (accessing_module_id, accessed_module_id) in all_accesses {
            params
                .graph
                .add_module_accessing_all(accessing_module_id, accessed_module_id);
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
    ) -> ImportExtractionResult {
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

        imports_with_context
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
        // Register the module with resolver to get its ID
        let module_id = discovery_params
            .resolver
            .register_module(import.to_string(), &import_path);

        if !discovery_params.processed_modules.contains(&module_id)
            && !discovery_params.queued_modules.contains(&module_id)
        {
            debug!(
                "Adding '{import}' (ID: {}) to discovery queue",
                module_id.as_u32()
            );
            discovery_params
                .modules_to_process
                .push((module_id, import_path));
            discovery_params.queued_modules.insert(module_id);
        } else {
            debug!(
                "Module '{import}' (ID: {}) already processed or queued, skipping",
                module_id.as_u32()
            );
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
            if let Some((resolved_name, import_path)) = params
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
                if let Some(to_module_id) = context.resolver.get_module_id_by_name(import) {
                    debug!(
                        "Adding dependency edge: module_id_{} -> {} (to: module_id_{})",
                        context.current_module_id.as_u32(),
                        import,
                        to_module_id.as_u32()
                    );
                    // TODO: Properly track TYPE_CHECKING information from ImportDiscoveryVisitor
                    // For now, we use the default (is_type_checking_only = false)
                    // This should be updated to use the actual is_type_checking_only flag from
                    // the DiscoveredImport when we refactor to preserve that information
                    context
                        .graph
                        .add_module_dependency(context.current_module_id, to_module_id);
                    debug!(
                        "Successfully added dependency edge: module_id_{} -> {} (to: module_id_{})",
                        context.current_module_id.as_u32(),
                        import,
                        to_module_id.as_u32()
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
        if context.resolver.classify_import(parent_module) == ImportType::FirstParty
            && let Some(parent_module_id) = context.resolver.get_module_id_by_name(parent_module)
        {
            // Skip if parent_module is the same as current module to avoid self-dependencies
            if parent_module_id == context.current_module_id {
                debug!(
                    "Skipping self-dependency: {} -> module_id_{}",
                    parent_module,
                    context.current_module_id.as_u32()
                );
                return;
            }

            debug!(
                "Adding parent package dependency edge: {} -> module_id_{}",
                parent_module,
                context.current_module_id.as_u32()
            );
            // TODO: Inherit TYPE_CHECKING information from child import
            context
                .graph
                .add_module_dependency(context.current_module_id, parent_module_id);
        }
    }

    /// Write requirements.txt file for stdout mode (current directory)
    fn write_requirements_file_for_stdout(
        &self,
        sorted_module_ids: &[crate::resolver::ModuleId],
        resolver: &ModuleResolver,
        graph: &CriboGraph,
    ) -> Result<()> {
        let requirements_content = self.generate_requirements(sorted_module_ids, resolver, graph);
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
        sorted_module_ids: &[crate::resolver::ModuleId],
        resolver: &ModuleResolver,
        graph: &CriboGraph,
        output_path: &Path,
    ) -> Result<()> {
        let requirements_content = self.generate_requirements(sorted_module_ids, resolver, graph);
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
            for (module_id, _imports, ast, source) in parsed_modules {
                // Calculate content hash for deterministic module naming
                use sha2::{Digest, Sha256};
                let mut hasher = Sha256::new();
                hasher.update(source.as_bytes());
                let hash = hasher.finalize();
                let content_hash = format!("{hash:x}");

                module_asts.push((*module_id, ast.clone(), content_hash));
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
            let module_ast_map: crate::types::FxIndexMap<crate::resolver::ModuleId, &ModModule> =
                module_asts
                    .iter()
                    .map(|(id, ast, _)| (*id, ast as &ModModule))
                    .collect();

            // Analyze movable imports using semantic analysis
            let movable_imports = import_rewriter.analyze_movable_imports_semantic(
                params.graph,
                &analysis.resolvable_cycles,
                &self.semantic_bundler,
                &module_ast_map,
            );

            debug!(
                "Found {} imports that can be moved to function scope using semantic analysis",
                movable_imports.len()
            );

            // Apply rewriting to each module AST
            for (module_id, ast, _) in &mut module_asts {
                import_rewriter.rewrite_module(ast, &movable_imports, *module_id);
            }
        }

        // Bundle all modules using static bundler
        let bundled_ast = static_bundler.bundle_modules(&crate::code_generator::BundleParams {
            modules: &module_asts,
            sorted_module_ids: params.sorted_module_ids,
            resolver: params.resolver,
            graph: params.graph,
            semantic_bundler: &self.semantic_bundler,
            circular_dep_analysis: params.circular_dep_analysis,
            tree_shaker: params.tree_shaker,
            python_version: self.config.python_version().unwrap_or(10),
        });

        // Generate Python code from AST
        let empty_parsed = get_empty_parsed_module();
        let stylist = ruff_python_codegen::Stylist::from_tokens(empty_parsed.tokens(), "");

        log::trace!("Bundled AST has {} statements", bundled_ast.body.len());
        if !bundled_ast.body.is_empty() {
            log::trace!(
                "First statement type in bundled AST: {:?}",
                std::mem::discriminant(&bundled_ast.body[0])
            );
        }

        let mut code_parts = Vec::new();
        for (i, stmt) in bundled_ast.body.iter().enumerate() {
            if i < 3 {
                log::trace!(
                    "Processing statement {}: type = {:?}",
                    i,
                    std::mem::discriminant(stmt)
                );
            }
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
        module_ids: &[crate::resolver::ModuleId],
        resolver: &ModuleResolver,
        graph: &CriboGraph,
    ) -> String {
        let mut third_party_imports = IndexSet::new();

        // TODO: Use TYPE_CHECKING information from the dependency graph to filter out
        // dependencies that are only used for type checking. These could be placed
        // in a separate section or excluded entirely based on configuration.
        // For now, all third-party imports are included.

        for module_id in module_ids {
            if let Some(module) = graph.modules.get(module_id) {
                let imports = self.extract_imports_from_module_items(&module.items);
                for import in &imports {
                    debug!("Checking import '{import}' for requirements");
                    if let ImportType::ThirdParty = resolver.classify_import(import) {
                        // Map the import name to the actual package name
                        // This handles cases like "markdown_it" -> "markdown-it-py"
                        let package_name = resolver.map_import_to_package_name(import);
                        debug!("Adding '{package_name}' to requirements (from '{import}')");
                        third_party_imports.insert(package_name);
                    }
                }
            }
        }

        let mut requirements: Vec<String> = third_party_imports.into_iter().collect();
        requirements.sort();

        requirements.join("\n")
    }
}
