//! Init function transformation infrastructure
//!
//! This module contains the refactored implementation of module-to-init-function
//! transformation, decomposed into manageable phases.

mod body_preparation;
mod cleanup;
mod finalization;
mod import_analysis;
mod import_transformation;
mod initialization;
mod orchestrator;
mod state;
mod statement_processing;
mod submodules;
mod wildcard_imports;
mod wrapper_globals;
mod wrapper_symbols;

use std::fmt;

pub(crate) use body_preparation::BodyPreparationPhase;
pub(crate) use cleanup::CleanupPhase;
pub(crate) use finalization::FinalizationPhase;
pub(crate) use import_analysis::ImportAnalysisPhase;
pub(crate) use import_transformation::ImportTransformationPhase;
pub(crate) use initialization::InitializationPhase;
pub(crate) use orchestrator::InitFunctionBuilder;
pub(crate) use state::InitFunctionState;
pub(crate) use statement_processing::StatementProcessingPhase;
pub(crate) use submodules::SubmoduleHandlingPhase;
pub(crate) use wildcard_imports::WildcardImportPhase;
pub(crate) use wrapper_globals::WrapperGlobalsPhase;
pub(crate) use wrapper_symbols::WrapperSymbolSetupPhase;

/// Errors that can occur during init function transformation
#[derive(Debug)]
pub(crate) enum TransformError {
    /// Module ID not found
    ModuleIdNotFound { module_name: String },
    /// Init function name not found for wrapper module
    InitFunctionNotFound { module_id: String },
}

impl fmt::Display for TransformError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ModuleIdNotFound { module_name } => {
                write!(f, "Module ID not found for module '{module_name}'")
            }
            Self::InitFunctionNotFound { module_id } => {
                write!(
                    f,
                    "Init function name not found for wrapper module '{module_id}'"
                )
            }
        }
    }
}

impl std::error::Error for TransformError {}
