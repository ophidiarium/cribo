//! Analysis pipeline runner
//!
//! This module implements the sequential analysis pipeline that runs
//! various analyzers on the immutable CriboGraph to produce AnalysisResults.

use anyhow::Result;
use log::{debug, info};
use rustc_hash::FxHashMap;

use crate::{
    analysis::{
        AnalysisResults, CircularDependencyAnalyzer, SymbolConflictDetector, SymbolOriginAnalyzer,
        SymbolOriginResults,
    },
    config::Config,
    cribo_graph::{CriboGraph, ModuleId},
    module_registry::ModuleRegistry,
    semantic_bundler::SemanticBundler,
    semantic_model_provider::SemanticModelProvider,
    transformation_detector::TransformationDetector,
    tree_shaking::TreeShaker,
};

/// Run the complete analysis pipeline on the graph
///
/// This function orchestrates the sequential execution of various analyzers:
/// 1. Circular dependency detection (fast graph algorithm)
/// 2. Semantic analysis for symbol conflicts (hybrid traversal)
/// 3. Tree-shaking analysis (graph traversal)
/// 4. Transformation detection (identifies all AST changes needed)
///
/// Each analyzer receives an immutable reference to the graph,
/// ensuring no mutations occur during analysis.
pub fn run_analysis_pipeline(
    graph: CriboGraph,
    registry: ModuleRegistry,
    _semantic_bundler: &SemanticBundler,
    semantic_provider: &SemanticModelProvider,
    tree_shake_enabled: bool,
    entry_module_name: &str,
    entry_module_id: ModuleId,
    config: &Config,
) -> Result<AnalysisResults> {
    info!("Starting analysis pipeline");
    let mut results = AnalysisResults {
        graph,
        entry_module: entry_module_id,
        module_registry: registry,
        circular_deps: None,
        symbol_conflicts: Vec::new(),
        tree_shake_results: None,
        symbol_origins: SymbolOriginResults::default(),
        transformations: FxHashMap::default(),
    };

    // Stage 1: Circular dependency detection
    debug!("Stage 1: Running circular dependency analysis");
    let circular_analyzer = CircularDependencyAnalyzer::new(&results.graph);
    let circular_deps = circular_analyzer.analyze();

    if circular_deps.has_cycles() {
        info!(
            "Found {} circular dependencies ({} resolvable, {} unresolvable)",
            circular_deps.total_cycles_detected,
            circular_deps.resolvable_cycles.len(),
            circular_deps.unresolvable_cycles.len()
        );
    } else {
        debug!("No circular dependencies detected");
    }
    results.circular_deps = Some(circular_deps);

    // Stage 2: Symbol origin analysis for tracking re-exports
    debug!("Stage 2: Analyzing symbol origins for re-exports and aliases");
    let origin_analyzer =
        SymbolOriginAnalyzer::new(&results.graph, &results.module_registry, semantic_provider);
    let symbol_origins = origin_analyzer.analyze_origins()?;

    info!(
        "Found {} symbol origin mappings (re-exports/aliases)",
        symbol_origins.len()
    );

    // Stage 3: Semantic analysis for symbol conflicts
    debug!("Stage 3: Analyzing symbol conflicts");
    let conflict_detector =
        SymbolConflictDetector::new(&results.graph, &results.module_registry, semantic_provider);
    let symbol_conflicts = conflict_detector.detect_conflicts(&symbol_origins)?;

    // Store symbol origins after using them
    results.symbol_origins = SymbolOriginResults { symbol_origins };

    if !symbol_conflicts.is_empty() {
        info!(
            "Found {} symbol conflicts across modules",
            symbol_conflicts.len()
        );
        for conflict in &symbol_conflicts {
            debug!(
                "Symbol '{}' conflicts across {} modules",
                conflict.symbol_name,
                conflict.conflicts.len()
            );
        }
    } else {
        debug!("No symbol conflicts detected");
    }
    results.symbol_conflicts = symbol_conflicts;

    // Stage 4: Tree-shaking analysis (if enabled)
    let tree_shake_results = if tree_shake_enabled {
        debug!("Stage 4: Running tree-shaking analysis");

        // Create tree shaker from graph
        let mut tree_shaker = TreeShaker::from_graph(&results.graph);

        // Run analysis from entry module
        tree_shaker.analyze(entry_module_name)?;

        // Generate tree-shake results from the analysis
        let tree_shake_results = tree_shaker.generate_results(&results.graph);

        debug!(
            "Tree-shaking analysis complete: {} items included, {} items removed",
            tree_shake_results.included_items.len(),
            tree_shake_results.removed_items.len()
        );

        Some(tree_shake_results)
    } else {
        debug!("Stage 4: Tree-shaking disabled, skipping");
        None
    };

    // Stage 5: Transformation detection
    debug!("Stage 5: Detecting required transformations");
    let transformation_detector = TransformationDetector::new(
        &results.graph,
        &results.module_registry,
        semantic_provider,
        tree_shake_results.as_ref(),
        config.python_version()?,
    );

    let transformations = transformation_detector.detect_transformations()?;

    info!(
        "Detected transformations for {} items",
        transformations.len()
    );

    results.transformations = transformations;
    results.tree_shake_results = tree_shake_results;

    info!("Analysis pipeline complete");
    Ok(results)
}
