use std::path::{Path, PathBuf};

/// `CriboGraph`: Advanced dependency graph implementation for Python bundling
///
/// This module provides a sophisticated dependency tracking system that combines:
/// - Fine-grained item-level tracking (inspired by Turbopack)
/// - Incremental update support (inspired by Rspack)
/// - Efficient graph algorithms using petgraph (inspired by Mako)
///
/// Key features:
/// - Statement/item level dependency tracking for precise tree shaking
/// - Incremental updates with partial graph modifications
/// - Cycle detection and handling
/// - Variable state tracking across scopes
/// - Side effect preservation
use anyhow::{Result, anyhow};
use petgraph::{
    algo::{is_cyclic_directed, tarjan_scc, toposort},
    graph::{DiGraph, NodeIndex},
};

use crate::{
    resolver::ModuleId,
    types::{FxIndexMap, FxIndexSet},
};

/// Unique identifier for an item within a module
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ItemId(u32);

impl ItemId {
    pub fn new(id: u32) -> Self {
        Self(id)
    }
}

/// Type of Python item (statement/definition)
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ItemType {
    /// Function definition
    FunctionDef { name: String },
    /// Class definition
    ClassDef { name: String },
    /// Variable assignment
    Assignment { targets: Vec<String> },
    /// Import statement
    Import {
        module: String,
        alias: Option<String>, // import module as alias
    },
    /// From import statement
    FromImport {
        module: String,
        names: Vec<(String, Option<String>)>, // (name, alias)
        level: u32,                           // relative import level
        is_star: bool,                        // from module import *
    },
    /// Module-level expression (side effect)
    Expression,
    /// If statement (for conditional imports)
    If { condition: String },
    /// Try-except block
    Try,
    /// Other statement types
    Other,
}

impl ItemType {
    /// Get the name of this item if it has one
    pub fn name(&self) -> Option<&str> {
        match self {
            ItemType::FunctionDef { name } | ItemType::ClassDef { name } => Some(name),
            _ => None,
        }
    }
}

/// Variable state tracking
#[derive(Debug, Clone)]
pub struct VarState {
    /// Items that write to this variable
    pub writers: Vec<ItemId>,
    /// Items that read this variable
    pub readers: Vec<ItemId>,
}

/// Data about a Python item (statement/definition)
#[derive(Debug, Clone)]
pub struct ItemData {
    /// Type of this item
    pub item_type: ItemType,
    /// Variables declared by this item
    pub var_decls: FxIndexSet<String>,
    /// Variables read by this item during execution
    pub read_vars: FxIndexSet<String>,
    /// Variables read eventually (e.g., inside function bodies)
    pub eventual_read_vars: FxIndexSet<String>,
    /// Variables written by this item
    pub write_vars: FxIndexSet<String>,
    /// Variables written eventually
    pub eventual_write_vars: FxIndexSet<String>,
    /// Whether this item has side effects
    pub has_side_effects: bool,
    /// For imports: the local names introduced by this import
    pub imported_names: FxIndexSet<String>,
    /// For re-exports: names that are explicitly re-exported
    pub reexported_names: FxIndexSet<String>,
    /// NEW: Top-level symbols defined by this item (for tree-shaking)
    pub defined_symbols: FxIndexSet<String>,
    /// NEW: Map of symbol -> other symbols it references (for tree-shaking)
    pub symbol_dependencies: FxIndexMap<String, FxIndexSet<String>>,
    /// NEW: Map of variable -> accessed attributes (for tree-shaking namespace access)
    /// e.g., {"greetings": ["message"]} for greetings.message
    pub attribute_accesses: FxIndexMap<String, FxIndexSet<String>>,
    /// For scoped items: the containing scope name (function or class name)
    pub containing_scope: Option<String>,
}

/// Fine-grained dependency graph for a single module
#[derive(Debug)]
pub struct ModuleDepGraph {
    /// Module identifier
    pub module_id: ModuleId,
    /// Module name (e.g., "utils.helpers")
    pub module_name: String,
    /// All items in this module
    pub items: FxIndexMap<ItemId, ItemData>,
    /// Items that are executed for side effects (in order)
    pub side_effect_items: Vec<ItemId>,
    /// Variable state tracking
    pub var_states: FxIndexMap<String, VarState>,
    /// Next item ID to allocate
    next_item_id: u32,
}

