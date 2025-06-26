//! Final bundle layout structure for declarative code generation
//!
//! This module provides the FinalBundleLayout structure that represents
//! the declarative final structure of the bundled code, replacing the
//! imperative ExecutionStep approach.

use indexmap::IndexMap;

use crate::cribo_graph::{ItemId, ModuleId};

/// Declarative representation of the final bundle structure
#[derive(Debug, Clone, Default)]
pub struct FinalBundleLayout {
    /// Future imports that must be hoisted to the very top
    pub future_imports: Vec<String>,

    /// Hoisted imports (stdlib and third-party)
    pub hoisted_imports: Vec<FinalHoistedImport>,

    /// Namespace object creations (for direct module imports)
    pub namespace_creations: Vec<NamespaceCreation>,

    /// Ordered list of (module_id, item_id) pairs to inline
    pub inlined_code: Vec<(ModuleId, ItemId)>,

    /// Namespace population steps (after all code is inlined)
    pub namespace_populations: Vec<NamespacePopulationStep>,
}

/// A hoisted import (stdlib or third-party)
#[derive(Debug, Clone)]
pub struct FinalHoistedImport {
    /// Import type
    pub import_type: HoistedImportType,
    /// Original source module
    pub source_module: Option<ModuleId>,
}

/// Type of hoisted import
#[derive(Debug, Clone)]
pub enum HoistedImportType {
    /// Direct import: `import module [as alias]`
    Direct {
        module: String,
        alias: Option<String>,
    },

    /// From import: `from module import symbol [as alias], ...`
    From {
        module: String,
        symbols: IndexMap<String, Option<String>>, // symbol -> alias
        level: u32,                                // relative import level (0 for absolute)
    },
}

/// Namespace object creation
#[derive(Debug, Clone)]
pub struct NamespaceCreation {
    /// Name of the namespace variable
    pub var_name: String,

    /// Module this namespace represents
    pub module_name: String,

    /// Exports that will be populated later
    pub exports: Vec<String>,
}

/// Step to populate a namespace after code is inlined
#[derive(Debug, Clone)]
pub struct NamespacePopulationStep {
    /// Namespace variable to populate
    pub namespace_var: String,

    /// Attribute name to set
    pub attribute: String,

    /// Symbol to assign to the attribute
    pub symbol: String,
}

impl FinalBundleLayout {
    /// Create a new empty layout
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a future import
    pub fn add_future_import(&mut self, import: String) {
        if !self.future_imports.contains(&import) {
            self.future_imports.push(import);
        }
    }

    /// Add a hoisted import
    pub fn add_hoisted_import(&mut self, import: FinalHoistedImport) {
        self.hoisted_imports.push(import);
    }

    /// Add a namespace creation
    pub fn add_namespace_creation(&mut self, creation: NamespaceCreation) {
        self.namespace_creations.push(creation);
    }

    /// Add an inlined code item
    pub fn add_inlined_code(&mut self, module_id: ModuleId, item_id: ItemId) {
        self.inlined_code.push((module_id, item_id));
    }

    /// Add a namespace population step
    pub fn add_namespace_population(&mut self, step: NamespacePopulationStep) {
        self.namespace_populations.push(step);
    }

    /// Check if the layout is empty
    pub fn is_empty(&self) -> bool {
        self.future_imports.is_empty()
            && self.hoisted_imports.is_empty()
            && self.namespace_creations.is_empty()
            && self.inlined_code.is_empty()
            && self.namespace_populations.is_empty()
    }
}

/// Builder for converting analysis results into FinalBundleLayout
pub struct FinalLayoutBuilder {
    layout: FinalBundleLayout,
}

impl FinalLayoutBuilder {
    /// Create a new builder
    pub fn new() -> Self {
        Self {
            layout: FinalBundleLayout::new(),
        }
    }

    /// Build the final layout
    pub fn build(self) -> FinalBundleLayout {
        self.layout
    }

    /// Add a future import
    pub fn with_future_import(mut self, import: String) -> Self {
        self.layout.add_future_import(import);
        self
    }

    /// Add multiple future imports
    pub fn with_future_imports(mut self, imports: Vec<String>) -> Self {
        for import in imports {
            self.layout.add_future_import(import);
        }
        self
    }

    /// Add a hoisted import
    pub fn with_hoisted_import(mut self, import: FinalHoistedImport) -> Self {
        self.layout.add_hoisted_import(import);
        self
    }

    /// Add inlined code items
    pub fn with_inlined_code(mut self, items: Vec<(ModuleId, ItemId)>) -> Self {
        for (module_id, item_id) in items {
            self.layout.add_inlined_code(module_id, item_id);
        }
        self
    }
}

impl Default for FinalLayoutBuilder {
    fn default() -> Self {
        Self::new()
    }
}
