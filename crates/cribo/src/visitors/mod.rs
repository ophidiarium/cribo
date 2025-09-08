//! AST visitor implementations for Cribo
//!
//! This module contains visitor patterns for traversing Python AST nodes,
//! enabling comprehensive import discovery and AST transformations.

mod export_collector;
mod import_discovery;
mod local_var_collector;
mod side_effect_detector;
pub mod symbol_collector;
pub mod utils;
mod variable_collector;

pub use export_collector::ExportCollector;
pub use import_discovery::{
    DiscoveredImport, ImportDiscoveryVisitor, ImportLocation, ImportType, ScopeElement,
};
pub use local_var_collector::LocalVarCollector;
pub use side_effect_detector::{ExpressionSideEffectDetector, SideEffectDetector};
pub use variable_collector::VariableCollector;
