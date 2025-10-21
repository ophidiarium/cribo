//! Body preparation phase for init function transformation
//!
//! This phase prepares for body processing by analyzing the module and setting up
//! necessary state and declarations.

use log::debug;
use ruff_python_ast::{Expr, ModModule, Stmt};

use super::state::InitFunctionState;
use crate::{
    ast_builder,
    code_generator::{bundler::Bundler, context::ModuleTransformContext, expression_handlers},
    types::FxIndexSet,
};

/// Context data computed during body preparation
pub(crate) struct BodyPreparationContext<'a> {
    /// Whether `__all__` is referenced in the module body
    pub all_is_referenced: bool,
    /// Variables referenced by exported functions
    pub vars_used_by_exported_functions: FxIndexSet<String>,
    /// Module scope symbols from semantic analysis
    pub module_scope_symbols: Option<&'a FxIndexSet<String>>,
    /// Built-in names that will be assigned as local variables
    pub builtin_locals: FxIndexSet<String>,
    /// Processed body statements after recursive transformation
    pub processed_body: Vec<Stmt>,
}

/// Phase responsible for preparing the body for processing
pub(crate) struct BodyPreparationPhase;

impl BodyPreparationPhase {
    /// Prepare for body processing by analyzing the module
    ///
    /// This phase:
    /// 1. Checks if `__all__` is referenced in the module body
    /// 2. Collects variables used by exported functions
    /// 3. Gets module scope symbols from semantic bundler
    /// 4. Scans for built-in names that will be assigned as local variables
    /// 5. Processes the body recursively using bundler
    /// 6. Filters out circular init attempts
    /// 7. Declares lifted globals
    ///
    /// Returns a context with computed analysis results and the processed body.
    pub(crate) fn execute<'a>(
        bundler: &'a Bundler<'_>,
        ctx: &ModuleTransformContext<'a>,
        ast: &ModModule,
        state: &mut InitFunctionState,
        lifted_names: Option<&crate::types::FxIndexMap<String, String>>,
    ) -> BodyPreparationContext<'a> {
        // Check if __all__ is referenced in the module body
        let all_is_referenced = Self::check_all_referenced(ast, ctx);

        // Collect all variables that are referenced by exported functions
        let vars_used_by_exported_functions =
            Self::collect_vars_used_by_exported_functions(bundler, ctx, ast);

        // Get module scope symbols from semantic bundler
        let module_scope_symbols = Self::get_module_scope_symbols(bundler, ctx);

        // Scan the body to find all built-in names that will be assigned as local variables
        let builtin_locals = Self::scan_builtin_locals(ast, ctx);

        // Process the body with recursive approach
        let processed_body_raw =
            bundler.process_body_recursive(ast.body.clone(), ctx.module_name, module_scope_symbols);

        // Filter out accidental attempts to (re)initialize the entry package (__init__)
        let processed_body = Self::filter_circular_init_attempts(processed_body_raw);

        debug!(
            "Processing init function for module '{}', inlined_import_bindings: {:?}",
            ctx.module_name, state.inlined_import_bindings
        );
        debug!("Processed body has {} statements", processed_body.len());

        // Declare lifted globals FIRST if any
        Self::declare_lifted_globals(lifted_names, state);

        BodyPreparationContext {
            all_is_referenced,
            vars_used_by_exported_functions,
            module_scope_symbols,
            builtin_locals,
            processed_body,
        }
    }

    /// Check if `__all__` is referenced in the module body
    fn check_all_referenced(ast: &ModModule, ctx: &ModuleTransformContext<'_>) -> bool {
        let mut all_is_referenced = false;
        for stmt in &ast.body {
            // Skip checking __all__ assignment itself
            if let Stmt::Assign(assign) = stmt
                && let Some(name) = expression_handlers::extract_simple_assign_target(assign)
                && name == "__all__"
            {
                continue;
            } else if let Stmt::AnnAssign(ann_assign) = stmt
                && let Expr::Name(target) = ann_assign.target.as_ref()
                && target.id.as_str() == "__all__"
            {
                // Skip annotated assignments to __all__ as a "reference"
                continue;
            }
            // Check if __all__ is referenced in this statement
            if crate::visitors::VariableCollector::statement_references_variable(stmt, "__all__") {
                all_is_referenced = true;
                break;
            }
        }

        debug!(
            "Module '{}' __all__ is referenced: {}",
            ctx.module_name, all_is_referenced
        );
        all_is_referenced
    }

    /// Collect all variables that are referenced by exported functions
    fn collect_vars_used_by_exported_functions(
        bundler: &Bundler<'_>,
        ctx: &ModuleTransformContext<'_>,
        ast: &ModModule,
    ) -> FxIndexSet<String> {
        let mut vars = FxIndexSet::default();
        for stmt in &ast.body {
            if let Stmt::FunctionDef(func_def) = stmt
                && bundler.should_export_symbol(func_def.name.as_ref(), ctx.module_name)
            {
                // This function will be exported, collect variables it references
                crate::visitors::VariableCollector::collect_referenced_vars(
                    &func_def.body,
                    &mut vars,
                );
            }
        }

        debug!(
            "Module '{}' has {} vars used by exported functions",
            ctx.module_name,
            vars.len()
        );
        vars
    }

    /// Get module scope symbols from semantic bundler
    fn get_module_scope_symbols<'a>(
        bundler: &'a Bundler<'_>,
        ctx: &ModuleTransformContext<'a>,
    ) -> Option<&'a FxIndexSet<String>> {
        let conflict_resolver = ctx.conflict_resolver?;

        debug!(
            "Looking up module ID for '{}' in conflict resolver",
            ctx.module_name
        );

        // Use the central module registry for fast, reliable lookup
        let module_id = bundler
            .module_info_registry
            .and_then(|registry| {
                let id = registry.get_id_by_name(ctx.module_name);
                if id.is_some() {
                    debug!(
                        "Found module ID for '{}' using module registry",
                        ctx.module_name
                    );
                } else {
                    debug!("Module '{}' not found in module registry", ctx.module_name);
                }
                id
            })
            .or_else(|| {
                log::warn!("No module registry available for module ID lookup");
                None
            });

        if let Some(module_id) = module_id {
            if let Some(module_info) = conflict_resolver.get_module_info(module_id) {
                debug!(
                    "Found module-scope symbols for '{}': {:?}",
                    ctx.module_name, module_info.module_scope_symbols
                );
                return Some(&module_info.module_scope_symbols);
            }
            log::warn!(
                "No semantic info found for module '{}' (module_id: {:?})",
                ctx.module_name,
                module_id
            );
        } else {
            log::warn!(
                "Could not find module ID for '{}' in semantic bundler",
                ctx.module_name
            );
        }

        None
    }

    /// Scan the body to find all built-in names that will be assigned as local variables
    fn scan_builtin_locals(
        ast: &ModModule,
        ctx: &ModuleTransformContext<'_>,
    ) -> FxIndexSet<String> {
        let mut builtin_locals = FxIndexSet::default();
        for stmt in &ast.body {
            let target_opt = match stmt {
                Stmt::Assign(assign) if assign.targets.len() == 1 => {
                    if let Expr::Name(target) = &assign.targets[0] {
                        Some(target)
                    } else {
                        None
                    }
                }
                Stmt::AnnAssign(ann_assign) if ann_assign.value.is_some() => {
                    if let Expr::Name(target) = ann_assign.target.as_ref() {
                        Some(target)
                    } else {
                        None
                    }
                }
                _ => None,
            };

            if let Some(target) = target_opt
                && ruff_python_stdlib::builtins::is_python_builtin(
                    target.id.as_str(),
                    ctx.python_version,
                    false,
                )
            {
                debug!(
                    "Found built-in type '{}' that will be assigned as local variable in init \
                     function",
                    target.id
                );
                builtin_locals.insert(target.id.to_string());
            }
        }

        builtin_locals
    }

    /// Filter out accidental attempts to (re)initialize the entry package (__init__)
    fn filter_circular_init_attempts(processed_body_raw: Vec<Stmt>) -> Vec<Stmt> {
        processed_body_raw
            .into_iter()
            .filter(|stmt| {
                if let Stmt::Assign(assign) = stmt
                    && assign.targets.len() == 1
                    && let Expr::Name(target) = &assign.targets[0]
                    && target.id.as_str() == crate::python::constants::INIT_STEM
                    && let Expr::Call(call) = assign.value.as_ref()
                    && let Expr::Name(func_name) = call.func.as_ref()
                    && crate::code_generator::module_registry::is_init_function(
                        func_name.id.as_str(),
                    )
                {
                    debug!(
                        "Skipping entry package __init__ re-initialization inside wrapper init to \
                         avoid circular init"
                    );
                    return false;
                }
                true
            })
            .collect()
    }

    /// Declare lifted globals if any exist
    fn declare_lifted_globals(
        lifted_names: Option<&crate::types::FxIndexMap<String, String>>,
        state: &mut InitFunctionState,
    ) {
        // Declare lifted globals FIRST if any - they need to be declared before any usage
        // But we'll initialize them later after the original variables are defined
        if let Some(lifted_names) = lifted_names
            && !lifted_names.is_empty()
        {
            // Declare all lifted globals once (sorted) for deterministic output
            let mut lifted: Vec<&str> = lifted_names.values().map(String::as_str).collect();
            lifted.sort_unstable();

            debug!(
                "Declared {} lifted globals: {:?}",
                lifted_names.len(),
                &lifted
            );

            state.body.push(ast_builder::statements::global(lifted));
        }
    }
}
