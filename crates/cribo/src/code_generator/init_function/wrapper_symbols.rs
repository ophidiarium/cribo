//! Wrapper symbol setup phase for init function transformation
//!
//! This phase creates placeholder assignments for symbols from wrapper modules
//! that will be properly initialized later.

use log::debug;

use super::state::InitFunctionState;
use crate::{ast_builder, code_generator::bundler::Bundler};

/// Phase responsible for setting up placeholder assignments for wrapper module symbols
pub(crate) struct WrapperSymbolSetupPhase;

impl WrapperSymbolSetupPhase {
    /// Create placeholder assignments for wrapper module symbols
    ///
    /// This phase:
    /// 1. Creates a `types.SimpleNamespace()` placeholder for each wrapper module symbol
    /// 2. Adds the placeholder as a module attribute for visibility
    ///
    /// These symbols will be properly assigned later when wrapper modules are initialized,
    /// but we need them to exist in the local scope (not as module attributes yet).
    /// We use a sentinel object that can have attributes set on it.
    pub(crate) fn execute(_bundler: &Bundler<'_>, state: &mut InitFunctionState) {
        // Note: deferred imports functionality has been removed
        // Import alias assignments were previously added here

        // Add placeholder assignments for wrapper module symbols
        for (symbol_name, _value_name) in &state.wrapper_module_symbols_global_only {
            debug!("Adding placeholder assignment for wrapper module symbol '{symbol_name}'");

            // Create assignment: symbol_name = types.SimpleNamespace()
            // This creates a placeholder that can have attributes set on it
            state.body.push(ast_builder::statements::simple_assign(
                symbol_name,
                ast_builder::expressions::call(
                    ast_builder::expressions::simple_namespace_ctor(),
                    vec![],
                    vec![],
                ),
            ));

            // Also add as module attribute so it's visible in vars(__cribo_module)
            state.body.push(
                crate::code_generator::module_registry::create_module_attr_assignment(
                    crate::code_generator::module_transformer::SELF_PARAM,
                    symbol_name,
                ),
            );
        }
    }
}
