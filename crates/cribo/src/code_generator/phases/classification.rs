//! Classification Phase
//!
//! This phase handles module classification and registration, including:
//! - Classifying modules as inlinable or wrapper
//! - Tracking module exports
//! - Registering wrapper modules with synthetic names
//! - Managing inlined module set

use std::path::PathBuf;

use ruff_python_ast::ModModule;

use crate::{
    analyzers::module_classifier::{ClassificationResult, ModuleClassifier},
    code_generator::bundler::Bundler,
    resolver::ModuleId,
    types::FxIndexMap,
};

/// Classification phase handler (stateless)
#[derive(Default)]
pub(crate) struct ClassificationPhase;

impl ClassificationPhase {
    /// Create a new classification phase
    pub(crate) const fn new() -> Self {
        Self
    }

    /// Execute the classification phase
    ///
    /// This method:
    /// 1. Creates a `ModuleClassifier` with the current bundler state
    /// 2. Classifies modules into inlinable vs wrapper categories
    /// 3. Stores classification results in the bundler
    /// 4. Tracks inlined modules and their exports
    /// 5. Registers wrapper modules with synthetic names
    ///
    /// Returns the `ClassificationResult` containing inlinable modules, wrapper modules,
    /// and module export information.
    pub(crate) fn execute(
        &self,
        bundler: &mut Bundler<'_>,
        modules: &FxIndexMap<ModuleId, (ModModule, PathBuf, String)>,
        python_version: u8,
    ) -> ClassificationResult {
        // Classify modules into inlinable and wrapper modules
        let classifier = ModuleClassifier::new(
            bundler.resolver,
            bundler.entry_is_package_init_or_main,
            bundler.namespace_imported_modules.clone(),
            bundler.circular_modules.clone(),
        );

        let classification = classifier.classify_modules(modules, python_version);

        // Store modules with explicit __all__ declarations
        bundler
            .modules_with_explicit_all
            .clone_from(&classification.modules_with_explicit_all);

        // Track inlined modules and store their exports
        Self::track_inlined_modules(bundler, &classification);

        // Register wrapper modules with synthetic names
        Self::register_wrapper_modules(bundler, &classification);

        classification
    }

    /// Track inlined modules and store their exports
    fn track_inlined_modules(bundler: &mut Bundler<'_>, classification: &ClassificationResult) {
        for (module_id, _, _, _) in &classification.inlinable_modules {
            bundler.inlined_modules.insert(*module_id);

            // Store module exports for inlined modules
            bundler.module_exports.insert(
                *module_id,
                classification
                    .module_exports_map
                    .get(module_id)
                    .cloned()
                    .flatten(),
            );
        }

        log::debug!("Tracked {} inlined modules", bundler.inlined_modules.len());
        debug_assert!(
            bundler
                .inlined_modules
                .is_disjoint(&bundler.wrapper_modules),
            "inlined_modules and wrapper_modules must be disjoint"
        );
    }

