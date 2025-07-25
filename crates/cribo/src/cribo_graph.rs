use std::path::PathBuf;

/// CriboGraph: Advanced dependency graph implementation for Python bundling
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
use indexmap::IndexSet;
use petgraph::{
    algo::{is_cyclic_directed, toposort},
    graph::{DiGraph, NodeIndex},
};
use rustc_hash::{FxHashMap, FxHashSet};

/// Unique identifier for a module
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ModuleId(u32);

impl ModuleId {
    pub fn new(id: u32) -> Self {
        Self(id)
    }

    /// Returns the underlying u32 value of the ModuleId
    #[inline]
    pub const fn as_u32(&self) -> u32 {
        self.0
    }
}

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
            ItemType::FunctionDef { name } => Some(name),
            ItemType::ClassDef { name } => Some(name),
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
    pub var_decls: FxHashSet<String>,
    /// Variables read by this item during execution
    pub read_vars: FxHashSet<String>,
    /// Variables read eventually (e.g., inside function bodies)
    pub eventual_read_vars: FxHashSet<String>,
    /// Variables written by this item
    pub write_vars: FxHashSet<String>,
    /// Variables written eventually
    pub eventual_write_vars: FxHashSet<String>,
    /// Whether this item has side effects
    pub has_side_effects: bool,
    /// For imports: the local names introduced by this import
    pub imported_names: FxHashSet<String>,
    /// For re-exports: names that are explicitly re-exported
    pub reexported_names: FxHashSet<String>,
    /// NEW: Top-level symbols defined by this item (for tree-shaking)
    pub defined_symbols: FxHashSet<String>,
    /// NEW: Map of symbol -> other symbols it references (for tree-shaking)
    pub symbol_dependencies: FxHashMap<String, FxHashSet<String>>,
    /// NEW: Map of variable -> accessed attributes (for tree-shaking namespace access)
    /// e.g., {"greetings": ["message"]} for greetings.message
    pub attribute_accesses: FxHashMap<String, FxHashSet<String>>,
    /// Track if this import was generated by stdlib normalization
    pub is_normalized_import: bool,
}

/// Fine-grained dependency graph for a single module
#[derive(Debug)]
pub struct ModuleDepGraph {
    /// Module identifier
    pub module_id: ModuleId,
    /// Module name (e.g., "utils.helpers")
    pub module_name: String,
    /// All items in this module
    pub items: FxHashMap<ItemId, ItemData>,
    /// Items that are executed for side effects (in order)
    pub side_effect_items: Vec<ItemId>,
    /// Variable state tracking
    pub var_states: FxHashMap<String, VarState>,
    /// Next item ID to allocate
    next_item_id: u32,
}

impl ModuleDepGraph {
    /// Create a new module dependency graph
    pub fn new(module_id: ModuleId, module_name: String) -> Self {
        Self {
            module_id,
            module_name,
            items: FxHashMap::default(),
            side_effect_items: Vec::new(),
            var_states: FxHashMap::default(),
            next_item_id: 0,
        }
    }

    /// Create a new module dependency graph that shares items from another graph
    /// This is used when the same file is imported with different names
    pub fn new_with_shared_items(
        module_id: ModuleId,
        module_name: String,
        source_graph: &ModuleDepGraph,
    ) -> Self {
        Self {
            module_id,
            module_name,
            // Clone all the data from the source graph to share the same items
            items: source_graph.items.clone(),
            side_effect_items: source_graph.side_effect_items.clone(),
            var_states: source_graph.var_states.clone(),
            next_item_id: source_graph.next_item_id,
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
                // Check if this item uses the import
                if item.read_vars.contains(import_name)
                    || item.eventual_read_vars.contains(import_name)
                {
                    return true;
                }

                // Check symbol-specific dependencies
                if let Some(deps) = item.symbol_dependencies.get(symbol)
                    && deps.contains(import_name)
                {
                    return true;
                }
            }
        }
        false
    }
}

/// State for Tarjan's strongly connected components algorithm
struct TarjanState {
    index_counter: usize,
    stack: Vec<NodeIndex>,
    indices: FxHashMap<NodeIndex, usize>,
    lowlinks: FxHashMap<NodeIndex, usize>,
    on_stack: FxHashMap<NodeIndex, bool>,
    components: Vec<Vec<NodeIndex>>,
}

