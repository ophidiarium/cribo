//! AST Builder Module for Synthetic Node Creation
//!
//! This module provides a centralized set of factory functions for creating synthetic
//! Abstract Syntax Tree (AST) nodes that are not originating directly from parsed source files.
//!
//! ## Purpose
//!
//! By providing a consistent set of factory functions, this module:
//! - **Improves Consistency**: Ensures all generated nodes have uniform properties
//! - **Enhances Readability**: Replaces verbose, manual node construction with concise,
//!   intention-revealing function calls
//! - **Simplifies Maintenance**: Allows future changes to AST construction to be made in a single,
//!   centralized location
//!
//! ## Design Principles
//!
//! All synthetic nodes created by this module use:
//! - `TextRange::default()` to signify their generated nature
//! - `AtomicNodeIndex::dummy()` for node indexing
//!
//! These factory functions assume valid inputs from callers and perform no runtime validation,
//! consistent with the underlying `ruff_python_ast` library design.
//!
//! ## Module Organization
//!
//! The module is organized into submodules by AST node type:
//! - `expressions`: Factory functions for creating expression nodes
//! - `statements`: Factory functions for creating statement nodes
//! - `other`: Factory functions for creating auxiliary AST nodes (aliases, keywords, etc.)
//!
//! ## Usage Examples
//!
//! ```rust
//! use ruff_python_ast::ExprContext;
//!
//! use crate::ast_builder::{expressions, other, statements};
//!
//! // Create a simple name expression: `module_name`
//! let name_expr = expressions::name("module_name", ExprContext::Load);
//!
//! // Create an assignment: `x = 42`
//! let assignment = statements::simple_assign("x", expressions::string_literal("42"));
//!
//! // Create an import alias: `import foo as bar`
//! let alias = other::alias("foo", Some("bar"));
//! ```

pub mod expressions;
pub mod other;
pub mod statements;

// Re-export commonly used functions for convenience
