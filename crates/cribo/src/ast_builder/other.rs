//! Auxiliary AST node factory functions
//!
//! This module provides factory functions for creating auxiliary AST nodes such as
//! aliases, keywords, arguments, and exception handlers. All nodes are created with
//! `TextRange::default()` and `AtomicNodeIndex::dummy()` to indicate their synthetic nature.

use ruff_python_ast::{Alias, AtomicNodeIndex, Identifier};
use ruff_text_size::TextRange;

/// Creates a synthetic identifier node.
///
/// # Arguments
/// * `name` - The identifier name
///
/// # Example
/// ```rust
/// // Creates an identifier: `foo`
/// let id = identifier("foo");
/// ```
pub fn identifier(name: &str) -> Identifier {
    Identifier::new(name, TextRange::default())
}

/// Creates an alias node for import statements.
///
/// # Arguments
/// * `name` - The name being imported
/// * `asname` - The alias name (None if no alias)
///
/// # Example
/// ```rust
/// // Creates: `foo as bar`
/// let alias = alias("foo", Some("bar"));
///
/// // Creates: `baz` (no alias)
/// let alias = alias("baz", None);
/// ```
pub fn alias(name: &str, asname: Option<&str>) -> Alias {
    Alias {
        name: identifier(name),
        asname: asname.map(identifier),
        range: TextRange::default(),
        node_index: AtomicNodeIndex::dummy(),
    }
}

// Note: The Arguments type in ruff_python_ast is for call arguments, not function parameters
// Function parameters would use a different structure (Parameters)
// For now, we'll skip implementing this until it's needed
