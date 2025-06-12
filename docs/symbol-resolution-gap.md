# Symbol Resolution Gap in Namespace Creation

## Problem Statement

The current symbol resolution system in Cribo has a fundamental architectural gap: it only tracks symbols that appear in multiple modules (for conflict detection), but when creating namespace objects for inlined modules, we need ALL symbols from those modules, including non-conflicting ones.

This issue was discovered when the `comprehensive_ast_rewrite` test failed after removing hardcoded test values. The test expects `base.initialize()` to work, but the `initialize` symbol isn't included in the namespace because it's unique to the `models.base` module.

## Current Architecture

### Symbol Registry Design

The `SymbolRegistry` is designed primarily for conflict detection:

```rust
pub struct SymbolRegistry {
    /// Symbol name -> list of modules that define it
    pub symbols: FxIndexMap<String, Vec<ModuleId>>,
    /// Renames: (ModuleId, OriginalName) -> NewName
    pub renames: FxIndexMap<(ModuleId, String), String>,
    /// Symbol binding information for scope analysis
    pub symbol_bindings: FxIndexMap<(ModuleId, String), SymbolBindingInfo>,
}
```

The registry tracks:

1. Which modules define each symbol (for conflict detection)
2. How conflicting symbols are renamed
3. Binding information for symbols

### Symbol Collection Flow

1. **Semantic Analysis** (`semantic_bundler.rs`):
   - Extracts ALL symbols from each module
   - Registers them in the global symbol registry
   - Stores complete symbol lists in `ModuleSemanticInfo`

2. **Conflict Detection**:
   - Identifies symbols that appear in multiple modules
   - Generates renames for conflicting symbols

3. **Namespace Creation** (`code_generator.rs`):
   - `collect_module_renames()` iterates over symbols in the registry
   - Only finds symbols that exist in multiple modules
   - Non-conflicting symbols are missed

### The Gap

When creating namespace objects (e.g., `base = types.SimpleNamespace()`), we need to populate them with ALL exported symbols from the module. However, `collect_module_renames()` only iterates over symbols in the registry that have this module in their list:

```rust
// Current implementation - misses non-conflicting symbols
for (symbol, modules) in symbol_registry.symbols.iter() {
    if !modules.contains(&module_id) {
        continue;
    }
    // This only processes symbols that appear in multiple modules
}
```

## Example Scenario

Consider `models.base` module with these symbols:

- `result` (conflicts with other modules) ✓ Included
- `process` (conflicts with other modules) ✓ Included
- `initialize` (unique to this module) ✗ Missing
- `BaseModel` (unique to this module) ✗ Missing
- `shadow_test` (unique to this module) ✗ Missing

The namespace object only gets 5 symbols instead of 8.

## Proposed Solutions

### Solution 1: Extend Symbol Registry (Recommended)

Modify the symbol registry to track all symbols, not just conflicting ones. Add a comprehensive symbol map:

```rust
pub struct SymbolRegistry {
    // Existing fields...
    /// Complete map of module -> all its exported symbols
    pub module_symbols: FxIndexMap<ModuleId, FxIndexSet<String>>,
}
```

**Implementation**:

1. During semantic analysis, populate `module_symbols` with ALL exported symbols
2. In `collect_module_renames()`, use `module_symbols` as the primary source
3. Check for renames in the existing rename map

**Pros**:

- Minimal changes to existing architecture
- Preserves conflict detection functionality
- Single source of truth for module symbols

**Cons**:

- Slight memory overhead (storing symbols twice)
- Need to keep both maps in sync

### Solution 2: Use ModuleSemanticInfo Directly

Leverage the existing `ModuleSemanticInfo` which already contains all exported symbols:

```rust
fn collect_module_renames(
    &self,
    module_name: &str,
    graph: &DependencyGraph,
    symbol_registry: &SymbolRegistry,
    semantic_bundler: &SemanticBundler, // Add this parameter
    symbol_renames: &mut FxIndexMap<String, FxIndexMap<String, String>>,
) {
    // Get module info from semantic bundler
    if let Some(module_info) = semantic_bundler.get_module_info(&module_id) {
        for symbol in &module_info.exported_symbols {
            // Check if symbol has a rename, otherwise use original name
            let renamed = symbol_registry
                .get_rename(&module_id, symbol)
                .unwrap_or(symbol);
            module_renames.insert(symbol.clone(), renamed.to_string());
        }
    }
}
```

**Pros**:

- No changes to data structures
- Uses existing semantic analysis results
- Most accurate - uses the same data that conflict detection used

**Cons**:

- Requires passing `semantic_bundler` through multiple function calls
- Tighter coupling between components

### Solution 3: Two-Phase Symbol Collection

Create a dedicated symbol collection phase that combines conflict detection with comprehensive symbol listing:

```rust
pub struct CompleteSymbolMap {
    /// Module -> (original_name -> final_name)
    pub symbols: FxIndexMap<ModuleId, FxIndexMap<String, String>>,
}

impl CompleteSymbolMap {
    pub fn build(symbol_registry: &SymbolRegistry, semantic_bundler: &SemanticBundler) -> Self {
        // Combine data from both sources
    }
}
```

**Pros**:

- Clear separation of concerns
- Can be optimized for namespace creation use case
- No modifications to existing structures

**Cons**:

- Additional processing step
- Potential for inconsistency if not properly synchronized

### Solution 4: Lazy Symbol Resolution

Instead of pre-collecting all symbols, resolve them on-demand during namespace creation:

```rust
fn create_namespace_assignment(
    &self,
    module_name: &str,
    local_name: &str,
    semantic_bundler: &SemanticBundler,
    symbol_registry: &SymbolRegistry,
) -> Vec<Stmt> {
    // Get all symbols directly from semantic analysis
    let all_symbols = semantic_bundler.get_module_symbols(module_name);

    // Create namespace with all symbols
    for symbol in all_symbols {
        let final_name = symbol_registry
            .get_rename(module_id, &symbol)
            .unwrap_or(&symbol);
        // Add to namespace
    }
}
```

**Pros**:

- No pre-computation needed
- Always uses fresh data
- Memory efficient

**Cons**:

- Requires semantic bundler access at code generation time
- May duplicate work if multiple namespaces for same module

## Recommendation

**Solution 1 (Extend Symbol Registry)** is recommended because:

1. It maintains a clean separation between semantic analysis and code generation
2. It's a natural extension of the existing architecture
3. It provides a single source of truth for all symbol information
4. The implementation is straightforward and low-risk

The memory overhead is minimal compared to the overall bundled output size, and having complete symbol information readily available will likely benefit other features in the future.

## Implementation Plan

1. **Update SymbolRegistry structure**:
   - Add `module_symbols: FxIndexMap<ModuleId, FxIndexSet<String>>`
   - Update `register_symbol()` to populate this map

2. **Modify collect_module_renames()**:
   - Use `module_symbols` as the primary source
   - Fall back to current logic for backward compatibility

3. **Update tests**:
   - Remove `xfail_` prefix from `comprehensive_ast_rewrite`
   - Add specific tests for non-conflicting symbol inclusion

4. **Documentation**:
   - Update architecture docs to reflect complete symbol tracking
   - Add comments explaining the dual-purpose nature of the registry

## Alternative Quick Fix

As a temporary workaround, we could restore the hardcoded symbols but mark them clearly as a hack:

```rust
// HACK: Temporary workaround for symbol resolution gap
// TODO: Remove when proper symbol resolution is implemented
// See docs/architecture/symbol-resolution-gap.md
```

However, this violates our principles and should only be considered if there's an urgent need to make the test pass before implementing the proper solution.
