//! Bundling Phases
//!
//! This module contains the individual phases that comprise the module bundling process.
//! Each phase is responsible for a specific aspect of bundling and produces a result
//! type that can be passed to subsequent phases.
//!
//! The phases are designed to be:
//! - **Testable**: Each phase can be tested in isolation
//! - **Composable**: Phases can be combined in different ways
//! - **Explicit**: Data dependencies between phases are visible through types

pub mod classification;
pub mod entry_module;
pub mod initialization;
pub mod post_processing;
pub mod processing;
