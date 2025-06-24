//! AST visitor implementations for Cribo
//!
//! This module contains visitor patterns for traversing Python AST nodes,
//! enabling comprehensive import discovery and AST transformations.

mod import_discovery;
mod no_ops_removal;
mod side_effect_detector;

pub use import_discovery::{
    DiscoveredImport, ExecutionContext, ImportDiscoveryVisitor, ImportLocation, ImportType,
    ScopeElement,
};
pub use no_ops_removal::NoOpsRemovalTransformer;
pub use side_effect_detector::{ExpressionSideEffectDetector, SideEffectDetector};
