//! Variable collection visitor for AST traversal
//!
//! This visitor tracks variable usage, references, and dependencies.

use ruff_python_ast::ModModule;

use crate::analyzers::types::CollectedVariables;

/// Visitor that collects variable usage information
pub struct VariableCollector;

impl VariableCollector {
    /// Run the collector on a module and return collected variables
    pub fn analyze(_module: &ModModule) -> CollectedVariables {
        // TODO: Implement in Phase 3
        CollectedVariables::default()
    }
}
