# Module ID Ownership and The Fundamental Zero

## Executive Summary

This proposal moves `ModuleId` ownership from `CriboGraph` to `ModuleResolver`, establishing a fundamental truth: **the entry point is ID 0**. This isn't arbitrary - it reflects the reality that bundling starts from a single entry point. This architectural change eliminates complex entry detection logic, fixes incorrect relative import resolution, and provides deterministic single-pass module discovery.

## The Philosophy of Zero

In bundling, everything starts from the entry point. It's the origin, the root, the beginning. Making it ID 0 is not just convenient - it's philosophically correct:

- **ID 0**: The entry point, where bundling begins
- **ID 1+**: Modules discovered during traversal, in order

This eliminates the need for complex boolean flags (`is_entry_module`) and path-based detection scattered throughout the codebase.

## Problem Statement

### Current Complexity

The codebase currently has complicated logic to detect and track the entry point:

```rust
// In orchestrator.rs - complex path-based detection
// Auto-detect the entry point's directory as a source directory
let filename = entry_path.file_name().and_then(|f| f.to_str());
if filename == Some("__init__.py") || filename == Some("__main__.py") {
    // Special handling...
}

// In import_transformer.rs - boolean flag passed everywhere
pub struct ImportTransformer {
    is_entry_module: bool,  // Tracked separately
    // ...
}

// Conditional logic scattered throughout
if self.is_entry_module {
    // Special entry module handling
}
```

### Core Issues

1. **Entry Detection Complexity**: Path analysis, boolean flags, special cases
2. **Lost Package Information**: Cannot distinguish between regular modules and packages (`__init__.py`)
3. **Incorrect Relative Import Resolution**: Without package information, relative imports fail
4. **Scattered Identity Logic**: Module registration spread across components

## Proposed Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                    ModuleResolver                            │
│  Single source of truth for module identity                  │
│                                                               │
│  Fundamental Truth: Entry Module = ID 0                      │
│                                                               │
│  Owns:                                                        │
│  - ModuleId type with ENTRY constant (0)                     │
│  - Module registration (sequential from 0)                   │
│  - Module metadata (name, path, is_package)                  │
│  - Entry point detection (built-in, not derived)             │
└────────────────────────┬────────────────────────────────────┘
                         │ Provides ModuleId
                         ▼
┌─────────────────────────────────────────────────────────────┐
│                      CriboGraph                              │
│  Pure dependency relationship tracking                        │
└─────────────────────────────────────────────────────────────┘
```

## Implementation Design

### Phase 1: ModuleId with Entry Point Semantics

```rust
// In crates/cribo/src/resolver.rs

/// Unique identifier for a module in the dependency graph
/// The entry module ALWAYS has ID 0 - this is a fundamental invariant
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ModuleId(pub u32);

impl ModuleId {
    /// The entry point - always ID 0
    /// This is where bundling starts, the origin of our module universe
    pub const ENTRY: ModuleId = ModuleId(0);

    #[inline]
    pub const fn new(id: u32) -> Self {
        Self(id)
    }

    #[inline]
    pub const fn as_u32(self) -> u32 {
        self.0
    }

    /// Check if this is the entry module
    /// No more complex path detection or boolean flags!
    #[inline]
    pub const fn is_entry(self) -> bool {
        self.0 == 0
    }
}

/// Module metadata tracked by resolver
#[derive(Debug, Clone)]
pub struct ModuleMetadata {
    pub id: ModuleId,
    pub name: String,
    pub canonical_path: PathBuf,
    pub is_package: bool,
    // No more is_entry flag - just check id.is_entry()!
}
```

### Phase 2: Simplified Entry Point Handling

```rust
/// Internal module registry
struct ModuleRegistry {
    next_id: u32,
    by_id: FxIndexMap<ModuleId, ModuleMetadata>,
    by_name: FxIndexMap<String, ModuleId>,
    by_path: FxIndexMap<PathBuf, ModuleId>,
}

impl ModuleRegistry {
    fn new() -> Self {
        Self {
            next_id: 0, // Start at 0 - entry point gets this
            by_id: FxIndexMap::default(),
            by_name: FxIndexMap::default(),
            by_path: FxIndexMap::default(),
        }
    }