/// High-level dependency graph managing multiple modules
/// Combines the best of three approaches:
/// - Turbopack's fine-grained tracking
/// - Rspack's incremental updates
/// - Mako's petgraph efficiency
#[derive(Debug)]
pub struct CriboGraph {
    /// All modules in the graph
    pub modules: FxHashMap<ModuleId, ModuleDepGraph>,
    /// Module name to ID mapping
    pub module_names: FxHashMap<String, ModuleId>,
    /// Module path to ID mapping
    pub module_paths: FxHashMap<PathBuf, ModuleId>,
    /// Petgraph for efficient algorithms (inspired by Mako)
    graph: DiGraph<ModuleId, ()>,
    /// Node index mapping
    node_indices: FxHashMap<ModuleId, NodeIndex>,
    /// Next module ID to allocate
    next_module_id: u32,

    // NEW: Fields for file-based deduplication
    /// Track canonical paths for each module
    module_canonical_paths: FxHashMap<ModuleId, PathBuf>,
    /// Track all import names that resolve to each canonical file
    /// This includes regular imports AND static importlib calls
    file_to_import_names: FxHashMap<PathBuf, IndexSet<String>>,
    /// Track the primary module ID for each file
    /// (The first import name discovered for this file)
    file_primary_module: FxHashMap<PathBuf, (String, ModuleId)>,
}

impl CriboGraph {
    /// Create a new cribo dependency graph
    pub fn new() -> Self {
        Self {
            modules: FxHashMap::default(),
            module_names: FxHashMap::default(),
            module_paths: FxHashMap::default(),
            graph: DiGraph::new(),
            node_indices: FxHashMap::default(),
            next_module_id: 0,
            module_canonical_paths: FxHashMap::default(),
            file_to_import_names: FxHashMap::default(),
            file_primary_module: FxHashMap::default(),
        }
    }

    /// Add a new module to the graph
    pub fn add_module(&mut self, name: String, path: PathBuf) -> ModuleId {
        // Always work with canonical paths
        let canonical_path = path.canonicalize().unwrap_or_else(|_| path.clone());

        // Check if this exact import name already exists
        if let Some(&existing_id) = self.module_names.get(&name) {
            // Verify it's the same file
            if let Some(existing_canonical) = self.module_canonical_paths.get(&existing_id) {
                if existing_canonical == &canonical_path {
                    return existing_id; // Same import name, same file - reuse
                } else {
                    // Error: same import name but different files
                    // This shouldn't happen with proper PYTHONPATH management
                    log::error!(
                        "Import name '{name}' refers to different files: {existing_canonical:?} \
                         and {canonical_path:?}. This may indicate a PYTHONPATH configuration \
                         issue or naming conflict. Consider using unique module names or \
                         adjusting your Python path configuration."
                    );
                }
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
                "File {canonical_path:?} already imported as '{primary_name}', adding additional \
                 import name '{name}'"
            );

            // Create a new ModuleId that shares the same dependency graph
            // This allows different import names to have different dependency relationships
            // while still pointing to the same file
            let id = ModuleId::new(self.next_module_id);
            self.next_module_id += 1;

            // Clone the dependency graph structure but with new module name
            let primary_graph = &self.modules[primary_id];
            let module = ModuleDepGraph::new_with_shared_items(id, name.clone(), primary_graph);

            // Now the new module shares the same item registry as the primary module
            self.modules.insert(id, module);
            self.module_names.insert(name, id);
            self.module_canonical_paths.insert(id, canonical_path);

            // Add to petgraph
            let node_idx = self.graph.add_node(id);
            self.node_indices.insert(id, node_idx);

            return id;
        }

        // This is the first time we're seeing this file
        let id = ModuleId::new(self.next_module_id);
        self.next_module_id += 1;

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

        log::debug!("Registered module '{name}' as primary for file {canonical_path:?}");

        id
    }

    /// Get a module by name
    pub fn get_module_by_name(&self, name: &str) -> Option<&ModuleDepGraph> {
        self.module_names
            .get(name)
            .and_then(|&id| self.modules.get(&id))
    }

