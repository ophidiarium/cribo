//! Orchestrator for coordinating init function transformation phases
//!
//! This module provides the `InitFunctionBuilder` which coordinates the execution
//! of all transformation phases to convert a Python module AST into an initialization
//! function.

use ruff_python_ast::{ModModule, Stmt};

use super::{
    BodyPreparationPhase, CleanupPhase, FinalizationPhase, ImportAnalysisPhase,
    ImportTransformationPhase, InitFunctionState, InitializationPhase, SubmoduleHandlingPhase,
    TransformError, WildcardImportPhase, WrapperGlobalsPhase, WrapperSymbolSetupPhase,
};
use crate::{
    code_generator::{bundler::Bundler, context::ModuleTransformContext},
    resolver::ModuleId,
    types::FxIndexMap,
};

/// Builder for coordinating the multi-phase transformation of a module AST
/// into an initialization function
pub struct InitFunctionBuilder<'a> {
    bundler: &'a Bundler<'a>,
    ctx: &'a ModuleTransformContext<'a>,
    symbol_renames: &'a FxIndexMap<ModuleId, FxIndexMap<String, String>>,
}

impl<'a> InitFunctionBuilder<'a> {
    /// Create a new builder with the required context
    pub fn new(
        bundler: &'a Bundler<'a>,
        ctx: &'a ModuleTransformContext<'a>,
        symbol_renames: &'a FxIndexMap<ModuleId, FxIndexMap<String, String>>,
    ) -> Self {
        Self {
            bundler,
            ctx,
            symbol_renames,
        }
    }

    /// Build the initialization function by executing all transformation phases
    ///
    /// This method orchestrates the following phases in order:
    /// 1. Initialization - Add guards and handle globals lifting
    /// 2. Import Analysis - Analyze imports without modifying AST
    /// 3. Import Transformation - Transform imports in AST
    /// 4. Wrapper Symbol Setup - Create placeholder assignments
    /// 5. Wildcard Import Processing - Handle `from module import *`
    /// 6. Body Preparation - Analyze and process module body
    /// 7. Wrapper Globals Collection - Collect wrapper module globals
    /// 8. Statement Processing - Process each statement (INLINE for now)
    /// 9. Submodule Handling - Set up submodule attributes
    /// 10. Final Cleanup - Add re-exports and explicit imports
    /// 11. Finalization - Create the function statement
    pub fn build(self, mut ast: ModModule) -> Result<Stmt, TransformError> {
        let mut state = InitFunctionState::new();

        // Phase 1: Initialization
        InitializationPhase::execute(self.bundler, self.ctx, &mut ast, &mut state)?;

        // Phase 2: Import Analysis
        ImportAnalysisPhase::execute(
            self.bundler,
            self.ctx,
            &ast,
            self.symbol_renames,
            &mut state,
        )?;

        // Phase 3: Import Transformation
        ImportTransformationPhase::execute(
            self.bundler,
            self.ctx,
            &mut ast,
            self.symbol_renames,
            &mut state,
        )?;

        // Phase 4: Wrapper Symbol Setup
        WrapperSymbolSetupPhase::execute(self.bundler, &mut state)?;

        // Phase 5: Wildcard Import Processing
        WildcardImportPhase::execute(self.bundler, self.ctx, &mut state)?;

        // Phase 6: Body Preparation
        // Clone lifted_names to avoid borrow conflict
        let lifted_names_for_prep = state.lifted_names.clone();
        let prep_context = BodyPreparationPhase::execute(
            self.bundler,
            self.ctx,
            &ast,
            &mut state,
            &lifted_names_for_prep,
        )?;

        // Phase 7: Wrapper Globals Collection
        WrapperGlobalsPhase::execute(&prep_context.processed_body, &mut state)?;

        // Phase 8: Statement Processing
        // TODO: This phase is not yet extracted - currently remains inline in
        // transform_module_to_init_function. The 580-line statement processing loop
        // will be extracted in a future iteration. For now, the orchestrator calls
        // the original function which includes this phase inline.
        //
        // Once extracted, this will be:
        // StatementProcessingPhase::execute(
        //     prep_context.processed_body,
        //     self.bundler,
        //     self.ctx,
        //     &prep_context,
        //     &state.lifted_names,
        //     &mut state,
        // )?;

        // Phase 9: Submodule Handling
        SubmoduleHandlingPhase::execute(self.bundler, self.ctx, self.symbol_renames, &mut state)?;

        // Phase 10: Final Cleanup
        CleanupPhase::execute(self.bundler, self.ctx, &mut state)?;

        // Phase 11: Finalization
        FinalizationPhase::build_function_stmt(self.bundler, self.ctx, state)
    }
}
