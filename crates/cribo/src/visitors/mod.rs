//! AST visitor implementations for Cribo
//!
//! This module contains visitor patterns for traversing Python AST nodes,
//! enabling comprehensive import discovery and AST transformations.

mod import_discovery;
mod side_effect_detector;

pub use import_discovery::{DiscoveredImport, ImportDiscoveryVisitor, ImportLocation};
pub use side_effect_detector::{ExpressionSideEffectDetector, SideEffectDetector};
