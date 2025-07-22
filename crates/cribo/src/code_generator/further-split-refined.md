# Refined Implementation Spec: Code Generator Reorganization

## 1. Introduction

This document provides a refined implementation specification for refactoring the `crates/cribo/src/code_generator/bundler.rs` file, addressing two key concerns:

1. **Proper organizational structure**: Pure analyzers and collectors should be separated from code generation logic
2. **Comprehensive visitor migration**: Detailed specification for migrating collection logic to the visitor pattern

## 2. Key Architectural Principles

### 2.1 Separation of Concerns

The refactoring will establish clear boundaries between:

- **Data Collection** (visitors): AST traversal and information gathering
- **Data Analysis** (analyzers): Processing collected data to derive insights
- **Code Generation** (generators): Creating new AST nodes and Python code
- **Transformation** (transformers): Modifying existing AST structures

### 2.2 Directory Structure

```
crates/cribo/src/
├── analyzers/                    # NEW: Pure analysis modules
│   ├── mod.rs
│   ├── dependency_analyzer.rs    # Dependency graph analysis
│   ├── symbol_analyzer.rs        # Symbol resolution and analysis
│   ├── import_analyzer.rs        # Import relationship analysis
│   └── namespace_analyzer.rs     # Namespace requirement analysis
├── code_generator/              # Code generation and transformation
│   ├── mod.rs
│   ├── bundler.rs              # Main orchestrator (~20k tokens)
│   ├── module_transformer.rs   # Module AST transformations
│   ├── import_transformer.rs   # Import rewriting
│   ├── expression_handlers.rs  # Expression creation/transformation
│   ├── namespace_manager.rs    # Namespace object generation
│   ├── module_registry.rs      # Module naming and registration
│   ├── import_deduplicator.rs  # Import cleanup
│   ├── circular_deps.rs        # (unchanged)
│   ├── globals.rs              # (unchanged)
│   └── context.rs              # (unchanged)
├── visitors/                    # AST traversal and data collection
│   ├── mod.rs
│   ├── import_discovery.rs     # (existing)
│   ├── side_effect_detector.rs # (existing)
│   ├── symbol_collector.rs     # NEW: Symbol collection visitor
│   ├── variable_collector.rs   # NEW: Variable usage visitor
│   └── export_collector.rs     # NEW: Export detection visitor
└── cribo_graph.rs              # Pure graph data structure
```

## 3. Visitor Pattern Implementation Guide

### 3.1 Visitor Architecture

Each visitor follows the same pattern established by existing visitors:

```rust
use ruff_python_ast::{visitor::Visitor /* specific AST nodes */};
use rustc_hash::{FxHashMap, FxHashSet};

pub struct MyVisitor {
    // Collected data structures
    collected_data: FxHashMap<String, Data>,
    // Current traversal state
    current_scope: Vec<Scope>,
    // Flags for context
    in_function: bool,
}

impl MyVisitor {
    pub fn new() -> Self { /* ... */
    }

    // Public API to run the visitor
    pub fn analyze(module: &ModModule) -> CollectedData {
        let mut visitor = Self::new();
        visitor.visit_body(&module.body);
        visitor.into_collected_data()
    }

    // Convert internal state to public data structure
    fn into_collected_data(self) -> CollectedData { /* ... */
    }
}

impl<'a> Visitor<'a> for MyVisitor {
    fn visit_stmt(&mut self, stmt: &'a Stmt) {
        // Handle specific statement types
        match stmt {
            Stmt::FunctionDef(func) => self.handle_function(func),
            // ... other cases
        }
        // Continue traversal
        walk_stmt(self, stmt);
    }
}
```

### 3.2 New Visitor Specifications

#### 3.2.1 Symbol Collector Visitor (`symbol_collector.rs`) ✅ COMPLETED

**Purpose**: Collect all symbol definitions, their scopes, and attributes.

**Collected Data**:

```rust
pub struct SymbolInfo {
    pub name: String,
    pub kind: SymbolKind,
    pub scope: ScopePath,
    pub is_exported: bool,
    pub is_global: bool,
    pub definition_range: TextRange,
}

pub enum SymbolKind {
    Function { decorators: Vec<String> },
    Class { bases: Vec<String> },
    Variable { is_constant: bool },
    Import { module: String },
}

pub struct CollectedSymbols {
    pub global_symbols: FxHashMap<String, SymbolInfo>,
    pub scoped_symbols: FxHashMap<ScopePath, Vec<SymbolInfo>>,
    pub module_renames: FxHashMap<String, String>,
}
```

