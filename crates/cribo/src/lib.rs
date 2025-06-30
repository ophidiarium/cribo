// This file only exists when the 'bench' feature is enabled
// It's used exclusively for benchmarking and does not affect dead code detection
// in normal builds

#![cfg(feature = "bench")]

pub mod ast_indexer;
pub mod code_generator;
pub mod combine;
pub mod config;
pub mod cribo_graph;
pub mod dirs;
pub mod graph_builder;
pub mod module_registry;
pub mod module_transformer;
pub mod orchestrator;
pub mod python_compatibility;
pub mod resolver;
pub mod semantic_analysis;
pub mod transformation_context;
pub mod tree_shaking;
pub mod unused_imports;
pub mod util;
pub mod visitors;