use std::path::{Path, PathBuf};

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
use log::debug;
use petgraph::{
    algo::{is_cyclic_directed, toposort},
    graph::{DiGraph, NodeIndex},
};
use rustc_hash::{FxHashMap, FxHashSet};

// Import circular dependency types for compatibility
use crate::analysis::{CircularDependencyType, ResolutionStrategy};

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

    /// Get the underlying u32 value for sorting
    pub fn as_u32(&self) -> u32 {
        self.0
    }
}

/// Type of Python item (statement/definition)
#[derive(Debug, Clone, PartialEq, Eq, Default)]
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
    #[default]
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

/// Dependency type between items
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DepType {
    /// Always needed (e.g., direct function call)
    Strong,
    /// Only needed if target is included (e.g., conditional import)
    Weak,
}

/// A single dependency relationship
#[derive(Debug, Clone)]
pub struct Dep {
    pub target: ItemId,
    pub dep_type: DepType,
}

/// Information about a module-level dependency edge
#[derive(Debug, Clone, Default)]
pub struct ModuleDependencyInfo {
    /// Whether this dependency is only used in TYPE_CHECKING blocks
    pub is_type_checking_only: bool,
}

/// Variable state tracking
#[derive(Debug, Clone)]
pub struct VarState {
    /// The item that declares this variable
    pub declarator: Option<ItemId>,
    /// Items that write to this variable
    pub writers: Vec<ItemId>,
    /// Items that read this variable
    pub readers: Vec<ItemId>,
}

/// Information about an unused import
#[derive(Debug, Clone)]
pub struct UnusedImportInfo {
    /// The item ID of the import statement
    pub item_id: ItemId,
    /// The imported name that is unused
    pub name: String,
    /// The module it was imported from
    pub module: String,
    /// Whether this is an explicit re-export
    pub is_reexport: bool,
}

/// Context for checking if an import is unused
struct ImportUsageContext<'a> {
    imported_name: &'a str,
    import_id: ItemId,
    is_init_py: bool,
    import_data: &'a ItemData,
}

