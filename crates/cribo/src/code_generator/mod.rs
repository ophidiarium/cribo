//! Code generation module for bundling Python modules into a single file
//!
//! This module implements the hybrid static bundling approach which:
//! - Pre-processes and hoists safe stdlib imports
//! - Wraps first-party modules in init functions to manage initialization order
//! - Uses `__initializing__` and `__initialized__` flags to prevent circular import loops
//! - Preserves Python semantics while avoiding forward reference issues

pub mod bundler;
pub mod circular_deps;
pub mod context;
pub mod docstring_extractor;
pub mod expression_handlers;
pub mod globals;
pub mod import_deduplicator;
pub mod import_transformer;
pub mod init_function;
pub mod inliner;
pub mod module_registry;
pub mod module_transformer;
pub mod namespace_manager;
pub mod phases;
pub mod symbol_source;

// Re-export the main bundler and key types
pub use bundler::Bundler;
pub use context::BundleParams;