    fn register(&mut self, name: String, path: &Path) -> ModuleId {
        let canonical_path = path.canonicalize().unwrap_or_else(|_| path.to_owned());

        // Check for duplicates
        if let Some(&id) = self.by_name.get(&name) {
            if self.by_id[&id].canonical_path == canonical_path {
                return id;
            }
        }

        if let Some(&id) = self.by_path.get(&canonical_path) {
            self.by_name.insert(name, id);
            return id;
        }

        // Allocate ID - entry gets 0, others get sequential IDs
        let id = ModuleId::new(self.next_id);
        self.next_id += 1;

        // The beauty: first registered module (entry) automatically gets ID 0!
        debug_assert!(
            id != ModuleId::ENTRY || self.by_id.is_empty(),
            "Entry module must be registered first"
        );

        let is_package = path
            .file_name()
            .map(|n| n == "__init__.py")
            .unwrap_or(false);

        let metadata = ModuleMetadata {
            id,
            name: name.clone(),
            canonical_path: canonical_path.clone(),
            is_package,
        };

        self.by_id.insert(id, metadata);
        self.by_name.insert(name, id);
        self.by_path.insert(canonical_path, id);

        id
    }
}
```

### Phase 3: Eliminating is_entry_module Flags

```rust
// OLD: ImportTransformer with boolean flag
pub struct ImportTransformer {
    is_entry_module: bool, // Passed through constructor
                           // ...
}

// NEW: ImportTransformer with module ID
pub struct ImportTransformer {
    module_id: ModuleId, // Just store the ID
                         // ...
}

impl ImportTransformer {
    fn transform_import(&mut self, import: &Import) -> Vec<Stmt> {
        // OLD: Complex boolean check
        // if self.is_entry_module { ... }

        // NEW: Clean, semantic check
        if self.module_id.is_entry() {
            // Handle entry-specific logic
        }

        // Even cleaner with pattern matching
        match self.module_id {
            ModuleId::ENTRY => {
                // Entry module logic
            }
            _ => {
                // Non-entry module logic
            }
        }
    }
}
```

### Phase 4: Orchestrator Simplification

```rust
impl BundlerOrchestrator {
    pub fn new(config: Config, entry_path: PathBuf) -> Self {
        let resolver = Arc::new(ModuleResolver::new(config.clone()));

        // The entry module MUST be registered first and WILL get ID 0
        let entry_name = derive_module_name(&entry_path);
        let entry_id = resolver.register_module(entry_name.clone(), &entry_path);

        // This is not a fragile assertion - it's documenting a fundamental invariant
        assert_eq!(
            entry_id,
            ModuleId::ENTRY,
            "Entry module must be ID 0 - bundling starts here"
        );

        Self {
            config,
            resolver,
            entry_path,
            entry_id, // Always ModuleId(0)
                      // No more is_entry flags to track!
        }
    }

    fn process_module(
        &mut self,
        module_path: &Path,
        module_name: &str,
        graph: &mut CriboGraph,
    ) -> Result<ProcessedModule> {
        // Register with resolver - entry already has ID 0
        let module_id = self
            .resolver
            .register_module(module_name.to_string(), module_path);

        // No need to track is_entry - it's encoded in the ID!
        graph.register_module(module_id, module_name.to_string(), module_path.to_owned());

        Ok(ProcessedModule {
            module_id: Some(module_id),
            // No is_entry field needed
        })
    }
}
```

### Phase 5: Resolver with Clean Semantics

```rust
pub struct ModuleResolver {
    config: Config,
    registry: Mutex<ModuleRegistry>,
    // Caches remain the same
}

impl ModuleResolver {
    /// Register a module - entry gets 0, others get sequential IDs
    pub fn register_module(&self, name: String, path: &Path) -> ModuleId {
        let mut registry = self.registry.lock().expect("Module registry lock poisoned");

        let id = registry.register(name.clone(), path);

        if id.is_entry() {
            info!("Registered ENTRY module '{}' at the origin (ID 0)", name);
        } else {
            debug!(
                "Registered module '{}' with ID {} (package: {})",
                name,
                id.as_u32(),
                registry.by_id[&id].is_package
            );
        }

        id
    }

    /// Check if a module is the entry point
    pub fn is_entry_module(&self, id: ModuleId) -> bool {
        id.is_entry() // Simple!
    }

    /// Get the entry module metadata
    pub fn get_entry_module(&self) -> Option<ModuleMetadata> {
        self.get_module(ModuleId::ENTRY)
    }