impl ModuleDepGraph {
    /// Create a new module dependency graph
    pub fn new(module_id: ModuleId, module_name: String) -> Self {
        Self {
            module_id,
            module_name,
            items: FxIndexMap::default(),
            side_effect_items: Vec::new(),
            var_states: FxIndexMap::default(),
            next_item_id: 0,
        }
    }

    /// Add a new item to the graph
    pub fn add_item(&mut self, data: ItemData) -> ItemId {
        let id = ItemId::new(self.next_item_id);
        self.next_item_id += 1;

        // Track imported names as variable declarations
        for imported_name in &data.imported_names {
            self.var_states
                .entry(imported_name.clone())
                .or_insert_with(|| VarState {
                    writers: Vec::new(),
                    readers: Vec::new(),
                });
        }

        // Track variable declarations
        for var in &data.var_decls {
            self.var_states
                .entry(var.clone())
                .or_insert_with(|| VarState {
                    writers: Vec::new(),
                    readers: Vec::new(),
                });
        }

        // Track variable reads
        for var in &data.read_vars {
            if let Some(state) = self.var_states.get_mut(var) {
                state.readers.push(id);
            }
        }

        // Track variable writes
        for var in &data.write_vars {
            if let Some(state) = self.var_states.get_mut(var) {
                state.writers.push(id);
            }
        }

        // Track side effects
        if data.has_side_effects {
            self.side_effect_items.push(id);
        }

        self.items.insert(id, data);
        id
    }

    /// Get all import items in the module with their IDs
    pub fn get_all_import_items(&self) -> Vec<(ItemId, &ItemData)> {
        self.items
            .iter()
            .filter(|(_, data)| {
                matches!(
                    data.item_type,
                    ItemType::Import { .. } | ItemType::FromImport { .. }
                )
            })
            .map(|(id, data)| (*id, data))
            .collect()
    }

    /// Check if a name is in __all__ export
    pub fn is_in_all_export(&self, name: &str) -> bool {
        // Look for __all__ assignments
        for item_data in self.items.values() {
            if let ItemType::Assignment { targets, .. } = &item_data.item_type
                && targets.contains(&"__all__".to_string())
            {
                // Check if the name is in the eventual_read_vars (where __all__ names are
                // stored)
                if item_data.eventual_read_vars.contains(name) {
                    return true;
                }
            }
        }
        false
    }

    /// Check if a symbol uses a specific import
    pub fn does_symbol_use_import(&self, symbol: &str, import_name: &str) -> bool {
        // Find the item that defines the symbol
        for item in self.items.values() {
            if item.defined_symbols.contains(symbol) {
                log::trace!(
                    "Checking if symbol '{}' uses import '{}' - read_vars: {:?}, \
                     eventual_read_vars: {:?}",
                    symbol,
                    import_name,
                    item.read_vars,
                    item.eventual_read_vars
                );
                // Check if this item uses the import
                if item.read_vars.contains(import_name)
                    || item.eventual_read_vars.contains(import_name)
                {
                    log::trace!("  Found: symbol '{symbol}' uses import '{import_name}'");
                    return true;
                }

                // Check symbol-specific dependencies
                if let Some(deps) = item.symbol_dependencies.get(symbol)
                    && deps.contains(import_name)
                {
                    log::trace!(
                        "  Found in symbol_dependencies: symbol '{symbol}' uses import \
                         '{import_name}'"
                    );
                    return true;
                }
            }
        }
        false
    }
}

// Note: Custom Tarjan SCC implementation removed in favor of petgraph::algo::tarjan_scc

