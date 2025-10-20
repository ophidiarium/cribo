//! Final cleanup phase for init function transformation
//!
//! This phase handles final elements: stdlib re-exports and remaining explicit imports
//! from inlined modules.

use log::debug;
use ruff_python_ast::{Expr, ExprContext, Stmt};

use super::state::InitFunctionState;
use crate::{ast_builder, code_generator::bundler::Bundler};

/// Phase responsible for final cleanup tasks
pub(crate) struct CleanupPhase;

impl CleanupPhase {
    /// Add final elements: stdlib re-exports and remaining imports
    ///
    /// This phase:
    /// 1. Adds stdlib re-exports to the module namespace
    /// 2. Adds explicit imports from inlined modules as module attributes (if not already added)
    ///
    /// **NOTE**: Wildcard imports (`imports_from_inlined`) were already handled earlier
    /// by the Wildcard Import Processing phase, so we only handle explicit imports here.
    pub(crate) fn execute(
        bundler: &Bundler<'_>,
        ctx: &crate::code_generator::context::ModuleTransformContext<'_>,
        state: &mut InitFunctionState,
    ) {
        // Add stdlib re-exports to the module namespace
        Self::add_stdlib_reexports(ctx, state);

        // Add explicit imports from inlined modules as module attributes
        Self::add_explicit_inlined_imports(bundler, ctx, state);
    }

    /// Add stdlib re-exports to the module namespace
    ///
    /// Creates assignments like: `_cribo_module.local_name = _cribo.module.symbol`
    fn add_stdlib_reexports(
        ctx: &crate::code_generator::context::ModuleTransformContext<'_>,
        state: &mut InitFunctionState,
    ) {
        for (local_name, proxy_path) in &state.stdlib_reexports {
            // Parse the proxy path to create the attribute access expression
            let parts: Vec<&str> = proxy_path.split('.').collect();
            let value_expr = ast_builder::expressions::dotted_name(&parts, ExprContext::Load);

            state.body.push(ast_builder::statements::assign(
                vec![ast_builder::expressions::attribute(
                    ast_builder::expressions::name(
                        crate::code_generator::module_transformer::SELF_PARAM,
                        ExprContext::Load,
                    ),
                    local_name,
                    ExprContext::Store,
                )],
                value_expr,
            ));

            debug!(
                "Added stdlib re-export to wrapper module '{}': {} = {}",
                ctx.module_name, local_name, proxy_path
            );
        }
    }

    /// Add explicit imports from inlined modules as module attributes
    ///
    /// For explicit imports from inlined modules that don't create assignments,
    /// we still need to set them as module attributes if they're exported.
    fn add_explicit_inlined_imports(
        bundler: &Bundler<'_>,
        ctx: &crate::code_generator::context::ModuleTransformContext<'_>,
        state: &mut InitFunctionState,
    ) {
        for imported_name in &state.inlined_import_bindings {
            if bundler.should_export_symbol(imported_name, ctx.module_name) {
                // Check if we already have a module attribute assignment for this
                let already_assigned = state.body.iter().any(|stmt| {
                    if let Stmt::Assign(assign) = stmt
                        && let [Expr::Attribute(attr)] = assign.targets.as_slice()
                        && let Expr::Name(name) = &*attr.value
                    {
                        return name.id.as_str()
                            == crate::code_generator::module_transformer::SELF_PARAM
                            && attr.attr == *imported_name;
                    }
                    false
                });

                if !already_assigned {
                    state.body.push(
                        crate::code_generator::module_registry::create_module_attr_assignment(
                            crate::code_generator::module_transformer::SELF_PARAM,
                            imported_name,
                        ),
                    );

                    debug!(
                        "Added explicit inlined import '{}' as module attribute for '{}'",
                        imported_name, ctx.module_name
                    );
                }
            }
        }
    }
}