    /// Resolve relative import with full module context
    pub fn resolve_relative_import(
        &self,
        module_id: ModuleId,
        level: u32,
        name: Option<&str>,
    ) -> Result<String> {
        let registry = self.registry.lock().expect("Module registry lock poisoned");

        let metadata = registry
            .by_id
            .get(&module_id)
            .ok_or_else(|| anyhow!("Unknown module ID: {}", module_id.as_u32()))?;

        // Entry module might have special import rules
        if module_id.is_entry() {
            // Handle any entry-specific import resolution
        }

        Ok(resolve_relative_import_pure(
            &metadata.name,
            metadata.is_package,
            level,
            name,
        ))
    }
}
```

## Benefits of ID 0 as Entry Point

### 1. Eliminates Complex Detection Logic

**Before**: Complex path analysis, boolean flags, special cases

```rust
// Scattered throughout codebase
is_entry_module: bool
if filename == "__main__.py" { /* special handling */ }
```

**After**: Single source of truth

```rust
if module_id.is_entry() { /* entry logic */ }
```

### 2. Natural Ordering

- ID 0: Entry point (where we start)
- ID 1: First discovered import
- ID 2: Second discovered import
- etc.

This matches the mental model of how bundling works!

### 3. Debugging Simplicity

When debugging, you immediately know:

- Module 0 is where the user started
- Higher IDs were discovered later
- The discovery order is preserved in the IDs

### 4. API Clarity

```rust
// Clear, semantic API
ModuleId::ENTRY           // The beginning
module_id.is_entry()      // Check if we're at the start
resolver.get_entry_module() // Get the origin
```

## Migration Path

### Step 1: Add ModuleId to Resolver

1. Move ModuleId with ENTRY constant to resolver.rs
2. Add compatibility re-export
3. Implement registry with ID 0 for entry

### Step 2: Replace is_entry_module Flags

1. Find all `is_entry_module: bool` fields
2. Replace with `module_id: ModuleId`
3. Update checks to use `module_id.is_entry()`

### Step 3: Remove Path-Based Detection

1. Remove complex entry detection in orchestrator
2. Trust that first registered module is entry
3. Simplify module name derivation

### Step 4: Update Components

1. Update ImportTransformer to use module_id
2. Update other components to check ID instead of flags
3. Remove redundant entry tracking

## Testing Strategy

```rust
#[test]
fn test_entry_is_always_zero() {
    let resolver = ModuleResolver::new(Default::default());

    // The first module registered MUST be the entry
    let entry = resolver.register_module("main".into(), &PathBuf::from("main.py"));
    assert_eq!(entry, ModuleId::ENTRY);
    assert!(entry.is_entry());
    assert_eq!(entry.as_u32(), 0);
}

#[test]
fn test_sequential_ids_after_entry() {
    let resolver = ModuleResolver::new(Default::default());

    // Entry gets 0
    let entry = resolver.register_module("main".into(), &PathBuf::from("main.py"));
    assert_eq!(entry.as_u32(), 0);

    // Next modules get sequential IDs
    let utils = resolver.register_module("utils".into(), &PathBuf::from("utils.py"));
    assert_eq!(utils.as_u32(), 1);
    assert!(!utils.is_entry());

    let helpers = resolver.register_module("helpers".into(), &PathBuf::from("helpers.py"));
    assert_eq!(helpers.as_u32(), 2);
}

