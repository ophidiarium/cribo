// Benchmark-only library interface
//
// This library is ONLY built when the 'bench' feature is enabled and exists solely
// to provide benchmark access to core types. Unlike main.rs which declares all
// internal modules, this minimal interface only exposes what benchmarks need.

#![cfg(all(feature = "bench", not(doctest)))]
#![allow(dead_code)] // Benchmark library: internal code used transitively via BundleOrchestrator

// Include only the modules that export public API types needed by benchmarks
pub mod config;
pub mod dependency_graph;
pub mod orchestrator;
pub mod resolver;

// Internal modules - these MUST be declared for orchestrator to compile,
// but since they're pub(crate) and used transitively, no dead_code warnings
pub(crate) mod analyzers;
pub(crate) mod ast_builder;
pub(crate) mod ast_indexer;
pub(crate) mod code_generator;
pub(crate) mod combine;
pub(crate) mod dirs;
pub(crate) mod graph_builder;
pub(crate) mod import_alias_tracker;
pub(crate) mod import_rewriter;
pub(crate) mod python;
pub(crate) mod side_effects;
pub(crate) mod symbol_conflict_resolver;
pub(crate) mod transformation_context;
pub(crate) mod tree_shaking;
pub(crate) mod types;
pub(crate) mod util;
pub(crate) mod visitors;