    /// Register wrapper modules with synthetic names and init functions
    fn register_wrapper_modules(bundler: &mut Bundler<'_>, classification: &ClassificationResult) {
        for (module_id, _ast, _module_path, content_hash) in &classification.wrapper_modules {
            // Store module exports
            bundler.module_exports.insert(
                *module_id,
                classification
                    .module_exports_map
                    .get(module_id)
                    .cloned()
                    .flatten(),
            );

            // Register module with synthetic name and init function
            let module_name = bundler
                .resolver
                .get_module_name(*module_id)
                .expect("Module name must exist for ModuleId");

            crate::code_generator::module_registry::register_module(
                *module_id,
                &module_name,
                content_hash,
                &mut bundler.module_synthetic_names,
                &mut bundler.module_init_functions,
            );

            // Track wrapper membership for downstream resolution logic
            bundler.wrapper_modules.insert(*module_id);

            log::debug!(
                "Registered wrapper module '{module_name}' with synthetic name and init function"
            );
        }

        log::debug!(
            "Registered {} wrapper modules",
            classification.wrapper_modules.len()
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{FxIndexMap, FxIndexSet};

    #[test]
    fn test_classification_result_structure() {
        // Verify ClassificationResult can be constructed
        let result = ClassificationResult {
            inlinable_modules: vec![],
            wrapper_modules: vec![],
            module_exports_map: FxIndexMap::default(),
            modules_with_explicit_all: FxIndexSet::default(),
        };

        assert!(result.inlinable_modules.is_empty());
        assert!(result.wrapper_modules.is_empty());
        assert!(result.module_exports_map.is_empty());
        assert!(result.modules_with_explicit_all.is_empty());
    }

    #[test]
    fn test_classification_result_with_data() {
        use ruff_python_ast::AtomicNodeIndex;
        use ruff_text_size::TextRange;

        let mut module_exports_map = FxIndexMap::default();
        module_exports_map.insert(ModuleId::ENTRY, Some(vec!["foo".to_owned()]));

        // Create an empty ModModule
        let empty_module = ModModule {
            node_index: AtomicNodeIndex::NONE,
            range: TextRange::default(),
            body: vec![],
        };

        let result = ClassificationResult {
            inlinable_modules: vec![(
                ModuleId::ENTRY,
                empty_module,
                PathBuf::new(),
                "hash".to_owned(),
            )],
            wrapper_modules: vec![],
            module_exports_map: module_exports_map.clone(),
            modules_with_explicit_all: FxIndexSet::default(),
        };

        assert_eq!(result.inlinable_modules.len(), 1);
        assert_eq!(result.wrapper_modules.len(), 0);
        assert_eq!(result.module_exports_map.len(), 1);
        assert_eq!(
            result.module_exports_map.get(&ModuleId::ENTRY),
            Some(&Some(vec!["foo".to_owned()]))
        );
    }

    #[test]
    fn test_modules_with_explicit_all_tracking() {
        let mut explicit_all = FxIndexSet::default();
        explicit_all.insert(ModuleId::ENTRY);
        explicit_all.insert(ModuleId::new(1));

        let result = ClassificationResult {
            inlinable_modules: vec![],
            wrapper_modules: vec![],
            module_exports_map: FxIndexMap::default(),
            modules_with_explicit_all: explicit_all.clone(),
        };

        assert_eq!(result.modules_with_explicit_all.len(), 2);
        assert!(result.modules_with_explicit_all.contains(&ModuleId::ENTRY));
        assert!(result.modules_with_explicit_all.contains(&ModuleId::new(1)));
    }

    #[test]
    fn test_classification_separates_inlinable_and_wrapper() {
        use ruff_python_ast::AtomicNodeIndex;
        use ruff_text_size::TextRange;

        // Create empty ModModule instances
        let empty_module1 = ModModule {
            node_index: AtomicNodeIndex::NONE,
            range: TextRange::default(),
            body: vec![],
        };
        let empty_module2 = ModModule {
            node_index: AtomicNodeIndex::NONE,
            range: TextRange::default(),
            body: vec![],
        };

        let result = ClassificationResult {
            inlinable_modules: vec![(
                ModuleId::ENTRY,
                empty_module1,
                PathBuf::new(),
                "hash1".to_owned(),
            )],
            wrapper_modules: vec![(
                ModuleId::new(1),
                empty_module2,
                PathBuf::new(),
                "hash2".to_owned(),
            )],
            module_exports_map: FxIndexMap::default(),
            modules_with_explicit_all: FxIndexSet::default(),
        };

        assert_eq!(result.inlinable_modules.len(), 1);
        assert_eq!(result.wrapper_modules.len(), 1);

        // Verify they're different modules
        let inlinable_id = result.inlinable_modules[0].0;
        let wrapper_id = result.wrapper_modules[0].0;
        assert_ne!(inlinable_id, wrapper_id);
    }
}
