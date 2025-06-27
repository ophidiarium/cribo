//! Analysis module containing modular analysis components
//!
//! This module provides structured analysis components for the bundling process,
//! following the Progressive Enrichment principle where data only becomes more
//! structured as it flows through the pipeline.

pub mod circular_analyzer;
pub mod circular_deps;
pub mod pipeline;
pub mod potential_exports;
pub mod symbol_conflict_detector;
pub mod symbol_origin_analyzer;

pub use circular_analyzer::CircularDependencyAnalyzer;
pub use circular_deps::{
    CircularDependencyAnalysis, CircularDependencyGroup, CircularDependencyType, ModuleEdge,
    ResolutionStrategy,
};
pub use pipeline::run_analysis_pipeline;
pub use potential_exports::PotentialExportsMap;
use rustc_hash::FxHashMap;
pub use symbol_conflict_detector::SymbolConflictDetector;
pub use symbol_origin_analyzer::{SymbolOriginAnalyzer, SymbolOriginResults};

use crate::{
    cribo_graph::{CriboGraph, ItemId, ModuleId},
    module_registry::ModuleRegistry,
    semantic_model_provider::GlobalBindingId,
    transformations::TransformationMetadata,
};

/// Results from the analysis pipeline
#[derive(Debug)]
pub struct AnalysisResults {
    /// The dependency graph (single source of truth for module metadata)
    pub graph: CriboGraph,

    /// Entry module ID for the bundle
    pub entry_module: ModuleId,

    /// Module registry
    pub module_registry: ModuleRegistry,

    /// Results from circular dependency analysis
    pub circular_deps: Option<CircularDependencyAnalysis>,

    /// Symbol conflicts detected across modules
    pub symbol_conflicts: Vec<SymbolConflict>,

    /// Results from tree-shaking analysis
    pub tree_shake_results: Option<TreeShakeResults>,

    /// Symbol origin mappings for re-exports and aliases
    pub symbol_origins: SymbolOriginResults,

    /// Transformation plan: (ModuleId, ItemId) -> required transformations
    pub transformations: FxHashMap<(ModuleId, ItemId), Vec<TransformationMetadata>>,
}

/// Represents a symbol conflict between modules
#[derive(Debug, Clone)]
pub struct SymbolConflict {
    /// The conflicting symbol name
    pub symbol_name: String,

    /// Conflicting instances with their global IDs
    pub conflicts: Vec<ConflictInstance>,
}

/// A specific instance of a conflicting symbol
#[derive(Debug, Clone)]
pub struct ConflictInstance {
    /// Global identifier for this binding
    pub global_id: GlobalBindingId,

    /// Module name for display purposes
    pub module_name: String,

    /// The type of symbol (function, class, variable, etc.)
    pub symbol_type: SymbolType,

    /// Source location of the definition
    pub definition_range: ruff_text_size::TextRange,
}

/// Type of symbol that's conflicting
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SymbolType {
    Function,
    Class,
    Variable,
    Import,
    Other,
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