/// High-level dependency graph managing multiple modules
/// Combines the best of three approaches:
/// - Turbopack's fine-grained tracking
/// - Rspack's incremental updates
/// - Mako's petgraph efficiency
#[derive(Debug)]
pub struct CriboGraph {
    /// All modules in the graph
    pub modules: FxIndexMap<ModuleId, ModuleDepGraph>,
    /// Module name to ID mapping
    pub module_names: FxIndexMap<String, ModuleId>,
    /// Module path to ID mapping
    pub module_paths: FxIndexMap<PathBuf, ModuleId>,
    /// Petgraph for efficient algorithms (inspired by Mako)
    graph: DiGraph<ModuleId, ()>,
    /// Node index mapping
    node_indices: FxIndexMap<ModuleId, NodeIndex>,

    // Fields for file-based deduplication
    /// Track canonical paths for each module
    module_canonical_paths: FxIndexMap<ModuleId, PathBuf>,
    /// Track all import names that resolve to each canonical file
    /// This includes regular imports AND static importlib calls
    file_to_import_names: FxIndexMap<PathBuf, FxIndexSet<String>>,
    /// Track the primary module ID for each file
    /// (The first import name discovered for this file)
    file_primary_module: FxIndexMap<PathBuf, (String, ModuleId)>,
    /// Track modules whose __all__ attribute is accessed
    /// Maps (`accessing_module_id`, `accessed_module_id`) for __all__ access tracking to prevent
    /// alias collisions
    modules_accessing_all: FxIndexSet<(ModuleId, ModuleId)>,
}

impl CriboGraph {
    /// Create a new cribo dependency graph
    pub fn new() -> Self {
        Self {
            modules: FxIndexMap::default(),
            module_names: FxIndexMap::default(),
            module_paths: FxIndexMap::default(),
            graph: DiGraph::new(),
            node_indices: FxIndexMap::default(),
            module_canonical_paths: FxIndexMap::default(),
            file_to_import_names: FxIndexMap::default(),
            file_primary_module: FxIndexMap::default(),
            modules_accessing_all: FxIndexSet::default(),
        }
    }

    /// Add a module to the graph with a pre-assigned `ModuleId` from the resolver
    pub fn add_module(&mut self, id: ModuleId, name: String, path: &Path) -> ModuleId {
        // Always work with canonical paths
        let canonical_path = path.canonicalize().unwrap_or_else(|_| path.to_owned());

        // Check if this exact import name already exists
        if let Some(&existing_id) = self.module_names.get(&name) {
            // Verify it's the same file
            if let Some(existing_canonical) = self.module_canonical_paths.get(&existing_id) {
                if existing_canonical == &canonical_path {
                    // Same import name, same file - track and reuse
                    self.file_to_import_names
                        .entry(canonical_path.clone())
                        .or_default()
                        .insert(name.clone());
                    return existing_id;
                }
                // Same import name but different files: reuse existing mapping deterministically.
                // Prevents alias flapping and preserves previously built edges.
                log::warn!(
                    "Import name '{name}' refers to different files: {} and {}. Reusing existing \
                     ModuleId {} to keep mapping stable.",
                    existing_canonical.display(),
                    canonical_path.display(),
                    existing_id.as_u32()
                );
                return existing_id;
            }
        }

        // Track this import name for the file
        self.file_to_import_names
            .entry(canonical_path.clone())
            .or_default()
            .insert(name.clone());

        // Check if this file already has a primary module
        if let Some((primary_name, primary_id)) = self.file_primary_module.get(&canonical_path) {
            log::info!(
                "File {} already imported as '{primary_name}', reusing ModuleId for import name \
                 '{name}'",
                canonical_path.display()
            );

            // IMPORTANT: Return the SAME ModuleId for the same physical file
            // This ensures circular dependency detection and all other processing
            // operates on physical files, not module names
            self.module_names.insert(name, *primary_id);

            return *primary_id;
        }

        // Use the pre-assigned ID from the resolver
        // The resolver guarantees that the entry module gets ID 0

        // Create module
        let module_graph = ModuleDepGraph::new(id, name.clone());
        self.modules.insert(id, module_graph);
        self.module_names.insert(name.clone(), id);
        self.module_paths.insert(canonical_path.clone(), id);
        self.module_canonical_paths
            .insert(id, canonical_path.clone());
        self.file_primary_module
            .insert(canonical_path.clone(), (name.clone(), id));

        // Add to petgraph
        let node_idx = self.graph.add_node(id);
        self.node_indices.insert(id, node_idx);

        log::debug!(
            "Registered module '{name}' as primary for file {}",
            canonical_path.display()
        );

        id
    }

