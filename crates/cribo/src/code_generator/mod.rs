//! Code generation module for bundling Python modules into a single file
//!
//! This module implements the hybrid static bundling approach which:
//! - Pre-processes and hoists safe stdlib imports
//! - Wraps first-party modules in init functions to manage initialization order
//! - Uses a module cache to handle circular dependencies
//! - Preserves Python semantics while avoiding forward reference issues

pub mod bundler;
pub mod circular_deps;
pub mod context;
pub mod expression_handlers;
pub mod globals;
pub mod import_deduplicator;
pub mod import_transformer;
pub mod module_registry;
pub mod module_transformer;
pub mod namespace_manager;

// Re-export the main bundler and key types
pub use bundler::HybridStaticBundler;
pub use context::BundleParams;
