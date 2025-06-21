# Circular Dependency Module Ordering Issue

## Problem Summary

When circular dependencies are detected in the module graph, the current implementation of `get_modules_with_cycle_resolution` incorrectly orders modules by placing ALL non-cycle modules before ALL cycle modules. This breaks the fundamental requirement that modules must be initialized before their dependents.

### Specific Example

In the `comprehensive_ast_rewrite` test:

1. **Circular dependencies detected:**
   - `services.auth → services` (package imports its submodule)
   - `models → models.base` (package imports its submodule)
   - `core → core.utils` (package imports its submodule)

2. **Dependency chain:**
   - `services.auth.manager` imports from `models.base`
   - `services.auth.manager` → `models.base` (cross-cycle dependency)

3. **Current incorrect ordering:**
   ```
   [non-cycle modules first]
   services.auth.manager  (non-cycle, but depends on models.base!)
   ...
   [cycle modules last]
   models.base           (part of cycle with models)
   ```

4. **Result:** `services.auth.manager` tries to access `models.base` before it's initialized, causing `KeyError: 'models.base'`

## Root Cause Analysis

The bug is in `orchestrator.rs::get_modules_with_cycle_resolution()` (lines 380-443):

```rust
// Current implementation - INCORRECT
let (mut cycle_ids, non_cycle_ids): (Vec<_>, Vec<_>) =
    all_module_ids.into_iter().partition(|&module_id| {
        // Separates cycle vs non-cycle modules
    });

// Add non-cycle modules first (they should sort topologically)
result.extend(non_cycle_ids);  // ❌ WRONG: Ignores dependencies to cycle modules!

// For cycle modules, try to maintain dependency order where possible
// ... then adds cycle modules ...
```

This approach fundamentally breaks because it assumes non-cycle modules can be initialized before cycle modules, but non-cycle modules can depend on cycle modules!

## Proposed Fix

### Algorithm Overview

Instead of partitioning modules into cycle/non-cycle groups, we need to:

1. **Build a modified dependency graph** where we break cycles by removing intra-cycle edges
2. **Perform standard topological sort** on this modified graph
3. **Preserve all dependencies** from non-cycle modules to cycle modules

### Detailed Implementation

```rust
fn get_modules_with_cycle_resolution(
    &self,
    graph: &CriboGraph,
    analysis: &CircularDependencyAnalysis,
) -> Result<Vec<ModuleId>> {
    // Step 1: Create a modified graph for topological sorting
    let mut modified_graph = petgraph::Graph::<ModuleId, ()>::new();
    let mut node_map = FxIndexMap::default();

    // Add all nodes
    for module_id in graph.modules.keys() {
        let node_idx = modified_graph.add_node(*module_id);
        node_map.insert(*module_id, node_idx);
    }

    // Step 2: Identify all modules in cycles
    let mut cycle_modules = FxIndexSet::default();
    for cycle in &analysis.resolvable_cycles {
        for module_name in &cycle.modules {
            if let Some(module_id) = graph.get_module_id_by_name(module_name) {
                cycle_modules.insert(module_id);
            }
        }
    }

    // Step 3: Add edges, but skip intra-cycle edges
    for (from_id, to_id) in graph.get_all_edges() {
        let is_intra_cycle_edge = cycle_modules.contains(&from_id)
            && cycle_modules.contains(&to_id)
            && are_in_same_cycle(from_id, to_id, analysis);

        if !is_intra_cycle_edge {
            // Keep this edge - it's either:
            // - Between non-cycle modules
            // - From non-cycle to cycle module (important!)
            // - From cycle to non-cycle module
            // - Between different cycles
            if let (Some(&from_idx), Some(&to_idx)) = (node_map.get(&from_id), node_map.get(&to_id))
            {
                modified_graph.add_edge(from_idx, to_idx, ());
            }
        }
    }

    // Step 4: Perform topological sort on modified graph
    let sorted_nodes = petgraph::algo::toposort(&modified_graph, None)
        .map_err(|_| anyhow!("Failed to sort even after breaking cycles"))?;

    // Convert back to module IDs
    Ok(sorted_nodes
        .into_iter()
        .map(|node_idx| modified_graph[node_idx])
        .collect())
}

// Helper function to check if two modules are in the same cycle
fn are_in_same_cycle(
    module_a: ModuleId,
    module_b: ModuleId,
    analysis: &CircularDependencyAnalysis,
) -> bool {
    // Check if both modules appear in the same cycle
    for cycle in &analysis.resolvable_cycles {
        let has_a = cycle
            .modules
            .iter()
            .any(|name| graph.get_module_id_by_name(name) == Some(module_a));
        let has_b = cycle
            .modules
            .iter()
            .any(|name| graph.get_module_id_by_name(name) == Some(module_b));
        if has_a && has_b {
            return true;
        }
    }
    false
}
```

### Key Improvements

1. **Preserves cross-cycle dependencies**: Dependencies from non-cycle modules to cycle modules are kept intact
2. **Breaks only necessary edges**: Only removes edges within the same strongly connected component
3. **Uses standard topological sort**: After edge removal, uses proven algorithm for ordering
4. **Handles multiple cycles**: Works correctly even with multiple independent cycles

## Testing Strategy

1. **Add unit test** for `get_modules_with_cycle_resolution` with the specific case:
   ```
   A (non-cycle) → B (in cycle with C)
   C → B (cycle edge)
   ```
   Expected order: C or B, then B or C, then A

2. **Verify existing tests** pass, especially:
   - `comprehensive_ast_rewrite`
   - `stickytape_explicit_relative_import_single_dot`
   - All cycle-related tests

3. **Add regression test** specifically for cross-cycle dependencies

## Implementation Notes

1. **Performance**: The modified algorithm has similar complexity to the original
2. **Determinism**: Use stable sorting within cycles for reproducible output
3. **Edge cases**: Handle package/submodule relationships specially if needed

## Alternative Approaches Considered

1. **Lazy initialization**: Initialize modules on-demand when imported
   - Rejected: Too complex, changes bundler architecture significantly

2. **Pre-compute full ordering**: Use Tarjan's SCC algorithm to find cycles, then order
   - This is essentially what the proposed fix does, but more explicitly

3. **Forbid cross-cycle dependencies**: Reject bundles with this pattern
   - Rejected: Too restrictive, valid Python code

## Conclusion

The current cycle resolution algorithm makes an incorrect assumption that non-cycle modules can always be initialized before cycle modules. The proposed fix maintains the graph structure while only breaking edges within cycles, allowing proper topological sorting that respects ALL dependencies.