**Implementation Details**:

- Track scope stack during traversal
- Detect global declarations
- Identify exported symbols via `__all__`
- Handle import aliases and renames
- Preserve decorator information for functions/classes

**Replaces bundler.rs methods**:

- ✅ `collect_global_symbols()`
- ✅ `collect_module_renames()` (integrated into symbol collector)
- ✅ `extract_all_exports()` (partially - **all** detection implemented)

#### 3.2.2 Variable Collector Visitor (`variable_collector.rs`)

**Purpose**: Track variable usage, references, and dependencies.

**Collected Data**:

```rust
pub struct VariableUsage {
    pub name: String,
    pub usage_type: UsageType,
    pub location: TextRange,
    pub scope: ScopePath,
}

pub enum UsageType {
    Read,
    Write,
    Delete,
    GlobalDeclaration,
    NonlocalDeclaration,
}

pub struct CollectedVariables {
    pub usages: Vec<VariableUsage>,
    pub function_globals: FxHashMap<String, FxHashSet<String>>,
    pub referenced_vars: FxHashSet<String>,
}
```

**Implementation Details**:

- Track reads/writes separately
- Handle augmented assignments
- Detect self-referential assignments
- Track function-level global usage
- Handle comprehension scopes

**Replaces bundler.rs methods**:

- `collect_referenced_vars()`
- `collect_vars_in_stmt()`
- `collect_vars_in_expr()`

#### 3.2.3 Export Collector Visitor (`export_collector.rs`)

**Purpose**: Detect module exports and re-exports.

**Collected Data**:

```rust
pub struct ExportInfo {
    pub exported_names: Option<Vec<String>>, // None means export all
    pub is_dynamic: bool,
    pub re_exports: Vec<ReExport>,
}

pub struct ReExport {
    pub from_module: String,
    pub names: Vec<(String, Option<String>)>, // (name, alias)
    pub is_star: bool,
}
```

**Implementation Details**:

- Parse `__all__` assignments
- Detect dynamic `__all__` modifications
- Track star imports that become re-exports
- Handle conditional exports

**Replaces bundler.rs methods**:

- Part of `extract_all_exports()`
- `is_package_init_reexport()`

### 3.3 Visitor Integration Pattern

Visitors are integrated into the bundler through analyzer modules:

```rust
// In analyzers/symbol_analyzer.rs
use crate::visitors::{SymbolCollector, VariableCollector};

pub struct SymbolAnalyzer;

impl SymbolAnalyzer {
    pub fn analyze_module(module: &ModModule) -> SymbolAnalysis {
        // Run visitors
        let symbols = SymbolCollector::analyze(module);
        let variables = VariableCollector::analyze(module);

        // Perform analysis on collected data
        Self::build_dependency_graph(&symbols, &variables)
    }
}
```

## 4. Phased Implementation Plan

### Phase 1: Create Analyzer Infrastructure ✅ COMPLETED

**New directories and base modules**:

1. ✅ Create `src/analyzers/` directory
2. ✅ Create `analyzers/mod.rs` with module declarations
3. ✅ Move analyzer-related types from `bundler.rs` to `analyzers/types.rs`

### Phase 2: Implement Symbol Collection Visitor ✅ COMPLETED

**Steps**:

1. ✅ Create `visitors/symbol_collector.rs`
2. ✅ Implement the visitor following the pattern above
3. ✅ Create `analyzers/symbol_analyzer.rs`
4. ✅ Migrate symbol analysis methods from `bundler.rs`
5. ✅ Update `bundler.rs` to use the new analyzer

**Methods to migrate**:

- ✅ `collect_global_symbols()` → visitor
- ✅ `find_symbol_module()` → analyzer
- ✅ `build_symbol_dependency_graph()` → analyzer
- ✅ `detect_hard_dependencies()` → analyzer (also migrated)

### Phase 3: Implement Variable Collection Visitor ✅ COMPLETED

**Steps**:

1. ✅ Create `visitors/variable_collector.rs`
2. ✅ Implement comprehensive variable tracking
3. ✅ Integrate with `symbol_analyzer.rs`
4. ✅ Remove variable collection from `bundler.rs`

**Methods to migrate**:

- ✅ `collect_referenced_vars()` → visitor
- ✅ `collect_vars_in_stmt()` → visitor
- ✅ `collect_vars_in_expr()` → visitor (as static helper)
- ✅ `collect_function_globals()` → visitor

