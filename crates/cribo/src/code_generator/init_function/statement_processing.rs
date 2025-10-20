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

use super::{InitFunctionState, body_preparation::BodyPreparationContext};
use crate::code_generator::{bundler::Bundler, context::ModuleTransformContext};

/// Statement Processing phase - processes transformed statements
pub(crate) struct StatementProcessingPhase;

impl StatementProcessingPhase {
    /// Execute the statement processing phase
    ///
    /// Takes the preparation context from `BodyPreparationPhase` and processes each statement,
    /// applying transformations and adding module attributes for exported symbols.
    pub(crate) fn execute(
        prep_context: BodyPreparationContext<'_>,
        bundler: &Bundler<'_>,
        ctx: &ModuleTransformContext<'_>,
        state: &mut InitFunctionState,
    ) {
        // Call the extracted function from module_transformer
        crate::code_generator::module_transformer::process_statements_for_init_function(
            prep_context.processed_body,
            bundler,
            ctx,
            prep_context.all_is_referenced,
            &prep_context.vars_used_by_exported_functions,
            prep_context.module_scope_symbols,
            &prep_context.builtin_locals,
            state.lifted_names.as_ref(),
            &state.inlined_import_bindings,
            &mut state.body,
            &mut state.initialized_lifted_globals,
        );
    }
}
