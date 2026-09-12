use std::{
    fmt::Write,
    fs,
    path::{Path, PathBuf},
    sync::{Arc, OnceLock},
};

use anyhow::{Context, Result, anyhow};
use indexmap::{IndexMap, IndexSet};
use log::{debug, info, trace, warn};
use ruff_python_ast::ModModule;

use crate::{
    analyzers::types::{
        CircularDependencyAnalysis, CircularDependencyGroup, CircularDependencyType,
        ResolutionStrategy,
    },
    code_generator::{Bundler, phases::orchestrator::PhaseOrchestrator},
    config::{Config, SourceMapMode},
    dependency_graph::DependencyGraph,
    import_rewriter::{ImportDeduplicationStrategy, ImportRewriter},
    module_facts::ModuleFacts,
    requirement_resolver::RequirementResolver,
    resolver::{ImportOrigin, ModuleId, ModuleResolver},
    source_map::{ProvenanceResolver, SourceMapOptions, build_source_map},
    symbol_conflict_resolver::SymbolConflictResolver,
    tree_shaking::TreeShaker,
    types::FxIndexMap,
    util::{module_name_from_relative, normalize_line_endings},
    visitors::{DiscoveredImport, ImportLocation, ScopeElement},
};

/// Static empty parsed module for creating Stylist instances
static EMPTY_PARSED_MODULE: OnceLock<ruff_python_parser::Parsed<ModModule>> = OnceLock::new();

/// Get or create the empty parsed module for Stylist creation
fn get_empty_parsed_module() -> &'static ruff_python_parser::Parsed<ModModule> {
    EMPTY_PARSED_MODULE
        .get_or_init(|| ruff_python_parser::parse_module("").expect("Failed to parse empty module"))
}

/// Path of the source map file for a bundle output path (`bundle.py` → `bundle.py.map`).
fn source_map_path_for(output_path: &Path) -> PathBuf {
    let mut file_name = output_path.file_name().map_or_else(
        || std::ffi::OsString::from("bundle.py"),
        std::ffi::OsStr::to_os_string,
    );
    file_name.push(".map");
    output_path.with_file_name(file_name)
}

/// Collision-resistant, unpredictable suffix for a staged map file.
///
/// In a shared writable directory a predictable temp name (e.g. PID-derived)
/// could be pre-created by another local user, turning every `create_new`
/// attempt into a denial of service. The suffix hashes process identity, an
/// ASLR-randomized stack address, wall-clock nanoseconds, and the attempt
/// counter — not guessable ahead of time, and fresh entropy per retry. This
/// names a transient file only; deterministic-output rules are unaffected.
fn staging_suffix(attempt: u32) -> String {
    use sha2::{Digest as _, Sha256};

    let stack_probe = 0_u8;
    let mut hasher = Sha256::new();
    hasher.update(std::process::id().to_le_bytes());
    hasher.update(attempt.to_le_bytes());
    hasher.update((&raw const stack_probe as usize).to_le_bytes());
    if let Ok(elapsed) = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
        hasher.update(elapsed.as_secs().to_le_bytes());
        hasher.update(elapsed.subsec_nanos().to_le_bytes());
    }
    let digest = hasher.finalize();
    let mut hex = String::with_capacity(16);
    for byte in &digest[..8] {
        write!(hex, "{byte:02x}").expect("Writing to String never fails");
    }
    hex
}

/// Stage the source map into a collision-resistant temp file next to `map_path`.
///
/// The file is opened with `create_new` (`O_EXCL` semantics), which neither
/// follows a pre-planted symlink nor truncates an existing file — a requirement
/// for outputs in shared sticky directories, where a temp name could otherwise
/// be predicted and pointed elsewhere. Names are unpredictable (see
/// [`staging_suffix`]) and collisions retry with fresh entropy, which also
/// keeps concurrent builds from stomping each other's staged map.
///
/// The staged file is born `0600` on Unix (mode set atomically at `open(2)`
/// time), so no other user can grab a readable handle between creation and a
/// later `chmod`. When a previous map exists *and is owned by this user*, its
/// permissions are then copied onto the staged file before any content is
/// written, so a restricted map (e.g. 0600 protecting `sourcesContent`) stays
/// restricted across rebuilds — while a foreign pre-created file in a shared
/// sticky directory cannot force a permissive mode. Otherwise the file keeps
/// `0600`; after the rename the map is owner-only by default, and users who
/// want it world-readable can `chmod` it once — the permissions persist
/// across subsequent rebuilds.
fn stage_map_file(map_path: &Path, map_json: &str) -> std::io::Result<PathBuf> {
    use std::io::Write as _;

    /// Owning uid on Unix; constant elsewhere (no ownership model to check).
    #[cfg(unix)]
    fn uid_of(metadata: &fs::Metadata) -> u32 {
        use std::os::unix::fs::MetadataExt as _;
        metadata.uid()
    }
    #[cfg(not(unix))]
    fn uid_of(_metadata: &fs::Metadata) -> u32 {
        0
    }

    let base_name = map_path.file_name().map_or_else(
        || std::ffi::OsString::from("bundle.py.map"),
        std::ffi::OsStr::to_os_string,
    );
    let mut attempt = 0_u32;
    loop {
        let mut tmp_name = base_name.clone();
        tmp_name.push(format!(".{}.tmp", staging_suffix(attempt)));
        let tmp_path = map_path.with_file_name(tmp_name);
        let mut open_options = fs::OpenOptions::new();
        open_options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            open_options.mode(0o600);
        }
        match open_options.open(&tmp_path) {
            Ok(mut file) => {
                // symlink_metadata: an attacker-planted symlink at the map
                // path must not decide the staged file's permissions. On Unix
                // the previous map must also be owned by the same uid as the
                // staged file (i.e. this process): in a shared sticky
                // directory anyone can pre-create a world-readable file at
                // the predictable final path, and inheriting its mode would
                // expose `sourcesContent` before the rename even fails.
                let staged_uid = uid_of(&file.metadata()?);
                let write_result = fs::symlink_metadata(map_path)
                    .ok()
                    .filter(fs::Metadata::is_file)
                    .filter(|metadata| uid_of(metadata) == staged_uid)
                    .map_or(Ok(()), |metadata| {
                        file.set_permissions(metadata.permissions())
                    })
                    .and_then(|()| file.write_all(map_json.as_bytes()))
                    .and_then(|()| file.flush());
                if let Err(err) = write_result {
                    drop(file);
                    let _ = fs::remove_file(&tmp_path);
                    return Err(err);
                }
                return Ok(tmp_path);
            }
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists && attempt < 1024 => {
                attempt += 1;
            }
            Err(err) => return Err(err),
        }
    }
}

/// Type alias for module processing queue
type ModuleQueue = Vec<(ModuleId, PathBuf)>;
/// Type alias for processed modules set
type ProcessedModules = IndexSet<ModuleId>;
/// Type alias for parsed module data with AST and source
/// (`module_id`, imports, ast, source)
type ParsedModuleData = (ModuleId, Vec<String>, ModModule, String);
/// Type alias for import extraction result
type ImportExtractionItem = (
    String,
    bool,
    Option<crate::visitors::ImportType>,
    Option<String>,
);
type ImportExtractionResult = Vec<ImportExtractionItem>;

/// Parameters for discovery phase operations
struct DiscoveryParams<'a> {
    resolver: &'a ModuleResolver,
    modules_to_process: &'a mut ModuleQueue,
    processed_modules: &'a ProcessedModules,
    queued_modules: &'a mut IndexSet<ModuleId>,
}

/// Parameters for static bundle emission
struct StaticBundleParams<'a> {
    sorted_module_ids: &'a [ModuleId],
    parsed_modules: Option<&'a [ParsedModuleData]>, // Optional pre-parsed modules
    resolver: &'a ModuleResolver,
    graph: &'a DependencyGraph,
    circular_dep_analysis: Option<&'a CircularDependencyAnalysis>,
    tree_shaker: Option<&'a TreeShaker<'a>>,
    /// Output file path when writing to disk; `None` for stdout output.
    /// Used for the source map `file` field and source path relativization.
    output_path: Option<&'a Path>,
}

/// Result of static bundle emission: the code plus an optional source map JSON.
struct EmittedBundle {
    code: String,
    /// Source Map v3 JSON, present when `Config::sourcemap` is enabled.
    source_map: Option<String>,
}

/// Context for dependency building operations
struct DependencyContext<'a> {
    resolver: &'a ModuleResolver,
    graph: &'a mut DependencyGraph,
    current_module_id: ModuleId,
}

/// Parameters for graph building operations
struct GraphBuildParams<'a> {
    resolver: &'a ModuleResolver,
    graph: &'a mut DependencyGraph,
}

/// Result of the AST processing pipeline
#[derive(Clone, Debug)]
struct ProcessedModule {
    /// The transformed AST after all pipeline stages
    ast: ModModule,
    /// The original source code (needed for semantic analysis and code generation)
    source: String,
    /// Shared module facts reused by discovery and later graph-driven analyses.
    facts: Arc<ModuleFacts>,
}

