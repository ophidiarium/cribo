//! Builder pattern for constructing BundlePlan

use super::{BundlePlan, HoistedImport, ImportRewrite, ImportRewriteAction, ModuleMetadata};
use crate::cribo_graph::{ItemId, ModuleId};

/// Builder for incrementally constructing a BundlePlan
#[derive(Debug)]
pub struct BundlePlanBuilder {
    plan: BundlePlan,
}

impl BundlePlanBuilder {
    /// Create a new builder
    pub fn new() -> Self {
        Self {
            plan: BundlePlan::new(),
        }
    }

    /// Add an import rewrite for circular dependency resolution
    pub fn add_import_rewrite(
        &mut self,
        module_id: ModuleId,
        import_item_id: ItemId,
        action: ImportRewriteAction,
    ) -> &mut Self {
        self.plan.add_import_rewrite(ImportRewrite {
            module_id,
            import_item_id,
            action,
        });
        self
    }

    /// Add a function-scoped import rewrite
    pub fn add_function_scoped_import(
        &mut self,
        module_id: ModuleId,
        import_item_id: ItemId,
        function_item_id: ItemId,
        function_name: String,
    ) -> &mut Self {
        self.add_import_rewrite(
            module_id,
            import_item_id,
            ImportRewriteAction::MoveToFunction {
                function_item_id,
                function_name,
            },
        )
    }

    /// Add a deferred import rewrite
    /// TODO: Implement deferred imports when needed
    pub fn add_deferred_import(
        &mut self,
        module_id: ModuleId,
        import_item_id: ItemId,
    ) -> &mut Self {
        // For now, treat deferred imports as regular imports
        // This functionality may need to be implemented differently
        let _ = (module_id, import_item_id);
        self
    }

    /// Add a lazy import rewrite
    pub fn add_lazy_import(
        &mut self,
        module_id: ModuleId,
        import_item_id: ItemId,
        lazy_var_name: String,
    ) -> &mut Self {
        self.add_import_rewrite(
            module_id,
            import_item_id,
            ImportRewriteAction::LazyImport { lazy_var_name },
        )
    }

    /// Set module metadata
    pub fn set_module_metadata(
        &mut self,
        module_id: ModuleId,
        metadata: ModuleMetadata,
    ) -> &mut Self {
        self.plan.set_module_metadata(module_id, metadata);
        self
    }

    /// Set module as inlinable
    pub fn set_module_inlinable(
        &mut self,
        module_id: ModuleId,
        has_side_effects: bool,
    ) -> &mut Self {
        self.set_module_metadata(
            module_id,
            ModuleMetadata {
                needs_init_wrapper: false,
                has_side_effects,
                ..Default::default()
            },
        )
    }

    /// Set module as wrapper
    pub fn set_module_wrapper(&mut self, module_id: ModuleId, has_side_effects: bool) -> &mut Self {
        self.set_module_metadata(
            module_id,
            ModuleMetadata {
                needs_init_wrapper: true,
                has_side_effects,
                ..Default::default()
            },
        )
    }

    /// Add hoisted import (Phase 2)
    pub fn add_hoisted_import(&mut self, import: HoistedImport) -> &mut Self {
        self.plan.hoisted_imports.push(import);
        self
    }

    /// Build the final plan
    pub fn build(self) -> BundlePlan {
        self.plan
    }
}

impl Default for BundlePlanBuilder {
    fn default() -> Self {
        Self::new()
    }
}