### Phase 4: Implement Export Collection Visitor ✅ COMPLETED

**Steps**:

1. ✅ Create `visitors/export_collector.rs`
2. ✅ Handle `__all__` and re-export patterns
3. ✅ Create dedicated export analysis in analyzer

**Methods to migrate**:

- ✅ `extract_all_exports()` → visitor + analyzer
- ✅ `extract_string_list_from_expr()` → visitor helper
- ✅ `is_package_init_reexport()` → analyzer

### Phase 5: Create Dedicated Analyzers ✅ COMPLETED

**New modules in `analyzers/`**:

1. ✅ **`dependency_analyzer.rs`**:
   - ✅ `detect_hard_dependencies()`
   - ✅ `sort_wrapper_modules_by_dependencies()`
   - ✅ `sort_wrapped_modules_by_dependencies()`

2. ✅ **`import_analyzer.rs`**:
   - ✅ `find_directly_imported_modules()`
   - ✅ `find_namespace_imported_modules()`
   - ✅ `find_matching_module_name_namespace()`

3. ✅ **`namespace_analyzer.rs`**:
   - ✅ `identify_required_namespaces()`
   - ✅ Namespace requirement detection logic

### Phase 6: Refactor cribo_graph.rs ✅ COMPLETED

**Move analysis methods to analyzers**:

- ✅ `analyze_circular_dependencies()` → `dependency_analyzer.rs`
- ✅ `find_unused_imports()` → `import_analyzer.rs`
- ✅ Keep only pure graph operations in `cribo_graph.rs`
- ✅ All related types moved to `analyzers/types.rs`
- ✅ Tests updated and passing

### Phase 7: Complete Code Generator Extraction

Continue with the original plan for:

- `expression_handlers.rs`
- `namespace_manager.rs`
- `import_deduplicator.rs`

But now these modules will call into analyzers rather than doing analysis themselves.

## 5. Validation and Testing Strategy

### 5.1 Unit Testing for Visitors

Each visitor should have comprehensive unit tests:

```rust
#[cfg(test)]
mod tests {
    use ruff_python_parser::parse_module;

    use super::*;

    #[test]
    fn test_symbol_collection() {
        let code = r#"
def foo():
    pass

class Bar:
    pass

x = 42
"#;
        let module = parse_module(code).unwrap();
        let symbols = SymbolCollector::analyze(&module);

        assert_eq!(symbols.global_symbols.len(), 3);
        // ... more assertions
    }
}
```

### 5.2 Integration Testing

- Ensure bundler produces identical output after each phase
- Run full test suite after each extraction
- Benchmark performance to ensure no regression

### 5.3 Migration Validation

For each migrated method:

1. Create parallel implementation using visitor
2. Compare outputs on test fixtures
3. Switch to visitor-based implementation
4. Remove old implementation

## 6. Benefits of This Approach

1. **Clear Separation**: Analyzers are separate from generators
2. **Reusability**: Visitors can be reused by multiple analyzers
3. **Testability**: Each component has a single responsibility
4. **Performance**: Visitors traverse AST once, collecting all needed data
5. **Maintainability**: New analysis can be added without touching bundler
6. **Extensibility**: Easy to add new visitors for future features

## 7. Success Metrics

- `bundler.rs` reduced to ~20,000 tokens (orchestration only)
- Zero analysis logic remaining in code_generator/
- All AST traversal consolidated in visitors/
- Each analyzer module < 10,000 tokens
- Visitor test coverage > 90%
- No performance regression in bundling

## 8. Implementation Status

- **Phase 1**: ✅ COMPLETED - Analyzer infrastructure created
- **Phase 2**: ✅ COMPLETED - Symbol collection visitor implemented
- **Phase 3**: ✅ COMPLETED - Variable collection visitor implemented
- **Phase 4**: ✅ COMPLETED - Export collection visitor implemented
- **Phase 5**: ✅ COMPLETED - Dedicated analyzers created
- **Phase 6**: ✅ COMPLETED - Refactored cribo_graph.rs
- **Phase 7**: ⏳ PENDING - Complete code generator extraction

## 9. Future Considerations

This architecture enables future enhancements:

- Incremental bundling (visitors can track changes)
- Parallel analysis (visitors are independent)
- Language server integration (reuse visitors)
- Advanced optimizations (based on collected data)
