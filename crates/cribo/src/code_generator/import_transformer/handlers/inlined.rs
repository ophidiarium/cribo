use ruff_python_ast::{Expr, ExprContext, Stmt};

use crate::{
    ast_builder::{expressions, statements},
    code_generator::bundler::Bundler,
    types::{FxIndexMap, FxIndexSet},
};

/// Handle inlined module import transformations
pub struct InlinedHandler;

impl InlinedHandler {
    /// Check if importing from an inlined module
    pub(in crate::code_generator::import_transformer) fn is_importing_from_inlined_module(
        module_name: &str,
        bundler: &Bundler,
    ) -> bool {
        bundler
            .get_module_id(module_name)
            .is_some_and(|id| bundler.inlined_modules.contains(&id))
    }

    /// Create namespace call for inlined module with all its symbols
    pub(in crate::code_generator::import_transformer) fn create_namespace_call_for_inlined_module(
        module_name: &str,
        module_renames: Option<&FxIndexMap<String, String>>,
        bundler: &Bundler,
    ) -> Expr {
        // Create a types.SimpleNamespace with all the module's symbols
        let mut keywords = Vec::new();
        let mut seen_args = FxIndexSet::default();

        // Add all renamed symbols as keyword arguments, avoiding duplicates
        if let Some(renames) = module_renames {
            for (original_name, renamed_name) in renames {
                // Check if the renamed name was already added
                if seen_args.contains(renamed_name) {
                    log::debug!(
                        "Skipping duplicate namespace argument '{renamed_name}' (from \
                         '{original_name}') for module '{module_name}'"
                    );
                    continue;
                }

                // Check if this symbol survived tree-shaking
                let module_id = bundler
                    .get_module_id(module_name)
                    .expect("Module should exist");
                if !bundler.is_symbol_kept_by_tree_shaking(module_id, original_name) {
                    log::debug!(
                        "Skipping tree-shaken symbol '{original_name}' from namespace for module \
                         '{module_name}'"
                    );
                    continue;
                }

                seen_args.insert(renamed_name.clone());

                keywords.push(expressions::keyword(
                    Some(original_name),
                    expressions::name(renamed_name, ExprContext::Load),
                ));
            }
        }

        // Also check if module has module-level variables that weren't renamed
        if let Some(module_id) = bundler.get_module_id(module_name)
            && let Some(exports) = bundler.module_exports.get(&module_id)
            && let Some(export_list) = exports
        {
            for export in export_list {
                // Check if this export was already added as a renamed symbol
                let was_renamed =
                    module_renames.is_some_and(|renames| renames.contains_key(export));
                if !was_renamed && !seen_args.contains(export) {
                    // Check if this symbol survived tree-shaking
                    if !bundler.is_symbol_kept_by_tree_shaking(module_id, export) {
                        log::debug!(
                            "Skipping tree-shaken export '{export}' from namespace for module \
                             '{module_name}'"
                        );
                        continue;
                    }

                    // This export wasn't renamed and wasn't already added, add it directly
                    seen_args.insert(export.clone());
                    keywords.push(expressions::keyword(
                        Some(export),
                        expressions::name(export, ExprContext::Load),
                    ));
                }
            }
        }

        // Create types.SimpleNamespace(**kwargs) call
        expressions::call(expressions::simple_namespace_ctor(), vec![], keywords)
    }

    /// Create `local = namespace_var` if names differ
    pub(in crate::code_generator::import_transformer) fn alias_local_to_namespace_if_needed(
        local_name: &str,
        namespace_var: &str,
        result_stmts: &mut Vec<Stmt>,
    ) {
        if local_name == namespace_var {
            return;
        }
        log::debug!("  Creating immediate local alias: {local_name} = {namespace_var}");
        result_stmts.push(statements::simple_assign(
            local_name,
            expressions::name(namespace_var, ExprContext::Load),
        ));
    }
}
