//! Wildcard import processing phase for init function transformation
//!
//! This phase processes wildcard imports and adds module attributes for exported symbols.
//! This is CRITICAL and must happen BEFORE processing the body, as the body may contain
//! code that accesses these symbols via `vars(__cribo_module)` or `locals()`.

use log::debug;

use super::state::InitFunctionState;
use crate::{code_generator::bundler::Bundler, types::FxIndexSet};

/// Phase responsible for processing wildcard imports
pub(crate) struct WildcardImportPhase;

impl WildcardImportPhase {
    /// Process wildcard imports and add module attributes
    ///
    /// This phase:
    /// 1. Deduplicates and sorts wildcard imports for deterministic output
    /// 2. For each wildcard-imported symbol, adds module attribute assignments
    /// 3. Handles symbols from both inlined modules (accessed via namespace) and non-inlined
    ///    modules (bare symbols)
    ///
    /// **CRITICAL**: This must happen BEFORE processing the body, as the body may contain
    /// code that accesses these symbols via `vars(__cribo_module)` or `locals()`
    /// (e.g., the setattr pattern used by httpx and similar libraries).
    pub(crate) fn execute(
        bundler: &Bundler<'_>,
        ctx: &crate::code_generator::context::ModuleTransformContext<'_>,
        state: &mut InitFunctionState,
    ) {
        // Dedup and sort wildcard imports for deterministic output
        let mut wildcard_attrs: Vec<(String, String, Option<String>)> = state
            .imports_from_inlined
            .iter()
            .cloned()
            .collect::<FxIndexSet<_>>()
            .into_iter()
            .collect();
        wildcard_attrs.sort_by(|a, b| a.0.cmp(&b.0)); // Sort by exported name

        for (exported_name, value_name, source_module) in wildcard_attrs {
            if bundler.should_export_symbol(&exported_name, ctx.module_name) {
                // Build the value expression: either a simple Name or Attribute access
                let value_expr = source_module.as_ref().map_or_else(
                    || {
                        // Direct name reference
                        crate::ast_builder::expressions::name(
                            &value_name,
                            ruff_python_ast::ExprContext::Load,
                        )
                    },
                    |module| {
                        // Access through the inlined module's namespace as an Attribute node
                        let sanitized =
                            crate::code_generator::module_registry::sanitize_module_name_for_identifier(
                                module,
                            );
                        crate::ast_builder::expressions::attribute(
                            crate::ast_builder::expressions::name(
                                &sanitized,
                                ruff_python_ast::ExprContext::Load,
                            ),
                            &value_name,
                            ruff_python_ast::ExprContext::Load,
                        )
                    },
                );

                // Create assignment: self.exported_name = value_expr
                state.body.push(crate::ast_builder::statements::assign(
                    vec![crate::ast_builder::expressions::attribute(
                        crate::ast_builder::expressions::name(
                            crate::code_generator::module_transformer::SELF_PARAM,
                            ruff_python_ast::ExprContext::Load,
                        ),
                        &exported_name,
                        ruff_python_ast::ExprContext::Store,
                    )],
                    value_expr,
                ));

                debug!(
                    "Added wildcard-imported symbol '{exported_name}' as module attribute for '{}'",
                    ctx.module_name
                );
            }
        }
    }
}
