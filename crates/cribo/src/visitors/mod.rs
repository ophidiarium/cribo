//! AST visitor implementations for Cribo
//!
//! This module contains visitor patterns for traversing Python AST nodes,
//! enabling comprehensive import discovery and AST transformations.

mod export_collector;
mod import_discovery;
mod local_var_collector;
mod side_effect_detector;
pub(crate) mod symbol_collector;
mod symbol_usage_visitor;
pub(crate) mod utils;
mod variable_collector;

pub(crate) use export_collector::ExportCollector;
pub(crate) use import_discovery::{
    DiscoveredImport, ImportDiscoveryVisitor, ImportLocation, ImportType, ScopeElement,
};
pub(crate) use local_var_collector::LocalVarCollector;
pub(crate) use side_effect_detector::{ExpressionSideEffectDetector, SideEffectDetector};
pub(crate) use symbol_usage_visitor::SymbolUsageVisitor;
pub(crate) use variable_collector::VariableCollector;
