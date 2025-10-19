//! Code generation module for bundling Python modules into a single file
//!
//! This module implements the hybrid static bundling approach which:
//! - Pre-processes and hoists safe stdlib imports
//! - Wraps first-party modules in init functions to manage initialization order
//! - Uses `__initializing__` and `__initialized__` flags to prevent circular import loops
//! - Preserves Python semantics while avoiding forward reference issues

pub(crate) mod bundler;
pub(crate) mod circular_deps;
pub(crate) mod context;
pub(crate) mod docstring_extractor;
pub(crate) mod expression_handlers;
pub(crate) mod globals;
pub(crate) mod import_deduplicator;
pub(crate) mod import_transformer;
pub(crate) mod init_function;
pub(crate) mod inliner;
pub(crate) mod module_registry;
pub(crate) mod module_transformer;
pub(crate) mod namespace_manager;
pub(crate) mod phases;
pub(crate) mod symbol_source;

// Re-export the main bundler and key types
pub(crate) use bundler::Bundler;
pub(crate) use context::BundleParams;
