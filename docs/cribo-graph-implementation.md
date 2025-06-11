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
- Efficient cycle detection with Tarjan's strongly connected components algorithm
- Fast neighbor queries for dependents/dependencies
- Advanced circular dependency analysis and classification

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

#### Enhanced Circular Dependency Analysis

```rust
pub enum CircularDependencyType {
    FunctionLevel,   // Resolvable by moving imports inside functions
    ClassLevel,      // May be resolvable depending on usage patterns
    ModuleConstants, // Unresolvable - temporal paradox
    ImportTime,      // Depends on execution order
}

pub enum ResolutionStrategy {
    LazyImport { modules: Vec<String> },
    FunctionScopedImport { import_statements: Vec<String> },
    ModuleSplit { suggestions: Vec<String> },
    Unresolvable { reason: String },
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

### Unused Import Detection

```rust
// Find unused imports in a module
let unused_imports = module.find_unused_imports(is_init_py);

for unused in unused_imports {
    println!("Unused import '{}' from '{}' (line {:?})",
        unused.name, unused.module, unused.item_id);
}

// The detection respects:
// - __all__ exports via reexported_names tracking
// - Star imports (always preserved)
// - Side-effect imports (e.g., logging.config)
// - __init__.py context (all imports preserved)
```

### Cycle Detection and Analysis

```rust
// Find circular dependencies with enhanced analysis
let analysis = graph.analyze_circular_dependencies();

// Handle resolvable cycles
for cycle in &analysis.resolvable_cycles {
    println!("Resolvable cycle ({}): {}", 
        cycle.cycle_type, cycle.modules.join(" → "));
    match &cycle.suggested_resolution {
        ResolutionStrategy::FunctionScopedImport { import_statements } => {
            for suggestion in import_statements {
                println!("  - {}", suggestion);
            }
        }
        _ => {}
    }
}

// Report unresolvable cycles
for cycle in &analysis.unresolvable_cycles {
    println!("UNRESOLVABLE cycle: {}", cycle.modules.join(" → "));
}
```

## Performance Characteristics

- **Module lookups**: O(1) using FxHashMap
- **Topological sort**: O(V + E) using petgraph
- **Cycle detection**: O(V + E) using Tarjan's algorithm (more efficient than Kosaraju for our use case)
- **Circular dependency analysis**: O(V + E) + O(c) where c is number of cycles
- **Tree shaking**: O(V + E) graph traversal
- **Incremental updates**: O(k) where k is number of changes
- **Unused import detection**: O(n*m) where n is imports and m is items in module

## Recent Enhancements (v0.4.23)

### 1. Graph-Based Unused Import Detection

The unused import detection system has been migrated from AST-based to graph-based approach:

- **Improved accuracy**: Uses actual variable usage tracking from the dependency graph
- **Better **all** support**: Correctly detects exports in `__all__` lists by checking `reexported_names`
- **Context-aware**: Preserves imports in `__init__.py` files and star imports
- **Side effect awareness**: Preserves imports that have side effects

### 2. Enhanced Circular Dependency Analysis

- **Tarjan's Algorithm**: Replaced Kosaraju's algorithm for more efficient SCC detection
- **Intelligent Classification**: Analyzes AST to classify cycles (function-level, class-level, etc.)
- **Resolution Suggestions**: Provides actionable suggestions for breaking cycles
- **Parent-Child Detection**: Recognizes normal Python package patterns vs problematic cycles

### 3. Improved Variable Tracking

- **Reexported Names**: New field in `ItemData` to track explicit re-exports
- **Import Alias Support**: Better handling of import aliases and their usage
- **Dotted Name Resolution**: Improved support for `xml.etree.ElementTree` style imports

## Future Enhancements

1. **Lazy Import Generation**: Automatically generate lazy import patterns for resolvable cycles
2. **Parallel Analysis**: Leverage Rust's concurrency for parallel module analysis
3. **Caching**: Add content hashing for unchanged module detection
4. **Visualization**: Export to GraphViz or other formats for debugging
5. **Import Optimization**: Suggest import reorganization based on usage patterns

## Comparison with Existing DependencyGraph

| Feature                 | Old DependencyGraph | CriboGraph     |
| ----------------------- | ------------------- | -------------- |
| Granularity             | Module-level        | Item-level     |
| Graph Library           | Custom              | petgraph       |
| Incremental Updates     | No                  | Yes            |
| Variable Tracking       | No                  | Yes            |
| Side Effect Tracking    | Basic               | Detailed       |
| Tree Shaking            | Module-level        | Function-level |
| Unused Import Detection | AST-based           | Graph-based    |
| Circular Dependency     | Basic detection     | Full analysis  |
| **all** Export Tracking | Limited             | Full support   |
| Import Alias Resolution | Basic               | Advanced       |

## Migration Path

1. **Phase 1**: Use CriboGraph alongside existing graph for comparison
2. **Phase 2**: Migrate analysis passes to use fine-grained data
3. **Phase 3**: Update code generation to leverage item-level info
4. **Phase 4**: Remove old dependency graph

## Conclusion

CriboGraph represents a significant advancement in Python bundling technology by bringing JavaScript bundler innovations to the Python ecosystem. The combination of fine-grained tracking, incremental updates, and efficient algorithms positions Cribo to handle even the most complex Python projects with excellent performance.
