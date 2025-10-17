//! Statement Processing phase - processes each statement from transformed module body
//!
//! This phase handles the core statement-by-statement processing within an init function,
//! applying transformations and adding module attributes as needed for different statement types.
//!
//! Handles:
//! - Import statements (skip hoisted)
//! - `ImportFrom` statements (complex relative import logic)
//! - `ClassDef` statements (set __module__, add as module attribute)
//! - `FunctionDef` statements (transform nested functions, add as attribute)
//! - Assign statements (MOST COMPLEX: 140+ lines of special cases)
//! - `AnnAssign` statements (similar to Assign with annotations)
//! - Try statements (collect exportable symbols from branches)
//! - Default statements (transform for module vars)

use ruff_python_ast::Stmt;

use super::{InitFunctionState, TransformError};
use crate::{
    code_generator::{bundler::Bundler, context::ModuleTransformContext},
    types::FxIndexSet,
};

/// Statement Processing phase - processes transformed statements
pub struct StatementProcessingPhase;

impl StatementProcessingPhase {
    /// Execute the statement processing phase
    ///
    /// Takes the `processed_body` from `BodyPreparationPhase` and processes each statement,
    /// applying transformations and adding module attributes for exported symbols.
    #[allow(dead_code)] // Will be used once orchestrator is complete
    pub fn execute(
        processed_body: Vec<Stmt>,
        bundler: &Bundler,
        ctx: &ModuleTransformContext,
        all_is_referenced: bool,
        vars_used_by_exported_functions: &FxIndexSet<String>,
        module_scope_symbols: Option<&FxIndexSet<String>>,
        builtin_locals: &FxIndexSet<String>,
        state: &mut InitFunctionState,
    ) -> Result<(), TransformError> {
        // Call the extracted function from module_transformer
        crate::code_generator::module_transformer::process_statements_for_init_function(
            processed_body,
            bundler,
            ctx,
            all_is_referenced,
            vars_used_by_exported_functions,
            module_scope_symbols,
            builtin_locals,
            &state.lifted_names,
            &state.inlined_import_bindings,
            &mut state.body,
            &mut state.initialized_lifted_globals,
        );

        Ok(())
    }
}
