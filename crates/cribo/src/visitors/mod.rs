//! AST visitor implementations for Cribo
//!
//! This module contains visitor patterns for traversing Python AST nodes,
//! enabling comprehensive import discovery and AST transformations.

mod import_discovery;
mod side_effect_detector;
mod symbol_collector;
mod variable_collector;

pub use import_discovery::{
    DiscoveredImport, ImportDiscoveryVisitor, ImportLocation, ImportType, ScopeElement,
};
pub use side_effect_detector::{ExpressionSideEffectDetector, SideEffectDetector};
pub use symbol_collector::SymbolCollector;
pub use variable_collector::VariableCollector;
