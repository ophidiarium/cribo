//! Submodule handling phase for init function transformation
//!
//! This phase sets up submodules as attributes on the parent module, handling
//! both inlined and wrapper submodules appropriately.

use log::debug;

use super::state::InitFunctionState;
use crate::{
    code_generator::{bundler::Bundler, context::ModuleTransformContext},
    resolver::ModuleId,
    types::{FxIndexMap, FxIndexSet},
};

/// Phase responsible for setting up submodules as module attributes
pub(crate) struct SubmoduleHandlingPhase;

impl SubmoduleHandlingPhase {
    /// Set up submodules as attributes on this module
    ///
    /// This phase:
    /// 1. Collects all direct submodules (both bundled and inlined)
    /// 2. Deduplicates submodules that appear in both lists
    /// 3. Checks for conflicts with imported symbols
    /// 4. For inlined submodules in wrapper context: binds existing namespace objects
    /// 5. For inlined submodules in non-wrapper context: creates namespace objects
    /// 6. For wrapper submodules: skips (will be set up when initialized)
    ///
    /// **NOTE**: This phase runs after Statement Processing to ensure submodule
    /// namespace objects are available for any references in later phases.
    pub(crate) fn execute(
        bundler: &Bundler<'_>,
        ctx: &ModuleTransformContext<'_>,
        symbol_renames: &FxIndexMap<ModuleId, FxIndexMap<String, String>>,
        state: &mut InitFunctionState,
    ) {
        // Set submodules as attributes on this module for later reference
        let current_module_prefix = format!("{}.", ctx.module_name);
        let mut submodules_to_add = Vec::new();

        // Collect all direct submodules from bundled_modules
        Self::collect_submodules_from_set(
            bundler,
            &bundler.bundled_modules,
            &current_module_prefix,
            &mut submodules_to_add,
        );

        // Also collect from inlined_modules
        Self::collect_submodules_from_set(
            bundler,
            &bundler.inlined_modules,
            &current_module_prefix,
            &mut submodules_to_add,
        );

        // Deduplicate: a submodule can appear in both bundled_modules and inlined_modules
        Self::deduplicate_submodules(&mut submodules_to_add);

        debug!(
            "Submodules to add for {}: {:?}",
            ctx.module_name, submodules_to_add
        );

        // Now add the (deduplicated) submodules as attributes
        for (full_name, relative_name) in submodules_to_add {
            Self::add_submodule_attribute(
                bundler,
                ctx,
                symbol_renames,
                &full_name,
                &relative_name,
                state,
            );
        }
    }

    /// Collect direct submodules from a given module set
    fn collect_submodules_from_set(
        bundler: &Bundler<'_>,
        module_set: &FxIndexSet<ModuleId>,
        current_module_prefix: &str,
        submodules_to_add: &mut Vec<(String, String)>,
    ) {
        for module_id in module_set {
            let module_name = bundler
                .resolver
                .get_module_name(*module_id)
                .expect("Module name should exist");

            if let Some(relative_name) = module_name.strip_prefix(current_module_prefix) {
                // Only handle direct children, not nested submodules
                if !relative_name.contains('.') {
                    submodules_to_add.push((module_name.clone(), relative_name.to_owned()));
                }
            }
        }
    }

    /// Deduplicate submodules that appear in both bundled and inlined sets
    ///
    /// A submodule can appear in both when it is first marked for bundling then later
    /// inlined during transformation decisions. We keep the first occurrence to preserve
    /// original relative ordering.
    fn deduplicate_submodules(submodules_to_add: &mut Vec<(String, String)>) {
        let mut seen: FxIndexSet<String> = FxIndexSet::default();
        submodules_to_add.retain(|(full_name, _)| seen.insert(full_name.clone()));
    }

    /// Add a submodule as an attribute on the parent module
    fn add_submodule_attribute(
        bundler: &Bundler<'_>,
        ctx: &ModuleTransformContext<'_>,
        symbol_renames: &FxIndexMap<ModuleId, FxIndexMap<String, String>>,
        full_name: &str,
        relative_name: &str,
        state: &mut InitFunctionState,
    ) {
        // CRITICAL: Check if this wrapper module already imports a symbol with the same name
        // as the submodule. If it does, skip setting the submodule namespace to avoid
        // overwriting the imported symbol.
        if state.imported_symbols.contains(relative_name) {
            debug!(
                "Skipping submodule namespace assignment for {full_name} because symbol \
                 '{relative_name}' is already imported"
            );
            return;
        }

        debug!(
            "Setting submodule {} as attribute {} on {}",
            full_name, relative_name, ctx.module_name
        );

        // Check if this is an inlined module
        let is_inlined = bundler
            .get_module_id(full_name)
            .is_some_and(|id| bundler.inlined_modules.contains(&id));

        if is_inlined {
            Self::handle_inlined_submodule(
                bundler,
                ctx,
                symbol_renames,
                full_name,
                relative_name,
                state,
            );
        } else {
            // For wrapped submodules, we'll set them up later when they're initialized
            // For now, just skip - the parent module will get the submodule reference
            // when the submodule's init function is called
            debug!("Skipping wrapped submodule {full_name} - will be set up when initialized");
        }
    }

    /// Handle an inlined submodule
    fn handle_inlined_submodule(
        bundler: &Bundler<'_>,
        ctx: &ModuleTransformContext<'_>,
        symbol_renames: &FxIndexMap<ModuleId, FxIndexMap<String, String>>,
        full_name: &str,
        relative_name: &str,
        state: &mut InitFunctionState,
    ) {
        // Check if we're inside a wrapper function context
        if ctx.is_wrapper_body {
            // Inside wrapper: bind existing global namespace object
            Self::bind_existing_namespace(full_name, relative_name, state);
        } else {
            // Non-wrapper context: create the namespace
            Self::create_inlined_namespace(
                bundler,
                full_name,
                relative_name,
                symbol_renames,
                state,
            );
        }
    }

    /// Bind an existing global namespace object to module attribute
    ///
    /// **IMPORTANT**: This references a namespace variable (e.g., `package___version__`)
    /// that MUST already exist at the global scope. These namespace objects are
    /// pre-created earlier in the bundling pipeline via
    /// `namespace_manager::generate_submodule_attributes_with_exclusions()`.
    fn bind_existing_namespace(
        full_name: &str,
        relative_name: &str,
        state: &mut InitFunctionState,
    ) {
        debug!(
            "Binding existing global namespace for inlined submodule '{full_name}' inside wrapper \
             module"
        );

        let namespace_var =
            crate::code_generator::module_registry::sanitize_module_name_for_identifier(full_name);

        // Example: module.submodule = pkg_submodule
        state.body.push(
            crate::code_generator::module_registry::create_module_attr_assignment_with_value(
                crate::code_generator::module_transformer::SELF_PARAM,
                relative_name,
                &namespace_var,
            ),
        );
    }

    /// Create namespace for inlined submodule
    fn create_inlined_namespace(
        bundler: &Bundler<'_>,
        full_name: &str,
        relative_name: &str,
        symbol_renames: &FxIndexMap<ModuleId, FxIndexMap<String, String>>,
        state: &mut InitFunctionState,
    ) {
        debug!("Creating namespace for inlined submodule '{full_name}' in non-wrapper context");

        // Use the existing helper function from module_transformer
        let create_namespace_stmts =
            crate::code_generator::module_transformer::create_namespace_for_inlined_submodule(
                bundler,
                full_name,
                relative_name,
                crate::code_generator::module_transformer::SELF_PARAM,
                symbol_renames,
            );

        state.body.extend(create_namespace_stmts);
    }
}
