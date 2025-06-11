# CriboGraph: Advanced Dependency Graph Implementation

## Overview

CriboGraph is our custom dependency graph implementation that combines the best architectural patterns from three leading JavaScript bundlers:

1. **Turbopack**: Fine-grained item-level dependency tracking
2. **Rspack**: Incremental update support
3. **Mako**: Efficient graph algorithms using petgraph

## Key Features

### 1. Fine-Grained Dependency Tracking (Turbopack-inspired)

Unlike traditional module-level dependency graphs, CriboGraph tracks dependencies at the statement/item level:

```rust
pub enum ItemType {
    FunctionDef {
        name: String,
    },
    ClassDef {
        name: String,
    },
    Assignment {
        targets: Vec<String>,
    },
    Import {
        module: String,
    },
    FromImport {
        module: String,
        names: Vec<(String, Option<String>)>,
        level: u32, // relative import level
    },
    Expression,
    If {
        condition: String,
    },
    Try,
    Other,
}
```

This enables:

- Precise tree shaking by tracking which functions/classes are actually used
- Better handling of conditional imports
- Accurate side effect tracking

### 2. Incremental Updates (Rspack-inspired)

The graph supports incremental updates through a pending updates queue:

```rust
pub struct GraphUpdate {
    pub module_updates: Vec<ModuleUpdate>,
    pub new_deps: Vec<(ModuleId, ModuleId)>,
    pub removed_deps: Vec<(ModuleId, ModuleId)>,
}
```

Benefits:

- Efficient re-bundling when only a few modules change
- Batched updates for better performance
- Clear separation between analysis and application phases

### 3. Efficient Graph Algorithms (Mako-inspired)

Using petgraph provides:

- O(V + E) topological sorting
- Efficient cycle detection with Kosaraju's algorithm
- Fast neighbor queries for dependents/dependencies

## Architecture

### Two-Level Structure

1. **CriboGraph**: High-level graph managing modules
2. **ModuleDepGraph**: Fine-grained graph for items within each module

### Key Components

#### Module Management

```rust
pub struct CriboGraph {
    modules: FxHashMap<ModuleId, ModuleDepGraph>,
    module_names: FxHashMap<String, ModuleId>,
    module_paths: FxHashMap<PathBuf, ModuleId>,
    module_metadata: FxHashMap<ModuleId, ModuleMetadata>,
    graph: DiGraph<ModuleId, ()>,
    node_indices: FxHashMap<ModuleId, NodeIndex>,
    pending_updates: Vec<GraphUpdate>,
}
```

#### Variable State Tracking

```rust
pub struct VarState {
    pub declarator: Option<ItemId>,
    pub writers: Vec<ItemId>,
    pub readers: Vec<ItemId>,
}
```

#### Dependency Types

```rust
pub enum DepType {
    Strong, // Always needed
    Weak,   // Only if target is included
}
```

## Usage Examples

### Creating a Module Graph

```rust
let mut graph = CriboGraph::new();

// Add modules
let utils_id = graph.add_module("utils".to_string(), PathBuf::from("utils.py"));
let main_id = graph.add_module("main".to_string(), PathBuf::from("main.py"));

// Add dependency: main depends on utils
graph.add_module_dependency(main_id, utils_id);

// Get topologically sorted modules
let sorted = graph.topological_sort()?; // [utils_id, main_id]
```

### Fine-Grained Item Tracking

```rust
let mut module = ModuleDepGraph::new(module_id, "mymodule".to_string());

// Add a function definition
let func_item = module.add_item(ItemData {
    item_type: ItemType::FunctionDef { name: "process".to_string() },
    var_decls: ["process".to_string()].into_iter().collect(),
    has_side_effects: false,
    // ... other fields
});

// Add a call to the function
let call_item = module.add_item(ItemData {
    item_type: ItemType::Expression,
    read_vars: ["process".to_string()].into_iter().collect(),
    has_side_effects: true,
    // ... other fields
});

// Track dependency
module.add_dependency(call_item, func_item, DepType::Strong);
```

### Tree Shaking

```rust
// Find all items needed for specific exports
let used_symbols = ["main", "process"].into_iter().map(String::from).collect();
let required_items = module.tree_shake(&used_symbols);
```

### Cycle Detection

```rust
// Find circular dependencies
let cycles = graph.find_cycles();
for cycle in cycles {
    println!("Circular dependency detected: {:?}", cycle);
}
```

## Performance Characteristics

- **Module lookups**: O(1) using FxHashMap
- **Topological sort**: O(V + E) using petgraph
- **Cycle detection**: O(V + E) using Kosaraju's algorithm
- **Tree shaking**: O(V + E) graph traversal
- **Incremental updates**: O(k) where k is number of changes

## Future Enhancements

1. **AST Integration**: Connect with Python AST parser to automatically extract items
2. **Parallel Analysis**: Leverage Rust's concurrency for parallel module analysis
3. **Caching**: Add content hashing for unchanged module detection
4. **Visualization**: Export to GraphViz or other formats for debugging

## Comparison with Existing DependencyGraph

| Feature              | Old DependencyGraph | CriboGraph     |
| -------------------- | ------------------- | -------------- |
| Granularity          | Module-level        | Item-level     |
| Graph Library        | Custom              | petgraph       |
| Incremental Updates  | No                  | Yes            |
| Variable Tracking    | No                  | Yes            |
| Side Effect Tracking | Basic               | Detailed       |
| Tree Shaking         | Module-level        | Function-level |

## Migration Path

1. **Phase 1**: Use CriboGraph alongside existing graph for comparison
2. **Phase 2**: Migrate analysis passes to use fine-grained data
3. **Phase 3**: Update code generation to leverage item-level info
4. **Phase 4**: Remove old dependency graph

## Conclusion

CriboGraph represents a significant advancement in Python bundling technology by bringing JavaScript bundler innovations to the Python ecosystem. The combination of fine-grained tracking, incremental updates, and efficient algorithms positions Cribo to handle even the most complex Python projects with excellent performance.
