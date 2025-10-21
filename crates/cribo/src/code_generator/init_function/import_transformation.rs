//! Import transformation phase for init function transformation
//!
//! This phase transforms all imports in the AST using `RecursiveImportTransformer`
//! and adds global declarations for bare/global symbols (non-inlined) and wrapper
//! placeholders. Symbols from inlined modules are accessed via their namespaces.

use log::debug;
use ruff_python_ast::ModModule;

use super::{TransformError, state::InitFunctionState};
use crate::{
    ast_builder,
    code_generator::{
        bundler::Bundler,
        context::ModuleTransformContext,
        import_transformer::{RecursiveImportTransformer, RecursiveImportTransformerParams},
    },
    resolver::ModuleId,
    types::{FxIndexMap, FxIndexSet},
};

/// Phase responsible for transforming imports in the module AST
pub(crate) struct ImportTransformationPhase;

impl ImportTransformationPhase {
    /// Transform imports using `RecursiveImportTransformer` and add global declarations
    ///
    /// This phase:
    /// 1. Creates and runs `RecursiveImportTransformer` to transform all import statements
    /// 2. Checks if namespace objects were created during transformation
    /// 3. Adds global declarations for bare/global symbols and wrapper placeholders (symbols from
    ///    inlined modules are accessed via namespaces, not globals)
    /// 4. Filters out tree-shaken symbols from global declarations
    ///
    /// Note: This phase mutates the AST, unlike the Import Analysis phase which only
    /// analyzes it.
    pub(crate) fn execute(
        bundler: &Bundler<'_>,
        ctx: &ModuleTransformContext<'_>,
        ast: &mut ModModule,
        symbol_renames: &FxIndexMap<ModuleId, FxIndexMap<String, String>>,
        state: &mut InitFunctionState,
    ) -> Result<(), TransformError> {
        let module_id = bundler.get_module_id(ctx.module_name).ok_or_else(|| {
            TransformError::ModuleIdNotFound {
                module_name: ctx.module_name.to_owned(),
            }
        })?;

        // Create the RecursiveImportTransformer
        // For wrapper modules, we don't need to defer imports since they run in their own scope
        let params = RecursiveImportTransformerParams {
            bundler,
            module_id,
            symbol_renames,
            is_wrapper_init: true, // This IS a wrapper init function
            python_version: ctx.python_version,
        };
        let mut transformer = RecursiveImportTransformer::new(&params);

        // Transform all imports in the AST
        transformer.transform_module(ast);

        // If namespace objects were created, we need types import
        // (though wrapper modules already have types import)
        if transformer.created_namespace_objects() {
            debug!(
                "Namespace objects were created in wrapper module, types import already present"
            );
        }

        // Add global declarations for symbols imported from inlined modules
        // This is necessary because the symbols are defined in the global scope
        // but we need to access them inside the init function
        // Also include wrapper module symbols that will be defined later
        if !state.imports_from_inlined.is_empty()
            || !state.wrapper_module_symbols_global_only.is_empty()
        {
            Self::add_global_declarations(bundler, state);
        }

        Ok(())
    }

    /// Add global declarations for symbols imported from inlined modules
    fn add_global_declarations(bundler: &Bundler<'_>, state: &mut InitFunctionState) {
        // Deduplicate by value name (what's actually in global scope) and sort for deterministic
        // output
        // Only add symbols from NON-inlined modules to globals (they exist as bare symbols)
        // Symbols from inlined modules will be accessed through their namespace
        let mut unique_imports: Vec<String> = state
            .imports_from_inlined
            .iter()
            .filter(|(_, _, source_module)| source_module.is_none())
            .map(|(_, value_name, _)| value_name.clone())
            .chain(
                state
                    .wrapper_module_symbols_global_only
                    .iter()
                    .map(|(_, value_name)| value_name.clone()),
            )
            .collect::<FxIndexSet<_>>()
            .into_iter()
            .collect();
        unique_imports.sort();

        // Filter out tree-shaken symbols
        if let Some(ref tree_shaking_keep) = bundler.tree_shaking_keep_symbols {
            // Use the pre-computed global set of kept symbols for efficient lookup.
            if let Some(ref all_kept_symbols) = bundler.kept_symbols_global {
                unique_imports.retain(|symbol| {
                    if all_kept_symbols.contains(symbol) {
                        if log::log_enabled!(log::Level::Debug) {
                            // This find is only executed when debug logging is enabled.
                            let module_name = tree_shaking_keep
                                .iter()
                                .find(|(_, symbols)| symbols.contains(symbol))
                                .and_then(|(id, _)| bundler.resolver.get_module_name(*id))
                                .unwrap_or_else(|| "unknown".to_owned());
                            debug!(
                                "Symbol '{symbol}' kept by tree-shaking from module \
                                 '{module_name}'"
                            );
                        }
                        true
                    } else {
                        debug!(
                            "Symbol '{symbol}' was removed by tree-shaking, excluding from global \
                             declaration"
                        );
                        false
                    }
                });
            }
        }

        if !unique_imports.is_empty() {
            debug!(
                "Adding global declaration for imported symbols from inlined modules: \
                 {unique_imports:?}"
            );
            state.body.push(ast_builder::statements::global(
                unique_imports.iter().map(String::as_str).collect(),
            ));
        }
    }
}
