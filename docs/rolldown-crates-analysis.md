# Rolldown Crates Analysis for Python Bundler Adaptation

## Overview

This document analyzes the Rolldown bundler codebase to identify abstract/generic crates that could be adapted for our Python bundler project.

## Key Generic/Abstract Crates

### 1. **rolldown_utils** - General Purpose Utilities

- **Purpose**: Collection of standalone utilities for bundler operations
- **Key Features**:
  - `BitSet`: Efficient bit manipulation for flags/sets
  - `make_unique_name`: Conflict-free name generation with counting
  - `pattern_filter`: String/regex pattern matching for include/exclude
  - `concat_string`: Efficient string concatenation
  - `index_vec_ext`: Parallel iteration extensions for IndexVec
  - `hash_placeholder`: Placeholder replacement utilities
  - `commondir`: Common directory resolution
  - `global_reference`: Managing global references
  - `stabilize_id`: ID stabilization utilities

### 2. **string_wizard** - String Manipulation Library

- **Purpose**: Advanced string manipulation with source mapping support
- **Key Features**:
  - `MagicString`: String manipulation with transformation tracking
  - `Joiner`: Efficient string joining operations
  - Source map generation support
  - Chunk-based string operations
- **Python Adaptation**: Useful for AST transformations and maintaining source mappings

### 3. **rolldown_fs** - File System Abstraction

- **Purpose**: Abstract file system interface
- **Key Features**:
  - `FileSystem` trait abstraction
  - `OsFileSystem`: OS file system implementation
  - `MemoryFileSystem`: In-memory file system (feature-gated)
  - Built on `vfs` crate for virtual file system support
- **Python Adaptation**: Perfect for testing and virtual environment support

### 4. **rolldown_std_utils** - Standard Library Extensions

- **Purpose**: Extensions for Rust standard library types
- **Key Features**:
  - `OptionExt`: Enhanced Option operations
  - `PathBufExt`: PathBuf extensions
  - `PathExt`: Path manipulation utilities
  - `pretty_type_name`: Type name formatting
- **Python Adaptation**: General-purpose utilities applicable to any bundler

### 5. **rolldown_common/types** - Core Data Structures

- **Purpose**: Common types used across the bundler
- **Key Generic Components**:
  - `HybridIndexVec<I, T>`: Flexible indexed storage (dense or sparse)
  - `ModuleTable`: Index-based module storage using `IndexVec`
  - `SymbolRef`: Cross-module symbol references
  - `BitSet` usage for flags and sets
  - Import/export tracking structures

### 6. **rolldown_filter_analyzer** - AST Analysis

- **Purpose**: Analyze AST for filterability/tree-shaking
- **Key Features**:
  - Control flow graph (CFG) analysis
  - Unreachable code detection
  - Return statement analysis
- **Python Adaptation**: Could be adapted for Python AST analysis

## Directly Reusable Components

### 1. Index-Based Data Structures

- **oxc_index** integration provides type-safe indexing
- `IndexVec` for dense storage with O(1) access
- `HybridIndexVec` for flexible dense/sparse storage
- Perfect for module graphs and symbol tables

### 2. String Interning and Manipulation

- `arcstr` for string interning (Arc<str>)
- `string_wizard` for complex string transformations
- Efficient memory usage for repeated strings

### 3. Pattern Matching

- `pattern_filter` module supports:
  - Glob patterns for file matching
  - Regex patterns with hybrid regex engine
  - Include/exclude filtering logic

### 4. Utility Functions

- `make_unique_name`: Generates unique names with numeric suffixes
- `concat_string!` macro: Efficient string concatenation
- `BitSet`: Space-efficient boolean flags

## Architecture Patterns to Adopt

### 1. Module Representation

```rust
// Rolldown's approach
pub struct ModuleTable {
    pub modules: IndexVec<ModuleIdx, Module>,
}
```

- Use newtype indices for type safety
- Store modules in contiguous memory
- Enable parallel processing with index ranges

### 2. Symbol Management

```rust
pub struct SymbolRef {
    pub owner: ModuleIdx,
    pub symbol: SymbolId,
}
```

- Two-level addressing: module + local symbol
- Canonical reference tracking for imports
- Efficient cross-module symbol resolution

### 3. Import/Export Tracking

- Separate indices for internal and external modules
- Import records with resolution metadata
- Named imports/exports with symbol mapping

## Integration Recommendations

### Phase 1: Direct Adoption

1. Copy `rolldown_utils` utilities (BitSet, make_unique_name, pattern_filter)
2. Adapt `string_wizard` for Python source manipulation
3. Use `oxc_index::IndexVec` for module storage

### Phase 2: Architecture Adoption

1. Implement module table with index-based storage
2. Create Python-specific symbol reference system
3. Build import/export tracking with named bindings

### Phase 3: Advanced Features

1. Adapt filter analyzer for Python AST
2. Implement file system abstraction for virtual environments
3. Add source map support via string_wizard

## Crates to Avoid/Skip

1. **rolldown_ecmascript*** - JavaScript-specific
2. **rolldown_plugin_*** - Plugin system (evaluate architecture only)
3. **rolldown_binding** - Node.js bindings
4. **rolldown_oxc_*** - JavaScript parser specific

## Conclusion

Rolldown provides excellent abstractions for:

- Index-based module graphs
- Efficient string manipulation
- Pattern matching and filtering
- File system abstraction

These patterns can significantly improve cribo's performance and maintainability when adapted for Python bundling.
