//! Wildcard import processing phase for init function transformation
//!
//! This phase processes wildcard imports and adds module attributes for exported symbols.
//! This is CRITICAL and must happen BEFORE processing the body, as the body may contain
//! code that accesses these symbols via `vars(__cribo_module)` or `locals()`.

use log::debug;

use super::{TransformError, state::InitFunctionState};
use crate::{code_generator::bundler::Bundler, types::FxIndexSet};

/// Phase responsible for processing wildcard imports
pub struct WildcardImportPhase;

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
    pub fn execute(
        bundler: &Bundler,
        ctx: &crate::code_generator::context::ModuleTransformContext,
        state: &mut InitFunctionState,
    ) -> Result<(), TransformError> {
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
                // If the symbol comes from an inlined module, access it through the module's
                // namespace
                let value_expr = if let Some(ref module) = source_module {
                    // Access through the inlined module's namespace
                    let sanitized =
                        crate::code_generator::module_registry::sanitize_module_name_for_identifier(
                            module,
                        );
                    format!("{sanitized}.{value_name}")
                } else {
                    value_name.clone()
                };

                state.body.push(
                    crate::code_generator::module_registry::create_module_attr_assignment_with_value(
                        crate::code_generator::module_transformer::SELF_PARAM,
                        &exported_name,
                        &value_expr,
                    ),
                );

                debug!(
                    "Added wildcard-imported symbol '{exported_name}' = '{value_expr}' as module \
                     attribute for '{}'",
                    ctx.module_name
                );
            }
        }

        Ok(())
    }
}