/// Main orchestrator for bundling operations
/// Note: Made `pub` for benchmark access via lib.rs (benchmarks are part of public API surface)
#[derive(Debug)]
#[allow(unreachable_pub)]
pub struct BundleOrchestrator {
    config: Config,
    conflict_resolver: SymbolConflictResolver,
    /// Cache of processed modules to ensure we only parse and transform once
    module_cache: std::sync::Mutex<FxIndexMap<PathBuf, ProcessedModule>>,
    /// Static `importlib.import_module` targets that stayed external during discovery;
    /// they never enter the module graph as imports, but their distributions must
    /// still reach requirements generation
    external_importlib_targets: std::sync::Mutex<crate::types::FxIndexSet<String>>,
    /// Absolute literal targets of preserved `import_module` calls (arguments not
    /// safely discardable): the call executes as a real runtime import even when the
    /// target is also bundled, so its distribution must stay installed
    preserved_importlib_targets: std::sync::Mutex<crate::types::FxIndexSet<String>>,
}

impl BundleOrchestrator {
    #[allow(unreachable_pub)]
    pub fn new(config: Config) -> Self {
        Self {
            config,
            conflict_resolver: SymbolConflictResolver::new(),
            module_cache: std::sync::Mutex::new(FxIndexMap::default()),
            external_importlib_targets: std::sync::Mutex::new(crate::types::FxIndexSet::default()),
            preserved_importlib_targets: std::sync::Mutex::new(crate::types::FxIndexSet::default()),
        }
    }

    /// Read access to the effective configuration.
    pub(crate) const fn config(&self) -> &Config {
        &self.config
    }

    /// Override the third-party bundling policy (used by dependency detection,
    /// which must always keep third-party imports external).
    pub(crate) const fn set_bundle_third_party(&mut self, value: bool) {
        self.config.bundle_third_party = Some(value);
    }

    /// Static `importlib.import_module` targets recorded during the last discovery
    /// run that must reach requirements generation (external targets plus preserved
    /// runtime calls).
    pub(crate) fn importlib_requirement_targets(&self) -> Vec<String> {
        let mut targets: Vec<String> = self
            .external_importlib_targets
            .lock()
            .expect("external importlib targets lock poisoned")
            .iter()
            .cloned()
            .collect();
        for target in self
            .preserved_importlib_targets
            .lock()
            .expect("preserved importlib targets lock poisoned")
            .iter()
        {
            if !targets.contains(target) {
                targets.push(target.clone());
            }
        }
        targets
    }

