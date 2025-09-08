// This file only exists when the 'bench' feature is enabled
// It's used exclusively for benchmarking and does not affect dead code detection
// in normal builds

#![cfg(all(feature = "bench", not(doctest)))]

pub mod analyzers;
pub mod ast_builder;
pub mod ast_indexer;
pub mod code_generator;
pub mod combine;
pub mod config;
pub mod cribo_graph;
pub mod dirs;
pub mod graph_builder;
pub mod import_alias_tracker;
pub mod import_rewriter;
pub mod orchestrator;
pub mod python;
pub mod resolver;
pub mod semantic_bundler;
pub mod side_effects;
pub mod transformation_context;
pub mod tree_shaking;
pub mod types;
pub mod util;
pub mod visitors;
