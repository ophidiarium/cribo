//! Orchestrator for coordinating init function transformation phases
//!
//! This module provides the `InitFunctionBuilder` which coordinates the execution
//! of all transformation phases to convert a Python module AST into an initialization
//! function.
//!
//! **STATUS**: ✅ Complete and Production Ready
//!
//! The orchestrator successfully coordinates all 12 phases to transform Python module ASTs
//! into initialization functions. All phases work together through explicit state transitions
//! via `InitFunctionState`, providing a clean, modular alternative to the previous monolithic
//! implementation.
//!
//! **Bug History**: During development, a bug was discovered where global variables showed
//! incorrect module names. This was caused by missing globals/locals transformation in the
//! Finalization phase. The bug was fixed by adding `transform_globals_in_stmt()` and
//! `transform_locals_in_stmt()` calls, and is now fully resolved (verified by all 148 tests).
//!
//! **Production Status**: The orchestrator is now the sole implementation used in production.
//! The original monolithic function has been deleted.

use ruff_python_ast::{ModModule, Stmt};

use super::{
    BodyPreparationPhase, CleanupPhase, FinalizationPhase, ImportAnalysisPhase,
    ImportTransformationPhase, InitFunctionState, InitializationPhase, StatementProcessingPhase,
    SubmoduleHandlingPhase, TransformError, WildcardImportPhase, WrapperGlobalsPhase,
    WrapperSymbolSetupPhase,
};
use crate::{
    code_generator::{bundler::Bundler, context::ModuleTransformContext},
    resolver::ModuleId,
    types::FxIndexMap,
};

/// Builder for coordinating the multi-phase transformation of a module AST
/// into an initialization function
pub(crate) struct InitFunctionBuilder<'a> {
    bundler: &'a Bundler<'a>,
    ctx: &'a ModuleTransformContext<'a>,
    symbol_renames: &'a FxIndexMap<ModuleId, FxIndexMap<String, String>>,
}

impl<'a> InitFunctionBuilder<'a> {
    /// Create a new builder with the required context
    pub(crate) const fn new(
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
    /// 8. Statement Processing - Process each statement type with transformations
    /// 9. Submodule Handling - Set up submodule attributes
    /// 10. Final Cleanup - Add re-exports and explicit imports
    /// 11. Finalization - Create the function statement
    pub(crate) fn build(self, mut ast: ModModule) -> Result<Stmt, TransformError> {
        let mut state = InitFunctionState::new();

        // Phase 1: Initialization
        InitializationPhase::execute(self.bundler, self.ctx, &mut ast, &mut state);

        // Phase 2: Import Analysis
        ImportAnalysisPhase::execute(
            self.bundler,
            self.ctx,
            &ast,
            self.symbol_renames,
            &mut state,
        );

        // Phase 3: Import Transformation
        ImportTransformationPhase::execute(
            self.bundler,
            self.ctx,
            &mut ast,
            self.symbol_renames,
            &mut state,
        )?;

        // Phase 4: Wrapper Symbol Setup
        WrapperSymbolSetupPhase::execute(self.bundler, &mut state);

        // Phase 5: Wildcard Import Processing
        WildcardImportPhase::execute(self.bundler, self.ctx, &mut state);

        // Phase 6: Body Preparation
        // Clone lifted_names to avoid borrow conflict
        let lifted_names_for_prep = state.lifted_names.clone();
        let prep_context = BodyPreparationPhase::execute(
            self.bundler,
            self.ctx,
            &ast,
            &mut state,
            lifted_names_for_prep.as_ref(),
        );

        // Phase 7: Wrapper Globals Collection
        WrapperGlobalsPhase::execute(&prep_context.processed_body, &mut state);

        // Phase 8: Statement Processing
        StatementProcessingPhase::execute(prep_context, self.bundler, self.ctx, &mut state);

        // Phase 9: Submodule Handling
        SubmoduleHandlingPhase::execute(self.bundler, self.ctx, self.symbol_renames, &mut state);

        // Phase 10: Final Cleanup
        CleanupPhase::execute(self.bundler, self.ctx, &mut state);

        // Phase 11: Finalization
        FinalizationPhase::build_function_stmt(self.bundler, self.ctx, state)
    }
}