#[test]
fn test_entry_module_special_handling() {
    let transformer = ImportTransformer::new(ModuleId::ENTRY /* ... */);
    // Test that entry module imports are handled correctly

    let transformer = ImportTransformer::new(ModuleId::new(1) /* ... */);
    // Test that non-entry module imports are handled differently
}
```

## Conclusion

Making the entry module ID 0 is not just a technical decision - it's acknowledging a fundamental truth about bundling. Everything starts from the entry point. It's the origin, the zero point of our coordinate system.

This change eliminates complex detection logic, removes boolean flags, and provides a clean, semantic API that matches how we think about bundling. The entry point isn't just "some module that happens to be first" - it's THE beginning, and its ID should reflect that: **0**.

## Phase 6: Eliminating entry_module_name from BundleParams

Currently, `BundleParams` carries `entry_module_name` which is used throughout the bundler to identify the entry module. Since we've established that the entry module is ALWAYS ID 0, we can eliminate this redundant tracking.

### Current Usage Analysis

The `entry_module_name` is currently used for:

1. **Entry module detection**: Checking if a module is the entry module by comparing names
2. **Package context**: Determining if entry is a package `__init__.py` or `__main__.py`
3. **Skipping transformations**: Entry module gets special treatment in various places
4. **Symbol collection**: Finding global symbols from the entry module

### Refactoring Strategy

Since every module in the bundling process is already registered with the resolver and has a ModuleId, we should ONLY pass ModuleIds and query the resolver for module information:

1. **Use ModuleId as the single identifier**: No more passing (name, path) tuples
   ```rust
   // OLD: (name, ast, path, content_hash)
   // NEW: (module_id, ast, content_hash)
   // Path and name are retrieved from resolver when needed
   ```

2. **Replace name comparisons with ID checks**:
   ```rust
   // OLD: if module_name == params.entry_module_name
   // NEW: if module_id == ModuleId::ENTRY
   ```

3. **Add resolver query methods**:
   ```rust
   impl ModuleResolver {
       pub fn get_module_name(&self, id: ModuleId) -> Option<String> {
           // Get module name from registry
       }

       pub fn get_module_path(&self, id: ModuleId) -> Option<PathBuf> {
           // Get module path from registry
       }

       pub fn is_entry_package(&self) -> bool {
           // Check if entry module (ID 0) is a package
           self.get_module(ModuleId::ENTRY)
               .map(|m| m.is_package)
               .unwrap_or(false)
       }
   }
   ```

4. **Update BundleParams**: Use ModuleIds and resolver reference
   ```rust
   pub struct BundleParams<'a> {
       pub modules: &'a [(ModuleId, ModModule, String)], // (id, ast, content_hash)
       pub sorted_module_ids: &'a [ModuleId],            // Just IDs, get details from resolver
       pub resolver: &'a ModuleResolver,                 // To query module info
       // entry_module_name removed - use ModuleId::ENTRY
       pub graph: &'a DependencyGraph,
       // ... other fields
   }
   ```

5. **Update all consumers**:
   - `Bundler`: Check `module_id == ModuleId::ENTRY` instead of name comparison
   - `ModuleClassifier`: Use module IDs for classification
   - `SymbolAnalyzer`: Find entry module by ID instead of name
   - `ImportAnalyzer`: Use IDs for import analysis

### Benefits

1. **Single Source of Truth**: Module information stored only in resolver
2. **No Duplication**: No more passing (name, path) tuples everywhere
3. **Consistency**: Single way to identify any module, including entry (ID 0)
4. **Performance**: Integer comparison instead of string comparison
5. **Memory Efficiency**: Pass around small IDs instead of cloning strings/paths
6. **Clarity**: No ambiguity about which module is the entry
7. **Simplification**: Remove redundant tracking and string comparisons

### Migration Steps

1. **Phase 1**: Add resolver query methods (get_module_name, get_module_path, etc.)
2. **Phase 2**: Update data structures to use ModuleId only (no name/path duplication)
3. **Phase 3**: Update comparison sites to use `module_id == ModuleId::ENTRY`
4. **Phase 4**: Remove entry_module_name from BundleParams and StaticBundleParams
5. **Phase 5**: Update all consumers to query resolver for module information
6. **Phase 6**: Update tests to verify entry module detection works via ID

## Implementation Checklist

- [x] Move ModuleId to resolver.rs with ENTRY constant
- [x] Implement ModuleId::is_entry() method
- [x] Update ModuleRegistry to start at 0
- [ ] Add resolver query methods (get_module_name, get_module_path, is_entry_package)
- [ ] Update data structures to use ModuleId only (remove name/path duplication)
- [ ] Update BundleParams to use ModuleIds and resolver reference
- [ ] Replace entry_module_name comparisons with module_id checks
- [ ] Remove entry_module_name from BundleParams
- [ ] Remove entry_module_name from StaticBundleParams
- [ ] Update Bundler to use module_id.is_entry() and query resolver
- [ ] Update ModuleClassifier to use module_id checks and query resolver
- [ ] Update SymbolAnalyzer to use module_id for entry detection
- [ ] Update ImportAnalyzer to use module_id
- [ ] Add tests verifying entry module detection via ID
- [ ] Run full test suite
- [ ] Run clippy and format

## Commands

```bash
# Find all is_entry_module usage to replace
rg "is_entry_module" --type rust

# Development iteration
cargo build --all-targets
cargo test --workspace

# Final validation
cargo clippy --workspace --all-targets
cargo fmt --all
cargo test --workspace
```
