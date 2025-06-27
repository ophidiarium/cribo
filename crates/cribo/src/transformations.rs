//! Transformation metadata for AST modifications
//!
//! This module defines the transformation plan that the analysis phase produces
//! and the bundle compiler executes. All AST modifications are declaratively
//! specified here rather than performed imperatively during analysis.

use ruff_python_ast::Stmt;
use ruff_text_size::TextRange;
use rustc_hash::FxHashMap;

use crate::cribo_graph::ItemId;

/// Metadata describing a transformation to be applied to an AST item
#[derive(Debug, Clone)]
pub enum TransformationMetadata {
    /// Stdlib import needs normalization
    /// Example: from typing import Any, List -> import typing
    StdlibImportRewrite {
        /// The canonical module name (e.g., "typing")
        canonical_module: String,
        /// Symbol mappings: (original, canonical) e.g., [("Any", "typing.Any")]
        symbols: Vec<(String, String)>,
    },

    /// Partial import removal - remove specific symbols from a from-import
    /// Example: from foo import One, Two, Three -> from foo import Two
    PartialImportRemoval {
        /// Symbols to keep: (name, optional_alias)
        remaining_symbols: Vec<(String, Option<String>)>,
        /// Symbols being removed (for debugging/logging)
        removed_symbols: Vec<String>,
    },

    /// Symbol usage needs rewriting (generic for all symbol transformations)
    /// Handles: qualifications (Any -> typing.Any), renames (foo -> _b_foo),
    /// attribute rewrites (j.dumps -> json.dumps)
    SymbolRewrite {
        /// Map of TextRange -> new text
        rewrites: FxHashMap<TextRange, String>,
    },

    /// Import needs moving for circular dependency resolution
    CircularDepImportMove {
        /// The scope (usually a function) to move the import into
        target_scope: ItemId,
        /// The import statement to move
        import_stmt: Stmt,
    },

    /// Import should be removed entirely
    RemoveImport {
        /// Reason for removal
        reason: RemovalReason,
    },
}

/// Reason for removing an import
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemovalReason {
    /// Import is never referenced in the code
    Unused,
    /// Import is only used in type annotations (when type stripping is enabled)
    TypeOnly,
    /// First-party import that will be inlined/bundled
    Bundled,
}

/// Priority for transformation execution order
impl TransformationMetadata {
    /// Get the priority of this transformation (lower number = higher priority)
    pub fn priority(&self) -> u32 {
        match self {
            TransformationMetadata::RemoveImport { .. } => 1, // Highest priority
            TransformationMetadata::CircularDepImportMove { .. } => 2,
            TransformationMetadata::StdlibImportRewrite { .. } => 3,
            TransformationMetadata::PartialImportRemoval { .. } => 4,
            TransformationMetadata::SymbolRewrite { .. } => 5, // Lowest priority
        }
    }
}

/// Sort transformations by priority for consistent execution order
pub fn sort_transformations(transformations: &mut Vec<TransformationMetadata>) {
    transformations.sort_by_key(|t| t.priority());
}
