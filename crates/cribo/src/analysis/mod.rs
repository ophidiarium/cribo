//! Analysis module containing modular analysis components
//!
//! This module provides structured analysis components for the bundling process,
//! following the Progressive Enrichment principle where data only becomes more
//! structured as it flows through the pipeline.

pub mod circular_analyzer;
pub mod circular_deps;
pub mod pipeline;

pub use circular_analyzer::CircularDependencyAnalyzer;
pub use circular_deps::{
    CircularDependencyAnalysis, CircularDependencyGroup, CircularDependencyType, ModuleEdge,
    ResolutionStrategy,
};
pub use pipeline::run_analysis_pipeline;

/// Results from the analysis pipeline
#[derive(Debug, Clone, Default)]
pub struct AnalysisResults {
    /// Results from circular dependency analysis
    pub circular_deps: Option<CircularDependencyAnalysis>,

    /// Symbol conflicts detected across modules
    pub symbol_conflicts: Vec<SymbolConflict>,

    /// Results from tree-shaking analysis
    pub tree_shake_results: Option<TreeShakeResults>,
}

/// Represents a symbol conflict between modules
#[derive(Debug, Clone)]
pub struct SymbolConflict {
    /// The conflicting symbol name
    pub symbol_name: String,

    /// Modules that define this symbol
    pub defining_modules: Vec<String>,

    /// Suggested resolution (e.g., rename pattern)
    pub suggested_resolution: String,
}

/// Results from tree-shaking analysis
#[derive(Debug, Clone)]
pub struct TreeShakeResults {
    /// Items that should be included in the bundle
    pub included_items: Vec<(crate::cribo_graph::ModuleId, crate::cribo_graph::ItemId)>,

    /// Items that were removed by tree-shaking
    pub removed_items: Vec<(crate::cribo_graph::ModuleId, crate::cribo_graph::ItemId)>,

    /// Modules that were completely removed
    pub removed_modules: Vec<crate::cribo_graph::ModuleId>,
}