    /// Get a module by ID
    pub fn get_module(&self, id: ModuleId) -> Option<&ModuleDepGraph> {
        self.modules.get(&id)
    }

    /// Get a module by name (for compatibility during migration)
    pub fn get_module_by_name(&self, name: &str) -> Option<&ModuleDepGraph> {
        self.module_names
            .get(name)
            .and_then(|&id| self.modules.get(&id))
    }

    /// Get a mutable module by name (for compatibility during migration)
    pub fn get_module_by_name_mut(&mut self, name: &str) -> Option<&mut ModuleDepGraph> {
        if let Some(&id) = self.module_names.get(name) {
            self.modules.get_mut(&id)
        } else {
            None
        }
    }

    /// Get modules that access __all__ attribute
    pub fn get_modules_accessing_all(&self) -> &FxIndexSet<(ModuleId, ModuleId)> {
        &self.modules_accessing_all
    }

    /// Add a module that accesses __all__ of another module
    pub fn add_module_accessing_all(
        &mut self,
        accessing_module_id: ModuleId,
        accessed_module_id: ModuleId,
    ) {
        self.modules_accessing_all
            .insert((accessing_module_id, accessed_module_id));
    }

    /// Add a dependency between modules (from depends on to)
    pub fn add_module_dependency(&mut self, from: ModuleId, to: ModuleId) {
        self.add_module_dependency_with_info(from, to, ());
    }

    /// Add a dependency between modules with additional information
    pub fn add_module_dependency_with_info(&mut self, from: ModuleId, to: ModuleId, info: ()) {
        if let (Some(&from_idx), Some(&to_idx)) =
            (self.node_indices.get(&from), self.node_indices.get(&to))
        {
            // For topological sort to work correctly with petgraph,
            // we need edge from dependency TO dependent
            // So if A depends on B, we add edge B -> A

            // Check if edge already exists to avoid duplicates
            if !self.graph.contains_edge(to_idx, from_idx) {
                self.graph.add_edge(to_idx, from_idx, info);
            }
        }
    }

    /// Get topologically sorted modules (uses petgraph)
    pub fn topological_sort(&self) -> Result<Vec<ModuleId>> {
        toposort(&self.graph, None)
            .map(|nodes| nodes.into_iter().map(|n| self.graph[n]).collect())
            .map_err(|_| anyhow!("Circular dependency detected"))
    }

    /// Check if the graph has cycles
    pub fn has_cycles(&self) -> bool {
        is_cyclic_directed(&self.graph)
    }

    /// Get all modules that a given module depends on
    pub fn get_dependencies(&self, module_id: ModuleId) -> Vec<ModuleId> {
        if let Some(&node_idx) = self.node_indices.get(&module_id) {
            // Since edges go from dependency to dependent, incoming edges are dependencies
            self.graph
                .neighbors_directed(node_idx, petgraph::Direction::Incoming)
                .map(|idx| self.graph[idx])
                .collect()
        } else {
            vec![]
        }
    }

    /// Find all strongly connected components (circular dependencies) using Tarjan's algorithm
    /// This is more efficient than Kosaraju for our use case and provides components in
    /// reverse topological order
    pub fn find_strongly_connected_components(&self) -> Vec<Vec<ModuleId>> {
        // Use petgraph's implementation for correctness and maintainability
        let components = tarjan_scc(&self.graph);

        // Convert NodeIndex components to ModuleId components and keep only real cycles
        // Include single-node components if there is a self-loop edge
        components
            .into_iter()
            .filter(|component| {
                component.len() > 1
                    || (component.len() == 1
                        && self.graph.contains_edge(component[0], component[0]))
            })
            .map(|component| component.into_iter().map(|idx| self.graph[idx]).collect())
            .collect()
    }
}

impl Default for CriboGraph {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analyzers::types::{CircularDependencyType, ResolutionStrategy};