    /// Single entry point for parsing and processing modules
    /// This is THE ONLY place where `ruff_python_parser::parse_module` should be called
    ///
    /// Pipeline:
    /// 1. Check cache
    /// 2. Read file and parse
    /// 3. Derive reusable module facts
    /// 4. Cache parse products
    fn process_module(&self, module_path: &Path, module_name: &str) -> Result<ProcessedModule> {
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
            return Ok(ProcessedModule {
                ast: cached.ast.clone(),
                source: cached.source.clone(),
                facts: Arc::clone(&cached.facts),
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
        let python_version = self.config.python_version().unwrap_or(10);
        let facts = Arc::new(ModuleFacts::from_ast(&ast, python_version)?);

        // Step 2: Cache the parse products. Identity and graph lifecycle are owned by
        // the resolver and dependency-graph build phase, respectively.
        let processed = ProcessedModule {
            ast: ast.clone(),
            source: source.clone(),
            facts: Arc::clone(&facts),
        };

        {
            let mut cache = self
                .module_cache
                .lock()
                .expect("Failed to acquire module cache lock");
            cache.insert(canonical_path, processed);
        }

        Ok(ProcessedModule { ast, source, facts })
    }

    /// Return the cached module facts for a previously processed module path.
    ///
    /// Facts are populated by [`Self::process_module`] during dependency-graph
    /// construction, so every module registered in the graph has an entry.
    pub(crate) fn cached_module_facts(&self, module_path: &Path) -> Option<Arc<ModuleFacts>> {
        let canonical_path = module_path
            .canonicalize()
            .unwrap_or_else(|_| module_path.to_path_buf());
        let cache = self
            .module_cache
            .lock()
            .expect("Failed to acquire module cache lock");
        cache
            .get(&canonical_path)
            .map(|processed| Arc::clone(&processed.facts))
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

            writeln!(error_msg, "Cycle {}: {}", i + 1, module_names.join(" → "))
                .expect("Writing to String never fails");
            writeln!(error_msg, "  Type: {:?}", cycle.cycle_type)
                .expect("Writing to String never fails");

            if let ResolutionStrategy::Unresolvable { reason } = &cycle.suggested_resolution {
                writeln!(error_msg, "  Reason: {reason}").expect("Writing to String never fails");
            }
            error_msg.push('\n');
        }

        error_msg
    }

    /// Core bundling logic shared between file and string output modes
    /// Returns the entry module name, parsed modules, circular dependency analysis, and optional
    /// tree shaker, with graph and resolver populated via mutable references
    pub(crate) fn bundle_core(
        &mut self,
        entry_path: &Path,
        graph: &mut DependencyGraph,
        resolver_opt: &mut Option<ModuleResolver>,
    ) -> Result<(
        String,
        Vec<ParsedModuleData>,
        Option<CircularDependencyAnalysis>,
    )> {
        // Discovery state from a previous bundle run on this orchestrator must not
        // leak into this run's requirements
        self.external_importlib_targets
            .lock()
            .expect("external importlib targets lock poisoned")
            .clear();
        self.preserved_importlib_targets
            .lock()
            .expect("preserved importlib targets lock poisoned")
            .clear();

        // Store the original entry path before transformation
        let original_entry_path = entry_path.to_path_buf();

        // Handle directory as entry point
        let entry_path = if entry_path.is_dir() {
            // Check for __init__.py first (standard package import behavior)
            let init_py = entry_path.join(crate::python::constants::INIT_FILE);
            let main_py = entry_path.join(crate::python::constants::MAIN_FILE);
            let init_exists = init_py.is_file();
            let main_exists = main_py.is_file();
            if init_exists {
                if main_exists {
                    warn!(
                        "Directory {} contains both {} and {}; preferring {}. For CLI behavior, \
                         pass {}/{} explicitly.",
                        entry_path.display(),
                        crate::python::constants::INIT_FILE,
                        crate::python::constants::MAIN_FILE,
                        crate::python::constants::INIT_FILE,
                        entry_path.display(),
                        crate::python::constants::MAIN_FILE
                    );
                }
                info!(
                    "Using {} as entry point from directory: {}",
                    crate::python::constants::INIT_FILE,
                    entry_path.display()
                );
                init_py
            } else if main_exists {
                info!(
                    "Using {} as entry point from directory: {}",
                    crate::python::constants::MAIN_FILE,
                    entry_path.display()
                );
                main_py
            } else {
                return Err(anyhow!(
                    "Directory {} does not contain {} or {}",
                    entry_path.display(),
                    crate::python::constants::INIT_FILE,
                    crate::python::constants::MAIN_FILE
                ));
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
            let src_dir = src_dir
                .canonicalize()
                .unwrap_or_else(|_| src_dir.to_path_buf());
            if !self.config.src.contains(&src_dir) {
                debug!("Adding entry directory to src paths: {}", src_dir.display());
                self.config.src.insert(0, src_dir);
            }
        }

        // Initialize resolver with the updated config
        let mut resolver = ModuleResolver::new(self.config.clone())?;

        // Set the entry file to establish the primary search path
        resolver.set_entry_file(entry_path, &original_entry_path);

        // Find the entry module name
        let entry_module_name = self.find_entry_module_name(entry_path, &resolver)?;
        info!("Entry module: {entry_module_name}");

        // CRITICAL: Register the entry module FIRST to guarantee it gets ID 0
        // This is a fundamental invariant of our architecture
        let entry_id = resolver.register_module(&entry_module_name, entry_path)?;
        assert_eq!(
            entry_id,
            ModuleId::ENTRY,
            "Entry module must be ID 0 - bundling starts here"
        );

        // Build dependency graph
        let mut build_params = GraphBuildParams {
            resolver: &resolver,
            graph,
        };
        let parsed_modules = self.build_dependency_graph(&mut build_params)?;

        // In DependencyGraph, we track all modules but focus on reachable ones
        debug!("Graph has {} modules", graph.modules.len());

        // Enhanced circular dependency detection and analysis
        let mut circular_dep_analysis = None;
        if graph.has_cycles() {
            let analysis =
                crate::analyzers::dependency_analyzer::analyze_circular_dependencies(graph);

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
    pub(crate) fn get_sorted_modules_from_graph(
        &self,
        graph: &DependencyGraph,
        circular_dep_analysis: Option<&CircularDependencyAnalysis>,
    ) -> Result<Vec<ModuleId>> {
        debug!(
            "get_sorted_modules_from_graph called with circular_dep_analysis: {}",
            circular_dep_analysis.is_some()
        );

        let module_ids = if circular_dep_analysis.is_some() {
            // Circular dependencies present — use SCC condensation for cycle-aware ordering
            debug!("Using SCC condensation for cycle-aware module ordering");
            graph.topological_sort_with_cycles()
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
    pub(crate) fn bundle_to_string(
        &mut self,
        entry_path: &Path,
        emit_requirements: bool,
    ) -> Result<String> {
        info!("Starting bundle process for stdout output");

        // Initialize empty graph - resolver will be created in bundle_core
        let mut graph = DependencyGraph::new();
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
            let mut shaker = TreeShaker::from_graph(&graph, &resolver);

            // Analyze from entry module (resolver guarantees ENTRY name is registered)
            // We use resolver to fetch it for logging and correctness where needed
            let entry_name = resolver
                .get_module_name(ModuleId::ENTRY)
                .unwrap_or_else(|| "__main__".to_owned());
            shaker.analyze(&entry_name);

            Some(shaker)
        } else {
            None
        };

        // Generate bundled code
        info!("Using hybrid static bundler");
        let emitted = self.emit_static_bundle(&StaticBundleParams {
            sorted_module_ids: &sorted_module_ids,
            parsed_modules: Some(&parsed_modules),
            resolver: &resolver,
            graph: &graph,
            circular_dep_analysis: circular_dep_analysis.as_ref(),
            tree_shaker: tree_shaker.as_ref(),
            output_path: None,
        })?;
        let mut bundled_code = emitted.code;

        // Bake the map digest into the runtime prologue (or blank the
        // placeholder when no map was produced).
        if self.config.sourcemap.is_some() {
            bundled_code =
                crate::source_map::apply_map_digest(&bundled_code, emitted.source_map.as_deref());
        }

        // Stdout output can only carry an inline map; other modes are rejected
        // at CLI validation time.
        if self.config.sourcemap == Some(SourceMapMode::Inline)
            && let Some(map_json) = emitted.source_map.as_deref()
        {
            bundled_code.push('\n');
            bundled_code.push_str(&crate::source_map::inline_source_mapping_comment(map_json));
        }

        // Generate requirements.txt if requested
        if emit_requirements {
            self.write_requirements_file_for_stdout(&sorted_module_ids, &resolver, &graph)?;
        }

        Ok(bundled_code)
    }

    /// Main bundling function
    #[allow(unreachable_pub)]
    pub fn bundle(
        &mut self,
        entry_path: &Path,
        output_path: &Path,
        emit_requirements: bool,
    ) -> Result<()> {
        info!("Starting bundle process");
        debug!("Output: {}", output_path.display());

        // Initialize empty graph - resolver will be created in bundle_core
        let mut graph = DependencyGraph::new();
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
            let mut shaker = TreeShaker::from_graph(&graph, &resolver);

            let entry_name = resolver
                .get_module_name(ModuleId::ENTRY)
                .unwrap_or_else(|| "__main__".to_owned());
            shaker.analyze(&entry_name);

            Some(shaker)
        } else {
            None
        };

        // Generate bundled code
        info!("Using hybrid static bundler");
        let emitted = self.emit_static_bundle(&StaticBundleParams {
            sorted_module_ids: &sorted_module_ids,
            parsed_modules: Some(&parsed_modules), // Use pre-parsed modules to avoid double parsing
            resolver: &resolver,
            graph: &graph,
            circular_dep_analysis: circular_dep_analysis.as_ref(),
            tree_shaker: tree_shaker.as_ref(),
            output_path: Some(output_path),
        })?;
        let mut bundled_code = emitted.code;

        // Bake the map digest into the runtime prologue (or blank the
        // placeholder when no map was produced): the digest lives inside the
        // executing code itself, immune to on-disk replacement.
        if self.config.sourcemap.is_some() {
            bundled_code =
                crate::source_map::apply_map_digest(&bundled_code, emitted.source_map.as_deref());
        }

        // Apply the configured source map delivery mode. The map file itself is
        // written only after the bundle write succeeds, so a failed run never
        // leaves an orphaned (and potentially stale) map next to an old bundle.
        let mut pending_map: Option<(PathBuf, &str)> = None;
        if let (Some(mode), Some(map_json)) = (self.config.sourcemap, emitted.source_map.as_deref())
        {
            match mode {
                SourceMapMode::Linked | SourceMapMode::External => {
                    let map_path = source_map_path_for(output_path);
                    if mode == SourceMapMode::Linked {
                        let map_file_name =
                            map_path.file_name().unwrap_or_else(|| map_path.as_os_str());
                        bundled_code.push('\n');
                        bundled_code.push_str(&crate::source_map::linked_source_mapping_comment(
                            map_file_name,
                        ));
                    }
                    pending_map = Some((map_path, map_json));
                }
                SourceMapMode::Inline => {
                    bundled_code.push('\n');
                    bundled_code
                        .push_str(&crate::source_map::inline_source_mapping_comment(map_json));
                }
            }
        }

        // Generate requirements.txt if requested
        if emit_requirements {
            self.write_requirements_file(&sorted_module_ids, &resolver, &graph, output_path)?;
        }

        // Publish the bundle and map as close to atomically as possible: the
        // map content is staged to a temp file *before* the bundle is written
        // (a staging failure aborts with the old pair intact) and renamed over
        // the final map path *after* (rename is atomic and replaces a stale
        // map even when the file itself is read-only). The only remaining
        // mismatch window is a rename failure, which requires directory-level
        // problems that would have failed the bundle write too.
        let staged_map = if let Some((map_path, map_json)) = pending_map {
            let tmp_path = stage_map_file(&map_path, map_json).with_context(|| {
                format!(
                    "Failed to stage source map file next to: {}",
                    map_path.display()
                )
            })?;
            Some((tmp_path, map_path))
        } else {
            None
        };

        // Write output file
        let bundle_write = fs::write(output_path, bundled_code)
            .with_context(|| format!("Failed to write output file: {}", output_path.display()));
        if let Err(err) = bundle_write {
            if let Some((tmp_path, _)) = staged_map {
                let _ = fs::remove_file(tmp_path);
            }
            return Err(err);
        }

        info!("Bundle written to: {}", output_path.display());

        if let Some((tmp_path, map_path)) = staged_map {
            // std's rename replaces an existing destination on every platform
            // (MoveFileExW with MOVEFILE_REPLACE_EXISTING on Windows). The one
            // residual case is a read-only destination on Windows, so fall
            // back to remove-then-rename before giving up.
            let publish = fs::rename(&tmp_path, &map_path).or_else(|_| {
                fs::remove_file(&map_path).and_then(|()| fs::rename(&tmp_path, &map_path))
            });
            if let Err(err) = publish {
                let _ = fs::remove_file(&tmp_path);
                return Err(err).with_context(|| {
                    format!("Failed to publish source map file: {}", map_path.display())
                });
            }
            info!("Source map written to: {}", map_path.display());
        }

        Ok(())
    }

    /// Extract imports from module items
    fn extract_imports_from_module_items(
        &self,
        items: &FxIndexMap<crate::dependency_graph::ItemId, crate::dependency_graph::ItemData>,
    ) -> Vec<String> {
        let mut imports = Vec::new();
        for item_data in items.values() {
            match &item_data.item_type {
                crate::dependency_graph::ItemType::Import { module, .. }
                | crate::dependency_graph::ItemType::FromImport { module, .. } => {
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
        // Canonicalize the entry path to handle relative paths
        let canonical_entry = entry_path
            .canonicalize()
            .unwrap_or_else(|_| entry_path.to_path_buf());
        for src_dir in &self.config.src {
            log::debug!("Checking if {canonical_entry:?} starts with {src_dir:?}");

            // Handle empty src_dir - skip it as it will match everything and produce absolute paths
            if src_dir.as_os_str().is_empty() {
                log::debug!("Skipping empty src_dir to avoid absolute path module names");
                continue;
            }

            let Ok(relative_path) = canonical_entry.strip_prefix(src_dir) else {
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

        // Special case: If the entry is __init__.py, use the package name
        if entry_path
            .file_name()
            .and_then(|f| f.to_str())
            .is_some_and(crate::python::module_path::is_init_file_name)
        {
            // Get the package name from the parent directory
            if let Some(parent) = entry_path.parent()
                && let Some(package_name) = self.find_module_in_src_dirs(parent)
            {
                log::debug!(
                    "Entry is {} in package '{}', using package name as module name",
                    crate::python::constants::INIT_FILE,
                    package_name
                );
                return Ok(package_name);
            }
            // Fallback if we can't determine the package name
            log::debug!(
                "Entry is {}, but couldn't determine package name, using '{}'",
                crate::python::constants::INIT_FILE,
                crate::python::constants::INIT_STEM
            );
            return Ok(crate::python::constants::INIT_STEM.to_owned());
        }

        // Special case: If the entry is __main__.py in a package, preserve the module suffix
        let file_name = entry_path.file_name().and_then(|f| f.to_str());
        log::debug!("Entry file name: {file_name:?}");
        if file_name.is_some_and(crate::python::module_path::is_main_file_name) {
            // Try to get the package name from the parent directory
            if let Some(parent) = entry_path.parent()
                && let Some(package_name) = self.find_module_in_src_dirs(parent)
            {
                let module_name = format!("{package_name}.{}", crate::python::constants::MAIN_STEM);
                log::debug!(
                    "Entry is {} in package '{}', using '{}' as module name",
                    crate::python::constants::MAIN_FILE,
                    package_name,
                    module_name
                );
                return Ok(module_name);
            }
            // Fall through to normal logic if we can't determine the package name
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
                anyhow!("Cannot determine module name from entry path: {entry_path:?}")
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
        Self::queue_main_entry_package_initializer(
            params.resolver,
            &mut modules_to_process,
            &mut queued_modules,
        )?;

        // Store module data for phase 2, including the parse products.
        type DiscoveryData = (ModuleId, PathBuf, Vec<String>, ProcessedModule);
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

            // Parse the module and cache its reusable facts.
            let processed = self.process_module(&module_path, &module_name)?;

            // Record literal distribution-metadata queries (importlib.metadata.version
            // et al.) and package-resource reads (importlib.resources.files,
            // pkgutil.get_data) before classifying this module's imports, so queried
            // providers and resource targets are kept external and installed
            if self.config.bundle_third_party()
                && (processed.source.contains("importlib")
                    || processed.source.contains("pkg_resources")
                    || processed.source.contains("pkgutil"))
            {
                let usage = crate::resolver::queried_distribution_requirements(&processed.ast);
                params
                    .resolver
                    .record_queried_distributions(usage.requirements);
                params
                    .resolver
                    .record_resource_read_imports(usage.resource_import_targets);
                if usage.enumerates_distributions {
                    params.resolver.record_global_distribution_enumeration();
                }
            }

            // Record imported providers whose filesystem/import-spec globals this
            // module reads (provider.__file__, provider.__spec__.origin, ...),
            // that are passed to source-inspection or module-identity APIs
            // (inspect.getsource, ismodule, isinstance against ModuleType), or
            // that appear in hash-requiring contexts (dict keys, set elements):
            // generated namespaces carry no faithful values and are unhashable,
            // so observed targets keep their installed module identity. No
            // cheap textual prefilter exists for the hash contexts, so the
            // collector runs on every module (one AST pass).
            if self.config.bundle_third_party() {
                params.resolver.record_resource_read_imports(
                    crate::visitors::utils::imported_module_dunder_read_targets(
                        &processed.ast.body,
                    ),
                );
            }

            // Record module names whose sys.modules entries this module observes
            // (sys.modules["dep"], sys.modules[dep.__name__]): bundled targets must
            // register in sys.modules when their init runs, because static imports
            // invoke the initializer directly rather than the import machinery
            if processed.source.contains("modules") {
                params.resolver.record_sys_modules_observed_targets(
                    crate::visitors::utils::sys_modules_observed_module_names(&processed.ast.body),
                );
            }

            // Extract imports from the processed AST
            let imports_with_context = self.extract_imports_from_facts(
                &processed.facts.discovered_imports,
                &module_path,
                Some(params.resolver),
            );
            // Reachability pruning consumes these edges as ABSOLUTE module names:
            // normalize relative importlib targets (".backend" with package="pkg")
            // the same way discovery queueing does, so the pruning pass sees
            // "pkg.backend" rather than a raw relative string it can never match
            let imports: Vec<String> = imports_with_context
                .iter()
                .map(|(m, _, import_type, package_context)| {
                    if *import_type == Some(crate::visitors::ImportType::ImportlibStatic)
                        && m.starts_with('.')
                        && let Some((resolved_name, _)) = params
                            .resolver
                            .resolve_importlib_static_with_context(m, package_context.as_deref())
                    {
                        resolved_name
                    } else {
                        m.clone()
                    }
                })
                .collect();
            debug!("Extracted imports from {module_name}: {imports:?}");

            discovered_modules.push((module_id, module_path.clone(), imports, processed));
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
                    package_context.as_ref(),
                    &mut discovery_params,
                )?;
            }
        }

        info!(
            "Phase 1 complete: discovered {} modules",
            discovered_modules.len()
        );

        // Metadata queries recorded late in discovery can flip earlier bundling
        // decisions; drop modules whose final classification is external before they
        // enter the graph. Dropped names are recorded as external importlib targets:
        // modules reached only through literal import_module calls leave no graph
        // import item, so requirements generation would otherwise never see them
        if self.config.bundle_third_party() {
            let modules_before_drop = discovered_modules.len();
            let mut external_targets = self
                .external_importlib_targets
                .lock()
                .expect("external importlib targets lock poisoned");
            discovered_modules.retain(|(module_id, module_path, _, _)| {
                if module_id.is_entry() {
                    return true;
                }
                let Some(module_name) = params.resolver.get_module_name(*module_id) else {
                    return true;
                };
                let keep = params
                    .resolver
                    .classify_import(&module_name)
                    .should_bundle();
                if !keep {
                    debug!(
                        "Dropping module '{module_name}' ({}) after discovery: final \
                         classification is external",
                        module_path.display()
                    );
                    external_targets.insert(module_name);
                }
                keep
            });
            drop(external_targets);
            // Dropping a module can orphan dependencies it alone pulled in: a
            // transitive module no longer reachable from the retained graph must not
            // be bundled, or its side effects would execute while the external parent
            // loads its own installed copy of the dependency
            if discovered_modules.len() != modules_before_drop {
                Self::prune_unreachable_modules(&mut discovered_modules, params.resolver);
            }
        }

        // PHASE 2: Add all modules to graph and create dependency edges
        info!("Phase 2: Adding modules to graph...");

        // First, add all modules to the graph and run semantic analysis.
        let mut parsed_modules: Vec<ParsedModuleData> = Vec::new();

        for (module_id, module_path, imports, processed) in discovered_modules {
            let module_name = params
                .resolver
                .get_module_name(module_id)
                .unwrap_or_else(|| format!("module_{}", module_id.as_u32()));
            debug!("Phase 2: Processing module '{module_name}'");

            params.graph.add_module(module_id, params.resolver);
            self.conflict_resolver
                .analyze_module(module_id, &processed.ast, &module_path);
            debug!("Added module to graph: {module_name} with ID {module_id:?}");

            // Build dependency graph BEFORE no-ops removal
            if let Some(module) = params.graph.get_module_mut(module_id) {
                debug_assert!(
                    module.items.is_empty(),
                    "Module graph should be empty before populating cached facts"
                );
                processed.facts.populate_module_graph(module);
            }

            // Store parsed module data for later use
            parsed_modules.push((module_id, imports, processed.ast, processed.source));
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
                                crate::dependency_graph::ItemType::Import { module, alias }
                                    if alias.as_deref() == Some(base_name) =>
                                {
                                    Some(module.clone())
                                }
                                _ => None,
                            })
                            .unwrap_or_else(|| base_name.clone());

                        // Try to resolve the accessed module name to a ModuleId
                        if let Some(accessed_module) = params
                            .resolver
                            .get_module_id_by_name(&resolved_module_name)
                            .and_then(|id| params.graph.get_module(id))
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

    /// Queue the containing package so its initializer runs before a package `__main__.py`.
    fn queue_main_entry_package_initializer(
        resolver: &ModuleResolver,
        modules_to_process: &mut ModuleQueue,
        queued_modules: &mut IndexSet<ModuleId>,
    ) -> Result<()> {
        if resolver.get_module_kind(ModuleId::ENTRY)
            != Some(crate::python::module_path::ModuleKind::Main)
        {
            return Ok(());
        }

        let Some(entry_module_name) = resolver.get_module_name(ModuleId::ENTRY) else {
            return Ok(());
        };
        let Some(package_name) =
            entry_module_name.strip_suffix(&format!(".{}", crate::python::constants::MAIN_STEM))
        else {
            return Ok(());
        };
        if !resolver.classify_import(package_name).should_bundle() {
            return Ok(());
        }
        let Some(package_path) = resolver.resolve_module_path(package_name)? else {
            return Ok(());
        };

        let package_id = resolver.register_module(package_name, &package_path)?;
        if queued_modules.insert(package_id) {
            debug!(
                "Adding entry package initializer '{}' to discovery queue",
                package_path.display()
            );
            modules_to_process.push((package_id, package_path));
        }
        Ok(())
    }

    /// Drop discovered modules that are no longer reachable from the entry module
    /// through the retained modules' imports.
    ///
    /// Reachability follows resolved import names plus their ancestor packages
    /// (importing a submodule imports its parents). Orphans are simply dropped, not
    /// recorded as external targets: bundled code never references them directly, and
    /// the external parent's own distribution requirements cover their installation.
    fn prune_unreachable_modules(
        discovered_modules: &mut Vec<(ModuleId, PathBuf, Vec<String>, ProcessedModule)>,
        resolver: &ModuleResolver,
    ) {
        let module_names: Vec<Option<String>> = discovered_modules
            .iter()
            .map(|(module_id, _, _, _)| resolver.get_module_name(*module_id))
            .collect();
        let mut index_by_name: FxIndexMap<String, usize> = FxIndexMap::default();
        for (index, name) in module_names.iter().enumerate() {
            if let Some(name) = name {
                index_by_name.insert(name.clone(), index);
            }
        }

        let mut reachable = vec![false; discovered_modules.len()];
        let mut queue: Vec<usize> = Vec::new();
        // Mark a name and its ancestor packages reachable
        let mark = |name: &str, reachable: &mut Vec<bool>, queue: &mut Vec<usize>| {
            let mut current = name;
            loop {
                if let Some(&index) = index_by_name.get(current)
                    && !reachable[index]
                {
                    reachable[index] = true;
                    queue.push(index);
                }
                match current.rsplit_once('.') {
                    Some((parent, _)) => current = parent,
                    None => break,
                }
            }
        };

        for (index, (module_id, _, _, _)) in discovered_modules.iter().enumerate() {
            if module_id.is_entry() {
                reachable[index] = true;
                queue.push(index);
                // The entry's ancestor packages are bundled alongside it (a package
                // __main__ entry pulls in its package initializer)
                if let Some(name) = &module_names[index] {
                    mark(name, &mut reachable, &mut queue);
                }
            }
        }
        while let Some(index) = queue.pop() {
            // Split borrows: clone the imports to walk them while marking
            for import in discovered_modules[index].2.clone() {
                mark(&import, &mut reachable, &mut queue);
            }
        }

        let mut index = 0;
        discovered_modules.retain(|(_, module_path, _, _)| {
            let keep = reachable[index];
            if !keep {
                debug!(
                    "Pruning module '{}' ({}) after discovery: it is only reachable through \
                     dropped external modules",
                    module_names[index].as_deref().unwrap_or("<unknown>"),
                    module_path.display()
                );
            }
            index += 1;
            keep
        });
    }

    /// Resolve imports from precomputed module facts with full context information.
    fn extract_imports_from_facts(
        &self,
        discovered_imports: &[DiscoveredImport],
        file_path: &Path,
        mut resolver: Option<&ModuleResolver>,
    ) -> ImportExtractionResult {
        debug!("ModuleFacts found {} imports", discovered_imports.len());
        if log::log_enabled!(log::Level::Trace) {
            for (i, import) in discovered_imports.iter().enumerate() {
                trace!(
                    "Import {}: type={:?}, module={:?}",
                    i, import.import_type, import.module_name
                );
            }
        }
        let mut imports_with_context: ImportExtractionResult = Vec::new();

        // Process each import and track if it's in an error-handling context
        for import in discovered_imports {
            let is_in_error_handler = Self::is_import_in_error_handler(&import.location);
            let extracted_imports = if matches!(
                import.import_type,
                crate::visitors::ImportType::ImportlibStatic
                    | crate::visitors::ImportType::ImportlibPreserved
            ) {
                self.handle_importlib_static(import, file_path, resolver, is_in_error_handler)
            } else if import.level > 0 {
                self.handle_relative_import(import, file_path, &mut resolver, is_in_error_handler)
            } else if let Some(ref module_name) = import.module_name {
                self.handle_absolute_import(module_name, import, &mut resolver, is_in_error_handler)
            } else if import.names.len() == 1 {
                self.handle_single_name_import(import, is_in_error_handler)
            } else {
                Vec::new()
            };

            imports_with_context.extend(extracted_imports);
        }

        imports_with_context
    }

    /// Handle `ImportlibStatic` imports and preserve package context metadata.
    ///
    /// A PRESERVED relative call (`import_module(".backend", __package__, **{})`)
    /// resolves its absolute candidate here: through the literal package
    /// context when present, otherwise against the containing file's location —
    /// discovery only records anchor-less relative targets whose package
    /// argument is provably `__package__`, so the file-path fallback stands in
    /// for exactly that runtime value. The verbatim runtime call then finds
    /// the bundled target registered with the finder.
    fn handle_importlib_static(
        &self,
        import: &DiscoveredImport,
        file_path: &Path,
        resolver: Option<&ModuleResolver>,
        is_in_error_handler: bool,
    ) -> ImportExtractionResult {
        let mut imports_set = IndexSet::new();
        self.process_importlib_static_import(import, &mut imports_set);

        imports_set
            .into_iter()
            .filter_map(|module_name| {
                let resolved_name = if import.import_type
                    == crate::visitors::ImportType::ImportlibPreserved
                    && module_name.starts_with('.')
                {
                    if let Some(package) = &import.package_context {
                        crate::python::importlib_call::resolve_relative_name(&module_name, package)?
                    } else {
                        let name_part = module_name.trim_start_matches('.');
                        let level = (module_name.len() - name_part.len()) as u32;
                        resolver?.resolve_relative_to_absolute_module_name(
                            level,
                            (!name_part.is_empty()).then_some(name_part),
                            file_path,
                        )?
                    }
                } else {
                    module_name
                };
                Some((
                    resolved_name,
                    is_in_error_handler,
                    Some(import.import_type),
                    import.package_context.clone(),
                ))
            })
            .collect()
    }

    /// Handle relative imports by resolving them against the current file path.
    fn handle_relative_import(
        &self,
        import: &DiscoveredImport,
        file_path: &Path,
        resolver: &mut Option<&ModuleResolver>,
        is_in_error_handler: bool,
    ) -> ImportExtractionResult {
        let mut imports_set = IndexSet::new();
        self.process_relative_import_set(import, file_path, resolver, &mut imports_set);
        Self::imports_with_basic_context(imports_set, is_in_error_handler)
    }

    /// Handle absolute imports and any imported names that resolve to submodules.
    fn handle_absolute_import(
        &self,
        module_name: &str,
        import: &DiscoveredImport,
        resolver: &mut Option<&ModuleResolver>,
        is_in_error_handler: bool,
    ) -> ImportExtractionResult {
        let mut imports_with_context =
            vec![(module_name.to_owned(), is_in_error_handler, None, None)];
        let mut imports_set = IndexSet::new();
        self.check_submodule_imports_set(module_name, import, resolver, &mut imports_set);

        imports_with_context.extend(
            imports_set
                .into_iter()
                .filter(|module| module != module_name)
                .map(|module| (module, is_in_error_handler, None, None)),
        );

        imports_with_context
    }

    /// Handle single-name imports that may resolve directly to modules.
    fn handle_single_name_import(
        &self,
        import: &DiscoveredImport,
        is_in_error_handler: bool,
    ) -> ImportExtractionResult {
        let mut imports_set = IndexSet::new();
        self.process_single_name_import_set(import, &mut imports_set);
        Self::imports_with_basic_context(imports_set, is_in_error_handler)
    }

    /// Attach the default extraction context to resolved import names.
    fn imports_with_basic_context(
        imports_set: IndexSet<String>,
        is_in_error_handler: bool,
    ) -> ImportExtractionResult {
        imports_set
            .into_iter()
            .map(|module| (module, is_in_error_handler, None, None))
            .collect()
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
        import: &DiscoveredImport,
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
        import: &DiscoveredImport,
        file_path: &Path,
        resolver: &mut Option<&ModuleResolver>,
        imports: &mut IndexSet<String>,
    ) {
        // Get resolver reference
        let Some(resolver_ref) = resolver else {
            debug!("No resolver available for relative import resolution");
            return;
        };

        let Some(base_module) = resolver_ref.resolve_relative_to_absolute_module_name(
            import.level,
            None, // Don't include module_name here, we'll handle it separately
            file_path,
        ) else {
            debug!(
                "Could not resolve relative import with level {}",
                import.level
            );
            return;
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
                    // Native extensions are resolved modules even though they have no bundle path.
                    if resolver.classify_import(&potential_submodule).is_resolved() {
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
        import: &DiscoveredImport,
        imports: &mut IndexSet<String>,
    ) {
        let (name, _) = &import.names[0];
        imports.insert(name.clone());
    }

    /// Check if any imported names are actually submodules (`IndexSet` version)
    fn check_submodule_imports_set(
        &self,
        module_name: &str,
        import: &DiscoveredImport,
        resolver: &mut Option<&ModuleResolver>,
        imports: &mut IndexSet<String>,
    ) {
        let Some(resolver) = resolver else { return };

        for (name, _) in &import.names {
            let full_module_name = format!("{module_name}.{name}");
            // Try to resolve the full module name to see if it's a module
            if resolver.classify_import(&full_module_name).is_resolved() {
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
        discovery_params: &mut DiscoveryParams<'_>,
    ) -> Result<()> {
        // For first-party modules, derive the actual module name from the path
        // This is critical for relative imports where the import string might be incomplete
        // For example, "jupyter" might actually be "rich.jupyter"

        // Special handling for __main__ modules that aren't the entry point
        // If the import explicitly includes __main__, and this isn't the entry module,
        // we should preserve the __main__ suffix
        let is_explicit_main_import = import.ends_with(".__main__");
        let is_entry_module = discovery_params
            .resolver
            .get_module_path(ModuleId::ENTRY)
            .is_some_and(|entry_path| {
                entry_path.canonicalize().unwrap_or(entry_path)
                    == import_path
                        .canonicalize()
                        .unwrap_or_else(|_| import_path.clone())
            });

        // For first-party modules, we need to be careful about module naming:
        // 1. For __main__ modules that aren't the entry, preserve the __main__ suffix
        // 2. For relative imports, derive the full module name from the path
        // 3. For absolute imports, use the import string directly (preserves symlink names)
        let actual_module_name = if is_explicit_main_import && !is_entry_module {
            // For non-entry __main__ modules, use the import string directly
            // to preserve the __main__ suffix
            log::debug!(
                "Preserving __main__ suffix for non-entry module: {} at {}",
                import,
                import_path.display()
            );
            import.to_owned()
        } else if import.starts_with('.') {
            // This is a relative import that has already been resolved to an absolute path
            // We should NOT see relative imports here, but if we do, try to derive the name
            self.find_module_in_src_dirs(&import_path).map_or_else(
                || {
                    log::debug!(
                        "Could not derive module name from path: {}, using import string: {}",
                        import_path.display(),
                        import
                    );
                    import.to_owned()
                },
                |module_name| {
                    log::debug!(
                        "Derived module name '{}' from path {} (relative import was '{}')",
                        module_name,
                        import_path.display(),
                        import
                    );
                    module_name
                },
            )
        } else {
            // For absolute imports, check if we need to derive the full module name
            // This is important for cases where the import might be incomplete (e.g., "jupyter"
            // instead of "rich.jupyter") But we also need to preserve symlink names
            self.find_module_in_src_dirs(&import_path).map_or_else(
                || {
                    log::debug!(
                        "Could not derive module name from path: {}, using import string: {}",
                        import_path.display(),
                        import
                    );
                    import.to_owned()
                },
                |derived_name| {
                    // Check if the derived name is significantly different (has more parts)
                    let import_parts = import.split('.').count();
                    let derived_parts = derived_name.split('.').count();

                    // A derived name is only usable when every component is a valid
                    // Python identifier; paths through e.g. a virtualenv inside the
                    // entry directory (".venv/lib/python3.12/site-packages/...") must
                    // not override the import name
                    let derived_is_importable = derived_name
                        .split('.')
                        .all(ruff_python_stdlib::identifiers::is_identifier);

                    if derived_parts > import_parts && derived_is_importable {
                        // The derived name has more context (e.g., "rich.jupyter" vs "jupyter")
                        log::debug!(
                            "Using derived module name '{}' instead of '{}' for path {}",
                            derived_name,
                            import,
                            import_path.display()
                        );
                        derived_name
                    } else {
                        // Use the import string to preserve things like symlink names
                        log::debug!(
                            "Using import string '{}' as module name for path {} (derived would \
                             be '{}')",
                            import,
                            import_path.display(),
                            derived_name
                        );
                        import.to_owned()
                    }
                },
            )
        };

        // Register the module with resolver to get its ID
        // Note: register_module is idempotent - if the path is already registered,
        // it returns the existing ID
        let module_id = discovery_params
            .resolver
            .register_module(&actual_module_name, &import_path)?;

        if !discovery_params.processed_modules.contains(&module_id)
            && !discovery_params.queued_modules.contains(&module_id)
        {
            debug!(
                "Adding '{}' (ID: {}) to discovery queue (from import '{}')",
                actual_module_name,
                module_id.as_u32(),
                import
            );
            discovery_params
                .modules_to_process
                .push((module_id, import_path));
            discovery_params.queued_modules.insert(module_id);
        } else {
            debug!(
                "Module '{}' (ID: {}) already processed or queued, skipping (from import '{}')",
                actual_module_name,
                module_id.as_u32(),
                import
            );
        }
        Ok(())
    }

    /// Add parent packages to discovery queue to ensure __init__.py files are included
    /// For example, if importing "greetings.irrelevant", also add "greetings"
    fn add_parent_packages_to_discovery(
        &self,
        import: &str,
        params: &mut DiscoveryParams<'_>,
    ) -> Result<()> {
        let parts: Vec<&str> = import.split('.').collect();

        // For each parent package level, try to add it to discovery
        for i in 1..parts.len() {
            let parent_module = parts[..i].join(".");
            self.try_add_parent_package_to_discovery(&parent_module, import, params)?;
        }
        Ok(())
    }

    /// Try to add a single parent package to discovery if it's first-party
    fn try_add_parent_package_to_discovery(
        &self,
        parent_module: &str,
        import: &str,
        params: &mut DiscoveryParams<'_>,
    ) -> Result<()> {
        if params
            .resolver
            .classify_import(parent_module)
            .should_bundle()
        {
            if let Ok(Some(parent_path)) = params.resolver.resolve_module_path(parent_module) {
                debug!(
                    "Adding parent package '{parent_module}' to discovery queue for import \
                     '{import}'"
                );
                self.add_to_discovery_queue_if_new(parent_module, parent_path, params)?;
            }
        }
        Ok(())
    }

    /// Process an import during discovery phase with error handling context
    fn process_import_for_discovery_with_context(
        &self,
        import: &str,
        is_in_error_handler: bool,
        import_type: Option<crate::visitors::ImportType>,
        package_context: Option<&String>,
        params: &mut DiscoveryParams<'_>,
    ) -> Result<()> {
        // Special handling for ImportlibStatic imports that might have invalid Python identifiers
        if import_type == Some(crate::visitors::ImportType::ImportlibPreserved) {
            // The call is preserved verbatim and executes as a real runtime import.
            // A bundleable target (first-party module or pure third-party package)
            // must be BUNDLED and registered in sys.modules so the runtime call
            // resolves it inside the single-file bundle; anything else must reach
            // requirements generation instead.
            let classification = params.resolver.classify_import(import);
            if classification.should_bundle()
                && let Ok(Some(import_path)) = params.resolver.resolve_module_path(import)
            {
                debug!(
                    "Preserved importlib target '{import}' is bundleable; queueing it and \
                     registering it in sys.modules"
                );
                params
                    .resolver
                    .record_preserved_importlib_target(import.to_owned());
                self.add_to_discovery_queue_if_new(import, import_path, params)?;
                self.add_parent_packages_to_discovery(import, params)?;
            } else {
                debug!("Recording preserved importlib target as external: {import}");
                self.preserved_importlib_targets
                    .lock()
                    .expect("preserved importlib targets lock poisoned")
                    .insert(import.to_owned());
            }
            return Ok(());
        }
        if import_type == Some(crate::visitors::ImportType::ImportlibStatic) {
            debug!("Processing ImportlibStatic import: {import}");

            // Try to resolve ImportlibStatic with package context
            if let Some((resolved_name, import_path)) = params
                .resolver
                .resolve_importlib_static_with_context(import, package_context.map(String::as_str))
            {
                // The resolved target is subject to the same bundling policy as any
                // other import: external classifications (known_third_party, native
                // artifacts, metadata usage) must not be bypassed by literal calls
                let classification = params.resolver.classify_import(&resolved_name);
                if classification.should_bundle() {
                    debug!(
                        "Resolved ImportlibStatic '{import}' to module '{resolved_name}' at path: \
                         {}",
                        import_path.display()
                    );
                    // Use the resolved name instead of the original import
                    self.add_to_discovery_queue_if_new(&resolved_name, import_path, params)?;
                    // Python executes parent package initializers before loading a
                    // submodule; queue them like the normal-import path does
                    self.add_parent_packages_to_discovery(&resolved_name, params)?;
                } else {
                    debug!(
                        "ImportlibStatic '{import}' resolved to '{resolved_name}' but classified \
                         as external (preserving)"
                    );
                    if !resolved_name.starts_with('.') {
                        self.external_importlib_targets
                            .lock()
                            .expect("external importlib targets lock poisoned")
                            .insert(resolved_name);
                    }
                }
            } else {
                // Try normal resolution in case it's a valid Python identifier
                let classification = params.resolver.classify_import(import);
                if classification.should_bundle() {
                    if let Ok(Some(import_path)) = params.resolver.resolve_module_path(import) {
                        debug!(
                            "Resolved ImportlibStatic '{import}' to path: {}",
                            import_path.display()
                        );
                        self.add_to_discovery_queue_if_new(import, import_path, params)?;
                    } else if !is_in_error_handler {
                        return Err(anyhow!(
                            "Failed to resolve ImportlibStatic module '{import}'. \nThis import \
                             would fail at runtime with: ModuleNotFoundError: No module named \
                             '{import}'"
                        ));
                    }
                } else {
                    debug!("ImportlibStatic '{import}' classified as external (preserving)");
                    // The preserved runtime call never enters the module graph as an
                    // import item; record it so requirements generation still sees it
                    if !import.starts_with('.') {
                        self.external_importlib_targets
                            .lock()
                            .expect("external importlib targets lock poisoned")
                            .insert(import.to_owned());
                    }
                }
            }
        } else {
            // Normal import handling
            let classification = params.resolver.classify_import(import);
            if classification.should_bundle() {
                debug!(
                    "'{import}' selected for bundling (origin: {:?}, source: {:?})",
                    classification.origin, classification.source
                );
                if let Ok(Some(import_path)) = params.resolver.resolve_module_path(import) {
                    debug!("Resolved '{import}' to path: {}", import_path.display());
                    self.add_to_discovery_queue_if_new(import, import_path, params)?;

                    // Also add parent packages for submodules to ensure __init__.py files are
                    // included For example, if importing
                    // "greetings.irrelevant", also add "greetings"
                    self.add_parent_packages_to_discovery(import, params)?;
                } else {
                    // If the import is not in an error handler, this is a fatal error
                    if is_in_error_handler {
                        debug!(
                            "Failed to resolve bundled module '{import}' but it's in an error \
                             handler (try/except or with suppress)"
                        );
                    } else {
                        return Err(anyhow!(
                            "Failed to resolve bundled module '{import}'. \nThis import would \
                             fail at runtime with: ModuleNotFoundError: No module named '{import}'"
                        ));
                    }
                }
            } else {
                debug!("'{import}' classified as external (preserving)");
            }
        }
        Ok(())
    }

    /// Process an import during dependency graph creation phase
    fn process_import_for_dependency(&self, import: &str, context: &mut DependencyContext<'_>) {
        if !context.resolver.classify_import(import).should_bundle() {
            return;
        }

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
        if context
            .resolver
            .classify_import(parent_module)
            .should_bundle()
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
        sorted_module_ids: &[ModuleId],
        resolver: &ModuleResolver,
        graph: &DependencyGraph,
    ) -> Result<()> {
        let requirements_content =
            self.generate_requirements(sorted_module_ids, resolver, graph)?;
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
        sorted_module_ids: &[ModuleId],
        resolver: &ModuleResolver,
        graph: &DependencyGraph,
        output_path: &Path,
    ) -> Result<()> {
        let requirements_content =
            self.generate_requirements(sorted_module_ids, resolver, graph)?;
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
    fn emit_static_bundle(&mut self, params: &StaticBundleParams<'_>) -> Result<EmittedBundle> {
        // First, detect and resolve conflicts after all modules have been analyzed
        let conflicts = self.conflict_resolver.detect_and_resolve_conflicts();
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

        let mut static_bundler = Bundler::new(params.resolver);

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
                let content_hash = hash.iter().fold(String::new(), |mut output, b| {
                    use std::fmt::Write;
                    let _ = write!(output, "{b:02x}");
                    output
                });

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
            let import_rewriter = ImportRewriter::new(ImportDeduplicationStrategy::FunctionStart);

            // Prepare module ASTs for semantic analysis
            let module_ast_map: FxIndexMap<ModuleId, &ModModule> =
                module_asts.iter().map(|(id, ast, _)| (*id, ast)).collect();

            // Analyze movable imports using semantic analysis
            let movable_imports = import_rewriter.analyze_movable_imports_semantic(
                params.graph,
                &analysis.resolvable_cycles,
                &self.conflict_resolver,
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

        // Bundle all modules using the phase-based orchestrator
        let mut bundled_ast = PhaseOrchestrator::bundle(
            &mut static_bundler,
            &crate::code_generator::BundleParams {
                modules: &module_asts,
                sorted_module_ids: params.sorted_module_ids,
                resolver: params.resolver,
                graph: params.graph,
                conflict_resolver: &self.conflict_resolver,
                circular_dep_analysis: params.circular_dep_analysis,
                tree_shaker: params.tree_shaker,
                python_version: self.config.python_version().unwrap_or(10),
            },
        );

        // Inject the traceback-remapping runtime before code generation so the
        // emitted text and the bundled AST stay structurally aligned for the
        // source map extraction walk.
        if let Some(mode) = self.config.sourcemap {
            crate::source_map::inject_runtime_prologue(&mut bundled_ast, mode);
        }
        let bundled_ast = bundled_ast;

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
            let stmt_code =
                crate::code_generator::python_codegen::generate_statement(stmt, &stylist);
            code_parts.push(stmt_code);
        }

        // Add shebang and header
        let mut final_output = vec![
            "#!/usr/bin/env python3".to_owned(),
            "# Generated by Cribo - Python Source Bundler".to_owned(),
            "# https://github.com/ophidiarium/cribo".to_owned(),
            String::new(), // Empty line
        ];
        final_output.extend(code_parts);
        let code = final_output.join("\n");

        // Extract source map when enabled: re-parse the emitted code and walk it
        // in parallel with the bundled AST (which carries node provenance).
        let source_map = self
            .config
            .sourcemap
            .map(|_| self.extract_source_map(&code, &bundled_ast, params))
            .transpose()?;

        Ok(EmittedBundle { code, source_map })
    }

    /// Build the Source Map v3 JSON for an emitted bundle.
    fn extract_source_map(
        &self,
        code: &str,
        bundled_ast: &ModModule,
        params: &StaticBundleParams<'_>,
    ) -> Result<String> {
        let parsed_modules = params
            .parsed_modules
            .context("source map generation requires parsed module data")?;

        // Module ordinals were assigned by the bundler's AST indexing pass in
        // the order of `parsed_modules`; register provenance in the same order.
        let mut provenance = ProvenanceResolver::default();
        for (module_id, _imports, _ast, source) in parsed_modules {
            let path = params
                .resolver
                .get_module_path(*module_id)
                .unwrap_or_else(|| {
                    let name = params
                        .resolver
                        .get_module_name(*module_id)
                        .unwrap_or_else(|| format!("module_{}", module_id.as_u32()));
                    PathBuf::from(&name)
                });
            let path = fs::canonicalize(&path)
                .or_else(|_| std::path::absolute(&path))
                .unwrap_or(path);
            provenance.push_module(path, source.clone());
        }

        let file_name = params.output_path.and_then(Path::file_name).map_or_else(
            || "<stdout>".to_owned(),
            |name| name.to_string_lossy().into_owned(),
        );
        // Source paths are relative to the directory the map lives in (the
        // output directory). For stdout output the bundle's eventual location
        // is unknown, so paths stay absolute.
        let base_dir = params.output_path.and_then(Path::parent).map(|dir| {
            let dir = if dir.as_os_str().is_empty() {
                Path::new(".")
            } else {
                dir
            };
            fs::canonicalize(dir)
                .or_else(|_| std::path::absolute(dir))
                .unwrap_or_else(|_| dir.to_path_buf())
        });

        let options = SourceMapOptions {
            file: &file_name,
            include_contents: self.config.include_sources_content(),
            base_dir: base_dir.as_deref(),
        };
        build_source_map(code, bundled_ast, &provenance, &options)
            .context("failed to generate requested source map")
    }

    /// Generate requirements.txt content from third-party imports
    fn generate_requirements(
        &self,
        module_ids: &[ModuleId],
        resolver: &ModuleResolver,
        graph: &DependencyGraph,
    ) -> Result<String> {
        let mut requirement_imports = IndexMap::new();
        // Bundled third-party import -> extras requested for it via module-map
        let mut bundled_third_party_imports: FxIndexMap<String, Vec<pep508_rs::ExtraName>> =
            FxIndexMap::default();

        // Collect every bundled third-party module by ID, not by import statement:
        // modules reached only through literal importlib.import_module calls have no
        // Import/FromImport graph items, but their distributions' declared
        // dependencies must still be propagated
        if self.config.bundle_third_party() {
            for module_id in module_ids {
                let Some(module_name) = resolver.get_module_name(*module_id) else {
                    continue;
                };
                let classification = resolver.classify_import(&module_name);
                // Namespace-package parents are synthetic containers claimed by every
                // provider distribution in the namespace; ownership must come from the
                // concrete bundled descendants only
                if classification.should_bundle()
                    && matches!(classification.origin, ImportOrigin::ThirdParty)
                    && !matches!(
                        classification.source,
                        crate::resolver::ImportSource::NamespacePackage
                    )
                {
                    let extras = self.module_map_extras(&module_name)?;
                    bundled_third_party_imports
                        .entry(module_name)
                        .or_default()
                        .extend(extras);
                }
            }
        }

        // TODO: Use TYPE_CHECKING information from the dependency graph to filter out
        // dependencies that are only used for type checking. These could be placed
        // in a separate section or excluded entirely based on configuration.
        // For now, all third-party imports are included.

        for module_id in module_ids {
            if let Some(module) = graph.modules.get(module_id) {
                let imports = self.extract_imports_from_module_items(&module.items);
                for import in &imports {
                    debug!("Checking import '{import}' for requirements");
                    let classification = resolver.classify_import(import);
                    // Under --bundle-third-party, imports whose source is inlined into
                    // the bundle need no requirement entry, but their distributions'
                    // declared dependencies are still carried over below
                    if self.config.bundle_third_party() && classification.should_bundle() {
                        continue;
                    }
                    if matches!(
                        classification.origin,
                        ImportOrigin::ThirdParty | ImportOrigin::Unknown
                    ) {
                        requirement_imports
                            .entry(import.clone())
                            .or_insert_with(|| resolver.get_import_search_root(import));
                    }
                }
            }
        }

        // Static importlib targets preserved as runtime calls never appear as graph
        // import items; include the recorded ones in requirement collection
        for import in self
            .external_importlib_targets
            .lock()
            .expect("external importlib targets lock poisoned")
            .iter()
        {
            let classification = resolver.classify_import(import);
            if matches!(
                classification.origin,
                ImportOrigin::ThirdParty | ImportOrigin::Unknown
            ) && !(self.config.bundle_third_party() && classification.should_bundle())
            {
                requirement_imports
                    .entry(import.clone())
                    .or_insert_with(|| resolver.get_import_search_root(import));
            }
        }

        // Targets of preserved import_module calls execute as real runtime imports
        // even when their source is also bundled, so their distributions must stay
        // installed regardless of the target's own bundling classification
        for import in self
            .preserved_importlib_targets
            .lock()
            .expect("preserved importlib targets lock poisoned")
            .iter()
        {
            let classification = resolver.classify_import(import);
            if matches!(
                classification.origin,
                ImportOrigin::ThirdParty | ImportOrigin::Unknown
            ) {
                requirement_imports
                    .entry(import.clone())
                    .or_insert_with(|| resolver.get_import_search_root(import));
            }
        }

        let requirement_resolver = RequirementResolver::new(
            &self.config.requirements,
            resolver.get_distribution_metadata_search_directories(),
        );
        let resolved_requirements = requirement_resolver.resolve(&requirement_imports)?;
        let mut entries: Vec<String> = resolved_requirements.into_iter().collect();

        // Carry over Requires-Dist constraints declared by bundled distributions for
        // their external (or dynamically imported) dependencies.
        // Distributions whose metadata bundled code queries at runtime (e.g.
        // `importlib.metadata.version("provider")`) need their dist-info installed
        // even when no module of theirs is imported; the query alone is a dependency,
        // and constrained literals (`pkg_resources.require("provider[speed]>=2")`)
        // keep their extras and version specifiers
        if self.config.bundle_third_party() {
            entries.extend(resolver.queried_installed_distribution_requirements());
            // Global enumeration (entry_points(), packages_distributions())
            // observes EVERY installed distribution, including plugin providers
            // that are never imported: carry them all into requirements
            entries.extend(resolver.globally_enumerated_distribution_requirements());
        }
        entries.extend(resolver.bundled_distribution_requirements(&bundled_third_party_imports));

        let requirements = Self::merge_requirement_entries(entries);

        Ok(requirements.join("\n"))
    }

    /// Merge raw requirement entries into a sorted, deduplicated requirement list.
    ///
    /// Entries are grouped by normalized name; within one name, equal-marker
    /// declarations are merged (preferring a constrained declaration over a bare
    /// resolved name) while distinct marker branches stay on separate lines, since
    /// pip evaluates each line's marker independently. Unparsable entries are kept
    /// verbatim so the installer can report them.
    pub(crate) fn merge_requirement_entries(entries: Vec<String>) -> Vec<String> {
        use std::str::FromStr;
        type ParsedRequirement = pep508_rs::Requirement<pep508_rs::VerbatimUrl>;
        let mut entries_by_name: IndexMap<String, Vec<ParsedRequirement>> = IndexMap::new();
        let mut unparsable_entries: Vec<String> = Vec::new();
        for entry in entries {
            let Ok(parsed) = ParsedRequirement::from_str(&entry) else {
                if !unparsable_entries.contains(&entry) {
                    unparsable_entries.push(entry);
                }
                continue;
            };
            let name = parsed.name.to_string();
            let branches = entries_by_name.entry(name).or_default();
            if let Some(existing) = branches
                .iter_mut()
                .find(|existing| existing.marker == parsed.marker)
            {
                // Same-marker duplicates intersect their constraints: extras are
                // unioned and version-specifier sets combined, so a module-map
                // override and a bundled distribution's Requires-Dist entry both
                // apply, like a normal installation would resolve them.
                // Conflicting direct URLs are unmergeable: both lines are
                // emitted so the installer reports the conflict.
                if let Some(unmergeable) =
                    ModuleResolver::merge_requirement_constraints(existing, parsed)
                {
                    branches.push(unmergeable);
                }
            } else {
                branches.push(parsed);
            }
        }
        let mut requirements: Vec<String> = entries_by_name
            .into_values()
            .flatten()
            .map(|requirement| requirement.to_string())
            .collect();
        requirements.extend(unparsable_entries);
        requirements.sort();
        requirements
    }

    /// Return the extras requested for a bundled import through a
    /// `requirements.module-map` entry (e.g. `provider = "provider[speed]"`), matched
    /// by longest prefix like requirement resolution itself.
    ///
    /// An invalid mapping is a hard error: bundled imports skip
    /// `RequirementResolver::override_for`, so this is the only place the
    /// configuration problem can surface for them.
    fn module_map_extras(&self, import_name: &str) -> Result<Vec<pep508_rs::ExtraName>> {
        use std::str::FromStr;
        let mapping = self
            .config
            .requirements
            .module_map
            .iter()
            .filter(|(prefix, _)| RequirementResolver::matches_prefix(prefix, import_name))
            .max_by_key(|(prefix, _)| prefix.split('.').count());
        let Some((prefix, requirement)) = mapping else {
            return Ok(Vec::new());
        };
        let parsed = pep508_rs::Requirement::<pep508_rs::VerbatimUrl>::from_str(requirement)
            .with_context(|| {
                format!(
                    "Invalid PEP 508 requirement '{requirement}' configured for import prefix \
                     '{prefix}'"
                )
            })?;
        Ok(parsed.extras)
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::TempDir;

    use super::*;

    /// End-to-end source map extraction: bundle a three-module project (inlined
    /// module, wrapper module with side effects, entry) with an inline map and
    /// verify statement mappings point back at the original files and lines.
    #[test]
    fn test_source_map_extraction_end_to_end() -> Result<()> {
        let temp_dir = TempDir::new()?;
        fs::write(
            temp_dir.path().join("main.py"),
            "from utils import add\nimport effects\n\nresult = add(1, 2)\nprint(result, \
             effects.X)\n",
        )?;
        fs::write(
            temp_dir.path().join("utils.py"),
            "def add(a, b):\n    total = a + b\n    return total\n",
        )?;
        // The print side effect forces this module onto the wrapper path.
        fs::write(
            temp_dir.path().join("effects.py"),
            "print(\"side effect\")\nX = 42\n",
        )?;

        let config = Config {
            sourcemap: Some(SourceMapMode::Inline),
            ..Config::default()
        };
        let mut orchestrator = BundleOrchestrator::new(config);
        let code = orchestrator.bundle_to_string(&temp_dir.path().join("main.py"), false)?;

        // The bundle must end with an inline sourceMappingURL comment.
        let marker = "# sourceMappingURL=data:application/json;base64,";
        let marker_pos = code
            .rfind(marker)
            .expect("inline source map comment present");
        let payload = code[marker_pos + marker.len()..].trim_end();
        let map_bytes = base64_simd::STANDARD
            .decode_to_vec(payload.as_bytes())
            .expect("valid base64 payload");
        let map_json = String::from_utf8(map_bytes).expect("valid UTF-8 source map");

        let map = oxc_sourcemap::SourceMap::from_json_string(&map_json)
            .expect("valid Source Map v3 JSON");
        assert_eq!(map.get_file(), Some("<stdout>"));
        // Inline mode omits sourcesContent by default.
        assert!(!map_json.contains("sourcesContent"));

        let lookup = map.generate_lookup_table();
        let find_generated_line = |needle: &str| -> u32 {
            code.lines()
                .position(|line| line.trim() == needle)
                .unwrap_or_else(|| panic!("bundle must contain a line matching `{needle}`"))
                as u32
        };
        let assert_maps_to = |needle: &str, source_suffix: &str, original_line: u32| {
            let generated_line = find_generated_line(needle);
            let token = map
                .lookup_token(&lookup, generated_line, 0)
                .unwrap_or_else(|| panic!("mapping for `{needle}` on line {generated_line}"));
            assert_eq!(
                token.get_dst_line(),
                generated_line,
                "`{needle}` must have a mapping on its own line, not inherit an earlier one"
            );
            let source = map
                .get_source(token.get_source_id().expect("source id"))
                .expect("source path");
            assert!(
                source.ends_with(source_suffix),
                "`{needle}` should map into {source_suffix}, got {source}"
            );
            assert_eq!(
                token.get_src_line(),
                original_line,
                "`{needle}` should map to 0-based line {original_line} of {source_suffix}"
            );
        };

        // Inlined module: statement nested in a function body.
        assert_maps_to("return total", "utils.py", 2);
        // Wrapper module: statement inside the synthesized init function.
        assert_maps_to("X = 42", "effects.py", 1);
        // Entry module statement.
        assert_maps_to("result = add(1, 2)", "main.py", 3);

        Ok(())
    }

    /// External importlib targets recorded during one bundle run must not leak into a
    /// later run on the same orchestrator instance.
    #[test]
    fn test_external_importlib_targets_cleared_between_runs() -> Result<()> {
        let temp_dir = TempDir::new()?;
        let first_entry = temp_dir.path().join("first.py");
        fs::write(
            &first_entry,
            "import importlib\n\ntry:\n    \
             importlib.import_module(\"totally_unknown_dist\")\nexcept ImportError:\n    pass\n",
        )?;
        let second_entry = temp_dir.path().join("second.py");
        fs::write(&second_entry, "print(\"plain\")\n")?;

        let mut orchestrator = BundleOrchestrator::new(Config::default());
        orchestrator.bundle_to_string(&first_entry, false)?;
        assert!(
            orchestrator
                .external_importlib_targets
                .lock()
                .expect("lock")
                .contains("totally_unknown_dist"),
            "the first run must record its external importlib target"
        );

        orchestrator.bundle_to_string(&second_entry, false)?;
        assert!(
            orchestrator
                .external_importlib_targets
                .lock()
                .expect("lock")
                .is_empty(),
            "run-specific discovery state must be cleared between bundle runs"
        );

        Ok(())
    }
}
