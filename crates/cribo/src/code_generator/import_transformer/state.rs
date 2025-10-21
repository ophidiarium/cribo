use ruff_python_ast::Stmt;

use crate::{
    code_generator::bundler::Bundler,
    types::{FxIndexMap, FxIndexSet},
};

/// Parameters for creating a `RecursiveImportTransformer`
#[derive(Debug)]
pub(crate) struct RecursiveImportTransformerParams<'a> {
    pub bundler: &'a Bundler<'a>,
    pub module_id: crate::resolver::ModuleId,
    pub symbol_renames: &'a FxIndexMap<crate::resolver::ModuleId, FxIndexMap<String, String>>,
    pub is_wrapper_init: bool,
    pub python_version: u8,
}

/// State for the recursive import transformer
pub(super) struct TransformerState<'a> {
    pub(super) bundler: &'a Bundler<'a>,
    pub(super) module_id: crate::resolver::ModuleId,
    pub(super) symbol_renames:
        &'a FxIndexMap<crate::resolver::ModuleId, FxIndexMap<String, String>>,
    /// Maps import aliases to their actual module names
    /// e.g., "`helper_utils`" -> "utils.helpers"
    pub(super) import_aliases: FxIndexMap<String, String>,
    /// Flag indicating if we're inside a wrapper module's init function
    pub(super) is_wrapper_init: bool,
    /// Track local variable assignments to avoid treating them as module aliases
    pub(super) local_variables: FxIndexSet<String>,
    /// Track variables that were assigned from `importlib.import_module()` of inlined modules
    /// Maps variable name to the inlined module name
    pub(super) importlib_inlined_modules: FxIndexMap<String, String>,
    /// Track if we created any types.SimpleNamespace calls
    pub(super) created_namespace_objects: bool,
    /// Track imports from wrapper modules that need to be rewritten
    /// Maps local name to (`wrapper_module`, `original_name`)
    pub(super) wrapper_module_imports: FxIndexMap<String, (String, String)>,
    /// Track which modules have already been populated with symbols in this transformation session
    /// This prevents duplicate namespace assignments when multiple imports reference the same
    /// module
    pub(super) populated_modules: FxIndexSet<crate::resolver::ModuleId>,
    /// Track which stdlib modules were actually imported in this module
    /// This prevents transforming references to stdlib modules that weren't imported
    pub(super) imported_stdlib_modules: FxIndexSet<String>,
    /// Python version for compatibility checks
    pub(super) python_version: u8,
    /// Track whether we're at module level (false when inside any local scope like function,
    /// class, etc.)
    pub(super) at_module_level: bool,
    /// Track names on the LHS of the current assignment while transforming its RHS.
    pub(super) current_assignment_targets: Option<FxIndexSet<String>>,
    /// Current function body being transformed (for compatibility with existing APIs)
    pub(super) current_function_body: Option<Vec<Stmt>>,
    /// Cached set of symbols used at runtime in the current function (for performance)
    pub(super) current_function_used_symbols: Option<FxIndexSet<String>>,
}

impl<'a> TransformerState<'a> {
    pub(super) fn new(params: &RecursiveImportTransformerParams<'a>) -> Self {
        Self {
            bundler: params.bundler,
            module_id: params.module_id,
            symbol_renames: params.symbol_renames,
            import_aliases: FxIndexMap::default(),
            is_wrapper_init: params.is_wrapper_init,
            local_variables: FxIndexSet::default(),
            importlib_inlined_modules: FxIndexMap::default(),
            created_namespace_objects: false,
            wrapper_module_imports: FxIndexMap::default(),
            populated_modules: FxIndexSet::default(),
            imported_stdlib_modules: FxIndexSet::default(),
            python_version: params.python_version,
            at_module_level: true,
            current_assignment_targets: None,
            current_function_body: None,
            current_function_used_symbols: None,
        }
    }

    /// Get the module name from the resolver
    pub(super) fn get_module_name(&self) -> String {
        self.bundler
            .resolver
            .get_module_name(self.module_id)
            .unwrap_or_else(|| format!("module#{}", self.module_id))
    }

    /// Check if this is the entry module
    pub(super) const fn is_entry_module(&self) -> bool {
        self.module_id.is_entry()
    }
}