    #[test]
    fn test_basic_module_graph() {
        let mut graph = CriboGraph::new();

        let utils_id = graph.add_module(
            ModuleId::new(0),
            "utils".to_string(),
            &PathBuf::from("utils.py"),
        );
        let main_id = graph.add_module(
            ModuleId::new(1),
            "main".to_string(),
            &PathBuf::from("main.py"),
        );

        graph.add_module_dependency(main_id, utils_id);

        let sorted = graph
            .topological_sort()
            .expect("Topological sort should succeed for acyclic graph");
        // Since main depends on utils, utils should come first in topological order
        assert_eq!(sorted, vec![utils_id, main_id]);
    }

    #[test]
    fn test_circular_dependency_detection() {
        let mut graph = CriboGraph::new();

        // Create a three-module circular dependency: A -> B -> C -> A
        let module_a = graph.add_module(
            ModuleId::new(0),
            "module_a".to_string(),
            &PathBuf::from("module_a.py"),
        );
        let module_b = graph.add_module(
            ModuleId::new(1),
            "module_b".to_string(),
            &PathBuf::from("module_b.py"),
        );
        let module_c = graph.add_module(
            ModuleId::new(2),
            "module_c".to_string(),
            &PathBuf::from("module_c.py"),
        );

        graph.add_module_dependency(module_a, module_b);
        graph.add_module_dependency(module_b, module_c);
        graph.add_module_dependency(module_c, module_a);

        // Check that cycles are detected
        assert!(graph.has_cycles());

        // Find strongly connected components
        let sccs = graph.find_strongly_connected_components();
        assert_eq!(sccs.len(), 1);
        assert_eq!(sccs[0].len(), 3);

        // Analyze circular dependencies using the analyzer
        let analysis = crate::analyzers::dependency_analyzer::DependencyAnalyzer::analyze_circular_dependencies(&graph);
        assert!(!analysis.resolvable_cycles.is_empty());
    }

    #[test]
    fn test_circular_dependency_classification() {
        let mut graph = CriboGraph::new();

        // Create a circular dependency with "constants" in the name
        let constants_a = graph.add_module(
            ModuleId::new(0),
            "constants_a".to_string(),
            &PathBuf::from("constants_a.py"),
        );
        let constants_b = graph.add_module(
            ModuleId::new(1),
            "constants_b".to_string(),
            &PathBuf::from("constants_b.py"),
        );

        // Add some constant assignments to make these actual constant modules
        if let Some(module_a) = graph.modules.get_mut(&constants_a) {
            module_a.add_item(ItemData {
                item_type: ItemType::Assignment {
                    targets: vec!["CONFIG".to_string()],
                },
                var_decls: ["CONFIG".into()].into_iter().collect(),
                read_vars: FxIndexSet::default(),
                eventual_read_vars: FxIndexSet::default(),
                write_vars: ["CONFIG".into()].into_iter().collect(),
                eventual_write_vars: FxIndexSet::default(),
                has_side_effects: false,
                imported_names: FxIndexSet::default(),
                reexported_names: FxIndexSet::default(),
                defined_symbols: FxIndexSet::default(),
                symbol_dependencies: FxIndexMap::default(),
                attribute_accesses: FxIndexMap::default(),
                containing_scope: None,
            });
        }

        if let Some(module_b) = graph.modules.get_mut(&constants_b) {
            module_b.add_item(ItemData {
                item_type: ItemType::Assignment {
                    targets: vec!["SETTINGS".to_string()],
                },
                var_decls: ["SETTINGS".into()].into_iter().collect(),
                read_vars: FxIndexSet::default(),
                eventual_read_vars: FxIndexSet::default(),
                write_vars: ["SETTINGS".into()].into_iter().collect(),
                eventual_write_vars: FxIndexSet::default(),
                has_side_effects: false,
                imported_names: FxIndexSet::default(),
                reexported_names: FxIndexSet::default(),
                defined_symbols: FxIndexSet::default(),
                symbol_dependencies: FxIndexMap::default(),
                attribute_accesses: FxIndexMap::default(),
                containing_scope: None,
            });
        }

        graph.add_module_dependency(constants_a, constants_b);
        graph.add_module_dependency(constants_b, constants_a);

        // Now we need to use the analyzer
        let analysis = crate::analyzers::dependency_analyzer::DependencyAnalyzer::analyze_circular_dependencies(&graph);
        assert_eq!(analysis.unresolvable_cycles.len(), 1);

        assert_eq!(
            analysis.unresolvable_cycles[0].cycle_type,
            CircularDependencyType::ModuleConstants
        );

        // Check resolution strategy
        if let ResolutionStrategy::Unresolvable { reason } =
            &analysis.unresolvable_cycles[0].suggested_resolution
        {
            assert!(reason.contains("temporal paradox"));
        } else {
            panic!("Expected unresolvable strategy for constants cycle");
        }
    }