/// Data about a Python item (statement/definition)
#[derive(Debug, Clone, Default)]
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
    /// Source span for error reporting
    pub span: Option<(usize, usize)>, // (start_line, end_line)
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
    /// Index of the statement in the module's AST body
    pub statement_index: Option<usize>,
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
    /// Dependencies between items
    pub deps: FxHashMap<ItemId, Vec<Dep>>,
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
            deps: FxHashMap::default(),
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
            deps: source_graph.deps.clone(),
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
                    declarator: Some(id),
                    writers: Vec::new(),
                    readers: Vec::new(),
                });
        }

        // Track variable declarations
        for var in &data.var_decls {
            self.var_states
                .entry(var.clone())
                .or_insert_with(|| VarState {
                    declarator: Some(id),
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

    /// Add a new item to the graph and return its NodeIndex
    /// This is used by the two-pass GraphBuilder to build symbol maps
    pub fn add_item_with_index(&mut self, data: ItemData) -> NodeIndex {
        // Add the item normally
        let item_id = self.add_item(data);

        // Create a NodeIndex for this item
        // In the actual graph structure, items don't have their own nodes in petgraph
        // This is a synthetic index for the symbol map
        // We use the item_id's u32 value as the node index
        NodeIndex::new(item_id.0 as usize)
    }

    /// Add a dependency between items
    pub fn add_dependency(&mut self, from: ItemId, to: ItemId, dep_type: DepType) {
        self.deps.entry(from).or_default().push(Dep {
            target: to,
            dep_type,
        });
    }

    /// Get all items that an item depends on (transitively)
    pub fn get_transitive_deps(&self, item: ItemId) -> FxHashSet<ItemId> {
        let mut visited = FxHashSet::default();
        let mut stack = vec![item];

        while let Some(current) = stack.pop() {
            if visited.insert(current) {
                self.add_dependencies_to_stack(&current, &mut stack);
            }
        }

        visited.remove(&item); // Don't include the starting item
        visited
    }

    /// Find unused imports in the module
    pub fn find_unused_imports(&self, is_init_py: bool) -> Vec<UnusedImportInfo> {
        let mut unused_imports = Vec::new();

        // First, collect all imported names
        let mut imported_items: Vec<(ItemId, &ItemData)> = Vec::new();
        for (id, data) in &self.items {
            if matches!(
                data.item_type,
                ItemType::Import { .. } | ItemType::FromImport { .. }
            ) && !data.imported_names.is_empty()
            {
                imported_items.push((*id, data));
            }
        }

        // For each imported name, check if it's used
        for (import_id, import_data) in imported_items {
            for imported_name in &import_data.imported_names {
                let ctx = ImportUsageContext {
                    imported_name,
                    import_id,
                    is_init_py,
                    import_data,
                };

                if self.is_import_unused(ctx) {
                    let module_name = match &import_data.item_type {
                        ItemType::Import { module, .. } => module.clone(),
                        ItemType::FromImport { module, .. } => module.clone(),
                        _ => continue,
                    };

                    unused_imports.push(UnusedImportInfo {
                        item_id: import_id,
                        name: imported_name.clone(),
                        module: module_name,
                        is_reexport: import_data.reexported_names.contains(imported_name),
                    });
                }
            }
        }

        unused_imports
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

    /// Check if a specific imported name is unused
    fn is_import_unused(&self, ctx: ImportUsageContext<'_>) -> bool {
        // Check for special cases where imports should be preserved
        if ctx.is_init_py {
            // In __init__.py, preserve all imports as they might be part of the public API
            return false;
        }

        // Check if it's a star import
        if let ItemType::FromImport { is_star: true, .. } = &ctx.import_data.item_type {
            // Star imports are always preserved
            return false;
        }

        // Check if it's explicitly re-exported
        if ctx.import_data.reexported_names.contains(ctx.imported_name) {
            return false;
        }

        // Check if it's in __all__ (module re-export)
        if self.is_in_all_export(ctx.imported_name) {
            return false;
        }

        // Check if the import has side effects (includes stdlib imports)
        if ctx.import_data.has_side_effects {
            return false;
        }

        // Check if the name is used anywhere in the module
        for (item_id, item_data) in &self.items {
            // Skip the import statement itself
            if *item_id == ctx.import_id {
                continue;
            }

            // Check if the name is read by this item
            if item_data.read_vars.contains(ctx.imported_name)
                || item_data.eventual_read_vars.contains(ctx.imported_name)
            {
                log::trace!(
                    "Import '{}' is used by item {:?} (read_vars: {:?}, eventual_read_vars: {:?})",
                    ctx.imported_name,
                    item_id,
                    item_data.read_vars,
                    item_data.eventual_read_vars
                );
                return false;
            }

            // For dotted imports like `import xml.etree.ElementTree`, also check if any of the
            // declared variables from that import are used
            if let Some(import_item) = self.items.get(&ctx.import_id) {
                let is_var_used = import_item.var_decls.iter().any(|var_decl| {
                    item_data.read_vars.contains(var_decl)
                        || item_data.eventual_read_vars.contains(var_decl)
                });

                if is_var_used {
                    log::trace!(
                        "Import '{}' is used via declared variables by item {:?}",
                        ctx.imported_name,
                        item_id
                    );
                    return false;
                }
            }
        }

        // Check if the name is in the module's __all__ export list
        if self.is_in_module_exports(ctx.imported_name) {
            return false;
        }

        log::trace!("Import '{}' is UNUSED", ctx.imported_name);
        true
    }

    /// Check if a name is in the module's __all__ export list
    fn is_in_module_exports(&self, name: &str) -> bool {
        // Look for __all__ assignment
        for item_data in self.items.values() {
            if let ItemType::Assignment { targets } = &item_data.item_type
                && targets.contains(&"__all__".to_string())
            {
                // Check if the name is in the reexported_names set
                // which contains the parsed __all__ list values
                return item_data.reexported_names.contains(name);
            }
        }
        false
    }

    /// Helper method to add dependencies to stack
    fn add_dependencies_to_stack(&self, current: &ItemId, stack: &mut Vec<ItemId>) {
        if let Some(deps) = self.deps.get(current) {
            for dep in deps {
                stack.push(dep.target);
            }
        }
    }

    /// Find all items needed for a set of used symbols
    pub fn tree_shake(&self, used_symbols: &IndexSet<String>) -> FxHashSet<ItemId> {
        let mut required_items = FxHashSet::default();

        // Start with items that define used symbols
        for (item_id, data) in &self.items {
            let defines_used_symbol = match &data.item_type {
                ItemType::FunctionDef { name } | ItemType::ClassDef { name } => {
                    used_symbols.contains(name.as_str())
                }
                ItemType::Assignment { targets } => {
                    targets.iter().any(|t| used_symbols.contains(t.as_str()))
                }
                _ => false,
            };

            if defines_used_symbol {
                self.collect_required_items(*item_id, &mut required_items);
            }
        }

        // Always include side effects in order
        for &item in &self.side_effect_items {
            required_items.insert(item);
        }

        required_items
    }

    /// Recursively collect all items required by a given item
    fn collect_required_items(&self, item: ItemId, required: &mut FxHashSet<ItemId>) {
        if !required.insert(item) {
            return; // Already processed
        }

        // Add all dependencies
        if let Some(deps) = self.deps.get(&item) {
            self.process_item_dependencies(deps, required);
        }
    }

    /// Process dependencies for an item
    fn process_item_dependencies(&self, deps: &[Dep], required: &mut FxHashSet<ItemId>) {
        for dep in deps {
            match dep.dep_type {
                DepType::Strong => {
                    self.collect_required_items(dep.target, required);
                }
                DepType::Weak => {
                    // Only include if target is already required
                    if required.contains(&dep.target) {
                        self.collect_required_items(dep.target, required);
                    }
                }
            }
        }
    }
}

/// Module metadata for optimization
#[derive(Debug, Clone)]
pub struct ModuleMetadata {
    /// Whether module has side effects
    pub has_side_effects: bool,
    /// Whether module is an entry point
    pub is_entry: bool,
    /// Whether module is from the standard library
    pub is_stdlib: bool,
    /// Size in bytes (for chunking decisions)
    pub size: usize,
    /// Hash of module content (for caching)
    pub content_hash: Option<u64>,
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

/// Color for DFS traversal (three-color marking)
#[derive(Debug, Clone, Copy, PartialEq)]
enum Color {
    White, // Not visited
    Gray,  // Currently visiting
    Black, // Finished visiting
}

/// State for cycle search operations
struct CycleSearchState {
    visited: FxHashMap<NodeIndex, Color>,
    path: Vec<NodeIndex>,
    cycles: Vec<Vec<NodeIndex>>,
}

/// Analysis result for cycle modules
struct CycleAnalysisResult {
    has_only_constants: bool,
    has_class_definitions: bool,
    has_module_level_imports: bool,
    imports_used_in_functions_only: bool,
}

// Note: Circular dependency analysis types have been moved to crate::analysis::circular_deps
// The old ImportEdge and ImportType are kept here temporarily for compatibility
// TODO: Remove these after updating all references

/// An import edge in the dependency graph (DEPRECATED - use ModuleEdge)
#[derive(Debug, Clone)]
pub struct ImportEdge {
    pub from_module: String,
    pub to_module: String,
    pub import_type: ImportType,
    pub line_number: Option<usize>,
}

/// Type of import statement (DEPRECATED - use EdgeType)
#[derive(Debug, Clone)]
pub enum ImportType {
    Direct,         // import module
    FromImport,     // from module import item
    RelativeImport, // from .module import item
    AliasedImport,  // import module as alias
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
    /// Module metadata
    pub module_metadata: FxHashMap<ModuleId, ModuleMetadata>,
    /// Petgraph for efficient algorithms (inspired by Mako)
    graph: DiGraph<ModuleId, ModuleDependencyInfo>,
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
    /// Check if a stdlib module has side effects that make it unsafe to hoist
    fn is_stdlib_with_side_effects(module_name: &str) -> bool {
        matches!(
            module_name,
            // Modules that modify global state - DO NOT HOIST
            "antigravity" // Opens web browser to xkcd comic
            | "this"    // Prints "The Zen of Python" to stdout
            | "__hello__"   // Prints "Hello world!" to stdout
            | "__phello__"  // Frozen version of __hello__ that prints to stdout
            | "site"    // Modifies sys.path and sets up site packages
            | "sitecustomize"   // User-specific site customization
            | "usercustomize"   // User-specific customization
            | "readline"    // Initializes readline library and terminal settings
            | "rlcompleter"  // Configures readline tab completion
            | "turtle"        // Initializes Tk graphics window
            | "tkinter"       // Initializes Tk GUI framework
            | "webbrowser"    // May launch web browser
            | "platform"     // May execute external commands for system info
            | "locale" // Modifies global locale settings
        )
    }

    /// Create a new cribo dependency graph
    pub fn new() -> Self {
        Self {
            modules: FxHashMap::default(),
            module_names: FxHashMap::default(),
            module_paths: FxHashMap::default(),
            module_metadata: FxHashMap::default(),
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

            // Copy metadata from primary module
            if let Some(primary_metadata) = self.module_metadata.get(primary_id) {
                self.module_metadata.insert(id, primary_metadata.clone());
            }

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

        // Check if module is from stdlib
        let root_module = name.split('.').next().unwrap_or(&name);
        let is_stdlib = ruff_python_stdlib::sys::is_known_standard_library(10, root_module);

        // Initialize metadata
        self.module_metadata.insert(
            id,
            ModuleMetadata {
                has_side_effects: is_stdlib && Self::is_stdlib_with_side_effects(&name),
                is_entry: false,
                is_stdlib,
                size: 0,
                content_hash: None,
            },
        );

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
        self.add_module_dependency_with_info(from, to, ModuleDependencyInfo::default());
    }

    /// Add a dependency between modules with additional information
    pub fn add_module_dependency_with_info(
        &mut self,
        from: ModuleId,
        to: ModuleId,
        info: ModuleDependencyInfo,
    ) {
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

    /// Get all modules that depend on a given module
    pub fn get_dependents(&self, module_id: ModuleId) -> Vec<ModuleId> {
        if let Some(&node_idx) = self.node_indices.get(&module_id) {
            // Since edges go from dependency to dependent, outgoing edges are dependents
            self.graph
                .neighbors_directed(node_idx, petgraph::Direction::Outgoing)
                .map(|idx| self.graph[idx])
                .collect()
        } else {
            vec![]
        }
    }

    /// Check if a module dependency is type-checking-only
    pub fn is_type_checking_only_dependency(&self, from: ModuleId, to: ModuleId) -> bool {
        if let (Some(&from_idx), Some(&to_idx)) =
            (self.node_indices.get(&from), self.node_indices.get(&to))
            && let Some(edge) = self.graph.find_edge(to_idx, from_idx)
            && let Some(weight) = self.graph.edge_weight(edge)
        {
            return weight.is_type_checking_only;
        }
        false
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

    /// Find all strongly connected components (circular dependencies) - alias for compatibility
    pub fn find_cycles(&self) -> Vec<Vec<ModuleId>> {
        self.find_strongly_connected_components()
    }

    /// Get module metadata
    pub fn get_metadata(&self, module_id: ModuleId) -> Option<&ModuleMetadata> {
        self.module_metadata.get(&module_id)
    }

    /// Update module metadata
    pub fn update_metadata(&mut self, module_id: ModuleId, metadata: ModuleMetadata) {
        self.module_metadata.insert(module_id, metadata);
    }

    /// Find cycle paths using DFS with three-color marking
    pub fn find_cycle_paths(&self) -> Result<Vec<Vec<String>>> {
        let mut state = CycleSearchState {
            visited: FxHashMap::default(),
            path: Vec::new(),
            cycles: Vec::new(),
        };

        // Initialize all nodes as white
        for &module_id in self.modules.keys() {
            if let Some(&node_idx) = self.node_indices.get(&module_id) {
                state.visited.insert(node_idx, Color::White);
            }
        }

        // DFS from each unvisited node
        for &module_id in self.modules.keys() {
            if let Some(&node_idx) = self.node_indices.get(&module_id)
                && state.visited[&node_idx] == Color::White
            {
                self.dfs_find_cycles(node_idx, &mut state);
            }
        }

        // Convert cycles from NodeIndex to module names
        let named_cycles = state
            .cycles
            .into_iter()
            .map(|cycle| {
                cycle
                    .into_iter()
                    .filter_map(|idx| {
                        let module_id = self.graph[idx];
                        self.modules
                            .get(&module_id)
                            .map(|module| module.module_name.clone())
                    })
                    .collect()
            })
            .collect();

        Ok(named_cycles)
    }

    /// DFS helper for finding cycles
    fn dfs_find_cycles(&self, node: NodeIndex, state: &mut CycleSearchState) {
        state.visited.insert(node, Color::Gray);
        state.path.push(node);

        // Check all neighbors
        for neighbor in self
            .graph
            .neighbors_directed(node, petgraph::Direction::Outgoing)
        {
            match state.visited.get(&neighbor).unwrap_or(&Color::White) {
                Color::White => {
                    self.dfs_find_cycles(neighbor, state);
                }
                Color::Gray => {
                    // Found a cycle - extract it from the path
                    if let Some(start_pos) = state.path.iter().position(|&n| n == neighbor) {
                        let cycle = state.path[start_pos..].to_vec();
                        state.cycles.push(cycle);
                    }
                }
                Color::Black => {} // Already processed
            }
        }

        state.path.pop();
        state.visited.insert(node, Color::Black);
    }

    /// Analyze circular dependencies and classify them
    pub fn analyze_circular_dependencies(&self) -> crate::analysis::CircularDependencyAnalysis {
        use crate::analysis::CircularDependencyAnalyzer;

        let analyzer = CircularDependencyAnalyzer::new(self);
        analyzer.analyze()
    }

    /// Build import chain for a strongly connected component
    fn build_import_chain_for_scc(&self, scc: &[ModuleId]) -> Vec<ImportEdge> {
        let mut import_chain = Vec::new();

        for &from_module_id in scc {
            let Some(from_module) = self.modules.get(&from_module_id) else {
                log::warn!("Module {from_module_id:?} not found in build_import_chain_for_scc");
                continue;
            };
            let from_name = &from_module.module_name;

            // Get dependencies of this module that are also in the SCC
            let deps = self.get_dependencies(from_module_id);
            for to_module_id in deps {
                if !scc.contains(&to_module_id) {
                    continue;
                }

                let Some(to_module) = self.modules.get(&to_module_id) else {
                    log::warn!("Module {to_module_id:?} not found in build_import_chain_for_scc");
                    continue;
                };
                let to_name = &to_module.module_name;

                // Check module-level imports to determine import type
                let import_type = self.determine_import_type(from_module_id, to_module_id);

                import_chain.push(ImportEdge {
                    from_module: from_name.clone(),
                    to_module: to_name.clone(),
                    import_type,
                    line_number: None, // Would need AST info
                });
            }
        }

        import_chain
    }

    /// Determine import type between two modules
    fn determine_import_type(&self, from_id: ModuleId, to_id: ModuleId) -> ImportType {
        // Check the module's items for import statements
        if let Some(from_module) = self.modules.get(&from_id) {
            for item_data in from_module.items.values() {
                if let Some(import_type) =
                    self.check_item_for_import_type(&item_data.item_type, to_id)
                {
                    return import_type;
                }
            }
        }
        ImportType::Direct // Default
    }

    /// Check if an item contains an import that matches the target module
    fn check_item_for_import_type(
        &self,
        item_type: &ItemType,
        to_id: ModuleId,
    ) -> Option<ImportType> {
        match item_type {
            ItemType::Import { module, alias } => {
                if self.module_names.get(module) == Some(&to_id) {
                    if alias.is_some() {
                        Some(ImportType::AliasedImport)
                    } else {
                        Some(ImportType::Direct)
                    }
                } else {
                    None
                }
            }
            ItemType::FromImport { module, level, .. } => {
                if self.module_names.get(module) == Some(&to_id) {
                    Some(if *level > 0 {
                        ImportType::RelativeImport
                    } else {
                        ImportType::FromImport
                    })
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    /// Classify the type of circular dependency
    fn classify_cycle_type(
        &self,
        module_names: &[String],
        import_chain: &[ImportEdge],
    ) -> CircularDependencyType {
        // Check if this is a parent-child package cycle
        // These occur when a package imports from its subpackage (e.g., pkg/__init__.py imports
        // from pkg.submodule)
        if self.is_parent_child_package_cycle(module_names) {
            // This is a normal Python pattern, not a problematic cycle
            return CircularDependencyType::FunctionLevel; // Most permissive type
        }

        // Perform AST analysis on the modules in the cycle
        let analysis_result = self.analyze_cycle_modules(module_names);

        // Use AST analysis results for classification
        if analysis_result.has_only_constants
            && !module_names.iter().any(|name| name.ends_with("__init__"))
        {
            // Modules that only contain constants create unresolvable cycles
            // Exception: __init__.py files often only have imports/exports which is normal
            return CircularDependencyType::ModuleConstants;
        }

        if analysis_result.has_class_definitions {
            // Check if the circular imports are used for inheritance
            // If all imports in the cycle are only used in functions, it's still FunctionLevel
            if analysis_result.imports_used_in_functions_only {
                return CircularDependencyType::FunctionLevel;
            }
            // Otherwise, it's a true class-level cycle
            return CircularDependencyType::ClassLevel;
        }

        // Fall back to name-based heuristics if AST analysis is inconclusive
        for module_name in module_names {
            if module_name.contains("constants") || module_name.contains("config") {
                return CircularDependencyType::ModuleConstants;
            }
            if module_name.contains("class") || module_name.ends_with("_class") {
                return CircularDependencyType::ClassLevel;
            }
        }

        // Check if imports can be moved to functions
        // Special case: if modules have NO items (empty or only imports), treat as FunctionLevel
        // This handles simple circular import cases like stickytape tests
        if self.all_modules_empty_or_imports_only(module_names) {
            // Simple circular imports can often be resolved
            CircularDependencyType::FunctionLevel
        } else if analysis_result.imports_used_in_functions_only {
            CircularDependencyType::FunctionLevel
        } else if analysis_result.has_module_level_imports
            || import_chain.iter().any(|edge| {
                edge.from_module.contains("__init__") || edge.to_module.contains("__init__")
            })
        {
            CircularDependencyType::ImportTime
        } else {
            CircularDependencyType::FunctionLevel
        }
    }

    /// Analyze modules in a cycle to determine their characteristics
    fn analyze_cycle_modules(&self, module_names: &[String]) -> CycleAnalysisResult {
        let mut has_only_constants = true;
        let mut has_class_definitions = false;
        let mut has_module_level_imports = false;
        let mut imports_used_in_functions_only = true;

        for module_name in module_names {
            let Some(&module_id) = self.module_names.get(module_name) else {
                continue;
            };

            let Some(module) = self.modules.get(&module_id) else {
                continue;
            };

            // Check if module only contains constant assignments
            let module_has_only_constants = self.module_has_only_constants(module);
            has_only_constants = has_only_constants && module_has_only_constants;

            // Check for class definitions
            if self.module_has_class_definitions(module) {
                has_class_definitions = true;
            }

            // Check if imports are at module level
            if self.module_has_module_level_imports(module) {
                has_module_level_imports = true;

                // Now check if those imports are only used inside functions
                if !self.are_imports_used_only_in_functions(module) {
                    imports_used_in_functions_only = false;
                }
            }
        }

        CycleAnalysisResult {
            has_only_constants,
            has_class_definitions,
            has_module_level_imports,
            imports_used_in_functions_only: !has_module_level_imports
                || imports_used_in_functions_only,
        }
    }

    /// Check if a module only contains constant assignments
    fn module_has_only_constants(&self, module: &ModuleDepGraph) -> bool {
        // Empty modules (no items) should not be considered as "only constants"
        // Modules with only imports should not be considered as "only constants"
        !module.items.is_empty()
            && module
                .items
                .values()
                .any(|item| matches!(item.item_type, ItemType::Assignment { .. }))
            && !module.items.values().any(|item| {
                matches!(
                    &item.item_type,
                    ItemType::FunctionDef { .. }
                        | ItemType::ClassDef { .. }
                        | ItemType::Expression
                        | ItemType::If { .. }
                        | ItemType::Try
                )
            })
    }

    /// Check if a module has class definitions
    fn module_has_class_definitions(&self, module: &ModuleDepGraph) -> bool {
        module
            .items
            .values()
            .any(|item| matches!(item.item_type, ItemType::ClassDef { .. }))
    }

    /// Check if a module has module-level imports
    fn module_has_module_level_imports(&self, module: &ModuleDepGraph) -> bool {
        module.items.values().any(|item| {
            matches!(
                item.item_type,
                ItemType::Import { .. } | ItemType::FromImport { .. }
            )
        })
    }

    /// Check if all modules in the cycle are empty or only contain imports
    fn all_modules_empty_or_imports_only(&self, module_names: &[String]) -> bool {
        module_names.iter().all(|module_name| {
            let Some(&module_id) = self.module_names.get(module_name) else {
                return true; // Module not found, assume empty
            };

            let Some(module) = self.modules.get(&module_id) else {
                return true; // Module not found, assume empty
            };

            // Module has no items, or only has import items
            module.items.is_empty()
                || module.items.values().all(|item| {
                    matches!(
                        item.item_type,
                        ItemType::Import { .. } | ItemType::FromImport { .. }
                    )
                })
        })
    }

    /// Check if imported items are only used inside functions
    fn are_imports_used_only_in_functions(&self, module: &ModuleDepGraph) -> bool {
        // Get all imported names from this module
        let mut imported_names = FxHashSet::default();

        // TODO: This has O(n*m) complexity where n is imported names and m is module items.
        // For modules with many imports and items, consider building an index of variable
        // usage upfront to reduce lookup time.

        for item in module.items.values() {
            match &item.item_type {
                ItemType::Import { alias, module } => {
                    let local_name = alias.as_ref().unwrap_or(module).clone();
                    imported_names.insert(local_name.clone());

                    // For dotted imports like `import xml.etree.ElementTree`,
                    // also track the root module name (e.g., "xml")
                    // since that's what appears in read_vars
                    if alias.is_none()
                        && module.contains('.')
                        && let Some(root) = module.split('.').next()
                    {
                        imported_names.insert(root.to_string());
                    }
                }
                ItemType::FromImport { names, .. } => {
                    for (name, alias) in names {
                        imported_names.insert(alias.as_ref().unwrap_or(name).clone());
                    }
                }
                _ => {}
            }
        }

        debug!(
            "Module {} has imported names: {:?}",
            module.module_name, imported_names
        );

        // For each imported name, check if it's only used inside functions
        // We need to check if the import appears in any item's read_vars (module level)
        // vs only appearing in eventual_read_vars (function level)
        for imported_name in &imported_names {
            debug!(
                "Checking usage of imported '{}' in module {}",
                imported_name, module.module_name
            );

            // Check all items in the module
            for (item_id, item_data) in &module.items {
                // Skip import statements themselves
                if matches!(
                    item_data.item_type,
                    ItemType::Import { .. } | ItemType::FromImport { .. }
                ) {
                    continue;
                }

                // If the import is used in read_vars, it's used at module level
                if item_data.read_vars.contains(imported_name) {
                    debug!(
                        "  -> Import '{}' used at module level in item {:?} (type: {:?})",
                        imported_name, item_id, item_data.item_type
                    );
                    return false;
                }

                // Note: Usage in eventual_read_vars is OK - that's function-level usage
                if item_data.eventual_read_vars.contains(imported_name) {
                    debug!(
                        "  -> Import '{imported_name}' used inside function in item {item_id:?}"
                    );
                }
            }
        }

        debug!(
            "All imports in module {} are only used inside functions",
            module.module_name
        );
        true
    }

    /// Check if a cycle is a parent-child package relationship
    fn is_parent_child_package_cycle(&self, module_names: &[String]) -> bool {
        // A parent-child cycle occurs when:
        // 1. We have exactly 2 modules in the cycle
        // 2. One module is a parent package of the other
        if module_names.len() != 2 {
            return false;
        }

        let mod1 = &module_names[0];
        let mod2 = &module_names[1];

        // Check if mod1 is parent of mod2 or vice versa
        mod2.starts_with(&format!("{mod1}.")) || mod1.starts_with(&format!("{mod2}."))
    }

    /// Suggest resolution strategy for a circular dependency
    fn suggest_resolution_for_cycle(
        &self,
        cycle_type: &CircularDependencyType,
        module_names: &[String],
    ) -> ResolutionStrategy {
        // This is a deprecated compatibility function
        let _ = module_names;
        match cycle_type {
            CircularDependencyType::FunctionLevel => ResolutionStrategy::FunctionScopedImport {
                import_to_function: FxHashMap::default(),
                descriptions: vec!["Move imports inside functions that use them".to_string()],
            },
            CircularDependencyType::ClassLevel => ResolutionStrategy::LazyImport {
                module_ids: vec![],
                lazy_var_names: FxHashMap::default(),
            },
            CircularDependencyType::ModuleConstants => ResolutionStrategy::Unresolvable {
                reason: "Module-level constants create temporal paradox - consider moving to a \
                         shared configuration module"
                    .into(),
                manual_suggestions: vec![
                    "Consider moving constants to a separate module".to_string(),
                ],
            },
            CircularDependencyType::ImportTime => ResolutionStrategy::ModuleSplit {
                module_id: ModuleId::new(0),
                suggested_names: vec!["extracted_module".to_string()],
                item_distribution: vec![],
            },
        }
    }

    /// Get all import names that resolve to the same file as the given module
    pub fn get_file_import_names(&self, module_id: ModuleId) -> Vec<String> {
        if let Some(canonical_path) = self.module_canonical_paths.get(&module_id)
            && let Some(names) = self.file_to_import_names.get(canonical_path)
        {
            return names.iter().cloned().collect();
        }
        vec![]
    }

    /// Check if two modules refer to the same file
    pub fn same_file(&self, module_id1: ModuleId, module_id2: ModuleId) -> bool {
        if let (Some(path1), Some(path2)) = (
            self.module_canonical_paths.get(&module_id1),
            self.module_canonical_paths.get(&module_id2),
        ) {
            return path1 == path2;
        }
        false
    }

    /// Get the canonical path for a module
    pub fn get_canonical_path(&self, module_id: ModuleId) -> Option<&PathBuf> {
        self.module_canonical_paths.get(&module_id)
    }

    /// Get the primary module for a given file path.
    /// The path will be canonicalized before lookup.
    pub fn get_primary_module_for_file(&self, path: &Path) -> Option<(String, ModuleId)> {
        let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        self.file_primary_module.get(&canonical).cloned()
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
    fn test_item_dependencies() {
        let mut module = ModuleDepGraph::new(ModuleId::new(0), "test".to_string());

        // Add a function definition
        let func_item = module.add_item(ItemData {
            item_type: ItemType::FunctionDef {
                name: "test_func".into(),
            },
            var_decls: ["test_func".into()].into_iter().collect(),
            read_vars: FxHashSet::default(),
            eventual_read_vars: FxHashSet::default(),
            write_vars: FxHashSet::default(),
            eventual_write_vars: FxHashSet::default(),
            has_side_effects: false,
            span: Some((1, 3)),
            imported_names: FxHashSet::default(),
            reexported_names: FxHashSet::default(),
            defined_symbols: ["test_func".into()].into_iter().collect(),
            symbol_dependencies: FxHashMap::default(),
            attribute_accesses: FxHashMap::default(),
            is_normalized_import: false,
            statement_index: None,
        });

        // Add a call to the function
        let call_item = module.add_item(ItemData {
            item_type: ItemType::Expression,
            var_decls: FxHashSet::default(),
            read_vars: ["test_func".into()].into_iter().collect(),
            eventual_read_vars: FxHashSet::default(),
            write_vars: FxHashSet::default(),
            eventual_write_vars: FxHashSet::default(),
            has_side_effects: true,
            span: Some((5, 5)),
            imported_names: FxHashSet::default(),
            reexported_names: FxHashSet::default(),
            defined_symbols: FxHashSet::default(),
            symbol_dependencies: FxHashMap::default(),
            attribute_accesses: FxHashMap::default(),
            is_normalized_import: false,
            statement_index: None,
        });

        // Add dependency
        module.add_dependency(call_item, func_item, DepType::Strong);

        // Test transitive dependencies
        let deps = module.get_transitive_deps(call_item);
        assert!(deps.contains(&func_item));
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

        // Find cycle paths
        let cycle_paths = graph
            .find_cycle_paths()
            .expect("Cycle path detection should not fail");
        assert!(!cycle_paths.is_empty());

        // Analyze circular dependencies
        let analysis = graph.analyze_circular_dependencies();
        assert_eq!(analysis.total_cycles_detected, 1);
        assert_eq!(analysis.largest_cycle_size, 3);
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

        graph.add_module_dependency(constants_a, constants_b);
        graph.add_module_dependency(constants_b, constants_a);

        let analysis = graph.analyze_circular_dependencies();
        // The new analyzer doesn't use name-based heuristics
        // It classifies cycles based on actual module content
        // Since these modules have no items, they'll be classified as FunctionLevel
        assert_eq!(analysis.resolvable_cycles.len(), 1);
        assert_eq!(
            analysis.resolvable_cycles[0].cycle_type,
            CircularDependencyType::FunctionLevel
        );
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
            span: Some((1, 3)),
            imported_names: FxHashSet::default(),
            reexported_names: FxHashSet::default(),
            defined_symbols: ["helper".into()].into_iter().collect(),
            symbol_dependencies: FxHashMap::default(),
            attribute_accesses: FxHashMap::default(),
            is_normalized_import: false,
            statement_index: None,
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
                span: Some((4, 6)),
                imported_names: FxHashSet::default(),
                reexported_names: FxHashSet::default(),
                defined_symbols: ["new_helper".into()].into_iter().collect(),
                symbol_dependencies: FxHashMap::default(),
                attribute_accesses: FxHashMap::default(),
                is_normalized_import: false,
                statement_index: None,
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
