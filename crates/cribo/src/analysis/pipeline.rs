//! Analysis pipeline runner
//!
//! This module implements the sequential analysis pipeline that runs
//! various analyzers on the immutable CriboGraph to produce AnalysisResults.

use anyhow::Result;
use log::{debug, info};

use crate::{
    analysis::{AnalysisResults, CircularDependencyAnalyzer},
    cribo_graph::CriboGraph,
    orchestrator::ModuleRegistry,
    semantic_bundler::SemanticBundler,
    tree_shaking::TreeShaker,
};

/// Run the complete analysis pipeline on the graph
///
/// This function orchestrates the sequential execution of various analyzers:
/// 1. Circular dependency detection (fast graph algorithm)
/// 2. Semantic analysis for symbol conflicts (hybrid traversal)
/// 3. Tree-shaking analysis (graph traversal)
///
/// Each analyzer receives an immutable reference to the graph,
/// ensuring no mutations occur during analysis.
pub fn run_analysis_pipeline(
    graph: &CriboGraph,
    _registry: &ModuleRegistry,
    _semantic_bundler: &SemanticBundler,
    tree_shake_enabled: bool,
    entry_module_name: &str,
) -> Result<AnalysisResults> {
    info!("Starting analysis pipeline");
    let mut results = AnalysisResults::default();

    // Stage 1: Circular dependency detection
    debug!("Stage 1: Running circular dependency analysis");
    let circular_analyzer = CircularDependencyAnalyzer::new(graph);
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

    // Stage 2: Semantic analysis for symbol conflicts
    debug!("Stage 2: Analyzing symbol conflicts");
    // TODO: Extract symbol conflicts from semantic_bundler
    // For now, semantic_bundler mutates internal state, but in the future
    // it should produce a Vec<SymbolConflict> that we can store
    results.symbol_conflicts = Vec::new();

    // Stage 3: Tree-shaking analysis (if enabled)
    if tree_shake_enabled {
        debug!("Stage 3: Running tree-shaking analysis");

        // Create tree shaker from graph
        let mut tree_shaker = TreeShaker::from_graph(graph);

        // Run analysis from entry module
        tree_shaker.analyze(entry_module_name)?;

        // For now, we don't have direct access to the results
        // In a future refactoring, TreeShaker should return structured results
        // instead of maintaining internal state
        debug!("Tree-shaking analysis complete");

        // TODO: Extract results from tree_shaker once it's refactored to return them
        results.tree_shake_results = None;
    } else {
        debug!("Stage 3: Tree-shaking disabled, skipping");
    }

    info!("Analysis pipeline complete");
    Ok(results)
}