    /// Get a mutable module by name
    pub fn get_module_by_name_mut(&mut self, name: &str) -> Option<&mut ModuleDepGraph> {
        if let Some(&id) = self.module_names.get(name) {
            self.modules.get_mut(&id)
        } else {
            None
        }
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
        let mut state = TarjanState {
            index_counter: 0,
            stack: Vec::new(),
            indices: FxHashMap::default(),
            lowlinks: FxHashMap::default(),
            on_stack: FxHashMap::default(),
            components: Vec::new(),
        };

        for node_index in self.graph.node_indices() {
            if !state.indices.contains_key(&node_index) {
                self.tarjan_strongconnect(node_index, &mut state);
            }
        }

        // Convert NodeIndex components to ModuleId components
        state
            .components
            .into_iter()
            .map(|component| component.into_iter().map(|idx| self.graph[idx]).collect())
            .collect()
    }

    /// Helper for Tarjan's algorithm
    fn tarjan_strongconnect(&self, v: NodeIndex, state: &mut TarjanState) {
        state.indices.insert(v, state.index_counter);
        state.lowlinks.insert(v, state.index_counter);
        state.index_counter += 1;
        state.stack.push(v);
        state.on_stack.insert(v, true);

        // Note: Our edges go from dependency to dependent, so we traverse outgoing edges
        for w in self
            .graph
            .neighbors_directed(v, petgraph::Direction::Outgoing)
        {
            if !state.indices.contains_key(&w) {
                self.tarjan_strongconnect(w, state);
                let w_lowlink = *state.lowlinks.get(&w).expect("w should exist in lowlinks");
                let v_lowlink = *state.lowlinks.get(&v).expect("v should exist in lowlinks");
                state.lowlinks.insert(v, v_lowlink.min(w_lowlink));
            } else if *state.on_stack.get(&w).unwrap_or(&false) {
                let w_index = *state.indices.get(&w).expect("w should exist in indices");
                let v_lowlink = *state.lowlinks.get(&v).expect("v should exist in lowlinks");
                state.lowlinks.insert(v, v_lowlink.min(w_index));
            }
        }

        if state.lowlinks[&v] == state.indices[&v] {
            let component = self.pop_scc_component(&mut state.stack, &mut state.on_stack, v);
            if component.len() > 1 {
                state.components.push(component);
            }
        }
    }

    /// Pop a strongly connected component from the stack
    fn pop_scc_component(
        &self,
        stack: &mut Vec<NodeIndex>,
        on_stack: &mut FxHashMap<NodeIndex, bool>,
        v: NodeIndex,
    ) -> Vec<NodeIndex> {
        let mut component = Vec::new();
        while let Some(w) = stack.pop() {
            on_stack.insert(w, false);
            component.push(w);
            if w == v {
                break;
            }
        }
        component
    }
}

