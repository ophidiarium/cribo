//! State container for init function transformation
//!
//! This module defines the `InitFunctionState` struct that holds all mutable state
//! accumulated during the transformation of a Python module into an init function.

use ruff_python_ast::Stmt;

use crate::types::{FxIndexMap, FxIndexSet};

/// State accumulated during init function transformation
///
/// This struct consolidates all the scattered local variables from the original
/// monolithic function into a single, explicit state container. This makes data
/// flow between phases clear and enables easier testing and debugging.
#[derive(Debug)]
pub(crate) struct InitFunctionState {
    /// Accumulated init function body statements
    pub body: Vec<Stmt>,

    /// Import tracking: symbols imported from inlined modules
    /// Format: (`exported_name`, `value_name`, `source_module`)
    /// - `exported_name`: The name as exported by the source module
    /// - `value_name`: The actual symbol name in global scope (may be renamed)
    /// - `source_module`: Optional module name the symbol comes from
    pub imports_from_inlined: Vec<(String, String, Option<String>)>,

    /// Local binding names created by explicit from-imports (asname if present)
    pub inlined_import_bindings: Vec<String>,

    /// Wrapper module symbols that need placeholders
    /// Format: (`symbol_name`, `value_name`)
    pub wrapper_module_symbols_global_only: Vec<(String, String)>,

    /// ALL imported symbols to avoid overwriting with submodule namespaces
    pub imported_symbols: FxIndexSet<String>,

    /// Stdlib symbols that need to be added to the module namespace
    /// Format: (`local_name`, `proxy_path`)
    pub stdlib_reexports: FxIndexSet<(String, String)>,

    /// Global variable management: lifted global variable names
    /// Maps original name -> lifted name
    pub lifted_names: Option<FxIndexMap<String, String>>,

    /// Track which lifted globals have been initialized
    pub initialized_lifted_globals: FxIndexSet<String>,
}

impl InitFunctionState {
    /// Create a new empty state container
    pub(crate) fn new() -> Self {
        Self {
            body: Vec::new(),
            imports_from_inlined: Vec::new(),
            inlined_import_bindings: Vec::new(),
            wrapper_module_symbols_global_only: Vec::new(),
            imported_symbols: FxIndexSet::default(),
            stdlib_reexports: FxIndexSet::default(),
            lifted_names: None,
            initialized_lifted_globals: FxIndexSet::default(),
        }
    }
}

impl Default for InitFunctionState {
    fn default() -> Self {
        Self::new()
    }
}
