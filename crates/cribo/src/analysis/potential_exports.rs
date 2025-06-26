//! Potential exports analysis for multi-pass symbol resolution
//!
//! This module implements the first pass of the multi-pass analysis architecture,
//! identifying all symbols that could potentially be exported from each module
//! before making any tree-shaking or bundling decisions.

use log::debug;
use rustc_hash::{FxHashMap, FxHashSet};

use crate::cribo_graph::{CriboGraph, ItemType, ModuleId};

/// Map of modules to their potential exports
#[derive(Debug, Clone, Default)]
pub struct PotentialExportsMap {
    /// Map from module ID to set of symbol names that could be exported
    exports: FxHashMap<ModuleId, FxHashSet<String>>,

    /// Map from module ID to symbols explicitly listed in __all__
    explicit_exports: FxHashMap<ModuleId, Vec<String>>,

    /// Map from module ID to whether it's a package __init__.py
    is_package_init: FxHashMap<ModuleId, bool>,
}

impl PotentialExportsMap {
    /// Create a new empty map
    pub fn new() -> Self {
        Self::default()
    }

    /// Analyze a graph to build the potential exports map
    pub fn from_graph(graph: &CriboGraph) -> Self {
        let mut map = Self::new();

        for (module_id, module_graph) in &graph.modules {
            let mut exports = FxHashSet::default();
            let mut explicit_exports = None;

            // Check if this is a package __init__.py
            let is_init = module_graph.module_name.ends_with("__init__");
            map.is_package_init.insert(*module_id, is_init);

            // Analyze each item in the module
            for item_data in module_graph.items.values() {
                match &item_data.item_type {
                    // Functions and classes are potential exports (unless private)
                    ItemType::FunctionDef { name } | ItemType::ClassDef { name } => {
                        // Skip private names (starting with _) unless they're dunder methods
                        if !name.starts_with('_') || name.starts_with("__") {
                            exports.insert(name.clone());
                        }
                    }

                    // Variable assignments are potential exports
                    ItemType::Assignment { targets } => {
                        // Check for __all__ assignments
                        if targets.contains(&"__all__".to_string()) {
                            // TODO: Parse the actual __all__ value from AST
                            // For now, we'll mark that this module has explicit exports
                            explicit_exports = Some(Vec::new());
                        } else {
                            for target in targets {
                                // Skip private names (starting with _)
                                if !target.starts_with('_') || target.starts_with("__") {
                                    exports.insert(target.clone());
                                }
                            }
                        }
                    }

                    // Imports can be re-exported
                    ItemType::Import { alias, .. } => {
                        if let Some(alias_name) = alias {
                            exports.insert(alias_name.clone());
                        }
                    }

                    ItemType::FromImport { names, .. } => {
                        for (name, alias) in names {
                            let export_name = alias.as_ref().unwrap_or(name);
                            exports.insert(export_name.clone());
                        }
                    }

                    _ => {}
                }

                // Also add any symbols from defined_symbols
                for symbol in &item_data.defined_symbols {
                    if !symbol.starts_with('_') || symbol.starts_with("__") {
                        exports.insert(symbol.clone());
                    }
                }

                // Add re-exported names
                for name in &item_data.reexported_names {
                    exports.insert(name.clone());
                }
            }

            map.exports.insert(*module_id, exports);

            if let Some(explicit) = explicit_exports {
                map.explicit_exports.insert(*module_id, explicit);
            }

            debug!(
                "Module {} has {} potential exports",
                module_graph.module_name,
                map.exports[module_id].len()
            );
        }

        map
    }

    /// Get potential exports for a module
    pub fn get_exports(&self, module_id: ModuleId) -> Option<&FxHashSet<String>> {
        self.exports.get(&module_id)
    }

    /// Check if a module has explicit __all__ exports
    pub fn has_explicit_exports(&self, module_id: ModuleId) -> bool {
        self.explicit_exports.contains_key(&module_id)
    }

    /// Get explicit __all__ exports for a module
    pub fn get_explicit_exports(&self, module_id: ModuleId) -> Option<&Vec<String>> {
        self.explicit_exports.get(&module_id)
    }

    /// Check if a symbol could be exported from a module
    pub fn could_export(&self, module_id: ModuleId, symbol: &str) -> bool {
        // If module has explicit exports, only those are exported
        if let Some(explicit) = self.get_explicit_exports(module_id) {
            return explicit.iter().any(|s| s == symbol);
        }

        // Otherwise, check potential exports
        self.exports
            .get(&module_id)
            .map(|exports| exports.contains(symbol))
            .unwrap_or(false)
    }

    /// Check if a module is a package __init__.py
    pub fn is_package_init(&self, module_id: ModuleId) -> bool {
        self.is_package_init
            .get(&module_id)
            .copied()
            .unwrap_or(false)
    }

    /// Get all modules that could export a given symbol
    pub fn find_exporters(&self, symbol: &str) -> Vec<ModuleId> {
        let mut exporters = Vec::new();

        for (module_id, exports) in &self.exports {
            if exports.contains(symbol) {
                // Check if it would actually be exported
                if self.could_export(*module_id, symbol) {
                    exporters.push(*module_id);
                }
            }
        }

        exporters
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cribo_graph::ItemData;

    #[test]
    fn test_potential_exports_functions_and_classes() {
        let mut graph = CriboGraph::new();
        let module_id = graph.add_module(
            "test_module".to_string(),
            std::path::PathBuf::from("test.py"),
        );

        let module = graph.modules.get_mut(&module_id).unwrap();

        // Add a function
        module.add_item(ItemData {
            item_type: ItemType::FunctionDef {
                name: "public_func".to_string(),
            },
            defined_symbols: ["public_func".into()].into_iter().collect(),
            ..Default::default()
        });

        // Add a class
        module.add_item(ItemData {
            item_type: ItemType::ClassDef {
                name: "PublicClass".to_string(),
            },
            defined_symbols: ["PublicClass".into()].into_iter().collect(),
            ..Default::default()
        });

        // Add a private function
        module.add_item(ItemData {
            item_type: ItemType::FunctionDef {
                name: "_private_func".to_string(),
            },
            defined_symbols: ["_private_func".into()].into_iter().collect(),
            ..Default::default()
        });

        let exports_map = PotentialExportsMap::from_graph(&graph);

        assert!(exports_map.could_export(module_id, "public_func"));
        assert!(exports_map.could_export(module_id, "PublicClass"));
        assert!(!exports_map.could_export(module_id, "_private_func"));
    }
}
