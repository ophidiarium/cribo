# CriboGraph Circular Dependency Detection

## Overview

We've successfully implemented Tarjan's strongly connected components algorithm and comprehensive circular dependency analysis in CriboGraph, matching the functionality from the existing `dependency_graph.rs` but with our enhanced architecture.

## Key Features Implemented

### 1. Tarjan's Algorithm for SCC Detection

```rust
pub fn find_strongly_connected_components(&self) -> Vec<Vec<ModuleId>>
```

- More efficient than Kosaraju's algorithm for our use case
- Returns components in reverse topological order
- Filters out single-node components (no self-cycles)

### 2. Cycle Path Detection with DFS

```rust
pub fn find_cycle_paths(&self) -> Result<Vec<Vec<String>>>
```

- Uses three-color marking (White/Gray/Black)
- Finds all cycle paths in the graph
- Returns module names for easy debugging

### 3. Comprehensive Circular Dependency Analysis

```rust
pub fn analyze_circular_dependencies(&self) -> CircularDependencyAnalysis
```

Returns detailed analysis including:

- Resolvable vs unresolvable cycles
- Cycle classification by type
- Import chains showing exact dependencies
- Resolution suggestions

### 4. Cycle Classification

Four types of circular dependencies:

1. **FunctionLevel**: Can be resolved by moving imports inside functions
2. **ClassLevel**: May be resolvable with lazy imports
3. **ModuleConstants**: Unresolvable temporal paradox
4. **ImportTime**: Depends on execution order

### 5. Resolution Strategies

```rust
pub enum ResolutionStrategy {
    LazyImport { modules: Vec<String> },
    FunctionScopedImport { import_statements: Vec<String> },
    ModuleSplit { suggestions: Vec<String> },
    Unresolvable { reason: String },
}
```

## Usage Example

```rust
let mut graph = CriboGraph::new();

// Add modules
let module_a = graph.add_module("module_a".to_string(), PathBuf::from("module_a.py"));
let module_b = graph.add_module("module_b".to_string(), PathBuf::from("module_b.py"));
let module_c = graph.add_module("module_c".to_string(), PathBuf::from("module_c.py"));

// Create circular dependency: A -> B -> C -> A
graph.add_module_dependency(module_a, module_b);
graph.add_module_dependency(module_b, module_c);
graph.add_module_dependency(module_c, module_a);

// Analyze
let analysis = graph.analyze_circular_dependencies();

// Results
println!("Total cycles: {}", analysis.total_cycles_detected);
println!("Largest cycle: {} modules", analysis.largest_cycle_size);

for group in &analysis.resolvable_cycles {
    println!("Resolvable cycle: {:?}", group.modules);
    println!("Type: {:?}", group.cycle_type);
    println!("Suggestion: {:?}", group.suggested_resolution);
}

for group in &analysis.unresolvable_cycles {
    println!("UNRESOLVABLE cycle: {:?}", group.modules);
    if let ResolutionStrategy::Unresolvable { reason } = &group.suggested_resolution {
        println!("Reason: {}", reason);
    }
}
```

## Integration with Fine-Grained Tracking

While the current implementation works at the module level, CriboGraph's architecture allows for future enhancement to analyze cycles at the item level:

```rust
// Future: Analyze which specific functions/classes create the cycle
for module in &scc {
    let module_graph = &graph.modules[module];
    // Analyze items that import from other modules in the SCC
    for (item_id, item_data) in &module_graph.items {
        match &item_data.item_type {
            ItemType::FromImport { module, names, .. } => {
                // Check if this import contributes to the cycle
            }
            _ => {}
        }
    }
}
```

## Algorithm Details

### Tarjan's SCC Algorithm

1. Performs DFS traversal maintaining:
   - `index`: Discovery time of each node
   - `lowlink`: Lowest node reachable
   - `stack`: Current path being explored
   - `on_stack`: Quick lookup for cycle detection

2. When `lowlink[v] == index[v]`, v is the root of an SCC

3. Time complexity: O(V + E)

### DFS Cycle Path Detection

1. Three-color marking:
   - White: Unvisited
   - Gray: Currently in DFS path
   - Black: Fully processed

2. When encountering a Gray node, we've found a cycle

3. Extract cycle from current path

## Testing

Comprehensive tests cover:

1. **Basic cycle detection**: Three-module circular dependency
2. **Classification**: Detecting unresolvable constants cycles
3. **Multiple cycles**: Handling complex dependency networks
4. **Edge cases**: Self-references, disconnected components

## Advantages Over Basic Implementation

1. **Performance**: Tarjan's algorithm is optimal for SCC detection
2. **Classification**: Intelligent categorization of cycle types
3. **Actionable Suggestions**: Specific resolution strategies
4. **Fine-grained Ready**: Architecture supports item-level analysis
5. **Incremental Updates**: Can efficiently recompute after changes

## Future Enhancements

1. **AST Integration**: Use actual import statements for classification
2. **Runtime Helpers**: Generate code to handle resolvable cycles
3. **Visualization**: Export cycle graphs for debugging
4. **Heuristics**: Learn from user resolutions to improve suggestions

## Conclusion

CriboGraph now has industrial-strength circular dependency detection that matches and exceeds the capabilities of the existing implementation. The combination of efficient algorithms, detailed analysis, and actionable suggestions positions Cribo to handle even the most complex Python dependency scenarios.