    #[test]
    fn test_file_based_deduplication() {
        let mut graph = CriboGraph::new();

        // Add a module with a canonical path
        let path = PathBuf::from("src/utils.py");
        let utils_id = graph.add_module(ModuleId::new(0), "utils".to_string(), &path);

        // Add some items to the utils module
        let utils_module = graph
            .modules
            .get_mut(&utils_id)
            .expect("Module should exist after add_module");
        let item1 = utils_module.add_item(ItemData {
            item_type: ItemType::FunctionDef {
                name: "helper".into(),
            },
            var_decls: ["helper".into()].into_iter().collect(),
            read_vars: FxIndexSet::default(),
            eventual_read_vars: FxIndexSet::default(),
            write_vars: FxIndexSet::default(),
            eventual_write_vars: FxIndexSet::default(),
            has_side_effects: false,
            imported_names: FxIndexSet::default(),
            reexported_names: FxIndexSet::default(),
            defined_symbols: ["helper".into()].into_iter().collect(),
            symbol_dependencies: FxIndexMap::default(),
            attribute_accesses: FxIndexMap::default(),
            containing_scope: None,
        });

        // Add the same file with a different import name
        // This should return the SAME ModuleId due to file-based deduplication
        let alt_utils_id = graph.add_module(ModuleId::new(1), "src.utils".to_string(), &path);

        // Verify that both names map to the same ModuleId (file-based deduplication)
        assert_eq!(utils_id, alt_utils_id, "Same file should get same ModuleId");

        // Verify the module exists
        assert!(graph.modules.contains_key(&utils_id));

        // Verify that both names are tracked
        assert_eq!(graph.module_names.get("utils"), Some(&utils_id));
        assert_eq!(graph.module_names.get("src.utils"), Some(&utils_id));

        // Get the module
        let utils_module = &graph.modules[&utils_id];

        // Check that the item exists in the module
        assert!(utils_module.items.contains_key(&item1));

        // The module should have the primary name (first registered)
        assert_eq!(utils_module.module_name, "utils");

        // Test that adding items affects the same module (since they share the same ModuleId)
        let item2 = {
            let utils_module = graph
                .modules
                .get_mut(&utils_id)
                .expect("Module should exist after add_module");
            utils_module.add_item(ItemData {
                item_type: ItemType::FunctionDef {
                    name: "new_helper".into(),
                },
                var_decls: ["new_helper".into()].into_iter().collect(),
                read_vars: FxIndexSet::default(),
                eventual_read_vars: FxIndexSet::default(),
                write_vars: FxIndexSet::default(),
                eventual_write_vars: FxIndexSet::default(),
                has_side_effects: false,
                imported_names: FxIndexSet::default(),
                reexported_names: FxIndexSet::default(),
                defined_symbols: ["new_helper".into()].into_iter().collect(),
                symbol_dependencies: FxIndexMap::default(),
                attribute_accesses: FxIndexMap::default(),
                containing_scope: None,
            })
        };

        // Since alt_utils_id and utils_id are the same, they point to the same module
        let module = &graph.modules[&alt_utils_id];

        // Verify that the new item is present (they're the same module)
        assert!(
            module.items.contains_key(&item2),
            "Items should be present since both IDs point to the same module"
        );
    }
}