// HashSet import moved to top

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

        let utils_id = graph.add_module("utils".to_string(), PathBuf::from("utils.py"));
        let main_id = graph.add_module("main".to_string(), PathBuf::from("main.py"));

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
        let module_a = graph.add_module("module_a".to_string(), PathBuf::from("module_a.py"));
        let module_b = graph.add_module("module_b".to_string(), PathBuf::from("module_b.py"));
        let module_c = graph.add_module("module_c".to_string(), PathBuf::from("module_c.py"));

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
        let constants_a =
            graph.add_module("constants_a".to_string(), PathBuf::from("constants_a.py"));
        let constants_b =
            graph.add_module("constants_b".to_string(), PathBuf::from("constants_b.py"));

        // Add some constant assignments to make these actual constant modules
        if let Some(module_a) = graph.modules.get_mut(&constants_a) {
            module_a.add_item(ItemData {
                item_type: ItemType::Assignment {
                    targets: vec!["CONFIG".to_string()],
                },
                var_decls: ["CONFIG".into()].into_iter().collect(),
                read_vars: FxHashSet::default(),
                eventual_read_vars: FxHashSet::default(),
                write_vars: ["CONFIG".into()].into_iter().collect(),
                eventual_write_vars: FxHashSet::default(),
                has_side_effects: false,
                imported_names: FxHashSet::default(),
                reexported_names: FxHashSet::default(),
                defined_symbols: FxHashSet::default(),
                symbol_dependencies: FxHashMap::default(),
                attribute_accesses: FxHashMap::default(),
                is_normalized_import: false,
            });
        }

        if let Some(module_b) = graph.modules.get_mut(&constants_b) {
            module_b.add_item(ItemData {
                item_type: ItemType::Assignment {
                    targets: vec!["SETTINGS".to_string()],
                },
                var_decls: ["SETTINGS".into()].into_iter().collect(),
                read_vars: FxHashSet::default(),
                eventual_read_vars: FxHashSet::default(),
                write_vars: ["SETTINGS".into()].into_iter().collect(),
                eventual_write_vars: FxHashSet::default(),
                has_side_effects: false,
                imported_names: FxHashSet::default(),
                reexported_names: FxHashSet::default(),
                defined_symbols: FxHashSet::default(),
                symbol_dependencies: FxHashMap::default(),
                attribute_accesses: FxHashMap::default(),
                is_normalized_import: false,
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
    fn test_cloned_items_for_same_file() {
        let mut graph = CriboGraph::new();

        // Add a module with a canonical path
        let path = PathBuf::from("src/utils.py");
        let utils_id = graph.add_module("utils".to_string(), path.clone());

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
            read_vars: FxHashSet::default(),
            eventual_read_vars: FxHashSet::default(),
            write_vars: FxHashSet::default(),
            eventual_write_vars: FxHashSet::default(),
            has_side_effects: false,
            imported_names: FxHashSet::default(),
            reexported_names: FxHashSet::default(),
            defined_symbols: ["helper".into()].into_iter().collect(),
            symbol_dependencies: FxHashMap::default(),
            attribute_accesses: FxHashMap::default(),
            is_normalized_import: false,
        });

        // Add the same file with a different import name
        let alt_utils_id = graph.add_module("src.utils".to_string(), path);

        // Verify that both modules exist
        assert!(graph.modules.contains_key(&utils_id));
        assert!(graph.modules.contains_key(&alt_utils_id));

        // Verify that they share the same items
        let utils_module = &graph.modules[&utils_id];
        let alt_utils_module = &graph.modules[&alt_utils_id];

        // Check that the item exists in both modules
        assert!(utils_module.items.contains_key(&item1));
        assert!(alt_utils_module.items.contains_key(&item1));

        // Check that they have the same number of items
        assert_eq!(utils_module.items.len(), alt_utils_module.items.len());

        // Check that the item data is identical
        assert_eq!(
            utils_module.items[&item1].item_type,
            alt_utils_module.items[&item1].item_type
        );

        // Verify module names are different
        assert_eq!(utils_module.module_name, "utils");
        assert_eq!(alt_utils_module.module_name, "src.utils");

        // Verify module IDs are different
        assert_ne!(utils_module.module_id, alt_utils_module.module_id);

        // Test that adding items to one module affects the other
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
                read_vars: FxHashSet::default(),
                eventual_read_vars: FxHashSet::default(),
                write_vars: FxHashSet::default(),
                eventual_write_vars: FxHashSet::default(),
                has_side_effects: false,
                imported_names: FxHashSet::default(),
                reexported_names: FxHashSet::default(),
                defined_symbols: ["new_helper".into()].into_iter().collect(),
                symbol_dependencies: FxHashMap::default(),
                attribute_accesses: FxHashMap::default(),
                is_normalized_import: false,
            })
        };

        // NOTE: The current implementation uses cloning instead of true sharing.
        // When multiple modules point to the same file, they get a snapshot of items
        // at creation time. This is the intended behavior for the bundler, as modules
        // are processed independently and items are discovered during the initial parse.
        // True sharing (e.g., Arc<RwLock<>>) would add unnecessary complexity for no
        // practical benefit in the bundling use case.
        let alt_utils_module = &graph.modules[&alt_utils_id];

        // Verify that items are NOT shared (current and intended behavior)
        assert!(
            !alt_utils_module.items.contains_key(&item2),
            "With current implementation, new items added to one module should NOT appear in \
             other modules"
        );
    }
}
