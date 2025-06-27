# Graph Import Metadata Architecture

## Overview

This document outlines the architectural refactoring to centralize module classification and metadata storage in the CriboGraph. The goal is to eliminate scattered logic, improve maintainability, and fix configuration plumbing issues.

## Problem Statement

### Current Issues

1. **Scattered Module Classification Logic**
   - Module classification (stdlib/third-party/first-party) is performed on-demand in multiple places
   - `bundle_compiler/compiler.rs` calls `is_stdlib_without_side_effects` for hoisting decisions
   - `resolver.rs` calls `is_stdlib_module` during resolution
   - Each component needs access to Python version configuration

2. **Configuration Plumbing Problems**
   - Python version must be threaded through multiple components
   - Led to bugs like missing `python_version` parameter in `resolver.rs:666`
   - Hardcoded `PYTHON_VERSION` constants appeared in the codebase

3. **Incomplete Data Model**
   - `CriboGraph` lacks fundamental module properties
   - No storage for module kind (stdlib/third-party/first-party)
   - No storage for behavioral flags (has_side_effects)
   - Forces downstream components to recompute information

4. **Name Collision**
   - Two different `ImportType` enums with completely different semantics
   - One in `cribo_graph.rs` (deprecated, describes import syntax)
   - One in `resolver.rs` (describes module classification)

## Architectural Principles

1. **Single Source of Truth**: Module classification and properties should be determined once during resolution
2. **Graph as Authority**: CriboGraph should store all static analysis results
3. **Resolver as Classifier**: The Resolver is the sole authority for module classification
4. **Separation of Concerns**: Module origin (kind) and behavior (side effects) are orthogonal

## Solution Design

### New Data Model

```rust
// in crates/cribo/src/types.rs
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModuleKind {
    StandardLibrary,
    ThirdParty,
    FirstParty,
}

// in crates/cribo/src/cribo_graph.rs
pub struct ModuleDepGraph {
    // ... existing fields ...
    pub kind: ModuleKind,
    pub has_side_effects: bool,
}
```

### Resolution Flow

```
User Request → Resolver → ModuleResolutionResult → GraphBuilder → CriboGraph → BundleCompiler
                   ↓                                      ↓
            Classification Logic                  Side Effect Detection
            (uses python_version)                 (stdlib knowledge + config)
```

### Separation of Concerns

1. **Resolver**: Determines module classification (ModuleKind) only
   - Finds module files on disk
   - Classifies as StandardLibrary/ThirdParty/FirstParty
   - Uses Python version for stdlib detection
   - Does NOT determine side effects

2. **Graph/GraphBuilder**: Determines behavioral properties
   - Side effect detection happens AFTER modules are in the graph
   - For stdlib: Uses hardcoded list from `stdlib_detection.rs`
   - For first-party: Analyzes AST (existing functionality)
   - For third-party: Defaults to true unless config overrides
   - Respects Python version for stdlib side effects

## Implementation Plan

### Phase 1: Prerequisites

1. **Rename Name Collision**
   - File: `crates/cribo/src/cribo_graph.rs`
   - Rename deprecated `ImportType` → `ImportSyntaxKind`
   - Update all references

### Phase 2: Create Shared Types

1. **Create Types Module**
   - Create `crates/cribo/src/types.rs`
   - Define `ModuleKind` enum
   - Add to lib.rs exports

### Phase 3: Enhance Graph Model

1. **Update ModuleDepGraph**
   - Add `kind: ModuleKind` field
   - Add `has_side_effects: bool` field
   - Update constructors and builders

### Phase 4: Centralize Classification

1. **Enhance Resolver**
   - Create `ModuleResolutionResult` struct:
     ```rust
     pub struct ModuleResolutionResult {
         pub path: PathBuf,
         pub kind: ModuleKind,
         pub has_side_effects: bool,
     }
     ```
   - Update resolution methods to return full result
   - Fix bug: use `self.config.python_version()` for stdlib detection
   - Implement complete classification logic

2. **Classification Logic**
   ```rust
   let python_version = self.config.python_version()?;
   let is_stdlib = is_stdlib_module(module_name, python_version);
   let has_side_effects = if is_stdlib {
       !is_stdlib_without_side_effects(module_name, python_version)
   } else {
       // Assume side effects for non-stdlib by default
       // Future: check pyproject.toml for side-effects key
       true
   };
   ```

### Phase 5: Update Consumers

1. **Update GraphBuilder**
   - Receive `ModuleResolutionResult` from resolver
   - Populate new fields when creating `ModuleDepGraph`
   - No longer needs to determine classification

2. **Refactor BundleCompiler**
   - Remove all direct calls to `is_stdlib_module`
   - Remove all direct calls to `is_stdlib_without_side_effects`
   - Use graph data: `if module.kind == ModuleKind::StandardLibrary && !module.has_side_effects`
   - No longer needs python_version for classification

### Phase 6: Cleanup

1. **Remove Redundant Code**
   - Remove `ImportType` from resolver.rs
   - Remove any remaining classification logic from compiler
   - Update tests to use new architecture

## Benefits

1. **Correctness**
   - Single source of truth for module classification
   - Consistent application of Python version configuration
   - Eliminates class of bugs like missing parameters

2. **Performance**
   - Classification computed once per module
   - Results cached in graph
   - No repeated filesystem access or computation

3. **Maintainability**
   - Clear separation of concerns
   - Centralized logic easier to update
   - Reduced coupling between components

4. **Extensibility**
   - Easy to add new module properties
   - Future support for side-effects detection in third-party packages
   - Clear extension points for new classification criteria

## Migration Notes

### Breaking Changes

- `ModuleDepGraph` structure changes require graph serialization updates
- Resolver API changes affect any direct consumers

### Compatibility

- Existing bundler output remains identical
- No changes to CLI interface
- Internal refactoring only

## Future Enhancements

1. **Side Effects Detection**
   - Read `side-effects` key from pyproject.toml
   - Analyze package.json for JavaScript-style side effects
   - Static analysis for common side effect patterns

2. **Module Metadata**
   - License information
   - Version constraints
   - Security advisories
   - Performance characteristics

3. **Graph Serialization**
   - Cache resolved module metadata
   - Speed up incremental builds
   - Share analysis across runs

## Testing Strategy

1. **Unit Tests**
   - Test resolver classification logic
   - Test graph storage and retrieval
   - Test compiler consumption of graph data

2. **Integration Tests**
   - Ensure bundler output unchanged
   - Test with various Python versions
   - Test with mixed module types

3. **Regression Tests**
   - Specific test for resolver.rs:666 bug
   - Test configuration propagation
   - Test side effect detection
