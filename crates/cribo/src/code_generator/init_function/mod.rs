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

pub use body_preparation::BodyPreparationPhase;
pub use cleanup::CleanupPhase;
pub use finalization::FinalizationPhase;
pub use import_analysis::ImportAnalysisPhase;
pub use import_transformation::ImportTransformationPhase;
pub use initialization::InitializationPhase;
pub use state::InitFunctionState;
pub use statement_processing::StatementProcessingPhase;
pub use submodules::SubmoduleHandlingPhase;
pub use wildcard_imports::WildcardImportPhase;
pub use wrapper_globals::WrapperGlobalsPhase;
pub use wrapper_symbols::WrapperSymbolSetupPhase;

/// Errors that can occur during init function transformation
#[derive(Debug)]
#[allow(dead_code)] // Will be used as phases are extracted
pub enum TransformError {
    /// Module ID not found
    ModuleIdNotFound { module_name: String },
    /// Init function name not found for wrapper module
    InitFunctionNotFound { module_id: String },
    /// No appropriate statement processor found
    NoStatementProcessor,
    /// General transformation error
    General(String),
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
            Self::NoStatementProcessor => {
                write!(f, "No statement processor found for statement type")
            }
            Self::General(msg) => write!(f, "Transformation error: {msg}"),
        }
    }
}

impl std::error::Error for TransformError {}
