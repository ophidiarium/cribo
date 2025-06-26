//! Analysis module containing modular analysis components
//!
//! This module provides structured analysis components for the bundling process,
//! following the Progressive Enrichment principle where data only becomes more
//! structured as it flows through the pipeline.

pub mod circular_analyzer;
pub mod circular_deps;

pub use circular_analyzer::CircularDependencyAnalyzer;
pub use circular_deps::{
    CircularDependencyAnalysis, CircularDependencyGroup, CircularDependencyType, ModuleEdge,
    ResolutionStrategy,
};
